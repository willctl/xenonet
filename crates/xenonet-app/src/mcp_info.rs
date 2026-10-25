//! core/mcp-info.js: a catalog of an MCP server for the console to show.
//! Configuration and credentials stay here: only presentation metadata
//! and the names of credentials reach the console. Defaults and examples
//! in a tool's schema can hold credentials, so a schema is shown without
//! them, and without vendor metadata.
//!
//! Collecting one server's catalog waits on Codex and on the server
//! itself, so it's a state machine: `begin` and each `step` say what to
//! ask next (a Codex request or a discovery), whoever runs it does the
//! asking, and the answer goes back in with `step`, until it says done.

use crate::jsv::{get, key_count, object, or, raw, strict_eq, text, truthy};
use crate::nodepath::NATIVE;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use url::Url;
use xenonet_base::text::{js_number, js_string};

/// The settings collecting reads: claudeConfigFile, claudeDesktopDir and
/// homeDir, as core/config.js has them.
pub type Settings = Value;

/// A file read as JSON, or `{}` (the JS's `json(file)`, or what a test gave).
pub type Read<'a> = dyn FnMut(Option<&str>) -> Value + 'a;

/// How long a catalog is kept before it's collected again.
pub const FRESH_MS: f64 = 30000.0;

/// A URI as the console may show it: no user or password, and no query or
/// fragment unless it's a template with an optional part ({?x}).
fn public_uri(value: Option<&Value>) -> Value {
    let raw = text(value, 1000);
    let Some(r) = raw.as_str().filter(|r| !r.is_empty()) else { return Value::Null };
    match Url::parse(r) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            if !template().is_match(r) {
                u.set_query(None);
                u.set_fragment(None);
            }
            Value::String(u.as_str().replace("%7B", "{").replace("%7D", "}"))
        }
        Err(_) => raw,
    }
}

fn template() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\{[^}]*\?[^}]*\}").expect("a fixed pattern"))
}

fn credential() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)key|token|secret|auth|password").expect("a fixed pattern"))
}

/// An http(s) URL with no user, password, query or fragment, or only its
/// origin; None for anything else.
pub fn public_url(value: Option<&str>, origin_only: bool) -> Option<String> {
    let mut u = Url::parse(value?).ok()?;
    if !matches!(u.scheme(), "https" | "http") {
        return None;
    }
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.set_query(None);
    u.set_fragment(None);
    Some(if origin_only { u.origin().ascii_serialization() } else { u.as_str().to_string() })
}

fn public_url_of(v: Option<&Value>, origin_only: bool) -> Value {
    // new URL(value) reads anything as a string; undefined and null aren't URLs.
    let s = match v {
        None | Some(Value::Null) => return Value::Null,
        Some(Value::String(s)) => s.clone(),
        Some(other) => js_string(other),
    };
    public_url(Some(&s), origin_only).map_or(Value::Null, Value::String)
}

const SCHEMA_KEYS: [&str; 12] = ["$ref", "$schema", "type", "title", "description", "format", "required", "enum", "minimum", "maximum", "minLength", "maxLength"];

/// A tool's input schema as the console shows it: what describes the
/// input, without defaults, examples or anything else.
pub fn display_schema(value: Option<&Value>, depth: usize) -> Value {
    if depth > 12 {
        return Value::Null;
    }
    let v = object(value);
    let mut out = Map::new();
    for k in SCHEMA_KEYS {
        if let Some(x) = v.get(k) {
            if crate::jstext::utf16_len(&xenonet_base::json::stringify(x, 0)) < 8000 {
                out.insert(k.into(), x.clone());
            }
        }
    }
    let nested = |m: &Map<String, Value>| Value::Object(m.iter().take(100).map(|(k, s)| (k.clone(), display_schema(Some(s), depth + 1))).collect());
    if truthy(v.get("properties")) {
        out.insert("properties".into(), nested(object(v.get("properties"))));
    }
    for k in ["$defs", "definitions"] {
        if truthy(v.get(k)) {
            out.insert(k.into(), nested(object(v.get(k))));
        }
    }
    for k in ["items", "additionalProperties"] {
        if let Some(x) = v.get(k) {
            out.insert(k.into(), if x.is_boolean() { x.clone() } else { display_schema(Some(x), depth + 1) });
        }
    }
    for k in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(a)) = v.get(k) {
            out.insert(k.into(), Value::Array(a.iter().take(30).map(|s| display_schema(Some(s), depth + 1)).collect()));
        }
    }
    Value::Object(out)
}

const STATUSES: [&str; 7] = ["notStarted", "starting", "connected", "authenticationRequired", "failed", "cancelled", "disabled"];
const AUTHS: [&str; 5] = ["unknown", "unsupported", "notLoggedIn", "bearerToken", "oAuth"];
const CAPABILITIES: [&str; 6] = ["tools", "resources", "prompts", "logging", "completions", "tasks"];
const HINTS: [&str; 4] = ["readOnlyHint", "destructiveHint", "idempotentHint", "openWorldHint"];

fn one_of(v: Option<&Value>, allowed: &[&str]) -> Value {
    match v.and_then(Value::as_str) {
        Some(s) if allowed.contains(&s) => Value::String(s.into()),
        _ => Value::String("unknown".into()),
    }
}

fn list(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

fn resource(r: &Value) -> Value {
    let r = Some(r);
    json!({
        "name": text(raw(r, "name"), 200),
        "title": text(raw(r, "title"), 200),
        "description": text(raw(r, "description"), 4000),
        "mimeType": text(raw(r, "mimeType"), 200),
        "uri": public_uri(get(r, "uri").or_else(|| raw(r, "uriTemplate"))),
    })
}

/// What a discovery (Codex's, or XenoNet's own) found, as the console shows it.
pub fn display_catalog(raw_v: Option<&Value>) -> Value {
    let info = object(raw(raw_v, "serverInfo"));
    let caps = object(raw(raw_v, "serverCapabilities"));
    let info_v = Value::Object(info.clone());
    let info_r = Some(&info_v);
    let tools: Vec<Value> = match raw(raw_v, "tools") {
        Some(Value::Array(a)) => a.clone(),
        t => object(t).values().cloned().collect(),
    };
    let tools: Vec<Value> = tools
        .iter()
        .take(500)
        .map(|t| {
            let t = Some(t);
            let ann = raw(t, "annotations");
            let annotations: Map<String, Value> = HINTS.iter().filter_map(|k| raw(ann, k).filter(|x| x.is_boolean()).map(|x| (k.to_string(), x.clone()))).collect();
            json!({
                "name": text(raw(t, "name"), 200),
                "title": text(raw(t, "title"), 200),
                "description": text(raw(t, "description"), 4000),
                "inputSchema": display_schema(raw(t, "inputSchema"), 0),
                "annotations": annotations,
            })
        })
        .collect();
    json!({
        "title": text(get(info_r, "title").or_else(|| raw(info_r, "name")), 200),
        "version": text(raw(info_r, "version"), 100),
        "description": text(raw(info_r, "description"), 4000),
        "website": public_url_of(raw(info_r, "websiteUrl"), false),
        "status": one_of(raw(raw_v, "runtimeStatus"), &STATUSES),
        "auth": one_of(raw(raw_v, "authStatus"), &AUTHS),
        "endpoint": public_url_of(raw(raw_v, "httpOrigin"), true),
        "capabilities": CAPABILITIES.iter().filter(|k| caps.contains_key(**k)).collect::<Vec<_>>(),
        "tools": tools,
        "resources": list(get(raw_v, "resources")).iter().take(500).map(resource).collect::<Vec<_>>(),
        "templates": list(get(raw_v, "resourceTemplates")).iter().take(500).map(resource).collect::<Vec<_>>(),
        "discoveryFailed": truthy(raw(raw_v, "toolsError")),
        "instructions": text(raw(raw_v, "instructions"), 12000),
    })
}

/// The package or plugin manifest beside a local entry point, if there's
/// one: looking doesn't run the command or install a package.
fn package_info(spec: &Value, read: &mut Read<'_>) -> Map<String, Value> {
    let spec_r = Some(spec);
    let args = std::iter::once(raw(spec_r, "command").cloned()).chain(list(raw(spec_r, "args")).into_iter().map(Some));
    for arg in args.flatten() {
        let Some(arg) = arg.as_str().filter(|a| NATIVE.is_absolute(a)).map(String::from) else { continue };
        let mut dir = NATIVE.dirname(&arg);
        for _ in 0..5 {
            let p = read(Some(&NATIVE.join(&[&dir, ".claude-plugin", "plugin.json"])));
            let v = if key_count(&p) > 0 { p } else { read(Some(&NATIVE.join(&[&dir, "package.json"]))) };
            let vr = Some(&v);
            if truthy(raw(vr, "name")) && (truthy(raw(vr, "author")) || truthy(raw(vr, "description"))) {
                let author = match raw(vr, "author") {
                    Some(a @ Value::String(_)) => text(Some(a), 200),
                    a => text(raw(a, "name"), 200),
                };
                let mut m = Map::new();
                m.insert("author".into(), author);
                m.insert("description".into(), text(raw(vr, "description"), 4000));
                m.insert("version".into(), text(raw(vr, "version"), 100));
                m.insert("website".into(), public_url_of(raw(vr, "homepage"), false));
                return m;
            }
            if NATIVE.dirname(&dir) == dir {
                break;
            }
            dir = NATIVE.dirname(&dir);
        }
    }
    Map::new()
}

/// What to do next: ask Codex, discover the server, or nothing more.
#[derive(Clone, Debug, PartialEq)]
pub enum Next {
    Request { method: &'static str, params: Value },
    Discover(Value),
    Done(Value),
}

/// A collection under way. It's handed to whoever runs it as JSON, and back.
pub struct Collect {
    state: Map<String, Value>,
}

impl Collect {
    pub fn to_value(&self) -> Value {
        Value::Object(self.state.clone())
    }

    pub fn from_value(v: Value) -> Self {
        Collect {
            state: match v {
                Value::Object(m) => m,
                _ => Map::new(),
            },
        }
    }

    fn server(&self) -> String {
        self.state.get("server").and_then(Value::as_str).unwrap_or("").to_string()
    }

    fn result(&mut self) -> &mut Map<String, Value> {
        match self.state.entry("result").or_insert_with(|| json!({})) {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    fn specs(&self) -> Vec<Value> {
        list(self.state.get("specs"))
    }

    fn using(&self) -> Vec<Value> {
        list(self.state.get("using"))
    }

    fn add(&mut self, app: &str, scope: &str, servers: Option<&Value>) {
        let server = self.server();
        let Some(spec) = object(servers).get(&server).cloned() else { return };
        if !spec.is_object() && !spec.is_array() {
            return;
        }
        let s = Some(&spec);
        let remote = get(s, "url").or_else(|| raw(s, "serverUrl")).cloned();
        let credentials: Vec<String> = object(raw(s, "env")).keys().filter(|k| credential().is_match(k)).take(30).cloned().collect();
        let connection = json!({
            "app": app,
            "scope": scope,
            "transport": if truthy(remote.as_ref()) { "HTTP" } else { "stdio" },
            "endpoint": public_url_of(remote.as_ref(), true),
            "enabled": raw(s, "enabled") != Some(&Value::Bool(false)),
            "credentials": credentials,
        });
        if let Some(Value::Array(a)) = self.state.get_mut("specs") {
            a.push(spec);
        }
        if let Some(Value::Array(c)) = self.result().get_mut("connections") {
            c.push(connection);
        }
    }

    fn codex_used(&self) -> bool {
        self.using()
            .iter()
            .any(|s| raw(Some(s), "app").and_then(Value::as_str) == Some("codex") || raw(Some(s), "id").and_then(Value::as_str).is_some_and(|id| id.starts_with("codex:")))
    }

    fn status_request(&self) -> Next {
        Next::Request {
            method: "mcpServerStatus/list",
            params: json!({ "serverName": self.server(), "detail": "full", "limit": 100, "cursor": self.state.get("cursor").cloned().unwrap_or(Value::Null) }),
        }
    }

    // After Codex has been asked (or couldn't be): discover the server
    // ourselves unless Codex had its catalog.
    fn discover_or_finish(&mut self, settings: &Settings, read: &mut Read<'_>) -> Next {
        let specs = self.specs();
        let usable = |s: &Value| {
            raw(Some(s), "enabled") != Some(&Value::Bool(false)) && (truthy(raw(Some(s), "command")) || truthy(raw(Some(s), "url")) || truthy(raw(Some(s), "serverUrl")))
        };
        let selected = specs.iter().find(|s| usable(s)).cloned();
        if selected.is_none() && !specs.is_empty() && specs.iter().all(|s| raw(Some(s), "enabled") == Some(&Value::Bool(false))) {
            self.result().insert("status".into(), json!("disabled"));
        }
        let catalog = truthy(self.result().get("catalog"));
        if let (false, Some(Value::Object(mut sel))) = (catalog, selected) {
            let cwd = or(sel.get("cwd"), raw(Some(settings), "homeDir").cloned().unwrap_or(Value::Null));
            sel.insert("cwd".into(), cwd);
            self.state.insert("stage".into(), json!("discover"));
            return Next::Discover(Value::Object(sel));
        }
        self.finish(read)
    }

    fn finish(&mut self, read: &mut Read<'_>) -> Next {
        let server = self.server();
        let specs = self.specs();
        let using = self.using();
        let metadata = specs.iter().map(|s| package_info(s, read)).find(|m| !m.is_empty()).unwrap_or_default();
        let r = self.result();
        for k in ["author", "description", "version", "website"] {
            if r.get(k).is_none_or(Value::is_null) {
                r.insert(k.into(), or(metadata.get(k), Value::Null));
            }
        }
        // The transcript proves use, not that a server is connected now.
        let mut latest = 0.0f64;
        for s in &using {
            for m in list(raw(Some(s), "mcp")) {
                if raw(Some(&m), "server").and_then(Value::as_str) == Some(server.as_str()) {
                    let ts = js_number(Some(&or(raw(Some(&m), "ts"), json!(0))));
                    latest = if ts.is_nan() || latest.is_nan() { f64::NAN } else { latest.max(ts) };
                }
            }
        }
        r.insert("lastCall".into(), if latest == 0.0 || latest.is_nan() { Value::Null } else { xenonet_base::json::number(latest) });
        let sessions: Vec<Value> = using
            .iter()
            .map(|s| {
                let sr = Some(s);
                let mut o = Map::new();
                if let Some(id) = raw(sr, "id") {
                    o.insert("id".into(), id.clone());
                }
                let app = get(sr, "app").cloned().or_else(|| raw(sr, "id").and_then(Value::as_str).map(|id| json!(id.split(':').next().unwrap_or(""))));
                if let Some(a) = app {
                    o.insert("app".into(), a);
                }
                o.insert("title".into(), text(raw(sr, "title"), 300));
                if let Some(st) = raw(sr, "status") {
                    o.insert("status".into(), st.clone());
                }
                Value::Object(o)
            })
            .collect();
        r.insert("sessions".into(), Value::Array(sessions));
        if !truthy(r.get("catalog")) {
            let failed = truthy(r.get("discoveryFailed"));
            r.insert("source".into(), json!(if failed { "Discovery unavailable; observed session activity" } else { "Configuration and observed session activity" }));
        }
        Next::Done(Value::Object(r.clone()))
    }

    fn assign(&mut self, catalog: Value, extra: &[(&str, Value)]) {
        let r = self.result();
        if let Value::Object(c) = catalog {
            for (k, v) in c {
                r.insert(k, v);
            }
        }
        for (k, v) in extra {
            r.insert((*k).into(), v.clone());
        }
    }
}

fn plugin_name(server: &str) -> Option<&str> {
    server.strip_prefix("plugin:").map(|rest| rest.split(':').next().unwrap_or(""))
}

/// Starts collecting a server's catalog: what the settings and the files
/// beside them say, then the first question for Codex.
pub fn begin(server: &str, sessions: &[Value], settings: &Settings, now: f64, read: &mut Read<'_>) -> (Collect, Next) {
    let st = Some(settings);
    let using: Vec<Value> = sessions.iter().filter(|s| list(raw(Some(s), "mcp")).iter().any(|m| raw(Some(m), "server").and_then(Value::as_str) == Some(server))).cloned().collect();
    let result = json!({ "server": server, "title": server, "author": null, "status": "unknown", "auth": "unknown", "tools": [], "resources": [], "templates": [], "capabilities": [], "connections": [], "checkedAt": now, "catalog": false });
    let mut c = Collect { state: Map::new() };
    c.state.insert("server".into(), json!(server));
    c.state.insert("result".into(), result);
    c.state.insert("specs".into(), json!([]));
    c.state.insert("using".into(), Value::Array(using.clone()));
    let file_of = |v: Option<&Value>| v.and_then(Value::as_str).map(String::from);
    let claude = read(file_of(raw(st, "claudeConfigFile")).as_deref());
    c.add("Claude Code", "user", raw(Some(&claude), "mcpServers"));
    for (cwd, project) in object(raw(Some(&claude), "projects")) {
        if using.iter().any(|s| raw(Some(s), "cwd").and_then(Value::as_str) == Some(cwd.as_str())) {
            c.add("Claude Code", "project", raw(Some(project), "mcpServers"));
        }
    }
    let mut cwds: Vec<String> = Vec::new();
    for s in &using {
        if let Some(cwd) = raw(Some(s), "cwd").and_then(Value::as_str).filter(|c| !c.is_empty()) {
            if !cwds.iter().any(|c| c == cwd) {
                cwds.push(cwd.into());
            }
        }
    }
    for cwd in &cwds {
        let m = read(Some(&NATIVE.join(&[cwd, ".mcp.json"])));
        c.add("Claude Code", "project", raw(Some(&m), "mcpServers"));
    }
    if let Some(dir) = get(st, "claudeDesktopDir").filter(|d| truthy(Some(d))) {
        let m = read(Some(&NATIVE.join(&[&js_string(dir), "claude_desktop_config.json"])));
        c.add("Claude Desktop", "user", raw(Some(&m), "mcpServers"));
    }
    // Grok Build's: its config.toml's [mcp_servers.<name>] tables, yours and
    // those in the folders where the server is used.
    if let Some(dir) = get(st, "grokHome").filter(|d| truthy(Some(d))) {
        let m = read(Some(&NATIVE.join(&[&js_string(dir), "config.toml"])));
        c.add("Grok Build", "user", raw(Some(&m), "mcp_servers"));
    }
    for cwd in &cwds {
        let m = read(Some(&NATIVE.join(&[cwd, ".grok", "config.toml"])));
        c.add("Grok Build", "project", raw(Some(&m), "mcp_servers"));
    }
    let home = raw(st, "homeDir").map(js_string).unwrap_or_default();
    let installed = read(Some(&NATIVE.join(&[&home, ".claude", "plugins", "installed_plugins.json"])));
    if let Some(name) = plugin_name(server) {
        for (key, entries) in object(raw(Some(&installed), "plugins")) {
            if key.split('@').next() != Some(name) {
                continue;
            }
            for entry in list(Some(entries)) {
                let Some(install) = raw(Some(&entry), "installPath").filter(|p| truthy(Some(p))).map(js_string) else { continue };
                let manifest = read(Some(&NATIVE.join(&[&install, ".claude-plugin", "plugin.json"])));
                let mr = Some(&manifest);
                let author = match raw(mr, "author") {
                    Some(a @ Value::String(_)) => text(Some(a), 200),
                    a => text(raw(a, "name"), 200),
                };
                let r = c.result();
                for (k, v) in [("author", author), ("description", text(raw(mr, "description"), 4000)), ("version", text(raw(mr, "version"), 100))] {
                    if r.get(k).is_none_or(Value::is_null) {
                        r.insert(k.into(), v);
                    }
                }
                let servers = read(Some(&NATIVE.join(&[&install, ".mcp.json"])));
                let local = server.split(':').skip(2).collect::<Vec<_>>().join(":");
                let table = get(Some(&servers), "mcpServers").unwrap_or(&servers);
                let Some(spec) = raw(Some(table), &local).filter(|s| truthy(Some(s))).cloned() else { continue };
                let mut spec_m = object(Some(&spec)).clone();
                // `args: spec.args?.map(…)`: undefined, so left out, unless
                // it's a list.
                if let Some(Value::Array(args)) = spec_m.get("args") {
                    let args: Vec<Value> = args.iter().map(|a| Value::String(js_string(a).replace("${CLAUDE_PLUGIN_ROOT}", &install))).collect();
                    spec_m.insert("args".into(), Value::Array(args));
                } else {
                    spec_m.shift_remove("args");
                }
                spec_m.insert("cwd".into(), json!(install));
                let mut wrap = Map::new();
                wrap.insert(server.into(), Value::Object(spec_m));
                c.add("Claude Code", "plugin", Some(&Value::Object(wrap)));
            }
        }
    }
    c.state.insert("stage".into(), json!("config"));
    (c, Next::Request { method: "config/read", params: json!({ "includeLayers": false }) })
}

/// What a question asked came back with: its answer, or the error's name
/// and code.
pub enum Answer {
    Ok(Value),
    Failed { name: Option<String>, code: Option<Value> },
}

/// Takes the answer to what was asked last and says what's next.
pub fn step(mut c: Collect, answer: Answer, settings: &Settings, read: &mut Read<'_>) -> (Collect, Next) {
    let stage = c.state.get("stage").and_then(Value::as_str).unwrap_or("").to_string();
    let next = match stage.as_str() {
        "config" => {
            let mut configured = false;
            // `const { config } = await request(…)`: an answer of null or
            // undefined throws, which is caught.
            if let Answer::Ok(ok) = &answer {
                if !ok.is_null() {
                    let codex = object(raw(raw(Some(ok), "config"), "mcp_servers")).clone();
                    configured = codex.contains_key(&c.server());
                    c.add("Codex", "effective", Some(&Value::Object(codex)));
                }
            }
            if configured || c.codex_used() {
                c.state.insert("stage".into(), json!("status"));
                c.state.insert("cursor".into(), Value::Null);
                c.state.insert("rows".into(), json!([]));
                c.state.insert("page".into(), json!(0));
                c.status_request()
            } else {
                c.discover_or_finish(settings, read)
            }
        }
        "status" => {
            // `rows.push(...(page.data ?? []))`: an answer that isn't a page
            // throws, which, like Codex failing, ends the listing.
            let page = match &answer {
                Answer::Ok(p) if !p.is_null() => Some(p.clone()),
                _ => None,
            };
            let rows = page.as_ref().and_then(|p| match raw(Some(p), "data") {
                Some(Value::Array(r)) => Some(r.clone()),
                None | Some(Value::Null) => Some(Vec::new()),
                _ => None,
            });
            match (page, rows) {
                (Some(p), Some(rows)) => {
                    if let Some(Value::Array(all)) = c.state.get_mut("rows") {
                        all.extend(rows);
                    }
                    let cursor = c.state.get("cursor").cloned();
                    let next_cursor = raw(Some(&p), "nextCursor").cloned();
                    let i = c.state.get("page").and_then(Value::as_u64).unwrap_or(0) + 1;
                    c.state.insert("page".into(), json!(i));
                    if !truthy(next_cursor.as_ref()) || strict_eq(next_cursor.as_ref(), cursor.as_ref()) || i >= 10 {
                        let server = c.server();
                        let found = list(c.state.get("rows")).into_iter().find(|r| raw(Some(r), "name").and_then(Value::as_str) == Some(server.as_str()));
                        if let Some(rawv) = found {
                            let catalog = display_catalog(Some(&rawv));
                            c.assign(catalog, &[("catalog", json!(!truthy(raw(Some(&rawv), "toolsError")))), ("source", json!("Codex MCP discovery"))]);
                        }
                        c.discover_or_finish(settings, read)
                    } else {
                        c.state.insert("cursor".into(), next_cursor.unwrap_or(Value::Null));
                        c.status_request()
                    }
                }
                _ => {
                    c.result().insert("discoveryFailed".into(), json!(true));
                    c.discover_or_finish(settings, read)
                }
            }
        }
        "discover" => {
            match answer {
                Answer::Ok(found) => {
                    let catalog = display_catalog(Some(&found));
                    c.assign(catalog, &[("catalog", json!(true)), ("source", json!("Read-only MCP discovery"))]);
                }
                Answer::Failed { name, code } => {
                    let auth = name.as_deref() == Some("UnauthorizedError") || code.as_ref().is_some_and(|c| c.as_f64() == Some(401.0));
                    let r = c.result();
                    r.insert("discoveryFailed".into(), json!(true));
                    r.insert("status".into(), json!(if auth { "authenticationRequired" } else { "failed" }));
                }
            }
            c.finish(read)
        }
        _ => c.finish(read),
    };
    (c, next)
}

/// Whether a catalog collected at `checked_at` is still fresh at `now`.
pub fn fresh(checked_at: f64, now: f64) -> bool {
    now - checked_at < FRESH_MS
}

/// The demo's made-up catalog.
pub fn demo_mcp_info(server: &str, now: f64) -> Value {
    let title = match server {
        "xenolith" => "Xenolith",
        "agent-wiki" => "Agent Wiki",
        _ => "Reference Library",
    };
    json!({ "server": server, "title": title, "author": "XenoNet demo", "version": "1.0.0", "description": "A made-up MCP catalog for exploring shared systems.", "status": "connected", "auth": "unsupported", "catalog": true, "source": "Demo MCP discovery", "checkedAt": now, "capabilities": ["tools", "resources"], "connections": [{ "app": "Codex", "scope": "user", "transport": "stdio", "enabled": true, "credentials": [] }], "tools": [{ "name": "search", "title": "Search references", "description": "Find relevant reference material by keyword.", "annotations": { "readOnlyHint": true, "destructiveHint": false }, "inputSchema": { "type": "object", "properties": { "query": { "type": "string", "description": "Words to find" } }, "required": ["query"] } }, { "name": "save_note", "description": "Save a note to the shared library.", "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true }, "inputSchema": { "type": "object", "properties": { "title": { "type": "string" }, "body": { "type": "string" } }, "required": ["title", "body"] } }], "resources": [{ "name": "Guide", "description": "The shared reference guide.", "uri": "library://guide", "mimeType": "text/markdown" }], "templates": [{ "name": "Topic", "description": "A reference page by topic.", "uri": "library://topics/{topic}", "mimeType": "text/markdown" }], "sessions": [], "lastCall": null })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_leaves_credentials_out() {
        let secret = "do-not-display-this";
        let raw = json!({ "runtimeStatus": "connected", "authStatus": "oAuth", "httpOrigin": format!("https://user:{secret}@example.com/mcp?token={secret}"), "serverInfo": { "name": "library", "version": "1", "websiteUrl": "javascript:alert(1)", "_meta": { "token": secret } }, "serverCapabilities": { "tools": {}, "secret": secret }, "tools": { "search": { "name": "search", "inputSchema": { "type": "object", "properties": { "token": { "type": "string", "default": secret, "examples": [secret] } }, "_meta": { "secret": secret } }, "annotations": { "readOnlyHint": true, "token": secret }, "_meta": { "secret": secret } } }, "resources": [], "resourceTemplates": [] });
        let info = display_catalog(Some(&raw));
        assert_eq!(info["endpoint"], "https://example.com");
        assert_eq!(info["website"], Value::Null);
        assert_eq!(info["tools"][0]["annotations"], json!({ "readOnlyHint": true }));
        assert!(!info.to_string().contains(secret));
        assert_eq!(public_url(Some("https://example.com/about?key=hidden#secret"), false).as_deref(), Some("https://example.com/about"));
        assert_eq!(public_uri(Some(&json!("library://topics/{topic}{?q}"))), json!("library://topics/{topic}{?q}"));
        assert_eq!(public_uri(Some(&json!("https://u:p@h/x?y#z"))), json!("https://h/x"));
    }

    #[test]
    fn collecting_asks_codex_then_discovers_then_reads_the_package() {
        let spec = json!({ "command": "node", "args": ["/workspace/library/index.js"], "env": { "API_TOKEN": "private-value" } });
        // The package's path as this host spells it.
        let package = NATIVE.join(&["/workspace/library", "package.json"]);
        let mut read = |f: Option<&str>| match f {
            Some("claude-config") => json!({ "mcpServers": { "library": spec.clone(), "other": { "command": "never-run" } } }),
            Some(p) if p == package => json!({ "name": "library", "author": { "name": "Library authors", "email": "not-needed" }, "version": "2" }),
            _ => json!({}),
        };
        let settings = json!({ "homeDir": "/workspace", "claudeConfigFile": "claude-config" });
        let using = [json!({ "id": "claude:one", "app": "claude", "title": "Look up references", "mcp": [{ "server": "library", "ts": 30 }] })];
        let (c, next) = begin("library", &using, &settings, 100.0, &mut read);
        assert_eq!(next, Next::Request { method: "config/read", params: json!({ "includeLayers": false }) });
        let (c, next) = step(c, Answer::Ok(json!({ "config": {} })), &settings, &mut read);
        let Next::Discover(sel) = next else { panic!("{next:?}") };
        assert_eq!(sel["env"]["API_TOKEN"], "private-value");
        assert_eq!(sel["cwd"], "/workspace");
        let (_, next) = step(
            c,
            Answer::Ok(
                json!({ "serverInfo": { "name": "library", "version": "2" }, "runtimeStatus": "connected", "authStatus": "unsupported", "tools": [], "resources": [], "resourceTemplates": [] }),
            ),
            &settings,
            &mut read,
        );
        let Next::Done(info) = next else { panic!() };
        assert_eq!(info["author"], "Library authors");
        assert_eq!(info["lastCall"], 30);
        assert_eq!(info["connections"][0]["credentials"], json!(["API_TOKEN"]));
        assert!(!info.to_string().contains("private-value") && !info.to_string().contains("not-needed"));
    }
}
