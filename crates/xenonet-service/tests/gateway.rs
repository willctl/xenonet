// The actor gateway against the JS one's answers (tests/fixtures/gateway.json,
// made by gateway.mjs beside it): each request the JS gateway was sent, sent
// to the Rust gateway with the same stand-ins for its cores, and the answer
// compared byte for byte (status, the headers that matter, the body).

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use xenonet_service::cores::{self, R};
use xenonet_service::gateway::actor_gateway::{ActorGateway, GatewayCores, GatewayHooks, GatewayRequest};
use xenonet_service::http::ReplyBody;

const FIXTURE: &str = include_str!("fixtures/gateway.json");

struct Stand;

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
    fn can(&self, _id: &str, _capability: &str) -> bool {
        true
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

fn gateway(port: u16) -> ActorGateway {
    let s = Arc::new(Stand);
    let cores = GatewayCores { engine: s.clone(), registration: s.clone(), actors: s.clone(), reports: s.clone(), runs: Some(s) };
    ActorGateway::new(cores, GatewayHooks::default(), port, Some("https://mac.example.ts.net/"), "test")
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
}
