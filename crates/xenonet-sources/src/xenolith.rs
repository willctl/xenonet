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

/// Which server a wiki folder belongs to, by the protocol it was seeded
/// with: Xenolith's says "# Xenolith protocol" near its top. Anything else
/// is Agent Wiki's, as XenoNet always read it.
pub fn context_server_of(dir: &str) -> &'static str {
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

/// The environment from a JS object of strings.
pub fn env_of(v: &Value) -> Env {
    v.as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default()
}
