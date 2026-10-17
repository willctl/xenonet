//! core/assistant/service.js: the assistant's reads, the one thing a remote
//! actor's adapter may ask of XenoNet once its grant's credential let it
//! in: assistant.read with one of six operations. The checks are here, so
//! a mistake in an adapter or a transport can't widen what a grant sees:
//! strict arguments in at most 16 KiB, four reads at a time, ten seconds
//! and 256 KiB an answer (a list that would be bigger comes back shorter,
//! with a page token for the rest), and the answer checked against the
//! strict schema before it goes.
//!
//! The grant's standing, its rate and the schemas are other modules'
//! (core/actors/connections.js and limits.js, core/schema/assistant.js);
//! the twin asks them, and drives each answer as a task whose requests
//! are the view, the actor, a session's stats, the attention ids' key, the
//! schema's check and the time.

use crate::cursors::{Cursors, For};
use crate::fixtures;
use crate::projection::{self as p, Visible};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::rc::Rc;
use xenonet_app::jsv::{less, strict_eq, truthy};
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;

pub struct Limits {
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub concurrent: f64,
    pub timeout_ms: f64,
    pub page: f64,
    pub page_max: f64,
}

pub const LIMITS: Limits = Limits { input_bytes: 16 << 10, output_bytes: 256 << 10, concurrent: 4.0, timeout_ms: 10e3, page: 20.0, page_max: 50.0 };

pub const MESSAGES: [(&str, &str); 13] = [
    ("NOT_AUTHORIZED", "This connection isn't allowed to read that."),
    ("NOT_FOUND", "XenoNet has nothing by that id for this connection."),
    ("GRANT_EXPIRED", "This connection's grant has expired. Its owner can pair it again in XenoNet."),
    ("GRANT_REVOKED", "This connection was revoked in XenoNet."),
    ("DISABLED", "Cloud access is turned off in XenoNet on this computer."),
    ("INVALID_ARGUMENT", "The request doesn't fit this tool's arguments."),
    ("CURSOR_EXPIRED", "That page token has expired. Ask for the list again without one."),
    ("SOURCE_UNAVAILABLE", "XenoNet can't read that source on this computer right now."),
    ("SERVICE_UNAVAILABLE", "XenoNet Service is unavailable on this paired device."),
    ("RATE_LIMITED", "Too many requests through this connection. Wait, then try again."),
    ("INCOMPATIBLE_VERSION", "This adapter and XenoNet Service use different interface versions. Update XenoNet on this computer."),
    ("TIMEOUT", "XenoNet took too long to answer."),
    ("INTERNAL_ERROR", "XenoNet couldn't answer this request."),
];

fn message_of(code: &str) -> &'static str {
    MESSAGES.iter().find(|(c, _)| *c == code).map_or(MESSAGES[12].1, |(_, m)| m)
}

fn retry_of(code: &str) -> Option<f64> {
    match code {
        "SOURCE_UNAVAILABLE" => Some(5000.0),
        "SERVICE_UNAVAILABLE" | "TIMEOUT" => Some(2000.0),
        "RATE_LIMITED" => Some(1000.0),
        _ => None,
    }
}

/// REQUEST_ID in core/schema/assistant.js: r_<uuid>.
pub fn is_request_id(s: &str) -> bool {
    let Some(u) = s.strip_prefix("r_") else { return false };
    let b = u.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { matches!(c, b'0'..=b'9' | b'a'..=b'f') })
}

/// An error's envelope (assistant/error/v1), from its code alone.
pub fn error_envelope(schema: &str, request_id: Option<&Value>, code: &str, retry_after_ms: Option<f64>) -> Value {
    let retry = retry_after_ms.or_else(|| retry_of(code));
    let id = request_id.filter(|r| is_request_id(&js_string(r))).cloned().unwrap_or(Value::Null);
    json!({
        "schema": schema,
        "request_id": id,
        "code": code,
        "message": message_of(code),
        "retryable": retry.is_some(),
        "retry_after_ms": retry.map_or(Value::Null, |r| xenonet_base::json::number(xenonet_base::units::js_round(r).max(0.0))),
    })
}

/// What the service is given once: the contract's names (from the
/// schema module), its limits and its version.
pub struct Config {
    /// [tool, operation], in the contract's order.
    pub operations: Vec<(String, String)>,
    pub attention_kinds: Vec<String>,
    pub result_schema: String,
    pub error_schema: String,
    pub version: String,
    pub limits: Limits,
}

impl Config {
    pub fn tool_of(&self, op: &str) -> Option<&str> {
        self.operations.iter().find(|(_, o)| o == op).map(|(t, _)| t.as_str())
    }
}

/// A read refused before it ran: its code and the log's reason.
pub struct No {
    pub code: &'static str,
    pub reason: &'static str,
}

/// A read's request, as far as it can be read: its id and operation (for
/// the log either way), and its tool and arguments, or why it's refused.
pub struct Begun {
    pub request_id: Value,
    pub op: Option<String>,
    pub tool: Option<String>,
    pub args: Value,
    pub no: Option<No>,
}

pub fn begin(cfg: &Config, request: &Value) -> Begun {
    let id = request.get("request_id");
    let request_id = id.filter(|r| is_request_id(&js_string(r))).cloned().unwrap_or(Value::Null);
    let op = request.get("operation").and_then(Value::as_str).filter(|o| cfg.tool_of(o).is_some()).map(String::from);
    let mut b = Begun { request_id, op: op.clone(), tool: op.as_deref().and_then(|o| cfg.tool_of(o)).map(String::from), args: json!({}), no: None };
    let bad = Some(No { code: "INVALID_ARGUMENT", reason: "bad-request" });
    let Value::Object(m) = request else {
        b.no = bad;
        return b;
    };
    if m.keys().any(|k| !["request_id", "operation", "arguments"].contains(&k.as_str())) || b.request_id.is_null() || op.is_none() {
        b.no = bad;
        return b;
    }
    b.args = m.get("arguments").filter(|a| !a.is_null()).cloned().unwrap_or_else(|| json!({}));
    if xenonet_base::json::stringify(&b.args, 0).len() > cfg.limits.input_bytes {
        b.no = Some(No { code: "INVALID_ARGUMENT", reason: "too-big" });
    }
    b
}

/// Reads at work under each grant, for the four-at-a-time limit.
#[derive(Default)]
pub struct Inflight(HashMap<String, f64>);

impl Inflight {
    /// Takes a slot, or false when the grant has none left.
    pub fn enter(&mut self, grant: &str, concurrent: f64) -> bool {
        let n = self.0.get(grant).copied().unwrap_or(0.0);
        if n >= concurrent {
            return false;
        }
        self.0.insert(grant.into(), n + 1.0);
        true
    }

    /// A read's work ended, whenever its answer went.
    pub fn leave(&mut self, grant: &str) {
        let n = self.0.get(grant).copied().unwrap_or(1.0) - 1.0;
        if n > 0.0 {
            self.0.insert(grant.into(), n);
        } else {
            self.0.remove(grant);
        }
    }
}

// ---------- one answer ----------

fn refuse(code: &str, reason: &str) -> Failure {
    let mut f = Map::new();
    f.insert("code".into(), json!(code));
    f.insert("reason".into(), json!(reason));
    f.insert("refused".into(), json!(true));
    Failure::with(code, f)
}

fn iso(ms: f64) -> Result<Value, Failure> {
    if ms.is_finite() && ms.abs() <= 8.64e15 {
        Ok(json!(xenonet_base::time::iso(ms)))
    } else {
        Err(Failure::msg("Invalid time value"))
    }
}

async fn now(io: &Io) -> Result<f64, Failure> {
    io.ask(json!({ "op": "now" })).await.map(|v| xenonet_base::text::js_number(Some(&v))).map_err(|_| Failure::Rethrow)
}

// cmp in page(): elementwise, by === and <.
fn cmp(a: &[Value], b: &Value) -> i32 {
    for (i, x) in a.iter().enumerate() {
        let y = b.get(i);
        if !strict_eq(Some(x), y) {
            return if less(Some(x), y) { -1 } else { 1 };
        }
    }
    0
}

/// A page of `items` (sorted by `keys`) after the cursor's key.
fn page<T>(items: Vec<(T, Vec<Value>)>, after: Option<&Value>, limit: f64) -> (Vec<T>, Option<Value>) {
    let rest: Vec<(T, Vec<Value>)> = match after.filter(|a| truthy(Some(a))) {
        Some(a) => items.into_iter().filter(|(_, k)| cmp(k, a) > 0).collect(),
        None => items,
    };
    let n = rest.len();
    let take = if limit.is_nan() { 0 } else { limit.max(0.0).min(n as f64) as usize };
    let mut shown: Vec<(T, Vec<Value>)> = rest.into_iter().take(take).collect();
    let next = if (n as f64) > limit { shown.last().map(|(_, k)| Value::Array(k.clone())) } else { None };
    (shown.drain(..).map(|(t, _)| t).collect(), next)
}

struct Out {
    freshness: p::Freshness,
    data: Value,
    targets: Vec<Value>,
}

struct Ctx<'a> {
    cfg: &'a Config,
    cursors: &'a Cursors,
    view: &'a Value,
    policy: &'a Value,
    t: f64,
    /// The page tokens' clock.
    ct: f64,
    args: &'a Value,
    limit: f64,
}

impl Ctx<'_> {
    fn for_<'b>(&'b self, op: &'b Value, filters: &'b Value) -> For<'b> {
        For { grant: self.policy.get("grantId"), revision: self.policy.get("revision"), op: Some(op), filters: Some(filters) }
    }

    fn after(&self, op: &str, filters: &Value) -> Result<Option<Value>, Failure> {
        let Some(c) = self.args.get("cursor").filter(|c| truthy(Some(c))) else { return Ok(None) };
        let o = json!(op);
        match self.cursors.read(&js_string(c), &self.for_(&o, filters), self.ct) {
            Ok(a) => Ok(a),
            Err(code) => Err(refuse(code, if code == "CURSOR_EXPIRED" { "cursor-cursor_expired" } else { "cursor-invalid_argument" })),
        }
    }

    fn next(&self, key: Option<Value>, op: &str, filters: &Value) -> Value {
        let o = json!(op);
        key.map_or(Value::Null, |k| json!(self.cursors.make(&self.for_(&o, filters), Some(&k), self.ct)))
    }

    fn place_of(&self, project: &Value) -> Option<Value> {
        p::pairs(self.policy.get("places")).into_iter().rev().find(|(k, _)| strict_eq(Some(k), Some(project))).map(|(_, v)| v)
    }

    // A project id the grant names, or NOT_FOUND, as for one that doesn't exist.
    fn own_project(&self) -> Result<Value, Failure> {
        let id = self.args.get("project_id").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
        if id.is_null() || self.place_of(&id).is_some() {
            Ok(id)
        } else {
            Err(refuse("NOT_FOUND", "project-not-granted"))
        }
    }

    fn targets_of(&self, project: &Value) -> Vec<Value> {
        if truthy(Some(project)) {
            vec![self.place_of(project).unwrap_or(Value::Null)]
        } else {
            vec![]
        }
    }

    fn observed(f: &p::Freshness) -> Result<Value, Failure> {
        f.observed_at.map_or(Ok(Value::Null), iso)
    }
}

fn status(c: &Ctx) -> Result<Out, Failure> {
    let policy = c.policy;
    let f = p::freshness_of(c.view, c.t, &p::all_sources());
    let or_null = |k: &str| policy.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
    let data = json!({
        "mode": "read_only",
        "connection": "connected",
        "assistant_api": 1,
        "service_version": xenonet_app::jstext::slice16(&c.cfg.version, 32),
        "capabilities": c.cfg.operations.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>(),
        "actor": { "actor_id": policy.get("actorId"), "product_id": or_null("productId"), "account_link_state": policy.get("accountLink"), "participation": "read_only", "messaging": "disabled" },
        "device": { "id": policy.get("deviceId"), "alias": or_null("deviceAlias") },
        "authorized_project_count": policy.get("projects").and_then(Value::as_array).map_or(0, Vec::len),
        "labels_enabled": truthy(policy.get("labels")),
        "data_mode": policy.get("dataMode"),
        "sources": p::sources_data(c.view),
        "transport_scope": policy.get("transportScope"),
        "audience_label": or_null("audienceLabel"),
    });
    Ok(Out { freshness: f, data, targets: vec![] })
}

fn projects(c: &Ctx) -> Result<Out, Failure> {
    let policy = c.policy;
    let lb = policy.get("lookbackMs").filter(|v| !v.is_null()).map_or(p::WINDOW_MS, |v| xenonet_base::text::js_number(Some(v)));
    let window_start = c.t - if lb.is_nan() { f64::NAN } else { p::WINDOW_MS.min(lb) };
    let filters = json!({});
    let after = c.after("projects", &filters)?;
    let projects: Vec<Value> = policy.get("projects").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut counts: Vec<(Value, Map<String, Value>)> = Vec::new();
    for pr in &projects {
        match counts.iter_mut().find(|(k, _)| strict_eq(Some(k), Some(pr))) {
            Some(slot) => slot.1 = p::empty_counts(),
            None => counts.push((pr.clone(), p::empty_counts())),
        }
    }
    for v in p::visible_sessions(c.view, policy, c.t) {
        if v.updated >= window_start {
            let Some((_, m)) = counts.iter_mut().find(|(k, _)| strict_eq(Some(k), Some(&v.project))) else { return Err(Failure::msg("Cannot read properties of undefined")) };
            let n = m.get(v.status).and_then(Value::as_f64).unwrap_or(0.0);
            m.insert(v.status.into(), xenonet_base::json::number(n + 1.0));
        }
    }
    let f = p::freshness_of(c.view, c.t, &p::all_sources());
    let observed = Ctx::observed(&f)?;
    let mut sorted = projects.clone();
    sorted.sort_by(|a, b| js_string(a).encode_utf16().cmp(js_string(b).encode_utf16()));
    let labels = truthy(policy.get("labels"));
    let places = p::pairs(c.view.get("places"));
    let all: Vec<(Value, Vec<Value>)> = sorted
        .into_iter()
        .map(|pr| {
            let label = if labels {
                let place = c.place_of(&pr);
                let name = place.and_then(|pl| places.iter().rev().find(|(k, _)| strict_eq(Some(k), Some(&pl))).map(|(_, v)| v.clone())).and_then(|v| v.get("name").cloned());
                p::label_of(name.as_ref())
            } else {
                Value::Null
            };
            let sc = counts.iter().find(|(k, _)| strict_eq(Some(k), Some(&pr))).map(|(_, m)| Value::Object(m.clone()));
            let mut m = Map::new();
            m.insert("project_id".into(), pr.clone());
            m.insert("label".into(), label);
            if let Some(sc) = sc {
                m.insert("session_counts".into(), sc);
            }
            m.insert("observed_at".into(), observed.clone());
            (Value::Object(m), vec![pr])
        })
        .collect();
    let (shown, next) = page(all, after.as_ref(), c.limit);
    Ok(Out { freshness: f, data: json!({ "window_start": iso(window_start)?, "projects": shown, "next_cursor": c.next(next, "projects", &filters) }), targets: vec![] })
}

fn sessions(c: &Ctx) -> Result<Out, Failure> {
    let policy = c.policy;
    let project = c.own_project()?;
    let lookback = p::lookback(policy);
    let since = match c.args.get("updated_since").filter(|v| truthy(Some(v))) {
        Some(s) => xenonet_base::time::parse_iso(&js_string(s)).unwrap_or(f64::NAN),
        None => c.t - p::WINDOW_MS.min(lookback),
    };
    if !since.is_finite() || since > c.t || c.t - since > lookback {
        return Err(refuse("INVALID_ARGUMENT", "updated-since-out-of-range"));
    }
    let statuses: Option<Vec<String>> = c.args.get("statuses").filter(|v| truthy(Some(v))).map(|s| {
        let mut list: Vec<String> = Vec::new();
        for x in s.as_array().into_iter().flatten() {
            let x = js_string(x);
            if !list.contains(&x) {
                list.push(x);
            }
        }
        list
    });
    let mut sorted_statuses = statuses.clone();
    if let Some(s) = &mut sorted_statuses {
        s.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    }
    let filters = json!({ "project": project, "statuses": sorted_statuses, "since": c.args.get("updated_since").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null) });
    let after = c.after("sessions", &filters)?;
    let upd = |v: &Visible| match v.s.get("updatedAt") {
        None | Some(Value::Null) => 0.0,
        other => xenonet_base::text::js_number(other),
    };
    let mut list: Vec<Visible> = p::visible_sessions(c.view, policy, c.t)
        .into_iter()
        .filter(|v| (!truthy(Some(&project)) || strict_eq(Some(&v.project), Some(&project))) && statuses.as_ref().is_none_or(|s| s.iter().any(|x| x == v.status)) && v.updated >= since)
        .collect();
    list.sort_by(|a, b| {
        let d = upd(b) - upd(a);
        if d != 0.0 && !d.is_nan() {
            return d.partial_cmp(&0.0).unwrap();
        }
        let (ia, ib) = (a.s.get("id").map(js_string).unwrap_or_default(), b.s.get("id").map(js_string).unwrap_or_default());
        ia.encode_utf16().cmp(ib.encode_utf16())
    });
    let keyed: Vec<(Visible, Vec<Value>)> = list.into_iter().map(|v| {
        let k = vec![xenonet_base::json::number(-upd(&v)), v.s.get("id").cloned().unwrap_or(Value::Null)];
        (v, k)
    }).collect();
    let (shown, next) = page(keyed, after.as_ref(), c.limit);
    let f = p::freshness_of(c.view, c.t, &p::all_sources());
    Ok(Out {
        freshness: f,
        data: json!({ "sessions": shown.iter().map(|v| p::session_summary(v, policy)).collect::<Vec<_>>(), "next_cursor": c.next(next, "sessions", &filters), "window_start": iso(since)? }),
        targets: c.targets_of(&project),
    })
}

fn attention(c: &Ctx, id_key: &[u8]) -> Result<Out, Failure> {
    let project = c.own_project()?;
    let kinds: Option<Vec<String>> = c.args.get("kinds").filter(|v| truthy(Some(v))).map(|k| {
        let mut list: Vec<String> = Vec::new();
        for x in k.as_array().into_iter().flatten() {
            let x = js_string(x);
            if !list.contains(&x) {
                list.push(x);
            }
        }
        list
    });
    let mut sorted = kinds.clone();
    if let Some(s) = &mut sorted {
        s.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    }
    let filters = json!({ "project": project, "kinds": sorted });
    let after = c.after("attention", &filters)?;
    let id_of = |parts: &str| format!("att_{}", &crate::crypto::hex(&crate::crypto::hmac_sha256(id_key, parts.as_bytes()))[..24]);
    let index = |k: &str| c.cfg.attention_kinds.iter().position(|x| x == k).map_or(-1.0, |i| i as f64);
    let mut items: Vec<(Value, Vec<Value>)> = p::attention_items(c.view, c.policy, c.t, &id_of)
        .into_iter()
        .filter(|i| (!truthy(Some(&project)) || strict_eq(i.get("project_id"), Some(&project))) && kinds.as_ref().is_none_or(|k| k.iter().any(|x| Some(x.as_str()) == i.get("kind").and_then(Value::as_str))))
        .map(|i| {
            let k = vec![json!(index(i.get("kind").and_then(Value::as_str).unwrap_or(""))), i.get("attention_id").cloned().unwrap_or(Value::Null)];
            (i, k)
        })
        .collect();
    items.sort_by(|x, y| {
        let d = x.1[0].as_f64().unwrap_or(0.0) - y.1[0].as_f64().unwrap_or(0.0);
        if d != 0.0 {
            return d.partial_cmp(&0.0).unwrap();
        }
        xenonet_app::jstext::locale_compare(&js_string(&x.1[1]), &js_string(&y.1[1]))
    });
    let (shown, next) = page(items, after.as_ref(), c.limit);
    let f = p::freshness_of(c.view, c.t, &p::all_sources());
    Ok(Out { freshness: f, data: json!({ "items": shown, "next_cursor": c.next(next, "attention", &filters) }), targets: c.targets_of(&project) })
}

fn actors(c: &Ctx) -> Result<Out, Failure> {
    let project = c.own_project()?;
    let filters = json!({ "project": project });
    let after = c.after("actors", &filters)?;
    let me = c.policy.get("actorId");
    let mut list: Vec<(Value, Vec<Value>)> = p::actor_summaries(c.view, c.policy, c.t, Some(&project).filter(|p| !p.is_null()))
        .into_iter()
        .map(|a| {
            let k = vec![json!(if strict_eq(a.get("actor_id"), me) { 0 } else { 1 }), a.get("actor_id").cloned().unwrap_or(Value::Null)];
            (a, k)
        })
        .collect();
    list.sort_by(|x, y| {
        let d = x.1[0].as_f64().unwrap_or(0.0) - y.1[0].as_f64().unwrap_or(0.0);
        if d != 0.0 {
            return d.partial_cmp(&0.0).unwrap();
        }
        xenonet_app::jstext::locale_compare(&js_string(&x.1[1]), &js_string(&y.1[1]))
    });
    let (shown, next) = page(list, after.as_ref(), c.limit);
    let f = p::freshness_of(c.view, c.t, &p::all_sources());
    Ok(Out { freshness: f, data: json!({ "self_actor_id": me, "actors": shown, "next_cursor": c.next(next, "actors", &filters) }), targets: c.targets_of(&project) })
}

async fn session(io: &Io, c: &Ctx<'_>, fixture: bool) -> Result<Out, Failure> {
    let id = c.args.get("session_id");
    let visible = p::visible_sessions(c.view, c.policy, c.t);
    let Some(v) = visible.iter().find(|x| strict_eq(x.s.get("id"), id)) else { return Err(refuse("NOT_FOUND", "session-not-visible")) };
    let app = v.s.get("app").map_or_else(|| "undefined".into(), js_string);
    let mut f = p::freshness_of(c.view, c.t, &[app]);
    let mut stats = Value::Null;
    if truthy(c.args.get("include_stats")) {
        let sid = v.s.get("id").map(js_string).unwrap_or_default();
        let raw = if fixture { Ok(fixtures::fixture_stats(&sid)) } else { io.ask(json!({ "op": "stats", "id": sid })).await };
        if let Ok(raw) = raw {
            if truthy(Some(&raw)) {
                stats = p::stats_data(&raw, now(io).await?);
            }
        }
        if stats.is_null() {
            f.warnings.push(json!({ "code": "STATS_UNAVAILABLE" }));
        }
    }
    let data = json!({ "session": p::session_summary(v, c.policy), "stats": stats });
    Ok(Out { freshness: f, data, targets: vec![v.s.get("id").cloned().unwrap_or(Value::Null)] })
}

// s.slice(n) on its UTF-16 units.
fn tail16(s: &str, n: usize) -> String {
    let units: Vec<u16> = s.encode_utf16().collect();
    String::from_utf16_lossy(units.get(n..).unwrap_or(&[]))
}

/// What a policy reads from: the map, or the made-up one, with the actor
/// itself in it either way.
async fn view_for(io: &Io, policy0: &Value) -> Result<(Value, Value), Failure> {
    let actor_id = policy0.get("actorId").cloned().unwrap_or(Value::Null);
    if policy0.get("dataMode").and_then(Value::as_str) != Some("fixture") {
        let mut view = io.ask(json!({ "op": "view" })).await.map_err(|_| Failure::Rethrow)?;
        let has = view.get("actors").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| strict_eq(x.get("id"), Some(&actor_id))));
        if !has {
            let me = io.ask(json!({ "op": "actor", "id": actor_id })).await.map_err(|_| Failure::Rethrow)?;
            if truthy(Some(&me)) {
                let mut list = vec![me];
                list.extend(view.get("actors").and_then(Value::as_array).cloned().unwrap_or_default());
                view["actors"] = Value::Array(list);
            }
        }
        return Ok((view, policy0.clone()));
    }
    let mut view = fixtures::fixture_view(now(io).await?);
    let me = io.ask(json!({ "op": "actor", "id": actor_id })).await.map_err(|_| Failure::Rethrow)?;
    let mut me = if !me.is_null() {
        me
    } else {
        let id = js_string(&actor_id);
        json!({ "id": actor_id, "kind": "agent", "productId": policy0.get("productId").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null), "name": tail16(&id, 6), "presence": "working", "via": ["remote"], "remote": { "bridge": "connected", "accountLink": policy0.get("accountLink") }, "reachable": false, "targets": [] })
    };
    // The actor's own targets are on the real map, which the fixture isn't.
    if let Value::Object(m) = &mut me {
        m.insert("targets".into(), json!([]));
    }
    let mut list = vec![me];
    list.extend(view.get("actors").and_then(Value::as_array).cloned().unwrap_or_default());
    view["actors"] = Value::Array(list);
    Ok((view, fixtures::fixture_policy(policy0)))
}

/// One operation, within its limits, as an envelope, shortened until it
/// fits: { envelope, bytes, targets, reach, projects }.
pub async fn answer(io: Io, cfg: Rc<Config>, cursors: Rc<Cursors>, op: String, tool: String, args: Value, policy0: Value, request_id: Value) -> Result<Value, Failure> {
    let (view, policy) = view_for(&io, &policy0).await?;
    let fixture_view = truthy(view.get("fixture"));
    let given = args.get("limit").filter(|v| !v.is_null()).map_or(cfg.limits.page, |v| xenonet_base::text::js_number(Some(v)));
    let limit_of = given.min(cfg.limits.page_max);
    let mut limit = limit_of;
    let id_key = if op == "attention" { io.ask(json!({ "op": "idKey" })).await.map_err(|_| Failure::Rethrow)? } else { Value::Null };
    let id_key: Vec<u8> = match &id_key {
        Value::String(s) => s.as_bytes().to_vec(),
        Value::Array(a) => a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect(),
        _ => Vec::new(),
    };
    loop {
        let t = now(&io).await?;
        let ct = io.ask(json!({ "op": "cursorNow" })).await.map(|v| xenonet_base::text::js_number(Some(&v))).map_err(|_| Failure::Rethrow)?;
        let c = Ctx { cfg: &cfg, cursors: &cursors, view: &view, policy: &policy, t, ct, args: &args, limit };
        let out = match op.as_str() {
            "status" => status(&c)?,
            "projects" => projects(&c)?,
            "sessions" => sessions(&c)?,
            "attention" => attention(&c, &id_key)?,
            "session" => session(&io, &c, policy.get("dataMode").and_then(Value::as_str) == Some("fixture")).await?,
            "actors" => actors(&c)?,
            _ => return Err(refuse("INVALID_ARGUMENT", "bad-request")),
        };
        let mut warnings = out.freshness.warnings.clone();
        if fixture_view {
            warnings.push(json!({ "code": "FIXTURE_DATA" }));
        }
        if limit < limit_of {
            warnings.push(json!({ "code": "PAGE_REDUCED" }));
        }
        warnings.truncate(16);
        let served = iso(now(&io).await?)?;
        let envelope = json!({
            "schema": cfg.result_schema,
            "request_id": request_id,
            "device_id": policy.get("deviceId"),
            "grant_revision": policy.get("revision"),
            "observed_at": Ctx::observed(&out.freshness)?,
            "served_at": served,
            "freshness": out.freshness.freshness,
            "partial": out.freshness.partial,
            "warnings": warnings,
            "data": out.data,
        });
        let fits = io.ask(json!({ "op": "check", "tool": tool, "envelope": envelope })).await.map_err(|_| Failure::Rethrow)?;
        if !truthy(Some(&fits)) {
            return Err(refuse("INTERNAL_ERROR", "result-off-contract"));
        }
        let bytes = xenonet_base::json::stringify(&envelope, 0).len();
        if bytes <= cfg.limits.output_bytes {
            let targets: Vec<Value> = if fixture_view { vec![] } else { out.targets.into_iter().filter(|x| truthy(Some(x))).collect() };
            let reach: Vec<Value> = if fixture_view {
                vec![]
            } else if !targets.is_empty() {
                targets.clone()
            } else {
                p::pairs(policy.get("places")).into_iter().map(|(_, v)| v).collect()
            };
            let projects = policy.get("projects").and_then(Value::as_array).map_or(0, Vec::len);
            return Ok(json!({ "envelope": envelope, "bytes": bytes, "targets": targets, "reach": reach, "projects": projects }));
        }
        if limit <= 1.0 {
            return Err(refuse("INTERNAL_ERROR", "result-too-big"));
        }
        limit = (limit / 2.0).floor().max(1.0);
    }
}
