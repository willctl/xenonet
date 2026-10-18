// The xenonet MCP server over stdio, as Claude Code meets it: the
// 2026-07-28 probe (server/discover) first, then the 2025 handshake. A
// channel session gets method-not-found for the probe, the 2025-11-25
// handshake with experimental["claude/channel"], and each message its long
// poll brings as notifications/claude/channel. A fake XenoNet stands in.

#![expect(clippy::unwrap_used, clippy::panic, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const SESSION: &str = "0190d3e0-0000-7000-8000-00000000000a";

// A XenoNet that answers the channel's polls with `polls` in turn (then
// says it's off), and any other call with { op }. What it heard goes to
// the channel: (path, agent header, api version header, body).
fn xenonet(polls: Vec<Value>) -> (u16, mpsc::Receiver<(String, String, String, Value)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut polls = polls.into_iter();
        for s in listener.incoming() {
            let mut s = s.unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let path = line.split(' ').nth(1).unwrap_or("").to_string();
            let (mut length, mut agent, mut api) = (0, String::new(), String::new());
            loop {
                let mut h = String::new();
                r.read_line(&mut h).unwrap();
                let h = h.trim_end().to_string();
                if h.is_empty() {
                    break;
                }
                let (k, v) = h.split_once(':').unwrap();
                match k.to_ascii_lowercase().as_str() {
                    "content-length" => length = v.trim().parse().unwrap(),
                    "x-xenonet-agent" => agent = v.trim().to_string(),
                    "x-xenonet-api-version" => api = v.trim().to_string(),
                    _ => {}
                }
            }
            let mut got = vec![0; length];
            r.read_exact(&mut got).unwrap();
            let body: Value = serde_json::from_slice(&got).unwrap_or(Value::Null);
            let answer = if path == "/api/engine/channel" { polls.next().unwrap_or(json!({ "items": [], "off": true })) } else { json!({ "op": path }) };
            tx.send((path, agent, api, body)).unwrap();
            let text = answer.to_string();
            write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{text}", text.len()).unwrap();
        }
    });
    (port, rx)
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    out: mpsc::Receiver<Value>,
}

impl Server {
    fn start(port: u16, channel: bool) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xenonet-mcp"))
            .env("XENONET_PORT", port.to_string())
            .env("CLAUDE_CODE_SESSION_ID", SESSION)
            .env("XENONET_CHANNEL", if channel { "1" } else { "0" })
            .env_remove("CLAUDE_PID")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout: ChildStdout = child.stdout.take().unwrap();
        let (tx, out) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                // Newline-delimited JSON-RPC: one message a line.
                tx.send(serde_json::from_str(&line).unwrap_or_else(|_| panic!("not JSON: {line}"))).unwrap();
            }
        });
        Server { child, stdin, out }
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn next(&self) -> Value {
        self.out.recv_timeout(Duration::from_secs(10)).expect("an answer")
    }

    fn ask(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let m = self.next();
            if m["id"] == json!(id) {
                return m;
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

// The 2026-07-28 probe, as a client of that revision sends it: the
// version and the client's capabilities in the request's _meta.
fn probe() -> Value {
    json!({ "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}, "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "2.1.302" } } })
}

fn init(s: &mut Server) -> Value {
    let r = s.ask(1, "initialize", json!({ "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "claude-code", "version": "2.1.302" } }));
    s.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
    r
}

#[test]
fn a_channel_session_gets_the_2025_handshake_and_its_messages() {
    let message = json!({ "id": 7, "from": "Xenolith <port>", "text": "[XenoNet from Xenolith] (message #7) hi" });
    let (port, heard) = xenonet(vec![json!({ "items": [message] })]);
    let mut s = Server::start(port, true);

    let probe = s.ask(0, "server/discover", probe());
    assert_eq!(probe["error"]["code"], json!(-32601), "the 2026 probe isn't a method here: {probe}");

    let r = init(&mut s);
    assert_eq!(r["result"]["protocolVersion"], json!("2025-11-25"), "{r}");
    assert_eq!(r["result"]["capabilities"]["experimental"], json!({ "claude/channel": {} }));
    assert_eq!(r["result"]["serverInfo"]["name"], json!("xenonet"));
    assert!(r["result"]["instructions"].as_str().unwrap().contains("<channel source=\"xenonet\""));

    // The message the long poll brought, as Claude Code reads a channel's.
    let note = loop {
        let m = s.next();
        if m.get("method") == Some(&json!("notifications/claude/channel")) {
            break m;
        }
    };
    assert_eq!(
        note,
        json!({ "jsonrpc": "2.0", "method": "notifications/claude/channel", "params": { "content": "[XenoNet from Xenolith] (message #7) hi", "meta": { "from": "Xenolith port", "message_id": "7" } } })
    );
    let (path, agent, api, body) = heard.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((path.as_str(), agent, api), ("/api/engine/channel", format!("claude:{SESSION}"), "1".to_string()));
    assert_eq!(body["waitSeconds"], json!(25));

    let tools = s.ask(2, "tools/list", json!({}));
    let names: Vec<_> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names, ["whoami", "peers", "who_has", "send", "inbox", "claim", "release", "focus", "doctor", "ask_gateway_token"]);

    let call = s.ask(3, "tools/call", json!({ "name": "send", "arguments": { "to": "repo", "text": "hello", "reply_to": 7 } }));
    assert!(call["result"].get("isError").is_none(), "an answer, as the JS server gives it: {call}");
    let (path, agent, _, body) = loop {
        let h = heard.recv_timeout(Duration::from_secs(5)).unwrap();
        if h.0 != "/api/engine/channel" {
            break h;
        }
    };
    assert_eq!((path.as_str(), agent.as_str()), ("/api/engine/send", format!("claude:{SESSION}").as_str()));
    assert_eq!((body["to"].clone(), body["text"].clone(), body["replyTo"].clone()), (json!("repo"), json!("hello"), json!(7)));
}

#[test]
fn another_session_gets_both_eras_and_no_channel() {
    let (port, _heard) = xenonet(vec![]);
    let mut s = Server::start(port, false);
    let probe = s.ask(0, "server/discover", probe());
    assert!(probe.get("result").is_some(), "the 2026 probe is answered: {probe}");
    let r = init(&mut s);
    assert_eq!(r["result"]["protocolVersion"], json!("2025-11-25"));
    assert!(r["result"]["capabilities"].get("experimental").is_none(), "{r}");
}

// The tools a session gets, each declared as the JS server declares it
// (tests/contract.rs compares them with the JS server's own).
#[test]
fn the_tools_are_the_js_servers() {
    let (port, _heard) = xenonet(vec![]);
    let mut s = Server::start(port, false);
    init(&mut s);
    let tools = s.ask(2, "tools/list", json!({}));
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["whoami", "peers", "who_has", "send", "inbox", "claim", "release", "focus", "doctor", "ask_gateway_token"]);
    for t in tools {
        assert_eq!(t["inputSchema"]["type"], json!("object"));
        assert_eq!(t["inputSchema"]["$schema"], json!("https://json-schema.org/draft/2020-12/schema"));
        assert!(t["title"].is_string() && t["description"].as_str().unwrap().len() > 40 && t["annotations"]["readOnlyHint"].is_boolean(), "{t}");
    }
}
