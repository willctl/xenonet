//! core/xenolith.js: where the shared context's folder is (Xenolith's,
//! else Agent Wiki's), and which MCP server writes to it. XenoNet only
//! reads the folder, by the format Xenolith documents
//! (xenolith/FORMAT.md).

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use xenonet_base::paths;

/// The MCP servers whose calls write to the shared context, by the names
/// Xenolith and Agent Wiki install them under. A folder's marker can name
/// another (see `read_marker`), which the config then names (wikiServer).
pub const CONTEXT_SERVERS: [&str; 2] = ["xenolith", "agent-wiki"];

pub fn is_context_server(server: &str) -> bool {
    CONTEXT_SERVERS.contains(&server)
}

/// Xenolith's tools. Under the server the config names for the shared
/// context (wikiServer), a call to one of them goes to the shared context
/// whatever that server is called.
pub const CONTEXT_TOOLS: [&str; 5] = ["context_start", "context_search", "context_read", "context_log", "context_upsert_page"];

/// Whether an MCP call goes to the shared context: any call to a server of
/// a known name, or a call to one of Xenolith's tools under `named`.
pub fn is_context_call(server: &str, tool: Option<&str>, named: Option<&str>) -> bool {
    is_context_server(server) || (named.is_some_and(|n| !n.is_empty() && n == server) && tool.is_some_and(|t| CONTEXT_TOOLS.contains(&t)))
}

/// The version of Xenolith's folder format XenoNet reads.
pub const CONTEXT_FORMAT: u64 = 1;
/// The marker at the folder's root that says which version a folder
/// follows and which MCP server writes it. Agent Wiki's folders, and
/// Xenolith's from before the marker, have none.
pub const MARKER_FILE: &str = "xenolith.json";
// The marker is a line of JSON; anything much bigger isn't one.
const MARKER_MAX: u64 = 64 * 1024;

/// What a folder's marker says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub format: u64,
    /// None when it isn't a server's name: the folder's server is then
    /// found as if it had no marker.
    pub server: Option<String>,
}

// /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/: a name an MCP server goes by in a
// session's tool names (mcp__<server>__<tool>) and on the map
// (resource:<server>).
fn server_name(s: &str) -> bool {
    let b = s.as_bytes();
    b.first().is_some_and(u8::is_ascii_alphanumeric) && b.len() <= 64 && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// What a folder's marker says. None when there's no marker, or one
/// XenoNet can't take: not a file, not a JSON object, or without a format
/// that's a whole number from 1 (Number.isSafeInteger in JS).
pub fn read_marker(dir: &str) -> Option<Marker> {
    let file = paths::join(dir, MARKER_FILE);
    let meta = std::fs::symlink_metadata(&file).ok()?;
    if !meta.is_file() || meta.len() > MARKER_MAX {
        return None;
    }
    let text = String::from_utf8_lossy(&std::fs::read(&file).ok()?).into_owned();
    let v = xenonet_base::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()?;
    let m = v.as_object()?;
    let format = m.get("format").and_then(Value::as_i64).filter(|n| (1..=9_007_199_254_740_991).contains(n))?;
    let server = m.get("server").and_then(Value::as_str).filter(|s| server_name(s)).map(String::from);
    Some(Marker { format: format.unsigned_abs(), server })
}

/// Why XenoNet can't read a folder whose marker gives this format, in a
/// sentence for the map, or None when it can. A folder in a later format
/// than XenoNet knows may have changed in ways it would misread, so
/// XenoNet doesn't guess at it.
pub fn format_problem(format: u64) -> Option<String> {
    (format > CONTEXT_FORMAT).then(|| {
        format!("This folder follows version {format} of Xenolith's format, and this XenoNet reads version {CONTEXT_FORMAT}, so it doesn't show its pages, log or inbox. Update XenoNet to see them.")
    })
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
    /// It makes the folder `<ENV>_DIR` or its config's wikiDir names when it
    /// starts, so XenoNet takes that folder before it's there
    /// (xenolith/FORMAT.md, "Finding the folder").
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
/// ~/.agent-wiki) counts while its new one isn't there. The server is the
/// one the folder's marker names, else the one whose lookup found it.
pub fn find_context_dir(env: &Env, home: &str, platform: &str) -> Option<(String, String)> {
    let found = |dir: String, server: &str| {
        let server = read_marker(&dir).and_then(|m| m.server).unwrap_or_else(|| server.to_string());
        Some((dir, server))
    };
    for p in &PRODUCTS {
        if let Some(dir) = var(env, &format!("{}_DIR", p.env)).filter(|d| p.makes_dir || exists(d)) {
            return found(paths::resolve(dir), p.server);
        }
        if let Some(dir) = named_in(&config_file(p, env, home, platform)).filter(|d| p.makes_dir || exists(d)) {
            return found(dir, p.server);
        }
        let fallback = paths::join(home, p.folder);
        if exists(&fallback) {
            return found(fallback, p.server);
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
/// say): the one its marker names. Without a marker, the product whose
/// own lookup names that folder (its `<ENV>_DIR`, its config's wikiDir or
/// its folder in home), so a wiki that moved keeps its server. Only for a
/// folder neither names does the protocol it was seeded with decide, since
/// people edit it: Xenolith's says "# Xenolith protocol" near its top.
/// Anything else is Agent Wiki's, as XenoNet always read it.
pub fn context_server_of(dir: &str, env: &Env, home: &str, platform: &str) -> String {
    if let Some(server) = read_marker(dir).and_then(|m| m.server) {
        return server;
    }
    guessed_server_of(dir, env, home, platform).to_string()
}

// The server of a folder without a marker that names one.
fn guessed_server_of(dir: &str, env: &Env, home: &str, platform: &str) -> &'static str {
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

/// The shared context's name on the map. Agent Wiki's server is Agent
/// Wiki's; any other is Xenolith's, under the name it installs itself as
/// or the one its folder's marker gives.
pub fn context_name(server: Option<&str>) -> &'static str {
    match server {
        Some(s) if !s.is_empty() && s != "agent-wiki" => "Xenolith",
        _ => "Agent Wiki",
    }
}

/// Xenolith's spaces (xenolith/rust/crates/core/src/spaces.rs): one wiki
/// each, for a project, an identity, an actor or a machine. The wiki's own
/// folder is this machine's, machine:local; every other is laid out the
/// same way in `spaces/<kind>/<name>/` under it.
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
    fn at(dir: &str, server: &str) -> (String, String) {
        (dir.to_string(), server.to_string())
    }

    #[test]
    fn xenoliths_config_where_xenolith_looks() {
        let h = home("find");
        let home = h.0.to_str().unwrap();
        let find = |e: &[(&str, &str)]| find_context_dir(&env(e), home, "linux");
        let later = format!("{home}/not-yet");
        assert_eq!(find(&[("XENOLITH_DIR", &later)]), Some(at(&later, "xenolith")), "before Xenolith made it");
        assert_eq!(find(&[("AGENT_WIKI_DIR", &later)]), None, "Agent Wiki's must be there");
        let legacy = format!("{home}/legacy");
        fs::create_dir_all(&legacy).unwrap();
        config(&h.0.join(".xenolith"), &legacy);
        assert_eq!(find(&[]), Some(at(&legacy, "xenolith")), "the old config in home");
        assert_eq!(find(&[("XENOLITH_CONFIG_DIR", "")]), None, "not with XENOLITH_CONFIG_DIR set");
        let cfg = h.0.join("cfg");
        let cfg_wiki = format!("{home}/cfg-wiki");
        fs::create_dir_all(&cfg_wiki).unwrap();
        config(&cfg, &cfg_wiki);
        let portable = h.0.join("portable");
        config(&portable, &legacy);
        assert_eq!(
            find(&[("XENOLITH_HOME", portable.to_str().unwrap()), ("XENOLITH_CONFIG_DIR", cfg.to_str().unwrap())]),
            Some(at(&cfg_wiki, "xenolith")),
            "the config folder wins"
        );
        let not_yet = format!("{home}/not-yet-from-config");
        config(&h.0.join(".config/xenolith"), &not_yet);
        assert_eq!(find(&[]), Some(at(&not_yet, "xenolith")), "its config's wikiDir, before Xenolith made it");
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

    #[test]
    fn the_marker_names_the_folders_server_and_format() {
        let h = home("marker");
        let home = h.0.to_str().unwrap();
        let wiki = h.0.join("Xenolith");
        fs::create_dir_all(&wiki).unwrap();
        let dir = wiki.to_str().unwrap();
        let mark = |text: &str| fs::write(wiki.join(MARKER_FILE), text).unwrap();
        let find = || find_context_dir(&env(&[]), home, "linux");
        let of = |d: &str| context_server_of(d, &env(&[]), home, "linux");
        assert_eq!((read_marker(dir), find()), (None, Some(at(dir, "xenolith"))), "no marker: the lookup's");

        mark("\u{feff}{\n  \"format\": 1,\n  \"server\": \"ctx\"\n}\n");
        assert_eq!(read_marker(dir), Some(Marker { format: 1, server: Some("ctx".into()) }));
        assert_eq!((find(), of(dir)), (Some(at(dir, "ctx")), "ctx".to_string()), "the marker's, before the lookup's");
        assert_eq!(context_name(Some("ctx")), "Xenolith");

        // A folder no lookup names, its protocol Agent Wiki's: the marker
        // decides, and without one the protocol does.
        let aside = h.0.join("aside");
        fs::create_dir_all(&aside).unwrap();
        fs::write(aside.join("PROTOCOL.md"), "# Agent Wiki protocol\n").unwrap();
        let aside_dir = aside.to_str().unwrap();
        assert_eq!(of(aside_dir), "agent-wiki");
        fs::write(aside.join(MARKER_FILE), "{\"format\":1,\"server\":\"xenolith\"}").unwrap();
        assert_eq!(of(aside_dir), "xenolith");

        // A server that isn't a name: the lookup's. The format still counts.
        mark("{\"format\":2,\"server\":\"my context\"}");
        assert_eq!(read_marker(dir), Some(Marker { format: 2, server: None }));
        assert_eq!(find(), Some(at(dir, "xenolith")));

        // Markers XenoNet can't take: as if there were none.
        for bad in [
            "{",
            "[]",
            "null",
            "{\"server\":\"ctx\"}",
            "{\"format\":\"1\",\"server\":\"ctx\"}",
            "{\"format\":0,\"server\":\"ctx\"}",
            "{\"format\":1.5,\"server\":\"ctx\"}",
            "{\"format\":9007199254740992,\"server\":\"ctx\"}",
        ] {
            mark(bad);
            assert_eq!((read_marker(dir), of(dir)), (None, "xenolith".to_string()), "{bad}");
        }
        fs::remove_file(wiki.join(MARKER_FILE)).unwrap();
        fs::create_dir(wiki.join(MARKER_FILE)).unwrap();
        assert_eq!(read_marker(dir), None, "a folder");
        fs::remove_dir(wiki.join(MARKER_FILE)).unwrap();
        let elsewhere = h.0.join("elsewhere.json");
        fs::write(&elsewhere, "{\"format\":1,\"server\":\"ctx\"}").unwrap();
        std::os::unix::fs::symlink(&elsewhere, wiki.join(MARKER_FILE)).unwrap();
        assert_eq!(read_marker(dir), None, "a link");
    }

    #[test]
    fn a_later_format_is_not_read() {
        assert_eq!(format_problem(CONTEXT_FORMAT), None);
        let problem = format_problem(2).unwrap();
        assert!(problem.contains("version 2") && problem.contains("reads version 1"), "{problem}");
        assert_eq!(
            [context_name(Some("xenolith")), context_name(Some("agent-wiki")), context_name(Some("")), context_name(None)],
            ["Xenolith", "Agent Wiki", "Agent Wiki", "Agent Wiki"]
        );
    }

    #[test]
    fn xenoliths_tools_count_under_the_server_the_config_names() {
        assert!(is_context_call("xenolith", Some("anything"), None), "a known name, any tool");
        assert!(is_context_call("agent-wiki", Some("wiki_log"), Some("ctx")));
        for tool in CONTEXT_TOOLS {
            assert!(is_context_call("ctx", Some(tool), Some("ctx")), "{tool}");
        }
        assert!(!is_context_call("ctx", Some("context_log"), None), "no name in the config");
        assert!(!is_context_call("ctx", Some("context_log"), Some("")));
        assert!(!is_context_call("ctx", Some("write_page"), Some("ctx")), "another of its tools");
        assert!(!is_context_call("ctx", None, Some("ctx")));
        assert!(!is_context_call("github", Some("context_log"), Some("ctx")), "another server");
    }
}
