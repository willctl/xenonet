//! core/assistant/tunnel-client.js: OpenAI's tunnel-client, as XenoNet
//! finds and drives it for a remote actor's connection: where it is,
//! which version, the profiles you made for it already, and the profile
//! XenoNet writes for a grant. XenoNet never downloads it.
//!
//! A profile names the tunnel (its id isn't a secret), where the runtime
//! key comes from (env:CONTROL_PLANE_API_KEY, set only in the client's own
//! environment), and the MCP command the tunnel forwards to.

use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::Style;
use xenonet_base::text::js_string;

pub const RELEASES: &str = "https://github.com/openai/tunnel-client/releases";
pub const TUNNELS_PAGE: &str = "https://platform.openai.com/settings/organization/tunnels";

/// /^tunnel_[0-9a-f]{16,64}$/
pub fn is_tunnel_id(s: &str) -> bool {
    s.strip_prefix("tunnel_").is_some_and(|h| (16..=64).contains(&h.len()) && h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
}

/// A runtime API key for the tunnel: /^sk-[A-Za-z0-9_-]{20,400}$/.
pub fn is_runtime_key(s: &str) -> bool {
    s.strip_prefix("sk-").is_some_and(|k| (20..=400).contains(&k.len()) && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'))
}

fn exe(platform: &str) -> &'static str {
    if platform == "win32" {
        "tunnel-client.exe"
    } else {
        "tunnel-client"
    }
}

fn style(platform: &str) -> Style {
    if platform == "win32" {
        Style::Win32
    } else {
        Style::Posix
    }
}

fn var<'a>(env: &'a Map<String, Value>, k: &str) -> Option<&'a Value> {
    env.get(k).filter(|v| !v.is_null())
}

/// `home` as the JS defaults it: HOME, then USERPROFILE, then "".
pub fn home_of(env: &Map<String, Value>, home: Option<&str>) -> String {
    home.map(String::from).or_else(|| var(env, "HOME").or_else(|| var(env, "USERPROFILE")).map(js_string)).unwrap_or_default()
}

/// Where an install of tunnel-client puts it, in the system's own style.
pub fn client_places(env: &Map<String, Value>, platform: &str, home: &str) -> Vec<String> {
    let exe = exe(platform);
    let p = style(platform);
    if platform == "win32" {
        let local = var(env, "LOCALAPPDATA").map_or_else(|| p.join(&[home, "AppData", "Local"]), js_string);
        return vec![p.join(&[&local, "Programs", "tunnel-client", exe]), p.join(&[&local, "tunnel-client", exe])];
    }
    ["/opt/homebrew/bin".to_string(), "/usr/local/bin".to_string(), p.join(&[home, ".local", "bin"]), p.join(&[home, "bin"])].iter().map(|d| p.join(&[d, exe])).collect()
}

/// The tunnel-client to run, or None: XENONET_TUNNEL_CLIENT, then PATH,
/// then where installs put it. `exists(f)` says whether a file is there
/// (Err for one it couldn't look at).
pub fn find_client(env: &Map<String, Value>, platform: &str, home: &str, exists: &mut dyn FnMut(&str) -> Result<bool, ()>) -> Option<String> {
    let mut ok = |f: &str| !f.is_empty() && exists(f).unwrap_or(false);
    if let Some(f) = var(env, "XENONET_TUNNEL_CLIENT").filter(|v| truthy(Some(v))).map(js_string) {
        if ok(&f) {
            return Some(f);
        }
    }
    let path = var(env, "PATH").map(js_string).unwrap_or_default();
    let pathext = var(env, "PATHEXT").map(js_string);
    if let Some(f) = xenonet_base::paths::on_path("tunnel-client", &path, pathext.as_deref(), platform == "win32", &mut |f| ok(f)) {
        return Some(f);
    }
    client_places(env, platform, home).into_iter().find(|f| ok(f))
}

/// The version `tunnel-client --version` printed, such as 0.0.16.
pub fn version_in(stdout: &str) -> Option<String> {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(\d+\.\d+\.\d+)").expect("a fixed pattern")).captures(stdout).map(|c| c[1].to_string())
}

/// Where tunnel-client keeps the profiles you made with it.
pub fn profile_dirs(env: &Map<String, Value>, platform: &str, home: &str) -> Vec<String> {
    if let Some(d) = var(env, "TUNNEL_CLIENT_PROFILE_DIR").filter(|v| truthy(Some(v))) {
        return vec![js_string(d)];
    }
    let p = style(platform);
    let mut dirs: Vec<String> = Vec::new();
    if platform == "win32" {
        let roaming = var(env, "APPDATA").map_or_else(|| p.join(&[home, "AppData", "Roaming"]), js_string);
        dirs.push(p.join(&[&roaming, "tunnel-client"]));
    }
    if platform == "darwin" {
        dirs.push(p.join(&[home, "Library", "Application Support", "tunnel-client"]));
    }
    let config = var(env, "XDG_CONFIG_HOME").map_or_else(|| p.join(&[home, ".config"]), js_string);
    dirs.push(p.join(&[&config, "tunnel-client"]));
    let mut out: Vec<String> = Vec::new();
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn value_of(text: &str, key: &str) -> Option<String> {
    let re =
        Regex::new(&format!(r#"(?m)^\s*-?\s*{key}:\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^']|'')*)'|([^#\r\n]*?))\s*(?:#.*)?$"#)).expect("a fixed pattern, and one of the profile's keys");
    let c = re.captures(text)?;
    if let Some(m) = c.get(1) {
        static ESC: OnceLock<Regex> = OnceLock::new();
        return Some(ESC.get_or_init(|| Regex::new(r"\\(.)").expect("a fixed pattern")).replace_all(m.as_str(), "$1").into_owned());
    }
    if let Some(m) = c.get(2) {
        return Some(m.as_str().replace("''", "'"));
    }
    c.get(3).map(|m| m.as_str().to_string()).filter(|s| !s.is_empty())
}

/// What a profile says, read line by line: its tunnel, its health address,
/// and which grant its MCP command serves. Not a YAML parser.
pub fn read_profile(text: &str) -> Value {
    static GRANT: OnceLock<Regex> = OnceLock::new();
    let command = value_of(text, "command");
    let grant = GRANT
        .get_or_init(|| Regex::new(r"(?-u:\b)mcp\s+assistant(?-u:\b).*--grant[ =](gr_[0-9a-f]{24})(?-u:\b)").expect("a fixed pattern"))
        .captures(command.as_deref().unwrap_or(""))
        .map(|c| c[1].to_string());
    let tunnel = value_of(text, "tunnel_id");
    json!({ "tunnelId": tunnel.filter(|t| is_tunnel_id(t)), "health": value_of(text, "listen_addr"), "grant": grant, "xenonet": grant.is_some() })
}

/// Where profiles are read from.
#[expect(clippy::result_unit_err, reason = "a profile that can't be read is skipped, whatever went wrong, as the JS's catch did")]
pub trait Files {
    fn list(&mut self, dir: &str) -> Result<Vec<String>, ()>;
    fn read(&mut self, file: &str) -> Result<String, ()>;
    fn mtime(&mut self, file: &str) -> Result<f64, ()>;
}

#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the name is lowered first")]
fn yaml_name(n: &str) -> Option<&str> {
    let lower = n.to_ascii_lowercase();
    if lower.ends_with(".yaml") {
        Some(&n[..n.len() - 5])
    } else if lower.ends_with(".yml") {
        Some(&n[..n.len() - 4])
    } else {
        None
    }
}

/// The profiles in `dirs` that run xenonet mcp assistant, newest first.
pub fn find_profiles(dirs: &[String], fs: &mut dyn Files) -> Vec<Value> {
    let mut out: Vec<(f64, Value)> = Vec::new();
    for dir in dirs {
        let Ok(names) = fs.list(dir) else { continue };
        for n in names.iter().filter(|n| yaml_name(n).is_some()) {
            let file = xenonet_app::nodepath::NATIVE.join(&[dir, n]);
            let Ok(text) = fs.read(&file) else { continue };
            let p = read_profile(&text);
            if p["xenonet"] != json!(true) {
                continue;
            }
            let Ok(at) = fs.mtime(&file) else { continue };
            out.push((at, json!({ "name": yaml_name(n).unwrap_or(n), "file": file, "tunnelId": p["tunnelId"], "health": p["health"], "grant": p["grant"] })));
        }
    }
    xenonet_base::sort::sort_by(&mut out, |a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    out.into_iter().map(|(_, p)| p).collect()
}

fn js_space(c: char) -> bool {
    xenonet_base::text::is_js_space(c)
}

/// A command line as tunnel-client splits it: words apart by spaces, a
/// word with spaces in double quotes, and forward slashes, since it reads
/// a backslash as an escape.
pub fn command_line(argv: &[Value]) -> String {
    argv.iter()
        .map(|a| {
            let a = js_string(a).replace('\\', "/");
            if a.chars().any(|c| js_space(c) || c == '"' || c == '\'') {
                format!("\"{}\"", a.replace('"', "\\\""))
            } else {
                a
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What tunnel-client runs for a grant: the xenonet binary (installed
/// XenoNet's launcher) with mcp assistant.
pub fn adapter_argv(bin: &Value, grant: &Value, port: &Value) -> Result<Vec<Value>, String> {
    if !crate::credentials::is_grant(&js_string(grant)) {
        return Err("not a grant id".into());
    }
    let mut argv = vec![bin.clone(), json!("mcp"), json!("assistant"), json!("--grant"), grant.clone()];
    if !xenonet_app::jsv::strict_eq(Some(port), Some(&json!(47840))) {
        argv.push(json!("--port"));
        argv.push(json!(js_string(port)));
    }
    Ok(argv)
}

/// The profile XenoNet writes for a grant: no secret in it, its health on
/// a port the system picks, and logs as JSON lines XenoNet can read.
pub fn profile_yaml(tunnel_id: &Value, command: &Value, label: Option<&Value>) -> Result<String, String> {
    if !is_tunnel_id(&js_string(tunnel_id)) {
        return Err("not a tunnel id".into());
    }
    let q = |s: &Value| format!("'{}'", js_string(s).replace('\'', "''"));
    let label = label.filter(|l| truthy(Some(l))).map_or(String::new(), |l| format!(" for “{}”", js_string(l).replace(['\r', '\n'], " ")));
    Ok([
        format!("# Written by XenoNet{label}. XenoNet writes it again when the connection"),
        "# changes, so change the connection in XenoNet rather than here.".into(),
        "config_version: 1".into(),
        "control_plane:".into(),
        "  base_url: \"https://api.openai.com\"".into(),
        format!("  tunnel_id: {}", q(tunnel_id)),
        "  api_key: \"env:CONTROL_PLANE_API_KEY\"".into(),
        "health:".into(),
        "  listen_addr: \"127.0.0.1:0\"".into(),
        "admin_ui:".into(),
        "  open_browser: false".into(),
        "log:".into(),
        "  level: info".into(),
        "  format: json".into(),
        "mcp:".into(),
        "  commands:".into(),
        "    - channel: main".into(),
        format!("      command: {}", q(command)),
        String::new(),
    ]
    .join("\n"))
}

/// Anything that looks like an OpenAI key, as sk-….
pub fn mask_keys(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"sk-[A-Za-z0-9_-]{8,}").expect("a fixed pattern")).replace_all(s, "sk-…").into_owned()
}

/// One line of tunnel-client's log, as XenoNet shows it: its level and
/// message, with anything that looks like a key taken out. None for a
/// line that isn't JSON.
pub fn log_line(line: &str) -> Option<Value> {
    let e = xenonet_base::json::parse(line).ok()?;
    if !e.is_object() && !e.is_array() {
        return None;
    }
    let pick = |a: &str, b: &str| e.get(a).filter(|v| !v.is_null()).or_else(|| e.get(b).filter(|v| !v.is_null())).cloned();
    let msg = pick("msg", "message").map_or_else(String::new, |m| js_string(&m));
    let msg: String = mask_keys(&msg).chars().map(|c| if (c as u32) < 0x20 { ' ' } else { c }).collect();
    let msg = xenonet_app::jstext::slice16(&msg, 300);
    let level = e.get("level").filter(|v| !v.is_null()).map_or_else(|| "info".to_string(), js_string).to_lowercase();
    let err = pick("error", "err");
    let msg = match err.filter(|x| truthy(Some(x))) {
        Some(x) => format!("{msg}: {}", xenonet_app::jstext::slice16(&mask_keys(&js_string(&x)), 200)),
        None => msg,
    };
    Some(json!({ "level": level, "msg": msg }))
}

/// How a running client stands, from its /health?details=true (None when
/// it didn't answer).
pub fn health_of(h: Option<&Value>) -> Value {
    let Some(h) = h.filter(|h| truthy(Some(h))) else { return json!({ "state": "starting", "why": "its health check hasn't answered yet" }) };
    let comps = h.get("components");
    let cp = comps.and_then(|c| c.get("control-plane")).filter(|v| truthy(Some(v)));
    let mcp = comps.and_then(|c| c.get("mcp")).filter(|v| !v.is_null());
    if !truthy(h.get("live")) {
        return json!({ "state": "degraded", "why": "it says it isn't live" });
    }
    if let Some(cp) = cp {
        if cp.get("status").and_then(Value::as_str) != Some("ok") {
            let said = cp.get("state").filter(|v| !v.is_null()).or_else(|| cp.get("status")).map_or_else(|| "undefined".into(), js_string);
            let fails = cp
                .get("details")
                .and_then(|d| d.get("consecutive_failures"))
                .filter(|v| truthy(Some(v)))
                .map_or(String::new(), |n| format!(", {} failures in a row", js_string(n)));
            return json!({ "state": "degraded", "why": format!("OpenAI's control plane: {said}{fails}") });
        }
    }
    if let Some(child) = mcp.and_then(|m| m.get("details")).and_then(|d| d.get("child_state")).filter(|v| truthy(Some(v))) {
        if child.as_str() != Some("running") {
            return json!({ "state": "degraded", "why": format!("xenonet mcp assistant is {}", js_string(child)) });
        }
    }
    if !truthy(h.get("ready")) {
        return json!({ "state": "starting", "why": "it isn't ready yet" });
    }
    let version = h.get("runtime").and_then(|r| r.get("version")).and_then(Value::as_str).map(|v| v.split('+').next().unwrap_or("").to_string());
    let polled = cp.and_then(|c| c.get("details")).and_then(|d| d.get("last_success")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    json!({ "state": "ready", "why": null, "version": version, "polledAt": polled })
}
