//! Connecting your agents: what XenoNet adds to the settings of each app
//! whose adapter has a connect (Claude Code, Codex, Grok Build), so every
//! session on this machine, in every project, can use the engine. Nothing
//! changes until you confirm in XenoNet, which first shows exactly what
//! will be added (preview), and it can all be taken out again (remove).
//!
//! Each adapter brings the format of its app's settings (`Format`): where
//! its MCP server and its hooks go, and how. What every format shares is
//! here (`Shared`): this XenoNet's options, the MCP server it adds, how its
//! entries are told apart from yours and from another XenoNet's, and the
//! settings files read and written. `Connect` runs the formats in the
//! registry's order. The machine's files and commands come through
//! `Machine`, which the command and the service each implement.
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

use crate::apps::App;
use serde_json::{json, Map, Value};
use xenonet_base::json::{number, number_string, stringify};
use xenonet_base::jsstr::{base64, base64_bytes, cmp_units, from_utf16le, js_key_order, utf16le};
use xenonet_base::nodepath::{Style, NATIVE};
use xenonet_base::paths::tilde;
use xenonet_base::text::{js_number, js_string, truthy};

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new($p).expect("a pattern that compiles"))
    }};
}

/// What went wrong on the machine: its message, and the system's code
/// when it has one ("ENOENT", "EACCES").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    pub code: Option<String>,
}

impl Problem {
    pub fn new(message: impl Into<String>) -> Self {
        Problem { message: message.into(), code: None }
    }

    pub fn coded(message: impl Into<String>, code: &str) -> Self {
        Problem { message: message.into(), code: Some(code.into()) }
    }

    pub fn is(&self, code: &str) -> bool {
        self.code.as_deref() == Some(code)
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Problem {}

pub type Result<T> = std::result::Result<T, Problem>;

/// A connect's host: the machine's files and commands, as the command and
/// the service have them.
pub trait Machine {
    /// A file's text; a missing one is a `Problem` coded "ENOENT".
    fn read_text(&mut self, file: &str) -> Result<String>;
    fn write_text(&mut self, file: &str, text: &str) -> Result<()>;
    fn exists(&mut self, path: &str) -> bool;
    /// A folder and the folders above it.
    fn make_dir(&mut self, dir: &str) -> Result<()>;
    fn copy(&mut self, from: &str, to: &str) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
    /// A file taken out; one that's already gone is fine.
    fn remove(&mut self, file: &str) -> Result<()>;
    /// A CLI on PATH, as the path it's found at.
    fn cli(&mut self, name: &str) -> Option<String>;
    /// A command's output, or what it said on stderr as the problem's
    /// message.
    fn run(&mut self, bin: &str, args: &[String]) -> Result<String>;
}

/// One of the XenoNets that run on a machine.
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: String,
    pub name: String,
    pub port: f64,
}

/// How a connect was made: this XenoNet's port and binary, the machine's
/// folders, and the XenoNets that run on it.
pub struct Options {
    pub port: f64,
    pub demo: bool,
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

/// `a === b` for JSON values: primitives by value, objects never.
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(_) | Value::Array(_), _) | (_, Value::Object(_) | Value::Array(_)) => false,
        (a, b) => a == b,
    }
}

/// A value with every object's keys in the order a settings file lists
/// them: array indexes first, then the rest as they came.
pub fn normalized(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(js_key_order(m.iter().map(|(k, v)| (k.clone(), normalized(v))).collect())),
        Value::Array(a) => Value::Array(a.iter().map(normalized).collect()),
        other => other.clone(),
    }
}

/// A value as one line of JSON.
pub fn json_text(v: &Value) -> String {
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

/// A list of values with arrays opened one level.
fn flat(items: Vec<Value>) -> Vec<Value> {
    items.into_iter().flat_map(|v| if let Value::Array(a) = v { a } else { vec![v] }).collect()
}

/// A group's hooks: an array's items, or the value when it's one.
pub fn hooks_of(g: &Value) -> Vec<Value> {
    match g.get("hooks") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.clone(),
        Some(other) => vec![other.clone()],
    }
}

/// A field as text: "" for one that's missing or null.
pub fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => js_string(other),
    }
}

/// A list of values as text, each.
pub fn list_strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|x| text_of(Some(x))).collect()).unwrap_or_default()
}

/// `'…'`, for a POSIX shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `'…'`, for PowerShell.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A path under a folder, on this machine.
pub fn join(dir: &str, name: &str) -> String {
    xenonet_base::nodepath::join(&[dir, name])
}

/// A port as text, as the entries carry it.
fn port_text(port: f64) -> String {
    number_string(port)
}

/// The names a XenoNet's MCP servers take: the installed XenoNet's xenonet
/// and xenonet-console, another's with its instance (xenonet-dev).
pub fn server_names(port: f64, instances: &[Instance]) -> (String, String) {
    let id = instances.iter().find(|i| i.port == port).map_or_else(|| port_text(port), |i| i.id.clone());
    let installed = instances.iter().find(|i| i.id == "installed").map(|i| i.port);
    let base = if installed == Some(port) { "xenonet".to_string() } else { format!("xenonet-{id}") };
    let console = format!("{base}-console");
    (base, console)
}

/// A settings object's "hooks" with XenoNet's taken out (`ours` tells them
/// apart), and with `add` put back in when given.
pub fn merge_hooks(c: &mut dyn Machine, settings: &Value, ours: &mut dyn FnMut(&mut dyn Machine, &Value) -> bool, add: Option<&Value>) -> Result<Value> {
    let mut hooks = spread(settings.get("hooks").filter(|h| !h.is_null()));
    let events: Vec<(String, Value)> = hooks.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    for (event, groups) in events {
        let groups = if let Value::Array(a) = groups { a } else { Vec::new() };
        let mut kept = Vec::new();
        for g in groups {
            if g.is_null() {
                return Err(Problem::new("Cannot read properties of null (reading 'hooks')"));
            }
            let mut g2 = spread(Some(&g));
            let mut left = Vec::new();
            for h in match g.get("hooks") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(Problem::new("(g.hooks ?? []).filter is not a function")),
            } {
                if !ours(c, &h) {
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

/// What every format shares: this XenoNet's options, the MCP server it
/// adds, and how its entries are told apart.
pub struct Shared {
    o: Options,
    server_env: Vec<(String, String)>,
    mcp: Value,
    names: (String, String),
}

impl Shared {
    pub fn new(o: Options) -> Self {
        let server_env = vec![("XENONET_PORT".to_string(), port_text(o.port))];
        let env_obj: Map<String, Value> = server_env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let mcp = json!({ "type": "stdio", "command": o.bin, "args": ["mcp"], "env": Value::Object(env_obj) });
        let names = server_names(o.port, &o.instances);
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

    /// The variables the MCP server and the hooks run with.
    pub fn server_env(&self) -> &[(String, String)] {
        &self.server_env
    }

    /// This XenoNet's port, as text.
    pub fn port(&self) -> String {
        port_text(self.o.port)
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

    /// A command entry's command as text: the Windows one's decoded.
    pub fn hook_text(h: &Value) -> String {
        let command = match h.get("commandWindows").filter(|v| !v.is_null()) {
            Some(v) => text_of(Some(v)),
            None => text_of(h.get("command")),
        };
        match re!(r"-EncodedCommand ([A-Za-z0-9+/=]+)$").captures(&command) {
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
        let form = re!(r#"(cli[\\/]xenonet\.js['"]? hook|(cli|server[\\/]engine)[\\/]hook\.js['"]?|[\\/]xenonet(\.exe)?['"]? hook)$"#);
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
            re!(r":([0-9]+)/api/hook/").captures(&text_of(h.get("url"))).map(|m| m[1].to_string())
        } else {
            re!(r#"XENONET_PORT=['"]?([0-9]+)"#).captures(&Self::hook_text(h)).map(|m| m[1].to_string())
        };
        m.map(|d| js_number(Some(&Value::String(d))))
    }

    /// Whether an entry is for this XenoNet's port, or names none.
    pub fn this_port(&self, h: &Value) -> bool {
        Self::port_of(h).is_none_or(|p| self.o.port == p)
    }

    /// Only the paths among a command's quoted words.
    pub fn hook_files(hooks: Option<&Value>, ours: &mut dyn FnMut(&Value) -> bool) -> Vec<String> {
        let quoted = re!(r"'((?:[^']|'')+)'");
        let mut out = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if !ours(&h) || h.get("type").and_then(Value::as_str) != Some("command") {
                    continue;
                }
                for m in quoted.captures_iter(&Self::hook_text(&h)) {
                    let f = m[1].replace("''", "'");
                    if NATIVE.is_absolute(&f) || Style::Win32.is_absolute(&f) {
                        out.push(f);
                    }
                }
            }
        }
        out
    }

    /// An entry is this XenoNet's when it's for this port, or says no
    /// port, or runs files that are gone; another live XenoNet's stays as
    /// it is. `is` tells a format's XenoNet entries from the rest.
    pub fn mine(&self, c: &mut dyn Machine, is: &dyn Fn(&Value) -> bool, h: &Value) -> bool {
        if !is(h) {
            return false;
        }
        if self.this_port(h) {
            return true;
        }
        let one = json!({ "x": [{ "hooks": [h] }] });
        Self::hook_files(Some(&one), &mut |h| is(h)).iter().any(|f| !c.exists(f))
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
        own || server.and_then(|s| s.get("args")).and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| re!(r"\.m?js$").is_match(&text_of(Some(x)))))
    }

    /// Other XenoNets connected here too, by port, for the preview.
    pub fn others(&self, c: &mut dyn Machine, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool) -> Vec<Option<f64>> {
        let mut out: Vec<Option<f64>> = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if is(&h) && !self.mine(c, is, &h) {
                    let p = Self::port_of(&h);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        // In order as text, so the preview reads the same each time.
        let text = |p: &Option<f64>| p.map_or("null".to_string(), port_text);
        out.sort_by(|a, b| cmp_units(&text(a), &text(b)));
        out
    }

    // ---------- files ----------

    /// A file's text, or None when there's none. One that can't be read is
    /// left alone, and the error names it with the home folder as ~ and
    /// says where it goes wrong without quoting it.
    fn read(&self, c: &mut dyn Machine, file: &str) -> Result<Option<String>> {
        let home = &self.o.home;
        match c.read_text(file) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.is("ENOENT") => Ok(None),
            Err(e) => {
                let why = e.code.clone().unwrap_or_else(|| tilde(&e.message, home));
                Err(Problem::new(format!("XenoNet can't read {} ({why}), so it leaves it alone", tilde(file, home))))
            }
        }
    }

    /// A text file, or "" when there's none.
    pub fn read_text(&self, c: &mut dyn Machine, file: &str) -> Result<String> {
        Ok(self.read(c, file)?.unwrap_or_default())
    }

    /// A settings file, or {} when there's none. One that isn't JSON is
    /// left alone too.
    pub fn read_json(&self, c: &mut dyn Machine, file: &str) -> Result<Value> {
        let Some(text) = self.read(c, file)? else { return Ok(json!({})) };
        match xenonet_base::json::parse(&text) {
            Ok(v) => Ok(normalized(&v)),
            Err(_) => {
                let message = match xenonet_schema::vendor::json_error_at(&text) {
                    Some((line, column)) => format!("isn't valid JSON (line {line}, column {column})"),
                    None => "isn't valid JSON".into(),
                };
                Err(Problem::new(format!("{} {message}, so XenoNet leaves it alone", tilde(file, &self.o.home))))
            }
        }
    }

    /// A settings file written whole: through a temporary file, with a
    /// copy of it as it was the first time XenoNet changes it.
    pub fn write_json(c: &mut dyn Machine, file: &str, data: &Value) -> Result<()> {
        c.make_dir(&NATIVE.dirname(file))?;
        let before = format!("{file}.before-xenonet");
        if c.exists(file) && !c.exists(&before) {
            c.copy(file, &before)?;
        }
        let tmp = format!("{file}.tmp-xenonet");
        c.write_text(&tmp, &format!("{}\n", stringify(&normalized(data), 2)))?;
        c.rename(&tmp, file)?;
        Ok(())
    }

    /// A settings object with this XenoNet's entries (by `is`) taken out,
    /// and `add` put in when given.
    pub fn merge(&self, c: &mut dyn Machine, settings: &Value, is: &dyn Fn(&Value) -> bool, add: Option<&Value>) -> Result<Value> {
        merge_hooks(c, settings, &mut |c, h| self.mine(c, is, h), add)
    }

    // ---------- what's there ----------

    /// The xenonet server as each app has it: missing, present (as XenoNet
    /// would add it) or different.
    pub fn server_state(&self, s: Option<&Value>, args: Option<&Value>) -> &'static str {
        let Some(s) = s.filter(|s| truthy(Some(s))) else {
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
    fn hooks_in_place(&self, c: &mut dyn Machine, settings: &Value, is: &dyn Fn(&Value) -> bool, add: &Value) -> Result<bool> {
        for (event, group) in spread(Some(add)) {
            let groups = match settings.get("hooks").and_then(|h| h.get(&event)) {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(Problem::new("(settings.hooks?.[event] ?? []).some is not a function")),
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
                if hs.iter().any(|h| self.mine(c, is, h) && json_text(h) == first) {
                    found = true;
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
    pub fn hooks_state(&self, c: &mut dyn Machine, settings: &Value, is: &dyn Fn(&Value) -> bool, add: &Value) -> Result<&'static str> {
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
    fn gone(c: &mut dyn Machine, entry: &Value) -> Vec<String> {
        let mut files = vec![entry.get("command").cloned().unwrap_or(Value::Null)];
        if let Some(Value::Array(args)) = entry.get("args") {
            files.extend(args.iter().filter(|a| re!(r"\.m?js$").is_match(&text_of(Some(a)))).cloned());
        }
        let mut out = Vec::new();
        for f in files {
            if !truthy(Some(&f)) {
                continue;
            }
            let f = text_of(Some(&f));
            if NATIVE.is_absolute(&f) && !c.exists(&f) {
                out.push(f);
            }
        }
        out
    }

    /// What an app's entries run that's gone: its server's files, and its
    /// hooks' (this XenoNet's own, by `is`).
    pub fn broken(&self, c: &mut dyn Machine, entry: Option<&Value>, hooks: Option<&Value>, is: &dyn Fn(&Value) -> bool) -> Vec<Value> {
        let mut files = match entry.filter(|e| truthy(Some(e))) {
            Some(e) => Self::gone(c, e),
            None => Vec::new(),
        };
        // This XenoNet's own entries, which may look at files.
        let mut mine_list = Vec::new();
        for g in flat(values(hooks)) {
            for h in hooks_of(&g) {
                if self.mine(c, is, &h) {
                    mine_list.push(h);
                }
            }
        }
        let wrapped = json!({ "x": [{ "hooks": mine_list }] });
        for f in Self::hook_files(Some(&wrapped), &mut |_| true) {
            if !c.exists(&f) {
                files.push(f);
            }
        }
        files.into_iter().map(|f| Value::String(format!("{} is gone", tilde(&f, &self.o.home)))).collect()
    }

    /// Another XenoNet on this machine connected too: its entries stay as
    /// they are.
    pub fn besides(&self, ports: &[Option<f64>]) -> Vec<Value> {
        ports
            .iter()
            .map(|p| {
                let shown = p.map_or("null".to_string(), port_text);
                let name = self.o.instances.iter().find(|i| *p == Some(i.port)).map(|i| format!(", {}", i.name)).unwrap_or_default();
                Value::String(format!("Another XenoNet (port {shown}{name}) is connected here too; its entries stay as they are."))
            })
            .collect()
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

/// An app's command for the preview: its path, or null.
pub fn cli_json(cli: Option<&str>) -> Value {
    cli.map_or(Value::Null, |c| Value::String(c.into()))
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

fn group(matcher: Option<&str>, hook: Value) -> Value {
    let mut g = Map::new();
    if let Some(m) = matcher {
        g.insert("matcher".into(), json!(m));
    }
    g.insert("hooks".into(), Value::Array(vec![hook]));
    Value::Object(g)
}

/// An app's hook events as the preview lists them.
pub fn event_list(events: &[(String, Option<String>)]) -> String {
    events.iter().map(|(e, m)| if m.is_some() { format!("{e} (edits)") } else { e.clone() }).collect::<Vec<_>>().join(", ")
}

/// An app's hook events, from its registry entry.
pub fn events_of(app: &App) -> Vec<(String, Option<String>)> {
    app.connect.as_ref().map(|c| c.events.clone()).unwrap_or_default()
}

/// What the preview says of entries an earlier XenoNet wrote.
pub const EARLIER: &str = "XenoNet's entries here run cli/xenonet.js with Node, as an earlier XenoNet wrote them. Connecting again has them run the xenonet binary.";

/// What the preview found of an app before its part: its command, its
/// xenonet server and its console's.
pub struct Found<'a> {
    pub cli: Option<&'a str>,
    pub server: Option<&'a Value>,
    pub console: Option<&'a Value>,
}

/// One config format's connect, made for one app: what XenoNet adds there,
/// and takes out.
pub trait Format: Send + Sync {
    /// The command its MCP server goes in through.
    fn cli(&self) -> &'static str;
    /// The MCP server of that name as the app has it.
    fn server(&self, k: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) -> Result<Option<Value>>;
    /// The app's part of the preview.
    fn entry(&self, k: &Shared, c: &mut dyn Machine, found: Found<'_>) -> Result<Value>;
    /// Puts XenoNet's hooks and servers in.
    fn apply(&self, k: &Shared, c: &mut dyn Machine) -> Result<()>;
    /// Takes XenoNet's hooks out.
    fn remove_hooks(&self, k: &Shared, c: &mut dyn Machine) -> Result<()>;
    /// Takes the MCP server of that name out.
    fn remove_server(&self, k: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str);
    /// Whether connect's own hook for `event` is in place, for a format
    /// XenoNet's plugin carries a copy of (Claude Code's settings); None
    /// for the others.
    fn hook_connected(&self, _k: &Shared, _c: &mut dyn Machine, _event: &str) -> Option<Result<bool>> {
        None
    }
    /// For an app that may not be here (Grok Build): whether it is, so
    /// connecting every app leaves it out until it is.
    fn installed(&self, _k: &Shared, _c: &mut dyn Machine) -> Result<bool> {
        Ok(true)
    }
}

/// A format's connect for an app.
pub type Maker = fn(&App, &Shared) -> Box<dyn Format>;

/// An app's format, or None for an app that connects through none.
pub type Formats<'a> = dyn Fn(&App, &Shared) -> Option<Box<dyn Format>> + 'a;

/// Names as a sentence lists them: "A, B and C".
fn and_list(names: &[&str]) -> String {
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

/// Connect: the apps of the registry that have `connect`, in its order,
/// each through the format its adapter brings.
pub struct Connect {
    k: Shared,
    apps: Vec<(App, Box<dyn Format>)>,
}

impl Connect {
    /// Connect through the adapters in place: an app whose adapter isn't
    /// there, or brings no format, is left out.
    pub fn new(o: Options) -> Self {
        Self::with(o, crate::apps::apps(), &|app, k| crate::platforms().get(&app.id)?.connect(app, k))
    }

    /// Connect over a registry and formats of a test's own.
    pub fn with(o: Options, registry: &[App], formats: &Formats<'_>) -> Self {
        let k = Shared::new(o);
        let apps = registry.iter().filter(|a| a.has("connect") && a.connect.is_some()).filter_map(|a| Some((a.clone(), formats(a, &k)?))).collect();
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
    pub fn installed_ids(&self, c: &mut dyn Machine) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for (app, f) in &self.apps {
            if f.installed(&self.k, c)? {
                out.push(app.id.clone());
            }
        }
        Ok(out)
    }

    fn clis(&self, c: &mut dyn Machine) -> Vec<Option<String>> {
        self.apps.iter().map(|(_, f)| c.cli(f.cli())).collect()
    }

    /// Each app's command first, then its xenonet server, then its
    /// console's, then its part of the preview, in the registry's order.
    pub fn preview(&self, c: &mut dyn Machine) -> Result<Value> {
        let clis = self.clis(c);
        let (xenonet, console) = self.k.names.clone();
        let mut servers = Vec::new();
        for ((_, f), cli) in self.apps.iter().zip(&clis) {
            servers.push(f.server(&self.k, c, cli.as_deref(), &xenonet)?);
        }
        let mut consoles = Vec::new();
        if self.k.o.console_mcp {
            for ((_, f), cli) in self.apps.iter().zip(&clis) {
                consoles.push(f.server(&self.k, c, cli.as_deref(), &console)?);
            }
        }
        let mut apps = Vec::new();
        for (i, ((_, f), cli)) in self.apps.iter().zip(&clis).enumerate() {
            apps.push(f.entry(&self.k, c, Found { cli: cli.as_deref(), server: servers[i].as_ref(), console: consoles.get(i).and_then(Option::as_ref) })?);
        }
        Ok(json!({ "port": number(self.k.o.port), "demo": self.k.o.demo, "apps": apps }))
    }

    pub fn apply(&self, c: &mut dyn Machine, apps: &[String]) -> Result<Value> {
        // Agents connected to the demo would see only its made-up sessions.
        if self.k.o.demo {
            let names: Vec<&str> = self.apps.iter().map(|(a, _)| a.name.as_str()).collect();
            return Err(Problem::new(format!("this is the demo, so connect {} from XenoNet, with your own sessions", and_list(&names))));
        }
        for (app, f) in &self.apps {
            if apps.contains(&app.id) {
                f.apply(&self.k, c)?;
            }
        }
        self.preview(c)
    }

    /// Each app's hooks first, then its server, then its console's.
    pub fn remove(&self, c: &mut dyn Machine) -> Result<Value> {
        for (_, f) in &self.apps {
            f.remove_hooks(&self.k, c)?;
        }
        let clis = self.clis(c);
        let (xenonet, console) = self.k.names.clone();
        for ((_, f), cli) in self.apps.iter().zip(&clis) {
            f.remove_server(&self.k, c, cli.as_deref(), &xenonet);
        }
        if self.k.o.console_mcp {
            for ((_, f), cli) in self.apps.iter().zip(&clis) {
                f.remove_server(&self.k, c, cli.as_deref(), &console);
            }
        }
        self.preview(c)
    }

    /// Whether connect's own hook for `event` is in place where XenoNet's
    /// plugin carries a copy of it (Claude Code's settings): false where no
    /// format says.
    pub fn hook_connected(&self, c: &mut dyn Machine, event: &str) -> Result<bool> {
        for (_, f) in &self.apps {
            if let Some(r) = f.hook_connected(&self.k, c, event) {
                return r;
            }
        }
        Ok(false)
    }
}

/// A machine for tests: files by path, the CLIs on PATH by name, and every
/// command asked, in order. A missing file reads as ENOENT.
#[cfg(any(test, feature = "testing"))]
#[derive(Default)]
pub struct FakeMachine {
    pub files: std::collections::BTreeMap<String, String>,
    pub clis: Vec<String>,
    pub asked: Vec<String>,
}

/// A path as the fake keeps it: with / for \\, so a test's keys are the
/// same on every system, though the code joins paths as the machine does.
#[cfg(any(test, feature = "testing"))]
fn key(path: &str) -> String {
    path.replace('\\', "/")
}

#[cfg(any(test, feature = "testing"))]
impl Machine for FakeMachine {
    fn read_text(&mut self, file: &str) -> Result<String> {
        self.files.get(&key(file)).cloned().ok_or_else(|| Problem::coded("no such file", "ENOENT"))
    }
    fn write_text(&mut self, file: &str, text: &str) -> Result<()> {
        self.files.insert(key(file), text.into());
        Ok(())
    }
    fn exists(&mut self, path: &str) -> bool {
        self.files.contains_key(&key(path))
    }
    fn make_dir(&mut self, _dir: &str) -> Result<()> {
        Ok(())
    }
    fn copy(&mut self, from: &str, to: &str) -> Result<()> {
        let text = self.files.get(&key(from)).cloned().unwrap_or_default();
        self.files.insert(key(to), text);
        Ok(())
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        let text = self.files.remove(&key(from)).unwrap_or_default();
        self.files.insert(key(to), text);
        Ok(())
    }
    fn remove(&mut self, file: &str) -> Result<()> {
        self.files.remove(&key(file));
        Ok(())
    }
    fn cli(&mut self, name: &str) -> Option<String> {
        self.asked.push(format!("cli {name}"));
        self.clis.iter().any(|c| c == name).then(|| format!("/bin/{name}"))
    }
    fn run(&mut self, bin: &str, args: &[String]) -> Result<String> {
        self.asked.push(format!("{bin} {}", args.join(" ")));
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn options(port: f64) -> Options {
        Options {
            port,
            demo: false,
            bin: "/x/xenonet".into(),
            home: "/h".into(),
            claude_config_dir: None,
            codex_home: None,
            grok_home: None,
            console_mcp: false,
            platform: "darwin".into(),
            instances: vec![Instance { id: "installed".into(), name: "XenoNet".into(), port: 47840.0 }, Instance { id: "dev".into(), name: "XenoNet Dev".into(), port: 47842.0 }],
        }
    }

    #[test]
    fn the_fake_machine_takes_windows_paths_as_the_tests_write_them() {
        // The code joins paths with \\ on Windows; the tests key files with /.
        let mut m = FakeMachine::default();
        m.files.insert("/h/.grok/config.toml".into(), "x".into());
        assert!(m.exists("\\h\\.grok\\config.toml"));
        assert_eq!(m.read_text("\\h\\.grok\\config.toml").unwrap(), "x");
        m.write_text("\\h\\.codex\\hooks.json", "{}").unwrap();
        assert_eq!(m.files["/h/.codex/hooks.json"], "{}");
        m.rename("\\h\\.codex\\hooks.json", "/h/.codex/hooks.old").unwrap();
        assert!(m.exists("/h/.codex/hooks.old") && !m.exists("\\h\\.codex\\hooks.json"));
    }

    #[test]
    fn merging_takes_ours_out_and_puts_new_ones_in() {
        let mut m = FakeMachine::default();
        let settings = json!({ "model": "x", "hooks": { "Stop": [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:47840/api/hook/claude" }, { "type": "command", "command": "mine" }] }], "Other": [{ "hooks": [{ "type": "http", "url": "http://127.0.0.1:1/api/hook/claude" }] }] } });
        let out =
            merge_hooks(&mut m, &settings, &mut |_, h| h.get("type").and_then(Value::as_str) == Some("http"), Some(&json!({ "Stop": { "hooks": [{ "type": "http" }] } }))).unwrap();
        assert_eq!(json_text(&out), r#"{"model":"x","hooks":{"Stop":[{"hooks":[{"type":"command","command":"mine"}]},{"hooks":[{"type":"http"}]}]}}"#);
        assert_eq!(merge_hooks(&mut m, &json!({ "hooks": {} }), &mut |_, _| true, None).unwrap(), json!({}));
    }

    #[test]
    fn hooks_say_which_xenonet_theyre_for() {
        let k = Shared::new(options(47842.0));
        let codex = |h: &Value| Shared::xenonet_command("codex", h);
        assert_eq!(k.names(), &("xenonet-dev".to_string(), "xenonet-dev-console".to_string()));
        assert_eq!(server_names(47840.0, &options(1.0).instances), ("xenonet".to_string(), "xenonet-console".to_string()));
        assert_eq!(server_names(5.0, &options(1.0).instances).0, "xenonet-5");
        let start = json!({ "type": "command", "command": k.hook_command("codex") });
        assert_eq!(start["command"], json!("XENONET_PORT=47842 '/x/xenonet' hook codex"));
        assert!(codex(&start));
        assert_eq!(Shared::port_of(&start), Some(47842.0));
        assert_eq!(k.mcp(), &json!({ "type": "stdio", "command": "/x/xenonet", "args": ["mcp"], "env": { "XENONET_PORT": "47842" } }));
        // An earlier XenoNet's hook, which ran Node and cli/xenonet.js, is
        // still a XenoNet's, and says so.
        let node = json!({ "type": "command", "command": "XENONET_PORT=47842 ELECTRON_RUN_AS_NODE=1 '/x/node' '/app/cli/xenonet.js' hook codex" });
        assert!(codex(&node));
        let hooks = json!({ "SessionStart": [{ "hooks": [node] }] });
        assert!(k.has_own(Some(&hooks), &codex) && k.earlier(Some(&hooks), &codex, None));
        assert!(!k.earlier(Some(&json!({ "SessionStart": [{ "hooks": [start] }] })), &codex, Some(k.mcp())));
        assert!(k.earlier(None, &codex, Some(&json!({ "command": "/x/node", "args": ["/app/cli/xenonet.js", "mcp"] }))));
        // Another XenoNet's binary, and a Windows one's, are XenoNet's too.
        assert!(codex(&json!({ "type": "command", "command": "XENONET_PORT=47840 '/r/launcher/xenonet' hook codex" })));
        assert!(codex(&json!({ "type": "command", "command": r"XENONET_PORT=47840 'C:\r\launcher\xenonet.exe' hook codex" })));
        assert!(!codex(&json!({ "type": "command", "command": "my-xenonet-thing hook codex" })));
        let win = k.windows_hook("codex");
        let h = json!({ "type": "command", "command": "x", "commandWindows": win });
        assert!(Shared::hook_text(&h).starts_with("# XenoNet hook codex\n$env:XENONET_PORT='47842'"));
        assert!(codex(&h));
        assert_eq!(Shared::port_of(&h), Some(47842.0));
        // Another XenoNet's entries, by port, with its name when it has
        // one; its binary is there, or they'd be anyone's to clear.
        let mut m = FakeMachine::default();
        m.files.insert("/r/xenonet".into(), String::new());
        let other = json!({ "SessionStart": [{ "hooks": [{ "type": "command", "command": "XENONET_PORT=47840 '/r/xenonet' hook codex" }, { "type": "command", "command": "XENONET_PORT=9 '/r/xenonet' hook codex" }] }] });
        let others = k.others(&mut m, Some(&other), &codex);
        assert_eq!(others, [Some(47840.0), Some(9.0)]);
        assert_eq!(k.besides(&others)[0], json!("Another XenoNet (port 47840, XenoNet) is connected here too; its entries stay as they are."));
        assert_eq!(k.besides(&others)[1], json!("Another XenoNet (port 9) is connected here too; its entries stay as they are."));
    }

    #[test]
    fn a_command_entry_is_one_apps_own_and_no_other_apps() {
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

    #[test]
    fn a_settings_file_that_cant_be_read_is_left_alone_and_named() {
        struct Locked;
        impl Machine for Locked {
            fn read_text(&mut self, _: &str) -> Result<String> {
                Err(Problem::coded("EACCES: permission denied", "EACCES"))
            }
            fn write_text(&mut self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn exists(&mut self, _: &str) -> bool {
                false
            }
            fn make_dir(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
            fn copy(&mut self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn rename(&mut self, _: &str, _: &str) -> Result<()> {
                Ok(())
            }
            fn remove(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
            fn cli(&mut self, _: &str) -> Option<String> {
                None
            }
            fn run(&mut self, _: &str, _: &[String]) -> Result<String> {
                Ok(String::new())
            }
        }
        let k = Shared::new(options(47840.0));
        assert_eq!(k.read_json(&mut Locked, "/h/.claude/settings.json").unwrap_err().message, "XenoNet can't read ~/.claude/settings.json (EACCES), so it leaves it alone");
        let mut m = FakeMachine::default();
        assert_eq!(k.read_json(&mut m, "/h/none.json").unwrap(), json!({}));
        m.files.insert("/h/bad.json".into(), "{ not json".into());
        assert_eq!(k.read_json(&mut m, "/h/bad.json").unwrap_err().message, "~/bad.json isn't valid JSON (line 1, column 3), so XenoNet leaves it alone");
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
        fn server(&self, _: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) -> Result<Option<Value>> {
            if let Some(cli) = cli {
                c.run(cli, &["mcp".into(), "get".into(), name.into()])?;
            }
            Ok(None)
        }
        fn entry(&self, k: &Shared, c: &mut dyn Machine, found: Found<'_>) -> Result<Value> {
            let settings = k.read_json(c, &self.file)?;
            let state = k.hooks_state(c, &settings, &self.is(), &self.hooks)?;
            Ok(json!({ "app": self.id, "name": self.name, "cli": cli_json(found.cli), "changes": [{ "id": "hooks", "state": state }], "notes": [], "broken": [] }))
        }
        fn apply(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
            let settings = k.read_json(c, &self.file)?;
            let merged = k.merge(c, &settings, &self.is(), Some(&self.hooks))?;
            Shared::write_json(c, &self.file, &merged)
        }
        fn remove_hooks(&self, k: &Shared, c: &mut dyn Machine) -> Result<()> {
            let settings = k.read_json(c, &self.file)?;
            let merged = k.merge(c, &settings, &self.is(), None)?;
            Shared::write_json(c, &self.file, &merged)
        }
        fn remove_server(&self, _: &Shared, c: &mut dyn Machine, cli: Option<&str>, name: &str) {
            if let Some(cli) = cli {
                let _ = c.run(cli, &["mcp".into(), "remove".into(), name.into()]);
            }
        }
    }

    #[test]
    fn a_made_up_app_in_a_registry_of_its_own_connects_through_its_own_format() {
        let wren = App {
            id: "wren".into(),
            name: "Wren".into(),
            connect: Some(crate::apps::Connect { format: "wren-hooks".into(), events: vec![("Start".into(), None), ("Edit".into(), Some("write".into()))], unheard: vec![] }),
            has: vec!["connect".into()],
            ..App::default()
        };
        // An app without connect, and one whose format no code reads, are
        // left out.
        let observed = App { id: "amp".into(), name: "Amp".into(), ..App::default() };
        let unknown = App {
            id: "kiro".into(),
            connect: Some(crate::apps::Connect { format: "kiro-toml".into(), events: vec![], unheard: vec![] }),
            has: vec!["connect".into()],
            ..App::default()
        };
        let formats = |app: &App, k: &Shared| (app.id == "wren").then(|| WrenHooks::make(app, k));
        let c = Connect::with(options(47840.0), &[observed, wren, unknown], &formats);
        assert_eq!(c.ids(), ["wren"]);
        let mut m = FakeMachine { clis: vec!["wren".into()], ..FakeMachine::default() };

        let before = c.preview(&mut m).unwrap();
        assert_eq!(before["port"], json!(47840));
        assert_eq!(before["demo"], json!(false));
        assert_eq!(before["apps"], json!([{ "app": "wren", "name": "Wren", "cli": "/bin/wren", "changes": [{ "id": "hooks", "state": "missing" }], "notes": [], "broken": [] }]));
        assert_eq!(m.asked, ["cli wren", "/bin/wren mcp get xenonet"], "its command, then its server");

        let after = c.apply(&mut m, &["wren".to_string()]).unwrap();
        assert_eq!(after["apps"][0]["changes"][0]["state"], json!("present"));
        let written: Value = serde_json::from_str(&m.files["/h/.wren/hooks.json"]).unwrap();
        assert_eq!(
            written,
            json!({ "hooks": { "Start": [{ "hooks": [{ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook wren" }] }], "Edit": [{ "matcher": "write", "hooks": [{ "type": "command", "command": "XENONET_PORT=47840 '/x/xenonet' hook wren" }] }] } })
        );
        assert_eq!(c.apply(&mut m, &["claude".to_string()]).unwrap(), after, "an app it doesn't name stays as it is");

        // Your own hook stays when XenoNet's go.
        let mut mixed = written;
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

        assert_eq!(
            (and_list(&["A"]), and_list(&["A", "B"]), and_list(&["A", "B", "C"]), and_list(&[])),
            ("A".to_string(), "A and B".to_string(), "A, B and C".to_string(), String::new())
        );
    }

    #[test]
    fn nothing_connects_until_an_adapter_is_in_place() {
        // No adapter is installed in this crate's tests, so the registry's
        // apps have no format to connect through.
        assert_eq!(Connect::new(options(47840.0)).ids(), Vec::<String>::new());
    }
}
