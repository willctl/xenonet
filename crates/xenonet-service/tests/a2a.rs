// A2A's JSON-RPC binding against the golden answers in
// test/fixtures/a2a/cases.json: requests written from the specification's
// examples, sent in order to a made-up engine whose one session answers
// every message, and each answer compared with the one kept there.
//
// XENONET_A2A_BLESS=<file> writes the answers this build gives to <file>, to
// look over before they replace the kept ones.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use xenonet_service::a2a::{A2a, A2aActor, A2aCores, A2aParts, A2aRequest, Limits};
use xenonet_service::cores::{self, R};
use xenonet_service::http::ReplyBody;

const CASES: &str = include_str!("../../../test/fixtures/a2a/cases.json");
const NOW: f64 = 1_792_512_345_678.0;
const ANSWER: &str = "Today will be sunny with a high of 75°F";

/// claude:x, which answers every message at once.
#[derive(Default)]
struct Weather {
    inbox: Mutex<Vec<Value>>,
    next: Mutex<i64>,
    arrived: Notify,
}

#[async_trait]
impl cores::Engine for Weather {
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
            "id": id + 1, "from": { "id": "claude:x", "title": "Weather", "app": "claude" }, "to": "you",
            "text": ANSWER, "at": NOW, "replyTo": id, "envelope": { "mid": format!("msg_reply-{}", id + 1) }
        }));
        self.arrived.notify_waiters();
        Ok(json!({ "id": id, "to": [{ "id": "claude:x", "title": "Weather", "delivery": "during its current turn" }] }))
    }

    async fn inbox(&self, _me: &str, wait: Duration) -> R<Value> {
        if self.inbox.lock().unwrap().is_empty() && wait > Duration::ZERO {
            let _ = tokio::time::timeout(wait, self.arrived.notified()).await;
        }
        Ok(Value::Array(std::mem::take(&mut *self.inbox.lock().unwrap())))
    }
}

impl cores::Actors for Weather {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Wren" }))
    }
    fn can(&self, _id: &str, _capability: &str) -> bool {
        true
    }
}

/// The ids XenoNet made (msg_<uuid>, ctx_<uuid>), named in the order they
/// first show: $task, $task2…, $context, $context2….
#[derive(Default)]
struct Names {
    of: Vec<(String, String)>,
}

impl Names {
    fn learn(&mut self, v: &Value) {
        match v {
            Value::String(s) => {
                for (prefix, name) in [("msg_", "$task"), ("ctx_", "$context")] {
                    let mut rest = s.as_str();
                    while let Some(i) = rest.find(prefix) {
                        let id: String = rest[i..].chars().take(prefix.len() + 36).collect();
                        let uuid = &id[prefix.len()..];
                        if uuid.len() == 36 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && !self.of.iter().any(|(k, _)| *k == id) {
                            let n = self.of.iter().filter(|(_, v)| v.starts_with(name)).count();
                            self.of.push((id.clone(), if n == 0 { name.to_string() } else { format!("{name}{}", n + 1) }));
                        }
                        rest = &rest[i + prefix.len()..];
                    }
                }
            }
            Value::Array(l) => l.iter().for_each(|x| self.learn(x)),
            Value::Object(o) => o.values().for_each(|x| self.learn(x)),
            _ => {}
        }
    }

    /// `v` with each id put as its name (`named`), or each name as its id.
    fn swap(&self, v: &Value, named: bool) -> Value {
        match v {
            Value::String(s) => {
                let mut s = s.clone();
                // The longest first, so $task2 isn't taken for $task.
                let mut pairs = self.of.clone();
                pairs.sort_by_key(|(_, n)| std::cmp::Reverse(n.len()));
                for (id, name) in &pairs {
                    s = if named { s.replace(id.as_str(), name) } else { s.replace(name.as_str(), id) };
                }
                Value::String(s)
            }
            Value::Array(l) => Value::Array(l.iter().map(|x| self.swap(x, named)).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), self.swap(x, named))).collect::<Map<_, _>>()),
            other => other.clone(),
        }
    }
}

#[tokio::test]
async fn the_specs_examples_answered() {
    let engine = Arc::new(Weather::default());
    let a = A2a::new(A2aParts {
        cores: A2aCores { engine: engine.clone(), actors: engine },
        file: None,
        version: "0.2.0".into(),
        home: "/home/nova".into(),
        name: Arc::new(|| Some("studio".into())),
        now: Arc::new(|| NOW),
        limits: Limits { block: Duration::from_secs(3), stream: Duration::from_secs(5), ..Limits::default() },
    })
    .unwrap();
    a.set_enabled(true).unwrap();
    let mut kept: Value = serde_json::from_str(CASES).unwrap();
    let mut out = kept.clone();
    let mut problems = Vec::new();

    let card = a.card(kept["card"]["address"].as_str().unwrap()).unwrap();
    out["card"]["answer"] = card.clone();
    if kept["card"].get("answer") != Some(&card) {
        problems.push("the card".to_string());
    }

    let wren = A2aActor { id: "actor:wren".into(), slug: "wren".into() };
    let mut names = Names::default();
    let cases = kept["cases"].as_array_mut().unwrap();
    for (i, c) in cases.iter().enumerate() {
        let mut headers: Vec<(String, String)> = vec![("content-type".into(), "application/json".into())];
        for (k, v) in c["headers"].as_object().unwrap() {
            headers.push((k.to_ascii_lowercase(), v.as_str().unwrap().to_string()));
        }
        let body = names.swap(&c["request"], false);
        let r = a.handle(&wren, A2aRequest { method: "POST".into(), target: "/a2a".into(), headers, body: body.to_string().into_bytes() }).await;
        let status = r.status;
        let (key, got) = match r.body {
            ReplyBody::Full(b) => ("answer", serde_json::from_slice::<Value>(&b).unwrap()),
            ReplyBody::Stream(mut rx) => {
                let mut events = Vec::new();
                while let Ok(Some(b)) = tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
                    let s = String::from_utf8(b.to_vec()).unwrap();
                    if let Some(data) = s.strip_prefix("data: ") {
                        events.push(serde_json::from_str::<Value>(data.trim_end()).unwrap());
                    }
                }
                ("events", Value::Array(events))
            }
        };
        names.learn(&got);
        let got = names.swap(&got, true);
        out["cases"][i]["status"] = json!(status);
        out["cases"][i][key] = got.clone();
        if c.get("status") != Some(&json!(status)) || c.get(key) != Some(&got) {
            problems.push(format!("{}: {}", c["name"].as_str().unwrap(), serde_json::to_string_pretty(&got).unwrap()));
        }
    }
    if let Ok(file) = std::env::var("XENONET_A2A_BLESS") {
        std::fs::write(file, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
    }
    let lookup: HashMap<_, _> = names.of.iter().map(|(id, n)| (n.clone(), id.clone())).collect();
    assert!(lookup.contains_key("$task") && lookup.contains_key("$task2"), "both tasks were made");
    assert!(problems.is_empty(), "answers that differ from test/fixtures/a2a/cases.json:\n{}", problems.join("\n"));
}
