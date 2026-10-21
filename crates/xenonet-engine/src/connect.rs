//! Connecting your agents: what XenoNet adds to Claude Code's, Codex's and
//! Grok Build's own settings so every session on this machine, in every
//! project, can use the engine: core/engine/connect.js. Nothing changes until you
//! confirm in XenoNet, which first shows exactly what will be added
//! (preview), and it can all be taken out again (remove).
//!
//!   Claude Code  the xenonet MCP server at user scope (through `claude
//!                mcp`), and HTTP hooks in ~/.claude/settings.json
//!   Codex        the xenonet MCP server (through `codex mcp`), and command
//!                hooks in ~/.codex/hooks.json
//!   Grok Build   the xenonet MCP server as tables in ~/.grok/config.toml,
//!                which XenoNet writes itself (toml.rs), and command hooks
//!                in a file of XenoNet's own, ~/.grok/hooks/xenonet.json
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
//! Which apps connect goes through are the app registry's: those that have
//! `connect`, in its order, each through the config format its entry names
//! (`format`: [`ClaudeSettings`], [`CodexHooks`], [`GrokHooks`]).
//!
//! The files and the CLIs are the twin's, called as the JS called them,
//! through `Calls`: "fs" (node:fs), "run" (a CLI, its output) and "cli" (a
//! CLI on PATH, or null).

use crate::js::{base64, base64_bytes, from_utf16le, utf16le};
use crate::peer::{Calls, Thrown};
use crate::sql::Arg;
use crate::store::js_text;
use serde_json::{json, Map, Value};
use xenonet_base::apps::App;
use xenonet_base::json::stringify;
use xenonet_base::paths::tilde;

/// XenoNet's plugin for Claude Code, which Claude Code knows as
/// `xenonet@<marketplace>` once it's installed.
const PLUGIN: &str = "xenonet";

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
    pub grok_home: Option<String>,
    pub console_mcp: bool,
    pub platform: String,
    pub instances: Vec<Instance>,
}

/// What every format shares (createConnect's `k`): this XenoNet's
/// options, the MCP server it adds, and how its entries are told apart.
pub struct Shared {
    o: Options,
    server_env: Vec<(String, String)>,
    mcp: Value,
    names: (String, String),
}

impl Shared {
    pub fn new(o: Options) -> Self {
        let server_env = vec![("XENONET_PORT".to_string(), js_text(&o.port))];
        let env_obj: Map<String, Value> = server_env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let mcp = json!({ "type": "stdio", "command": o.bin, "args": ["mcp"], "env": Value::Object(env_obj) });
        let names = server_names(&o.port, &o.instances);
        Shared { o, server_env, mcp, names }
    }

    pub fn options(&self) -> &Options {
        &self.o
    }

    /// The names of this XenoNet's MCP servers: its own and its console's.
    pub fn names(&self) -> &(String, String) {
        &self.names
    }

    /// The MCP server XenoNet adds, as an app's settings hold it.
    pub fn mcp(&self) -> &Value {
        &self.mcp
    }

    fn port(&self) -> String {
        js_text(&self.o.port)
    }

    /// The xenonet binary, with the home folder as ~.
    pub fn command(&self) -> String {
        tilde(&self.o.bin, &self.o.home)
    }

    /// The command a hook of `app`'s runs.
    pub fn hook_command(&self, app: &str) -> String {
        let env: Vec<String> = self.server_env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("{} {} hook {app}", env.join(" "), quote(&self.o.bin))
    }

    /// The same, as PowerShell runs it on Windows.
    pub fn windows_hook(&self, app: &str) -> String {
        let env: Vec<String> = self.server_env.iter().map(|(k, v)| format!("$env:{k}={}", ps_quote(v))).collect();
        let script = format!("# XenoNet hook {app}\n{}; & {} hook {}; exit $LASTEXITCODE", env.join("; "), ps_quote(&self.o.bin), ps_quote(app));
        format!("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}", base64(&utf16le(&script)))
    }

    // ---------- whose an entry is ----------

    pub fn hook_text(h: &Value) -> String {
        let command = match h.get("commandWindows").filter(|v| !v.is_null()) {
            Some(v) => text_of(Some(v)),
            None => text_of(h.get("command")),
        };
        match crate::re!(r"-EncodedCommand ([A-Za-z0-9+/=]+)$").captures(&command) {
            Some(m) => from_utf16le(&base64_bytes(&m[1])),
            None => command,
        }
    }

    /// Any XenoNet's command entries for `app`, whichever XenoNet or
    /// checkout added them: the binary's, cli/xenonet.js's with Node, or
    /// the hook.js of earlier versions.
    pub fn xenonet_command(app: &str, h: &Value) -> bool {
        if h.get("type").and_then(Value::as_str) != Some("command") {
            return false;
        }
        let command = text_of(h.get("command"));
        // `(<one of the forms>) <app>$`: the command ends with the app,
        // after one of the forms.
        let form = crate::re!(r#"(cli[\\/]xenonet\.js['"]? hook|(cli|server[\\/]engine)[\\/]hook\.js['"]?|[\\/]xenonet(\.exe)?['"]? hook)$"#);
        if command.strip_suffix(app).and_then(|c| c.strip_suffix(' ')).is_some_and(|c| form.is_match(c)) {
            return true;
        }
        let text = Self::hook_text(h);
        text.starts_with(&format!("# XenoNet hook {app}\n")) && text.ends_with(&format!(" hook '{app}'; exit $LASTEXITCODE"))
    }

    /// Which XenoNet an entry is for: its port, from its URL or
    /// XENONET_PORT; None for an early form that didn't say.
    pub fn port_of(h: &Value) -> Option<f64> {
        let m = if h.get("type").and_then(Value::as_str) == Some("http") {
            crate::re!(r":([0-9]+)/api/hook/").captures(&text_of(h.get("url"))).map(|m| m[1].to_string())
        } else {
            crate::re!(r#"XENONET_PORT=['"]?([0-9]+)"#).captures(&Self::hook_text(h)).map(|m| m[1].to_string())
        };
        m.map(|d| xenonet_base::text::js_number(Some(&Value::String(d))))
    }

    /// Whether an entry is for this XenoNet's port, or names none.
    fn this_port(&self, h: &Value) -> bool {
        Self::port_of(h).is_none_or(|p| matches!(self.o.port, Arg::Num(q) if q == p))
    }

    /// Only the paths among a command's quoted words.
    pub fn hook_files(hooks: Option<&Value>, ours: &mut dyn FnMut(&Value) -> bool) -> Vec<String> {
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

    pub fn exists(c: &mut dyn Calls, f: &str) -> Result<bool> {
        Ok(c.call("fs", "existsSync", vec![Arg::text(f)])?.truthy())
    }

    /// An entry is this XenoNet's when it's for this port, or says no
    /// port, or runs files that are gone; another live XenoNet's stays as
    /// it is. `is` tells a format's XenoNet entries from the rest.
    pub fn mine(&self, c: &mut dyn Calls, is: &dyn Fn(&Value) -> bool, h: &Value) -> Result<bool> {
        if !is(h) {
            return Ok(false);
        }
        if self.this_port(h) {
            return Ok(true);
        }
        let one = json!({ "x": [{ "hooks": [h] }] });
        for f in Self::hook_files(Some(&one), &mut |h| is(h)) {
            if !Self::exists(c, &f)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether this XenoNet has hooks here, in place or not: its port's,
    /// or an early form's that named none.
    pub fn has_own(&self, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool) -> bool {
        flat(values(hooks)).iter().flat_map(hooks_of).any(|h| is(&h) && self.this_port(&h))
    }

    /// Whether this XenoNet's entries here are an earlier XenoNet's, which
    /// run Node and cli/xenonet.js (or hook.js) rather than the binary: a
    /// hook of its own, or its MCP server.
    pub fn earlier(&self, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool, server: Option<&Value>) -> bool {
        let node = |t: &str| t.contains("xenonet.js") || t.contains("hook.js");
        let own = flat(values(hooks))
            .iter()
            .flat_map(hooks_of)
            .any(|h| h.get("type").and_then(Value::as_str) == Some("command") && is(&h) && self.this_port(&h) && node(&Self::hook_text(&h)));
        own || server.and_then(|s| s.get("args")).and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| crate::re!(r"\.m?js$").is_match(&text_of(Some(x)))))
    }

    /// Other XenoNets connected here too, by port, for the preview.
    pub fn others(&self, c: &mut dyn Calls, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool) -> Result<Vec<Option<f64>>> {
        let mut out: Vec<Option<f64>> = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if is(&h) && !self.mine(c, is, &h)? {
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
    pub fn read_json(&self, c: &mut dyn Calls, file: &str) -> Result<Value> {
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

    pub fn write_json(c: &mut dyn Calls, file: &str, data: &Value) -> Result<()> {
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

    /// A settings object with this XenoNet's entries (by `is`) taken out,
    /// and `add` put in when given.
    pub fn merge(&self, c: &mut dyn Calls, settings: &Value, is: &dyn Fn(&Value) -> bool, add: Option<&Value>) -> Result<Value> {
        merge_hooks(c, settings, &mut |c, h| self.mine(c, is, h), add)
    }

    /// A CLI on PATH, or null.
    pub fn cli(c: &mut dyn Calls, name: &str) -> Result<Arg> {
        c.call("", "cli", vec![Arg::text(name)])
    }

    pub fn run(c: &mut dyn Calls, bin: &Arg, args: Vec<String>) -> Result<Arg> {
        c.call("", "run", vec![bin.clone(), Arg::List(args.into_iter().map(Arg::text).collect())])
    }

    // ---------- what's there ----------

    /// The xenonet server as each app has it: missing, present (as XenoNet
    /// would add it) or different.
    pub fn server_state(&self, s: Option<&Value>, args: Option<&Value>) -> &'static str {
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

    /// Whether every group in `add` is already in place.
    fn hooks_in_place(&self, c: &mut dyn Calls, settings: &Value, is: &dyn Fn(&Value) -> bool, add: &Value) -> Result<bool> {
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
                    if self.mine(c, is, h)? && json_text(h) == first {
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

    /// A hooks change's state: as XenoNet would add them, or there but not
    /// so (an earlier XenoNet's, say), or not there.
    pub fn hooks_state(&self, c: &mut dyn Calls, settings: &Value, is: &dyn Fn(&Value) -> bool, add: &Value) -> Result<&'static str> {
        let in_place = self.hooks_in_place(c, settings, is, add)?;
        Ok(if in_place {
            "present"
        } else if self.has_own(settings.get("hooks"), is) {
            "different"
        } else {
            "missing"
        })
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

    /// What an app's entries run that's gone: its server's files, and its
    /// hooks' (this XenoNet's own, by `is`).
    pub fn broken(&self, c: &mut dyn Calls, entry: Option<&Value>, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool) -> Result<Vec<Value>> {
        let mut files = match entry.filter(|e| crate::js::truthy(Some(e))) {
            Some(e) => Self::gone(c, e)?,
            None => Vec::new(),
        };
        // hookFiles with this XenoNet's own entries, which may look at files.
        let mut mine_list = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if self.mine(c, is, &h)? {
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

    /// Another XenoNet on this machine connected too: its entries stay as
    /// they are.
    pub fn besides(&self, ports: &[Option<f64>]) -> Vec<Value> {
        ports
            .iter()
            .map(|p| {
                let shown = p.map_or("null".to_string(), |n| js_text(&Arg::num(n)));
                let name = self.o.instances.iter().find(|i| matches!((&i.port, p), (Arg::Num(a), Some(b)) if a == b)).map(|i| format!(", {}", i.name)).unwrap_or_default();
                Value::String(format!("Another XenoNet (port {shown}{name}) is connected here too; its entries stay as they are."))
            })
            .collect()
    }

    pub fn arg_json(a: &Arg) -> Value {
        match a {
            Arg::Num(n) => xenonet_base::json::number(*n),
            other => other.to_json(),
        }
    }

    /// The console operator's MCP server, when it's XenoNet's too.
    pub fn console_change(&self, app: &str, s: Option<&Value>) -> Vec<Value> {
        if !self.o.console_mcp {
            return Vec::new();
        }
        let mut args = self.mcp["args"].as_array().cloned().unwrap_or_default();
        args.push(json!("console"));
        let (_, console) = &self.names;
        vec![
            json!({ "id": "console", "what": format!("The {console} MCP server, to operate Desktop"), "where": format!("{app} user settings"), "state": self.server_state(s, Some(&Value::Array(args))), "detail": format!("{} mcp console (XENONET_PORT={})", self.command(), self.port()) }),
        ]
    }
}

/// The groups XenoNet adds for an app's hook events, each with its
/// matcher when it takes one: `hook(event)` is the hook in each.
pub fn hook_set(events: &[(String, Option<String>)], hook: impl Fn(&str) -> Value) -> Value {
    let mut set = Map::new();
    for (event, matcher) in events {
        set.insert(event.clone(), group(matcher.as_deref(), hook(event)));
    }
    Value::Object(set)
}

/// An app's hook events as the preview lists them.
pub fn event_list(events: &[(String, Option<String>)]) -> String {
    events.iter().map(|(e, m)| if m.is_some() { format!("{e} (edits)") } else { e.clone() }).collect::<Vec<_>>().join(", ")
}

/// What the preview found of an app before its part: its command, its
/// xenonet server and its console's.
pub struct Found<'a> {
    pub cli: &'a Arg,
    pub server: Option<&'a Value>,
    pub console: Option<&'a Value>,
}

/// One config format's connect (core/engine/connect.js's FORMATS), made
/// for one app: what XenoNet adds there, and takes out.
pub trait Format: Send + Sync {
    /// The command its MCP server goes in through.
    fn cli(&self) -> &'static str;
    /// The MCP server of that name as the app has it.
    fn server(&self, k: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) -> Result<Option<Value>>;
    /// The app's part of the preview.
    fn entry(&self, k: &Shared, c: &mut dyn Calls, found: Found<'_>) -> Result<Value>;
    /// Puts XenoNet's hooks and servers in.
    fn apply(&self, k: &Shared, c: &mut dyn Calls) -> Result<()>;
    /// Takes XenoNet's hooks out.
    fn remove_hooks(&self, k: &Shared, c: &mut dyn Calls) -> Result<()>;
    /// Takes the MCP server of that name out.
    fn remove_server(&self, k: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str);
    /// Whether connect's own hook for `event` is in place, for a format
    /// XenoNet's plugin carries a copy of (Claude Code's settings); None
    /// for the others.
    fn hook_connected(&self, _k: &Shared, _c: &mut dyn Calls, _event: &str) -> Option<Result<bool>> {
        None
    }
    /// For an app that may not be here (Grok Build): whether it is, so
    /// connecting every app leaves it out until it is.
    fn installed(&self, _k: &Shared, _c: &mut dyn Calls) -> Result<bool> {
        Ok(true)
    }
}

/// A format's connect for an app.
pub type Maker = fn(&App, &Shared) -> Box<dyn Format>;

/// The formats XenoNet connects an app through, by the name its registry
/// entry gives (connect.format).
pub fn format(name: &str) -> Option<Maker> {
    match name {
        "claude-settings" => Some(ClaudeSettings::make),
        "codex-hooks" => Some(CodexHooks::make),
        "grok-hooks" => Some(GrokHooks::make),
        _ => None,
    }
}

fn events_of(app: &App) -> Vec<(String, Option<String>)> {
    app.connect.as_ref().map(|c| c.events.clone()).unwrap_or_default()
}

/// Claude Code's settings (claude-settings): the xenonet MCP server at
/// user scope, through `claude mcp`, kept in .claude.json; HTTP hooks in
/// settings.json; and XenoNet's plugin for Claude Code, which carries both
/// for the installed XenoNet.
pub struct ClaudeSettings {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    settings_file: String,
    state_file: String,
    /// Claude Code's list of the plugins it has installed.
    plugins_file: String,
    /// Whether XenoNet's plugin, which reaches the installed XenoNet, is
    /// for this XenoNet.
    plugin_port: bool,
    hook_url: String,
    hooks: Value,
}

impl ClaudeSettings {
    pub fn make(app: &App, k: &Shared) -> Box<dyn Format> {
        Box::new(Self::new(app, k))
    }

    pub fn new(app: &App, k: &Shared) -> Self {
        let o = &k.o;
        let dir = o.claude_config_dir.clone().unwrap_or_else(|| join(&o.home, ".claude"));
        // Claude Code keeps user-scope MCP servers in .claude.json, in
        // CLAUDE_CONFIG_DIR when that's set.
        let state_file = match o.claude_config_dir.as_deref().filter(|d| !d.is_empty()) {
            Some(d) => join(d, ".claude.json"),
            None => join(&o.home, ".claude.json"),
        };
        let hook_url = format!("http://127.0.0.1:{}/api/hook/{}", k.port(), app.id);
        let win = o.platform == "win32";
        let events = events_of(app);
        // Claude Code takes HTTP hooks, which cost nothing to run, except at
        // SessionStart, which takes only commands.
        let hooks = hook_set(&events, |event| {
            if event == "SessionStart" {
                let mut h = Map::new();
                h.insert("type".into(), json!("command"));
                h.insert("command".into(), json!(if win { k.windows_hook(&app.id) } else { k.hook_command(&app.id) }));
                if win {
                    h.insert("shell".into(), json!("powershell"));
                }
                h.insert("timeout".into(), json!(10));
                Value::Object(h)
            } else {
                json!({ "type": "http", "url": hook_url, "timeout": 5 })
            }
        });
        ClaudeSettings {
            id: app.id.clone(),
            name: app.name.clone(),
            events,
            settings_file: join(&dir, "settings.json"),
            state_file,
            plugins_file: join(&join(&dir, "plugins"), "installed_plugins.json"),
            plugin_port: o.instances.iter().any(|i| i.id == "installed" && same_arg(&i.port, &o.port)),
            hook_url,
            hooks,
        }
    }

    /// Any XenoNet's entry for this app: an HTTP hook posting to a
    /// XenoNet's hook route for it, or a command entry.
    fn is_xenonet(&self, h: &Value) -> bool {
        let http = h.get("type").and_then(Value::as_str) == Some("http") && {
            let url = text_of(h.get("url"));
            crate::re!(r"^http://(127\.0\.0\.1|localhost):[0-9]+/api/hook/").find(&url).is_some_and(|m| url[m.end()..] == self.id)
        };
        http || Shared::xenonet_command(&self.id, h)
    }

    /// XenoNet's plugin for Claude Code, as Claude Code knows it
    /// (`xenonet@<marketplace>`), when it's installed and turned on in
    /// Claude Code's user settings. The plugin reaches the installed
    /// XenoNet, so only that XenoNet counts it. A list of plugins that
    /// can't be read counts as none.
    fn plugin(&self, k: &Shared, c: &mut dyn Calls, settings: &Value) -> Option<String> {
        if !self.plugin_port {
            return None;
        }
        let enabled = settings.get("enabledPlugins").and_then(Value::as_object)?;
        let installed = k.read_json(c, &self.plugins_file).ok()?;
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
}

impl Format for ClaudeSettings {
    fn cli(&self) -> &'static str {
        "claude"
    }

    fn server(&self, k: &Shared, c: &mut dyn Calls, _cli: &Arg, name: &str) -> Result<Option<Value>> {
        let state = k.read_json(c, &self.state_file)?;
        Ok(state.get("mcpServers").filter(|m| !m.is_null()).and_then(|m| m.get(name)).cloned())
    }

    fn entry(&self, k: &Shared, c: &mut dyn Calls, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let port = k.port();
        let home = &k.o.home;
        let command = k.command();
        let mut changes = k.console_change(&self.id, found.console);
        let settings = k.read_json(c, &self.settings_file)?;
        let plugin = self.plugin(k, c, &settings);
        let mcp_what = format!("The {xenonet} MCP server, for every project");
        let mcp_detail = format!("{command} mcp (XENONET_PORT={port})");
        let hooks_what = format!("Hooks: {}", event_list(&self.events));
        let hooks_detail = format!("posting to {}; SessionStart runs {command} hook {}", self.hook_url, self.id);
        let own_hooks = k.has_own(settings.get("hooks"), &is);
        let mut twice = false;
        if let Some(key) = &plugin {
            // The plugin carries the server and the hooks; connect's own
            // copies beside it are to come out.
            let carried = format!("XenoNet's plugin for Claude Code ({key})");
            if k.server_state(found.server, None) == "missing" {
                changes.push(json!({ "id": "mcp", "what": mcp_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("xenonet mcp, from your PATH (XENONET_PORT={port})") }));
            } else {
                twice = true;
                changes.push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, beside the plugin's copy", "state": "remove", "detail": mcp_detail }));
            }
            if own_hooks {
                twice = true;
                changes.push(json!({ "id": "hooks", "what": hooks_what, "where": format!("{}, beside the plugin's copies", tilde(&self.settings_file, home)), "state": "remove", "detail": hooks_detail }));
            } else {
                changes.push(json!({ "id": "hooks", "what": hooks_what, "where": carried, "state": "present", "via": "plugin", "detail": format!("posting to {}?via=plugin; SessionStart runs xenonet hook {}", self.hook_url, self.id) }));
            }
        } else {
            changes.push(json!({ "id": "mcp", "what": mcp_what, "where": "Claude Code's user settings, through claude mcp add-json --scope user", "state": k.server_state(found.server, None), "detail": mcp_detail }));
            let state = k.hooks_state(c, &settings, &is, &self.hooks)?;
            changes.push(json!({ "id": "hooks", "what": hooks_what, "where": tilde(&self.settings_file, home), "state": state, "detail": hooks_detail }));
        }
        // With the plugin, connecting adds nothing through the claude
        // command, so whether it's there doesn't matter.
        let mut notes: Vec<Value> = if found.cli.truthy() || plugin.is_some() {
            Vec::new()
        } else {
            vec![json!("XenoNet can't find the claude command, so it can add the hooks but not the MCP server. Install Claude Code's CLI, or add the server by hand.")]
        };
        if let Some(key) = &plugin {
            notes.push(json!(format!("Claude Code has XenoNet's plugin ({key}), which carries the {xenonet} MCP server and the hooks, so connecting leaves them to it.")));
        }
        if twice {
            notes.push(json!(TWICE));
        }
        if k.earlier(settings.get("hooks"), &is, found.server) {
            notes.push(json!(EARLIER));
        }
        let others = k.others(c, settings.get("hooks"), &is)?;
        notes.extend(k.besides(&others));
        let mut broken = k.broken(c, found.server, settings.get("hooks"), &is)?;
        if plugin.is_some() && !Shared::cli(c, "xenonet")?.truthy() {
            broken.push(json!(NO_COMMAND));
        }
        Ok(json!({ "app": self.id, "name": self.name, "cli": Shared::arg_json(found.cli), "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, console) = k.names();
        let settings = k.read_json(c, &self.settings_file)?;
        // With XenoNet's plugin there, connect's own copies of the server
        // and the hooks come out, and none go in.
        let plugin = self.plugin(k, c, &settings).is_some();
        if !plugin {
            let merged = k.merge(c, &settings, &is, Some(&self.hooks))?;
            Shared::write_json(c, &self.settings_file, &merged)?;
        } else if k.has_own(settings.get("hooks"), &is) {
            let merged = k.merge(c, &settings, &is, None)?;
            Shared::write_json(c, &self.settings_file, &merged)?;
        }
        let bin = Shared::cli(c, "claude")?;
        if bin.truthy() {
            let _ = Shared::run(c, &bin, vec!["mcp".into(), "remove".into(), xenonet.clone(), "--scope".into(), "user".into()]);
            if !plugin {
                Shared::run(c, &bin, vec!["mcp".into(), "add-json".into(), xenonet.clone(), json_text(k.mcp()), "--scope".into(), "user".into()])?;
            }
            if k.o.console_mcp {
                let mut console_mcp = k.mcp().clone();
                if let Some(Value::Array(a)) = console_mcp.get_mut("args") {
                    a.push(json!("console"));
                }
                let _ = Shared::run(c, &bin, vec!["mcp".into(), "remove".into(), console.clone(), "--scope".into(), "user".into()]);
                Shared::run(c, &bin, vec!["mcp".into(), "add-json".into(), console.clone(), json_text(&console_mcp), "--scope".into(), "user".into()])?;
            }
        }
        Ok(())
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        let settings = k.read_json(c, &self.settings_file)?;
        let merged = k.merge(c, &settings, &|h| self.is_xenonet(h), None)?;
        Shared::write_json(c, &self.settings_file, &merged)
    }

    fn remove_server(&self, _k: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) {
        if cli.truthy() {
            let _ = Shared::run(c, cli, vec!["mcp".into(), "remove".into(), name.into(), "--scope".into(), "user".into()]);
        }
    }

    /// Whether this XenoNet's own Claude Code hook for `event`, as connect
    /// adds it, is in Claude Code's settings and can run (a command's files
    /// are all there). While it is, the copy XenoNet's plugin posts stands
    /// by, so the event reaches XenoNet once. Settings that can't be read
    /// hold no hook Claude Code runs.
    fn hook_connected(&self, k: &Shared, c: &mut dyn Calls, event: &str) -> Option<Result<bool>> {
        let Ok(settings) = k.read_json(c, &self.settings_file) else {
            return Some(Ok(false));
        };
        let groups = match settings.get("hooks").and_then(|h| h.get(event)) {
            Some(Value::Array(a)) => a.clone(),
            _ => return Some(Ok(false)),
        };
        for h in groups.iter().flat_map(hooks_of) {
            if !self.is_xenonet(&h) || !k.this_port(&h) {
                continue;
            }
            let one = json!({ "x": [{ "hooks": [h] }] });
            let mut runs = true;
            for f in Shared::hook_files(Some(&one), &mut |_| true) {
                match Shared::exists(c, &f) {
                    Ok(true) => {}
                    Ok(false) => runs = false,
                    Err(e) => return Some(Err(e)),
                }
            }
            if runs {
                return Some(Ok(true));
            }
        }
        Some(Ok(false))
    }
}

/// Codex's hooks (codex-hooks): the xenonet MCP server through `codex
/// mcp`, and command hooks in hooks.json, which Codex asks you to trust
/// once.
pub struct CodexHooks {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    hooks_file: String,
    hooks: Value,
}

impl CodexHooks {
    pub fn make(app: &App, k: &Shared) -> Box<dyn Format> {
        Box::new(Self::new(app, k))
    }

    pub fn new(app: &App, k: &Shared) -> Self {
        let o = &k.o;
        let win = o.platform == "win32";
        let events = events_of(app);
        // Codex allows SessionEnd hooks 3 seconds at most.
        let hooks = hook_set(&events, |event| {
            let mut h = Map::new();
            h.insert("type".into(), json!("command"));
            h.insert("command".into(), json!(k.hook_command(&app.id)));
            if win {
                h.insert("commandWindows".into(), json!(k.windows_hook(&app.id)));
            }
            h.insert("timeout".into(), json!(if event == "SessionEnd" { 3 } else { 10 }));
            Value::Object(h)
        });
        CodexHooks { id: app.id.clone(), name: app.name.clone(), events, hooks_file: join(&o.codex_home.clone().unwrap_or_else(|| join(&o.home, ".codex")), "hooks.json"), hooks }
    }

    fn is_xenonet(&self, h: &Value) -> bool {
        Shared::xenonet_command(&self.id, h)
    }
}

impl Format for CodexHooks {
    fn cli(&self) -> &'static str {
        "codex"
    }

    fn server(&self, _k: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) -> Result<Option<Value>> {
        if !cli.truthy() {
            return Ok(None);
        }
        let r = Shared::run(c, cli, vec!["mcp".into(), "get".into(), name.into(), "--json".into()]).unwrap_or(Arg::Null);
        if !r.truthy() {
            return Ok(None);
        }
        Ok(Some(match xenonet_base::json::parse(&js_text(&r)) {
            Ok(Value::Null) | Err(_) => json!({ "command": null }),
            Ok(s) => match s.get("transport").filter(|t| !t.is_null()) {
                Some(t) => t.clone(),
                None => s,
            },
        }))
    }

    fn entry(&self, k: &Shared, c: &mut dyn Calls, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let home = &k.o.home;
        let command = k.command();
        let mut changes = k.console_change(&self.id, found.console);
        changes.push(json!({ "id": "mcp", "what": format!("The {xenonet} MCP server, for every project"), "where": "Codex's config, through codex mcp add", "state": k.server_state(found.server, None), "detail": format!("{command} mcp (XENONET_PORT={})", k.port()) }));
        let hooks_file = k.read_json(c, &self.hooks_file)?;
        let state = k.hooks_state(c, &hooks_file, &is, &self.hooks)?;
        changes.push(json!({ "id": "hooks", "what": format!("Hooks: {}", event_list(&self.events)), "where": tilde(&self.hooks_file, home), "state": state, "detail": format!("running {command} hook {}", self.id) }));
        let mut notes: Vec<Value> =
            if found.cli.truthy() { Vec::new() } else { vec![json!("XenoNet can't find the codex command, so it can add the hooks but not the MCP server.")] };
        notes.push(json!("Codex runs new hooks only once you trust them: run /hooks in Codex and trust XenoNet's."));
        let hooks_file = k.read_json(c, &self.hooks_file)?;
        if k.earlier(hooks_file.get("hooks"), &is, found.server) {
            notes.push(json!(EARLIER));
        }
        let others = k.others(c, hooks_file.get("hooks"), &is)?;
        notes.extend(k.besides(&others));
        let hooks_file = k.read_json(c, &self.hooks_file)?;
        let broken = k.broken(c, found.server, hooks_file.get("hooks"), &is)?;
        Ok(json!({ "app": self.id, "name": self.name, "cli": Shared::arg_json(found.cli), "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        let (xenonet, console) = k.names();
        let hooks = k.read_json(c, &self.hooks_file)?;
        let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), Some(&self.hooks))?;
        Shared::write_json(c, &self.hooks_file, &merged)?;
        let bin = Shared::cli(c, "codex")?;
        if bin.truthy() {
            let env: Vec<String> = k.server_env.iter().flat_map(|(key, v)| ["--env".to_string(), format!("{key}={v}")]).collect();
            let tail = |extra: Option<&str>| {
                let mut t = vec!["--".to_string(), k.o.bin.clone()];
                t.extend(list_strings(&k.mcp["args"]));
                if let Some(e) = extra {
                    t.push(e.into());
                }
                t
            };
            let _ = Shared::run(c, &bin, vec!["mcp".into(), "remove".into(), xenonet.clone()]);
            let mut args = vec!["mcp".to_string(), "add".into(), xenonet.clone()];
            args.extend(env.clone());
            args.extend(tail(None));
            Shared::run(c, &bin, args)?;
            if k.o.console_mcp {
                let _ = Shared::run(c, &bin, vec!["mcp".into(), "remove".into(), console.clone()]);
                let mut args = vec!["mcp".to_string(), "add".into(), console.clone()];
                args.extend(env);
                args.extend(tail(Some("console")));
                Shared::run(c, &bin, args)?;
            }
        }
        Ok(())
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        if Shared::exists(c, &self.hooks_file)? {
            let hooks = k.read_json(c, &self.hooks_file)?;
            let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), None)?;
            Shared::write_json(c, &self.hooks_file, &merged)?;
        }
        Ok(())
    }

    fn remove_server(&self, _k: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) {
        if cli.truthy() {
            let _ = Shared::run(c, cli, vec!["mcp".into(), "remove".into(), name.into()]);
        }
    }
}

/// A text file, or "" when there's none. One that can't be read is left
/// alone, as a settings file is.
fn read_text(k: &Shared, c: &mut dyn Calls, file: &str) -> Result<String> {
    let home = &k.o.home;
    match c.call("fs", "readFileSync", vec![Arg::text(file), Arg::text("utf8")]) {
        Ok(t) => Ok(js_text(&t)),
        Err(e) => {
            let code = e.code.as_deref().filter(|v| !v.is_null());
            if code.and_then(Value::as_str) == Some("ENOENT") {
                return Ok(String::new());
            }
            let why = code.map_or_else(|| tilde(&e.message, home), |v| crate::js::template(Some(v)));
            Err(Thrown::new(format!("XenoNet can't read {} ({why}), so it leaves it alone", tilde(file, home))))
        }
    }
}

/// Grok Build's settings (grok-hooks): the xenonet MCP server as tables in
/// config.toml, which XenoNet writes itself (`grok mcp add` rewrites the
/// file and drops its comments; see toml.rs), and command hooks in Claude
/// Code's format in a file of XenoNet's own, hooks/xenonet.json, which Grok
/// runs without asking, as it does every file in its hooks folder.
/// GROK_HOME moves both. Grok gives a session's MCP servers its id
/// (GROK_SESSION_ID), so the server knows whose it is.
pub struct GrokHooks {
    id: String,
    name: String,
    events: Vec<(String, Option<String>)>,
    grok_home: String,
    config_file: String,
    hooks_file: String,
    hooks: Value,
}

impl GrokHooks {
    pub fn make(app: &App, k: &Shared) -> Box<dyn Format> {
        Box::new(Self::new(app, k))
    }

    pub fn new(app: &App, k: &Shared) -> Self {
        let o = &k.o;
        let win = o.platform == "win32";
        let events = events_of(app);
        // Grok runs a hook's command with sh -c, and on Windows under the
        // shell it finds there, so the Windows form is one command line. Its
        // SessionEnd hooks get little time.
        let hooks = hook_set(&events, |event| {
            let command = if win { k.windows_hook(&app.id) } else { k.hook_command(&app.id) };
            json!({ "type": "command", "command": command, "timeout": if event == "SessionEnd" { 3 } else { 10 } })
        });
        let grok_home = o.grok_home.clone().unwrap_or_else(|| join(&o.home, ".grok"));
        GrokHooks {
            id: app.id.clone(),
            name: app.name.clone(),
            events,
            config_file: join(&grok_home, "config.toml"),
            hooks_file: join(&join(&grok_home, "hooks"), "xenonet.json"),
            grok_home,
            hooks,
        }
    }

    fn is_xenonet(&self, h: &Value) -> bool {
        Shared::xenonet_command(&self.id, h)
    }

    fn foreign(&self, k: &Shared, name: &str) -> String {
        format!(
            "{} names mcp_servers.{name} in a form XenoNet doesn't edit. Take it out, or write it as a [mcp_servers.{name}] table, and connect again.",
            tilde(&self.config_file, &k.o.home)
        )
    }

    // Writes config.toml as `edit` makes it: through a copy of the file (so
    // it keeps its mode), with a copy of it as it was the first time
    // XenoNet changes it.
    fn write_config(&self, k: &Shared, c: &mut dyn Calls, edit: &dyn Fn(&str) -> std::result::Result<String, String>) -> Result<()> {
        let before = read_text(k, c, &self.config_file)?;
        let after = edit(&before).map_err(Thrown::new)?;
        if after == before {
            return Ok(());
        }
        c.call("fs", "mkdirSync", vec![Arg::text(dirname(&self.config_file)), Arg::Obj(vec![("recursive".into(), Arg::Bool(true))])])?;
        let tmp = format!("{}.tmp-xenonet", self.config_file);
        if Shared::exists(c, &self.config_file)? {
            let first = format!("{}.before-xenonet", self.config_file);
            if !Shared::exists(c, &first)? {
                c.call("fs", "copyFileSync", vec![Arg::text(&self.config_file), Arg::text(first)])?;
            }
            c.call("fs", "copyFileSync", vec![Arg::text(&self.config_file), Arg::text(&tmp)])?;
        }
        c.call("fs", "writeFileSync", vec![Arg::text(&tmp), Arg::text(after)])?;
        c.call("fs", "renameSync", vec![Arg::text(tmp), Arg::text(&self.config_file)])?;
        Ok(())
    }

    // XenoNet's hooks file: written as the settings files are, and taken
    // out once no XenoNet's hooks are left in it.
    fn write_hooks(&self, k: &Shared, c: &mut dyn Calls, add: Option<&Value>) -> Result<()> {
        let hooks = k.read_json(c, &self.hooks_file)?;
        let merged = k.merge(c, &hooks, &|h| self.is_xenonet(h), add)?;
        if add.is_none() && merged.as_object().is_some_and(Map::is_empty) {
            c.call("fs", "rmSync", vec![Arg::text(&self.hooks_file), Arg::Obj(vec![("force".into(), Arg::Bool(true))])])?;
            return Ok(());
        }
        Shared::write_json(c, &self.hooks_file, &merged)
    }

    fn command_args(k: &Shared, extra: Option<&str>) -> (String, Vec<String>) {
        let mut args = list_strings(&k.mcp["args"]);
        if let Some(e) = extra {
            args.push(e.into());
        }
        (k.o.bin.clone(), args)
    }
}

impl Format for GrokHooks {
    fn cli(&self) -> &'static str {
        "grok"
    }

    fn installed(&self, _k: &Shared, c: &mut dyn Calls) -> Result<bool> {
        Ok(Shared::cli(c, "grok")?.truthy() || Shared::exists(c, &self.grok_home)?)
    }

    fn server(&self, k: &Shared, c: &mut dyn Calls, _cli: &Arg, name: &str) -> Result<Option<Value>> {
        Ok(crate::toml::read_server(&read_text(k, c, &self.config_file)?, name))
    }

    fn entry(&self, k: &Shared, c: &mut dyn Calls, found: Found<'_>) -> Result<Value> {
        let is = |h: &Value| self.is_xenonet(h);
        let (xenonet, _) = k.names();
        let home = &k.o.home;
        let command = k.command();
        let text = read_text(k, c, &self.config_file)?;
        let hooks = k.read_json(c, &self.hooks_file)?;
        let installed = self.installed(k, c)?;
        let foreign = crate::toml::foreign_server(&text, xenonet);
        let mut changes = k.console_change(&self.id, found.console);
        changes.push(json!({
            "id": "mcp",
            "what": format!("The {xenonet} MCP server, for every project"),
            "where": tilde(&self.config_file, home),
            "state": if foreign { "different" } else { k.server_state(found.server, None) },
            "detail": format!("{command} mcp (XENONET_PORT={}), as [mcp_servers.{xenonet}]; the rest of the file stays as it is", k.port()),
        }));
        let state = k.hooks_state(c, &hooks, &is, &self.hooks)?;
        changes.push(json!({ "id": "hooks", "what": format!("Hooks: {}", event_list(&self.events)), "where": tilde(&self.hooks_file, home), "state": state, "detail": format!("running {command} hook {}", self.id) }));
        let mut notes: Vec<Value> = if found.cli.truthy() {
            Vec::new()
        } else {
            vec![json!("XenoNet can't find the grok command. Connecting writes Grok Build's settings all the same, for when you install it.")]
        };
        if foreign {
            notes.push(json!(self.foreign(k, xenonet)));
        }
        notes.push(json!("Grok Build runs the hooks in its hooks folder without asking."));
        let others = k.others(c, hooks.get("hooks"), &is)?;
        notes.extend(k.besides(&others));
        let broken = k.broken(c, found.server, hooks.get("hooks"), &is)?;
        Ok(json!({ "app": self.id, "name": self.name, "cli": Shared::arg_json(found.cli), "installed": installed, "changes": changes, "notes": notes, "broken": broken }))
    }

    fn apply(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        self.write_hooks(k, c, Some(&self.hooks))?;
        let (xenonet, console) = k.names().clone();
        let console_mcp = k.o.console_mcp;
        let env = k.server_env.clone();
        let (bin, args) = Self::command_args(k, None);
        let (_, console_args) = Self::command_args(k, Some("console"));
        self.write_config(k, c, &|text| {
            let out = crate::toml::with_server(text, &xenonet, &bin, &args, &env)?;
            if console_mcp {
                crate::toml::with_server(&out, &console, &bin, &console_args, &env)
            } else {
                Ok(out)
            }
        })
    }

    fn remove_hooks(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
        if Shared::exists(c, &self.hooks_file)? {
            self.write_hooks(k, c, None)?;
        }
        Ok(())
    }

    // Only this XenoNet's servers, by their names: another's stay.
    fn remove_server(&self, k: &Shared, c: &mut dyn Calls, _cli: &Arg, name: &str) {
        if Shared::exists(c, &self.config_file).unwrap_or(false) {
            let _ = self.write_config(k, c, &|text| Ok(crate::toml::without_server(text, name)));
        }
    }
}

/// Names as a sentence lists them: "A, B and C".
fn and_list(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// Connect: the apps of the registry that have `connect`, in its order,
/// each through its format.
pub struct Connect {
    k: Shared,
    apps: Vec<(App, Box<dyn Format>)>,
}

impl Connect {
    pub fn new(o: Options) -> Self {
        Self::with(o, xenonet_base::apps::apps(), &format)
    }

    /// Connect over a registry and formats of a test's own.
    pub fn with(o: Options, registry: &[App], formats: &dyn Fn(&str) -> Option<Maker>) -> Self {
        let k = Shared::new(o);
        let apps = registry
            .iter()
            .filter(|a| a.has("connect"))
            .filter_map(|a| {
                let make = formats(&a.connect.as_ref()?.format)?;
                Some((a.clone(), make(a, &k)))
            })
            .collect();
        Connect { k, apps }
    }

    pub fn names(&self) -> &(String, String) {
        self.k.names()
    }

    /// The ids of the apps connect goes through.
    pub fn ids(&self) -> Vec<String> {
        self.apps.iter().map(|(a, _)| a.id.clone()).collect()
    }

    /// The apps apply connects when it's given none: those that are here.
    pub fn installed_ids(&self, c: &mut dyn Calls) -> Result<Vec<Arg>> {
        let mut out = Vec::new();
        for (app, f) in &self.apps {
            if f.installed(&self.k, c)? {
                out.push(Arg::text(app.id.clone()));
            }
        }
        Ok(out)
    }

    fn clis(&self, c: &mut dyn Calls) -> Result<Vec<Arg>> {
        self.apps.iter().map(|(_, f)| Shared::cli(c, f.cli())).collect()
    }

    /// Each app's command first, then its xenonet server, then its
    /// console's, then its part of the preview, in the registry's order.
    pub fn preview(&self, c: &mut dyn Calls) -> Result<Value> {
        let clis = self.clis(c)?;
        let (xenonet, console) = self.k.names.clone();
        let mut servers = Vec::new();
        for ((_, f), cli) in self.apps.iter().zip(&clis) {
            servers.push(f.server(&self.k, c, cli, &xenonet)?);
        }
        let mut consoles = Vec::new();
        if self.k.o.console_mcp {
            for ((_, f), cli) in self.apps.iter().zip(&clis) {
                consoles.push(f.server(&self.k, c, cli, &console)?);
            }
        }
        let mut apps = Vec::new();
        for (i, ((_, f), cli)) in self.apps.iter().zip(&clis).enumerate() {
            apps.push(f.entry(&self.k, c, Found { cli, server: servers[i].as_ref(), console: consoles.get(i).and_then(Option::as_ref) })?);
        }
        Ok(json!({ "port": Shared::arg_json(&self.k.o.port), "demo": Shared::arg_json(&self.k.o.demo), "apps": apps }))
    }

    pub fn apply(&self, c: &mut dyn Calls, apps: &[Arg]) -> Result<Value> {
        // Agents connected to the demo would see only its made-up sessions.
        if self.k.o.demo.truthy() {
            let names: Vec<&str> = self.apps.iter().map(|(a, _)| a.name.as_str()).collect();
            return Err(Thrown::new(format!("this is the demo, so connect {} from XenoNet, with your own sessions", and_list(&names))));
        }
        for (app, f) in &self.apps {
            if apps.iter().any(|a| a.as_str() == Some(app.id.as_str())) {
                f.apply(&self.k, c)?;
            }
        }
        self.preview(c)
    }

    /// Each app's hooks first, then its server, then its console's.
    pub fn remove(&self, c: &mut dyn Calls) -> Result<Value> {
        for (_, f) in &self.apps {
            f.remove_hooks(&self.k, c)?;
        }
        let clis = self.clis(c)?;
        let (xenonet, console) = self.k.names.clone();
        for ((_, f), cli) in self.apps.iter().zip(&clis) {
            f.remove_server(&self.k, c, cli, &xenonet);
        }
        if self.k.o.console_mcp {
            for ((_, f), cli) in self.apps.iter().zip(&clis) {
                f.remove_server(&self.k, c, cli, &console);
            }
        }
        self.preview(c)
    }

    /// Whether connect's own hook for `event` is in place where XenoNet's
    /// plugin carries a copy of it (Claude Code's settings): false where no
    /// format says.
    pub fn hook_connected(&self, c: &mut dyn Calls, event: &str) -> Result<bool> {
        for (_, f) in &self.apps {
            if let Some(r) = f.hook_connected(&self.k, c, event) {
                return r;
            }
        }
        Ok(false)
    }
}

/// What the preview says while connect's own entries are in Claude Code's
/// settings beside the plugin's.
const TWICE: &str = "xenonet connect's own entries are in Claude Code's settings beside the plugin's. XenoNet ignores the plugin's copy of any hook connect added, so each event still arrives once. Connecting takes connect's entries out.";

/// What the preview says when the plugin can't find the command it runs.
const NO_COMMAND: &str = "XenoNet's plugin runs the xenonet command, which isn't on your PATH";

/// What the preview says of entries an earlier XenoNet wrote.
const EARLIER: &str = "XenoNet's entries here run cli/xenonet.js with Node, as an earlier XenoNet wrote them. Connecting again has them run the xenonet binary.";

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

    fn options(port: f64) -> Options {
        Options {
            port: Arg::num(port),
            demo: Arg::Bool(false),
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            grok_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![
                Instance { id: "installed".into(), name: "XenoNet".into(), port: Arg::num(47840.0) },
                Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: Arg::num(47842.0) },
            ],
        }
    }

    fn at(port: f64) -> Connect {
        Connect::new(options(port))
    }

    /// Claude Code's settings format as connect makes it for that port.
    fn claude_at(port: f64) -> (Shared, ClaudeSettings) {
        let k = Shared::new(options(port));
        let f = ClaudeSettings::new(xenonet_base::apps::app("claude").unwrap(), &k);
        (k, f)
    }

    #[test]
    fn the_plugin_counts_when_installed_and_turned_on_and_only_for_the_installed_xenonet() {
        let installed = r#"{ "version": 2, "plugins": { "xenonet@xenonet": [{ "scope": "user" }] } }"#;
        // The paths as this machine joins them.
        let (settings_file, list_file) = {
            let (_, f) = claude_at(47840.0);
            (f.settings_file, f.plugins_file)
        };
        let files = |enabled: &str, list: Option<&str>| {
            let mut m = std::collections::HashMap::new();
            m.insert(settings_file.clone(), format!(r#"{{ "enabledPlugins": {enabled} }}"#));
            if let Some(l) = list {
                m.insert(list_file.clone(), l.to_string());
            }
            Files(m)
        };
        let plugin = |(k, c): (Shared, ClaudeSettings), mut f: Files| {
            let settings = k.read_json(&mut f, &c.settings_file).unwrap();
            c.plugin(&k, &mut f, &settings)
        };
        let on = r#"{ "xenonet@xenonet": true }"#;
        assert_eq!(plugin(claude_at(47840.0), files(on, Some(installed))), Some("xenonet@xenonet".into()));
        assert_eq!(plugin(claude_at(47842.0), files(on, Some(installed))), None, "XenoNet Dev's own connect");
        assert_eq!(plugin(claude_at(47840.0), files(r#"{ "xenonet@xenonet": false }"#, Some(installed))), None, "turned off");
        assert_eq!(plugin(claude_at(47840.0), files(on, None)), None, "not installed");
        assert_eq!(plugin(claude_at(47840.0), files(on, Some("{ not json"))), None, "a list that can't be read");
        assert_eq!(plugin(claude_at(47840.0), files(r#"{ "xenonet-tools@xenonet": true }"#, Some(installed))), None, "another plugin");
    }

    #[test]
    fn connects_own_hook_for_an_event_is_there_only_when_it_can_run() {
        let c = at(47840.0);
        let (_, cf) = claude_at(47840.0);
        let start = cf.hooks["SessionStart"].clone();
        let stop = cf.hooks["Stop"].clone();
        let settings = json!({ "hooks": { "SessionStart": [start], "Stop": [stop] } }).to_string();
        let mut m = std::collections::HashMap::new();
        m.insert(cf.settings_file.clone(), settings);
        let mut f = Files(m.clone());
        assert!(c.hook_connected(&mut f, "Stop").unwrap());
        assert!(!c.hook_connected(&mut f, "SessionStart").unwrap(), "the binary it runs is gone");
        assert!(!c.hook_connected(&mut f, "Notification").unwrap());
        assert!(!at(47842.0).hook_connected(&mut f, "Stop").unwrap(), "another XenoNet's");
        m.insert("/x/xenonet".to_string(), String::new());
        assert!(c.hook_connected(&mut Files(m.clone()), "SessionStart").unwrap());
        m.insert(cf.settings_file.clone(), "{ not json".into());
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
        let c = Connect::new(options(47842.0));
        let (k, cf) = claude_at(47842.0);
        let claude = |h: &Value| cf.is_xenonet(h);
        let codex = |h: &Value| Shared::xenonet_command("codex", h);
        assert_eq!(c.names(), &("xenonet-dev".to_string(), "xenonet-dev-console".to_string()));
        let start = &cf.hooks["SessionStart"]["hooks"][0];
        assert_eq!(start["command"], json!("XENONET_PORT=47842 '/x/xenonet' hook claude"));
        assert!(claude(start));
        assert_eq!(Shared::port_of(start), Some(47842.0));
        assert_eq!(k.mcp(), &json!({ "type": "stdio", "command": "/x/xenonet", "args": ["mcp"], "env": { "XENONET_PORT": "47842" } }));
        // An earlier XenoNet's hook, which ran Node and cli/xenonet.js, is
        // still a XenoNet's, and says so.
        let node = json!({ "type": "command", "command": "XENONET_PORT=47842 ELECTRON_RUN_AS_NODE=1 '/x/node' '/app/cli/xenonet.js' hook claude" });
        assert!(claude(&node));
        let hooks = json!({ "SessionStart": [{ "hooks": [node] }] });
        assert!(k.has_own(Some(&hooks), &claude) && k.earlier(Some(&hooks), &claude, None));
        assert!(!k.earlier(Some(&cf.hooks), &claude, Some(k.mcp())));
        assert!(k.earlier(None, &codex, Some(&json!({ "command": "/x/node", "args": ["/app/cli/xenonet.js", "mcp"] }))));
        // Another XenoNet's binary, and a Windows one's, are XenoNet's too.
        assert!(codex(&json!({ "type": "command", "command": "XENONET_PORT=47840 '/r/launcher/xenonet' hook codex" })));
        assert!(codex(&json!({ "type": "command", "command": r"XENONET_PORT=47840 'C:\r\launcher\xenonet.exe' hook codex" })));
        assert!(!codex(&json!({ "type": "command", "command": "my-xenonet-thing hook codex" })));
        let win = k.windows_hook("codex");
        let h = json!({ "type": "command", "command": "x", "commandWindows": win });
        assert!(Shared::hook_text(&h).starts_with("# XenoNet hook codex\n$env:XENONET_PORT='47842'"));
        assert!(codex(&h));
    }

    #[test]
    fn an_entry_is_one_apps_own_and_no_other_apps() {
        let (_, cf) = claude_at(47840.0);
        for (url, ours) in [
            ("http://127.0.0.1:47840/api/hook/claude", true),
            ("http://localhost:47842/api/hook/claude", true),
            ("http://127.0.0.1:47840/api/hook/codex", false),
            ("http://127.0.0.1:47840/api/hook/claude/listen", false),
            ("http://127.0.0.1:47840/api/hook/claudes", false),
            ("http://127.0.0.1:/api/hook/claude", false),
            ("https://127.0.0.1:47840/api/hook/claude", false),
            ("http://10.0.0.1:47840/api/hook/claude", false),
        ] {
            assert_eq!(cf.is_xenonet(&json!({ "type": "http", "url": url })), ours, "{url}");
        }
        for (command, app, ours) in [
            ("XENONET_PORT=47840 '/x/xenonet' hook claude", "claude", true),
            ("XENONET_PORT=47840 '/x/xenonet' hook claude", "codex", false),
            ("XENONET_PORT=47840 '/x/xenonet' hook codex", "codex", true),
            ("XENONET_PORT=47840 '/x/xenonet' hook xcodex", "codex", false),
            ("XENONET_PORT=47840 '/x/xenonet' hook  codex", "codex", false),
            ("node /app/cli/hook.js codex", "codex", true),
            ("node '/app/server/engine/hook.js' claude", "claude", true),
            ("xenonet hook codex", "codex", false),
        ] {
            assert_eq!(Shared::xenonet_command(app, &json!({ "type": "command", "command": command })), ours, "{command} for {app}");
        }
    }

    /// A machine's files and CLIs as connect sees them: files by path, the
    /// CLIs on PATH by name, and every command run, in order.
    #[derive(Default)]
    struct Machine {
        files: std::collections::BTreeMap<String, String>,
        clis: Vec<String>,
        asked: Vec<String>,
    }
    impl Calls for Machine {
        fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
            let text = |i: usize| args.get(i).map(js_text).unwrap_or_default();
            match (path, method) {
                ("fs", "readFileSync") => self.files.get(&text(0)).map(|t| Arg::text(t.clone())).ok_or_else(|| Thrown::coded("no such file", "ENOENT")),
                ("fs", "existsSync") => Ok(Arg::Bool(self.files.contains_key(&text(0)))),
                ("fs", "mkdirSync") => Ok(Arg::Undef),
                ("fs", "copyFileSync") => {
                    let from = self.files.get(&text(0)).cloned().unwrap_or_default();
                    self.files.insert(text(1), from);
                    Ok(Arg::Undef)
                }
                ("fs", "writeFileSync") => {
                    self.files.insert(text(0), text(1));
                    Ok(Arg::Undef)
                }
                ("fs", "renameSync") => {
                    let moved = self.files.remove(&text(0)).unwrap_or_default();
                    self.files.insert(text(1), moved);
                    Ok(Arg::Undef)
                }
                ("", "cli") => {
                    self.asked.push(format!("cli {}", text(0)));
                    Ok(if self.clis.contains(&text(0)) { Arg::text(format!("/bin/{}", text(0))) } else { Arg::Null })
                }
                ("", "run") => {
                    let argv: Vec<String> = match args.get(1) {
                        Some(Arg::List(l)) => l.iter().map(js_text).collect(),
                        _ => Vec::new(),
                    };
                    self.asked.push(format!("{} {}", text(0), argv.join(" ")));
                    Ok(Arg::text(""))
                }
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

    /// A made-up app's made-up config format: command hooks in
    /// ~/.wren/hooks.json, and its MCP server through `wren mcp`, built
    /// from what every format shares.
    struct WrenHooks {
        id: String,
        name: String,
        file: String,
        hooks: Value,
    }
    impl WrenHooks {
        fn make(app: &App, k: &Shared) -> Box<dyn Format> {
            let hooks = hook_set(&events_of(app), |_| json!({ "type": "command", "command": k.hook_command(&app.id) }));
            Box::new(WrenHooks { id: app.id.clone(), name: app.name.clone(), file: format!("{}/.wren/hooks.json", k.options().home), hooks })
        }
        fn is(&self) -> impl Fn(&Value) -> bool + '_ {
            |h| Shared::xenonet_command(&self.id, h)
        }
    }
    impl Format for WrenHooks {
        fn cli(&self) -> &'static str {
            "wren"
        }
        fn server(&self, _: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) -> Result<Option<Value>> {
            if cli.truthy() {
                Shared::run(c, cli, vec!["mcp".into(), "get".into(), name.into()])?;
            }
            Ok(None)
        }
        fn entry(&self, k: &Shared, c: &mut dyn Calls, found: Found<'_>) -> Result<Value> {
            let settings = k.read_json(c, &self.file)?;
            let state = k.hooks_state(c, &settings, &self.is(), &self.hooks)?;
            Ok(json!({ "app": self.id, "name": self.name, "cli": Shared::arg_json(found.cli), "changes": [{ "id": "hooks", "state": state }], "notes": [], "broken": [] }))
        }
        fn apply(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
            let settings = k.read_json(c, &self.file)?;
            let merged = k.merge(c, &settings, &self.is(), Some(&self.hooks))?;
            Shared::write_json(c, &self.file, &merged)
        }
        fn remove_hooks(&self, k: &Shared, c: &mut dyn Calls) -> Result<()> {
            let settings = k.read_json(c, &self.file)?;
            let merged = k.merge(c, &settings, &self.is(), None)?;
            Shared::write_json(c, &self.file, &merged)
        }
        fn remove_server(&self, _: &Shared, c: &mut dyn Calls, cli: &Arg, name: &str) {
            if cli.truthy() {
                let _ = Shared::run(c, cli, vec!["mcp".into(), "remove".into(), name.into()]);
            }
        }
    }

    #[test]
    fn a_made_up_app_in_a_registry_of_its_own_connects_through_its_own_format() {
        let wren = App {
            id: "wren".into(),
            name: "Wren".into(),
            connect: Some(xenonet_base::apps::Connect {
                format: "wren-hooks".into(),
                events: vec![("Start".into(), None), ("Edit".into(), Some("write".into()))],
                unheard: vec![],
            }),
            has: vec!["connect".into()],
            ..App::default()
        };
        // An app without connect, and one whose format no code reads, are
        // left out.
        let observed = App { id: "amp".into(), name: "Amp".into(), ..App::default() };
        let unknown = App {
            id: "kiro".into(),
            connect: Some(xenonet_base::apps::Connect { format: "kiro-toml".into(), events: vec![], unheard: vec![] }),
            has: vec!["connect".into()],
            ..App::default()
        };
        let formats = |name: &str| (name == "wren-hooks").then_some(WrenHooks::make as Maker);
        let c = Connect::with(options(47840.0), &[observed, wren, unknown], &formats);
        assert_eq!(c.ids(), ["wren"]);
        let mut m = Machine { clis: vec!["wren".into()], ..Machine::default() };

        let before = c.preview(&mut m).unwrap();
        assert_eq!(before["apps"], json!([{ "app": "wren", "name": "Wren", "cli": "/bin/wren", "changes": [{ "id": "hooks", "state": "missing" }], "notes": [], "broken": [] }]));
        assert_eq!(m.asked, ["cli wren", "/bin/wren mcp get xenonet"], "its command, then its server");

        let after = c.apply(&mut m, &[Arg::text("wren")]).unwrap();
        assert_eq!(after["apps"][0]["changes"][0]["state"], json!("present"));
        let written: Value = serde_json::from_str(&m.files["/h/.wren/hooks.json"]).unwrap();
        assert_eq!(
            written,
            json!({ "hooks": { "Start": [{ "hooks": [{ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook wren" }] }], "Edit": [{ "matcher": "write", "hooks": [{ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook wren" }] }] } })
        );
        assert_eq!(c.apply(&mut m, &[Arg::text("claude")]).unwrap(), after, "an app it doesn't name stays as it is");

        // Your own hook stays when XenoNet's go.
        let mut mixed = written.clone();
        mixed["hooks"]["Start"][0]["hooks"].as_array_mut().unwrap().push(json!({ "type": "command", "command": "my-own" }));
        m.files.insert("/h/.wren/hooks.json".into(), mixed.to_string());
        m.asked.clear();
        let removed = c.remove(&mut m).unwrap();
        assert_eq!(removed["apps"][0]["changes"][0]["state"], json!("missing"));
        assert_eq!(
            serde_json::from_str::<Value>(&m.files["/h/.wren/hooks.json"]).unwrap(),
            json!({ "hooks": { "Start": [{ "hooks": [{ "type": "command", "command": "my-own" }] }] } })
        );
        assert_eq!(m.asked[..3], ["cli wren", "/bin/wren mcp remove xenonet", "cli wren"], "its hooks, then its server, then the preview");
        assert!(!c.hook_connected(&mut m, "Start").unwrap(), "no format here says");
    }

    #[test]
    fn the_registry_connects_claude_code_codex_and_grok_build_through_their_formats() {
        assert_eq!(at(47840.0).ids(), ["claude", "codex", "grok"]);
        let demo = Connect::new(Options { demo: Arg::Bool(true), ..options(47840.0) });
        assert_eq!(demo.apply(&mut NoCalls, &[]).unwrap_err().message, "this is the demo, so connect Claude Code, Codex and Grok Build from XenoNet, with your own sessions");
        assert_eq!((and_list(&["A"]), and_list(&["A", "B"]), and_list(&[])), ("A".to_string(), "A and B".to_string(), String::new()));
    }
}
