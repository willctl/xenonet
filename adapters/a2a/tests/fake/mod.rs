//! A fake A2A 1.0 agent, for tests alone: a TLS server on loopback (std
//! threads and rustls's own stream) with a test CA and its key made for
//! these tests (../fixtures). It serves a card, answers
//! SendStreamingMessage with each state a task can reach by the words it's
//! sent ("hello", "more", "sign in", "fail", "drop"), drops one stream half
//! way and answers SubscribeToTask after, and echoes the bearer in its
//! errors. The service's tests use it too.

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use xenonet_a2a::card::{self, Pinned};
use xenonet_a2a::{Agent, Bearer, Private};

pub const TOKEN: &str = "tok-remote-0123456789abcdef";

#[derive(Default)]
pub struct Seen {
    pub auth: Vec<String>,
    pub methods: Vec<String>,
    pub bodies: Vec<String>,
    /// The card's version: change it and the card's pin no longer holds.
    pub version: String,
    /// The interface's host:port, when it isn't the card's.
    pub interface: Option<String>,
}

pub struct Fake {
    pub port: u16,
    pub seen: Arc<Mutex<Seen>>,
}

pub fn tls() -> Arc<rustls::ServerConfig> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(include_bytes!("../fixtures/localhost.pem")).map(Result::unwrap).collect();
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("../fixtures/localhost.key")).unwrap();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider).with_safe_default_protocol_versions().unwrap().with_no_client_auth().with_single_cert(certs, key).unwrap();
    Arc::new(config)
}

pub fn roots() -> Vec<reqwest::Certificate> {
    vec![reqwest::Certificate::from_pem(include_bytes!("../fixtures/ca.pem")).unwrap()]
}

type Stream = rustls::StreamOwned<rustls::ServerConnection, TcpStream>;

/// One request: its method, path, headers (names lowercased) and body.
/// A request as read: method, path, headers and body.
type Request = (String, String, Vec<(String, String)>, Vec<u8>);

fn read_request(s: &mut Stream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if s.read(&mut byte).ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
    let len = headers.iter().find(|(k, _)| k == "content-length").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    Some((method, path, headers, body))
}

fn answer(s: &mut Stream, status: &str, headers: &[(&str, String)], body: &[u8]) {
    let mut out = format!("HTTP/1.1 {status}\r\nconnection: close\r\ncontent-length: {}\r\n", body.len());
    for (k, v) in headers {
        let _ = write!(out, "{k}: {v}\r\n");
    }
    out.push_str("\r\n");
    let _ = s.write_all(out.as_bytes());
    let _ = s.write_all(body);
    let _ = s.flush();
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn task(state: &str, text: Option<&str>) -> Value {
    let mut status = json!({ "state": state });
    if let Some(t) = text {
        status["message"] = json!({ "messageId": format!("m-{state}"), "role": "ROLE_AGENT", "parts": [{ "text": t }] });
    }
    json!({ "task": { "id": "task-1", "contextId": "ctx-1", "status": status } })
}

fn status(state: &str, text: Option<&str>) -> Value {
    let mut status = json!({ "state": state });
    if let Some(t) = text {
        status["message"] = json!({ "messageId": format!("s-{state}"), "role": "ROLE_AGENT", "parts": [{ "text": t }] });
    }
    json!({ "statusUpdate": { "taskId": "task-1", "contextId": "ctx-1", "status": status } })
}

/// An event stream of `events`, then the connection closed (cut short
/// when `drop` says so, with no close).
fn stream(s: &mut Stream, id: &Value, events: &[Value]) {
    let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-store\r\nconnection: close\r\n\r\n");
    for e in events {
        let _ = s.write_all(format!("data: {}\n\n", rpc_result(id, e.clone())).as_bytes());
        let _ = s.flush();
    }
}

fn card(port: u16, seen: &Seen) -> Value {
    let host = seen.interface.clone().unwrap_or_else(|| format!("localhost:{port}"));
    json!({
        "name": "Scout",
        "description": "Finds things out.",
        "version": seen.version,
        "supportedInterfaces": [{ "url": format!("https://{host}/a2a"), "protocolBinding": "JSONRPC", "protocolVersion": "1.0" }],
        "capabilities": { "streaming": true, "pushNotifications": false },
        "securitySchemes": { "bearer": { "httpAuthSecurityScheme": { "scheme": "Bearer" } } },
        "securityRequirements": [{ "schemes": { "bearer": { "list": [] } } }],
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{ "id": "find", "name": "Find", "description": "Finds things out.", "tags": ["search"] }]
    })
}

fn serve(port: u16, seen: &Arc<Mutex<Seen>>, mut s: Stream) {
    let Some((method, path, headers, body)) = read_request(&mut s) else { return };
    let json_type = ("content-type", "application/json".to_string());
    match (method.as_str(), path.as_str()) {
        ("GET", "/.well-known/agent-card.json") => {
            let c = card(port, &seen.lock().unwrap());
            answer(&mut s, "200 OK", &[json_type], c.to_string().as_bytes());
        }
        ("GET", "/elsewhere") => answer(&mut s, "302 Found", &[("location", "https://agents.example.com/.well-known/agent-card.json".into())], b""),
        ("GET", "/here") => answer(&mut s, "302 Found", &[("location", "/.well-known/agent-card.json".into())], b""),
        ("POST", "/a2a") => {
            let auth = headers.iter().find(|(k, _)| k == "authorization").map(|(_, v)| v.clone()).unwrap_or_default();
            let req: Value = serde_json::from_slice(&body).unwrap();
            {
                let mut g = seen.lock().unwrap();
                g.auth.push(auth.clone());
                g.methods.push(req["method"].as_str().unwrap_or("").to_string());
                g.bodies.push(String::from_utf8_lossy(&body).into_owned());
            }
            let id = req["id"].clone();
            if auth != format!("Bearer {TOKEN}") {
                return answer(&mut s, "401 Unauthorized", &[json_type], b"{}");
            }
            let text = req["params"]["message"]["parts"][0]["text"].as_str().unwrap_or("").to_string();
            match req["method"].as_str().unwrap_or("") {
                // More for a task that asked for it: it completes.
                "SendStreamingMessage" if req["params"]["message"]["taskId"] == "task-1" => {
                    stream(&mut s, &id, &[status("TASK_STATE_WORKING", None), status("TASK_STATE_COMPLETED", Some(&format!("went on with {text}")))]);
                }
                "SendStreamingMessage" => match text.as_str() {
                    "hello" => stream(
                        &mut s,
                        &id,
                        &[
                            task("TASK_STATE_SUBMITTED", None),
                            status("TASK_STATE_WORKING", Some("reading")),
                            json!({ "artifactUpdate": { "taskId": "task-1", "contextId": "ctx-1", "artifact": { "artifactId": "a1", "name": "answer", "parts": [{ "text": "the answer" }] }, "lastChunk": true } }),
                            status("TASK_STATE_COMPLETED", None),
                        ],
                    ),
                    "more" => stream(&mut s, &id, &[task("TASK_STATE_SUBMITTED", None), status("TASK_STATE_INPUT_REQUIRED", Some("Which branch?"))]),
                    "sign in" => stream(&mut s, &id, &[task("TASK_STATE_AUTH_REQUIRED", Some("Sign in at https://agents.example.com/login"))]),
                    "fail" => stream(&mut s, &id, &[task("TASK_STATE_SUBMITTED", None), status("TASK_STATE_FAILED", Some(&format!("your token {TOKEN} can't do that")))]),
                    // Working, then the connection goes with no end.
                    "drop" => stream(&mut s, &id, &[task("TASK_STATE_WORKING", None)]),
                    _ => {
                        let e = json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": format!("bad request from {auth}") } });
                        answer(&mut s, "200 OK", &[json_type], e.to_string().as_bytes());
                    }
                },
                "SubscribeToTask" => stream(&mut s, &id, &[status("TASK_STATE_COMPLETED", Some("picked up"))]),
                "CancelTask" => {
                    let t = task("TASK_STATE_CANCELED", None)["task"].clone();
                    answer(&mut s, "200 OK", &[json_type], rpc_result(&id, t).to_string().as_bytes());
                }
                _ => answer(&mut s, "404 Not Found", &[], b""),
            }
        }
        _ => answer(&mut s, "404 Not Found", &[], b""),
    }
    s.conn.send_close_notify();
    let _ = s.flush();
}

pub fn fake() -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen { version: "1.0.0".into(), ..Seen::default() }));
    let config = tls();
    let s2 = seen.clone();
    std::thread::spawn(move || {
        for tcp in listener.incoming() {
            let Ok(tcp) = tcp else { continue };
            let (config, seen) = (config.clone(), s2.clone());
            std::thread::spawn(move || {
                let conn = rustls::ServerConnection::new(config).unwrap();
                serve(port, &seen, rustls::StreamOwned::new(conn, tcp));
            });
        }
    });
    Fake { port, seen }
}

impl Fake {
    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.port)
    }

    pub fn allowed(&self) -> Private {
        Private { host: Some(format!("localhost:{}", self.port)) }
    }

    pub async fn agent(&self) -> Agent {
        let c = card::fetch(&self.url("/"), &self.allowed(), Some(&roots())).await.unwrap();
        Agent::new(Pinned::of(&c, self.allowed()), Bearer::new(TOKEN.into())).trusting(roots())
    }
}
