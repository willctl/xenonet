//! Connecting your agents: what XenoNet adds to Claude Code's and Codex's
//! own settings so every session on this machine, in every project, can
//! use the engine: core/engine/connect.js. Nothing changes until you
//! confirm in XenoNet, which first shows exactly what will be added
//! (preview), and it can all be taken out again (remove).
//!
//!   Claude Code  the xenonet MCP server at user scope (through `claude
//!                mcp`), and HTTP hooks in ~/.claude/settings.json
//!   Codex        the xenonet MCP server (through `codex mcp`), and command
//!                hooks in ~/.codex/hooks.json
//!
//! Both run the xenonet binary (`xenonet mcp`, `xenonet hook <app>`): the
//! launcher's for installed XenoNet, which never moves. Entries an earlier
//! XenoNet wrote run Node and cli/xenonet.js instead; they're still its own,
//! so connecting again replaces them and disconnecting takes them out.
//!
//! XenoNet's hooks are told apart from yours by their URL or command, and a
//! copy of each settings file is kept the first time XenoNet changes it
//! (`<file>.before-xenonet`). Each XenoNet on this machine is told apart by
//! its port: connecting or disconnecting one changes its own entries and
//! leaves another's alone. Entries whose files are gone are anyone's to
//! clear.
//!
//! XenoNet's plugin for Claude Code (plugins/xenonet) carries the same
//! server and hooks for the installed XenoNet, so they're kept to one:
//! while the plugin is installed and turned on, connecting leaves Claude
//! Code to it and takes connect's own copies out; and while connect's own
//! hook for an event is in Claude Code's settings, the service ignores the
//! plugin's copy of it (`hook_connected`), so an event never arrives twice.
//!
//! The files and the CLIs are the twin's, called as the JS called them,
//! through `Calls`: "fs" (node:fs), "run" (a CLI, its output) and "cli" (a
//! CLI on PATH, or null).

use crate::js::{base64, base64_bytes, from_utf16le, utf16le};
use crate::peer::{Calls, Thrown};
use crate::sql::Arg;
use crate::store::js_text;
use serde_json::{json, Map, Value};
use xenonet_base::json::stringify;
use xenonet_base::paths::tilde;

const CLAUDE_EVENTS: [(&str, Option<&str>); 7] = [
    ("SessionStart", None),
    ("UserPromptSubmit", None),
    ("PreToolUse", Some("Edit|Write|MultiEdit|NotebookEdit")),
    ("PostToolUse", None),
    ("Stop", None),
    ("SessionEnd", None),
    ("Notification", None),
];
/// XenoNet's plugin for Claude Code, which Claude Code knows as
/// `xenonet@<marketplace>` once it's installed.
const PLUGIN: &str = "xenonet";
const CODEX_EVENTS: [(&str, Option<&str>); 6] =
    [("SessionStart", None), ("UserPromptSubmit", None), ("PreToolUse", Some("apply_patch|Edit|Write")), ("PostToolUse", None), ("Stop", None), ("SessionEnd", None)];

pub type Result<T> = std::result::Result<T, Thrown>;

/// One of the XenoNets that run on a machine (core/instances.js).
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub port: Arg,
}

/// `a === b` for JSON values: primitives by value, objects never.
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(_) | Value::Array(_), _) | (_, Value::Object(_) | Value::Array(_)) => false,
        (a, b) => a == b,
    }
}

fn same_arg(a: &Arg, b: &Arg) -> bool {
    match (a, b) {
        (Arg::Num(x), Arg::Num(y)) => x == y,
        (Arg::Text(x), Arg::Text(y)) => x == y,
        (Arg::Null, Arg::Null) | (Arg::Undef, Arg::Undef) => true,
        (Arg::Bool(x), Arg::Bool(y)) => x == y,
        _ => false,
    }
}

/// A JS object's keys in the order JS gives them: array indexes first, in
/// order, then the rest as they came.
fn js_order(m: &Map<String, Value>) -> Map<String, Value> {
    let index = |k: &str| k.parse::<u32>().ok().filter(|n| *n != u32::MAX && n.to_string() == k);
    let mut ints: Vec<(u32, &String)> = m.keys().filter_map(|k| index(k).map(|n| (n, k))).collect();
    ints.sort();
    let mut out = Map::new();
    for (_, k) in ints {
        out.insert(k.clone(), normalized(&m[k]));
    }
    for (k, v) in m {
        if index(k).is_none() {
            out.insert(k.clone(), normalized(v));
        }
    }
    out
}

/// A value with every object's keys in JS's order.
pub fn normalized(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(js_order(m)),
        Value::Array(a) => Value::Array(a.iter().map(normalized).collect()),
        other => other.clone(),
    }
}

/// JSON.stringify(v).
fn json_text(v: &Value) -> String {
    stringify(&normalized(v), 0)
}

/// `{ ...v }`: an object's own keys, an array's or a string's indexes, and
/// nothing for the rest.
fn spread(v: Option<&Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect(),
        Some(Value::String(s)) => s.encode_utf16().collect::<Vec<u16>>().iter().enumerate().map(|(i, u)| (i.to_string(), Value::String(String::from_utf16_lossy(&[*u])))).collect(),
        _ => Map::new(),
    }
}

/// `Object.values(v ?? {})`.
fn values(v: Option<&Value>) -> Vec<Value> {
    spread(v).into_iter().map(|(_, x)| x).collect()
}

/// `.flat()` of a list of values: arrays opened one level.
fn flat(items: Vec<Value>) -> Vec<Value> {
    items.into_iter().flat_map(|v| if let Value::Array(a) = v { a } else { vec![v] }).collect()
}

/// `g.hooks ?? []`, as flatMap reads it: an array's items, or the value.
fn hooks_of(g: &Value) -> Vec<Value> {
    match g.get("hooks") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
    }
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => crate::js::template(Some(other)),
    }
}

/// `'…'`, for a POSIX shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `'…'`, for PowerShell.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// path.isAbsolute on this machine.
fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        win32_absolute(p)
    } else {
        p.starts_with('/')
    }
}

fn win32_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with('\\') || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
}

/// path.join on this machine: empty parts left out.
fn join(dir: &str, name: &str) -> String {
    xenonet_base::paths::join_all(&[dir, name])
}

/// path.dirname on this machine, as Node works it out.
fn dirname(p: &str) -> String {
    let sep = |c: u8| c == b'/' || (cfg!(windows) && c == b'\\');
    let b = p.as_bytes();
    if b.is_empty() {
        return ".".into();
    }
    let root = sep(b[0]);
    let mut end = None;
    let mut matched_slash = true;
    for i in (1..b.len()).rev() {
        if sep(b[i]) {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None => {
            if root {
                p[..1].to_string()
            } else {
                ".".into()
            }
        }
        Some(1) if root => p[..2].to_string(),
        Some(e) => p[..e].to_string(),
    }
}

/// The names a XenoNet's MCP servers take: the installed XenoNet's xenonet
/// and xenonet-console, another's with its instance (xenonet-dev).
pub fn server_names(port: &Arg, instances: &[Instance]) -> (String, String) {
    let id = instances.iter().find(|i| same_arg(&i.port, port)).map_or_else(|| js_text(port), |i| i.id.clone());
    let installed = instances.iter().find(|i| i.id == "installed").map_or(Arg::Undef, |i| i.port.clone());
    let base = if same_arg(port, &installed) { "xenonet".to_string() } else { format!("xenonet-{id}") };
    let console = format!("{base}-console");
    (base, console)
}

/// A settings object's "hooks" with XenoNet's taken out (`ours` tells them
/// apart), and with `add` put back in when given.
pub fn merge_hooks(c: &mut dyn Calls, settings: &Value, ours: &mut dyn FnMut(&mut dyn Calls, &Value) -> Result<bool>, add: Option<&Value>) -> Result<Value> {
    let mut hooks = spread(settings.get("hooks").filter(|h| !h.is_null()));
    let events: Vec<(String, Value)> = hooks.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (event, groups) in events {
        let groups = if let Value::Array(a) = groups { a } else { Vec::new() };
        let mut kept = Vec::new();
        for g in groups {
            if g.is_null() {
                return Err(Thrown::new("Cannot read properties of null (reading 'hooks')"));
            }
            let mut g2 = spread(Some(&g));
            let mut left = Vec::new();
            for h in match g.get("hooks") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(Thrown::new("(g.hooks ?? []).filter is not a function")),
            } {
                if !ours(c, &h)? {
                    left.push(h);
                }
            }
            g2.insert("hooks".into(), Value::Array(left.clone()));
            if !left.is_empty() {
                kept.push(Value::Object(g2));
            }
        }
        if kept.is_empty() {
            hooks.shift_remove(&event);
        } else {
            hooks.insert(event, Value::Array(kept));
        }
    }
    if let Some(add) = add {
        for (event, group) in spread(Some(add)) {
            let mut list = match hooks.get(&event) {
                Some(Value::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            list.push(group);
            hooks.insert(event, Value::Array(list));
        }
    }
    let empty = hooks.is_empty();
    let mut out = spread(Some(settings));
    out.insert("hooks".into(), Value::Object(hooks));
    if empty {
        out.shift_remove("hooks");
    }
    Ok(normalized(&Value::Object(out)))
}

/// How a connect was made: what core/engine/connect.js's createConnect
/// was given.
pub struct Options {
    pub port: Arg,
    pub demo: Arg,
    /// The xenonet binary the entries run.
    pub bin: String,
    pub home: String,
    pub claude_config_dir: Option<String>,
    pub codex_home: Option<String>,
    pub console_mcp: bool,
    pub platform: String,
    pub instances: Vec<Instance>,
}

pub struct Connect {
    o: Options,
    server_env: Vec<(String, String)>,
    mcp: Value,
    hook_url: String,
    claude_settings: String,
    claude_state: String,
    /// Claude Code's list of the plugins it has installed.
    claude_plugins: String,
    /// Whether XenoNet's plugin, which reaches the installed XenoNet, is
    /// for this XenoNet.
    plugin_port: bool,
    codex_hooks: String,
    names: (String, String),
    claude_hooks: Value,
    codex_hook_set: Value,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Claude,
    Codex,
}

impl Connect {
    pub fn new(o: Options) -> Self {
        let server_env = vec![("XENONET_PORT".to_string(), js_text(&o.port))];
        let env_obj: Map<String, Value> = server_env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let mcp = json!({ "type": "stdio", "command": o.bin, "args": ["mcp"], "env": Value::Object(env_obj) });
        let hook_url = format!("http://127.0.0.1:{}/api/hook/claude", js_text(&o.port));
        let claude_dir = o.claude_config_dir.clone().unwrap_or_else(|| join(&o.home, ".claude"));
        let claude_settings = join(&claude_dir, "settings.json");
        // Claude Code keeps user-scope MCP servers in .claude.json, in
        // CLAUDE_CONFIG_DIR when that's set.
        let claude_state = match o.claude_config_dir.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => join(d, ".claude.json"),
            None => join(&o.home, ".claude.json"),
        };
        let claude_plugins = join(&join(&claude_dir, "plugins"), "installed_plugins.json");
        let plugin_port = o.instances.iter().any(|i| i.id == "installed" && same_arg(&i.port, &o.port));
        let codex_hooks = join(&o.codex_home.clone().unwrap_or_else(|| join(&o.home, ".codex")), "hooks.json");
        let names = server_names(&o.port, &o.instances);
        let mut c = Connect {
            o,
            server_env,
            mcp,
            hook_url,
            claude_settings,
            claude_state,
            claude_plugins,
            plugin_port,
            codex_hooks,
            names,
            claude_hooks: Value::Null,
            codex_hook_set: Value::Null,
        };
        let win = c.o.platform == "win32";
        // Claude Code takes HTTP hooks, which cost nothing to run, except at
        // SessionStart, which takes only commands.
        let mut claude = Map::new();
        for (event, matcher) in CLAUDE_EVENTS {
            let hook = if event == "SessionStart" {
                let mut h = Map::new();
                h.insert("type".into(), json!("command"));
                h.insert("command".into(), json!(if win { c.windows_hook("claude") } else { c.hook_command("claude") }));
                if win {
                    h.insert("shell".into(), json!("powershell"));
                }
                h.insert("timeout".into(), json!(10));
                Value::Object(h)
            } else {
                json!({ "type": "http", "url": c.hook_url, "timeout": 5 })
            };
            claude.insert(event.into(), group(matcher, hook));
        }
        // Codex allows SessionEnd hooks 3 seconds at most.
        let mut codex = Map::new();
        for (event, matcher) in CODEX_EVENTS {
            let mut h = Map::new();
            h.insert("type".into(), json!("command"));
            h.insert("command".into(), json!(c.hook_command("codex")));
            if win {
                h.insert("commandWindows".into(), json!(c.windows_hook("codex")));
            }
            h.insert("timeout".into(), json!(if event == "SessionEnd" { 3 } else { 10 }));
            codex.insert(event.into(), group(matcher, Value::Object(h)));
        }
        c.claude_hooks = Value::Object(claude);
        c.codex_hook_set = Value::Object(codex);
        c
    }

    pub fn names(&self) -> &(String, String) {
        &self.names
    }

    fn hook_command(&self, app: &str) -> String {
        let env: Vec<String> = self.server_env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{} {} hook {app}", env.join(" "), quote(&self.o.bin))
    }

    fn windows_hook(&self, app: &str) -> String {
        let env: Vec<String> = self.server_env.iter().map(|(k, v)| format!("$env:{k}={}", ps_quote(v))).collect();
        let script = format!("# XenoNet hook {app}\n{}; & {} hook {}; exit $LASTEXITCODE", env.join("; "), ps_quote(&self.o.bin), ps_quote(app));
        format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}", base64(&utf16le(&script)))
    }

    // ---------- whose an entry is ----------

    fn hook_text(h: &Value) -> String {
        let command = match h.get("commandWindows").filter(|v| !v.is_null()) {
            Some(v) => text_of(Some(v)),
            None => text_of(h.get("command")),
        };
        match crate::re!(r"-EncodedCommand ([A-Za-z0-9+/=]+)$").captures(&command) {
            Some(m) => from_utf16le(&base64_bytes(&m[1])),
            None => command,
        }
    }

    /// Any XenoNet's command entries, whichever XenoNet or checkout added
    /// them: the binary's, cli/xenonet.js's with Node, or the hook.js of
    /// earlier versions.
    fn xenonet_command(kind: Kind, h: &Value) -> bool {
        if h.get("type").and_then(Value::as_str) != Some("command") {
            return false;
        }
        let command = text_of(h.get("command"));
        let (form, app) = match kind {
            Kind::Claude => (crate::re!(r#"(cli[\\/]xenonet\.js['"]? hook|(cli|server[\\/]engine)[\\/]hook\.js['"]?|[\\/]xenonet(\.exe)?['"]? hook) claude$"#), "claude"),
            Kind::Codex => (crate::re!(r#"(cli[\\/]xenonet\.js['"]? hook|(cli|server[\\/]engine)[\\/]hook\.js['"]?|[\\/]xenonet(\.exe)?['"]? hook) codex$"#), "codex"),
        };
        if form.is_match(&command) {
            return true;
        }
        let text = Self::hook_text(h);
        text.starts_with(&format!("# XenoNet hook {app}\n")) && text.ends_with(&format!(" hook '{app}'; exit $LASTEXITCODE"))
    }

    fn xenonet_hook(kind: Kind, h: &Value) -> bool {
        match kind {
            Kind::Claude => {
                (h.get("type").and_then(Value::as_str) == Some("http") && crate::re!(r"^http://(127\.0\.0\.1|localhost):[0-9]+/api/hook/claude$").is_match(&text_of(h.get("url"))))
                    || Self::xenonet_command(Kind::Claude, h)
            }
            Kind::Codex => Self::xenonet_command(Kind::Codex, h),
        }
    }

    /// Which XenoNet an entry is for: its port, from its URL or
    /// XENONET_PORT; None for an early form that didn't say.
    fn port_of(h: &Value) -> Option<f64> {
        let m = if h.get("type").and_then(Value::as_str) == Some("http") {
            crate::re!(r":([0-9]+)/api/hook/").captures(&text_of(h.get("url"))).map(|m| m[1].to_string())
        } else {
            crate::re!(r#"XENONET_PORT=['"]?([0-9]+)"#).captures(&Self::hook_text(h)).map(|m| m[1].to_string())
        };
        m.map(|d| xenonet_base::text::js_number(Some(&Value::String(d))))
    }

    /// Only the paths among a command's quoted words.
    fn hook_files(hooks: Option<&Value>, ours: &mut dyn FnMut(&Value) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if !ours(&h) || h.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                for m in crate::re!(r"'((?:[^']|'')+)'").captures_iter(&Self::hook_text(&h)) {
                    let f = m[1].replace("''", "'");
                    if is_absolute(&f) || win32_absolute(&f) {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    fn exists(c: &mut dyn Calls, f: &str) -> Result<bool> {
        Ok(c.call("fs", "existsSync", vec![Arg::text(f)])?.truthy())
    }

    /// An entry is this XenoNet's when it's for this port, or says no port,
    /// or runs files that are gone; another live XenoNet's stays as it is.
    fn mine(&self, c: &mut dyn Calls, kind: Kind, h: &Value) -> Result<bool> {
        if !Self::xenonet_hook(kind, h) {
            return Ok(false);
        }
        let port = Self::port_of(h);
        if port.is_none_or(|p| matches!(self.o.port, Arg::Num(q) if q == p)) {
            return Ok(true);
        }
        let one = json!({ "x": [{ "hooks": [h] }] });
        for f in Self::hook_files(Some(&one), &mut |h| Self::xenonet_hook(kind, h)) {
            if !Self::exists(c, &f)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether this XenoNet has hooks here, in place or not: its port's,
    /// or an early form's that named none.
    fn has_own(&self, hooks: Option<&Value>, kind: Kind) -> bool {
        flat(values(hooks)).iter().flat_map(hooks_of).any(|h| Self::xenonet_hook(kind, &h) && Self::port_of(&h).is_none_or(|p| matches!(self.o.port, Arg::Num(q) if q == p)))
    }

    /// XenoNet's plugin for Claude Code, as Claude Code knows it
    /// (`xenonet@<marketplace>`), when it's installed and turned on in
    /// Claude Code's user settings. The plugin reaches the installed
    /// XenoNet, so only that XenoNet counts it. A list of plugins that
    /// can't be read counts as none.
    fn plugin(&self, c: &mut dyn Calls, settings: &Value) -> Option<String> {
        if !self.plugin_port {
            return None;
        }
        let enabled = settings.get("enabledPlugins").and_then(Value::as_object)?;
        let installed = self.read_json(c, &self.claude_plugins).ok()?;
        let installed = installed.get("plugins").and_then(Value::as_object)?;
        enabled
            .iter()
            .find(|(key, on)| {
                key.split_once('@').is_some_and(|(name, _)| name == PLUGIN)
                    && **on == Value::Bool(true)
                    && installed.get(*key).and_then(Value::as_array).is_some_and(|a| !a.is_empty())
            })
            .map(|(key, _)| key.clone())
    }

    /// Whether this XenoNet's own Claude Code hook for `event`, as connect
    /// adds it, is in Claude Code's settings and can run (a command's files
    /// are all there). While it is, the copy XenoNet's plugin posts stands
    /// by, so the event reaches XenoNet once. Settings that can't be read
    /// hold no hook Claude Code runs.
    pub fn hook_connected(&self, c: &mut dyn Calls, event: &str) -> Result<bool> {
        let Ok(settings) = self.read_json(c, &self.claude_settings) else {
            return Ok(false);
        };
        let groups = match settings.get("hooks").and_then(|h| h.get(event)) {
            Some(Value::Array(a)) => a.clone(),
            _ => return Ok(false),
        };
        for h in groups.iter().flat_map(hooks_of) {
            if !Self::xenonet_hook(Kind::Claude, &h) || !Self::port_of(&h).is_none_or(|p| matches!(self.o.port, Arg::Num(q) if q == p)) {
                continue;
            }
            let one = json!({ "x": [{ "hooks": [h] }] });
            let mut runs = true;
            for f in Self::hook_files(Some(&one), &mut |_| true) {
                if !Self::exists(c, &f)? {
                    runs = false;
                }
            }
            if runs {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether this XenoNet's entries here are an earlier XenoNet's, which
    /// run Node and cli/xenonet.js (or hook.js) rather than the binary: a
    /// hook of its own, or its MCP server.
    fn earlier(&self, hooks: Option<&Value>, kind: Kind, server: Option<&Value>) -> bool {
        let node = |t: &str| t.contains("xenonet.js") || t.contains("hook.js");
        let own = flat(values(hooks)).iter().flat_map(hooks_of).any(|h| {
            h.get("type").and_then(Value::as_str) == Some("command")
                && Self::xenonet_hook(kind, &h)
                && Self::port_of(&h).is_none_or(|p| matches!(self.o.port, Arg::Num(q) if q == p))
                && node(&Self::hook_text(&h))
        });
        own || server.and_then(|s| s.get("args")).and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| crate::re!(r"\.m?js$").is_match(&text_of(Some(x)))))
    }

    /// Other XenoNets connected here too, by port, for the preview.
    fn others(&self, c: &mut dyn Calls, hooks: Option<&Value>, kind: Kind) -> Result<Vec<Option<f64>>> {
        let mut out: Vec<Option<f64>> = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if Self::xenonet_hook(kind, &h) && !self.mine(c, kind, &h)? {
                    let p = Self::port_of(&h);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        // sort(), as strings.
        let text = |p: &Option<f64>| p.map_or("null".to_string(), |n| js_text(&Arg::num(n)));
        out.sort_by(|a, b| crate::js::utf16_cmp(&text(a), &text(b)));
        Ok(out)
    }

    // ---------- files ----------

    /// A settings file, or {} when there's none. One that can't be read, or
    /// isn't JSON, is left alone, and the error names it with the home
    /// folder as ~ and says where it goes wrong without quoting it.
    fn read_json(&self, c: &mut dyn Calls, file: &str) -> Result<Value> {
        let home = &self.o.home;
        let text = match c.call("fs", "readFileSync", vec![Arg::text(file), Arg::text("utf8")]) {
            Ok(t) => js_text(&t),
            Err(e) => {
                let code = e.code.as_deref().filter(|v| !v.is_null());
                if code.and_then(Value::as_str) == Some("ENOENT") {
                    return Ok(json!({}));
                }
                let why = code.map_or_else(|| tilde(&e.message, home), |v| crate::js::template(Some(v)));
                return Err(Thrown::new(format!("XenoNet can't read {} ({why}), so it leaves it alone", tilde(file, home))));
            }
        };
        match xenonet_base::json::parse(&text) {
            Ok(v) => Ok(normalized(&v)),
            Err(_) => {
                let message = match xenonet_schema::vendor::json_error_at(&text) {
                    Some((line, column)) => format!("isn't valid JSON (line {line}, column {column})"),
                    None => "isn't valid JSON".into(),
                };
                Err(Thrown::new(format!("{} {message}, so XenoNet leaves it alone", tilde(file, home))))
            }
        }
    }

    fn write_json(c: &mut dyn Calls, file: &str, data: &Value) -> Result<()> {
        c.call("fs", "mkdirSync", vec![Arg::text(dirname(file)), Arg::Obj(vec![("recursive".into(), Arg::Bool(true))])])?;
        let before = format!("{file}.before-xenonet");
        if Self::exists(c, file)? && !Self::exists(c, &before)? {
            c.call("fs", "copyFileSync", vec![Arg::text(file), Arg::text(before)])?;
        }
        let tmp = format!("{file}.tmp-xenonet");
        c.call("fs", "writeFileSync", vec![Arg::text(tmp.clone()), Arg::text(format!("{}\n", stringify(&normalized(data), 2)))])?;
        c.call("fs", "renameSync", vec![Arg::text(tmp), Arg::text(file)])?;
        Ok(())
    }

    fn cli(c: &mut dyn Calls, name: &str) -> Result<Arg> {
        c.call("", "cli", vec![Arg::text(name)])
    }

    fn run(c: &mut dyn Calls, bin: &Arg, args: Vec<String>) -> Result<Arg> {
        c.call("", "run", vec![bin.clone(), Arg::List(args.into_iter().map(Arg::text).collect())])
    }

    // ---------- what's there ----------

    /// The xenonet server as each app has it: missing, present (as XenoNet
    /// would add it) or different.
    fn server_state(&self, s: Option<&Value>, args: Option<&Value>) -> &'static str {
        let Some(s) = s.filter(|s| crate::js::truthy(Some(s))) else {
            return "missing";
        };
        let args = args.unwrap_or(&self.mcp["args"]);
        let same_command = strict_eq(s.get("command").unwrap_or(&Value::Null), &self.mcp["command"]) && s.get("command").is_some();
        let same_args = s.get("args").is_some_and(|a| json_text(a) == json_text(args));
        let same_port = s.get("env").and_then(|e| e.get("XENONET_PORT")).is_some_and(|p| strict_eq(p, &self.mcp["env"]["XENONET_PORT"]));
        if same_command && same_args && same_port {
            "present"
        } else {
            "different"
        }
    }

    fn codex_entry(c: &mut dyn Calls, bin: &Arg, name: &str) -> Option<Value> {
        if !bin.truthy() {
            return None;
        }
        let r = Self::run(c, bin, vec!["mcp".into(), "get".into(), name.into(), "--json".into()]).unwrap_or(Arg::Null);
        if !r.truthy() {
            return None;
        }
        Some(match xenonet_base::json::parse(&js_text(&r)) {
            Ok(Value::Null) | Err(_) => json!({ "command": null }),
            Ok(s) => match s.get("transport").filter(|t| !t.is_null()) {
                Some(t) => t.clone(),
                None => s,
            },
        })
    }

    /// `x.mcpServers?.[name]`.
    fn server_in(state: &Value, name: &str) -> Option<Value> {
        state.get("mcpServers").filter(|m| !m.is_null()).and_then(|m| m.get(name)).cloned()
    }

    /// Whether every group in `add` is already in place.
    fn hooks_in_place(&self, c: &mut dyn Calls, settings: &Value, kind: Kind, add: &Value) -> Result<bool> {
        for (event, group) in spread(Some(add)) {
            let groups = match settings.get("hooks").and_then(|h| h.get(&event)) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(Thrown::new("(settings.hooks?.[event] ?? []).some is not a function")),
            };
            let matcher = group.get("matcher").cloned().unwrap_or(Value::Null);
            let first = json_text(&group["hooks"][0]);
            let mut found = false;
            for g in groups {
                let theirs = g.get("matcher").cloned().unwrap_or(Value::Null);
                if !strict_eq(&theirs, &matcher) {
                    continue;
                }
                let Some(Value::Array(hs)) = g.get("hooks") else { continue };
                for h in hs {
                    if self.mine(c, kind, h)? && json_text(h) == first {
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            if !found {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Files a connected entry runs that no longer exist.
    fn gone(c: &mut dyn Calls, entry: &Value) -> Result<Vec<String>> {
        let mut files = vec![entry.get("command").cloned().unwrap_or(Value::Null)];
        if let Some(Value::Array(args)) = entry.get("args") {
            files.extend(args.iter().filter(|a| crate::re!(r"\.m?js$").is_match(&text_of(Some(a)))).cloned());
        }
        let mut out = Vec::new();
        for f in files {
            if !crate::js::truthy(Some(&f)) {
                continue;
            }
            let f = text_of(Some(&f));
            if is_absolute(&f) && !Self::exists(c, &f)? {
                out.push(f);
            }
        }
        Ok(out)
    }

    fn broken(&self, c: &mut dyn Calls, entry: Option<&Value>, hooks: Option<&Value>, kind: Kind) -> Result<Vec<Value>> {
        let mut files = match entry.filter(|e| crate::js::truthy(Some(e))) {
            Some(e) => Self::gone(c, e)?,
            None => Vec::new(),
        };
        // hookFiles with this XenoNet's own entries, which may look at files.
        let mut mine_list = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if self.mine(c, kind, &h)? {
                    mine_list.push(h);
                }
            }
        }
        let wrapped = json!({ "x": [{ "hooks": mine_list }] });
        for f in Self::hook_files(Some(&wrapped), &mut |_| true) {
            if !Self::exists(c, &f)? {
                files.push(f);
            }
        }
        Ok(files.into_iter().map(|f| Value::String(format!("{} is gone", tilde(&f, &self.o.home)))).collect())
    }

    fn besides(&self, ports: &[Option<f64>]) -> Vec<Value> {
        ports
            .iter()
            .map(|p| {
                let shown = p.map_or("null".to_string(), |n| js_text(&Arg::num(n)));
                let name = self.o.instances.iter().find(|i| matches!((&i.port, p), (Arg::Num(a), Some(b)) if a == b)).map(|i| format!(", {}", i.name)).unwrap_or_default();
                Value::String(format!("Another XenoNet (port {shown}{name}) is connected here too; its entries stay as they are."))
            })
            .collect()
    }

    fn arg_json(a: &Arg) -> Value {
        match a {
            Arg::Num(n) => xenonet_base::json::number(*n),
            other => other.to_json(),
        }
    }

    pub fn preview(&self, c: &mut dyn Calls) -> Result<Value> {
        let claude = Self::cli(c, "claude")?;
        let codex = Self::cli(c, "codex")?;
        let (xenonet, console) = self.names.clone();
        let claude_entry = Self::server_in(&self.read_json(c, &self.claude_state)?, &xenonet);
        let codex_server = Self::codex_entry(c, &codex, &xenonet);
        let (console_claude, console_codex) = if self.o.console_mcp {
            let a = Self::server_in(&self.read_json(c, &self.claude_state)?, &console);
            let b = Self::codex_entry(c, &codex, &console);
            (a, b)
        } else {
            (None, None)
        };
        let port = js_text(&self.o.port);
        let home = &self.o.home;
        let command = tilde(&self.o.bin, home);
        let mut console_args = self.mcp["args"].as_array().cloned().unwrap_or_default();
        console_args.push(json!("console"));
        let console_args = Value::Array(console_args);
        let console_change = |app: &str, s: Option<&Value>| -> Vec<Value> {
            if !self.o.console_mcp {
                return Vec::new();
            }
            vec![
                json!({ "id": "console", "what": format!("The {console} MCP server, to operate Desktop"), "where": format!("{app} user settings"), "state": self.server_state(s, Some(&console_args)), "detail": format!("{command} mcp console (XENONET_PORT={port})") }),
            ]
        };
        let events = |list: &[(&str, Option<&str>)]| list.iter().map(|(e, m)| if m.is_some() { format!("{e} (edits)") } else { e.to_string() }).collect::<Vec<_>>().join(", ");

        let mut claude_changes = console_change("claude", console_claude.as_ref());
        let settings = self.read_json(c, &self.claude_settings)?;
        let plugin = self.plugin(c, &settings);
        let mcp_what = format!("The {xenonet} MCP server, for every project");
        let mcp_detail = format!("{command} mcp (XENONET_PORT={port})");
        let hooks_what = format!("Hooks: {}", events(&CLAUDE_EVENTS));
        let hooks_detail = format!("posting to {}; SessionStart runs {command} hook claude", self.hook_url);
        let own_hooks = self.has_own(settings.get("hooks"), Kind::Claude);
        let mut twice = false;
        if let Some(key) = &plugin {
            // The plugin carries the server and the hooks; connect's own
            // copies beside it are to come out.
            let carried = format!("XenoNet's plugin for Claude Code ({key})");
            if self.server_state(claude_entry.as_ref(), None) == "missing" {
                claude_changes.push(json!({ "id": "mcp", "what": mcp_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("xenonet mcp, from your PATH (XENONET_PORT={port})") }));
            } else {
                twice = true;
                claude_changes
                    .push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, beside the plugin's copy", "state": "remove", "detail": mcp_detail }));
            }
            if own_hooks {
                twice = true;
                claude_changes.push(json!({ "id": "hooks", "what": hooks_what, "where": format!("{}, beside the plugin's copies", tilde(&self.claude_settings, home)), "state": "remove", "detail": hooks_detail }));
            } else {
                claude_changes.push(json!({ "id": "hooks", "what": hooks_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("posting to {}?via=plugin; SessionStart runs xenonet hook claude", self.hook_url) }));
            }
        } else {
            claude_changes.push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, through claude mcp add-json --scope user", "state": self.server_state(claude_entry.as_ref(), None), "detail": mcp_detail }));
            let in_place = self.hooks_in_place(c, &settings, Kind::Claude, &self.claude_hooks)?;
            claude_changes
                .push(json!({ "id": "hooks", "what": hooks_what, "where": tilde(&self.claude_settings, home), "state": hooks_state(in_place, own_hooks), "detail": hooks_detail }));
        }
        // With the plugin, connecting adds nothing through the claude
        // command, so whether it's there doesn't matter.
        let mut claude_notes: Vec<Value> = if claude.truthy() || plugin.is_some() {
            Vec::new()
        } else {
            vec![json!("XenoNet can't find the claude command, so it can add the hooks but not the MCP server. Install Claude Code's CLI, or add the server by hand.")]
        };
        if let Some(key) = &plugin {
            claude_notes.push(json!(format!("Claude Code has XenoNet's plugin ({key}), which carries the {xenonet} MCP server and the hooks, so connecting leaves them to it.")));
        }
        if twice {
            claude_notes.push(json!(TWICE));
        }
        if self.earlier(settings.get("hooks"), Kind::Claude, claude_entry.as_ref()) {
            claude_notes.push(json!(EARLIER));
        }
        let others = self.others(c, settings.get("hooks"), Kind::Claude)?;
        claude_notes.extend(self.besides(&others));
        let mut claude_broken = self.broken(c, claude_entry.as_ref(), settings.get("hooks"), Kind::Claude)?;
        if plugin.is_some() && !Self::cli(c, "xenonet")?.truthy() {
            claude_broken.push(json!(NO_COMMAND));
        }

        let mut codex_changes = console_change("codex", console_codex.as_ref());
        codex_changes.push(json!({ "id": "mcp", "what": format!("The {xenonet} MCP server, for every project"), "where": "Codex's config, through codex mcp add", "state": self.server_state(codex_server.as_ref(), None), "detail": format!("{command} mcp (XENONET_PORT={port})") }));
        let hooks_file = self.read_json(c, &self.codex_hooks)?;
        let in_place = self.hooks_in_place(c, &hooks_file, Kind::Codex, &self.codex_hook_set)?;
        let state = hooks_state(in_place, self.has_own(hooks_file.get("hooks"), Kind::Codex));
        codex_changes.push(json!({ "id": "hooks", "what": format!("Hooks: {}", events(&CODEX_EVENTS)), "where": tilde(&self.codex_hooks, home), "state": state, "detail": format!("running {command} hook codex") }));
        let mut codex_notes: Vec<Value> =
            if codex.truthy() { Vec::new() } else { vec![json!("XenoNet can't find the codex command, so it can add the hooks but not the MCP server.")] };
        codex_notes.push(json!("Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's."));
        let hooks_file = self.read_json(c, &self.codex_hooks)?;
        if self.earlier(hooks_file.get("hooks"), Kind::Codex, codex_server.as_ref()) {
            codex_notes.push(json!(EARLIER));
        }
        let others = self.others(c, hooks_file.get("hooks"), Kind::Codex)?;
        codex_notes.extend(self.besides(&others));
        let hooks_file = self.read_json(c, &self.codex_hooks)?;
        let codex_broken = self.broken(c, codex_server.as_ref(), hooks_file.get("hooks"), Kind::Codex)?;

        Ok(json!({
            "port": Self::arg_json(&self.o.port),
            "demo": Self::arg_json(&self.o.demo),
            "apps": [
                { "app": "claude", "name": "Claude Code", "cli": Self::arg_json(&claude), "changes": claude_changes, "notes": claude_notes, "broken": claude_broken },
                { "app": "codex", "name": "Codex", "cli": Self::arg_json(&codex), "changes": codex_changes, "notes": codex_notes, "broken": codex_broken },
            ],
        }))
    }

    pub fn apply(&self, c: &mut dyn Calls, apps: &[Arg]) -> Result<Value> {
        // Agents connected to the demo would see only its made-up sessions.
        if self.o.demo.truthy() {
            return Err(Thrown::new("this is the demo, so connect Claude Code and Codex from XenoNet, with your own sessions"));
        }
        let has = |app: &str| apps.iter().any(|a| a.as_str() == Some(app));
        let (xenonet, console) = self.names.clone();
        let mut console_mcp = self.mcp.clone();
        if let Some(Value::Array(a)) = console_mcp.get_mut("args") {
            a.push(json!("console"));
        }
        if has("claude") {
            let settings = self.read_json(c, &self.claude_settings)?;
            // With XenoNet's plugin there, connect's own copies of the
            // server and the hooks come out, and none go in.
            let plugin = self.plugin(c, &settings).is_some();
            if !plugin {
                let merged = merge_hooks(c, &settings, &mut |c, h| self.mine(c, Kind::Claude, h), Some(&self.claude_hooks))?;
                Self::write_json(c, &self.claude_settings, &merged)?;
            } else if self.has_own(settings.get("hooks"), Kind::Claude) {
                let merged = merge_hooks(c, &settings, &mut |c, h| self.mine(c, Kind::Claude, h), None)?;
                Self::write_json(c, &self.claude_settings, &merged)?;
            }
            let bin = Self::cli(c, "claude")?;
            if bin.truthy() {
                let _ = Self::run(c, &bin, vec!["mcp".into(), "remove".into(), xenonet.clone(), "--scope".into(), "user".into()]);
                if !plugin {
                    Self::run(c, &bin, vec!["mcp".into(), "add-json".into(), xenonet.clone(), json_text(&self.mcp), "--scope".into(), "user".into()])?;
                }
                if self.o.console_mcp {
                    let _ = Self::run(c, &bin, vec!["mcp".into(), "remove".into(), console.clone(), "--scope".into(), "user".into()]);
                    Self::run(c, &bin, vec!["mcp".into(), "add-json".into(), console.clone(), json_text(&console_mcp), "--scope".into(), "user".into()])?;
                }
            }
        }
        if has("codex") {
            let hooks = self.read_json(c, &self.codex_hooks)?;
            let merged = merge_hooks(c, &hooks, &mut |c, h| self.mine(c, Kind::Codex, h), Some(&self.codex_hook_set))?;
            Self::write_json(c, &self.codex_hooks, &merged)?;
            let bin = Self::cli(c, "codex")?;
            if bin.truthy() {
                let env: Vec<String> = self.server_env.iter().flat_map(|(k, v)| ["--env".to_string(), format!("{k}={v}")]).collect();
                let tail = |extra: Option<&str>| {
                    let mut t = vec!["--".to_string(), self.o.bin.clone()];
                    t.extend(list_strings(&self.mcp["args"]));
                    if let Some(e) = extra {
                        t.push(e.into());
                    }
                    t
                };
                let _ = Self::run(c, &bin, vec!["mcp".into(), "remove".into(), xenonet.clone()]);
                let mut args = vec!["mcp".to_string(), "add".into(), xenonet.clone()];
                args.extend(env.clone());
                args.extend(tail(None));
                Self::run(c, &bin, args)?;
                if self.o.console_mcp {
                    let _ = Self::run(c, &bin, vec!["mcp".into(), "remove".into(), console.clone()]);
                    let mut args = vec!["mcp".to_string(), "add".into(), console.clone()];
                    args.extend(env);
                    args.extend(tail(Some("console")));
                    Self::run(c, &bin, args)?;
                }
            }
        }
        self.preview(c)
    }

    pub fn remove(&self, c: &mut dyn Calls) -> Result<Value> {
        let settings = self.read_json(c, &self.claude_settings)?;
        let merged = merge_hooks(c, &settings, &mut |c, h| self.mine(c, Kind::Claude, h), None)?;
        Self::write_json(c, &self.claude_settings, &merged)?;
        if Self::exists(c, &self.codex_hooks)? {
            let hooks = self.read_json(c, &self.codex_hooks)?;
            let merged = merge_hooks(c, &hooks, &mut |c, h| self.mine(c, Kind::Codex, h), None)?;
            Self::write_json(c, &self.codex_hooks, &merged)?;
        }
        let claude = Self::cli(c, "claude")?;
        let codex = Self::cli(c, "codex")?;
        let (xenonet, console) = self.names.clone();
        if claude.truthy() {
            let _ = Self::run(c, &claude, vec!["mcp".into(), "remove".into(), xenonet.clone(), "--scope".into(), "user".into()]);
        }
        if codex.truthy() {
            let _ = Self::run(c, &codex, vec!["mcp".into(), "remove".into(), xenonet.clone()]);
        }
        if self.o.console_mcp {
            if claude.truthy() {
                let _ = Self::run(c, &claude, vec!["mcp".into(), "remove".into(), console.clone(), "--scope".into(), "user".into()]);
            }
            if codex.truthy() {
                let _ = Self::run(c, &codex, vec!["mcp".into(), "remove".into(), console.clone()]);
            }
        }
        self.preview(c)
    }
}

/// What the preview says while connect's own entries are in Claude Code's
/// settings beside the plugin's.
const TWICE: &str = "xenonet connect's own entries are in Claude Code's settings beside the plugin's. XenoNet ignores the plugin's copy of any hook connect added, so each event still arrives once. Connecting takes connect's entries out.";

/// What the preview says when the plugin can't find the command it runs.
const NO_COMMAND: &str = "XenoNet's plugin runs the xenonet command, which isn't on your PATH";

/// What the preview says of entries an earlier XenoNet wrote.
const EARLIER: &str = "XenoNet's entries here run cli/xenonet.js with Node, as an earlier XenoNet wrote them. Connecting again has them run the xenonet binary.";

/// A hooks change's state: as XenoNet would add them, or there but not
/// so (an earlier XenoNet's, say), or not there.
fn hooks_state(in_place: bool, own: bool) -> &'static str {
    if in_place {
        "present"
    } else if own {
        "different"
    } else {
        "missing"
    }
}

fn group(matcher: Option<&str>, hook: Value) -> Value {
    let mut g = Map::new();
    if let Some(m) = matcher {
        g.insert("matcher".into(), json!(m));
    }
    g.insert("hooks".into(), Value::Array(vec![hook]));
    Value::Object(g)
}

fn list_strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|x| text_of(Some(x))).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoCalls;
    impl Calls for NoCalls {
        fn call(&mut self, _: &str, _: &str, _: Vec<Arg>) -> Result<Arg> {
            Ok(Arg::Bool(true))
        }
        fn get(&mut self, _: &str) -> Result<Arg> {
            Ok(Arg::Undef)
        }
        fn kind(&mut self, _: &str) -> Result<String> {
            Ok("undefined".into())
        }
    }

    /// Files by path, as node:fs reads them: a missing one is ENOENT.
    struct Files(std::collections::HashMap<String, String>);
    impl Calls for Files {
        fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
            let file = args.first().and_then(Arg::as_str).unwrap_or_default().to_string();
            match (path, method) {
                ("fs", "readFileSync") => self.0.get(&file).map(|t| Arg::text(t.clone())).ok_or_else(|| Thrown::coded("no such file", "ENOENT")),
                ("fs", "existsSync") => Ok(Arg::Bool(self.0.contains_key(&file))),
                ("", "cli") => Ok(Arg::Null),
                _ => Err(Thrown::new(format!("connect asked for {path}.{method}"))),
            }
        }
        fn get(&mut self, _: &str) -> Result<Arg> {
            Ok(Arg::Undef)
        }
        fn kind(&mut self, _: &str) -> Result<String> {
            Ok("undefined".into())
        }
    }

    fn at(port: f64) -> Connect {
        Connect::new(Options {
            port: Arg::num(port),
            demo: Arg::Bool(false),
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![
                Instance { id: "installed".into(), name: "XenoNet".into(), port: Arg::num(47840.0) },
                Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: Arg::num(47842.0) },
            ],
        })
    }

    #[test]
    fn the_plugin_counts_when_installed_and_turned_on_and_only_for_the_installed_xenonet() {
        let installed = r#"{ "version": 2, "plugins": { "xenonet@xenonet": [{ "scope": "user" }] } }"#;
        // The paths as this machine joins them.
        let (settings_file, list_file) = (at(47840.0).claude_settings, at(47840.0).claude_plugins);
        let files = |enabled: &str, list: Option<&str>| {
            let mut m = std::collections::HashMap::new();
            m.insert(settings_file.clone(), format!(r#"{{ "enabledPlugins": {enabled} }}"#));
            if let Some(l) = list {
                m.insert(list_file.clone(), l.to_string());
            }
            Files(m)
        };
        let plugin = |c: &Connect, mut f: Files| {
            let settings = c.read_json(&mut f, &c.claude_settings).unwrap();
            c.plugin(&mut f, &settings)
        };
        let on = r#"{ "xenonet@xenonet": true }"#;
        assert_eq!(plugin(&at(47840.0), files(on, Some(installed))), Some("xenonet@xenonet".into()));
        assert_eq!(plugin(&at(47842.0), files(on, Some(installed))), None, "XenoNet Dev's own connect");
        assert_eq!(plugin(&at(47840.0), files(r#"{ "xenonet@xenonet": false }"#, Some(installed))), None, "turned off");
        assert_eq!(plugin(&at(47840.0), files(on, None)), None, "not installed");
        assert_eq!(plugin(&at(47840.0), files(on, Some("{ not json"))), None, "a list that can't be read");
        assert_eq!(plugin(&at(47840.0), files(r#"{ "xenonet-tools@xenonet": true }"#, Some(installed))), None, "another plugin");
    }

    #[test]
    fn connects_own_hook_for_an_event_is_there_only_when_it_can_run() {
        let c = at(47840.0);
        let start = c.claude_hooks["SessionStart"].clone();
        let stop = c.claude_hooks["Stop"].clone();
        let settings = json!({ "hooks": { "SessionStart": [start], "Stop": [stop] } }).to_string();
        let mut m = std::collections::HashMap::new();
        m.insert(c.claude_settings.clone(), settings);
        let mut f = Files(m.clone());
        assert!(c.hook_connected(&mut f, "Stop").unwrap());
        assert!(!c.hook_connected(&mut f, "SessionStart").unwrap(), "the binary it runs is gone");
        assert!(!c.hook_connected(&mut f, "Notification").unwrap());
        assert!(!at(47842.0).hook_connected(&mut f, "Stop").unwrap(), "another XenoNet's");
        m.insert("/x/xenonet".to_string(), String::new());
        assert!(c.hook_connected(&mut Files(m.clone()), "SessionStart").unwrap());
        m.insert(c.claude_settings.clone(), "{ not json".into());
        assert!(!c.hook_connected(&mut Files(m), "Stop").unwrap(), "settings that can't be read");
    }

    #[test]
    fn dirname_works_as_nodes_does() {
        for (p, d) in [("/a/b", "/a"), ("/a//b", "/a/"), ("/", "/"), ("a", "."), ("", "."), ("//a", "//"), ("/a/b/", "/a")] {
            assert_eq!(dirname(p), d, "{p}");
        }
    }

    #[test]
    fn merging_takes_ours_out_and_puts_new_ones_in() {
        let settings = json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:47840/api/hook/claude" }, { "type": "command", "command": "mine" }] }], "Other": [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:1/api/hook/claude" }] }] } });
        let out = merge_hooks(
            &mut NoCalls,
            &settings,
            &mut |_, h| Ok(h.get("type").and_then(Value::as_str) == Some("http")),
            Some(&json!({ "Stop": { "hooks": [{ "type": "http" }] } })),
        )
        .unwrap();
        assert_eq!(json_text(&out), r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]},{"hooks":[{"type":"http"}]}]}}"#);
        assert_eq!(merge_hooks(&mut NoCalls, &json!({ "hooks": {} }), &mut |_, _| Ok(true), None).unwrap(), json!({}));
    }

    #[test]
    fn hooks_say_which_xenonet_theyre_for() {
        let c = Connect::new(Options {
            port: Arg::num(47842.0),
            demo: Arg::Bool(false),
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![
                Instance { id: "installed".into(), name: "XenoNet".into(), port: Arg::num(47840.0) },
                Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: Arg::num(47842.0) },
            ],
        });
        assert_eq!(c.names(), &("xenonet-dev".to_string(), "xenonet-dev-console".to_string()));
        let start = &c.claude_hooks["SessionStart"]["hooks"][0];
        assert_eq!(start["command"], json!("XENONET_PORT=47842 '/x/xenonet' hook claude"));
        assert!(Connect::xenonet_hook(Kind::Claude, start));
        assert_eq!(Connect::port_of(start), Some(47842.0));
        assert_eq!(c.mcp, json!({ "type": "stdio", "command": "/x/xenonet", "args": ["mcp"], "env": { "XENONET_PORT": "47842" } }));
        // An earlier XenoNet's hook, which ran Node and cli/xenonet.js, is
        // still a XenoNet's, and says so.
        let node = json!({ "type": "command", "command": "XENONET_PORT=47842 ELECTRON_RUN_AS_NODE=1 '/x/node' '/app/cli/xenonet.js' hook claude" });
        assert!(Connect::xenonet_hook(Kind::Claude, &node));
        let hooks = json!({ "SessionStart": [{ "hooks": [node] }] });
        assert!(c.has_own(Some(&hooks), Kind::Claude) && c.earlier(Some(&hooks), Kind::Claude, None));
        assert!(!c.earlier(Some(&c.claude_hooks), Kind::Claude, Some(&c.mcp)));
        assert!(c.earlier(None, Kind::Codex, Some(&json!({ "command": "/x/node", "args": ["/app/cli/xenonet.js", "mcp"] }))));
        // Another XenoNet's binary, and a Windows one's, are XenoNet's too.
        assert!(Connect::xenonet_hook(Kind::Codex, &json!({ "type": "command", "command": "XENONET_PORT=47840 '/r/launcher/xenonet' hook codex" })));
        assert!(Connect::xenonet_hook(Kind::Codex, &json!({ "type": "command", "command": r"XENONET_PORT=47840 'C:\r\launcher\xenonet.exe' hook codex" })));
        assert!(!Connect::xenonet_hook(Kind::Codex, &json!({ "type": "command", "command": "my-xenonet-thing hook codex" })));
        let win = c.windows_hook("codex");
        let h = json!({ "type": "command", "command": "x", "commandWindows": win });
        assert!(Connect::hook_text(&h).starts_with("# XenoNet hook codex\n$env:XENONET_PORT='47842'"));
        assert!(Connect::xenonet_hook(Kind::Codex, &h));
    }
}
