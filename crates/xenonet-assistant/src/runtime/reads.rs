//! What a remote actor reads and how (crates/xenonet-assistant): the view
//! of XenoNet's own state, the six read-only operations with their checks,
//! paging, shape, size and the four-at-a-time limit, the grant's
//! standing and rate, the audit of every read, and what a read says of its
//! actor on the map. The made-up reads (a grant you haven't verified, and
//! the pairing's preview) are the fixtures'. A read's page cursors are the
//! token it came in with, narrowed (`pages`).

use super::connections::{EngineRows, Remote};
use crate::pages::{Granted, Standalone};
use crate::service::{self, Config, Inflight, Pages, ReadHost, LIMITS};
use crate::{audit, fixtures, projection, source};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_actors::adapters::{ADAPTERS, TRANSPORTS};
use xenonet_actors::connections::Store as _;
use xenonet_actors::remote_presence::{self, RemotePresence};
use xenonet_api::cores::{Actors, AssistantService, AssistantSource, Audit, CoreError, EngineStore, Grants, State, R};
use xenonet_api::limits::Buckets;
use xenonet_api::wire::Emitter;
use xenonet_os::error::Failure;
use xenonet_os::fx;
use xenonet_os::sync::Locked;
use xenonet_schema::typed;
use xenonet_text::time::now_ms;

pub const RESULT_SCHEMA: &str = "xenonet.assistant.result.v1";
pub const ERROR_SCHEMA: &str = "xenonet.assistant.error.v1";
const OPERATIONS: [(&str, &str); 6] = [
    ("xenonet_status", "status"),
    ("xenonet_projects", "projects"),
    ("xenonet_sessions", "sessions"),
    ("xenonet_attention", "attention"),
    ("xenonet_session", "session"),
    ("xenonet_actors", "actors"),
];
const ATTENTION_KINDS: [&str; 4] = ["waiting", "stalled", "collision", "turn_completed"];
const STATUSES: [&str; 6] = ["working", "waiting", "idle", "completed", "stuck", "unknown"];
// A grant's rate: one read a second, ten at once.
const RATE: (f64, f64) = (1.0, 10.0);

fn config(version: &str) -> Config {
    Config {
        operations: OPERATIONS.iter().map(|(t, o)| (t.to_string(), o.to_string())).collect(),
        attention_kinds: ATTENTION_KINDS.iter().map(ToString::to_string).collect(),
        result_schema: RESULT_SCHEMA.into(),
        error_schema: ERROR_SCHEMA.into(),
        version: version.into(),
        limits: LIMITS,
    }
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("a pattern of the assistant's contract"))
}

const UUID: &str = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";

// A field of a tool's arguments, and its check.
type Field<'a> = (&'a str, &'a dyn Fn(&Value) -> bool);

/// `INPUTS[tool].safeParse(args)` (core/schema/assistant.js): each tool's
/// arguments, a strict object of the fields it names.
pub fn input_ok(tool: &str, args: &Value) -> bool {
    static CURSOR: OnceLock<Regex> = OnceLock::new();
    static PROJECT: OnceLock<Regex> = OnceLock::new();
    static SESSION: OnceLock<Regex> = OnceLock::new();
    static ISO: OnceLock<Regex> = OnceLock::new();
    let Some(o) = args.as_object() else { return false };
    let cursor = |v: &Value| v.as_str().is_some_and(|s| re(&CURSOR, r"^xenonet_actor_[A-Za-z0-9_-]{64,4096}$").is_match(s));
    let limit = |v: &Value| v.as_f64().is_some_and(|n| n.fract() == 0.0 && (1.0..=50.0).contains(&n));
    let project = |v: &Value| v.as_str().is_some_and(|s| re(&PROJECT, xenonet_model::assistant::PROJECT_ID).is_match(s));
    let session = |v: &Value| v.as_str().is_some_and(|s| re(&SESSION, &format!("^(claude|codex|grok):{UUID}$")).is_match(s));
    let iso = |v: &Value| v.as_str().is_some_and(|s| re(&ISO, r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?Z$").is_match(s));
    let list_of =
        |v: &Value, allowed: &[&str]| v.as_array().is_some_and(|a| !a.is_empty() && a.len() <= allowed.len() && a.iter().all(|x| x.as_str().is_some_and(|s| allowed.contains(&s))));
    let fields: &[Field<'_>] = match tool {
        "xenonet_status" => &[],
        "xenonet_projects" => &[("cursor", &cursor), ("limit", &limit)],
        "xenonet_sessions" => &[("project_id", &project), ("statuses", &|v| list_of(v, &STATUSES)), ("updated_since", &iso), ("cursor", &cursor), ("limit", &limit)],
        "xenonet_attention" => &[("project_id", &project), ("kinds", &|v| list_of(v, &ATTENTION_KINDS)), ("cursor", &cursor), ("limit", &limit)],
        "xenonet_session" => &[("session_id", &session), ("include_stats", &|v| v.is_boolean())],
        "xenonet_actors" => &[("project_id", &project), ("cursor", &cursor), ("limit", &limit)],
        _ => return false,
    };
    if tool == "xenonet_session" && !o.contains_key("session_id") {
        return false;
    }
    o.iter().all(|(k, v)| fields.iter().find(|(f, _)| f == k).is_some_and(|(_, ok)| ok(v)))
}

/// The answer against the contract's: an assistant/result/v1, read as its
/// type.
fn result_ok(envelope: &Value) -> bool {
    typed::fits::<xenonet_model::assistant::Answer>(envelope)
}

/// What answering a read needs of the world: the view, an actor, a
/// session's stats and the key for ids.
#[derive(Clone)]
enum World {
    Real(Arc<Reads>),
    Fixture,
}

/// Where a read's page cursors come from: the grant it came in with (its
/// text, and its actor's slug), or the made-up reads' own.
enum Cursors {
    Granted(Arc<dyn Grants>, String, String),
    Standalone(Arc<Standalone>),
}

/// One read to answer: its operation and tool, its arguments, the grant's
/// policy and the request's id.
struct Asked {
    op: String,
    tool: String,
    args: Value,
    policy: Value,
    request_id: Value,
}

/// What an answer reads of the world, on the thread the answers run on;
/// a session's stats are read on the service's runtime.
struct Here {
    world: World,
    rt: tokio::runtime::Handle,
}

fn failure(e: CoreError) -> Failure {
    Failure::msg(e.message)
}

#[async_trait(?Send)]
impl ReadHost for Here {
    async fn now(&self) -> f64 {
        now_ms()
    }
    async fn view(&self) -> Result<Value, Failure> {
        match &self.world {
            World::Real(r) => r.view().map_err(failure),
            World::Fixture => Ok(fixtures::fixture_view(now_ms())),
        }
    }
    async fn actor(&self, id: &Value) -> Result<Value, Failure> {
        Ok(match &self.world {
            World::Real(r) => r.actor(id),
            World::Fixture => Value::Null,
        })
    }
    async fn stats(&self, id: &str) -> Result<Value, Failure> {
        match &self.world {
            World::Real(r) => {
                let (state, id) = (r.state.clone(), id.to_string());
                let read = self.rt.spawn(async move { state.session_stats(&id).await.map(|s| s.unwrap_or(Value::Null)).map_err(failure) });
                read.await.unwrap_or_else(|e| Err(Failure::msg(format!("a request failed: {e}"))))
            }
            World::Fixture => Ok(fixtures::fixture_stats(id)),
        }
    }
    async fn id_key(&self) -> Result<Vec<u8>, Failure> {
        match &self.world {
            World::Real(r) => {
                let k = r.remote.as_ref().map(|c| c.id_key()).transpose().map_err(failure)?.unwrap_or(Value::Null);
                Ok(xenonet_text::text::display(&k).into_bytes())
            }
            World::Fixture => Ok(b"fixture".to_vec()),
        }
    }
    async fn check(&self, _tool: &str, envelope: &Value) -> Result<bool, Failure> {
        Ok(result_ok(envelope))
    }
}

/// One answer: its envelope and what it read, or its refusal (`{ refused,
/// code, reason }`) or what it failed with.
async fn answer(world: World, version: String, cursors: Cursors, a: Asked) -> Result<Value, Value> {
    let Asked { op, tool, args, policy, request_id } = a;
    let rt = tokio::runtime::Handle::current();
    let done = fx::local(move || async move {
        let cfg = config(&version);
        let granted;
        let pages: &dyn Pages = match &cursors {
            Cursors::Granted(grants, token, slug) => {
                granted = Granted { grants: grants.as_ref(), token, slug };
                &granted
            }
            Cursors::Standalone(s) => s.as_ref(),
        };
        service::answer(&Here { world, rt }, &cfg, pages, &op, &tool, &args, &policy, &request_id).await
    })
    .await;
    match done {
        Some(Ok(v)) => Ok(v),
        Some(Err(f)) => Err(f.to_value()),
        None => Err(fx::error("a read stopped before it was done", None)),
    }
}

/// What a read refused for: its code, and the log's reason.
fn refusal(e: &Value) -> Option<(String, String)> {
    (e.get("refused") == Some(&json!(true))).then(|| (xenonet_text::text::display(&e["code"]), e.get("reason").map(xenonet_text::text::display).unwrap_or_default()))
}

/// The reads: the source's view, the grants' reads, their audit.
pub struct Reads {
    pub remote: Option<Arc<Remote>>,
    pub state: Arc<dyn State>,
    pub actors: Option<Arc<dyn Actors>>,
    pub engine_store: Arc<dyn EngineStore>,
    pub store: Option<xenonet_store::engine::Shared>,
    pub version: String,
    pub emitter: Emitter,
    /// The made-up reads' page cursors, for the pairing's preview.
    standalone: OnceLock<Arc<Standalone>>,
    view: Mutex<Option<(f64, Value)>>,
    buckets: Mutex<Buckets>,
    inflight: Arc<Mutex<Inflight>>,
    presence: Mutex<RemotePresence>,
}

// The registry, as a remote actor's reads put it to it, and who shows the
// reads.
struct Presence<'a> {
    r: &'a Reads,
}

// What the actors failed with, as the presence core hears it.
fn actor_error(e: CoreError) -> xenonet_actors::store::Error {
    xenonet_actors::store::Error { message: e.message, code: e.code }
}

impl remote_presence::Host for Presence<'_> {
    fn has(&mut self, id: &str) -> bool {
        self.r.actors.as_ref().is_some_and(|a| a.has(id))
    }
    fn report(&mut self, id: &str, report: &Value) -> Result<(), xenonet_actors::store::Error> {
        self.r.actors.as_ref().map_or(Ok(()), |a| a.report(id, report).map_err(actor_error))
    }
    fn target(&mut self, id: &str, node: &str, relation: &str, at: f64) -> Result<(), xenonet_actors::store::Error> {
        self.r.actors.as_ref().map_or(Ok(()), |a| a.target_at(id, node, relation, at).map_err(actor_error))
    }
    fn on_read(&mut self, read: Value) {
        let connection = xenonet_text::text::display(&read["connection"]);
        let transport = self.r.engine_store.connection(&connection).ok().flatten().and_then(|c| c.get("transport_kind").cloned()).unwrap_or(Value::Null);
        if let Some(sink) = self.r.emitter.sink() {
            sink.broadcast("remote-read", &json!({ "actor": read["actor"], "transport": transport, "reach": read["reach"], "at": read["at"] }));
        }
    }
}

struct Around<'a> {
    state: &'a dyn State,
    actors: Option<&'a Arc<dyn Actors>>,
}

impl source::Around for Around<'_> {
    fn home_of(&mut self, id: &Value) -> Value {
        self.state.home_of(&xenonet_text::text::display(id)).unwrap_or(Value::Null)
    }
    fn product_of(&mut self, id: Option<&Value>) -> Value {
        product(id)
    }
    fn registered(&mut self, id: &Value) -> bool {
        self.actors.is_some_and(|a| a.is_registered(&xenonet_text::text::display(id)))
    }
}

// productOf(id)?.id ?? null.
fn product(id: Option<&Value>) -> Value {
    id.and_then(Value::as_str).and_then(xenonet_adapter::products::product_of).map_or(Value::Null, |p| json!(p.id))
}

impl Reads {
    pub fn new(
        remote: Option<Arc<Remote>>,
        state: Arc<dyn State>,
        actors: Option<Arc<dyn Actors>>,
        engine_store: Arc<dyn EngineStore>,
        store: Option<xenonet_store::engine::Shared>,
        version: &str,
        emitter: Emitter,
    ) -> Arc<Self> {
        Arc::new(Reads {
            remote,
            state,
            actors,
            engine_store,
            store,
            version: version.into(),
            emitter,
            standalone: OnceLock::new(),
            view: Mutex::new(None),
            buckets: Mutex::new(Buckets::new(1024)),
            inflight: Arc::new(Mutex::new(Inflight::default())),
            presence: Mutex::new(RemotePresence::default()),
        })
    }

    /// stateSource's view: the map, the live state, the actors and the two
    /// apps' sources, kept for a second.
    pub fn view(&self) -> R<Value> {
        let t = now_ms();
        if let Some((at, v)) = self.view.locked().clone() {
            if t - at < source::VIEW_MS {
                return Ok(v);
            }
        }
        let live = self.state.live_snapshot()?;
        let nodes = self.state.nodes()?;
        let list = match &self.actors {
            Some(a) => a.list()?.get("actors").and_then(Value::as_array).cloned().unwrap_or_default(),
            None => Vec::new(),
        };
        let sources = self.state.assistant_sources(t)?;
        let mut around = Around { state: self.state.as_ref(), actors: self.actors.as_ref() };
        let v = source::view(
            t,
            &nodes,
            &live,
            &list,
            self.actors.is_some(),
            projection::all_sources()
                .into_iter()
                .map(|app| {
                    let s = sources.get(&app).cloned().unwrap_or(Value::Null);
                    (app, s)
                })
                .collect(),
            &mut around,
        );
        *self.view.locked() = Some((t, v.clone()));
        Ok(v)
    }

    fn actor(&self, id: &Value) -> Value {
        let Some(a) = &self.actors else { return Value::Null };
        match a.snapshot(&xenonet_text::text::display(id), None, false) {
            Ok(Some(snap)) => {
                let p = product(snap.get("product").and_then(|p| p.get("id")));
                source::actor(&snap, p)
            }
            _ => Value::Null,
        }
    }

    fn log(&self, record: Value) {
        if let Some(s) = &self.store {
            let row = audit::access(&record, now_ms());
            let _ = s.with(|st| st.add_audit(&row));
        }
    }

    /// service.read(session, request): the read, or the error envelope.
    pub async fn read(self: &Arc<Self>, session: &Value, request: &Value) -> Value {
        let started = now_ms();
        let cfg = config(&self.version);
        let b = service::begin(&cfg, request);
        let grant = xenonet_text::text::display(&session["grantId"]);
        let request_id = b.request_id.clone();
        let op = b.op.clone().unwrap_or_else(|| "unknown".into());
        let log = |result: &str, extra: Map<String, Value>| {
            let mut rec = json!({ "requestId": request_id, "grant": session["grantId"], "revision": extra.get("revision").cloned().unwrap_or(Value::Null), "actor": session["actorId"], "connection": session["connectionId"], "operation": op, "result": result, "latencyMs": xenonet_text::json::number(now_ms() - started) });
            if let Value::Object(m) = &mut rec {
                for (k, v) in extra {
                    m.insert(k, v);
                }
            }
            self.log(rec);
        };
        let no = |code: &str, reason: &str, extra: Map<String, Value>| {
            let retry = extra.get("retryAfterMs").and_then(Value::as_f64);
            log(if reason.is_empty() { code } else { reason }, extra);
            json!({ "ok": false, "error": service::error_envelope(ERROR_SCHEMA, Some(&request_id), code, retry) })
        };
        let extra = |pairs: &[(&str, Value)]| -> Map<String, Value> { pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
        if let Some(n) = &b.no {
            return no(n.code, n.reason, Map::new());
        }
        let (Some(op_s), Some(tool)) = (b.op.clone(), b.tool.clone()) else { return no("INVALID_ARGUMENT", "bad-request", Map::new()) };
        let Some(remote) = self.remote.clone() else { return no("SERVICE_UNAVAILABLE", "no-connections", Map::new()) };
        // The token the adapter said hello with, which its page cursors
        // narrow.
        let token = session["token"].as_str().unwrap_or_default().to_string();
        let slug = xenonet_text::text::display(&session["actorId"]).trim_start_matches("actor:").to_string();
        let mut standing = remote.recheck_at(&session["grantId"], &json!(-1));
        if standing["ok"] != json!(true) {
            return no(&xenonet_text::text::display(&standing["code"]), &xenonet_text::text::display(&standing["reason"]), Map::new());
        }
        let wait = self.buckets.locked().take(&grant, RATE.0, RATE.1, 1.0, now_ms());
        if wait != 0.0 {
            return no("RATE_LIMITED", "rate", extra(&[("retryAfterMs", xenonet_text::json::number(wait))]));
        }
        if !self.inflight.locked().enter(&grant, cfg.limits.concurrent) {
            return no("RATE_LIMITED", "concurrency", extra(&[("retryAfterMs", json!(1000))]));
        }
        // The slot is held until the work under it ends, even after a
        // timeout has answered.
        let mut work: Option<tokio::task::JoinHandle<Result<Value, Value>>> = None;
        let out = async {
            if !input_ok(&tool, &b.args) {
                return no("INVALID_ARGUMENT", "bad-arguments", Map::new());
            }
            for _ in 0..2 {
                let Ok(policy) = remote.policy_of(&standing) else { return no("INTERNAL_ERROR", "exception", Map::new()) };
                let running = tokio::spawn(answer(
                    World::Real(self.clone()),
                    self.version.clone(),
                    Cursors::Granted(remote.grants.clone(), token.clone(), slug.clone()),
                    Asked { op: op_s.clone(), tool: tool.clone(), args: b.args.clone(), policy: policy.clone(), request_id: request_id.clone() },
                ));
                let got = {
                    let mut handle = running;
                    match tokio::time::timeout(Duration::from_millis(cfg.limits.timeout_ms as u64), &mut handle).await {
                        Ok(r) => r.unwrap_or_else(|e| Err(fx::error(e.to_string(), None))),
                        Err(_) => {
                            work = Some(handle);
                            return no("TIMEOUT", "timeout", Map::new());
                        }
                    }
                };
                let out = match got {
                    Ok(o) => o,
                    Err(e) => {
                        return match refusal(&e) {
                            Some((code, reason)) => no(&code, &reason, Map::new()),
                            None => no("INTERNAL_ERROR", "exception", Map::new()),
                        }
                    }
                };
                // Once more before it goes: a grant revoked, expired or turned
                // off meanwhile gets nothing, and one whose scope changed reads
                // again.
                let again = remote.recheck_at(&session["grantId"], &policy["revision"]);
                if again["ok"] != json!(true) {
                    return no(&xenonet_text::text::display(&again["code"]), &xenonet_text::text::display(&again["reason"]), extra(&[("revision", policy["revision"].clone())]));
                }
                if again["changed"] == json!(true) {
                    standing = again;
                    continue;
                }
                remote.used(&grant);
                let targets: Vec<Value> = out["targets"].as_array().cloned().unwrap_or_default();
                let reach: Vec<Value> = out["reach"].as_array().cloned().unwrap_or_default();
                let (actor, connection) = (xenonet_text::text::display(&session["actorId"]), xenonet_text::text::display(&session["connectionId"]));
                let _ = self.presence.locked().read(&mut Presence { r: self }, &actor, &connection, &targets, &reach, now_ms(), true);
                log("ok", extra(&[("revision", policy["revision"].clone()), ("bytes", out["bytes"].clone()), ("projects", out["projects"].clone())]));
                return json!({ "ok": true, "result": out["envelope"] });
            }
            no("SERVICE_UNAVAILABLE", "scope-kept-changing", Map::new())
        }
        .await;
        match work {
            Some(handle) => {
                let (inflight, g) = (self.inflight.clone(), grant.clone());
                tokio::spawn(async move {
                    let _ = handle.await;
                    inflight.locked().leave(&g);
                });
            }
            None => self.inflight.locked().leave(&grant),
        }
        out
    }

    /// The fixture service's read (createFixtureService): made-up data,
    /// for the pairing's preview.
    pub async fn fixture_read(&self, operation: &str, labels: bool) -> Value {
        let request_id = json!(format!("r_{}", xenonet_os::random_uuid()));
        let cfg = config(&self.version);
        let b = service::begin(&cfg, &json!({ "request_id": request_id, "operation": operation, "arguments": {} }));
        let (Some(op), Some(tool), None) = (b.op.clone(), b.tool.clone(), b.no.as_ref()) else { return Value::Null };
        let policy = fixtures::standalone_policy(labels);
        let standalone = self.standalone.get_or_init(|| Arc::new(Standalone::new(xenonet_text::text::display(&policy["actorId"]).trim_start_matches("actor:")))).clone();
        match answer(World::Fixture, self.version.clone(), Cursors::Standalone(standalone), Asked { op, tool, args: b.args.clone(), policy, request_id }).await {
            Ok(out) => out["envelope"].get("data").cloned().unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    }

    /// What a grant with these projects and labels would send: the
    /// made-up answers first, then xenonet_projects and xenonet_sessions as
    /// they'd read the map now, under the store's own project ids. A place
    /// that gives rights in no project (a candidate checkout) reads
    /// nothing, as it would in the grant.
    pub async fn preview(&self, body: &Value, known: &Value) -> R<Value> {
        let mut chosen: Vec<String> = Vec::new();
        for p in body.get("projects").and_then(Value::as_array).into_iter().flatten() {
            let Some(p) = p.as_str() else { continue };
            if known.get(p).is_some() && !chosen.iter().any(|c| c == p) {
                chosen.push(p.to_string());
            }
        }
        chosen.truncate(64);
        let labels = body.get("labels") == Some(&json!(true));
        let mut synthetic = Map::new();
        for op in ["projects", "sessions", "attention"] {
            synthetic.insert(op.into(), self.fixture_read(op, labels).await);
        }
        let rows = self.store.clone().map(EngineRows);
        let mut places: Vec<(String, String)> = Vec::new();
        for place in chosen {
            let id = match &rows {
                Some(r) => r.project_of(&json!(place)).map_err(|e| CoreError::new(e.message))?,
                None => Value::Null,
            };
            if let Some(id) = id.as_str() {
                places.push((id.to_string(), place));
            }
        }
        let mut ids: Vec<&str> = Vec::new();
        for (p, _) in &places {
            if !ids.contains(&p.as_str()) {
                ids.push(p);
            }
        }
        let policy = json!({ "labels": labels, "lookbackMs": projection::WINDOW_MS, "places": places.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>() });
        let t = now_ms();
        let view = self.view()?;
        let mut visible = projection::visible_sessions(&view, &policy, t);
        let mut counts: Vec<(String, Map<String, Value>)> = ids.iter().map(|p| ((*p).to_string(), projection::empty_counts())).collect();
        for v in &visible {
            let project = xenonet_text::text::display(&v.project);
            if let Some((_, c)) = counts.iter_mut().find(|(p, _)| *p == project) {
                let n = c.get(v.status).and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
                c.insert(v.status.into(), xenonet_text::json::number(n));
            }
        }
        let place_name = |place: &str| -> Option<String> {
            view.get("places")
                .and_then(Value::as_array)
                .and_then(|pairs| pairs.iter().find(|p| p.get(0).and_then(Value::as_str) == Some(place)))
                .and_then(|p| p.get(1))
                .and_then(|p| p.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let projects: Vec<Value> = ids
            .iter()
            .map(|p| {
                let place = places.iter().find(|(k, _)| k.as_str() == *p).map_or("", |(_, v)| v.as_str());
                let label = if labels { place_name(place).and_then(|n| projection::clean_label(&n)).map_or(Value::Null, Value::String) } else { Value::Null };
                json!({ "project_id": p, "label": label, "session_counts": counts.iter().find(|(k, _)| k == p).map_or_else(|| json!({}), |(_, c)| Value::Object(c.clone())) })
            })
            .collect();
        visible.sort_by(|a, b| {
            let u = |v: &projection::Visible<'_>| v.s.get("updatedAt").and_then(Value::as_f64).unwrap_or(0.0);
            u(b).total_cmp(&u(a))
        });
        let sessions: Vec<Value> = visible.iter().take(20).map(|v| projection::session_summary(v, &policy)).collect();
        let yours: Vec<Value> = places
            .iter()
            .map(|(p, place)| json!({ "project_id": p, "place": place, "name": known.get(place).and_then(|k| k.get("name")).cloned().unwrap_or(Value::Null) }))
            .collect();
        Ok(json!({ "at": xenonet_text::time::iso(t), "labels": labels, "synthetic": synthetic, "real": { "projects": projects, "sessions": sessions }, "yours": yours }))
    }
}

/// The products that can be paired (core/actors/adapters.js, products.js).
pub fn products() -> Value {
    Value::Array(
        ADAPTERS
            .iter()
            .map(|a| {
                let name = a.product.and_then(xenonet_adapter::products::product_of).map_or("An actor of your own", |p| p.name);
                let transports: Vec<Value> = a.transports.iter().map(|t| json!({ "id": t, "name": TRANSPORTS.iter().find(|x| x.id == *t).map(|x| x.name) })).collect();
                json!({ "key": a.key, "product": a.product, "name": name, "transports": transports, "support": a.support })
            })
            .collect(),
    )
}

pub struct SourceCore(pub Arc<Reads>);

#[async_trait]
impl AssistantSource for SourceCore {
    fn products(&self) -> R<Value> {
        Ok(products())
    }

    fn sources_data(&self) -> R<Value> {
        Ok(projection::sources_data(&self.0.view()?))
    }

    async fn preview(&self, body: &Value, known: &Value) -> R<Value> {
        self.0.preview(body, known).await
    }
}

pub struct ServiceCore(pub Arc<Reads>);

#[async_trait]
impl AssistantService for ServiceCore {
    async fn read(&self, session: &Value, params: &Value) -> R<Value> {
        Ok(self.0.read(session, params).await)
    }

    fn forget(&self, grant: &str) {
        self.0.buckets.locked().forget(grant);
    }
}

/// The log of what remote actors read (core/assistant/audit.js), in the
/// engine's store.
pub struct AuditCore(pub Arc<Reads>);

impl Audit for AuditCore {
    fn recent(&self, grant: Option<&str>, limit: f64) -> R<Value> {
        let Some(s) = &self.0.store else { return Ok(json!([])) };
        let rows = s.with(|st| st.audit(now_ms() - 7.0 * 24.0 * 3600e3, audit::limit_of(limit), grant)).map_err(|e| CoreError::new(e.to_string()))?;
        let rows: Vec<Value> = rows.iter().map(super::row_json).collect();
        Ok(Value::Array(audit::recent(&rows)))
    }

    fn access(&self, record: Value) {
        self.0.log(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_api::cores::Pending;

    // test/assistant-service.test.js's argument cases.
    #[test]
    fn each_tools_arguments_as_the_contract_has_them() {
        assert!(input_ok("xenonet_status", &json!({})));
        assert!(!input_ok("xenonet_status", &json!({ "x": 1 })), "strict");
        assert!(input_ok("xenonet_sessions", &json!({ "statuses": ["working"], "limit": 5, "updated_since": "2026-10-01T00:00:00Z" })));
        assert!(!input_ok("xenonet_sessions", &json!({ "statuses": [] })));
        assert!(!input_ok("xenonet_sessions", &json!({ "limit": 51 })));
        assert!(!input_ok("xenonet_session", &json!({})), "a session's id is required");
        assert!(input_ok("xenonet_session", &json!({ "session_id": "claude:0190fa00-0000-7000-8000-000000000001", "include_stats": true })));
        assert!(!input_ok("xenonet_projects", &json!({ "cursor": "nope" })));
        assert!(!input_ok("nope", &json!({})));
    }

    // The preview's made-up answers, read as a fixture grant reads them,
    // each fitting its contract.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_fixtures_answer_each_operation() {
        let r = Reads::new(None, Arc::new(Pending), None, Arc::new(Pending), None, "0.0.0", Emitter::new());
        for op in ["projects", "sessions", "attention"] {
            let data = r.fixture_read(op, true).await;
            assert!(!data.is_null(), "{op}");
        }
        assert_eq!(products().as_array().unwrap().len(), ADAPTERS.len());
        assert_eq!(products()[0]["transports"][0]["name"], "OpenAI Secure MCP Tunnel");
    }
}
