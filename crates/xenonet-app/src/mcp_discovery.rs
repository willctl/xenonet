//! core/mcp-discovery.js: only the selected, already configured server is
//! started. Discovery lists what it offers; it never calls a tool, reads a
//! resource or starts an OAuth flow.
//!
//! The talking is MCP's, through the client the caller has (the MCP SDK's
//! in the twin, for now): this says how to reach the server, how far to
//! page, and what the answer is.

use crate::jsv::{get, raw, truthy};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_base::text::js_string;

pub const TIMEOUT_MS: u32 = 8000;
pub const SIGNAL_MS: u32 = 12000;
pub const MAX_PAGES: usize = 10;
pub const MAX_ROWS: usize = 500;

fn var_ref() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").expect("a fixed pattern"))
}

/// A configured value with ${VAR}s filled in from this process's
/// environment ("" for one that isn't set).
pub fn env_value(v: &Value, env: &Map<String, Value>) -> String {
    let s = js_string(v);
    var_ref().replace_all(&s, |c: &regex::Captures<'_>| env.get(&c[1]).filter(|v| !v.is_null()).map(js_string).unwrap_or_default()).into_owned()
}

/// How to reach a server: by URL (Server-Sent Events for type "sse",
/// Streamable HTTP otherwise), with its headers filled in, or by starting
/// its command, with its environment on top of the default one.
pub fn transport(spec: &Value, env: &Map<String, Value>) -> Value {
    let s = Some(spec);
    let url = get(s, "url").or_else(|| raw(s, "serverUrl")).cloned();
    let fill = |m: Option<&Value>| -> Map<String, Value> { crate::jsv::object(m).iter().map(|(k, v)| (k.clone(), Value::String(env_value(v, env)))).collect() };
    if truthy(url.as_ref()) {
        let kind = if raw(s, "type").and_then(Value::as_str) == Some("sse") { "sse" } else { "http" };
        return json!({ "kind": kind, "url": url.map(|u| js_string(&u)), "headers": fill(raw(s, "headers")) });
    }
    let args = match raw(s, "args") {
        None | Some(Value::Null) => json!([]),
        Some(a) => a.clone(),
    };
    json!({ "kind": "stdio", "command": raw(s, "command").cloned().unwrap_or(Value::Null), "args": args, "cwd": raw(s, "cwd").cloned().unwrap_or(Value::Null), "env": fill(raw(s, "env")) })
}

/// Whether to ask for another page after `pages` pages, `rows` rows so far,
/// the cursor this page was asked with and the one it gave.
pub fn another_page(pages: usize, rows: usize, cursor: Option<&Value>, next: Option<&Value>) -> bool {
    pages < MAX_PAGES && truthy(next) && !crate::jsv::strict_eq(cursor, next) && rows < MAX_ROWS
}

/// The answer, from what the client found.
pub fn found(server_info: Option<&Value>, capabilities: Option<&Value>, instructions: Option<&Value>, tools: Value, resources: Value, templates: Value, by_url: bool) -> Value {
    let mut out = Map::new();
    if let Some(v) = server_info {
        out.insert("serverInfo".into(), v.clone());
    }
    out.insert("serverCapabilities".into(), capabilities.filter(|c| !c.is_null()).cloned().unwrap_or_else(|| json!({})));
    if let Some(v) = instructions {
        out.insert("instructions".into(), v.clone());
    }
    out.insert("tools".into(), tools);
    out.insert("resources".into(), resources);
    out.insert("resourceTemplates".into(), templates);
    out.insert("runtimeStatus".into(), json!("connected"));
    out.insert("authStatus".into(), json!(if by_url { "unknown" } else { "unsupported" }));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_the_environment_and_pages_within_bounds() {
        let env: Map<String, Value> = json!({ "TOKEN": "t0k" }).as_object().unwrap().clone();
        let t = transport(&json!({ "url": "https://x/mcp", "headers": { "Authorization": "Bearer ${TOKEN}", "X": "${MISSING}" } }), &env);
        assert_eq!(t, json!({ "kind": "http", "url": "https://x/mcp", "headers": { "Authorization": "Bearer t0k", "X": "" } }));
        assert_eq!(transport(&json!({ "command": "node" }), &env)["args"], json!([]));
        assert!(another_page(1, 10, None, Some(&json!("c1"))));
        assert!(!another_page(1, 10, Some(&json!("c1")), Some(&json!("c1"))));
        assert!(!another_page(10, 10, None, Some(&json!("c2"))));
        assert!(!another_page(1, 500, None, Some(&json!("c2"))));
    }
}
