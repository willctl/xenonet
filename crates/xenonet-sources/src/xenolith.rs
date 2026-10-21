//! core/xenolith.js: where the shared context's wiki is (Xenolith's, else
//! Agent Wiki's), and which MCP server writes to it. XenoNet only reads the
//! folder.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use xenonet_base::paths;

/// The MCP servers whose calls write to the shared context.
pub const CONTEXT_SERVERS: [&str; 2] = ["xenolith", "agent-wiki"];

pub fn is_context_server(server: &str) -> bool {
    CONTEXT_SERVERS.contains(&server)
}

/// The environment, as a test or the process gives it.
pub type Env = BTreeMap<String, String>;

struct Product {
    server: &'static str,
    env: &'static str,
    windows: &'static str,
    mac: &'static str,
    xdg: &'static str,
    folder: &'static str,
    legacy: &'static str,
    /// It makes the folder <ENV>_DIR names when it starts, so XenoNet takes
    /// that folder before it's there.
    makes_dir: bool,
}

const PRODUCTS: [Product; 2] = [
    Product { server: "xenolith", env: "XENOLITH", windows: "Xenolith", mac: "Xenolith", xdg: "xenolith", folder: "Xenolith", legacy: ".xenolith", makes_dir: true },
    Product { server: "agent-wiki", env: "AGENT_WIKI", windows: "AgentWiki", mac: "Agent Wiki", xdg: "agent-wiki", folder: "AgentWiki", legacy: ".agent-wiki", makes_dir: false },
];

fn var<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.get(name).map(String::as_str).filter(|v| !v.is_empty())
}

fn is_absolute(p: &str) -> bool {
    Path::new(p).is_absolute()
}

// A product's config folder, as its programs find it
// (xenolith/rust/crates/core/src/paths.rs): <ENV>_CONFIG_DIR, then
// <ENV>_HOME (portable mode, everything in one folder), then the
// platform's own place.
fn config_dir(p: &Product, env: &Env, home: &str, platform: &str) -> String {
    if let Some(dir) = var(env, &format!("{}_CONFIG_DIR", p.env)) {
        return paths::resolve(dir);
    }
    if let Some(portable) = var(env, &format!("{}_HOME", p.env)) {
        return paths::resolve(portable);
    }
    if platform == "win32" {
        let appdata = var(env, "APPDATA").map_or_else(|| paths::join_all(&[home, "AppData", "Roaming"]), String::from);
        return paths::join(&appdata, p.windows);
    }
    let xdg = var(env, "XDG_CONFIG_HOME").filter(|v| is_absolute(v));
    if platform == "darwin" {
        return match xdg {
            Some(x) => paths::join(x, p.xdg),
            None => paths::join_all(&[home, "Library", "Application Support", p.mac]),
        };
    }
    paths::join(&xdg.map_or_else(|| paths::join(home, ".config"), String::from), p.xdg)
}

fn exists(p: &str) -> bool {
    Path::new(p).exists()
}

// The config.json a product reads (Xenolith's config_path in
// xenolith/rust/crates/core/src/wiki.rs): the one in its config folder,
// or, until its installer has moved it there, the one in its old folder in
// home. Never the old one in portable mode or with <ENV>_CONFIG_DIR set.
fn config_file(p: &Product, env: &Env, home: &str, platform: &str) -> String {
    let file = paths::join(&config_dir(p, env, home, platform), "config.json");
    if var(env, &format!("{}_HOME", p.env)).is_none() && !env.contains_key(&format!("{}_CONFIG_DIR", p.env)) && !exists(&file) {
        let legacy = paths::join_all(&[home, p.legacy, "config.json"]);
        if exists(&legacy) {
            return legacy;
        }
    }
    file
}

// The wikiDir a config file names, there or not.
fn named_in(file: &str) -> Option<String> {
    let text = String::from_utf8_lossy(&std::fs::read(file).ok()?).into_owned();
    let v = xenonet_base::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()?;
    let dir = v.get("wikiDir")?.as_str()?;
    (!xenonet_base::text::js_trim(dir).is_empty()).then(|| dir.to_string())
}

/// The shared context's folder and the server that owns it: Xenolith's
/// (XENOLITH_DIR, its config.json's wikiDir, ~/Xenolith), else Agent
/// Wiki's the same way. Each product's old config in home (~/.xenolith,
/// ~/.agent-wiki) counts while its new one isn't there.
pub fn find_context_dir(env: &Env, home: &str, platform: &str) -> Option<(String, &'static str)> {
    for p in &PRODUCTS {
        if let Some(dir) = var(env, &format!("{}_DIR", p.env)).filter(|d| p.makes_dir || exists(d)) {
            return Some((paths::resolve(dir), p.server));
        }
        if let Some(dir) = named_in(&config_file(p, env, home, platform)).filter(|d| exists(d)) {
            return Some((dir, p.server));
        }
        let fallback = paths::join(home, p.folder);
        if exists(&fallback) {
            return Some((fallback, p.server));
        }
    }
    None
}

// Two spellings of one folder: the same once resolved, or once their links
// are followed.
fn same_dir(a: &str, b: &str) -> bool {
    if paths::resolve(a) == paths::resolve(b) {
        return true;
    }
    matches!((xenonet_base::fs::realpath_native(a), xenonet_base::fs::realpath_native(b)), (Ok(ra), Ok(rb)) if ra == rb)
}

/// Which server a wiki folder belongs to (one XENONET_WIKI_DIR names,
/// say): the product whose own lookup names that folder (its <ENV>_DIR,
/// its config's wikiDir or its folder in home), so a wiki that moved keeps
/// its server. Only for a folder neither names does the protocol it was
/// seeded with decide, since people edit it: Xenolith's says "# Xenolith
/// protocol" near its top. Anything else is Agent Wiki's, as XenoNet
/// always read it.
pub fn context_server_of(dir: &str, env: &Env, home: &str, platform: &str) -> &'static str {
    for p in &PRODUCTS {
        let named = [var(env, &format!("{}_DIR", p.env)).map(String::from), named_in(&config_file(p, env, home, platform)), Some(paths::join(home, p.folder))];
        if named.iter().flatten().any(|d| same_dir(d, dir)) {
            return p.server;
        }
    }
    if let Ok(bytes) = std::fs::read(paths::join(dir, "PROTOCOL.md")) {
        let head = xenonet_base::jsstr::head(&String::from_utf8_lossy(&bytes), 200);
        // /^#\s*Xenolith protocol/m: a "#" at a line's start, then any
        // white space (line breaks too), then the words.
        let starts = std::iter::once(0).chain(head.char_indices().filter(|(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')).map(|(i, c)| i + c.len_utf8()));
        let protocol =
            starts.collect::<Vec<_>>().into_iter().any(|i| head[i..].strip_prefix('#').is_some_and(|r| xenonet_base::text::js_trim_start(r).starts_with("Xenolith protocol")));
        if protocol {
            return "xenolith";
        }
    }
    "agent-wiki"
}

pub fn context_name(server: Option<&str>) -> &'static str {
    if server == Some("xenolith") {
        "Xenolith"
    } else {
        "Agent Wiki"
    }
}

/// Xenolith's spaces (xenolith/rust/crates/core/src/spaces.rs): one wiki
/// each, for a project, an identity, an actor or a machine. The wiki's own
/// folder is this machine's, machine:local; every other is laid out the
/// same way in spaces/<kind>/<name>/ under it.
pub const SPACE_KINDS: [&str; 4] = ["project", "identity", "actor", "machine"];

/// The space a context_* call works in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Space {
    /// This machine's own, the wiki's folder (also when none is given).
    Local,
    /// Another, by its id (project:atlas).
    Other(String),
    /// One Xenolith refuses, so that the call wrote nothing.
    Refused,
}

// /^[a-z0-9][a-z0-9._-]{0,63}$/, and no "..".
fn space_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.first().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.len() <= 64
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
        && !name.contains("..")
}

/// The space a context_* call works in, from its `space` as Xenolith's
/// server reads it (SpaceId::parse).
pub fn space_of(v: Option<&Value>) -> Space {
    let Some(text) = v.and_then(Value::as_str).filter(|s| !xenonet_base::text::js_trim(s).is_empty()) else { return Space::Local };
    let t = xenonet_base::text::js_trim(text).to_lowercase();
    let Some((kind, name)) = t.split_once(':') else { return Space::Refused };
    if !SPACE_KINDS.contains(&kind) || !space_name(name) {
        return Space::Refused;
    }
    if kind == "machine" && name == "local" {
        Space::Local
    } else {
        Space::Other(format!("{kind}:{name}"))
    }
}

/// The environment from a JS object of strings.
pub fn env_of(v: &Value) -> Env {
    v.as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default()
}

// Spelled as on a unix host, as Node's posix path module would.
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::fs;

    struct Home(std::path::PathBuf);
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn home(tag: &str) -> Home {
        let dir = std::env::temp_dir().join(format!("xenonet-xenolith-rs-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Home(fs::canonicalize(&dir).unwrap())
    }
    fn env(pairs: &[(&str, &str)]) -> Env {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }
    fn config(dir: &Path, wiki: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("config.json"), format!("{{\"wikiDir\":{}}}", serde_json::to_string(wiki).unwrap())).unwrap();
    }

    #[test]
    fn xenoliths_config_where_xenolith_looks() {
        let h = home("find");
        let home = h.0.to_str().unwrap();
        let find = |e: &[(&str, &str)]| find_context_dir(&env(e), home, "linux");
        let later = format!("{home}/not-yet");
        assert_eq!(find(&[("XENOLITH_DIR", &later)]), Some((later.clone(), "xenolith")), "before Xenolith made it");
        assert_eq!(find(&[("AGENT_WIKI_DIR", &later)]), None, "Agent Wiki's must be there");
        let legacy = format!("{home}/legacy");
        fs::create_dir_all(&legacy).unwrap();
        config(&h.0.join(".xenolith"), &legacy);
        assert_eq!(find(&[]), Some((legacy.clone(), "xenolith")), "the old config in home");
        assert_eq!(find(&[("XENOLITH_CONFIG_DIR", "")]), None, "not with XENOLITH_CONFIG_DIR set");
        let cfg = h.0.join("cfg");
        let cfg_wiki = format!("{home}/cfg-wiki");
        fs::create_dir_all(&cfg_wiki).unwrap();
        config(&cfg, &cfg_wiki);
        let portable = h.0.join("portable");
        config(&portable, &legacy);
        assert_eq!(find(&[("XENOLITH_HOME", portable.to_str().unwrap()), ("XENOLITH_CONFIG_DIR", cfg.to_str().unwrap())]), Some((cfg_wiki, "xenolith")), "the config folder wins");
    }

    #[test]
    fn the_server_is_the_one_whose_lookup_names_the_folder() {
        let h = home("server");
        let home = h.0.to_str().unwrap();
        let moved = h.0.join("notes");
        fs::create_dir_all(&moved).unwrap();
        fs::write(moved.join("PROTOCOL.md"), "# Our notes\n").unwrap();
        let of = |dir: &str, e: &[(&str, &str)]| context_server_of(dir, &env(e), home, "linux");
        let moved = moved.to_str().unwrap();
        assert_eq!(of(moved, &[]), "agent-wiki", "the protocol, as a last resort");
        config(&h.0.join(".config/xenolith"), moved);
        assert_eq!(of(moved, &[]), "xenolith");
        assert_eq!(of(&format!("{moved}/"), &[]), "xenolith", "however it's spelled");
        let aside = format!("{home}/aside");
        assert_eq!(of(&aside, &[("XENOLITH_DIR", &aside)]), "xenolith");
        assert_eq!(of(&format!("{home}/AgentWiki"), &[]), "agent-wiki");
        let link = h.0.join("link");
        std::os::unix::fs::symlink(moved, &link).unwrap();
        assert_eq!(of(link.to_str().unwrap(), &[]), "xenolith", "a link to it");
    }
}
