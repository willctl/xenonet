//! core/components/packages.js: a component's package. It's checked
//! against its release's hash and size before it's unpacked; unpacked only
//! when it holds nothing but files and folders inside it; and a published
//! version can never be replaced by another package of the same version.
//! What's installed is a pointer to one version (current.json), switched
//! with a rename.

use crate::runtime::{bin_name, runtime_dir, Env, Machine};
use crate::tar::{unpack, Unpack};
use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use xenonet_app::fsx::{self, FsError};
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{now_ms, Failure, Io};
use xenonet_base::text::js_string;

pub const COMPONENTS: [&str; 2] = ["service", "tools"];
/// The largest package XenoNet takes: 512 MiB.
pub const MAX_PACKAGE: usize = 512 << 20;

fn semver() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[0-9]+\.[0-9]+\.[0-9]+$").expect("a fixed pattern"))
}

pub fn sha_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new("^[a-f0-9]{64}$").expect("a fixed pattern"))
}

// RegExp.test reads its argument as a string.
fn test(re: &Regex, v: Option<&Value>) -> bool {
    re.is_match(&v.map_or_else(|| "undefined".to_string(), js_string))
}

pub fn is_semver(v: Option<&Value>) -> bool {
    test(semver(), v)
}

pub fn is_sha(v: Option<&Value>) -> bool {
    test(sha_re(), v)
}

/// The SHA-256 of bytes, in hex.
pub fn digest(data: &[u8]) -> String {
    xenonet_base::jsstr::hex(&Sha256::digest(data))
}

/// Whether version a is newer than b.
pub fn newer(a: Option<&Value>, b: Option<&Value>) -> Result<bool, String> {
    let (Some(a), Some(b)) = (a.filter(|v| is_semver(Some(v))), b.filter(|v| is_semver(Some(v)))) else {
        return Err("Invalid component version.".into());
    };
    let parts = |v: &Value| -> Vec<f64> { js_string(v).split('.').map(|p| p.parse::<f64>().unwrap_or(f64::NAN)).collect() };
    let (x, y) = (parts(a), parts(b));
    for i in 0..3 {
        if x[i] != y[i] {
            return Ok(x[i] > y[i]);
        }
    }
    Ok(false)
}

/// A package's component.json, if it's one this installation can use.
pub fn package_info(value: Option<&Value>, platform: &str, arch: &str) -> Result<Value, String> {
    let v = value.unwrap_or(&Value::Null);
    let is = |k: &str, want: &Value| v.get(k) == Some(want);
    let component_ok = v.get("component").and_then(Value::as_str).is_some_and(|c| COMPONENTS.contains(&c));
    if !is("format", &json!(1))
        || !component_ok
        || !is_semver(v.get("version"))
        || v.get("platform").and_then(Value::as_str) != Some(platform)
        || v.get("arch").and_then(Value::as_str) != Some(arch)
        || !is("dataSchema", &json!(1))
    {
        return Err("This component package is not compatible with this XenoNet installation.".into());
    }
    crate::protocol::negotiate(Some(&crate::protocol::api_value(crate::protocol::SERVICE_API)), v.get("api"))?;
    Ok(v.clone())
}

pub fn components_dir(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), "components"])
}

/// The version of a component that's selected, and where it is; None when
/// there's none.
pub fn current_component(component: &str, env: &Env, m: &Machine) -> Result<Option<Value>, FsError> {
    if !COMPONENTS.contains(&component) {
        return Err(fsx::said("Choose service or tools."));
    }
    let dir = NATIVE.join(&[&components_dir(env, m), component]);
    let text = match fsx::read_text(&NATIVE.join(&[&dir, "current.json"])) {
        Ok(t) => t,
        Err(e) if e.is("ENOENT") => return Ok(None),
        Err(e) => return Err(e),
    };
    let current = xenonet_base::json::parse(&text).map_err(|e| fsx::said(e.to_string()))?;
    if current.is_null() {
        return Err(fsx::said("Cannot read properties of null (reading 'sha256')"));
    }
    if !is_sha(current.get("sha256")) || !is_semver(current.get("version")) {
        return Err(fsx::said("Invalid installed component pointer."));
    }
    let version = js_string(&current["version"]);
    let sha = js_string(&current["sha256"]);
    let mut out = match current {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    out.insert("root".into(), json!(NATIVE.join(&[&dir, &format!("{version}-{}", &sha[..16])])));
    Ok(Some(Value::Object(out)))
}

fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = xenonet_base::jsstr::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// Selects a version of a component: its pointer, written to a temporary
/// file and renamed into place.
pub fn select_component(component: &str, value: &Value, env: &Env, m: &Machine) -> Result<(), FsError> {
    let dir = NATIVE.join(&[&components_dir(env, m), component]);
    fsx::mkdir_p(&dir, None)?;
    let file = NATIVE.join(&[&dir, "current.json"]);
    let tmp = format!("{file}.{}.tmp", random_uuid());
    let mut pointer = Map::new();
    for k in ["version", "sha256"] {
        if let Some(v) = value.get(k) {
            pointer.insert(k.into(), v.clone());
        }
    }
    fsx::write_file(&tmp, xenonet_base::json::stringify(&Value::Object(pointer), 0).as_bytes(), Some(0o600))?;
    fsx::rename(&tmp, &file)
}

fn with_root(manifest: Value, root: &str, sha256: &Value) -> Value {
    let mut m = match manifest {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    m.insert("root".into(), json!(root));
    m.insert("sha256".into(), sha256.clone());
    Value::Object(m)
}

fn read_json(file: &str) -> Result<Value, Failure> {
    let text = fsx::read_text(file)?;
    xenonet_base::json::parse(&text).map_err(|e| Failure::msg(e.to_string()))
}

/// Checks and unpacks a downloaded package beside the other versions of
/// its component, without selecting it. A version unpacked already is
/// taken as it is. On Windows, where a scanner can hold a file it just saw
/// for a moment, the last rename is tried again for up to 30 s; nothing is
/// unpacked again or changed.
pub async fn stage_package(io: &Io, file: &str, expected: &Value, env: &Env, m: &Machine) -> Result<Value, Failure> {
    let component = expected.get("component").and_then(Value::as_str).filter(|c| COMPONENTS.contains(c));
    let (Some(component), true, true) = (component, is_semver(expected.get("version")), is_sha(expected.get("sha256"))) else {
        return Err(Failure::msg("Invalid component release manifest."));
    };
    let version = js_string(&expected["version"]);
    let sha = js_string(&expected["sha256"]);
    let bytes = std::fs::read(file).map_err(|e| FsError::of(e, "open", file))?;
    let size_ok = expected.get("size").and_then(Value::as_f64) == Some(bytes.len() as f64);
    if bytes.len() > MAX_PACKAGE || !size_ok || digest(&bytes) != sha {
        return Err(Failure::msg("Component download failed its hash or size check."));
    }
    let parent = NATIVE.join(&[&components_dir(env, m), component]);
    fsx::mkdir_p(&parent, Some(0o700))?;
    let dest = NATIVE.join(&[&parent, &format!("{version}-{}", &sha[..16])]);
    let existing = current_component(component, env, m)?;
    if let Some(e) = &existing {
        if e.get("version").map(js_string).as_deref() == Some(version.as_str()) && e.get("sha256").map(js_string).as_deref() != Some(sha.as_str()) {
            return Err(Failure::msg("A published component version cannot be replaced. Release a new version."));
        }
    }
    if fsx::exists(&dest) {
        let manifest = package_info(Some(&read_json(&NATIVE.join(&[&dest, "component.json"]))?), &m.platform, &m.arch).map_err(Failure::msg)?;
        return Ok(with_root(manifest, &dest, &expected["sha256"]));
    }
    let staging = fsx::mkdtemp(&NATIVE.join(&[&parent, ".stage-"]))?;
    match unpack(&bytes, std::path::Path::new(&staging)) {
        Ok(()) => {}
        Err(Unpack::Refused) => return Err(Failure::msg("Unsafe or oversized component archive.")),
        Err(Unpack::Bad(why)) => return Err(Failure::msg(why)),
        Err(Unpack::Write(e)) => return Err(e.into()),
    }
    let manifest = package_info(Some(&read_json(&NATIVE.join(&[&staging, "component.json"]))?), &m.platform, &m.arch).map_err(Failure::msg)?;
    if manifest.get("component").and_then(Value::as_str) != Some(component) || manifest.get("version").map(js_string).as_deref() != Some(version.as_str()) {
        return Err(Failure::msg("Component archive differs from its release manifest."));
    }
    // Service and Tools are each the xenonet binary, in bin/.
    if !std::fs::metadata(NATIVE.join(&[&staging, "bin", bin_name(&m.platform)])).is_ok_and(|meta| meta.is_file()) {
        return Err(Failure::msg(format!("Component is missing bin/{}.", bin_name(&m.platform))));
    }
    let until = now_ms() + 30000.0;
    loop {
        match std::fs::rename(&staging, &dest) {
            Ok(()) => break,
            Err(e) => {
                if fsx::exists(&dest) {
                    break;
                }
                let code = xenonet_base::fs::code_of(&e);
                let mut err = FsError::of(e, "rename", &staging);
                if err.code.is_some() {
                    err.message = format!("{} -> '{dest}'", err.message);
                }
                if !cfg!(windows) || !matches!(code, Some("EPERM" | "EACCES" | "EBUSY")) || now_ms() >= until {
                    return Err(err.into());
                }
                io.sleep(250.0).await;
            }
        }
    }
    Ok(with_root(manifest, &dest, &expected["sha256"]))
}

/// The install lock: one updater at a time, shared by Desktop, the CLI
/// and bundle setup. A lock left by a process that's gone is taken over.
///
/// Asks { op: "alive", pid } (process.kill(pid, 0)) when the lock is held.
pub async fn lock(io: &Io, env: &Env, m: &Machine) -> Result<String, Failure> {
    let dir = runtime_dir(env, m);
    fsx::mkdir_p(&dir, Some(0o700))?;
    let file = NATIVE.join(&[&dir, "install.lock"]);
    if let Err(e) = create_lock(&file) {
        if !e.is("EEXIST") {
            return Err(e.into());
        }
        let text = fsx::read_text(&file)?;
        let pid = xenonet_base::text::js_number(Some(&Value::String(text)));
        if pid > 0.0 {
            // process.kill(pid, 0): there, or not (ESRCH); anything else is
            // passed on.
            match io.ask(json!({ "op": "alive", "pid": pid })).await {
                Ok(_) => return Err(Failure::msg("Another XenoNet update is in progress.")),
                Err(e) if e.get("code").and_then(Value::as_str) == Some("ESRCH") => {}
                Err(_) => return Err(Failure::Rethrow),
            }
        } else {
            return Err(Failure::msg("Another XenoNet update is starting."));
        }
        fsx::unlink(&file)?;
        create_lock(&file)?;
    }
    Ok(file)
}

fn create_lock(file: &str) -> Result<(), FsError> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(file).map_err(|e| FsError::of(e, "open", file))?;
    std::io::Write::write_all(&mut f, std::process::id().to_string().as_bytes()).map_err(|e| FsError::of(e, "write", file))
}

/// Lets the install lock go.
pub fn unlock(file: &str) -> Result<(), FsError> {
    fsx::unlink(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_packages() {
        assert_eq!(newer(Some(&json!("1.10.0")), Some(&json!("1.9.9"))), Ok(true));
        assert_eq!(newer(Some(&json!("1.0.0")), Some(&json!("1.0.0"))), Ok(false));
        assert!(newer(Some(&json!("1.0")), Some(&json!("1.0.0"))).is_err());
        let ok = json!({ "format": 1, "component": "tools", "version": "1.2.3", "platform": "darwin", "arch": "arm64", "dataSchema": 1, "api": { "min": 1, "max": 1 } });
        assert!(package_info(Some(&ok), "darwin", "arm64").is_ok());
        assert!(package_info(Some(&ok), "linux", "arm64").unwrap_err().contains("not compatible"));
        let mut newer_api = ok.clone();
        newer_api["api"] = json!({ "min": 2, "max": 2 });
        assert!(package_info(Some(&newer_api), "darwin", "arm64").unwrap_err().contains("incompatible"));
        assert_eq!(digest(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
