//! What a remote actor reads and how (core/assistant/source.js,
//! projection.js, service.js, fixtures.js, audit.js; crates/xenonet-assistant;
//! core/actors/remote-presence.js): the view of XenoNet's own state, the six
//! read-only operations with their checks, paging, shape, size and the
//! four-at-a-time limit (the Rust's, each answer a task), the grant's
//! standing and rate, the audit of every read, and what a read says of its
//! actor on the map. The made-up reads (a grant you haven't verified, and
//! the pairing's preview) are the fixtures'.

use super::arg;
use super::connections::Remote;
use crate::fx::{self, now_ms, Locked, Perform};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_actors::adapters::{ADAPTERS, TRANSPORTS};
use xenonet_actors::limits::Buckets;
use xenonet_actors::remote_presence::RemotePresence;
use xenonet_assistant::cursors::Cursors;
use xenonet_assistant::service::{self, Config, Inflight, LIMITS};
use xenonet_assistant::{audit, fixtures, projection, source};
use xenonet_engine::peer::{Calls, Result as CallResult, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_schema::check::{compile, Checker, Data};
use xenonet_service::cores::{Actors, AssistantService, AssistantSource, Audit, CoreError, EngineStore, State, R};
use xenonet_service::wire::Emitter;

pub const RESULT_SCHEMA: &str = "xenonet.assistant.result.v1";
pub const ERROR_SCHEMA: &str = "xenonet.assistant.error.v1";
const RESULT_DOC: &str = include_str!("../../../../schemas/assistant/result/v1.json");
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
    let cursor = |v: &Value| v.as_str().is_some_and(|s| re(&CURSOR, r"^[A-Za-z0-9_-]{1,900}\.[A-Za-z0-9_-]{43}$").is_match(s));
    let limit = |v: &Value| v.as_f64().is_some_and(|n| n.fract() == 0.0 && (1.0..=50.0).contains(&n));
    let project = |v: &Value| v.as_str().is_some_and(|s| re(&PROJECT, &format!("^prj_{UUID}$")).is_match(s));
    let session = |v: &Value| v.as_str().is_some_and(|s| re(&SESSION, &format!("^(claude|codex):{UUID}$")).is_match(s));
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

fn result_checker() -> Option<&'static Checker> {
    static C: OnceLock<Option<Checker>> = OnceLock::new();
    C.get_or_init(|| xenonet_base::json::parse(RESULT_DOC).ok().and_then(|d| compile(&[d]).ok())).as_ref()
}

/// RESULTS[tool].safeParse(envelope): the answer against the contract's.
fn result_ok(envelope: &Value) -> bool {
    result_checker().is_some_and(|c| c.check(c.root(0), &Data::from(envelope), false).ok())
}

fn key() -> Vec<u8> {
    xenonet_app::fsx::random_hex(32).into_bytes()
}

/// What answering a read needs of the world: the view, an actor, a
/// session's stats and the key for ids.
#[derive(Clone)]
enum World {
    Real(Arc<Reads>),
    Fixture,
}

/// One answer, as a task (service.answer) whose requests are answered
/// here: its envelope and what it read, or its refusal (`{ refused, code,
/// reason }`) or what it threw.
/// One read to answer: its operation and tool, its arguments, the grant's
/// policy and the request's id.
struct Asked {
    op: String,
    tool: String,
    args: Value,
    policy: Value,
    request_id: Value,
}

async fn answer(world: World, version: String, cursor_key: Vec<u8>, a: Asked) -> Result<Value, Value> {
    let Asked { op, tool, args, policy, request_id } = a;
    let w = world.clone();
    let perform: Perform = Arc::new(move |q: Value| {
        let w = w.clone();
        Box::pin(async move {
            let op = q.get("op").and_then(Value::as_str).unwrap_or("");
            match (op, &w) {
                ("now" | "cursorNow", _) => Ok(xenonet_base::json::number(now_ms())),
                ("check", _) => Ok(json!(result_ok(q.get("envelope").unwrap_or(&Value::Null)))),
                ("view", World::Fixture) => Ok(fixtures::fixture_view(now_ms())),
                ("actor", World::Fixture) => Ok(Value::Null),
                ("stats", World::Fixture) => Ok(fixtures::fixture_stats(&q.get("id").map(xenonet_base::text::js_string).unwrap_or_default())),
                ("idKey", World::Fixture) => Ok(json!("fixture".as_bytes())),
                ("view", World::Real(r)) => r.view().map_err(|e| fx::error(e.message, None)),
                ("actor", World::Real(r)) => Ok(r.actor(&q["id"])),
                ("stats", World::Real(r)) => {
                    let id = q.get("id").map(xenonet_base::text::js_string).unwrap_or_default();
                    r.state.session_stats(&id).await.map(|s| s.unwrap_or(Value::Null)).map_err(|e| fx::error(e.message, None))
                }
                ("idKey", World::Real(r)) => {
                    let k = r.remote.as_ref().map(|c| c.id_key()).transpose().map_err(|e| fx::error(e.message, None))?.unwrap_or(Value::Null);
                    Ok(json!(xenonet_base::text::js_string(&k).as_bytes()))
                }
                (other, _) => Err(fx::error(format!("no such request: {other}"), None)),
            }
        })
    });
    fx::drive(
        move |io| async move {
            let cfg = Rc::new(config(&version));
            let cursors = Rc::new(Cursors::new(Some(cursor_key)));
            service::answer(io, cfg, cursors, op, tool, args, policy, request_id).await
        },
        perform,
    )
    .await
}

/// What a read refused for: its code, and the log's reason.
fn refusal(e: &Value) -> Option<(String, String)> {
    (e.get("refused") == Some(&json!(true))).then(|| (xenonet_base::text::js_string(&e["code"]), e.get("reason").map(xenonet_base::text::js_string).unwrap_or_default()))
}

/// The reads: the source's view, the grants' reads, their audit.
pub struct Reads {
    pub remote: Option<Arc<Remote>>,
    pub state: Arc<dyn State>,
    pub actors: Option<Arc<dyn Actors>>,
    pub engine_store: Arc<dyn EngineStore>,
    pub store: Option<xenonet_engine::store::Shared>,
    pub version: String,
    pub emitter: Emitter,
    cursor_key: Vec<u8>,
    view: Mutex<Option<(f64, Value)>>,
    buckets: Mutex<Buckets>,
    inflight: Arc<Mutex<Inflight>>,
    presence: Mutex<RemotePresence>,
}

// The registry, as remote presence calls it, and onRead.
struct PresenceCalls<'a> {
    r: &'a Reads,
}

impl Calls for PresenceCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> CallResult<Arg> {
        match (path, method) {
            ("actors", m) => {
                let Some(a) = &self.r.actors else { return Ok(Arg::Undef) };
                a.registry(m, args.iter().map(Arg::to_json).collect()).map(|v| arg(&v)).map_err(|e| super::thrown_of(&e))
            }
            ("", "onRead") => {
                let read = args.first().map_or(Value::Null, Arg::to_json);
                let connection = xenonet_base::text::js_string(&read["connection"]);
                let transport = self.r.engine_store.connection(&connection).ok().flatten().and_then(|c| c.get("transport_kind").cloned()).unwrap_or(Value::Null);
                if let Some(ctx) = self.r.emitter.ctx() {
                    ctx.broadcast("remote-read", &json!({ "actor": read["actor"], "transport": transport, "reach": read["reach"], "at": read["at"] }));
                }
                Ok(Arg::Undef)
            }
            (p, m) => Err(Thrown::new(format!("{p}.{m} is not a function"))),
        }
    }
    fn get(&mut self, _path: &str) -> CallResult<Arg> {
        Ok(Arg::Undef)
    }
    fn kind(&mut self, path: &str) -> CallResult<String> {
        Ok(if path == "actors" { "object".into() } else { "undefined".into() })
    }
}

struct Around<'a> {
    state: &'a dyn State,
    actors: Option<&'a Arc<dyn Actors>>,
}

impl source::Around for Around<'_> {
    fn home_of(&mut self, id: &Value) -> Value {
        self.state.home_of(&xenonet_base::text::js_string(id)).unwrap_or(Value::Null)
    }
    fn product_of(&mut self, id: Option<&Value>) -> Value {
        product(id)
    }
    fn registered(&mut self, id: &Value) -> bool {
        self.actors.is_some_and(|a| a.registry("isRegistered", vec![id.clone()]).is_ok_and(|v| xenonet_base::text::truthy(Some(&v))))
    }
}

// productOf(id)?.id ?? null.
fn product(id: Option<&Value>) -> Value {
    id.and_then(Value::as_str).and_then(xenonet_engine::products::product_of).map_or(Value::Null, |p| json!(p.id))
}

impl Reads {
    pub fn new(
        remote: Option<Arc<Remote>>,
        state: Arc<dyn State>,
        actors: Option<Arc<dyn Actors>>,
        engine_store: Arc<dyn EngineStore>,
        store: Option<xenonet_engine::store::Shared>,
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
            cursor_key: key(),
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
            sources.get("claude").cloned().unwrap_or(Value::Null),
            sources.get("codex").cloned().unwrap_or(Value::Null),
            &mut around,
        );
        *self.view.locked() = Some((t, v.clone()));
        Ok(v)
    }

    fn actor(&self, id: &Value) -> Value {
        let Some(a) = &self.actors else { return Value::Null };
        match a.snapshot(&xenonet_base::text::js_string(id), None, false) {
            Ok(Some(snap)) => {
                let p = product(snap.get("product").and_then(|p| p.get("id")));
                source::actor(&snap, p)
            }
            _ => Value::Null,
        }
    }

    fn log(&self, record: Value) {
        if let Some(s) = &self.store {
            let row = arg(&audit::access(&record, now_ms()));
            let _ = s.with(|st| st.add_audit(&row));
        }
    }

    /// service.read(session, request): the read, or the error envelope.
    pub async fn read(self: &Arc<Self>, session: &Value, request: &Value) -> Value {
        let started = now_ms();
        let cfg = config(&self.version);
        let b = service::begin(&cfg, request);
        let grant = xenonet_base::text::js_string(&session["grantId"]);
        let request_id = b.request_id.clone();
        let op = b.op.clone().unwrap_or_else(|| "unknown".into());
        let log = |result: &str, extra: Map<String, Value>| {
            let mut rec = json!({ "requestId": request_id, "grant": session["grantId"], "revision": extra.get("revision").cloned().unwrap_or(Value::Null), "actor": session["actorId"], "connection": session["connectionId"], "operation": op, "result": result, "latencyMs": xenonet_base::json::number(now_ms() - started) });
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
        let mut standing = remote.recheck_at(&session["grantId"], &json!(-1));
        if standing["ok"] != json!(true) {
            return no(&xenonet_base::text::js_string(&standing["code"]), &xenonet_base::text::js_string(&standing["reason"]), Map::new());
        }
        let wait = self.buckets.locked().take(&grant, RATE.0, RATE.1, 1.0, now_ms());
        if wait != 0.0 {
            return no("RATE_LIMITED", "rate", extra(&[("retryAfterMs", xenonet_base::json::number(wait))]));
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
                    self.cursor_key.clone(),
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
                    return no(
                        &xenonet_base::text::js_string(&again["code"]),
                        &xenonet_base::text::js_string(&again["reason"]),
                        extra(&[("revision", policy["revision"].clone())]),
                    );
                }
                if again["changed"] == json!(true) {
                    standing = again;
                    continue;
                }
                remote.used(&grant);
                let targets: Vec<Arg> = out["targets"].as_array().map(|a| a.iter().map(arg).collect()).unwrap_or_default();
                let reach: Vec<Arg> = out["reach"].as_array().map(|a| a.iter().map(arg).collect()).unwrap_or_default();
                let _ = self.presence.locked().read(&mut PresenceCalls { r: self }, &arg(&session["actorId"]), &arg(&session["connectionId"]), &targets, &reach, now_ms(), true);
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
        let request_id = json!(format!("r_{}", xenonet_service::borrowed::random_uuid()));
        let cfg = config(&self.version);
        let b = service::begin(&cfg, &json!({ "request_id": request_id, "operation": operation, "arguments": {} }));
        let (Some(op), Some(tool), None) = (b.op.clone(), b.tool.clone(), b.no.as_ref()) else { return Value::Null };
        let policy = fixtures::standalone_policy(labels);
        match answer(World::Fixture, self.version.clone(), self.cursor_key.clone(), Asked { op, tool, args: b.args.clone(), policy, request_id }).await {
            Ok(out) => out["envelope"].get("data").cloned().unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    }

    /// What a grant with these projects and labels would send: the
    /// made-up answers first, then xenonet_projects and xenonet_sessions as
    /// they'd read the map now, under ids made for the preview alone.
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
        let places: Vec<(String, String)> = chosen.into_iter().map(|place| (format!("prj_{}", xenonet_service::borrowed::random_uuid()), place)).collect();
        let policy = json!({ "labels": labels, "lookbackMs": projection::WINDOW_MS, "places": places.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>() });
        let t = now_ms();
        let view = self.view()?;
        let mut visible = projection::visible_sessions(&view, &policy, t);
        let mut counts: Vec<(String, Map<String, Value>)> = places.iter().map(|(p, _)| (p.clone(), projection::empty_counts())).collect();
        for v in &visible {
            let project = xenonet_base::text::js_string(&v.project);
            if let Some((_, c)) = counts.iter_mut().find(|(p, _)| *p == project) {
                let n = c.get(v.status).and_then(Value::as_f64).unwrap_or(0.0) + 1.0;
                c.insert(v.status.into(), xenonet_base::json::number(n));
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
        let projects: Vec<Value> = places
            .iter()
            .map(|(p, place)| {
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
        Ok(json!({ "at": xenonet_base::time::iso(t), "labels": labels, "synthetic": synthetic, "real": { "projects": projects, "sessions": sessions }, "yours": yours }))
    }
}

/// The products that can be paired (core/actors/adapters.js, products.js).
pub fn products() -> Value {
    Value::Array(
        ADAPTERS
            .iter()
            .map(|a| {
                let name = a.product.and_then(xenonet_engine::products::product_of).map_or("An actor of your own", |p| p.name);
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
        let ask = arg(&json!({ "since": xenonet_base::json::number(now_ms() - 7.0 * 24.0 * 3600e3), "limit": xenonet_base::json::number(audit::limit_of(limit)), "grant": grant }));
        let rows = s.with(|st| st.audit(&ask)).map_err(|e| CoreError::new(e.to_string()))?;
        let rows: Vec<Value> = rows.iter().map(|r| xenonet_actors::connections::row_arg(r).to_json()).collect();
        Ok(Value::Array(audit::recent(&rows)))
    }

    fn access(&self, record: Value) {
        self.0.log(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_service::cores::Pending;

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
