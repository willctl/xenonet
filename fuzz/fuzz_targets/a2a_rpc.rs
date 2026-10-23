#![no_main]

// A2A's JSON-RPC binding at the gateway's /a2a (docs/design/network.md,
// section 12), as an A2A client on the internet reaches it once the front
// door checked its token: requests through `A2a::handle`, with a task
// table in memory and a made-up engine whose inbox holds the input's
// messages. Each run makes a new A2a and sends it up to four requests: the
// first byte of each picks a method (or the input's own body), and the
// task ids it answered can come back in the next ones. Then the Agent
// Card for an address, and the actor the front door's check names.
//
// Nothing may panic. Every JSON answer is JSON, every 200 a JSON-RPC 2.0
// answer, and every event of a stream one SSE event: a comment, or a
// `data:` line of JSON-RPC with no line break in it, then a blank line.

use async_trait::async_trait;
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_fuzz::net::Choices;
use xenonet_service::a2a::{card, A2a, A2aActor, A2aCores, A2aParts, A2aRequest, Limits};
use xenonet_service::cores::{Actors, CoreError, Engine, R};
use xenonet_service::http::ReplyBody;

const METHODS: [&str; 13] = [
    "SendMessage",
    "SendStreamingMessage",
    "GetTask",
    "ListTasks",
    "CancelTask",
    "SubscribeToTask",
    "CreateTaskPushNotificationConfig",
    "GetTaskPushNotificationConfig",
    "ListTaskPushNotificationConfigs",
    "DeleteTaskPushNotificationConfig",
    "GetExtendedAgentCard",
    "message/send",
    "",
];

/// The engine and actors, made up: three sessions reach, the inbox holds
/// the input's messages until they're read.
struct Fake {
    inbox: Mutex<Vec<Value>>,
    next: Mutex<i64>,
}

#[async_trait]
impl Engine for Fake {
    async fn send(&self, _me: &str, msg: Value, _extra: Option<Value>) -> R<Value> {
        let to = msg["to"].as_str().unwrap_or("").to_string();
        let recipients: Vec<&str> = match to.as_str() {
            one @ ("claude:x" | "claude:a" | "claude:b") => vec![one],
            "project:atlas" => vec!["claude:a", "claude:b"],
            "pending" => return Err(CoreError::pending("core/engine/engine.js")),
            other => return Err(CoreError::new(format!("no session or actor you can reach is {other}"))),
        };
        let id = {
            let mut n = self.next.lock().map_err(|_| CoreError::new("poisoned"))?;
            *n += 1;
            100 + *n
        };
        Ok(json!({ "id": id, "to": recipients.iter().map(|r| json!({ "id": r, "title": "t" })).collect::<Vec<_>>() }))
    }

    async fn inbox(&self, _me: &str, _wait: Duration) -> R<Value> {
        Ok(Value::Array(self.inbox.lock().map(|mut i| std::mem::take(&mut *i)).unwrap_or_default()))
    }
}

impl Actors for Fake {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Wren", "product": null }))
    }
    fn can(&self, _id: &str, _capability: &str) -> bool {
        true
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime"))
}

fn a2a(inbox: Vec<Value>) -> Arc<A2a> {
    let fake = Arc::new(Fake { inbox: Mutex::new(inbox), next: Mutex::new(0) });
    let a = A2a::new(A2aParts {
        cores: A2aCores { engine: fake.clone(), actors: fake },
        file: None,
        version: "0.2.0".into(),
        home: "/home/nova".into(),
        name: Arc::new(|| Some("home".into())),
        now: Arc::new(|| 1_792_512_345_678.0),
        limits: Limits {
            block: Duration::from_millis(2),
            stream: Duration::from_millis(15),
            keepalive: Duration::from_millis(3),
            pull: Duration::from_millis(1),
            backoff: Duration::from_millis(1),
        },
    })
    .expect("A2A in memory");
    a.set_enabled(true).expect("A2A on");
    a
}

/// A JSON-RPC answer, as a client reads it.
fn is_rpc(v: &Value) -> bool {
    v["jsonrpc"] == "2.0" && (v.get("result").is_some() != v.get("error").is_some())
}

/// The answer's body, checked: a JSON answer's JSON, or each event of a
/// stream. The task ids it names.
fn read(reply: xenonet_service::http::Reply) -> Vec<String> {
    let mut ids = Vec::new();
    let mut note = |v: &Value| {
        for t in [&v["result"]["task"]["id"], &v["result"]["id"], &v["result"]["taskId"], &v["result"]["statusUpdate"]["taskId"]] {
            if let Some(id) = t.as_str() {
                ids.push(id.to_string());
            }
        }
    };
    match reply.body {
        ReplyBody::Full(b) => {
            let v: Value = serde_json::from_slice(&b).unwrap_or_else(|e| panic!("an answer that isn't JSON ({e}): {}", String::from_utf8_lossy(&b)));
            if reply.status == 200 {
                assert!(is_rpc(&v), "a 200 that isn't a JSON-RPC answer: {v}");
            }
            note(&v);
        }
        ReplyBody::Stream(mut rx) => {
            assert!(reply.headers.iter().any(|(k, v)| k == "content-type" && v == "text/event-stream"));
            let mut buf = Vec::new();
            runtime().block_on(async {
                let end = tokio::time::Instant::now() + Duration::from_millis(60);
                while let Ok(Some(chunk)) = tokio::time::timeout_at(end, rx.recv()).await {
                    buf.extend_from_slice(&chunk);
                }
            });
            let text = String::from_utf8(buf).expect("a stream that isn't UTF-8");
            for event in text.split_terminator("\n\n") {
                if event.starts_with(':') {
                    assert!(!event.contains('\n'), "a comment over lines: {event:?}");
                    continue;
                }
                let data = event.strip_prefix("data: ").unwrap_or_else(|| panic!("an event that isn't data: {event:?}"));
                assert!(!data.contains(['\n', '\r']), "an event's data breaks its line: {data:?}");
                let v: Value = serde_json::from_str(data).unwrap_or_else(|e| panic!("an event that isn't JSON ({e}): {data:?}"));
                assert!(is_rpc(&v), "an event that isn't a JSON-RPC answer: {v}");
                note(&v);
            }
            assert!(text.is_empty() || text.ends_with("\n\n"), "a stream that ends mid-event: {text:?}");
        }
    }
    ids
}

/// The params of a request for `method`, built so it reaches the method's
/// own checks, with the input's fields over them.
fn params(method: &str, ids: &[String], c: &mut Choices<'_>) -> Value {
    let task = ids.get(c.below(ids.len().max(1))).cloned().map_or(Value::Null, Value::String);
    let base = match method {
        "SendMessage" | "SendStreamingMessage" => {
            let text = String::from_utf8_lossy(c.chunk()).into_owned();
            let target = *c.pick(&["claude:x", "repo:atlas", "project:atlas", "all", "pending", "nobody", ""]);
            let mut m = json!({ "messageId": format!("m{}", c.u8()), "role": "ROLE_USER", "parts": [{ "text": if text.is_empty() { "hi".into() } else { text } }] });
            if !target.is_empty() {
                m["metadata"] = json!({ "target": target });
            }
            if c.bool() {
                m["contextId"] = json!(format!("ctx{}", c.u8() % 4));
            }
            let mut p = json!({ "message": m });
            if c.bool() {
                p["configuration"] = json!({ "returnImmediately": c.bool(), "historyLength": c.u8() % 4, "acceptedOutputModes": ["text/plain"] });
            }
            p
        }
        "ListTasks" => json!({ "pageSize": c.u8() % 4 + 1, "includeArtifacts": c.bool(), "historyLength": c.u8() % 3 }),
        _ => json!({ "id": task, "historyLength": c.u8() % 3 }),
    };
    xenonet_fuzz::net::overlay(base, c.value(), c)
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    // Without the input's own, the inbox holds a reply to the first message
    // sent and one to none.
    let inbox: Vec<Value> = match c.value() {
        Value::Array(l) => l,
        Value::Null => vec![
            json!({ "id": 500, "from": { "id": "claude:x", "title": "Fix login" }, "text": "On fix/login.", "at": 5.0, "replyTo": 101, "envelope": { "mid": "msg_r500" } }),
            json!({ "id": 501, "from": "claude:a", "text": "Also this.", "replyTo": null }),
        ],
        one => vec![one],
    };
    let a = a2a(inbox);
    let actor = A2aActor { id: "actor:wren".into(), slug: "wren".into() };
    let mut ids: Vec<String> = Vec::new();
    for _ in 0..=c.below(4) {
        let pick = c.u8();
        let body = if pick > 231 {
            c.chunk().to_vec()
        } else {
            let method = METHODS[usize::from(pick) % METHODS.len()];
            let id = match c.below(4) {
                0 => json!(c.u8()),
                1 => json!(format!("r{}", c.u8())),
                2 => Value::Null,
                _ => c.value(),
            };
            let mut req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params(method, &ids, &mut c) });
            if c.rarely(16) {
                req = xenonet_fuzz::net::overlay(req, c.value(), &mut c);
            }
            req.to_string().into_bytes()
        };
        let mut headers = vec![("content-type".to_string(), (*c.pick(&["application/json", "application/json; charset=utf-8", "text/plain", "application/a2a+json"])).to_string())];
        match c.below(4) {
            0 => headers.push(("a2a-version".into(), "1.0".into())),
            1 => headers.push(("A2A-Version".into(), String::from_utf8_lossy(c.chunk()).into_owned())),
            2 => headers.push(("a2a-version".into(), (*c.pick(&["1.0.3", "0.3", "1", "", " 1.0 "])).to_string())),
            _ => {}
        }
        let target = if c.rarely(32) { format!("/a2a?{}", String::from_utf8_lossy(c.chunk())) } else { "/a2a".into() };
        let method = if c.rarely(8) { "GET" } else { "POST" };
        let req = A2aRequest { method: method.into(), target, headers, body };
        let a2 = a.clone();
        let who = actor.clone();
        let reply = runtime().block_on(async move { tokio::time::timeout(Duration::from_millis(80), a2.handle(&who, req)).await.ok() });
        if let Some(reply) = reply {
            ids.extend(read(reply));
        }
        if c.is_empty() {
            break;
        }
    }
    // The card for the address a Host header gives, and the actor a token
    // check names.
    let host = String::from_utf8_lossy(c.chunk()).into_owned();
    if let Some(v) = a.card(&format!("http://{host}")) {
        let _ = serde_json::to_string(&v).expect("the card is JSON");
    }
    let _ = (card::host_of(&host), card::origin(&host));
    let _ = A2aActor::from_auth(&c.value());
});
