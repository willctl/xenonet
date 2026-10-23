// The actor gateway against the JS one's answers (tests/fixtures/gateway.json,
// made by gateway.mjs beside it): each request the JS gateway was sent, sent
// to the Rust gateway with the same stand-ins for its cores, and the answer
// compared byte for byte (status, the headers that matter, the body).

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use xenonet_service::cores::{self, R};
use xenonet_service::gateway::actor_gateway::{ActorGateway, GatewayCores, GatewayHooks, GatewayRequest};
use xenonet_service::http::ReplyBody;

const FIXTURE: &str = include_str!("fixtures/gateway.json");

/// The gateway's cores; `message` is whether you let this actor message
/// agents.
struct Stand {
    message: bool,
}

#[async_trait]
impl cores::Engine for Stand {
    async fn whoami(&self, _me: &str) -> R<Value> {
        Ok(json!({ "you": "actor:muse" }))
    }
    async fn peers(&self, _me: &str, scope: &str) -> R<Value> {
        Ok(json!([{ "id": "claude:x", "scope": scope }]))
    }
    async fn send(&self, _me: &str, msg: Value, extra: Option<Value>) -> R<Value> {
        let mut out = json!({ "id": 7 });
        if let Some(to) = msg.get("to") {
            out["to"] = to.clone();
        }
        out["method"] = extra.unwrap()["envelope"]["auth"]["method"].clone();
        Ok(out)
    }
    async fn inbox(&self, _me: &str, wait: Duration) -> R<Value> {
        Ok(json!({ "messages": [], "waitMs": wait.as_millis() as u64 }))
    }
}

impl cores::Registration for Stand {
    fn authenticate_gateway(&self, token: &str) -> Value {
        if token == format!("xenonet_gateway_{}", "a".repeat(64)) {
            json!({ "ok": true, "id": "actor:muse", "slug": "muse", "state": "allowed" })
        } else {
            json!({ "ok": false, "status": 401, "code": "bad-token", "error": "XenoNet doesn't know that token" })
        }
    }
    fn me(&self, _auth: &Value, _extra: Option<Value>) -> R<Value> {
        Ok(json!({ "schema": "actor/me/v1", "id": "actor:muse", "unread": 2 }))
    }
}

impl cores::Actors for Stand {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Muse", "product": { "id": "muse" } }))
    }
    fn can(&self, _id: &str, capability: &str) -> bool {
        capability != "message-agents" || self.message
    }
}

impl cores::Reports for Stand {
    fn take_heartbeat(&self, _id: &str, _body: &Value) -> R<()> {
        Ok(())
    }
}

impl cores::Runs for Stand {
    fn report(&self, _actor: &str, run: Value) -> Value {
        json!({ "ok": true, "id": format!("run:{}", run["run"].as_str().unwrap_or("")) })
    }
}

fn cores(message: bool) -> GatewayCores {
    let s = Arc::new(Stand { message });
    GatewayCores { engine: s.clone(), registration: s.clone(), actors: s.clone(), reports: s.clone(), runs: Some(s) }
}

fn gateway(port: u16) -> ActorGateway {
    ActorGateway::new(cores(true), GatewayHooks::default(), port, Some("https://mac.example.ts.net/"), "test")
}

/// A tool called through the gateway with Muse's token: the text it
/// answered, and whether that's an error.
async fn tool(g: &ActorGateway, name: &str, arguments: Value) -> (bool, String) {
    let body = json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": name, "arguments": arguments } });
    let headers = [
        ("host", "mac.example.ts.net".to_string()),
        ("content-type", "application/json".into()),
        ("accept", "application/json, text/event-stream".into()),
        ("authorization", format!("Bearer xenonet_gateway_{}", "a".repeat(64))),
    ];
    let req = GatewayRequest {
        method: "POST".into(),
        target: "/mcp".into(),
        headers: headers.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        body: body.to_string().into_bytes(),
    };
    let r = g.serve(req, false).await;
    // A tool's answer is JSON, never a stream.
    let b = match &r.body {
        ReplyBody::Full(b) => b.to_vec(),
        ReplyBody::Stream(_) => Vec::new(),
    };
    let answer: Value = serde_json::from_slice(&b).unwrap();
    let result = &answer["result"];
    (result["isError"] == json!(true), result["content"][0]["text"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn an_actor_sends_only_with_your_say_so_and_within_its_limits() {
    let at = Arc::new(std::sync::Mutex::new(1_000_000.0));
    let clock = {
        let at = at.clone();
        Arc::new(move || *at.lock().unwrap())
    };
    let host = Some("https://mac.example.ts.net/");
    // Without the capability you allow, nothing is sent.
    let held = ActorGateway::with_clock(cores(false), GatewayHooks::default(), 47997, host, "test", clock.clone());
    let refused = (true, "send needs the message-agents capability, which your user didn't allow this actor".to_string());
    assert_eq!(tool(&held, "send", json!({ "to": "claude:x", "text": "hi" })).await, refused);
    assert!(!tool(&held, "peers", json!({})).await.0, "reading isn't sending");
    // Six an hour to a repo or to all, sixty in all.
    let g = ActorGateway::with_clock(cores(true), GatewayHooks::default(), 47996, host, "test", clock.clone());
    for to in ["repo", "all", "project", "repo", "all", "repo"] {
        assert!(!tool(&g, "send", json!({ "to": to, "text": "hi" })).await.0, "{to}");
    }
    assert_eq!(tool(&g, "send", json!({ "to": "all", "text": "hi" })).await, (true, "an actor sends at most 6 messages an hour to a repo or to all; try again in 600 s".into()));
    for i in 0..54 {
        assert!(!tool(&g, "send", json!({ "to": "claude:x", "text": "hi" })).await.0, "{i}");
    }
    assert_eq!(tool(&g, "send", json!({ "to": "claude:x", "text": "hi" })).await, (true, "an actor sends at most 60 messages an hour; try again in 60 s".into()));
    *at.lock().unwrap() += 60e3;
    assert!(!tool(&g, "send", json!({ "to": "claude:x", "text": "hi" })).await.0, "a minute on, one more");
    // Heartbeats and run reports, a second apart at most, after a burst.
    for _ in 0..4 {
        assert!(!tool(&g, "heartbeat", json!({ "presence": "working" })).await.0);
    }
    assert_eq!(tool(&g, "heartbeat", json!({})).await, (true, "one heartbeat a second is the most the gateway takes".into()));
    for i in 0..32 {
        assert!(!tool(&g, "report_run", json!({ "run": format!("r{i}"), "app": "claude" })).await.0, "{i}");
    }
    assert_eq!(tool(&g, "report_run", json!({ "run": "r32", "app": "claude" })).await, (true, "too many run reports: one a second is the most the gateway takes".into()));
    *at.lock().unwrap() += 1000.0;
    assert!(!tool(&g, "heartbeat", json!({})).await.0 && !tool(&g, "report_run", json!({ "run": "r33", "app": "claude" })).await.0);
}

#[tokio::test]
async fn the_gateway_answers_as_the_js_one_does() {
    let port = 47999;
    let g = gateway(port);
    let fixture: Value = serde_json::from_str(&FIXTURE.replace("127.0.0.1:PORT", &format!("127.0.0.1:{port}"))).unwrap();
    let mut differ = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let r = &case["request"];
        let headers: Vec<(String, String)> = r["headers"].as_object().unwrap().iter().map(|(k, v)| (k.to_lowercase(), v.as_str().unwrap().to_string())).collect();
        let body = r["body"].as_str().unwrap_or("").as_bytes().to_vec();
        let req = GatewayRequest { method: r["method"].as_str().unwrap().into(), target: r["path"].as_str().unwrap().into(), headers, body };
        let got = g.serve(req, false).await;
        let want = &case["answer"];
        let text = match &got.body {
            ReplyBody::Full(b) => String::from_utf8_lossy(b).into_owned(),
            ReplyBody::Stream(_) => "(a stream)".into(),
        };
        let mut got_headers = serde_json::Map::new();
        for k in ["content-type", "cache-control", "www-authenticate", "allow", "retry-after", "x-xenonet-probe"] {
            if let Some(v) = got.header(k) {
                got_headers.insert(k.into(), json!(v));
            }
        }
        let (ws, wh, wb) = (want["status"].as_u64().unwrap() as u16, &want["headers"], want["body"].as_str().unwrap());
        if got.status != ws || &Value::Object(got_headers.clone()) != wh || text != wb {
            differ.push(format!("{name}:\n  js:   {ws} {wh} {wb}\n  rust: {} {} {text}", got.status, Value::Object(got_headers)));
        }
    }
    assert!(differ.is_empty(), "{} of {} differ:\n{}", differ.len(), fixture["cases"].as_array().unwrap().len(), differ.join("\n"));
}

#[tokio::test]
async fn a_probe_xenonet_asked_for_is_answered_once_and_bad_tokens_are_held_off() {
    let g = gateway(47998);
    let probe = |word: &str| GatewayRequest {
        method: "GET".into(),
        target: "/mcp".into(),
        headers: vec![("host".into(), "mac.example.ts.net".into()), ("x-xenonet-probe".into(), word.into())],
        body: vec![],
    };
    g.expect_probe("w1");
    let r = g.serve(probe("w1"), false).await;
    assert_eq!((r.status, r.header("x-xenonet-probe")), (204, Some("w1")));
    assert_eq!(g.serve(probe("w1"), false).await.status, 401, "once");
    let bad = || GatewayRequest {
        method: "POST".into(),
        target: "/mcp".into(),
        headers: vec![("host".into(), "mac.example.ts.net".into()), ("authorization".into(), format!("Bearer xenonet_gateway_{}", "b".repeat(64)))],
        body: b"{}".to_vec(),
    };
    for _ in 0..20 {
        assert_eq!(g.serve(bad(), false).await.status, 401);
    }
    let r = g.serve(bad(), false).await;
    assert_eq!(r.status, 401, "the 21st still answers, and starts the minute");
    let r = g.serve(bad(), false).await;
    assert_eq!((r.status, r.header("retry-after").is_some()), (429, true));
    assert_eq!(g.status()["refused"], json!(22), "the probe's, the 21 tokens; a call held off is refused before its token is read");
    assert_eq!(g.serve(bad(), true).await.status, 429);
    // Held off is the source they came from, as the tunnel says it: not
    // everyone.
    let mut elsewhere = bad();
    elsewhere.headers.push(("x-forwarded-for".into(), "198.51.100.5".into()));
    assert_eq!(g.serve(elsewhere, false).await.status, 401);
}

#[tokio::test]
async fn turning_the_gateway_off_closes_the_connections_it_kept_alive() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let g = gateway(port);
    g.start().await.unwrap();
    let mut c = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let ask = format!("GET /mcp HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\n\r\n");
    c.write_all(ask.as_bytes()).await.unwrap();
    let mut buf = [0u8; 4096];
    let n = c.read(&mut buf).await.unwrap();
    assert!(buf[..n].starts_with(b"HTTP/1.1 "), "answered while on");
    g.stop();
    // Asked again on the connection a client kept alive, as fetch does:
    // closed, and not answered.
    let _ = c.write_all(ask.as_bytes()).await;
    let mut after = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(10), c.read_to_end(&mut after)).await;
    assert!(read.is_ok(), "the connection is closed, not left open");
    assert!(!String::from_utf8_lossy(&after).contains("HTTP/1.1 "), "not answered once off");
}
