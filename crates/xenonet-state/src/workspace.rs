//! core/workspace.js: the projects folder XenoNet reads, its root:
//! XENONET_PROJECTS_DIR when it's set, else the folder you chose, else
//! ~/Projects. A folder is checked before XenoNet reads it: it has to exist,
//! be a folder XenoNet can read, and be neither the whole disk nor your home
//! folder (nor a folder holding it), judged by where it leads when it's a
//! link. The config, the store and the scan stay with the twin.

use serde_json::{json, Value};
use std::path::Path;
use xenonet_base::json::number_string;
use xenonet_base::paths::{self, is_under, path_key};
use xenonet_base::text::{js_string, js_trim, truthy};

/// The folder state.json says you chose, read as the file is (nothing moved
/// up, set aside or written); None without one.
pub fn saved_root(file: &str) -> Option<String> {
    let b = std::fs::read(file).ok()?;
    let v = xenonet_base::json::parse(&String::from_utf8_lossy(&b)).ok()?;
    v.get("workspace")?.get("root")?.as_str().map(String::from)
}

fn is_absolute(p: &str) -> bool {
    Path::new(p).is_absolute()
}

// paths.tilde: nothing to do without a home.
fn tilde(text: &str, home: &str) -> String {
    if text.is_empty() || home.is_empty() {
        text.to_string()
    } else {
        paths::tilde(text, home)
    }
}

/// The folder in use and where that comes from: `env_dir` is
/// XENONET_PROJECTS_DIR (as set; the demo has none), `saved` the choice as
/// the store or state.json has it.
pub fn root(env_dir: Option<&str>, saved: Option<&Value>, home: &str) -> Value {
    let env = env_dir.filter(|d| !d.is_empty()).map(paths::resolve);
    // One that isn't a full path (state.json edited by hand) is no choice.
    let chosen = saved.and_then(Value::as_str).filter(|s| is_absolute(s)).map(String::from);
    let fallback = paths::join(home, "Projects");
    let path = env.clone().or_else(|| chosen.clone()).unwrap_or_else(|| fallback.clone());
    let source = if env.is_some() {
        "env"
    } else if chosen.is_some() {
        "chosen"
    } else {
        "default"
    };
    json!({ "path": path, "source": source, "env": env, "chosen": chosen, "default": fallback, "home": home })
}

/// Why the folder can't be chosen: while XENONET_PROJECTS_DIR is set, it
/// decides. None when it isn't.
pub fn locked(env_dir: Option<&str>, home: &str) -> Option<String> {
    let env = paths::resolve(env_dir.filter(|d| !d.is_empty())?);
    Some(format!("XENONET_PROJECTS_DIR is set, so it decides which folder XenoNet reads ({}). To choose one here, unset it and restart XenoNet.", tilde(&env, home)))
}

/// A folder with its links followed, the path sessions record, or as given
/// when it can't be followed (it's gone).
pub fn real(dir: &str) -> String {
    match std::fs::canonicalize(dir) {
        Ok(p) => plain(p.to_string_lossy().into_owned()),
        Err(_) => paths::resolve(dir),
    }
}

// A canonical path as Node's realpath gives it: on Windows, without the
// \\?\ that Rust's canonicalize puts in front.
fn plain(p: String) -> String {
    if let Some(unc) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    p.strip_prefix(r"\\?\").map_or(p.clone(), String::from)
}

/// What choose() saves: the folder it leads to, or None for none or the
/// default folder itself, which stays the default.
pub fn chosen(dir: Option<&str>, default: &str) -> Option<String> {
    let dir = dir.filter(|d| !d.is_empty())?;
    let r = real(dir);
    (path_key(&r) != path_key(&real(default))).then_some(r)
}

/// A folder as someone typed it: ~ and ~/… are the home folder; anything
/// else has to be a full path. None when it isn't one.
pub fn resolve_folder(input: Option<&Value>, home: &str) -> Option<String> {
    let s = input.and_then(Value::as_str).map_or("", js_trim);
    if s == "~" {
        return Some(paths::resolve(home));
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        return Some(paths::resolve(&paths::join(home, rest)));
    }
    (!s.is_empty() && is_absolute(s)).then(|| paths::resolve(s))
}

fn privacy(shown: &str, platform: &str) -> String {
    if platform == "darwin" {
        " Let XenoNet read it in System Settings › Privacy & Security › Files and Folders (or Full Disk Access), then try again.".into()
    } else {
        format!(" Check the permissions on {shown}.")
    }
}

fn code_of(e: &std::io::Error) -> String {
    xenonet_base::fs::code_of(e).map_or_else(|| "undefined".into(), String::from)
}

/// Whether XenoNet can read a folder as its projects folder: { ok, path,
/// link, error }, with the folder's entries (`names`) when it can.
pub fn validate(input: Option<&Value>, home: &str, platform: &str) -> Value {
    let dir = resolve_folder(input, home);
    let no = |error: String| json!({ "ok": false, "path": dir, "link": null, "error": error });
    let Some(d) = dir.clone() else { return no("Give the folder's full path, or start it with ~.".into()) };
    let shown = tilde(&d, home);
    let st = match std::fs::metadata(&d) {
        Ok(m) => Some(m),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound) || xenonet_base::fs::code_of(&e) == Some("ENOTDIR") => None,
        Err(e) => return no(format!("XenoNet can't reach {shown} ({}).{}", code_of(&e), privacy(&shown, platform))),
    };
    let Some(st) = st else { return no(format!("{shown} doesn't exist.")) };
    let target = match std::fs::canonicalize(&d) {
        Ok(t) => plain(t.to_string_lossy().into_owned()),
        Err(e) => return no(format!("XenoNet can't reach {shown} ({}).{}", code_of(&e), privacy(&shown, platform))),
    };
    if !st.is_dir() {
        return no(format!("{shown} isn't a folder."));
    }
    let link = path_key(&target) != path_key(&d);
    let that = |what: &str| if link { format!("{shown} is a link to {what}.") } else { format!("That's {what}.") };
    if Path::new(&target).parent().is_none() {
        return no(format!("{} Choose the folder that holds your projects.", that("the whole disk")));
    }
    let key = path_key(&target);
    let home_key = path_key(&real(home));
    if key == home_key {
        return no(format!("{} Choose the folder in it that holds your projects, such as ~/Projects.", that("your home folder")));
    }
    if let (Some(h), Some(k)) = (&home_key, &key) {
        if is_under(h, k) {
            let who = if link { format!("{shown} is a link to {}, which", tilde(&target, home)) } else { shown.clone() };
            return no(format!("{who} holds your home folder. Choose the folder that holds your projects."));
        }
    }
    let names: Vec<String> = match paths::read_dir_sorted(Path::new(&target)) {
        Ok(entries) => entries.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect(),
        Err(e) => return no(format!("XenoNet can't read {shown} ({}).{}", code_of(&e), privacy(&shown, platform))),
    };
    json!({ "ok": true, "path": target, "link": if link { json!(d) } else { Value::Null }, "error": null, "names": names })
}

/// The first half of check(): the folder's validation, whether it's the
/// one in use, and the warnings found before scanning. A folder XenoNet
/// can't use is the whole answer (`done`).
pub fn check_start(input: Option<&Value>, home: &str, projects_dir: &str, platform: &str, vcs_markers: &[&str]) -> Value {
    let v = validate(input, home, platform);
    let ok = v["ok"] == true;
    let path = v["path"].as_str().map(String::from);
    let current = path.as_deref().is_some_and(|p| path_key(&if ok { p.to_string() } else { real(p) }) == path_key(&real(projects_dir)));
    if !ok {
        return json!({ "done": { "ok": false, "path": v["path"], "link": null, "error": v["error"], "current": current, "warnings": [], "scan": null } });
    }
    let path = path.expect("validate gives a path with ok");
    let shown = tilde(&path, home);
    let mut warnings = Vec::new();
    if let Some(l) = v["link"].as_str() {
        warnings.push(format!("{} is a link, so XenoNet would read the folder it leads to, {shown}.", tilde(l, home)));
    }
    let names: Vec<&str> = v["names"].as_array().map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).collect());
    if vcs_markers.iter().any(|m| names.contains(m)) {
        warnings.push(format!("{shown} is a repo itself, so XenoNet would show the folders in it as projects. Choose the folder that holds your repos."));
    }
    json!({ "path": path, "link": v["link"], "shown": shown, "current": current, "warnings": warnings })
}

/// The answer of check(), from its first half and the scan's figures
/// (None when it took longer than `time_ms`).
pub fn check_finish(start: &Value, sc: Option<&Value>, time_ms: f64) -> Value {
    let shown = start["shown"].as_str().unwrap_or("");
    let sc = sc.filter(|s| truthy(Some(s)));
    if let Some(s) = sc {
        if !truthy(s.get("ok")) {
            return json!({ "ok": false, "path": start["path"], "link": start["link"], "error": format!("XenoNet can't read {shown}."), "current": start["current"], "warnings": [], "scan": null });
        }
    }
    let mut warnings: Vec<Value> = start["warnings"].as_array().cloned().unwrap_or_default();
    match sc {
        None => warnings.push(json!(format!(
            "Scanning {shown} took more than {} s: it's big, or slow to read, and XenoNet reads it again every few seconds.",
            number_string(time_ms / 1000.0)
        ))),
        Some(s) if !truthy(s.get("projects")) && !truthy(s.get("groups")) => warnings.push(json!(format!("XenoNet finds no projects in {shown}: no repos, and no folders in it."))),
        _ => {}
    }
    if let Some(s) = sc.filter(|s| truthy(s.get("truncated"))) {
        warnings.push(json!(format!(
            "The scan stopped at its {}-folder limit, so some projects may be missing. XENONET_PROJECTS_MAX_DIRS raises it.",
            s.get("maxDirs").map_or_else(|| "undefined".into(), js_string)
        )));
    }
    let scan = sc.map_or(Value::Null, |s| {
        let mut m = serde_json::Map::new();
        for k in ["projects", "groups", "dirsRead", "truncated", "depth", "maxDirs", "ms"] {
            if let Some(v) = s.get(k) {
                m.insert(k.into(), v.clone());
            }
        }
        Value::Object(m)
    });
    json!({ "ok": true, "path": start["path"], "link": start["link"], "error": null, "current": start["current"], "warnings": warnings, "scan": scan })
}

/// Which variable says an instance's state or cache folder.
pub fn instance_dir_var(kind: &str) -> &'static str {
    if kind == "state" {
        "XENONET_STATE_DIR"
    } else {
        "XENONET_CACHE_DIR"
    }
}

/// The instance a command that finds no XenoNet running stands for: XenoNet
/// Dev, when it's the one asked about.
pub fn stands_for(asked: &[Value], dev_port: &Value, dev_name: &str) -> Option<String> {
    (asked.len() == 1 && &asked[0] == dev_port).then(|| dev_name.to_string())
}

// Spelled as on a unix host, where Node's path module is posix; on Windows
// the JS tests run against the Rust (test:rust) check the same. (Two
// attributes: clippy takes only a plain cfg(test) for test code.)
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn folders_as_typed() {
        assert_eq!(resolve_folder(Some(&json!("~")), "/home/me").as_deref(), Some("/home/me"));
        assert_eq!(resolve_folder(Some(&json!("~/code/")), "/home/me").as_deref(), Some("/home/me/code"));
        assert_eq!(resolve_folder(Some(&json!("  /a/b/../c  ")), "/home/me").as_deref(), Some("/a/c"));
        for no in [json!(""), json!("code"), json!("~nobody/x"), json!(null), json!(42)] {
            assert_eq!(resolve_folder(Some(&no), "/home/me"), None);
        }
        let r = root(None, Some(&json!("code")), "/home/me");
        assert_eq!((r["path"].clone(), r["source"].clone()), (json!("/home/me/Projects"), json!("default")));
        assert!(locked(Some("/x"), "/home/me").unwrap().contains("(/x)"));
        assert_eq!(validate(Some(&json!("/")), "/home/me", "linux")["error"], "That's the whole disk. Choose the folder that holds your projects.");
    }
}
