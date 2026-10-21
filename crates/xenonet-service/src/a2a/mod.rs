//! A2A, in to XenoNet (docs/design/network.md, section 12): agents built on
//! frameworks that speak A2A 1.0 (the Linux Foundation's Agent2Agent) reach
//! the sessions here as actors, through the gateway.
//!
//! The gateway's front door answers `/.well-known/agent-card.json` with
//! [`A2a::card`] and, once it has checked a gateway token, hands `/a2a` to
//! [`A2a::handle`] with the actor the token is for. This module is the rest:
//!
//! - **Each A2A client is an actor**, with its gateway token, its scopes
//!   (default none, which the engine holds it to) and the gateway's limits:
//!   60 messages an hour, 6 of them to a repo, a selector or all.
//! - **A message is a XenoNet message** from the actor, sent with the
//!   engine's `send` as the gateway's own `send` tool sends it, to the
//!   target its metadata names, the context's last speaker, or the actor's
//!   default target.
//! - **A task is that message**, by its mid. It's SUBMITTED until the engine
//!   has it, WORKING once it's in its recipients' inboxes, COMPLETED when one
//!   replies (the reply is its status message and an artifact), CANCELED on
//!   CancelTask (the recipients get a note), and REJECTED or FAILED, with
//!   why, when it can't go.
//! - **A context is a XenoNet thread**: the next message in it answers the
//!   last one (`reply_to`), so the engine's hop limit holds there too.
//! - **Replies are read from the actor's inbox**, by one pull at a time per
//!   actor, which always finishes and keeps what it took even when the
//!   client that started it goes away.
//! - **No outbound calls**: no push notifications.
//!
//! The tasks are kept in `<state>/a2a.sqlite` (store.rs), with the switch,
//! which is off until the owner turns it on in the Actors panel.

pub mod card;
pub mod routes;
pub mod rpc;
pub mod store;
pub mod tasks;

use crate::borrowed::{attest, platform, random_uuid, Sender};
use crate::buckets::{Buckets, Rate};
use crate::cores::{Actors, Engine};
use crate::http::{parse_target, Reply, ReplyBody};
use bytes::Bytes;
use card::CardSettings;
use rpc::{json_reply, result, sse_error, sse_frame, sse_headers, RpcError};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::{ReplyRow, Store, TaskRow};
use tasks::Shown;
use tokio::sync::{mpsc, watch};
use xenonet_base::sync::Locked;

/// The most of a body XenoNet reads, as the gateway's own limit.
pub const BODY_MAX: usize = 64 << 10;
/// An actor's messages, as the gateway's `send` counts them.
pub const MESSAGES: Rate = Rate { rate: 60.0 / 3600.0, burst: 60.0 };
/// Of those, to a repo, a selector or all.
pub const WIDE: Rate = Rate { rate: 6.0 / 3600.0, burst: 6.0 };
const ON: &str = "on";

/// The actor the front door checked the token of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A2aActor {
    /// `actor:<slug>`.
    pub id: String,
    pub slug: String,
}

impl A2aActor {
    /// From what `Registration::authenticate_gateway` answered for a token
    /// it took (`{ ok: true, id, slug }`); None for anything else.
    pub fn from_auth(auth: &Value) -> Option<A2aActor> {
        if auth.get("ok") != Some(&Value::Bool(true)) {
            return None;
        }
        let id = auth.get("id")?.as_str()?;
        let slug = auth.get("slug").and_then(Value::as_str).map_or_else(|| id.trim_start_matches("actor:").to_string(), str::to_string);
        id.starts_with("actor:").then(|| A2aActor { id: id.to_string(), slug })
    }
}

/// One request to `/a2a`, as the front door read it.
#[derive(Clone, Debug, Default)]
pub struct A2aRequest {
    pub method: String,
    /// The path and query.
    pub target: String,
    /// The headers, names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The cores A2A calls: the engine's send and inbox, as the gateway's
/// tools do, and the actors (capabilities, names).
#[derive(Clone)]
pub struct A2aCores {
    pub engine: Arc<dyn Engine>,
    pub actors: Arc<dyn Actors>,
}

/// How long things wait.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// A SendMessage that waits for the reply waits this long at most, then
    /// answers with the task as it is: proxies cut long requests.
    pub block: Duration,
    /// A stream ends after this, while its task is still open; the client
    /// follows on with SubscribeToTask. It fits the front door's limit on a
    /// connection.
    pub stream: Duration,
    /// A comment on an open stream this often, so proxies keep it.
    pub keepalive: Duration,
    /// One pull of an actor's inbox waits this long for a message.
    pub pull: Duration,
    /// After a pull that failed, the next waits this long.
    pub backoff: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            block: Duration::from_secs(25),
            stream: Duration::from_secs(85),
            keepalive: Duration::from_secs(15),
            pull: Duration::from_secs(20),
            backoff: Duration::from_secs(2),
        }
    }
}

pub struct A2aParts {
    pub cores: A2aCores,
    /// `<state>/a2a.sqlite`, or None to keep it in memory (the demo, tests).
    pub file: Option<PathBuf>,
    /// XenoNet's version, for the card.
    pub version: String,
    pub home: String,
    /// The network's name, or this host's id in it, when it's in one.
    pub name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    pub now: Arc<dyn Fn() -> f64 + Send + Sync>,
    pub limits: Limits,
}

pub struct A2a {
    cores: A2aCores,
    store: Mutex<Store>,
    problem: Option<String>,
    buckets: Mutex<Buckets>,
    watches: Mutex<HashMap<String, watch::Sender<u64>>>,
    pulling: Mutex<HashSet<String>>,
    version: String,
    home: String,
    name: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    now: Arc<dyn Fn() -> f64 + Send + Sync>,
    limits: Limits,
}

/// A new task, kept as SUBMITTED, and what's to be sent for it.
#[derive(Debug)]
pub struct Pending {
    task: TaskRow,
    text: String,
    to: Option<String>,
    reply_to: Option<i64>,
}

enum Prepared {
    /// The client sent this message before: its task.
    Existing(String),
    New(Box<Pending>),
}

/// SendMessageRequest's configuration.
struct Config {
    history: Option<usize>,
    immediately: bool,
}

fn config_of(params: &Value) -> Result<Config, RpcError> {
    let c = match params.get("configuration") {
        None | Some(Value::Null) => return Ok(Config { history: None, immediately: false }),
        Some(c @ Value::Object(_)) => c,
        Some(_) => return Err(RpcError::invalid_params("configuration", "must be an object")),
    };
    if c.get("taskPushNotificationConfig").is_some_and(|p| !p.is_null()) {
        return Err(RpcError::push_not_supported());
    }
    if !tasks::accepts_text(c.get("acceptedOutputModes")) {
        return Err(RpcError::content_type("XenoNet answers in text/plain, which acceptedOutputModes leaves out"));
    }
    let immediately = match c.get("returnImmediately") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(RpcError::invalid_params("configuration.returnImmediately", "must be a boolean")),
    };
    Ok(Config { history: tasks::history_length(c.get("historyLength"), "configuration.historyLength")?, immediately })
}

fn db(e: &rusqlite::Error) -> RpcError {
    RpcError::internal(&format!("XenoNet's task table: {e}"))
}

/// A number the engine gave, as an id.
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
}

/// The messages an inbox answered: a list, or `{ messages }`.
fn messages_of(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(l) => l.clone(),
        Value::Object(o) => o.get("messages").and_then(Value::as_array).cloned().unwrap_or_default(),
        _ => Vec::new(),
    }
}

impl A2a {
    /// The A2A side, with its table opened. A table that can't be opened
    /// leaves A2A off, kept in memory, saying why.
    pub fn new(parts: A2aParts) -> Result<Arc<A2a>, String> {
        let (store, problem) = match Store::open(parts.file.as_deref()) {
            Ok(s) => (s, None),
            Err(e) => (Store::open(None).map_err(|m| m.to_string())?, Some(format!("a2a.sqlite: {e}"))),
        };
        Ok(Arc::new(A2a {
            cores: parts.cores,
            store: Mutex::new(store),
            problem,
            buckets: Mutex::new(Buckets::default()),
            watches: Mutex::new(HashMap::new()),
            pulling: Mutex::new(HashSet::new()),
            version: parts.version,
            home: parts.home,
            name: parts.name,
            now: parts.now,
            limits: parts.limits,
        }))
    }

    fn now(&self) -> f64 {
        (self.now)()
    }

    // ---------- the switch, and the window's view ----------

    /// Whether A2A agents may reach XenoNet: off until the owner says.
    pub fn enabled(&self) -> bool {
        self.problem.is_none() && self.store.locked().setting(ON).ok().flatten().as_deref() == Some("true")
    }

    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        if let Some(p) = &self.problem {
            return Err(p.clone());
        }
        self.store.locked().set_setting(ON, if on { "true" } else { "false" }).map_err(|e| e.to_string())
    }

    /// The target an actor's messages go to when they name none, set by
    /// the owner; None takes it away.
    pub fn set_default_target(&self, actor: &str, target: Option<&str>) -> Result<(), String> {
        let target = target.map(xenonet_base::text::js_trim).filter(|t| !t.is_empty());
        if target.is_some_and(|t| t.len() > 300) {
            return Err("a target is at most 300 characters".into());
        }
        self.store.locked().set_default_target(actor, target, self.now()).map_err(|e| e.to_string())
    }

    /// An actor was removed: its tasks, threads and default target go with
    /// its record.
    pub fn forget(&self, actor: &str) {
        let _ = self.store.locked().forget_agent(actor);
    }

    /// For the Actors panel: the switch, the card's and endpoint's
    /// addresses at the gateway's `address` (none while it has none), and
    /// the agents that called.
    pub fn status(&self, address: Option<&str>) -> Value {
        let base = address.filter(|a| !a.is_empty()).map(card::origin);
        let agents: Vec<Value> = self
            .store
            .locked()
            .agents()
            .unwrap_or_default()
            .into_iter()
            .map(|mut a| {
                let id = a["id"].as_str().unwrap_or("").to_string();
                a["name"] = self.cores.actors.info(&id).and_then(|i| i.get("name").cloned()).unwrap_or(Value::Null);
                a
            })
            .collect();
        json!({
            "on": self.enabled(),
            "card": base.as_ref().map(|b| format!("{b}{}", card::CARD_PATH)),
            "endpoint": base.as_ref().map(|b| format!("{b}{}", card::PATH)),
            "agents": agents,
            "problem": self.problem,
        })
    }

    /// How long a stream lasts before it ends itself: the front door keeps
    /// its connection that long and a little more.
    pub fn stream_limit(&self) -> Duration {
        self.limits.stream
    }

    /// The Agent Card at `public_address`, or None while A2A is off (the
    /// front door answers 404 then).
    pub fn card(&self, public_address: &str) -> Option<Value> {
        if !self.enabled() {
            return None;
        }
        let name = (self.name)().filter(|n| !n.is_empty()).unwrap_or_else(|| card::host_of(public_address));
        Some(card::agent_card(&CardSettings { name, version: self.version.clone() }, public_address))
    }

    // ---------- the JSON-RPC binding ----------

    /// One request to `/a2a`, from `actor`, whose gateway token the front
    /// door checked. A JSON answer, or an event stream for the streaming
    /// methods.
    pub async fn handle(self: &Arc<Self>, actor: &A2aActor, req: A2aRequest) -> Reply {
        let t = self.now();
        // As if there were no /a2a, whatever the front door checked.
        if !self.enabled() {
            return json_reply(404, &json!({ "error": "A2A is off here: the owner turns it on in XenoNet's Actors panel" }), &[]);
        }
        if req.method != "POST" {
            return json_reply(405, &RpcError::invalid_request("A2A's JSON-RPC binding takes POST").to_json(&Value::Null), &[("allow", "POST")]);
        }
        if req.body.len() > BODY_MAX {
            return json_reply(413, &RpcError::invalid_request("that body is over 64 KiB").to_json(&Value::Null), &[]);
        }
        let ct = req.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-type")).map_or("", |(_, v)| v.as_str()).to_ascii_lowercase();
        if !ct.contains("json") {
            return json_reply(415, &RpcError::invalid_request("Content-Type must be application/json").to_json(&Value::Null), &[]);
        }
        let _ = self.store.locked().called(&actor.id, t);
        let Ok(parsed) = serde_json::from_slice::<Value>(&req.body) else {
            return json_reply(200, &RpcError::parse().to_json(&Value::Null), &[]);
        };
        let (id, method, params) = match rpc::envelope(&parsed) {
            Ok(x) => x,
            Err((id, e)) => return json_reply(200, &e.to_json(&id), &[]),
        };
        let streaming = matches!(method.as_str(), "SendStreamingMessage" | "SubscribeToTask");
        let fail = |e: RpcError| if streaming { sse_error(&id, &e) } else { json_reply(e.status, &e.to_json(&id), &[]) };
        let (_, query) = parse_target(&req.target);
        let asked = rpc::asked_version(&req.headers, &query);
        if !rpc::speaks(&asked) {
            return fail(RpcError::version(&asked));
        }
        let answer = match method.as_str() {
            "SendMessage" => self.send_message(actor, &params).await,
            "GetTask" => self.get_task(actor, &params).await,
            "ListTasks" => self.list_tasks(actor, &params).await,
            "CancelTask" => self.cancel_task(actor, &params).await,
            "SendStreamingMessage" => return self.send_streaming(actor, id.clone(), &params).unwrap_or_else(fail),
            "SubscribeToTask" => return self.subscribe(actor, id.clone(), &params).await.unwrap_or_else(fail),
            "CreateTaskPushNotificationConfig" | "GetTaskPushNotificationConfig" | "ListTaskPushNotificationConfigs" | "DeleteTaskPushNotificationConfig" => {
                Err(RpcError::push_not_supported())
            }
            "GetExtendedAgentCard" => Err(RpcError::unsupported("XenoNet has no extended Agent Card; the public one says it all")),
            other => Err(RpcError::method_not_found(other)),
        };
        match answer {
            Ok(v) => json_reply(200, &result(&id, v), &[]),
            Err(e) => fail(e),
        }
    }

    fn load(&self, actor: &str, id: &str) -> Result<(TaskRow, Vec<ReplyRow>), RpcError> {
        let s = self.store.locked();
        let t = s.task(actor, id).map_err(|e| db(&e))?.ok_or_else(|| RpcError::task_not_found(id))?;
        let r = s.replies(&t.id).map_err(|e| db(&e))?;
        Ok((t, r))
    }

    fn task_param(params: &Value) -> Result<String, RpcError> {
        match params.get("id") {
            Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
            _ => Err(RpcError::invalid_params("id", "the task's id is required")),
        }
    }

    async fn send_message(self: &Arc<Self>, actor: &A2aActor, params: &Value) -> Result<Value, RpcError> {
        let config = config_of(params)?;
        let id = match self.prepare(actor, params)? {
            Prepared::Existing(id) => id,
            Prepared::New(p) => {
                let id = p.task.id.clone();
                self.deliver_detached(actor, *p).await;
                id
            }
        };
        if config.immediately {
            self.sync_now(&actor.id).await;
        } else {
            let deadline = Instant::now() + self.limits.block;
            self.follow(actor, &id, (String::new(), 0), deadline, &mut |_| true).await;
        }
        let (t, r) = self.load(&actor.id, &id)?;
        Ok(json!({ "task": tasks::task(&t, &r, Shown { history: config.history, artifacts: true }) }))
    }

    async fn get_task(self: &Arc<Self>, actor: &A2aActor, params: &Value) -> Result<Value, RpcError> {
        let id = Self::task_param(params)?;
        let history = tasks::history_length(params.get("historyLength"), "historyLength")?;
        self.sync_now(&actor.id).await;
        let (t, r) = self.load(&actor.id, &id)?;
        Ok(tasks::task(&t, &r, Shown { history, artifacts: true }))
    }

    async fn list_tasks(self: &Arc<Self>, actor: &A2aActor, params: &Value) -> Result<Value, RpcError> {
        let context = match params.get("contextId") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => return Err(RpcError::invalid_params("contextId", "must be a string")),
        };
        let state = match params.get("status") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if store::STATES.contains(&s.as_str()) => Some(s.clone()),
            Some(Value::String(s)) if s == "TASK_STATE_UNSPECIFIED" || s.is_empty() => None,
            Some(other) => return Err(RpcError::invalid_params("status", &format!("{other} isn't a task state; one of {}", store::STATES.join(", ")))),
        };
        let size = match params.get("pageSize") {
            None | Some(Value::Null) => 50,
            Some(n) => match n.as_i64() {
                Some(n @ 1..=100) => n,
                _ => return Err(RpcError::invalid_params("pageSize", &format!("must be between 1 and 100 inclusive, got {n}"))),
            },
        };
        let cursor = match params.get("pageToken") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(cursor_of(s).ok_or_else(|| RpcError::invalid_params("pageToken", "isn't a token XenoNet gave"))?),
            Some(_) => return Err(RpcError::invalid_params("pageToken", "must be a string")),
        };
        let after = match params.get("statusTimestampAfter") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(xenonet_base::time::parse_iso(s).ok_or_else(|| RpcError::invalid_params("statusTimestampAfter", "must be an ISO 8601 time"))?),
            Some(_) => return Err(RpcError::invalid_params("statusTimestampAfter", "must be an ISO 8601 time")),
        };
        let history = tasks::history_length(params.get("historyLength"), "historyLength")?;
        let artifacts = match params.get("includeArtifacts") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(RpcError::invalid_params("includeArtifacts", "must be a boolean")),
        };
        self.sync_now(&actor.id).await;
        let s = self.store.locked();
        let (mut rows, total) = s.list(&actor.id, &store::Filter { context, state, after, cursor, limit: size + 1 }).map_err(|e| db(&e))?;
        let more = rows.len() as i64 > size;
        rows.truncate(usize::try_from(size).unwrap_or(50));
        let next = if more { rows.last().map(|t| token_of(t.updated_at, t.seq)).unwrap_or_default() } else { String::new() };
        let mut list = Vec::new();
        for t in &rows {
            let r = s.replies(&t.id).map_err(|e| db(&e))?;
            list.push(tasks::task(t, &r, Shown { history, artifacts }));
        }
        Ok(json!({ "tasks": list, "nextPageToken": next, "pageSize": size, "totalSize": total }))
    }

    async fn cancel_task(self: &Arc<Self>, actor: &A2aActor, params: &Value) -> Result<Value, RpcError> {
        let id = Self::task_param(params)?;
        self.sync_now(&actor.id).await;
        let (t, _) = self.load(&actor.id, &id)?;
        if store::terminal(&t.state) {
            return Err(RpcError::not_cancelable(&id, &t.state));
        }
        self.store.locked().set_state(&id, store::CANCELED, Some("Canceled by the agent that sent it."), self.now()).map_err(|e| db(&e))?;
        self.bump(&actor.id);
        // The note goes in its own task, so a client that goes doesn't stop it.
        if let Some(message) = t.message {
            let me = self.clone();
            let actor = actor.clone();
            let to = match t.recipients.as_slice() {
                [one] => Some(one.clone()),
                _ => t.target.clone(),
            };
            let _ = tokio::spawn(async move { me.cancel_note(&actor, to, message).await }).await;
        }
        let (t, r) = self.load(&actor.id, &id)?;
        Ok(tasks::task(&t, &r, Shown::ALL))
    }

    /// The recipients hear that no answer is needed. It's a message, so the
    /// actor's limits count it, and past them it isn't sent.
    async fn cancel_note(&self, actor: &A2aActor, to: Option<String>, message: i64) {
        let Some(to) = to else { return };
        if self.take(actor, &to).is_some() {
            return;
        }
        let envelope = self.envelope(actor, &format!("msg_{}", random_uuid()));
        let text = "The agent that sent this canceled its request, so it needs no answer now.";
        let _ = self.cores.engine.send(&actor.id, json!({ "to": to, "text": text, "replyTo": message }), Some(json!({ "envelope": envelope }))).await;
    }

    // ---------- streams ----------

    fn send_streaming(self: &Arc<Self>, actor: &A2aActor, id: Value, params: &Value) -> Result<Reply, RpcError> {
        config_of(params)?;
        let start = match self.prepare(actor, params)? {
            Prepared::Existing(task) => Start::Follow(task),
            Prepared::New(p) => Start::Deliver(p),
        };
        Ok(self.stream(actor, id, start))
    }

    async fn subscribe(self: &Arc<Self>, actor: &A2aActor, id: Value, params: &Value) -> Result<Reply, RpcError> {
        let task = Self::task_param(params)?;
        self.sync_now(&actor.id).await;
        let (t, _) = self.load(&actor.id, &task)?;
        if store::terminal(&t.state) {
            return Err(RpcError::unsupported(&format!("the task is {}; GetTask reads it", t.state)));
        }
        Ok(self.stream(actor, id, Start::Follow(task)))
    }

    fn stream(self: &Arc<Self>, actor: &A2aActor, id: Value, start: Start) -> Reply {
        let (tx, rx) = mpsc::unbounded_channel();
        let me = self.clone();
        let actor = actor.clone();
        tokio::spawn(async move { me.run_stream(actor, id, start, tx).await });
        Reply { status: 200, headers: sse_headers(), body: ReplyBody::Stream(rx) }
    }

    async fn run_stream(self: Arc<Self>, actor: A2aActor, id: Value, start: Start, tx: mpsc::UnboundedSender<Bytes>) {
        let send = |v: Value| tx.send(sse_frame(&result(&id, v))).is_ok();
        let deadline = Instant::now() + self.limits.stream;
        // The task as it is first, then each change.
        let (task, pending) = match start {
            Start::Deliver(p) => (p.task.id.clone(), Some(p)),
            Start::Follow(task) => {
                self.sync_now(&actor.id).await;
                (task, None)
            }
        };
        let known = match self.load(&actor.id, &task) {
            Ok((t, r)) => {
                send(json!({ "task": tasks::task(&t, &r, Shown::ALL) }));
                (t.state, r.len())
            }
            Err(e) => {
                let _ = tx.send(sse_frame(&e.to_json(&id)));
                return;
            }
        };
        if let Some(p) = pending {
            self.deliver_detached(&actor, *p).await;
        } else if store::terminal(&known.0) {
            return;
        }
        self.follow(&actor, &task, known, deadline, &mut |event| match event {
            Some(v) => send(v),
            None => tx.send(Bytes::from_static(b": keep-alive\n\n")).is_ok(),
        })
        .await;
    }

    /// Follows a task until it ends, `deadline` passes, or `emit` says the
    /// listener went. `known` is the state and the number of replies the
    /// listener has; each change since is an event (replies as artifacts,
    /// then the status), and None asks for a keep-alive.
    async fn follow(self: &Arc<Self>, actor: &A2aActor, task: &str, known: (String, usize), deadline: Instant, emit: &mut (dyn FnMut(Option<Value>) -> bool + Send)) {
        let mut changes = self.watch(&actor.id);
        let (mut state, mut seen) = known;
        let mut said = Instant::now();
        loop {
            let Ok((t, r)) = self.load(&actor.id, task) else { return };
            for reply in r.iter().skip(seen) {
                if !emit(Some(tasks::artifact_event(&t, reply))) {
                    return;
                }
                said = Instant::now();
            }
            seen = r.len();
            if t.state != state {
                state.clone_from(&t.state);
                if !emit(Some(tasks::status_event(&t, &r))) {
                    return;
                }
                said = Instant::now();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if store::terminal(&t.state) || left.is_zero() {
                return;
            }
            self.ensure_pull(&actor.id, self.limits.pull.min(left));
            let quiet = (said + self.limits.keepalive).min(deadline);
            tokio::select! {
                _ = changes.changed() => {}
                () = tokio::time::sleep_until(quiet.into()) => {
                    if Instant::now() < deadline {
                        if !emit(None) {
                            return;
                        }
                        said = Instant::now();
                    }
                }
            }
        }
    }

    // ---------- a message going out ----------

    /// Checks a client's message and keeps its task as SUBMITTED, or finds
    /// the task it had already.
    fn prepare(&self, actor: &A2aActor, params: &Value) -> Result<Prepared, RpcError> {
        let inc = tasks::incoming(params)?;
        let t = self.now();
        let s = self.store.locked();
        if let Some(task) = &inc.task_id {
            return Err(match s.task(&actor.id, task).map_err(|e| db(&e))? {
                None => RpcError::task_not_found(task),
                Some(old) if store::terminal(&old.state) => RpcError::unsupported(&format!(
                    "the task is {}, so it takes no more messages; send a new message with contextId {} to go on in its thread",
                    old.state, old.context
                )),
                Some(old) => RpcError::unsupported(&format!("a XenoNet task is one message; send a new message with contextId {} to go on in its thread", old.context)),
            });
        }
        if let Some(old) = s.task_by_client_message(&actor.id, &inc.message_id).map_err(|e| db(&e))? {
            return Ok(Prepared::Existing(old.id));
        }
        let context = inc.context_id.clone().unwrap_or_else(|| format!("ctx_{}", random_uuid()));
        let thread = s.context(&actor.id, &context).map_err(|e| db(&e))?;
        let to = match (&inc.target, &thread) {
            (Some(target), _) => Some(tasks::engine_to(target)),
            (None, Some(c)) => c.last_sender.clone().or_else(|| c.target.clone()),
            (None, None) => s.default_target(&actor.id).map_err(|e| db(&e))?.map(|d| tasks::engine_to(&d)),
        };
        s.ensure_context(&actor.id, &context, t).map_err(|e| db(&e))?;
        let task = TaskRow {
            seq: 0,
            id: format!("msg_{}", random_uuid()),
            actor: actor.id.clone(),
            context,
            client_message: inc.message_id,
            message: None,
            target: to.clone(),
            recipients: Vec::new(),
            state: store::SUBMITTED.into(),
            reason: None,
            request: inc.message,
            created_at: t,
            updated_at: t,
        };
        s.insert_task(&task).map_err(|e| db(&e))?;
        Ok(Prepared::New(Box::new(Pending { task, text: inc.text, to, reply_to: thread.and_then(|c| c.last_message) })))
    }

    /// Sends a task's message in a task of its own: a client that goes
    /// away doesn't leave it half sent.
    async fn deliver_detached(self: &Arc<Self>, actor: &A2aActor, p: Pending) {
        let me = self.clone();
        let actor = actor.clone();
        let _ = tokio::spawn(async move { me.deliver(&actor, p).await }).await;
    }

    /// One more message from the actor, or how long until it may send it.
    fn take(&self, actor: &A2aActor, to: &str) -> Option<String> {
        let t = self.now();
        let mut b = self.buckets.locked();
        let wide = tasks::is_wide(to);
        let mut wait = if wide { b.take(&format!("{}|wide", actor.slug), WIDE, 1.0, t) } else { 0.0 };
        if wait == 0.0 {
            wait = b.take(&format!("{}|messages", actor.slug), MESSAGES, 1.0, t);
        }
        (wait != 0.0).then(|| {
            let why = if wide { "an actor sends at most 6 messages an hour to a repo, a selector or all" } else { "an actor sends at most 60 messages an hour" };
            format!("{why}; try again in {} s", (wait / 1000.0).ceil())
        })
    }

    fn envelope(&self, actor: &A2aActor, mid: &str) -> Value {
        let info = self.cores.actors.info(&actor.id);
        let name = info.as_ref().and_then(|i| i.get("name")).and_then(Value::as_str).unwrap_or(&actor.slug).to_string();
        let product = info.as_ref().and_then(|i| i.get("product")).and_then(|p| p.get("id")).and_then(Value::as_str).map(str::to_string);
        let mut env = attest(Sender { id: &actor.id, kind: "actor", name: Some(name), product }, "gateway", &Value::Null, &self.home, self.now(), platform());
        // The task is the message: its id is the message's mid.
        env["mid"] = json!(mid);
        env
    }

    async fn deliver(&self, actor: &A2aActor, p: Pending) {
        let id = p.task.id.clone();
        let refuse = |state: &str, why: &str| {
            let _ = self.store.locked().set_state(&id, state, Some(why), self.now());
            self.bump(&actor.id);
        };
        let Some(to) = p.to else {
            return refuse(
                store::REJECTED,
                "Say which session to reach: put its id (claude:…, codex:…) or repo:<name> in the message's metadata as \"target\", or ask your user to set a default target for this agent in XenoNet's Actors panel.",
            );
        };
        if !self.cores.actors.can(&actor.id, "message-agents") {
            return refuse(store::REJECTED, "Sending needs the message-agents capability, which your user didn't allow this agent.");
        }
        if let Some(why) = self.take(actor, &to) {
            return refuse(store::REJECTED, &why);
        }
        let mut msg = json!({ "to": to, "text": p.text });
        if let Some(r) = p.reply_to {
            msg["replyTo"] = json!(r);
        }
        let envelope = self.envelope(actor, &id);
        match self.cores.engine.send(&actor.id, msg, Some(json!({ "envelope": envelope }))).await {
            Ok(v) => {
                let Some(message) = int(&v["id"]) else {
                    return refuse(store::FAILED, "XenoNet's engine didn't say it kept the message.");
                };
                let recipients: Vec<String> = match &v["to"] {
                    Value::Array(l) => l.iter().filter_map(|r| r.get("id").and_then(Value::as_str).or_else(|| r.as_str())).map(str::to_string).collect(),
                    Value::String(s) => vec![s.clone()],
                    _ => Vec::new(),
                };
                let t = self.now();
                {
                    let s = self.store.locked();
                    let _ = s.set_sent(&id, message, &to, &recipients, t);
                    let _ = s.context_sent(&actor.id, &p.task.context, &to, message, t);
                }
                self.bump(&actor.id);
            }
            Err(e) if e.is_pending() => refuse(store::FAILED, "XenoNet's engine isn't running in this service, so nothing was sent."),
            Err(e) => refuse(store::REJECTED, &e.message),
        }
    }

    // ---------- replies coming in ----------

    fn watch(&self, actor: &str) -> watch::Receiver<u64> {
        self.watches.locked().entry(actor.to_string()).or_insert_with(|| watch::channel(0).0).subscribe()
    }

    /// Something of the actor's tasks changed: whoever follows them looks.
    fn bump(&self, actor: &str) {
        if let Some(w) = self.watches.locked().get(actor) {
            w.send_modify(|n| *n += 1);
        }
    }

    /// A pull of the actor's inbox, unless one is on already, waiting up to
    /// `wait` for a message. It always runs to the end and keeps what it
    /// took, then tells the followers.
    fn ensure_pull(self: &Arc<Self>, actor: &str, wait: Duration) -> Option<tokio::task::JoinHandle<()>> {
        if !self.pulling.locked().insert(actor.to_string()) {
            return None;
        }
        let me = self.clone();
        let actor = actor.to_string();
        Some(tokio::spawn(async move {
            let started = Instant::now();
            match me.cores.engine.inbox(&actor, wait).await {
                Ok(v) => {
                    let list = messages_of(&v);
                    if list.is_empty() && wait > Duration::ZERO {
                        // Another reader took them, or the engine came back
                        // early: no spinning.
                        let left = wait.saturating_sub(started.elapsed());
                        tokio::time::sleep(left.min(Duration::from_secs(1))).await;
                    }
                    me.apply(&actor, &list);
                }
                Err(_) => tokio::time::sleep(me.limits.backoff).await,
            }
            me.pulling.locked().remove(&actor);
            me.bump(&actor);
        }))
    }

    /// What's in the actor's inbox now, kept, before an answer that shows
    /// its tasks. A pull that's on already will see anything new.
    async fn sync_now(self: &Arc<Self>, actor: &str) {
        if let Some(h) = self.ensure_pull(actor, Duration::ZERO) {
            let _ = h.await;
        }
    }

    /// Messages from the actor's inbox, each to the task it answers: the
    /// task whose message it replies to, the open task in that thread, or
    /// the oldest open task that went to its sender. One that answers none
    /// joins the latest task that went to its sender, or is kept aside.
    fn apply(&self, actor: &str, list: &[Value]) {
        let s = self.store.locked();
        let now = self.now();
        for m in list {
            let Some(message) = int(&m["id"]) else { continue };
            let sender = m["from"]["id"].as_str().or_else(|| m["from"].as_str()).map(str::to_string);
            let name = m["from"]["title"].as_str().or_else(|| m["from"]["name"].as_str()).filter(|n| !n.is_empty()).map(str::to_string);
            let text = xenonet_base::text::js_string(&m["text"]);
            let at = m["at"].as_f64().unwrap_or(now);
            let found = (|| -> rusqlite::Result<Option<TaskRow>> {
                if let Some(r) = int(&m["replyTo"]) {
                    if let Some(t) = s.task_by_message(actor, r)? {
                        return Ok(Some(t));
                    }
                    if let Some(t) = s.task_by_reply(actor, r)? {
                        return Ok(Some(s.open_in_context(actor, &t.context)?.unwrap_or(t)));
                    }
                }
                let Some(from) = &sender else { return Ok(None) };
                Ok(match s.open_to(actor, from)? {
                    Some(t) => Some(t),
                    None => s.latest_to(actor, from)?,
                })
            })();
            match found {
                Ok(Some(t)) => {
                    let reply = ReplyRow { message, mid: m["envelope"]["mid"].as_str().map(str::to_string), sender: sender.clone(), sender_name: name, text, at };
                    if s.add_reply(&t.id, &reply).unwrap_or(false) && !store::terminal(&t.state) {
                        let _ = s.set_state(&t.id, store::COMPLETED, None, now);
                    }
                    let _ = s.context_heard(actor, &t.context, message, sender.as_deref(), now);
                }
                Ok(None) => {
                    let _ = s.add_loose(actor, message, sender.as_deref(), &text, at);
                }
                Err(_) => {}
            }
        }
    }
}

enum Start {
    Deliver(Box<Pending>),
    Follow(String),
}

/// A page's token: where the last task shown sorts.
fn token_of(at: f64, seq: i64) -> String {
    xenonet_base::jsstr::hex(format!("{}:{seq}", at as i64).as_bytes())
}

fn cursor_of(token: &str) -> Option<(f64, i64)> {
    if !token.len().is_multiple_of(2) || token.len() > 80 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..token.len()).step_by(2).map(|i| u8::from_str_radix(token.get(i..i + 2)?, 16).ok()).collect();
    let text = String::from_utf8(bytes?).ok()?;
    let (at, seq) = text.split_once(':')?;
    Some((at.parse::<i64>().ok()? as f64, seq.parse().ok()?))
}

/// The network's name, or this host's id in it, from
/// `<state>/network/network.json`; None outside a network.
pub fn network_name(state_dir: &str) -> Option<String> {
    let text = std::fs::read_to_string(std::path::Path::new(state_dir).join("network").join("network.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let pick = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty() && s.len() <= 64).map(str::to_string);
    pick("name").or_else(|| pick("host"))
}

#[cfg(test)]
mod tests;
