// The routes with stand-ins for the cores, as the JS route tests have them
// (test/engine-routes.test.js, delivery-routes, workspace-routes,
// hosts-routes, environments, integrations-routes): the same cases, through
// the whole API's dispatch, with what each core was asked recorded.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_service::cores::hosts::HubAnswer;
use xenonet_service::cores::{self, CoreError, Cores, Kind, R};
use xenonet_service::ctx::Ctx;
use xenonet_service::http::{Peer, Reply, Req};
use xenonet_service::routes::api;
use xenonet_service::serve::answered;

const ME: &str = "claude:11111111-2222-3333-4444-555555555555";
const S: &str = "claude:0190d3e0-0000-7000-8000-000000000001";

/// Every core this file stands in for, recording what it was asked.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<(String, Value)>>,
    check: Mutex<Value>,
    person: Mutex<Value>,
    window: Mutex<Value>,
    hook_out: Mutex<Option<Value>>,
    hub_answers: Mutex<Vec<(String, HubAnswer)>>,
    environment: Mutex<Value>,
    root_info: Value,
    fail_send: Mutex<Option<String>>,
    fail_integration: Mutex<Option<CoreError>>,
    /// An actor's side: the verdict on its process, and what you allowed it.
    actor_verdict: Mutex<Value>,
    allowed: Mutex<Vec<&'static str>>,
}

impl Fake {
    fn new() -> Fake {
        let f = Fake { root_info: json!({ "path": "/Users/nova/Projects", "source": "default" }), ..Default::default() };
        *f.check.lock().unwrap() = json!({ "ok": true, "root": { "pid": 4242 } });
        *f.person.lock().unwrap() = json!({ "ok": true });
        *f.window.lock().unwrap() = json!({ "ok": true });
        *f.environment.lock().unwrap() = json!("trench");
        *f.actor_verdict.lock().unwrap() = json!({ "ok": true, "checked": true, "chain": [{ "pid": 50, "exe": "/home/nova/bin/wren" }] });
        f
    }
    fn note(&self, name: &str, args: Value) {
        self.calls.lock().unwrap().push((name.to_string(), args));
    }
    fn was(&self, name: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, a)| a.clone()).collect()
    }
}

#[async_trait]
impl cores::Engine for Fake {
    async fn hook(&self, app: &str, event: Option<&str>, _body: &Value) -> R<Option<Value>> {
        self.note("hook", json!([app, event]));
        Ok(self.hook_out.lock().unwrap().clone())
    }
    async fn snapshot(&self) -> R<Value> {
        self.note("snapshot", json!([]));
        Ok(json!({ "agents": [] }))
    }
    async fn agents(&self) -> R<Value> {
        Ok(json!([{ "id": ME }]))
    }
    async fn who_has(&self, me: Option<&str>, path: &str) -> R<Value> {
        self.note("whoHas", json!([me, path]));
        Ok(json!([]))
    }
    async fn whoami(&self, _me: &str) -> R<Value> {
        Ok(json!({ "you": ME }))
    }
    async fn peers(&self, me: &str, scope: &str) -> R<Value> {
        self.note("peers", json!([me, { "scope": scope }]));
        Ok(json!([]))
    }
    async fn send(&self, me: &str, msg: Value, extra: Option<Value>) -> R<Value> {
        if let Some(e) = self.fail_send.lock().unwrap().clone() {
            return Err(CoreError::new(e));
        }
        self.note("send", json!([me, msg, extra.map(|e| e["envelope"]["auth"].clone())]));
        Ok(json!({ "id": 1 }))
    }
    async fn inbox(&self, me: &str, wait: Duration) -> R<Value> {
        self.note("inbox", json!([me, { "waitMs": wait.as_millis() as u64 }]));
        Ok(json!([]))
    }
    async fn claim(&self, me: &str, args: Value) -> R<Value> {
        self.note("claim", json!([me, args]));
        Ok(json!({ "claimed": [] }))
    }
    async fn release(&self, me: &str, paths: Value) -> R<Value> {
        self.note("release", json!([me, { "paths": paths }]));
        Ok(json!({ "released": 0 }))
    }
    async fn set_focus(&self, me: &str, text: Value) -> R<Value> {
        self.note("setFocus", json!([me, text]));
        Ok(json!({ "focus": "x" }))
    }
    fn touch(&self, me: &str, info: Value) -> R<()> {
        self.note("touch", json!([me, info]));
        Ok(())
    }
    async fn view(&self, _me: &str) -> R<Option<Value>> {
        Ok(Some(json!({ "title": "the session" })))
    }
    fn listening(&self) -> R<Vec<(String, Value)>> {
        Ok(vec![])
    }
}

#[async_trait]
impl cores::Connect for Fake {
    async fn preview(&self) -> R<Value> {
        Ok(json!({ "apps": [] }))
    }
    async fn apply(&self, apps: Value) -> R<Value> {
        self.note("apply", apps);
        Ok(json!({ "applied": true }))
    }
    async fn remove(&self) -> R<Value> {
        Ok(json!({ "removed": true }))
    }
}

#[async_trait]
impl cores::Verify for Fake {
    async fn check(&self, _peer: Peer, id: &str, hook: bool) -> Value {
        self.note("check", json!([id, hook]));
        self.check.lock().unwrap().clone()
    }
    async fn person(&self, _peer: Peer) -> Value {
        self.note("person", json!([]));
        self.person.lock().unwrap().clone()
    }
    async fn window(&self, _peer: Peer) -> Value {
        self.window.lock().unwrap().clone()
    }
    async fn actor(&self, _peer: Peer, slug: &str, _product: Value, _pid: Value) -> Value {
        self.note("actor", json!(slug));
        self.actor_verdict.lock().unwrap().clone()
    }
}

/// Wren's token, one for an actor still waiting for you, and one XenoNet
/// doesn't know (made up, and split so the doctor's look for tokens left in
/// repos passes over this file).
fn token(which: char) -> String {
    format!("Bearer {}{}", concat!("xenonet_", "actor_"), which.to_string().repeat(64))
}

impl cores::Registration for Fake {
    fn authenticate(&self, token: &str) -> Value {
        match token.chars().last() {
            Some('a') => json!({ "ok": true, "id": "actor:wren", "slug": "wren", "state": "allowed" }),
            Some('b') => json!({ "ok": true, "id": "actor:kite", "slug": "kite", "state": "waiting" }),
            _ => json!({ "ok": false, "code": "bad-token", "error": "XenoNet doesn't know that token" }),
        }
    }
}

impl cores::Actors for Fake {
    fn can(&self, _id: &str, capability: &str) -> bool {
        self.allowed.lock().unwrap().contains(&capability)
    }
    fn info(&self, _id: &str) -> Option<Value> {
        Some(json!({ "name": "Wren", "product": null }))
    }
}

// Every body passes the schemas here, so a route's own checks are what
// the tests reach.
impl cores::Schemas for Fake {
    fn check(&self, _schema: &str, _value: &Value) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl cores::Channels for Fake {
    async fn poll(&self, me: &str, wait: Duration) -> R<Value> {
        self.note("poll", json!([me, wait.as_millis() as u64]));
        Ok(json!([{ "id": 1, "text": "hi" }]))
    }
}

#[async_trait]
impl cores::State for Fake {
    fn root_info(&self) -> R<Value> {
        Ok(self.root_info.clone())
    }
    async fn set_root(&self, path: Value) -> R<Value> {
        self.note("setRoot", path.clone());
        Ok(json!({ "path": if path.is_null() { json!("/Users/nova/Projects") } else { path }, "changed": true }))
    }
    async fn check_root(&self, path: Value) -> R<Value> {
        self.note("checkRoot", path.clone());
        Ok(json!({ "ok": true, "path": path }))
    }
}

impl cores::Policy for Fake {
    fn paused(&self) -> R<bool> {
        Ok(!self.was("setPaused").is_empty() && self.was("setPaused").last().unwrap() == &json!(true))
    }
    fn grants(&self) -> R<Value> {
        Ok(json!([]))
    }
    fn consent(&self) -> R<Value> {
        Ok(json!({}))
    }
    fn read_audit(&self, n: f64) -> R<Value> {
        Ok(json!({ "n": n }))
    }
    fn set_paused(&self, paused: bool) -> R<Value> {
        self.note("setPaused", json!(paused));
        Ok(json!({ "paused": paused }))
    }
    fn grant(&self, actor: &str, scope: &str, ttl: f64) -> R<Value> {
        if !scope.contains(':') {
            return Err(CoreError::new(format!("\"{scope}\" isn't a scope")));
        }
        self.note("grant", json!([actor, scope, ttl]));
        Ok(json!({ "ok": true }))
    }
    fn allow(&self, session: &str, remote: bool, ttl: f64) -> R<Value> {
        self.note("allow", json!([session, remote, ttl]));
        Ok(json!({ "until": 1 }))
    }
    fn disallow(&self, session: &str) -> R<()> {
        self.note("disallow", json!(session));
        Ok(())
    }
}

impl cores::Router for Fake {
    fn list(&self, _n: usize) -> R<Value> {
        Ok(json!([]))
    }
}

impl cores::HostsConfig for Fake {
    fn self_info(&self) -> R<Value> {
        Ok(json!({ "id": "studio", "key": "7c1e0000000000000000000000000001", "createdAt": 1 }))
    }
    fn set_self(&self, id: &Value) -> R<Value> {
        self.note("setSelf", id.clone());
        Ok(json!({ "id": id, "key": "7c1e0000000000000000000000000001", "createdAt": 1 }))
    }
    fn ssh_config(&self) -> R<Value> {
        Ok(json!("/Users/nova/Library/Application Support/XenoNet/ssh_config"))
    }
}

#[async_trait]
impl cores::Hub for Fake {
    fn has(&self, host: &str) -> bool {
        host == "vm1"
    }
    async fn get(&self, host: &str, path: &str) -> R<HubAnswer> {
        self.note("get", json!([host, path]));
        if let Some((_, a)) = self.hub_answers.lock().unwrap().iter().find(|(p, _)| p == path) {
            return Ok(HubAnswer { status: a.status, body: a.body.clone() });
        }
        Ok(HubAnswer { status: 200, body: json!({ "id": S, "path": path }) })
    }
    fn list(&self) -> R<Value> {
        Ok(json!([{ "id": "vm1", "state": "live" }]))
    }
    fn add(&self, body: &Value, replace: bool) -> R<Value> {
        self.note("add", json!([body, { "replace": replace }]));
        Ok(json!({ "id": body["id"], "state": "connecting" }))
    }
    fn detail(&self, id: &str) -> R<Value> {
        Ok(json!({ "id": id, "engine": null }))
    }
    fn remove(&self, id: &str, forget: bool) -> R<Value> {
        self.note("remove", json!(id));
        Ok(json!({ "id": id, "removed": true, "forget": forget }))
    }
    fn reconnect(&self, id: &str) -> R<Value> {
        Ok(json!({ "id": id, "state": "connecting" }))
    }
    async fn test(&self, id: &str) -> R<Value> {
        Ok(json!({ "id": id, "ok": true }))
    }
}

impl cores::Store for Fake {
    fn environment(&self) -> R<Value> {
        Ok(self.environment.lock().unwrap().clone())
    }
    fn set_environment(&self, id: &str) -> R<()> {
        *self.environment.lock().unwrap() = json!(id);
        Ok(())
    }
}

#[async_trait]
impl cores::Integrations for Fake {
    async fn call(&self, method: &str, args: Vec<Value>) -> R<Value> {
        if let Some(e) = self.fail_integration.lock().unwrap().clone() {
            return Err(e);
        }
        self.note(method, Value::Array(args));
        Ok(json!({ "name": method }))
    }
}

/// A service whose cores are this fake where it stands in, pending elsewhere.
fn service(hub: bool) -> (Arc<Ctx>, Arc<Fake>) {
    let f = Arc::new(Fake::new());
    let mut c = Cores::pending(true, true, hub);
    c.engine = f.clone();
    c.connect = f.clone();
    c.verify = f.clone();
    c.channels = f.clone();
    c.state = f.clone();
    c.policy = f.clone();
    c.router = f.clone();
    c.hosts = f.clone();
    c.hub = hub.then(|| f.clone() as Arc<dyn cores::Hub>);
    c.store = f.clone();
    c.integrations = f.clone();
    (Ctx::for_tests(c), f)
}

/// The same, with actors on: tokens, their processes and what you allowed.
fn with_actors() -> (Arc<Ctx>, Arc<Fake>) {
    let f = Arc::new(Fake::new());
    let mut c = Cores::pending(true, false, false);
    c.engine = f.clone();
    c.verify = f.clone();
    c.policy = f.clone();
    c.registration = Some(f.clone());
    c.actors = Some(f.clone());
    c.schemas = f.clone();
    (Ctx::for_tests(c), f)
}

async fn go(ctx: &Arc<Ctx>, method: &str, path: &str, headers: &[(&str, &str)], body: Option<Value>) -> Reply {
    let text = body.map(|b| b.to_string());
    let answer = api(ctx, Req::fake(method, path, headers, text.as_deref())).await;
    answered(ctx, method, path, answer)
}

fn status_error(r: &Reply) -> (u16, Value) {
    (r.status, r.body_json()["error"].clone())
}

const AGENT: (&str, &str) = ("x-xenonet-agent", ME);

// ---------- test/engine-routes.test.js ----------

#[tokio::test]
async fn codex_hooks_go_to_the_engine_checked_against_the_asking_process() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "POST", "/api/hook/codex", &[], Some(json!({}))).await.status, 204, "nothing to say");
    assert!(f.was("check").is_empty(), "no session, nothing to check");
    *f.hook_out.lock().unwrap() = Some(json!({ "hookSpecificOutput": { "additionalContext": "hi" } }));
    let r = go(&ctx, "POST", "/api/hook/codex", &[], Some(json!({ "session_id": "t1", "hook_event_name": "SessionStart" }))).await;
    assert_eq!((r.status, r.body_json()["hookSpecificOutput"]["additionalContext"].clone()), (200, json!("hi")));
    assert_eq!(f.was("check")[0], json!(["codex:t1", true]));
    assert_eq!(f.was("hook")[1], json!(["codex", "SessionStart"]));
    *f.check.lock().unwrap() = json!({ "ok": false, "reason": "that's another process's session" });
    let r = go(&ctx, "POST", "/api/hook/codex", &[], Some(json!({ "session_id": "t1" }))).await;
    assert_eq!(status_error(&r), (403, json!("that's another process's session")));
    assert_eq!(f.was("hook").len(), 2, "not a third time");
}

#[tokio::test]
async fn the_console_and_the_commands_reads() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "GET", "/api/engine/state", &[], None).await.body_json(), json!({ "agents": [] }));
    assert_eq!(go(&ctx, "GET", "/api/engine/agents", &[], None).await.body_json(), json!([{ "id": ME }]));
    assert_eq!(go(&ctx, "GET", "/api/engine/who?path=src/a.js", &[], None).await.status, 400, "a relative path");
    assert_eq!(go(&ctx, "GET", "/api/engine/who?path=/repo/src/a.js", &[], None).await.status, 200);
    assert_eq!(f.was("whoHas")[0], json!([null, "/repo/src/a.js"]));
    assert_eq!(go(&ctx, "GET", "/api/engine/whoami", &[], None).await.status, 404, "agents' calls are posts");
    assert_eq!(go(&ctx, "POST", "/api/engine/nope", &[AGENT], None).await.status, 404);
}

#[tokio::test]
async fn connecting_agents_is_for_people_only() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "GET", "/api/engine/connect", &[], None).await.body_json(), json!({ "apps": [] }));
    assert_eq!(go(&ctx, "POST", "/api/engine/connect", &[], Some(json!({ "apps": ["claude"] }))).await.body_json(), json!({ "applied": true }));
    assert_eq!(f.was("apply")[0], json!(["claude"]));
    assert_eq!(go(&ctx, "DELETE", "/api/engine/connect", &[], None).await.body_json(), json!({ "removed": true }));
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "an agent is asking" });
    let r = go(&ctx, "POST", "/api/engine/connect", &[], Some(json!({ "apps": ["claude"] }))).await;
    assert_eq!(status_error(&r), (403, json!("an agent is asking")));
    assert_eq!(f.was("apply").len(), 1);
    let r = go(&ctx, "POST", "/api/engine/connect", &[("authorization", "Bearer x")], None).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (403, json!("yours")), "never an actor");
}

#[tokio::test]
async fn an_agents_calls_say_who_it_is_are_checked_and_note_that_its_in_touch() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "POST", "/api/engine/whoami", &[], Some(json!({}))).await.status, 400, "no agent named");
    assert_eq!(go(&ctx, "POST", "/api/engine/whoami", &[("x-xenonet-agent", "someone")], None).await.status, 400);
    let r = go(&ctx, "POST", "/api/engine/whoami", &[AGENT], Some(json!({ "cwd": "/repo", "pid": 7 }))).await;
    assert_eq!(r.body_json(), json!({ "you": ME }));
    assert_eq!(f.was("touch")[0], json!([ME, { "app": "claude", "cwd": "/repo", "via": "mcp", "pid": 4242 }]), "the pid XenoNet saw, not the one claimed");
    *f.check.lock().unwrap() = json!({ "ok": true });
    go(&ctx, "POST", "/api/engine/focus", &[AGENT], Some(json!({ "text": "tests", "cwd": 3, "pid": "7" }))).await;
    assert_eq!(f.was("touch")[1][1], json!({ "app": "claude", "cwd": null, "via": "mcp", "pid": 7 }));
    assert_eq!(f.was("setFocus")[0], json!([ME, "tests"]));
    *f.check.lock().unwrap() = json!({ "ok": false, "reason": "not you" });
    let r = go(&ctx, "POST", "/api/engine/send", &[AGENT], Some(json!({ "to": "all", "text": "hi" }))).await;
    assert_eq!(status_error(&r), (403, json!("not you")));
    assert_eq!(f.was("send"), Vec::<Value>::new());
}

#[tokio::test]
async fn each_of_an_agents_tools_reaches_the_engine_with_what_it_needs() {
    let (ctx, f) = service(false);
    for (op, body) in [
        ("peers", json!({ "scope": "machine" })),
        ("peers", json!({ "scope": "galaxy" })),
        ("who-has", json!({ "path": "src/a.js" })),
        ("send", json!({ "to": "repo", "text": "hi", "replyTo": 3 })),
        ("inbox", json!({ "waitSeconds": 500 })),
        ("claim", json!({ "paths": ["src/"], "note": "refactor", "minutes": 30 })),
        ("release", json!({ "paths": [] })),
        ("release", json!({ "paths": ["src/"] })),
    ] {
        assert_eq!(go(&ctx, "POST", &format!("/api/engine/{op}"), &[AGENT], Some(body)).await.status, 200, "{op}");
    }
    let scopes: Vec<Value> = f.was("peers").iter().map(|c| c[1].clone()).collect();
    assert_eq!(scopes, vec![json!({ "scope": "machine" }), json!({ "scope": "repo" })]);
    assert_eq!(f.was("whoHas")[0], json!([ME, "src/a.js"]));
    assert_eq!(f.was("send")[0][1], json!({ "to": "repo", "text": "hi", "replyTo": 3 }));
    // The envelope says how XenoNet knew: the session's own process.
    assert_eq!(f.was("send")[0][2]["method"], json!("session"));
    assert_eq!(f.was("inbox")[0][1], json!({ "waitMs": 120000 }), "two minutes at most");
    assert_eq!(f.was("claim")[0][1], json!({ "paths": ["src/"], "note": "refactor", "minutes": 30 }));
    let released: Vec<Value> = f.was("release").iter().map(|c| c[1].clone()).collect();
    assert_eq!(released, vec![json!({ "paths": null }), json!({ "paths": ["src/"] })]);
}

#[tokio::test]
async fn an_error_from_the_engine_is_the_callers_to_read() {
    let (ctx, f) = service(false);
    *f.fail_send.lock().unwrap() = Some("the message is empty".into());
    let r = go(&ctx, "POST", "/api/engine/send", &[AGENT], None).await;
    assert_eq!(status_error(&r), (400, json!("the message is empty")));
}

#[tokio::test]
async fn a_token_goes_in_the_header_never_in_a_url() {
    let (ctx, _) = service(false);
    let r = go(&ctx, "POST", "/api/engine/whoami?token=abc", &[AGENT], None).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (400, json!("bad-request")));
    let r = go(&ctx, "GET", "/api/actors?x=xenonet_actor_00", &[], None).await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn an_actors_calls_need_its_token_its_own_process_and_what_you_allowed_it() {
    let (ctx, f) = with_actors();
    let wren = token('a');
    let as_wren: [(&str, &str); 1] = [("authorization", &wren)];
    let code = |r: &Reply| (r.status, r.body_json()["code"].clone(), r.body_json()["error"].clone());
    // Reading needs no capability; sending and claiming do.
    assert_eq!(go(&ctx, "POST", "/api/engine/whoami", &as_wren, Some(json!({}))).await.body_json(), json!({ "you": ME }));
    assert_eq!(go(&ctx, "POST", "/api/engine/peers", &as_wren, Some(json!({}))).await.status, 200);
    let r = go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "repo", "text": "hi" }))).await;
    assert_eq!(code(&r), (403, json!("not-allowed"), json!("send needs the message-agents capability, which this actor wasn't allowed")));
    let r = go(&ctx, "POST", "/api/engine/claim", &as_wren, Some(json!({ "paths": ["src/"] }))).await;
    assert_eq!(code(&r), (403, json!("not-allowed"), json!("claim needs the edit-repos capability, which this actor wasn't allowed")));
    assert!(f.was("send").is_empty() && f.was("claim").is_empty());
    f.allowed.lock().unwrap().extend(["message-agents", "edit-repos"]);
    assert_eq!(go(&ctx, "POST", "/api/engine/claim", &as_wren, Some(json!({ "paths": ["src/"] }))).await.status, 200);
    assert_eq!(f.was("claim")[0], json!(["actor:wren", { "paths": ["src/"] }]));
    // A send says how XenoNet knew who sent it: the token, from a process it checked.
    assert_eq!(go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "claude:x", "text": "hi" }))).await.status, 200);
    assert_eq!(f.was("send")[0], json!(["actor:wren", { "to": "claude:x", "text": "hi" }, { "method": "token", "process": "checked", "strength": "partial" }]));
    // Six messages an hour to a repo or to all, sixty in all.
    for i in 0..6 {
        assert_eq!(go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "all", "text": format!("all {i}") }))).await.status, 200);
    }
    let r = go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "repo", "text": "one more" }))).await;
    assert_eq!(code(&r), (429, json!("too-many"), json!("an actor sends at most 6 messages an hour to a repo or to all")));
    assert!(r.header("retry-after").is_some());
    for i in 0..53 {
        assert_eq!(go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "claude:x", "text": format!("{i}") }))).await.status, 200, "{i}");
    }
    let r = go(&ctx, "POST", "/api/engine/send", &as_wren, Some(json!({ "to": "claude:x", "text": "sixty-one" }))).await;
    assert_eq!(code(&r), (429, json!("too-many"), json!("an actor sends at most 60 messages an hour")));
    assert_eq!(f.was("send").len(), 60);
    // Its focus is an event on the feed: 120 at once at most.
    for i in 0..120 {
        assert_eq!(go(&ctx, "POST", "/api/engine/focus", &as_wren, Some(json!({ "text": format!("{i}") }))).await.status, 200, "{i}");
    }
    assert_eq!(go(&ctx, "POST", "/api/engine/focus", &as_wren, Some(json!({ "text": "again" }))).await.status, 429);
    // What it sends has to be JSON, and small.
    let r = api(&ctx, Req::fake("POST", "/api/engine/whoami", &as_wren, Some("{"))).await;
    assert_eq!(code(&answered(&ctx, "POST", "/api/engine/whoami", r)), (400, json!("bad-request"), json!("that body isn't JSON")));
    let big = json!({ "text": "x".repeat(17 << 10) });
    assert_eq!(code(&go(&ctx, "POST", "/api/engine/whoami", &as_wren, Some(big)).await), (413, json!("too-big"), json!("that body is over the size this route takes")));
    // Only from its own processes, only once you've allowed it, only with
    // a token XenoNet knows, and as an actor or a session, never both.
    *f.actor_verdict.lock().unwrap() = json!({ "ok": false, "checked": true, "reason": "the pid it names has to be the calling process or one of its parents" });
    let r = go(&ctx, "POST", "/api/engine/whoami", &as_wren, Some(json!({}))).await;
    assert_eq!(code(&r), (403, json!("wrong-process"), json!("the pid it names has to be the calling process or one of its parents")));
    let kite = token('b');
    let r = go(&ctx, "POST", "/api/engine/whoami", &[("authorization", &kite)], Some(json!({}))).await;
    assert_eq!(code(&r).0, 403);
    assert_eq!(code(&r).1, json!("waiting"));
    let unknown = token('c');
    let r = go(&ctx, "POST", "/api/engine/whoami", &[("authorization", &unknown)], Some(json!({}))).await;
    assert_eq!((code(&r).0, code(&r).1, r.header("www-authenticate")), (401, json!("bad-token"), Some("Bearer realm=\"xenonet\", error=\"invalid_token\"")));
    let r = go(&ctx, "POST", "/api/engine/whoami", &[("authorization", "Basic d3Jlbg==")], Some(json!({}))).await;
    assert_eq!(code(&r).1, json!("no-token"));
    let r = go(&ctx, "POST", "/api/engine/whoami", &[("authorization", &wren), AGENT], Some(json!({}))).await;
    assert_eq!(code(&r), (400, json!("bad-request"), json!("a caller is a session or an actor, never both, so send x-xenonet-agent or a token, not both")));
    // A Claude Code channel is a session's alone.
    assert_eq!(go(&ctx, "POST", "/api/engine/channel", &as_wren, Some(json!({}))).await.status, 404);
}

#[tokio::test]
async fn managing_actors_is_the_windows_or_yours_never_an_actors() {
    let (ctx, f) = with_actors();
    let key = ctx.window_key.reveal().to_string();
    let wren = token('a');
    let code = |r: &Reply| (r.status, r.body_json()["code"].clone(), r.body_json()["error"].clone());
    // The window's own: with its key, no token, from its own process.
    let no_key = json!("this is for XenoNet's own window, and the request didn't come with its key");
    let in_xenonet = "this is for XenoNet's own window, and it came from a program run in XenoNet";
    for (m, p) in [
        ("POST", "/api/actors"),
        ("GET", "/api/actors/requests"),
        ("GET", "/api/actors/tokens"),
        ("POST", "/api/actors/requests/r1"),
        ("POST", "/api/actors/wren/token"),
        ("POST", "/api/actors/wren/gateway"),
        ("PUT", "/api/actors/wren/wake"),
        ("POST", "/api/actors/you"),
        ("DELETE", "/api/actors/you"),
    ] {
        let at = format!("{m} {p}");
        assert_eq!(code(&go(&ctx, m, p, &[], Some(json!({}))).await), (403, json!("yours"), no_key.clone()), "{at}");
        assert_eq!(code(&go(&ctx, m, p, &[("x-xenonet-window", "not-its-key")], Some(json!({}))).await), (403, json!("yours"), no_key.clone()), "{at}");
        let both = [("x-xenonet-window", key.as_str()), ("authorization", wren.as_str())];
        assert_eq!(code(&go(&ctx, m, p, &both, Some(json!({}))).await), (403, json!("yours"), json!("this is for XenoNet's own window, and an actor can't do it")), "{at}");
        *f.window.lock().unwrap() = json!({ "ok": false, "reason": in_xenonet });
        assert_eq!(code(&go(&ctx, m, p, &[("x-xenonet-window", &key)], Some(json!({}))).await), (403, json!("yours"), json!(in_xenonet)), "{at}");
        *f.window.lock().unwrap() = json!({ "ok": true });
        assert_ne!(go(&ctx, m, p, &[("x-xenonet-window", &key)], Some(json!({}))).await.status, 403, "{at}: the window's own gets past");
    }
    // Yours: from the window, or your own terminal, never an actor.
    let under_codex = "this is yours to do, and it came from under a Codex process; run it in your own terminal or in XenoNet";
    for (m, p) in [
        ("DELETE", "/api/actors/wren"),
        ("DELETE", "/api/actors/wren/gateway"),
        ("GET", "/api/actors/wakes"),
        ("POST", "/api/actors/wren/wake/test"),
        ("DELETE", "/api/actors/wren/wake"),
    ] {
        let at = format!("{m} {p}");
        assert_eq!(code(&go(&ctx, m, p, &[("authorization", &wren)], None).await), (403, json!("yours"), json!("this is yours to do, and an actor can't do it")), "{at}");
        assert_eq!(code(&go(&ctx, m, p, &[("x-xenonet-window", "not-its-key")], None).await), (403, json!("yours"), no_key.clone()), "{at}");
        *f.person.lock().unwrap() = json!({ "ok": false, "reason": under_codex });
        assert_eq!(code(&go(&ctx, m, p, &[], None).await), (403, json!("yours"), json!(under_codex)), "{at}");
        *f.person.lock().unwrap() = json!({ "ok": true });
        assert_ne!(go(&ctx, m, p, &[], None).await.status, 403, "{at}: from your terminal");
        assert_ne!(go(&ctx, m, p, &[("x-xenonet-window", &key)], None).await.status, 403, "{at}: from the window");
    }
    // An actor registers no actors and asks for no gateway token; a
    // program asking for either is held to its process.
    let r = go(&ctx, "POST", "/api/actors/register", &[("authorization", &wren)], Some(json!({}))).await;
    assert_eq!(code(&r), (403, json!("not-allowed"), json!("an actor can't register actors: that's for you, in XenoNet's window or your own terminal")));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/register", &[], Some(json!({}))).await).1, json!("wrong-process"));
    let r = go(&ctx, "POST", "/api/actors/wren/gateway/ask", &[("authorization", &wren)], Some(json!({}))).await;
    assert_eq!(code(&r).1, json!("not-allowed"));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/wren/gateway/ask", &[], Some(json!({}))).await).1, json!("wrong-process"));
    assert_eq!(go(&ctx, "POST", "/api/actors/%20not%20a%20slug/gateway/ask", &[], Some(json!({}))).await.status, 404);
}

#[tokio::test]
async fn an_actor_reports_only_as_itself_from_its_own_process_and_within_its_rates() {
    let (ctx, f) = with_actors();
    let (wren, kite) = (token('a'), token('b'));
    let as_wren: [(&str, &str); 1] = [("authorization", &wren)];
    let code = |r: &Reply| (r.status, r.body_json()["code"].clone(), r.body_json()["error"].clone());
    // A heartbeat is only a heartbeat.
    for (k, to) in [("events", "events"), ("targets", "targets"), ("remove", "targets"), ("replace", "targets")] {
        let r = go(&ctx, "POST", "/api/actors/heartbeat", &as_wren, Some(json!({ k: [] }))).await;
        assert_eq!(code(&r), (400, json!("bad-request"), json!(format!("{k} go to POST /api/actors/{to}, not to a heartbeat"))));
    }
    // Two a second per instance, at most.
    for _ in 0..2 {
        assert_ne!(go(&ctx, "POST", "/api/actors/heartbeat", &as_wren, Some(json!({}))).await.status, 429);
    }
    let r = go(&ctx, "POST", "/api/actors/heartbeat", &as_wren, Some(json!({}))).await;
    assert_eq!((r.status, r.header("retry-after")), (429, Some("1")));
    assert_ne!(go(&ctx, "POST", "/api/actors/heartbeat", &as_wren, Some(json!({ "instance": "phone" }))).await.status, 429, "another instance has its own");
    // Events and targets come as lists, of a bounded size.
    let r = go(&ctx, "POST", "/api/actors/events", &as_wren, Some(json!({ "events": vec![json!({}); 51] }))).await;
    assert_eq!(code(&r), (413, json!("too-big"), json!("50 events a call at most")));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/events", &as_wren, Some(json!({ "events": 5 }))).await).2, json!("events is a list of what it did"));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/targets", &as_wren, Some(json!({ "targets": "src" }))).await).2, json!("targets is a list"));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/targets", &as_wren, Some(json!({ "remove": {} }))).await).2, json!("remove is a list"));
    assert_eq!(code(&go(&ctx, "POST", "/api/actors/heartbeat", &as_wren, Some(json!([]))).await).2, json!("the body is a JSON object"));
    // One still waiting for you may ask how it stands, and nothing else.
    assert_ne!(go(&ctx, "GET", "/api/actors/me", &[("authorization", &kite)], None).await.status, 403);
    for (m, p) in
        [("POST", "/api/actors/heartbeat"), ("POST", "/api/actors/events"), ("POST", "/api/actors/targets"), ("DELETE", "/api/actors/me"), ("GET", "/api/actors/me/stream")]
    {
        assert_eq!(code(&go(&ctx, m, p, &[("authorization", &kite)], Some(json!({}))).await).1, json!("waiting"), "{m} {p}");
    }
    // And each comes from the actor's own processes.
    *f.actor_verdict.lock().unwrap() =
        json!({ "ok": false, "checked": true, "reason": "an actor's token doesn't work from under a session, and this came from under a Codex process" });
    for (m, p) in [
        ("GET", "/api/actors/me"),
        ("POST", "/api/actors/heartbeat"),
        ("POST", "/api/actors/events"),
        ("POST", "/api/actors/targets"),
        ("DELETE", "/api/actors/me"),
        ("GET", "/api/actors/me/stream"),
    ] {
        let body = match (m, p) {
            ("POST", "/api/actors/heartbeat") => Some(json!({})),
            ("POST", _) => Some(json!({ "events": [], "targets": [] })),
            _ => None,
        };
        assert_eq!(code(&go(&ctx, m, p, &as_wren, body).await).1, json!("wrong-process"), "{m} {p}");
    }
    assert_eq!(code(&go(&ctx, "GET", "/api/actors/me/stream", &[], None).await).1, json!("no-token"));
}

#[tokio::test]
async fn with_actors_off_a_token_reaches_nothing() {
    let mut c = Cores::pending(false, false, false);
    c.engine = Arc::new(Fake::new());
    let off = Ctx::for_tests(c);
    let r = go(&off, "POST", "/api/engine/whoami", &[("authorization", &token('a'))], Some(json!({}))).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (503, json!("off")));
}

// ---------- test/delivery-routes.test.js ----------

#[tokio::test]
async fn deliverys_controls_are_yours_never_an_actors_or_an_agents() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "POST", "/api/delivery/pause", &[], Some(json!({ "paused": true }))).await.body_json()["paused"], json!(true));
    go(&ctx, "POST", "/api/delivery/grant", &[], Some(json!({ "actor": "actor:wren", "scope": "project:atlas", "hours": 2 }))).await;
    assert_eq!(f.was("grant")[0], json!(["actor:wren", "project:atlas", 7200000.0]));
    let r = go(&ctx, "POST", "/api/delivery/allow", &[], Some(json!({ "session": ME, "hours": 1 }))).await;
    assert_eq!(r.body_json(), json!({ "session": ME, "until": 1 }));
    assert_eq!(go(&ctx, "POST", "/api/delivery/allow", &[], Some(json!({ "session": "fix the bug" }))).await.status, 400, "not a session's id");
    assert_eq!(go(&ctx, "POST", "/api/delivery/allow", &[], Some(json!({ "session": "ses_abc123" }))).await.body_json()["session"], json!("opencode:ses-abc123"));
    assert_eq!(go(&ctx, "POST", "/api/delivery/grant", &[], Some(json!({ "actor": "actor:wren", "scope": "fix the bug" }))).await.status, 400);
    assert_eq!(go(&ctx, "POST", "/api/delivery/pause", &[("authorization", "Bearer x")], None).await.status, 403, "never an actor");
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "this is yours to do" });
    assert_eq!(go(&ctx, "POST", "/api/delivery/pause", &[], Some(json!({ "paused": false }))).await.status, 403, "never an agent");
    assert_eq!(f.was("setPaused"), vec![json!(true)]);
    let state = go(&ctx, "GET", "/api/delivery?audit=7", &[], None).await.body_json();
    assert_eq!((state["paused"].clone(), state["audit"].clone()), (json!(true), json!({ "n": 7 })));
}

#[tokio::test]
async fn the_channel_operation_answers_a_claude_code_session_and_no_one_else() {
    let (ctx, f) = service(false);
    let r = go(&ctx, "POST", "/api/engine/channel", &[AGENT], Some(json!({ "waitSeconds": 5 }))).await;
    assert_eq!(r.body_json(), json!({ "items": [{ "id": 1, "text": "hi" }] }));
    assert_eq!(f.was("poll")[0], json!([ME, 5000]));
    let codex = go(&ctx, "POST", "/api/engine/channel", &[("x-xenonet-agent", "codex:bbbbbbbb-0002")], None).await;
    assert_eq!(codex.body_json(), json!({ "items": [], "off": true }));
    assert_eq!(go(&ctx, "POST", "/api/engine/channel", &[("authorization", "Bearer x")], None).await.status, 404);
    go(&ctx, "POST", "/api/engine/peers", &[AGENT], Some(json!({ "scope": "app:opencode status:idle" }))).await;
    go(&ctx, "POST", "/api/engine/peers", &[AGENT], Some(json!({ "scope": "nonsense here" }))).await;
    let scopes: Vec<Value> = f.was("peers").iter().map(|c| c[1]["scope"].clone()).collect();
    assert_eq!(scopes, vec![json!("app:opencode status:idle"), json!("repo")]);
}

// ---------- test/workspace-routes.test.js ----------

#[tokio::test]
async fn changing_the_projects_folder_is_yours() {
    let (ctx, f) = service(false);
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "this is yours to do, and it came from under a Claude Code process; run it in your own terminal or in XenoNet" });
    for (method, body) in [("POST", Some(json!({ "path": "~/Clients" }))), ("DELETE", None)] {
        let r = go(&ctx, method, "/api/workspace/root", &[], body).await;
        assert_eq!(
            (r.status, r.body_json()),
            (403, json!({ "error": "this is yours to do, and it came from under a Claude Code process; run it in your own terminal or in XenoNet", "code": "yours" })),
            "{method}"
        );
    }
    assert!(f.was("setRoot").is_empty(), "no folder changed");
    // Looking and checking are anyone's.
    assert_eq!(go(&ctx, "GET", "/api/workspace/root", &[], None).await.status, 200);
    assert_eq!(go(&ctx, "POST", "/api/workspace/check", &[], Some(json!({ "path": "~/Clients" }))).await.body_json(), json!({ "ok": true, "path": "~/Clients" }));
    assert_eq!(f.was("person").len(), 2);
}

#[tokio::test]
async fn from_your_terminal_the_folder_changes_and_goes_back() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "POST", "/api/workspace/root", &[], Some(json!({ "path": "~/Clients" }))).await.body_json(), json!({ "path": "~/Clients", "changed": true }));
    assert_eq!(go(&ctx, "DELETE", "/api/workspace/root", &[], None).await.status, 200);
    assert_eq!(go(&ctx, "POST", "/api/workspace/root", &[], Some(json!({}))).await.status, 400, "no folder named");
    assert_eq!(f.was("setRoot"), vec![json!("~/Clients"), Value::Null]);
}

// ---------- test/hosts-routes.test.js ----------

#[tokio::test]
async fn this_xenonet_as_a_host_needs_no_flag() {
    for hub in [true, false] {
        let (ctx, _) = service(hub);
        assert_eq!(
            go(&ctx, "GET", "/api/hosts/self", &[], None).await.body_json(),
            json!({ "id": "studio", "key": "7c1e0000000000000000000000000001", "createdAt": 1, "home": "/home/nova" })
        );
        assert_eq!(go(&ctx, "POST", "/api/hosts/self", &[], Some(json!({ "id": "lab" }))).await.body_json()["id"], json!("lab"));
        assert_eq!(go(&ctx, "POST", "/api/hosts/self", &[], Some(json!({}))).await.status, 400);
    }
    let (ctx, f) = service(true);
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "this is yours to do, and it came from under a Claude Code process" });
    let r = go(&ctx, "POST", "/api/hosts/self", &[], Some(json!({ "id": "lab" }))).await;
    assert_eq!(status_error(&r), (403, json!("this is yours to do, and it came from under a Claude Code process")));
    assert_eq!(f.was("setSelf"), Vec::<Value>::new());
}

#[tokio::test]
async fn without_the_flag_showing_hosts_answers_404() {
    let (ctx, _) = service(false);
    for (m, p) in [("GET", "/api/hosts"), ("POST", "/api/hosts"), ("GET", "/api/hosts/vm1"), ("DELETE", "/api/hosts/vm1"), ("POST", "/api/hosts/vm1/test")] {
        let r = go(&ctx, m, p, &[], Some(json!({ "id": "vm1" }))).await;
        assert_eq!(r.status, 404, "{m} {p}");
        assert!(r.body_json()["error"].as_str().unwrap().contains("XENONET_HOSTS=1"));
    }
    // A remote id is left to the routes that don't know it: the state's.
    let r = go(&ctx, "GET", &format!("/api/session/{}/transcript", urlencode(&format!("@vm1:{S}"))), &[], None).await;
    assert_eq!(r.status, 501, "the server's own route, whose state is pending here");
}

#[tokio::test]
async fn adding_and_removing_are_yours_the_rest_whats_on_the_list() {
    let (ctx, f) = service(true);
    assert_eq!(
        go(&ctx, "GET", "/api/hosts", &[], None).await.body_json(),
        json!({ "self": { "id": "studio", "key": "7c1e0000000000000000000000000001", "createdAt": 1 }, "sshConfig": "/Users/nova/Library/Application Support/XenoNet/ssh_config", "hosts": [{ "id": "vm1", "state": "live" }] })
    );
    assert_eq!(go(&ctx, "POST", "/api/hosts", &[], Some(json!({ "id": "vm2", "target": "lab@vm2.local" }))).await.body_json(), json!({ "id": "vm2", "state": "connecting" }));
    assert_eq!(f.was("add")[0], json!([{ "id": "vm2", "target": "lab@vm2.local" }, { "replace": false }]));
    go(&ctx, "POST", "/api/hosts", &[], Some(json!({ "id": "vm2", "replace": true }))).await;
    assert_eq!(f.was("add")[1][1], json!({ "replace": true }));
    assert_eq!(go(&ctx, "DELETE", "/api/hosts/vm1?forget=1", &[], None).await.body_json(), json!({ "id": "vm1", "removed": true, "forget": true }));
    assert_eq!(go(&ctx, "POST", "/api/hosts/vm1/reconnect", &[], None).await.body_json(), json!({ "id": "vm1", "state": "connecting" }));
    assert_eq!(go(&ctx, "POST", "/api/hosts/vm1/test", &[], None).await.body_json(), json!({ "id": "vm1", "ok": true }));
    assert_eq!(go(&ctx, "POST", "/api/hosts/vm9/test", &[], None).await.status, 404);
    assert_eq!(go(&ctx, "GET", "/api/hosts/vm1", &[], None).await.body_json(), json!({ "id": "vm1", "engine": null }));
    assert_eq!(go(&ctx, "GET", "/api/hosts/Not-A-Host", &[], None).await.status, 404);
    assert_eq!(go(&ctx, "PUT", "/api/hosts/vm1", &[], None).await.status, 405);
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "yours to do" });
    assert_eq!(go(&ctx, "POST", "/api/hosts", &[], Some(json!({ "id": "vm2" }))).await.status, 403);
    assert_eq!(go(&ctx, "DELETE", "/api/hosts/vm1", &[], None).await.status, 403);
    assert_eq!((f.was("add").len(), f.was("remove").len()), (2, 1));
}

fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[tokio::test]
async fn a_remote_sessions_transcript_and_stats_come_through_its_relay() {
    let (ctx, f) = service(true);
    let id = format!("@vm1:{S}");
    let t = go(&ctx, "GET", &format!("/api/session/{}/transcript?limit=50&cursor=abc_1", urlencode(&id)), &[], None).await;
    assert_eq!(t.status, 200);
    assert_eq!(f.was("get")[0], json!(["vm1", format!("/api/session/{S}/transcript?limit=50&cursor=abc_1")]));
    assert_eq!(t.body_json(), json!({ "id": id, "path": format!("/api/session/{S}/transcript?limit=50&cursor=abc_1"), "host": "vm1" }), "its ids as this XenoNet writes them");
    go(&ctx, "GET", &format!("/api/session/{id}/stats?limit=9"), &[], None).await;
    assert_eq!(f.was("get")[1], json!(["vm1", format!("/api/session/{S}/stats")]), "stats take no query");
    for bad in [format!("/api/session/{}/transcript?limit=all", urlencode(&id)), format!("/api/session/{}/transcript?cursor={}", urlencode(&id), urlencode("a b"))] {
        assert_eq!(go(&ctx, "GET", &bad, &[], None).await.status, 400, "{bad}");
    }
    for unknown in [format!("@vm9:{S}"), "@vm1:project:~/x".to_string(), "@vm1:@vm2:claude:x".to_string(), "@vm1:claude:../../x".to_string()] {
        assert_eq!(go(&ctx, "GET", &format!("/api/session/{}/stats", urlencode(&unknown)), &[], None).await.status, 404, "{unknown}");
    }
    assert_eq!(f.was("get").len(), 2, "nothing else reached a relay");

    f.hub_answers.lock().unwrap().push((
        format!("/api/collision?a={S}&b=claude:2"),
        HubAnswer { status: 200, body: json!({ "a": { "id": S, "title": "A" }, "b": { "id": "claude:2", "title": "B" }, "files": [] }) },
    ));
    let c = go(&ctx, "GET", &format!("/api/collision?a={}&b={}", urlencode(&id), urlencode("@vm1:claude:2")), &[], None).await;
    assert_eq!(c.body_json(), json!({ "a": { "id": id, "title": "A" }, "b": { "id": "@vm1:claude:2", "title": "B" }, "files": [], "host": "vm1" }));
    assert_eq!(go(&ctx, "GET", &format!("/api/collision?a={}&b={S}", urlencode(&id)), &[], None).await.status, 400, "across machines comes later");
}

// ---------- test/environments.test.js ----------

#[tokio::test]
async fn environment_settings_persist_and_bad_or_unauthorized_choices_are_refused() {
    let (ctx, f) = service(false);
    let mut events = ctx.events.subscribe(false);
    events.recv().await;
    let r = go(&ctx, "GET", "/api/environment", &[], None).await.body_json();
    assert_eq!(r["current"], json!("trench"));
    let ids: Vec<Value> = r["environments"].as_array().unwrap().iter().map(|e| e["id"].clone()).collect();
    assert_eq!(ids, vec![json!("nebula"), json!("aurora"), json!("trench"), json!("vault")]);
    go(&ctx, "POST", "/api/environment", &[], Some(json!({ "id": "vault" }))).await;
    assert_eq!(*f.environment.lock().unwrap(), json!("vault"));
    assert_eq!(events.recv().await.unwrap(), bytes::Bytes::from("event: environment\ndata: {\"id\":\"vault\"}\n\n"));
    assert_eq!(go(&ctx, "POST", "/api/environment", &[], Some(json!({ "id": "unknown" }))).await.status, 404);
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "no" });
    go(&ctx, "POST", "/api/environment", &[], Some(json!({ "id": "aurora" }))).await;
    assert_eq!(*f.environment.lock().unwrap(), json!("vault"));
    assert_eq!(go(&ctx, "PUT", "/api/environment", &[], None).await.status, 405);
}

// ---------- test/integrations-routes.test.js ----------

#[tokio::test]
async fn integration_reads_are_yours_and_bearer_actors_cant_read_or_write() {
    let (ctx, f) = service(false);
    for route in ["/api/integrations", "/api/integrations/history", "/api/integrations/pi", "/api/integrations/sandbox/review?id=job"] {
        assert_eq!(go(&ctx, "GET", route, &[("authorization", "Bearer actor")], None).await.status, 403);
    }
    *f.person.lock().unwrap() = json!({ "ok": false, "reason": "no" });
    for route in ["/api/integrations", "/api/integrations/history"] {
        assert_eq!(go(&ctx, "GET", route, &[], None).await.status, 403);
    }
    assert!(f.calls.lock().unwrap().iter().all(|(n, _)| n == "person"));
}

#[tokio::test]
async fn every_integration_change_needs_the_window_before_its_body_is_read() {
    let (ctx, f) = service(false);
    for suffix in ["route/preview", "route/send", "route/outcome", "sandbox/check", "sandbox/launch", "sandbox/delete", "pi/import"] {
        let r = go(&ctx, "POST", &format!("/api/integrations/{suffix}"), &[], Some(json!({}))).await;
        assert_eq!(r.status, 403, "no window key: {suffix}");
    }
    assert!(f.calls.lock().unwrap().iter().all(|(n, _)| n == "person"));
}

#[tokio::test]
async fn the_integration_routes_pass_what_each_method_takes() {
    let (ctx, f) = service(false);
    let key = ctx.window_key.reveal().to_string();
    let w = [("x-xenonet-window", key.as_str())];
    for (suffix, name, body, args) in [
        ("route/preview", "routing.preview", json!({ "request": "task" }), json!([{ "request": "task" }])),
        ("route/send", "routing.route", json!({ "ticket": "t" }), json!(["t"])),
        ("route/outcome", "routing.outcome", json!({ "id": "d", "outcome": "useful" }), json!(["d", "useful"])),
        ("sandbox/export", "shell.exportReview", json!({ "id": "job", "revision": "r" }), json!(["job", "r"])),
        ("pi/enable", "enablePi", json!({ "on": true }), json!([true])),
    ] {
        assert_eq!(go(&ctx, "GET", &format!("/api/integrations/{suffix}"), &w, None).await.status, 404, "never on GET");
        let r = go(&ctx, "POST", &format!("/api/integrations/{suffix}"), &w, Some(body)).await;
        assert_eq!(r.body_json()["name"], json!(name));
        assert_eq!(f.was(name)[0], args);
    }
    go(&ctx, "GET", "/api/integrations/history?q=test&kind=decision", &[], None).await;
    assert_eq!(f.was("history")[0], json!([{ "q": "test", "kind": "decision" }]));
    go(&ctx, "GET", "/api/integrations/sandbox/review?id=job", &[], None).await;
    assert_eq!(f.was("shell.review")[0], json!(["job"]));
    *f.fail_integration.lock().unwrap() = Some(CoreError::of(Kind::Integration, 409, "expired", "Preview again."));
    let r = go(&ctx, "POST", "/api/integrations/route/send", &w, Some(json!({}))).await;
    assert_eq!((r.status, r.body_json()), (409, json!({ "error": "Preview again.", "code": "expired" })));
    *f.fail_integration.lock().unwrap() = Some(CoreError::new("PRIVATE_TOKEN"));
    let r = go(&ctx, "POST", "/api/integrations/pi/import", &w, Some(json!({}))).await;
    assert_eq!(r.status, 502);
    assert!(!r.body_json().to_string().contains("PRIVATE_TOKEN"));
    let r = go(&ctx, "POST", "/api/integrations/pi/import", &w, None).await;
    assert_eq!(r.status, 502, "an empty body is {{}}, which reaches the integration");
}

// ---------- server/index.js's own checks ----------

#[tokio::test]
async fn the_version_header_and_pending_cores() {
    let (ctx, _) = service(false);
    let r = go(&ctx, "GET", "/api/health", &[("x-xenonet-api-version", "2")], None).await;
    assert_eq!((r.status, r.body_json()["api"].clone()), (426, json!({ "min": 1, "max": 1 })));
    assert_eq!(go(&ctx, "GET", "/api/health", &[("x-xenonet-api-version", "1")], None).await.status, 200);
    let r = go(&ctx, "GET", "/api/digest", &[], None).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (501, json!("pending")), "a core not in Rust yet says so");
    assert_eq!(go(&ctx, "GET", "/api/nothing-here", &[], None).await.status, 404);
    let r = go(&ctx, "POST", "/api/layout", &[], None).await;
    assert_eq!(r.status, 501);
}

#[tokio::test]
async fn bodies_that_arent_json_or_are_too_big_are_the_clients_error() {
    let (ctx, _) = service(false);
    let r = api(&ctx, Req::fake("POST", "/api/layout", &[], Some("{ not json"))).await;
    assert_eq!(r.err().and_then(|e| e.status), Some(400));
    let big = format!("{{\"positions\":{{}},\"pad\":\"{}\"}}", "x".repeat(1 << 20));
    let r = api(&ctx, Req::fake("POST", "/api/layout", &[], Some(&big))).await;
    assert_eq!(r.err().and_then(|e| e.status), Some(413));
}

#[tokio::test]
async fn the_windows_message_to_an_actor_needs_its_key_and_an_actor() {
    let (ctx, f) = service(false);
    let r = go(&ctx, "POST", "/api/actors/you/message", &[], Some(json!({ "to": "actor:muse", "text": "hi" }))).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (403, json!("yours")));
    let key = ctx.window_key.reveal().to_string();
    let r = go(&ctx, "POST", "/api/actors/you/message", &[("x-xenonet-window", &key), ("authorization", "Bearer x")], Some(json!({ "to": "actor:muse" }))).await;
    assert_eq!(r.status, 403, "never with a token");
    let r = go(&ctx, "POST", "/api/actors/you/message", &[("x-xenonet-window", &key)], Some(json!({ "to": ME, "text": "hi" }))).await;
    assert_eq!((r.status, r.body_json()["code"].clone()), (400, json!("bad-request")), "sessions get messages from sessions");
    let r = go(&ctx, "POST", "/api/actors/you/message", &[("x-xenonet-window", &key)], Some(json!({ "to": "actor:muse", "text": "hi" }))).await;
    assert_eq!(r.status, 200);
    assert_eq!(f.was("send")[0][0], json!("actor:you"));
    assert_eq!(f.was("send")[0][2], json!({ "method": "window", "process": "n/a", "strength": "strong" }));
}

#[tokio::test]
async fn claude_codes_hooks_are_checked_against_the_sessions_process() {
    let (ctx, f) = service(false);
    assert_eq!(go(&ctx, "POST", "/api/hook/claude", &[], Some(json!({ "session_id": "not-a-session" }))).await.status, 204);
    assert_eq!(f.was("check"), Vec::<Value>::new());
    *f.check.lock().unwrap() = json!({ "ok": false, "reason": "that's another process's session" });
    let r = go(&ctx, "POST", "/api/hook/claude", &[], Some(json!({ "session_id": "11111111-2222-3333-4444-555555555555", "hook_event_name": "Stop" }))).await;
    assert_eq!(status_error(&r), (403, json!("that's another process's session")));
    assert_eq!(f.was("check")[0], json!([ME, true]));
}
