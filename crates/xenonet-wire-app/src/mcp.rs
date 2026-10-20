//! What an MCP server on the map is (core/mcp-info.js, mcp-discovery.js;
//! crates/xenonet-app/src/mcp_info.rs and mcp_discovery.rs), without its
//! configuration or credentials. Collecting a catalog is the Rust's state
//! machine; this answers what it asks: Codex's word on the server, and the
//! server's own, by starting the one already configured and listing what it
//! offers (never calling a tool, reading a resource or signing in). The
//! demo's catalogs are made up.
//!
//! The listing is a small MCP client of its own, over a program's standard
//! input and output or Streamable HTTP, where the JS had the MCP SDK's.
//! Codex is asked through the app-server the map's Codex reader runs
//! (State::codex_request); while that's pending, as Codex not answering.

use crate::fx::Locked;
use crate::fx::{self, now_ms};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use xenonet_app::{mcp_discovery as discovery, mcp_info as info};
use xenonet_service::cores::{McpInfo, State, R};

pub struct Catalog {
    state: Arc<dyn State>,
    settings: Value,
    version: String,
    cache: Mutex<HashMap<String, Value>>,
    pending: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

// What collecting reads of the settings, and of each session.
fn settings_of(c: &Value) -> Value {
    json!({ "claudeConfigFile": c.get("claudeConfigFile"), "claudeDesktopDir": c.get("claudeDesktopDir"), "homeDir": c.get("homeDir") })
}

fn session_of(s: &Value) -> Value {
    json!({ "id": s.get("id"), "app": s.get("app"), "title": s.get("title"), "status": s.get("status"), "cwd": s.get("cwd"), "mcp": s.get("mcp") })
}

// The Rust reads the files itself.
fn read_file(f: Option<&str>) -> Value {
    f.and_then(|f| std::fs::read(f).ok()).and_then(|b| xenonet_base::json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or_else(|| json!({}))
}

impl Catalog {
    pub fn new(config: &Value, version: &str, state: Arc<dyn State>) -> Self {
        Catalog { state, settings: settings_of(config), version: version.into(), cache: Mutex::new(HashMap::new()), pending: Mutex::new(HashMap::new()) }
    }

    async fn collect(&self, server: &str, sessions: &[Value]) -> Value {
        let mut read = |f: Option<&str>| read_file(f);
        let sessions: Vec<Value> = sessions.iter().map(session_of).collect();
        let (mut c, mut next) = info::begin(server, &sessions, &self.settings, now_ms(), &mut read);
        loop {
            let answer = match next {
                info::Next::Done(r) => return r,
                info::Next::Request { method, params } => match self.state.codex_request(method, params).await {
                    Ok(v) => info::Answer::Ok(v),
                    Err(e) => info::Answer::Failed { name: None, code: Some(json!(e.code.unwrap_or_else(|| "unavailable".into()))) },
                },
                info::Next::Discover(spec) => match discover(&spec, &self.version).await {
                    Ok(found) => info::Answer::Ok(found),
                    Err((name, code)) => info::Answer::Failed { name, code },
                },
            };
            (c, next) = info::step(c, answer, &self.settings, &mut read);
        }
    }
}

#[async_trait]
impl McpInfo for Catalog {
    async fn get(&self, server: &str, sessions: &Value, refresh: bool) -> R<Value> {
        let hit = || self.cache.locked().get(server).cloned().filter(|h| info::fresh(h["checkedAt"].as_f64().unwrap_or(0.0), now_ms()));
        if !refresh {
            if let Some(h) = hit() {
                return Ok(h);
            }
        }
        // One collection a server at a time: an ask meanwhile gets its result.
        let lock = self.pending.locked().entry(server.to_string()).or_default().clone();
        let was_busy = lock.try_lock().is_err();
        let _one = lock.lock().await;
        if was_busy {
            if let Some(h) = self.cache.locked().get(server).cloned() {
                return Ok(h);
            }
        }
        let r = self.collect(server, sessions.as_array().map_or(&[][..], Vec::as_slice)).await;
        self.cache.locked().insert(server.to_string(), r.clone());
        Ok(r)
    }

    fn demo(&self, server: &str) -> R<Value> {
        Ok(info::demo_mcp_info(server, now_ms()))
    }
}

/// Why a listing failed: the error's name and code, as the catalog reads it.
type Why = (Option<String>, Option<Value>);

fn why(name: &str, code: Option<Value>) -> Why {
    (Some(name.into()), code)
}

/// One MCP connection: a request and its answer.
#[async_trait]
trait Channel: Send {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Why>;
    async fn notify(&mut self, method: &str);
    async fn close(&mut self);
}

struct Stdio {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    lines: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
}

#[async_trait]
impl Channel for Stdio {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Why> {
        self.id += 1;
        let id = self.id;
        let line = format!("{}\n", xenonet_base::json::stringify(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }), 0));
        self.stdin.write_all(line.as_bytes()).await.map_err(|_| why("Error", None))?;
        let wait = async {
            loop {
                let Ok(Some(l)) = self.lines.next_line().await else { return Err(why("Error", Some(json!("ECONNRESET")))) };
                let Ok(msg) = xenonet_base::json::parse(&l) else { continue };
                // The server asking something of its own (roots, sampling):
                // this client offers nothing.
                if msg.get("method").is_some() {
                    if let Some(rid) = msg.get("id") {
                        let no = json!({ "jsonrpc": "2.0", "id": rid, "error": { "code": -32601, "message": "Method not found" } });
                        let _ = self.stdin.write_all(format!("{}\n", xenonet_base::json::stringify(&no, 0)).as_bytes()).await;
                    }
                    continue;
                }
                if msg.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                return answer_of(&msg);
            }
        };
        tokio::time::timeout(Duration::from_millis(u64::from(discovery::TIMEOUT_MS)), wait).await.unwrap_or_else(|_| Err(why("McpError", Some(json!(-32001)))))
    }

    async fn notify(&mut self, method: &str) {
        let line = format!("{}\n", xenonet_base::json::stringify(&json!({ "jsonrpc": "2.0", "method": method }), 0));
        let _ = self.stdin.write_all(line.as_bytes()).await;
    }

    async fn close(&mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = self.child.kill().await;
    }
}

fn answer_of(msg: &Value) -> Result<Value, Why> {
    match msg.get("error") {
        Some(e) => Err(why("McpError", e.get("code").cloned())),
        None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
    }
}

struct Http {
    url: String,
    headers: Vec<(String, String)>,
    session: Option<String>,
    id: u64,
}

impl Http {
    async fn post(&mut self, body: Value, id: Option<u64>) -> Result<Value, Why> {
        let mut headers = vec![("content-type".to_string(), "application/json".to_string()), ("accept".to_string(), "application/json, text/event-stream".to_string())];
        headers.extend(self.headers.iter().cloned());
        if let Some(s) = &self.session {
            headers.push(("mcp-session-id".into(), s.clone()));
        }
        let res = xenonet_service::net::fetch(
            "POST",
            &self.url,
            &headers,
            Some(xenonet_base::json::stringify(&body, 0).into_bytes()),
            Duration::from_millis(u64::from(discovery::TIMEOUT_MS)),
        )
        .await
        .map_err(|_| why("TypeError", None))?;
        if res.status == 401 {
            return Err(why("StreamableHTTPError", Some(json!(401))));
        }
        if !res.ok() && res.status != 202 {
            return Err(why("StreamableHTTPError", Some(json!(res.status))));
        }
        if let Some(s) = res.header("mcp-session-id") {
            self.session = Some(s.to_string());
        }
        let Some(id) = id else { return Ok(Value::Null) };
        let text = res.text();
        let is_stream = res.header("content-type").is_some_and(|t| t.contains("text/event-stream"));
        let messages: Vec<Value> = if is_stream {
            text.split("\n\n")
                .filter_map(|event| {
                    let data: Vec<&str> = event.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
                    (!data.is_empty()).then(|| xenonet_base::json::parse(&data.join("\n")).ok()).flatten()
                })
                .collect()
        } else {
            match xenonet_base::json::parse(&text) {
                Ok(Value::Array(a)) => a,
                Ok(v) => vec![v],
                Err(_) => Vec::new(),
            }
        };
        messages.iter().find(|m| m.get("id").and_then(Value::as_u64) == Some(id)).map_or(Err(why("Error", None)), answer_of)
    }
}

#[async_trait]
impl Channel for Http {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, Why> {
        self.id += 1;
        let id = self.id;
        self.post(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }), Some(id)).await
    }

    async fn notify(&mut self, method: &str) {
        let _ = self.post(json!({ "jsonrpc": "2.0", "method": method }), None).await;
    }

    async fn close(&mut self) {}
}

// getDefaultEnvironment(): what the MCP SDK passes a program it starts.
fn default_env() -> Vec<(String, String)> {
    let names: &[&str] = if cfg!(windows) {
        &["APPDATA", "HOMEDRIVE", "HOMEPATH", "LOCALAPPDATA", "PATH", "PROCESSOR_ARCHITECTURE", "SYSTEMDRIVE", "SYSTEMROOT", "TEMP", "USERNAME", "USERPROFILE", "PROGRAMFILES"]
    } else {
        &["HOME", "LOGNAME", "PATH", "SHELL", "TERM", "USER"]
    };
    names.iter().filter_map(|n| std::env::var(n).ok().filter(|v| !v.starts_with("()")).map(|v| (n.to_string(), v))).collect()
}

fn open(how: &Value) -> Result<Box<dyn Channel>, Why> {
    match how["kind"].as_str() {
        Some("stdio") => {
            let command = how["command"].as_str().filter(|c| !c.is_empty()).ok_or_else(|| why("Error", None))?;
            let args: Vec<String> = how["args"].as_array().map(|a| a.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default();
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(&args)
                .env_clear()
                .envs(default_env())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            if let Some(env) = how["env"].as_object() {
                cmd.envs(fx::env_pairs(env));
            }
            if let Some(cwd) = how["cwd"].as_str().filter(|c| !c.is_empty()) {
                cmd.current_dir(cwd);
            }
            let mut child = cmd.spawn().map_err(|e| why("Error", xenonet_base::fs::code_of(&e).map(|c| json!(c))))?;
            let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return Err(why("Error", None)) };
            Ok(Box::new(Stdio { child, stdin, lines: BufReader::new(stdout).lines(), id: 0 }))
        }
        Some("http") => {
            let headers = how["headers"].as_object().map(|h| h.iter().map(|(k, v)| (k.clone(), xenonet_base::text::js_string(v))).collect()).unwrap_or_default();
            Ok(Box::new(Http { url: how["url"].as_str().unwrap_or("").to_string(), headers, session: None, id: 0 }))
        }
        // Server-Sent Events' older transport needs a stream held open
        // beside the requests, which this client doesn't keep.
        _ => Err(why("Error", None)),
    }
}

async fn pages(ch: &mut dyn Channel, method: &str, key: &str) -> Result<Vec<Value>, Why> {
    let mut rows = Vec::new();
    let mut cursor: Option<Value> = None;
    for n in 1.. {
        let params = cursor.as_ref().map_or_else(|| json!({}), |c| json!({ "cursor": c }));
        let page = ch.request(method, params).await?;
        rows.extend(page.get(key).and_then(Value::as_array).cloned().unwrap_or_default());
        let next = page.get("nextCursor").filter(|c| !c.is_null()).cloned();
        if !discovery::another_page(n, rows.len(), cursor.as_ref(), next.as_ref()) {
            break;
        }
        cursor = next;
    }
    Ok(rows)
}

/// discoverMcp(spec): what the server says it offers.
pub async fn discover(spec: &Value, version: &str) -> Result<Value, Why> {
    let how = discovery::transport(spec, &fx::env_map());
    let by_url = how["kind"] != "stdio";
    let mut ch = open(&how)?;
    let talk = async {
        let init =
            ch.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "xenonet-inspector", "version": version } })).await?;
        ch.notify("notifications/initialized").await;
        let caps = init.get("capabilities").cloned().unwrap_or_else(|| json!({}));
        let has = |k: &str| xenonet_base::text::truthy(caps.get(k));
        let tools = if has("tools") { pages(ch.as_mut(), "tools/list", "tools").await? } else { Vec::new() };
        let (resources, templates) = if has("resources") {
            let r = pages(ch.as_mut(), "resources/list", "resources").await?;
            let t = pages(ch.as_mut(), "resources/templates/list", "resourceTemplates").await.unwrap_or_default();
            (r, t)
        } else {
            (Vec::new(), Vec::new())
        };
        Ok::<Value, Why>(discovery::found(
            init.get("serverInfo"),
            Some(&caps),
            init.get("instructions"),
            Value::Array(tools),
            Value::Array(resources),
            Value::Array(templates),
            by_url,
        ))
    };
    let out = tokio::time::timeout(Duration::from_millis(u64::from(discovery::SIGNAL_MS)), talk).await.unwrap_or_else(|_| Err(why("AbortError", None)));
    ch.close().await;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_demos_catalog_is_made_up() {
        let c = Catalog::new(&json!({ "homeDir": "/nowhere" }), "0.0.0", Arc::new(xenonet_service::cores::Pending));
        let d = c.demo("agent-wiki").unwrap();
        assert_eq!(d["server"], "agent-wiki");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_configured_program_is_listed_never_called() {
        // A made-up server: answers initialize and tools/list, line by line.
        let script = r#"while IFS= read -r line; do
  case "$line" in
    *'"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"1.0.0"}}}' ;;
    *'"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"look","description":"Looks","inputSchema":{"type":"object"}}]}}' ;;
  esac
done"#;
        let found = discover(&json!({ "command": "/bin/sh", "args": ["-c", script] }), "0.0.0").await.unwrap();
        assert_eq!(found["tools"][0]["name"], "look");
        assert_eq!(found["serverInfo"]["name"], "fake");
        assert_eq!(found["authStatus"], "unsupported");
        let gone = discover(&json!({ "command": "/no/such/server" }), "0.0.0").await.unwrap_err();
        assert_eq!(gone.1, Some(json!("ENOENT")));
    }

    #[tokio::test]
    async fn a_server_no_one_configured_is_from_what_the_sessions_show() {
        let home = std::env::temp_dir().join(format!("xenonet-wire-mcp-{}", std::process::id()));
        let c = Catalog::new(&json!({ "homeDir": home, "claudeConfigFile": home.join(".claude.json") }), "0.0.0", Arc::new(xenonet_service::cores::Pending));
        let sessions = json!([{ "id": "claude:a", "app": "claude", "title": "t", "status": "idle", "mcp": [{ "server": "nowhere", "ts": 5 }] }]);
        let r = c.get("nowhere", &sessions, false).await.unwrap();
        assert_eq!((r["server"].clone(), r["lastCall"].clone(), r["catalog"].clone()), (json!("nowhere"), json!(5), json!(false)));
        assert_eq!(c.get("nowhere", &sessions, false).await.unwrap()["checkedAt"], r["checkedAt"], "fresh for a while");
    }
}
