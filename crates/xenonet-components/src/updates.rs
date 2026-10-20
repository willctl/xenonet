//! core/components/updates.js: the components this XenoNet could update
//! to, from the latest release's components.json, and their downloads:
//! only from the release's own address, written to a temporary file no
//! bigger than the release says, and kept only when its SHA-256 is the
//! release's.

use crate::packages::{current_component, digest, is_sha, newer, package_info, MAX_PACKAGE};
use crate::runtime::{runtime_dir, Env, Machine};
use regex::Regex;
use serde_json::{json, Value};
use std::io::Write;
use std::sync::OnceLock;
use xenonet_app::fsx::{self, FsError};
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;

pub const FEED: &str = "https://github.com/willctl/xenonet/releases/latest/download/components.json";
const DOWNLOADS: &str = "https://github.com/willctl/xenonet/releases/latest/download/";

fn file_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^XenoNet-(service|tools)-[A-Za-z0-9_.-]+\.tar\.gz$").expect("a fixed pattern"))
}

fn url_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^https://github\.com/willctl/xenonet/releases/latest/download/XenoNet-[A-Za-z0-9_.-]+\.tar\.gz$").expect("a fixed pattern"))
}

fn size_ok(v: Option<&Value>) -> bool {
    v.and_then(Value::as_f64).is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9007199254740991.0 && n >= 1.0 && n <= MAX_PACKAGE as f64)
}

/// The packages of the latest release for this machine, each with the
/// version installed now and whether it's newer. A release without the
/// feed has none. Asks { op: "fetch" } and { op: "json" }.
pub async fn component_updates(io: &Io, env: &Env, m: &Machine) -> Result<Value, Failure> {
    let res = io.ask(json!({ "op": "fetch", "url": FEED, "timeoutMs": 20000 })).await.map_err(|_| Failure::Rethrow)?;
    if res.get("status").and_then(Value::as_f64) == Some(404.0) {
        return Ok(json!([]));
    }
    if res.get("ok") != Some(&json!(true)) {
        return Err(Failure::msg("XenoNet could not check component updates. Try again later."));
    }
    let manifest = io.ask(json!({ "op": "json", "res": res["res"] })).await.map_err(|_| Failure::Rethrow)?;
    let packages = match (manifest.get("format"), manifest.get("packages")) {
        (Some(f), Some(Value::Array(p))) if f == &json!(1) => p.clone(),
        _ => return Err(Failure::msg("Invalid component update manifest.")),
    };
    let mut out = Vec::new();
    for item in packages.iter().filter(|i| i.get("platform").and_then(Value::as_str) == Some(m.platform.as_str()) && i.get("arch").and_then(Value::as_str) == Some(m.arch.as_str()))
    {
        package_info(Some(item), &m.platform, &m.arch).map_err(Failure::msg)?;
        let file = item.get("file").map_or_else(|| "undefined".into(), js_string);
        if !file_re().is_match(&file) || !size_ok(item.get("size")) || !is_sha(item.get("sha256")) {
            return Err(Failure::msg("Invalid component download."));
        }
        let component = js_string(&item["component"]);
        let current = current_component(&component, env, m)?;
        let available = match &current {
            None => true,
            Some(c) => newer(item.get("version"), c.get("version")).map_err(Failure::msg)?,
        };
        let mut o = item.as_object().cloned().unwrap_or_default();
        o.insert("current".into(), current.as_ref().and_then(|c| c.get("version")).cloned().unwrap_or(Value::Null));
        o.insert("available".into(), json!(available));
        o.insert("url".into(), json!(format!("{DOWNLOADS}{file}")));
        out.push(Value::Object(o));
    }
    Ok(Value::Array(out))
}

fn valid(file: &str, size: f64, sha: &str) -> Result<bool, FsError> {
    let meta = std::fs::metadata(file).map_err(|e| FsError::of(e, "stat", file))?;
    if meta.len() as f64 != size {
        return Ok(false);
    }
    let bytes = std::fs::read(file).map_err(|e| FsError::of(e, "open", file))?;
    Ok(digest(&bytes) == sha)
}

/// Where a component's download goes, after checking where it comes
/// from: its file, and whether a good copy is there already (a bad one is
/// taken away).
pub fn download_plan(item: &Value, env: &Env, m: &Machine) -> Result<(String, bool), FsError> {
    let url = item.get("url").map_or_else(|| "undefined".into(), js_string);
    if !url_re().is_match(&url) {
        return Err(fsx::said("Unexpected component download address."));
    }
    if !is_sha(item.get("sha256")) || !size_ok(item.get("size")) {
        return Err(fsx::said("Invalid component download."));
    }
    let sha = js_string(&item["sha256"]);
    let size = item["size"].as_f64().unwrap_or(0.0);
    let dir = NATIVE.join(&[&runtime_dir(env, m), "downloads"]);
    fsx::mkdir_p(&dir, Some(0o700))?;
    let file = NATIVE.join(&[&dir, &format!("{sha}.tar.gz")]);
    if fsx::exists(&file) && !valid(&file, size, &sha)? {
        fsx::unlink(&file)?;
    }
    let have = fsx::exists(&file);
    Ok((file, have))
}

/// A download under way: written to a temporary file of its own, refused
/// past the size the release says, and kept only when it's whole and its
/// SHA-256 is the release's.
pub struct Download {
    file: String,
    tmp: String,
    out: Option<std::fs::File>,
    size: f64,
    sha256: String,
    got: f64,
}

impl Download {
    pub fn new(file: &str, size: f64, sha256: &str) -> Result<Self, FsError> {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).expect("the system's random numbers");
        let tmp = format!("{file}.{}.tmp", xenonet_base::jsstr::hex(&b));
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let out = o.open(&tmp).map_err(|e| FsError::of(e, "open", &tmp))?;
        Ok(Download { file: file.into(), tmp, out: Some(out), size, sha256: sha256.into(), got: 0.0 })
    }

    pub fn write(&mut self, chunk: &[u8]) -> Result<(), FsError> {
        self.got += chunk.len() as f64;
        if self.got > self.size {
            return Err(fsx::said("Component download exceeds its expected size."));
        }
        let tmp = self.tmp.clone();
        self.out.as_mut().ok_or_else(|| fsx::said("the download is over"))?.write_all(chunk).map_err(|e| FsError::of(e, "write", &tmp))
    }

    /// Checks what came and puts it in place.
    pub fn finish(&mut self) -> Result<String, FsError> {
        if let Some(f) = self.out.take() {
            f.sync_all().ok();
        }
        if !valid(&self.tmp, self.size, &self.sha256)? {
            return Err(fsx::said("Component download failed its hash or size check. Try again."));
        }
        fsx::rename(&self.tmp, &self.file)?;
        Ok(self.file.clone())
    }

    /// Takes the temporary file away, whatever happened.
    pub fn discard(&mut self) {
        self.out.take();
        let _ = fsx::rm_rf(&self.tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: &str = "https://github.com/willctl/xenonet/releases/latest/download/";

    fn place(name: &str) -> (std::path::PathBuf, Env, Machine) {
        let dir = std::env::temp_dir().join(format!("xenonet-updates-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut env = Env::new();
        env.insert("XENONET_RUNTIME_DIR".into(), json!(dir.join("runtime").to_string_lossy()));
        let m = Machine { platform: "linux".into(), arch: "x64".into(), home: dir.to_string_lossy().into_owned() };
        (dir, env, m)
    }

    #[test]
    fn a_download_comes_only_from_the_release_and_says_its_hash_and_size() {
        let (dir, env, m) = place("plan");
        let sha = digest(b"abcd");
        let item = |url: &str, sha: Value, size: Value| json!({ "url": url, "sha256": sha, "size": size });
        for url in [
            "https://github.example/willctl/xenonet/releases/latest/download/XenoNet-tools-1.2.3.tar.gz",
            "http://github.com/willctl/xenonet/releases/latest/download/XenoNet-tools-1.2.3.tar.gz",
            "https://github.com/someone/xenonet/releases/latest/download/XenoNet-tools-1.2.3.tar.gz",
            "https://github.com/willctl/xenonet/releases/latest/download/../XenoNet-tools-1.2.3.tar.gz",
            "https://github.com/willctl/xenonet/releases/latest/download/XenoNet-tools-1.2.3.tar.gz?to=elsewhere",
            "https://github.com/willctl/xenonet/releases/latest/download/XenoNet-tools-1.2.3.tar.gz.exe",
            "https://github.com/willctl/xenonet/releases/latest/download/xenonet.exe",
        ] {
            assert_eq!(download_plan(&item(url, json!(sha), json!(4)), &env, &m).unwrap_err().message, "Unexpected component download address.", "{url}");
        }
        assert_eq!(download_plan(&json!({ "sha256": sha, "size": 4 }), &env, &m).unwrap_err().message, "Unexpected component download address.");
        let url = format!("{AT}XenoNet-tools-1.2.3.tar.gz");
        for (sha, size) in
            [(json!("ABCD"), json!(4)), (json!(null), json!(4)), (json!(sha), json!(0)), (json!(sha), json!(1.5)), (json!(sha), json!("4")), (json!(sha), json!(MAX_PACKAGE + 1))]
        {
            assert_eq!(download_plan(&item(&url, sha.clone(), size.clone()), &env, &m).unwrap_err().message, "Invalid component download.", "{sha} {size}");
        }
        assert!(!dir.join("runtime").exists(), "nothing was made for a download refused");
        // A copy that's there already is kept only when it's whole and its own.
        let (file, have) = download_plan(&item(&url, json!(sha), json!(4)), &env, &m).unwrap();
        assert!(!have && file.ends_with(&format!("{sha}.tar.gz")));
        std::fs::write(&file, "abce").unwrap();
        assert_eq!(download_plan(&item(&url, json!(sha), json!(4)), &env, &m).unwrap(), (file.clone(), false));
        assert!(!std::path::Path::new(&file).exists(), "a bad copy is taken away");
        std::fs::write(&file, "abcd").unwrap();
        assert_eq!(download_plan(&item(&url, json!(sha), json!(4)), &env, &m).unwrap(), (file.clone(), true));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_download_is_refused_past_its_size_and_kept_only_when_its_hash_is_the_releases() {
        let (dir, _, _) = place("download");
        let file = dir.join("p.tar.gz").to_string_lossy().into_owned();
        let sha = digest(b"abcd");
        let tmps = || std::fs::read_dir(&dir).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().ends_with(".tmp")).count();
        // More than it said.
        let mut d = Download::new(&file, 4.0, &sha).unwrap();
        d.write(b"abc").unwrap();
        assert_eq!(d.write(b"de").unwrap_err().message, "Component download exceeds its expected size.");
        d.discard();
        assert_eq!(tmps(), 0);
        // Its size, but not its bytes; or less than it said.
        for got in [&b"abce"[..], b"abc"] {
            let mut d = Download::new(&file, 4.0, &sha).unwrap();
            d.write(got).unwrap();
            assert_eq!(d.finish().unwrap_err().message, "Component download failed its hash or size check. Try again.");
            d.discard();
            assert!(!std::path::Path::new(&file).exists() && tmps() == 0);
        }
        let mut d = Download::new(&file, 4.0, &sha).unwrap();
        d.write(b"ab").unwrap();
        d.write(b"cd").unwrap();
        assert_eq!(d.finish().unwrap(), file);
        assert_eq!(std::fs::read(&file).unwrap(), b"abcd");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
