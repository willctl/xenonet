//! A2A against a made-up engine: each JSON-RPC method, the states a task
//! goes through, the refusals (scope, capability, limits), the version
//! header, bad requests, and streams.

use super::*;
use crate::cores::{CoreError, R};
use async_trait::async_trait;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

/// The engine and actors, made up: sessions claude:x, claude:a and
/// claude:b (the last two in repo atlas); claude:nope is out of scope.
#[derive(Default)]
struct Fake {
    sent: Mutex<Vec<(String, Value, Value)>>,
    inbox: Mutex<Vec<Value>>,
    arrived: Notify,
    next: Mutex<i64>,
    deny_messages: AtomicBool,
    pending: AtomicBool,
}

impl Fake {
    fn rowid(&self) -> i64 {
        let mut n = self.next.locked();
        *n += 1;
        100 + *n
    }

    /// A message to the agent's inbox, as the engine's inbox answers it.
    fn reply(&self, from: &str, text: &str, reply_to: Option<i64>) -> i64 {
        let id = self.rowid();
        self.inbox.locked().push(json!({
            "id": id, "from": { "id": from, "title": format!("{from} title"), "app": "claude" }, "to": "you",
            "text": text, "at": 5.0, "replyTo": reply_to, "envelope": { "mid": format!("msg_reply{id}") }
        }));
        self.arrived.notify_waiters();
        id
    }

    fn sent(&self) -> Vec<(String, Value, Value)> {
        self.sent.locked().clone()
    }
}

#[async_trait]
impl Engine for Fake {
    async fn send(&self, me: &str, msg: Value, extra: Option<Value>) -> R<Value> {
        if self.pending.load(Ordering::SeqCst) {
            return Err(CoreError::pending("core/engine/engine.js"));
        }
        let to = msg["to"].as_str().unwrap_or("").to_string();
        let recipients: Vec<&str> = match to.as_str() {
            one @ ("claude:x" | "claude:a" | "claude:b") => vec![one],
            "project:atlas" => vec!["claude:a", "claude:b"],
            other => return Err(CoreError::new(format!("no session or actor you can reach is {other}"))),
        };
        let id = self.rowid();
        self.sent.locked().push((me.to_string(), msg, extra.unwrap_or(Value::Null)));
        Ok(json!({ "id": id, "to": recipients.iter().map(|r| json!({ "id": r, "title": "t", "delivery": "during its current turn" })).collect::<Vec<_>>() }))
    }

    async fn inbox(&self, _me: &str, wait: Duration) -> R<Value> {
        if self.inbox.locked().is_empty() && wait > Duration::ZERO {
            let _ = tokio::time::timeout(wait, self.arrived.notified()).await;
        }
        Ok(Value::Array(std::mem::take(&mut *self.inbox.locked())))
    }
}

impl Actors for Fake {
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Wren", "product": null }))
    }
    fn can(&self, _id: &str, capability: &str) -> bool {
        capability != "message-agents" || !self.deny_messages.load(Ordering::SeqCst)
    }
}

fn limits() -> Limits {
    Limits {
        block: Duration::from_millis(1500),
        stream: Duration::from_secs(5),
        keepalive: Duration::from_millis(400),
        pull: Duration::from_millis(500),
        backoff: Duration::from_millis(50),
    }
}

fn a2a_with(fake: Arc<Fake>, clock: Arc<Mutex<f64>>) -> Arc<A2a> {
    let c = clock.clone();
    let a = A2a::new(A2aParts {
        cores: A2aCores { engine: fake.clone(), actors: fake },
        file: None,
        version: "0.2.0".into(),
        home: "/home/nova".into(),
        name: Arc::new(|| None),
        now: Arc::new(move || *c.locked()),
        limits: limits(),
    })
    .unwrap();
    a.set_enabled(true).unwrap();
    a
}

fn setup() -> (Arc<A2a>, Arc<Fake>) {
    let fake = Arc::new(Fake::default());
    (a2a_with(fake.clone(), Arc::new(Mutex::new(1_792_512_345_678.0))), fake)
}

fn wren() -> A2aActor {
    A2aActor { id: "actor:wren".into(), slug: "wren".into() }
}

fn post(body: &Value) -> A2aRequest {
    A2aRequest {
        method: "POST".into(),
        target: "/a2a".into(),
        headers: vec![("content-type".into(), "application/json".into()), ("a2a-version".into(), "1.0".into())],
        body: body.to_string().into_bytes(),
    }
}

fn rpc(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
}

fn message(id: &str, text: &str, metadata: Value) -> Value {
    json!({ "messageId": id, "role": "ROLE_USER", "parts": [{ "text": text }], "metadata": metadata })
}

async fn call(a: &Arc<A2a>, actor: &A2aActor, req: A2aRequest) -> (u16, Value) {
    let r = a.handle(actor, req).await;
    let body = match r.body {
        ReplyBody::Full(b) => serde_json::from_slice(&b).unwrap(),
        ReplyBody::Stream(_) => panic!("a stream where JSON was due"),
    };
    (r.status, body)
}

async fn ask(a: &Arc<A2a>, method: &str, params: Value) -> Value {
    call(a, &wren(), post(&rpc(method, params))).await.1
}

/// Every event of a stream until it ends.
async fn events(r: Reply) -> Vec<Value> {
    assert_eq!(r.header("content-type"), Some("text/event-stream"));
    let ReplyBody::Stream(mut rx) = r.body else { panic!("not a stream") };
    let mut out = Vec::new();
    while let Ok(Some(b)) = tokio::time::timeout(Duration::from_secs(8), rx.recv()).await {
        let s = String::from_utf8(b.to_vec()).unwrap();
        if let Some(data) = s.strip_prefix("data: ") {
            out.push(serde_json::from_str(data.trim_end()).unwrap());
        }
    }
    out
}

#[tokio::test]
async fn the_card_shows_only_while_a2a_is_on() {
    let (a, _) = setup();
    assert_eq!(a.card("https://gw.example.com").unwrap()["name"], "XenoNet on gw.example.com", "named for the address outside a network");
    a.set_enabled(false).unwrap();
    assert!(a.card("https://gw.example.com").is_none());
    assert_eq!(a.status(Some("https://gw.example.com/mcp"))["card"], "https://gw.example.com/.well-known/agent-card.json");
    assert_eq!(a.status(None)["card"], Value::Null);
}

#[tokio::test]
async fn a_version_it_doesnt_speak_is_refused() {
    let (a, _) = setup();
    let mut req = post(&rpc("GetTask", json!({ "id": "x" })));
    req.headers.retain(|(k, _)| k != "a2a-version");
    let (_, body) = call(&a, &wren(), req.clone()).await;
    assert_eq!(body["error"]["code"], -32009, "no header means 0.3");
    req.headers.push(("a2a-version".into(), "0.3".into()));
    assert_eq!(call(&a, &wren(), req.clone()).await.1["error"]["code"], -32009);
    req.headers.retain(|(k, _)| k != "a2a-version");
    req.target = "/a2a?A2A-Version=1.0".into();
    assert_eq!(call(&a, &wren(), req).await.1["error"]["code"], -32001, "the parameter does too; then the task isn't there");
}

#[tokio::test]
async fn bad_requests() {
    let (a, _) = setup();
    let mut req = post(&json!({}));
    req.body = b"{not json".to_vec();
    assert_eq!(call(&a, &wren(), req).await, (200, json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Invalid JSON payload" } })));
    assert_eq!(ask(&a, "Nope", json!({})).await["error"]["code"], -32601);
    assert_eq!(call(&a, &wren(), post(&json!([rpc("GetTask", json!({}))]))).await.1["error"]["code"], -32600);
    let mut get = post(&json!({}));
    get.method = "GET".into();
    assert_eq!(call(&a, &wren(), get).await.0, 405);
    let mut text = post(&rpc("GetTask", json!({ "id": "x" })));
    text.headers[0].1 = "text/plain".into();
    assert_eq!(call(&a, &wren(), text).await.0, 415);
    let mut big = post(&json!({}));
    big.body = vec![b' '; BODY_MAX + 1];
    assert_eq!(call(&a, &wren(), big).await.0, 413);
    assert_eq!(ask(&a, "GetTask", json!({})).await["error"]["code"], -32602);
}

#[tokio::test]
async fn what_xenonet_doesnt_do() {
    let (a, _) = setup();
    for m in ["CreateTaskPushNotificationConfig", "GetTaskPushNotificationConfig", "ListTaskPushNotificationConfigs", "DeleteTaskPushNotificationConfig"] {
        assert_eq!(ask(&a, m, json!({ "taskId": "t" })).await["error"]["code"], -32003, "{m}");
    }
    assert_eq!(ask(&a, "GetExtendedAgentCard", json!({})).await["error"]["code"], -32004);
    let push = json!({ "message": message("m1", "hi", json!({ "target": "claude:x" })), "configuration": { "taskPushNotificationConfig": { "url": "https://x.example/hook" } } });
    assert_eq!(ask(&a, "SendMessage", push).await["error"]["code"], -32003);
    let file = json!({ "message": { "messageId": "m2", "role": "ROLE_USER", "parts": [{ "url": "https://x.example/a.pdf", "mediaType": "application/pdf" }] } });
    assert_eq!(ask(&a, "SendMessage", file).await["error"]["code"], -32005);
    let json_only = json!({ "message": message("m3", "hi", json!({})), "configuration": { "acceptedOutputModes": ["application/json"] } });
    assert_eq!(ask(&a, "SendMessage", json_only).await["error"]["code"], -32005);
}

#[tokio::test]
async fn a_task_is_the_message_until_its_reply() {
    let (a, fake) = setup();
    let r =
        ask(&a, "SendMessage", json!({ "message": message("m1", "Which branch has the fix?", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } }))
            .await;
    let task = &r["result"]["task"];
    assert_eq!(task["status"]["state"], "TASK_STATE_WORKING");
    assert_eq!(task["status"]["message"]["parts"][0]["text"], "Sent to claude:x. Its reply completes this task.");
    let id = task["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("msg_"));
    // The engine got a message from the actor, through the gateway, whose
    // mid is the task's id.
    let sent = fake.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "actor:wren");
    assert_eq!(sent[0].1, json!({ "to": "claude:x", "text": "Which branch has the fix?" }));
    assert_eq!(sent[0].2["envelope"]["mid"], json!(id));
    assert_eq!(sent[0].2["envelope"]["auth"]["method"], "gateway");
    assert_eq!(sent[0].2["envelope"]["sender"]["name"], "Wren");
    // The session answers it.
    fake.reply("claude:x", "On fix/login.", Some(101));
    let t = ask(&a, "GetTask", json!({ "id": id })).await["result"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(t["status"]["message"]["parts"][0]["text"], "On fix/login.");
    assert_eq!(t["status"]["message"]["role"], "ROLE_AGENT");
    assert_eq!(t["artifacts"][0]["parts"][0]["text"], "On fix/login.");
    assert_eq!(t["artifacts"][0]["metadata"]["from"], "claude:x");
    assert_eq!(t["history"].as_array().unwrap().len(), 2);
    assert_eq!(t["history"][0]["contextId"], t["contextId"]);
    let short = ask(&a, "GetTask", json!({ "id": id, "historyLength": 1 })).await["result"].clone();
    assert_eq!(short["history"].as_array().unwrap().len(), 1);
    assert_eq!(short["history"][0]["role"], "ROLE_AGENT", "the latest");
    // Another agent sees none of it.
    let moth = A2aActor { id: "actor:moth".into(), slug: "moth".into() };
    assert_eq!(call(&a, &moth, post(&rpc("GetTask", json!({ "id": id })))).await.1["error"]["code"], -32001);
}

#[tokio::test]
async fn send_message_waits_for_the_reply() {
    let (a, fake) = setup();
    let f = fake.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        f.reply("claude:x", "Done.", Some(101));
    });
    let r = ask(&a, "SendMessage", json!({ "message": message("m1", "Run the tests", json!({ "target": "claude:x" })) })).await;
    assert_eq!(r["result"]["task"]["status"]["state"], "TASK_STATE_COMPLETED");
    // With no reply, it answers once it has waited long enough.
    let started = Instant::now();
    let r = ask(&a, "SendMessage", json!({ "message": message("m2", "Anyone?", json!({ "target": "claude:x" })) })).await;
    assert_eq!(r["result"]["task"]["status"]["state"], "TASK_STATE_WORKING");
    assert!(started.elapsed() >= Duration::from_millis(1400));
}

#[tokio::test]
async fn refusals_reject_the_task_with_why() {
    let (a, fake) = setup();
    let send = |id: &str, metadata: Value| json!({ "message": message(id, "hi", metadata), "configuration": { "returnImmediately": true } });
    // Out of scope: the engine refuses.
    let t = ask(&a, "SendMessage", send("m1", json!({ "target": "claude:nope" }))).await["result"]["task"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_REJECTED");
    assert_eq!(t["status"]["message"]["parts"][0]["text"], "no session or actor you can reach is claude:nope");
    // No target, and none by default.
    let t = ask(&a, "SendMessage", send("m2", json!({}))).await["result"]["task"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_REJECTED");
    assert!(t["status"]["message"]["parts"][0]["text"].as_str().unwrap().starts_with("Say which session to reach"));
    // A default target the owner set.
    a.set_default_target("actor:wren", Some("claude:x")).unwrap();
    let t = ask(&a, "SendMessage", send("m3", json!({}))).await["result"]["task"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_WORKING");
    // Without the capability.
    fake.deny_messages.store(true, Ordering::SeqCst);
    let t = ask(&a, "SendMessage", send("m4", json!({ "target": "claude:x" }))).await["result"]["task"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_REJECTED");
    assert!(t["status"]["message"]["parts"][0]["text"].as_str().unwrap().contains("message-agents"));
    fake.deny_messages.store(false, Ordering::SeqCst);
    // An engine that isn't there fails it.
    fake.pending.store(true, Ordering::SeqCst);
    let t = ask(&a, "SendMessage", send("m5", json!({ "target": "claude:x" }))).await["result"]["task"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_FAILED");
}

#[tokio::test]
async fn an_actor_sends_sixty_an_hour_and_six_wide() {
    let clock = Arc::new(Mutex::new(1_000_000.0));
    let fake = Arc::new(Fake::default());
    let a = a2a_with(fake.clone(), clock.clone());
    let send = |i: usize, target: &str| json!({ "message": message(&format!("m{i}-{target}"), "hi", json!({ "target": target })), "configuration": { "returnImmediately": true } });
    let state = |v: Value| v["result"]["task"]["status"]["state"].as_str().unwrap().to_string();
    for i in 0..6 {
        assert_eq!(state(ask(&a, "SendMessage", send(i, "repo:atlas")).await), "TASK_STATE_WORKING");
    }
    let wide = ask(&a, "SendMessage", send(6, "repo:atlas")).await;
    assert_eq!(state(wide.clone()), "TASK_STATE_REJECTED");
    assert_eq!(wide["result"]["task"]["status"]["message"]["parts"][0]["text"], "an actor sends at most 6 messages an hour to a repo, a selector or all; try again in 600 s");
    for i in 0..54 {
        assert_eq!(state(ask(&a, "SendMessage", send(i, "claude:x")).await), "TASK_STATE_WORKING", "{i}");
    }
    let over = ask(&a, "SendMessage", send(99, "claude:x")).await;
    assert_eq!(over["result"]["task"]["status"]["message"]["parts"][0]["text"], "an actor sends at most 60 messages an hour; try again in 60 s");
    *clock.locked() += 60e3;
    assert_eq!(state(ask(&a, "SendMessage", send(100, "claude:x")).await), "TASK_STATE_WORKING", "a minute on, one more");
    assert_eq!(fake.sent().iter().filter(|s| s.1["to"] == "project:atlas").count(), 6, "repo:atlas is the engine's project:atlas");
}

#[tokio::test]
async fn the_same_message_twice_is_one_task() {
    let (a, fake) = setup();
    let p = json!({ "message": message("m1", "hi", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } });
    let one = ask(&a, "SendMessage", p.clone()).await["result"]["task"]["id"].clone();
    let two = ask(&a, "SendMessage", p).await["result"]["task"]["id"].clone();
    assert_eq!(one, two);
    assert_eq!(fake.sent().len(), 1);
}

#[tokio::test]
async fn a_context_is_a_thread() {
    let (a, fake) = setup();
    let first = ask(&a, "SendMessage", json!({ "message": message("m1", "Start", json!({ "target": "project:atlas" })), "configuration": { "returnImmediately": true } })).await;
    let ctx = first["result"]["task"]["contextId"].as_str().unwrap().to_string();
    assert!(ctx.starts_with("ctx_"));
    let reply = fake.reply("claude:b", "Here.", Some(101));
    let t = ask(&a, "GetTask", json!({ "id": first["result"]["task"]["id"] })).await;
    assert_eq!(t["result"]["status"]["state"], "TASK_STATE_COMPLETED");
    // The next message in the context answers the reply, to whoever sent it.
    let mut m = message("m2", "Thanks; and the tests?", json!({}));
    m["contextId"] = json!(ctx);
    let second = ask(&a, "SendMessage", json!({ "message": m, "configuration": { "returnImmediately": true } })).await;
    assert_eq!(second["result"]["task"]["contextId"], json!(ctx));
    let sent = fake.sent();
    assert_eq!(sent[1].1, json!({ "to": "claude:b", "text": "Thanks; and the tests?", "replyTo": reply }));
    // A message to a task that's done is refused; so is one to a task that
    // isn't there.
    let mut m = message("m3", "more", json!({}));
    m["taskId"] = first["result"]["task"]["id"].clone();
    assert_eq!(ask(&a, "SendMessage", json!({ "message": m })).await["error"]["code"], -32004);
    let mut m = message("m4", "more", json!({}));
    m["taskId"] = json!("msg_nope");
    assert_eq!(ask(&a, "SendMessage", json!({ "message": m })).await["error"]["code"], -32001);
}

#[tokio::test]
async fn a_reply_without_reply_to_answers_the_oldest_open_task_to_its_sender() {
    let (a, fake) = setup();
    let send = |id: &str| json!({ "message": message(id, "hi", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } });
    let one = ask(&a, "SendMessage", send("m1")).await["result"]["task"]["id"].clone();
    let two = ask(&a, "SendMessage", send("m2")).await["result"]["task"]["id"].clone();
    fake.reply("claude:x", "To the first.", None);
    assert_eq!(ask(&a, "GetTask", json!({ "id": one })).await["result"]["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(ask(&a, "GetTask", json!({ "id": two })).await["result"]["status"]["state"], "TASK_STATE_WORKING");
    // A message from someone it never wrote to is kept aside.
    fake.reply("codex:z", "Unasked.", None);
    ask(&a, "ListTasks", json!({})).await;
    assert_eq!(a.store.locked().loose_count("actor:wren").unwrap(), 1);
}

#[tokio::test]
async fn cancel_tells_the_session() {
    let (a, fake) = setup();
    let id = ask(&a, "SendMessage", json!({ "message": message("m1", "hi", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } })).await["result"]
        ["task"]["id"]
        .clone();
    let t = ask(&a, "CancelTask", json!({ "id": id })).await["result"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_CANCELED");
    let note = &fake.sent()[1];
    assert_eq!(note.1["to"], "claude:x");
    assert_eq!(note.1["replyTo"], 101);
    assert!(note.1["text"].as_str().unwrap().contains("needs no answer"));
    assert_eq!(ask(&a, "CancelTask", json!({ "id": id })).await["error"]["code"], -32002);
    // A late reply stays in its history; the task stays canceled.
    fake.reply("claude:x", "Too late.", Some(101));
    let t = ask(&a, "GetTask", json!({ "id": id })).await["result"].clone();
    assert_eq!(t["status"]["state"], "TASK_STATE_CANCELED");
    assert_eq!(t["history"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn list_tasks_pages_newest_first() {
    let clock = Arc::new(Mutex::new(1_000_000.0));
    let fake = Arc::new(Fake::default());
    let a = a2a_with(fake.clone(), clock.clone());
    for i in 0..5 {
        *clock.locked() += 1000.0;
        ask(&a, "SendMessage", json!({ "message": message(&format!("m{i}"), "hi", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } })).await;
    }
    let page = ask(&a, "ListTasks", json!({ "pageSize": 2 })).await["result"].clone();
    assert_eq!(page["totalSize"], 5);
    assert_eq!(page["pageSize"], 2);
    assert_eq!(page["tasks"].as_array().unwrap().len(), 2);
    assert!(page["tasks"][0].get("artifacts").is_none(), "left out unless asked for");
    let token = page["nextPageToken"].as_str().unwrap().to_string();
    assert_ne!(token, "");
    let next = ask(&a, "ListTasks", json!({ "pageSize": 2, "pageToken": token })).await["result"].clone();
    assert_ne!(next["tasks"][0]["id"], page["tasks"][0]["id"]);
    let last = ask(&a, "ListTasks", json!({ "pageSize": 2, "pageToken": next["nextPageToken"] })).await["result"].clone();
    assert_eq!(last["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(last["nextPageToken"], "", "there's always a token, empty on the last page");
    let with = ask(&a, "ListTasks", json!({ "includeArtifacts": true, "status": "TASK_STATE_WORKING", "historyLength": 0 })).await["result"].clone();
    assert_eq!(with["tasks"][0]["artifacts"], json!([]));
    assert!(with["tasks"][0].get("history").is_none());
    assert_eq!(with["totalSize"], 5);
    assert_eq!(ask(&a, "ListTasks", json!({ "status": "TASK_STATE_COMPLETED" })).await["result"]["totalSize"], 0);
    for bad in [json!({ "pageSize": 150 }), json!({ "pageSize": 0 }), json!({ "status": "TASK_STATE_RUNNING" }), json!({ "pageToken": "zz" }), json!({ "historyLength": -5 })] {
        assert_eq!(ask(&a, "ListTasks", bad.clone()).await["error"]["code"], -32602, "{bad}");
    }
}

#[tokio::test]
async fn a_stream_follows_the_task_to_its_reply() {
    let (a, fake) = setup();
    let f = fake.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        f.reply("claude:x", "All green.", Some(101));
    });
    let r = a.handle(&wren(), post(&rpc("SendStreamingMessage", json!({ "message": message("m1", "Are the tests green?", json!({ "target": "claude:x" })) })))).await;
    let ev = events(r).await;
    let kinds: Vec<&str> = ev.iter().map(|e| e["result"].as_object().unwrap().keys().next().unwrap().as_str()).collect();
    assert_eq!(kinds, ["task", "statusUpdate", "artifactUpdate", "statusUpdate"]);
    assert_eq!(ev[0]["result"]["task"]["status"]["state"], "TASK_STATE_SUBMITTED");
    assert_eq!(ev[1]["result"]["statusUpdate"]["status"]["state"], "TASK_STATE_WORKING");
    assert_eq!(ev[2]["result"]["artifactUpdate"]["artifact"]["parts"][0]["text"], "All green.");
    assert_eq!(ev[2]["result"]["artifactUpdate"]["lastChunk"], true);
    assert_eq!(ev[3]["result"]["statusUpdate"]["status"]["state"], "TASK_STATE_COMPLETED");
    assert!(ev.iter().all(|e| e["id"] == 1 && e["jsonrpc"] == "2.0"));
    let id = ev[0]["result"]["task"]["id"].clone();
    assert_eq!(ev[3]["result"]["statusUpdate"]["taskId"], id);
    // A task that's done can't be subscribed to.
    let r = a.handle(&wren(), post(&rpc("SubscribeToTask", json!({ "id": id })))).await;
    let ev = events(r).await;
    assert_eq!(ev[0]["error"]["code"], -32004);
}

#[tokio::test]
async fn a_refused_stream_ends_at_its_refusal() {
    let (a, _) = setup();
    let r = a.handle(&wren(), post(&rpc("SendStreamingMessage", json!({ "message": message("m1", "hi", json!({ "target": "claude:nope" })) })))).await;
    let ev = events(r).await;
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[1]["result"]["statusUpdate"]["status"]["state"], "TASK_STATE_REJECTED");
    // A bad request to a streaming method is an error event.
    let mut req = post(&rpc("SendStreamingMessage", json!({})));
    req.headers.retain(|(k, _)| k != "a2a-version");
    let ev = events(a.handle(&wren(), req).await).await;
    assert_eq!(ev[0]["error"]["code"], -32009);
}

#[tokio::test]
async fn a_subscription_follows_an_open_task_and_keeps_alive() {
    let (a, fake) = setup();
    let id = ask(&a, "SendMessage", json!({ "message": message("m1", "hi", json!({ "target": "claude:x" })), "configuration": { "returnImmediately": true } })).await["result"]
        ["task"]["id"]
        .clone();
    let r = a.handle(&wren(), post(&rpc("SubscribeToTask", json!({ "id": id })))).await;
    let ReplyBody::Stream(mut rx) = r.body else { panic!("not a stream") };
    let first = String::from_utf8(rx.recv().await.unwrap().to_vec()).unwrap();
    assert!(first.contains("\"TASK_STATE_WORKING\""));
    // Quiet for a while: a comment keeps it open.
    let quiet = String::from_utf8(tokio::time::timeout(Duration::from_secs(2), rx.recv()).await.unwrap().unwrap().to_vec()).unwrap();
    assert_eq!(quiet, ": keep-alive\n\n");
    fake.reply("claude:x", "Yes.", Some(101));
    let mut rest = String::new();
    while let Ok(Some(b)) = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
        rest.push_str(std::str::from_utf8(&b).unwrap());
    }
    assert!(rest.contains("artifactUpdate") && rest.contains("TASK_STATE_COMPLETED"));
}

#[tokio::test]
async fn auth_from_the_gateway() {
    assert_eq!(A2aActor::from_auth(&json!({ "ok": true, "id": "actor:wren", "slug": "wren" })), Some(wren()));
    assert_eq!(A2aActor::from_auth(&json!({ "ok": false, "id": "actor:wren" })), None);
    assert_eq!(A2aActor::from_auth(&json!({ "ok": true, "id": "claude:x" })), None, "only an actor");
}
