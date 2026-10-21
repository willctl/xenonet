// A2A through the gateway's real front door (gateway/front_door.rs): the
// Agent Card, which anyone may read, and /a2a, which takes a gateway token
// checked as /mcp checks one, in that order. Requests go over TCP to the
// actor gateway's listener on 127.0.0.1, with made-up cores behind it: one
// session in the actor's scope, claude:x, which answers every message.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use xenonet_service::a2a::{A2a, A2aCores, A2aParts, Limits};
use xenonet_service::cores::{self, R};
use xenonet_service::gateway::actor_gateway::{ActorGateway, GatewayCores, GatewayHooks};

const PUBLIC: &str = "mac.example.ts.net";

fn token() -> String {
    format!("xenonet_gateway_{}", "b".repeat(64))
}

/// claude:x, the one session Wren's scope reaches, which answers at once.
#[derive(Default)]
struct Stand {
    inbox: Mutex<Vec<Value>>,
    next: Mutex<i64>,
    arrived: Notify,
}

#[async_trait]
impl cores::Engine for Stand {
    async fn send(&self, _me: &str, msg: Value, _extra: Option<Value>) -> R<Value> {
        if msg["to"] != "claude:x" {
            return Err(cores::CoreError::new(format!("no session or actor you can reach is {}", msg["to"].as_str().unwrap_or(""))));
        }
        let id = {
            let mut n = self.next.lock().unwrap();
            *n += 2;
            100 + *n
        };
        self.inbox.lock().unwrap().push(json!({
            "id": id + 1, "from": { "id": "claude:x", "title": "Docs", "app": "claude" }, "to": "actor:wren",
            "text": "The docs build is green.", "at": 1.0, "replyTo": id, "envelope": { "mid": format!("msg_reply-{}", id + 1) }
        }));
        self.arrived.notify_waiters();
        Ok(json!({ "id": id, "to": [{ "id": "claude:x", "title": "Docs", "delivery": "during its current turn" }] }))
    }

    async fn inbox(&self, _me: &str, wait: Duration) -> R<Value> {
        if self.inbox.lock().unwrap().is_empty() && wait > Duration::ZERO {
            let _ = tokio::time::timeout(wait, self.arrived.notified()).await;
        }
        Ok(Value::Array(std::mem::take(&mut *self.inbox.lock().unwrap())))
    }
}

impl cores::Registration for Stand {
    fn authenticate_gateway(&self, t: &str) -> Value {
        if t == token() {
            json!({ "ok": true, "id": "actor:wren", "slug": "wren", "state": "allowed" })
        } else {
            json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that token" })
        }
    }
}

impl cores::Actors for Stand {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Wren" }))
    }
    fn can(&self, _id: &str, _capability: &str) -> bool {
        true
    }
}

#[async_trait]
impl cores::Reports for Stand {}

/// One HTTP/1.1 request by hand: the status, the headers (names
/// lowercased) and the body.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, Vec<(String, String)>, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        let _ = write!(text, "{k}: {v}\r\n");
    }
    let _ = write!(text, "Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(text.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    let (head, rest) = out.split_once("\r\n\r\n").unwrap_or((&out, ""));
    let mut lines = head.lines();
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|c| c.parse().ok()).unwrap_or(0);
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(": ")).map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect();
    (status, headers, rest.to_string())
}

fn header<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
    h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

async fn call(port: u16, method: &'static str, path: String, headers: Vec<(String, String)>, body: String) -> (u16, Vec<(String, String)>, String) {
    tokio::task::spawn_blocking(move || {
        let h: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        http(port, method, &path, &h, &body)
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_card_and_a2a_through_the_front_door_in_mcps_order() {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let stand = Arc::new(Stand::default());
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let heard = calls.clone();
    let hooks =
        GatewayHooks { on_call: Arc::new(move |id: &str, _slug: &str, _host: Option<&str>, _at: f64| heard.lock().unwrap().push(id.to_string())), ..GatewayHooks::default() };
    let g = ActorGateway::new(
        GatewayCores { engine: stand.clone(), registration: stand.clone(), actors: stand.clone(), reports: stand.clone(), runs: None },
        hooks,
        port,
        Some(PUBLIC),
        "test",
    );
    let a2a = A2a::new(A2aParts {
        cores: A2aCores { engine: stand.clone(), actors: stand.clone() },
        file: None,
        version: "0.0.0-test".into(),
        home: "/home/nova".into(),
        name: Arc::new(|| Some("home".into())),
        now: Arc::new(|| 1_792_512_345_678.0),
        limits: Limits { block: Duration::from_secs(5), ..Limits::default() },
    })
    .unwrap();
    g.set_a2a(a2a.clone());
    g.start().await.unwrap();
    let here = format!("127.0.0.1:{port}");
    let h = |extra: &[(&str, &str)]| {
        let mut v = vec![("Host".to_string(), here.clone())];
        v.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        v
    };
    let bearer = format!("Bearer {}", token());
    let send = |id: u64, target: &str| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "SendMessage", "params": { "message": { "role": "ROLE_USER", "parts": [{ "text": "Is the docs build green?" }], "messageId": format!("m-{id}"), "metadata": { "target": target } } } }).to_string()
    };
    let rpc = [("A2A-Version", "1.0"), ("Content-Type", "application/json"), ("Authorization", bearer.as_str())];

    // Off: the card isn't there; /a2a takes a token first, so only its
    // holder learns that A2A is off.
    assert_eq!(call(port, "GET", "/.well-known/agent-card.json".into(), h(&[]), String::new()).await.0, 404);
    assert_eq!(call(port, "POST", "/a2a".into(), h(&[("Authorization", "Bearer xenonet_gateway_nope")]), "{}".into()).await.0, 401);
    assert_eq!(call(port, "POST", "/a2a".into(), h(&[]), "{}".into()).await.0, 401);
    assert_eq!(call(port, "POST", "/a2a".into(), h(&rpc), send(1, "claude:x")).await.0, 404);
    assert!(calls.lock().unwrap().is_empty());

    a2a.set_enabled(true).unwrap();
    // The card: public, at the address it was read at; with /mcp's checks
    // of the host and web pages.
    let (s, _, body) = call(port, "GET", "/.well-known/agent-card.json".into(), h(&[]), String::new()).await;
    assert_eq!(s, 200, "{body}");
    let card: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(card["supportedInterfaces"][0]["url"], format!("http://{here}/a2a"));
    assert_eq!(card["name"], "XenoNet on home");
    let (s, _, body) = call(port, "GET", "/.well-known/agent-card.json".into(), vec![("Host".into(), PUBLIC.into())], String::new()).await;
    assert_eq!(s, 200);
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["supportedInterfaces"][0]["url"], format!("https://{PUBLIC}/a2a"));
    assert_eq!(call(port, "GET", "/.well-known/agent-card.json".into(), vec![("Host".into(), "evil.example".into())], String::new()).await.0, 421);
    assert_eq!(call(port, "GET", "/.well-known/agent-card.json".into(), h(&[("Origin", "https://evil.example")]), String::new()).await.0, 403);
    assert_eq!(call(port, "POST", "/.well-known/agent-card.json".into(), h(&[]), String::new()).await.0, 405);

    // /a2a, refused as /mcp refuses: the host, a web page, a token in the
    // URL, no token, an unknown token.
    assert_eq!(call(port, "POST", "/a2a".into(), vec![("Host".into(), "evil.example".into())], send(2, "claude:x")).await.0, 421);
    assert_eq!(call(port, "POST", "/a2a".into(), h(&[("Origin", "https://evil.example"), ("Authorization", bearer.as_str())]), send(2, "claude:x")).await.0, 403);
    let in_url = format!("/a2a?token={}", token());
    assert_eq!(call(port, "POST", in_url, h(&rpc), send(2, "claude:x")).await.0, 400);
    let (s, hd, _) = call(port, "POST", "/a2a".into(), h(&[("Content-Type", "application/json")]), send(2, "claude:x")).await;
    assert_eq!((s, header(&hd, "www-authenticate")), (401, Some("Bearer realm=\"xenonet-gateway\"")));
    let (s, hd, _) = call(port, "POST", "/a2a".into(), h(&[("Authorization", "Bearer xenonet_gateway_nope")]), send(2, "claude:x")).await;
    assert_eq!((s, header(&hd, "www-authenticate")), (401, Some("Bearer realm=\"xenonet-gateway\", error=\"invalid_token\"")));
    // 64 KiB at most, once the token is good.
    let big = format!("{{\"pad\":\"{}\"}}", "x".repeat(70 << 10));
    assert_eq!(call(port, "POST", "/a2a".into(), h(&rpc), big).await.0, 413);
    assert!(calls.lock().unwrap().is_empty(), "nothing refused counts as a call");

    // A message to the session in scope is a task, completed by its reply.
    let (s, _, body) = call(port, "POST", "/a2a".into(), h(&rpc), send(3, "claude:x")).await;
    assert_eq!(s, 200, "{body}");
    let task = serde_json::from_str::<Value>(&body).unwrap()["result"]["task"].clone();
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{task}");
    assert_eq!(task["status"]["message"]["parts"][0]["text"], "The docs build is green.");
    assert_eq!(*calls.lock().unwrap(), ["actor:wren"]);
    // GetTask finds it, by the same actor.
    let get = json!({ "jsonrpc": "2.0", "id": 4, "method": "GetTask", "params": { "id": task["id"] } }).to_string();
    let (s, _, body) = call(port, "POST", "/a2a".into(), h(&rpc), get).await;
    assert_eq!(s, 200, "{body}");
    let got = serde_json::from_str::<Value>(&body).unwrap()["result"].clone();
    assert_eq!((got["id"].clone(), got["status"]["state"].clone()), (task["id"].clone(), json!("TASK_STATE_COMPLETED")));
    // One out of scope is refused by the engine, and the task says why.
    let (s, _, body) = call(port, "POST", "/a2a".into(), h(&rpc), send(5, "claude:elsewhere")).await;
    assert_eq!(s, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["result"]["task"]["status"]["state"], "TASK_STATE_REJECTED", "{v}");
    // A stream is served as one.
    let stream = json!({ "jsonrpc": "2.0", "id": 6, "method": "SendStreamingMessage", "params": { "message": { "role": "ROLE_USER", "parts": [{ "text": "And the tests?" }], "messageId": "m-6", "metadata": { "target": "claude:x" } } } }).to_string();
    let (s, hd, body) = call(port, "POST", "/a2a".into(), h(&rpc), stream).await;
    assert_eq!((s, header(&hd, "content-type").map(|c| c.starts_with("text/event-stream"))), (200, Some(true)), "{body}");
    assert!(body.contains("TASK_STATE_COMPLETED"), "{body}");

    // Unknown tokens hold the door shut to their source alone: twenty from
    // one address the tunnel forwarded for, and that one waits a minute.
    let from = |ip: &str| h(&[("Authorization", "Bearer xenonet_gateway_nope"), ("X-Forwarded-For", ip)]);
    for _ in 0..21 {
        assert_eq!(call(port, "POST", "/a2a".into(), from("198.51.100.4"), "{}".into()).await.0, 401);
    }
    assert_eq!(call(port, "POST", "/a2a".into(), from("198.51.100.4"), "{}".into()).await.0, 429);
    assert_eq!(call(port, "POST", "/a2a".into(), from("198.51.100.5"), "{}".into()).await.0, 401, "another source isn't held off");
    // A client's own X-Forwarded-For in front of the tunnel's is no way out.
    assert_eq!(call(port, "POST", "/a2a".into(), from("203.0.113.1, 198.51.100.4"), "{}".into()).await.0, 429);
    assert_eq!(call(port, "POST", "/a2a".into(), h(&rpc), send(8, "claude:x")).await.0, 200, "nor is this machine");

    // Turned off again: as if there were no /a2a.
    a2a.set_enabled(false).unwrap();
    assert_eq!(call(port, "POST", "/a2a".into(), h(&rpc), send(7, "claude:x")).await.0, 404);
    assert_eq!(call(port, "GET", "/.well-known/agent-card.json".into(), h(&[]), String::new()).await.0, 404);
    g.stop();
}
