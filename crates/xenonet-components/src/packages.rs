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

/// /^[a-f0-9]{64}$/: a SHA-256 in hex, or a runtime file's token.
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

    use crate::tar::tests::archive;
    use std::path::PathBuf;
    use xenonet_app::task::{run_with, Done, Task};

    /// A runtime folder of a test's own, on a made-up linux-x64 machine.
    struct Place {
        dir: PathBuf,
        env: Env,
        m: Machine,
    }

    fn place(name: &str) -> Place {
        let dir = std::env::temp_dir().join(format!("xenonet-packages-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = Env::new();
        env.insert("XENONET_RUNTIME_DIR".into(), json!(dir.join("runtime").to_string_lossy()));
        Place { m: Machine { platform: "linux".into(), arch: "x64".into(), home: dir.to_string_lossy().into_owned() }, env, dir }
    }

    fn component_json(component: &str, version: &str, platform: &str) -> Vec<u8> {
        json!({ "format": 1, "component": component, "version": version, "platform": platform, "arch": "x64", "dataSchema": 1, "api": { "min": 1, "max": 1 } })
            .to_string()
            .into_bytes()
    }

    /// A Tools package: its component.json and bin/xenonet, less the ones
    /// `leave` names, and `more` entries.
    fn tools(version: &str, leave: &[&str], more: &[(&str, u8, &[u8], u32)]) -> Vec<u8> {
        let manifest = component_json("tools", version, "linux");
        let all: Vec<(&str, u8, &[u8], u32)> = [("component.json", b'0', &manifest[..], 0o644), ("bin/xenonet", b'0', &b"#!/bin/sh\n"[..], 0o755)]
            .into_iter()
            .filter(|(n, ..)| !leave.contains(n))
            .chain(more.iter().copied())
            .collect();
        archive(&all)
    }

    /// The release's word on a package: what it is and its hash and size.
    fn release(bytes: &[u8], version: &str) -> Value {
        json!({ "component": "tools", "version": version, "sha256": digest(bytes), "size": bytes.len() })
    }

    fn stage(p: &Place, bytes: &[u8], expected: &Value) -> Done {
        let file = p.dir.join(format!("{}.tar.gz", digest(bytes)));
        std::fs::write(&file, bytes).unwrap();
        let (file, expected, env, m) = (file.to_string_lossy().into_owned(), expected.clone(), p.env.clone(), p.m.clone());
        run_with(Task::new(move |io| async move { stage_package(&io, &file, &expected, &env, &m).await }), |q| Err(json!({ "message": format!("nothing here answers {q}") })))
    }

    fn refusal(d: Done) -> String {
        match d {
            Err(f) => f.message().to_string(),
            Ok(v) => panic!("staged: {v}"),
        }
    }

    fn tools_dir(p: &Place) -> PathBuf {
        p.dir.join("runtime").join("components").join("tools")
    }

    #[test]
    fn a_package_is_checked_against_its_release_before_anything_is_unpacked() {
        let p = place("checked");
        let bytes = tools("1.2.3", &[], &[]);
        let good = release(&bytes, "1.2.3");
        for (k, v) in [
            ("component", json!("updater")),
            ("component", json!(null)),
            ("version", json!("1.2")),
            ("version", json!("1.2.3/../..")),
            ("sha256", json!("ABC")),
            ("sha256", json!(null)),
        ] {
            let mut bad = good.clone();
            bad[k] = v;
            assert_eq!(refusal(stage(&p, &bytes, &bad)), "Invalid component release manifest.", "{bad}");
        }
        for (k, v) in [("size", json!(bytes.len() + 1)), ("size", json!(null)), ("size", json!(bytes.len().to_string())), ("sha256", json!(digest(b"another package")))] {
            let mut bad = good.clone();
            bad[k] = v;
            assert_eq!(refusal(stage(&p, &bytes, &bad)), "Component download failed its hash or size check.", "{bad}");
        }
        assert!(!p.dir.join("runtime").exists(), "nothing was unpacked or made");
        std::fs::remove_dir_all(&p.dir).unwrap();
    }

    #[test]
    fn an_unsafe_or_mismatched_archive_is_never_installed() {
        let p = place("unsafe");
        let refused = |bytes: Vec<u8>, version: &str| refusal(stage(&p, &bytes, &release(&bytes, version)));
        assert_eq!(refused(tools("1.2.3", &[], &[("../escape", b'0', b"x", 0o644)]), "1.2.3"), "Unsafe or oversized component archive.");
        assert_eq!(refused(tools("1.2.3", &[], &[("bin/link", b'2', b"", 0o777)]), "1.2.3"), "Unsafe or oversized component archive.");
        assert!(!tools_dir(&p).join("escape").exists() && !p.dir.join("escape").exists());
        assert_eq!(refused(tools("9.9.9", &[], &[]), "1.2.3"), "Component archive differs from its release manifest.");
        let service = archive(&[("component.json", b'0', &component_json("service", "1.2.3", "linux"), 0o644)]);
        assert_eq!(refused(service, "1.2.3"), "Component archive differs from its release manifest.");
        let elsewhere = archive(&[("component.json", b'0', &component_json("tools", "1.2.3", "win32"), 0o644)]);
        assert_eq!(refused(elsewhere, "1.2.3"), "This component package is not compatible with this XenoNet installation.");
        assert_eq!(refused(tools("1.2.3", &["bin/xenonet"], &[]), "1.2.3"), "Component is missing bin/xenonet.");
        assert_eq!(refused(tools("1.2.3", &["bin/xenonet"], &[("bin/xenonet/", b'5', b"", 0o755)]), "1.2.3"), "Component is missing bin/xenonet.");
        // An earlier XenoNet's package, a node and its JS, has no binary.
        let earlier = archive(&[
            ("component.json", b'0', &component_json("tools", "1.2.3", "linux"), 0o644),
            ("node", b'0', b"#!/bin/sh\n", 0o755),
            ("cli/xenonet.js", b'0', b"// tools", 0o644),
        ]);
        assert_eq!(refused(earlier, "1.2.3"), "Component is missing bin/xenonet.");
        assert!(refused(b"not a package at all".repeat(40), "1.2.3").starts_with("TAR_ENTRY_INVALID"));
        // None of them went in beside the versions that are.
        let left: Vec<String> = std::fs::read_dir(tools_dir(&p)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(left.iter().all(|n| n.starts_with(".stage-")), "{left:?}");
        std::fs::remove_dir_all(&p.dir).unwrap();
    }

    #[test]
    fn a_published_version_is_never_replaced() {
        let p = place("published");
        let bytes = tools("1.2.3", &[], &[]);
        let staged = stage(&p, &bytes, &release(&bytes, "1.2.3")).unwrap();
        let root = staged["root"].as_str().unwrap().to_string();
        assert!(root.ends_with(&format!("1.2.3-{}", &digest(&bytes)[..16])));
        select_component("tools", &staged, &p.env, &p.m).unwrap();
        let current = current_component("tools", &p.env, &p.m).unwrap().unwrap();
        assert_eq!((current["version"].clone(), current["root"].clone()), (json!("1.2.3"), json!(root)));
        // The same version again, from a package of other bytes.
        let other = tools("1.2.3", &[], &[("extra.js", b'0', b"// more", 0o644)]);
        assert_eq!(refusal(stage(&p, &other, &release(&other, "1.2.3"))), "A published component version cannot be replaced. Release a new version.");
        // The same package again: the version unpacked already, as it is.
        std::fs::write(PathBuf::from(&root).join("bin/xenonet"), "// as it was").unwrap();
        assert_eq!(stage(&p, &bytes, &release(&bytes, "1.2.3")).unwrap()["root"], json!(root));
        assert_eq!(std::fs::read_to_string(PathBuf::from(&root).join("bin/xenonet")).unwrap(), "// as it was");
        std::fs::remove_dir_all(&p.dir).unwrap();
    }

    #[test]
    fn the_installed_pointer_is_checked() {
        let p = place("pointer");
        assert_eq!(current_component("updater", &p.env, &p.m).unwrap_err().message, "Choose service or tools.");
        assert_eq!(current_component("tools", &p.env, &p.m), Ok(None));
        std::fs::create_dir_all(tools_dir(&p)).unwrap();
        let pointer = tools_dir(&p).join("current.json");
        for (text, want) in [
            ("null", "Cannot read properties of null (reading 'sha256')"),
            (r#"{"version":"1.2.3","sha256":"../../../elsewhere"}"#, "Invalid installed component pointer."),
            (r#"{"version":"../1.2.3","sha256":"0000000000000000000000000000000000000000000000000000000000000000"}"#, "Invalid installed component pointer."),
            (r#"{"version":"1.2.3"}"#, "Invalid installed component pointer."),
        ] {
            std::fs::write(&pointer, text).unwrap();
            assert_eq!(current_component("tools", &p.env, &p.m).unwrap_err().message, want, "{text}");
        }
        std::fs::write(&pointer, "{").unwrap();
        assert!(current_component("tools", &p.env, &p.m).is_err());
        std::fs::remove_dir_all(&p.dir).unwrap();
    }

    #[test]
    fn one_updater_at_a_time() {
        let p = place("lock");
        let take = |answer: Result<Value, Value>| {
            let (env, m) = (p.env.clone(), p.m.clone());
            run_with(Task::new(move |io| async move { lock(&io, &env, &m).await.map(Value::String) }), move |_| answer.clone())
        };
        let file = take(Ok(json!(null))).unwrap();
        let file = file.as_str().unwrap();
        assert_eq!(std::fs::read_to_string(file).unwrap(), std::process::id().to_string());
        // Held by a process that's there: refused; by one that's gone: taken.
        assert_eq!(take(Ok(json!(null))).unwrap_err().message(), "Another XenoNet update is in progress.");
        assert_eq!(take(Err(json!({ "code": "EPERM" }))), Err(Failure::Rethrow));
        assert!(take(Err(json!({ "code": "ESRCH" }))).is_ok());
        // A lock with no pid in it yet is one starting.
        std::fs::write(file, "").unwrap();
        assert_eq!(take(Ok(json!(null))).unwrap_err().message(), "Another XenoNet update is starting.");
        unlock(file).unwrap();
        assert!(take(Ok(json!(null))).is_ok());
        std::fs::remove_dir_all(&p.dir).unwrap();
    }
}
