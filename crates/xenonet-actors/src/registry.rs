//! Actors: the persistent entities beside the session graph
//! (core/actors/index.js; docs/design/actors/interface.md): the person at
//! the console, always-on agents, and programs that register themselves.
//!
//! This is the registry. Everything XenoNet learns of an actor comes
//! through it, whatever the source: an observer reading a product's state,
//! the person's window (`you`), a registered actor's reports, or the demo's
//! scripts (`demo`), which use the same calls:
//!
//!   upsert(def)          the actor itself: name, product, how XenoNet knows it
//!   report(id, ...)      one of its instances says it's there: presence,
//!                        status, whether it needs you
//!   record(id, ...)      something it did, as facts; XenoNet writes the detail
//!   target(id, ...)      a node of the map it acts on
//!
//! It keeps them in engine.sqlite, builds actor/snapshot/v1 and
//! actor/list/v1 from them, and decides presence on a clock: two missed
//! heartbeats are away, ten are offline.
//!
//! What it asks of the world (the time, the map, whether a process runs)
//! goes through `Host`, and its rows through `Store`. What it tells (an
//! event, the list changed) is queued for whoever runs it (`take_emits`,
//! `take_changed`). A definition, a report or a record comes as a JSON
//! object: a key left out is one the source didn't say, a null one it did.
//!
//! The person at the console is seen only by the window. Everywhere else
//! actor:you is a placeholder.

use crate::detail::{clip, place_name, presence_words, status_line, write_detail};
use crate::registration::{token_expired, TOKEN_UNUSED_MS};
use crate::store::{self, Tx};
use crate::you::You;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use xenonet_adapter::apps::app_of_session;
use xenonet_model::ids::{parse_actor_id, YOU};
use xenonet_os::nodepath::{style_of, NATIVE};
use xenonet_text::json::number;
use xenonet_text::text::display;
use xenonet_text::text::truthy;

pub const LIST_THROTTLE_MS: f64 = 500.0;
pub const CLOCK_MS: f64 = 5000.0;
pub const PRUNE_MS: f64 = 3600e3;
const DAY_MS: f64 = 24.0 * 3600e3;
const WEEK_MS: f64 = 7.0 * DAY_MS;
const RECENT: f64 = 20.0;
const TARGETS_SHOWN: usize = 16;
const TARGETS_KEPT: usize = 32;
const LIST_MAX: usize = 32;
const INSTANCES_MAX: usize = 8;
const GRACE_MS: f64 = 5000.0;
const ACTED_MS: f64 = 90e3;
const SESSION_WAIT_MS: f64 = 2.0 * 60e3;
const EVENTS_MAX: f64 = 5000.0;
const EDIT_HOLD_MS: f64 = 30.0 * 60e3;
const AWAIT_MS: f64 = 2.0 * 60e3;
const AWAIT_MAX: usize = 64;
/// A remote actor's read is evidence it's at work then, and for this long
/// after; then it's out of sight (unknown), never offline.
pub const REMOTE_LEASE_MS: f64 = 90e3;
const ACTIONS: [&str; 5] = ["spawned", "edited", "ran", "messaged", "said"];
const FACTS: [&str; 6] = ["count", "paths", "platform", "subject", "app", "to"];

fn rank(p: &str) -> i32 {
    match p {
        "working" => 4,
        "idle" => 3,
        "away" => 2,
        "unknown" => 1,
        _ => 0,
    }
}

/// How a call failed: an Error with these words, or the store's own.
#[derive(Debug, Clone, PartialEq)]
pub enum Fail {
    Error(String),
    Store(store::Error),
}

impl From<store::Error> for Fail {
    fn from(e: store::Error) -> Self {
        Fail::Store(e)
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

fn err<T>(m: impl Into<String>) -> Result<T> {
    Err(Fail::Error(m.into()))
}

/// What the registry asks of the world.
pub trait Host {
    fn now(&mut self) -> f64;
    /// What the map says of a node (map.describe), or null.
    fn describe(&mut self, id: &str) -> Value;
    /// Whether there's a map that can say what it has (map.has).
    fn has_map(&mut self) -> bool;
    /// Whether the map has a node: None when the map can't say (there's
    /// none, or it has no has()).
    fn has(&mut self, id: &str) -> Option<bool>;
    /// What map.ready() said: None when there's no map, or no ready().
    fn ready(&mut self) -> Option<bool>;
    /// The place holding a resolved absolute path (map.placeOf), or null.
    fn place_of(&mut self, p: &str) -> Value;
    /// The session busy most lately (map.recentSession), or null.
    fn recent_session(&mut self) -> Value;
    /// Whether a process still runs.
    fn alive(&mut self, pid: f64) -> bool;
}

/// What the registry keeps in the engine's database: the actors' rows,
/// their events and their targets, each row a JSON object of its columns.
pub trait Store: Tx + Send + Sync {
    fn actor_rows(&self) -> store::Result<Vec<Value>>;
    fn actor_targets(&self) -> store::Result<Vec<Value>>;
    /// An actor's last `limit` events, newest last.
    fn last_actor_events(&self, actor: &str, limit: f64) -> store::Result<Vec<Value>>;
    /// An actor's row, new or again: everything but its tokens.
    fn save_actor(&self, row: &Value) -> store::Result<()>;
    /// An actor gone, with its events and targets.
    fn remove_actor(&self, slug: &str) -> store::Result<()>;
    /// An actor's events and targets let go, the actor kept.
    fn forget_actor(&self, slug: &str) -> store::Result<()>;
    fn add_actor_event(&self, event: &Value) -> store::Result<()>;
    fn save_actor_target(&self, target: &Value) -> store::Result<()>;
    fn drop_actor_target(&self, actor: &str, target: &str, relation: &str) -> store::Result<()>;
    /// How many events of each kind each actor has had since `since`
    /// ({ actor, kind, n }).
    fn actor_event_counts(&self, since: f64) -> store::Result<Vec<Value>>;
    /// The targets of each actor's spawned events since `since`, the first
    /// time from each source ({ actor, target, via, at }).
    fn actor_spawns(&self, since: f64) -> store::Result<Vec<Value>>;
    /// What an actor's events of one kind said as facts since `since`,
    /// newest first ({ at, facts, via }).
    fn actor_facts_since(&self, actor: &str, kind: &str, since: f64) -> store::Result<Vec<Value>>;
    /// Events since `since`, oldest first, at most `limit`: only `only`'s,
    /// or all but `except`'s.
    fn actor_events_since(&self, since: f64, limit: f64, only: Option<&str>, except: Option<&str>) -> store::Result<Vec<Value>>;
    /// Each actor's token times ({ slug, hasToken, at, hasGateway,
    /// gatewayAt }).
    fn actor_token_times(&self) -> store::Result<Vec<Value>>;
    /// What's kept, at start and every hour.
    fn prune(&self, now: f64) -> store::Result<()>;
}

/// A column of a row, as text.
fn text<'a>(r: &'a Value, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str)
}

/// A column of a row, as a number.
fn num(r: &Value, k: &str) -> Option<f64> {
    r.get(k).and_then(Value::as_f64)
}

/// A column of a row, or null.
fn col(r: &Value, k: &str) -> Value {
    r.get(k).cloned().unwrap_or(Value::Null)
}

/// `${v}` of a field, undefined when it isn't there.
fn template(v: Option<&Value>) -> String {
    xenonet_text::text::template(v)
}

/// What kind of node an id names, by its form.
pub fn node_kind(id: &str) -> Option<&'static str> {
    if parse_actor_id(id).is_some() {
        return Some("actor");
    }
    if app_of_session(id).is_some() {
        return Some("session");
    }
    ["project", "zone", "native"].into_iter().find(|k| id.strip_prefix(k).is_some_and(|r| r.starts_with(':')))
}

fn node_kind_of(v: &Value) -> Option<&'static str> {
    match v {
        Value::Null => node_kind(""),
        Value::String(s) => node_kind(s),
        other => node_kind(&template(Some(other))),
    }
}

// The relation an event gives its actor to its target.
fn relation_for(kind: &str, target: &Value) -> Option<&'static str> {
    let k = node_kind_of(target)?;
    if k == "actor" || kind == "presence" || kind == "status" {
        return None;
    }
    match kind {
        "spawned" => Some(if k == "session" { "spawned" } else { "works-on" }),
        "edited" => (k != "session").then_some("edited"),
        "said" => (k == "session").then_some("messaged"),
        _ => (k != "session").then_some("works-on"),
    }
}

fn parse_json(s: Option<&str>, fallback: Value) -> Value {
    match s {
        None => fallback,
        Some(s) => xenonet_text::json::parse(s).unwrap_or(fallback),
    }
}

fn event_of(r: &Value) -> Value {
    json!({
        "schema": "actor/event/v1",
        "id": col(r, "id"),
        "actor": col(r, "actor"),
        "at": col(r, "at"),
        "kind": col(r, "kind"),
        "target": col(r, "target"),
        "detail": col(r, "detail"),
        "via": col(r, "via"),
    })
}

fn placeholder() -> Value {
    json!({
        "schema": "actor/snapshot/v1",
        "id": YOU,
        "kind": "human",
        "name": "You",
        "product": null,
        "host": null,
        "via": ["observed"],
        "presence": "unknown",
        "presenceSince": null,
        "lastSeen": null,
        "heartbeatMs": null,
        "status": null,
        "capabilities": [],
        "targets": [],
        "recent": [],
        "counts": { "events24h": 0, "spawned24h": 0, "messages24h": 0 },
        "identity": null,
    })
}

// The usage an observer reports (adapters/hermes), cut to the snapshot's
// shape: numbers where numbers go, names clipped to 80 characters, five
// models. Null or a non-object is none.
pub fn clean_usage(u: &Value) -> Option<Value> {
    if !u.is_object() {
        return None;
    }
    let num = |a: Option<&Value>| a.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).unwrap_or(0.0);
    let cost = |a: Option<&Value>| a.and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).map_or(Value::Null, number);
    let name = |a: Option<&Value>| match a.and_then(Value::as_str) {
        Some(t) if !t.is_empty() => Value::String(String::from_utf16_lossy(&t.encode_utf16().take(80).collect::<Vec<u16>>())),
        _ => Value::Null,
    };
    let t = u.get("tokens");
    fn field<'a>(o: Option<&'a Value>, k: &str) -> Option<&'a Value> {
        o.and_then(|o| o.get(k))
    }
    let models: Vec<Value> = u
        .get("models")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(5)
                .map(|m| {
                    let m = Some(m);
                    json!({ "provider": name(field(m, "provider")), "model": name(field(m, "model")), "mode": name(field(m, "mode")), "tokens": number(num(field(m, "tokens"))), "costUsd": cost(field(m, "costUsd")) })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(json!({
        "since": number(num(u.get("since"))),
        "sessions": number(num(u.get("sessions")).floor()),
        "costUsd": cost(u.get("costUsd")),
        "estimated": truthy(u.get("estimated")),
        "unpriced": number(num(u.get("unpriced")).floor()),
        "tokens": { "input": number(num(field(t, "input"))), "output": number(num(field(t, "output"))), "reasoning": number(num(field(t, "reasoning"))), "cacheRead": number(num(field(t, "cacheRead"))), "cacheWrite": number(num(field(t, "cacheWrite"))) },
        "models": models,
    }))
}

fn opt(n: Option<f64>) -> Value {
    n.map_or(Value::Null, number)
}

#[derive(Clone, Debug)]
pub struct Instance {
    pub presence: String,
    pub at: f64,
    pub heartbeat_ms: Option<f64>,
    pub pid: Option<f64>,
    pub restored: bool,
    pub lease_ms: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct Target {
    pub id: String,
    pub relation: String,
    pub since: f64,
    pub last_at: f64,
}

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: String,
    pub slug: String,
    pub kind: String,
    pub name: Value,
    pub product: Value,
    pub via: Vec<String>,
    pub capabilities: Vec<Value>,
    pub heartbeat_ms: Option<f64>,
    pub identity: Value,
    pub enabled: bool,
    pub created_at: f64,
    pub allowed_at: Option<f64>,
    pub last_seen: Option<f64>,
    pub presence: String,
    pub presence_since: Option<f64>,
    pub status: Option<String>,
    pub status_at: Option<f64>,
    pub needs_you: Option<Value>,
    /// What the app's own records say its sessions used (`clean_usage`);
    /// an observer's to say again, so never stored.
    pub usage: Option<Value>,
    pub seq: f64,
    pub instances: Vec<(String, Instance)>,
    pub acted_at: Option<f64>,
    pub cwd: Value,
    pub pid: Option<f64>,
    pub targets: Vec<(String, Target)>,
    pub recent: Vec<Value>,
    pub dirty: bool,
    pub settled_at: Option<f64>,
    pub remote: Option<Value>,
    pub installed: Option<bool>,
    pub simulated_offline: bool,
    pub simulated_remote: bool,
}

impl Actor {
    fn has_via(&self, v: &str) -> bool {
        self.via.iter().any(|x| x == v)
    }
    fn instance(&self, name: &str) -> Option<&Instance> {
        self.instances.iter().find(|(n, _)| n == name).map(|(_, i)| i)
    }
    fn target_mut(&mut self, key: &str) -> Option<&mut Target> {
        self.targets.iter_mut().find(|(k, _)| k == key).map(|(_, t)| t)
    }
    fn drop_target(&mut self, key: &str) -> Option<Target> {
        let i = self.targets.iter().position(|(k, _)| k == key)?;
        Some(self.targets.remove(i).1)
    }
}

/// A spawned report waiting for its session.
#[derive(Clone, Debug)]
pub struct Waiting {
    pub actor: String,
    pub app: Value,
    pub place: Value,
    pub own: Vec<f64>,
    pub at: f64,
    pub until: f64,
}

/// Who started which sessions: each listed actor's name by its id, and
/// the starts.
pub type Started = (Vec<(String, Value)>, Vec<Value>);

pub struct Actors {
    pub store: Arc<dyn Store>,
    pub demo: bool,
    all: Vec<Actor>,
    pub waiting: Vec<Waiting>,
    last_list: Option<String>,
    changed: bool,
    emits: Vec<(&'static str, Value)>,
    pub you: You,
}

impl Actors {
    /// The actors kept in `store`, and you, if only as a placeholder.
    pub fn new(store: Arc<dyn Store>, demo: bool, host: &mut dyn Host) -> Result<Self> {
        let mut a = Actors { store, demo, all: Vec::new(), waiting: Vec::new(), last_list: None, changed: false, emits: Vec::new(), you: You::default() };
        a.load()?;
        let via = if demo { json!(["demo", "observed"]) } else { json!(["observed"]) };
        a.upsert(&json!({ "id": YOU, "kind": "human", "name": "You", "via": via, "heartbeatMs": number(crate::you::YOU_HEARTBEAT_MS) }), host)?;
        Ok(a)
    }

    /// `f` in a transaction of the store's, unless one is under way.
    fn transaction<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let store = self.store.clone();
        store::transaction(store.as_ref(), || f(self))
    }

    /// The events told since this was last asked: ("event", event).
    pub fn take_emits(&mut self) -> Vec<(&'static str, Value)> {
        std::mem::take(&mut self.emits)
    }

    /// Whether the list may have changed since this was last asked.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn mark_changed(&mut self) {
        self.changed = true;
    }

    fn get(&self, id: &str) -> Option<&Actor> {
        self.all.iter().find(|a| a.id == id)
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Actor> {
        self.all.iter_mut().find(|a| a.id == id)
    }

    fn need(&self, id: &str) -> Result<&Actor> {
        self.get(id).ok_or_else(|| Fail::Error(format!("no actor {id}")))
    }

    fn need_index(&self, id: &str) -> Result<usize> {
        self.all.iter().position(|a| a.id == id).ok_or_else(|| Fail::Error(format!("no actor {id}")))
    }

    // ---------- what the API asks of an actor ----------

    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
    pub fn ids(&self) -> Vec<String> {
        self.all.iter().map(|a| a.id.clone()).collect()
    }
    pub fn presence(&self, id: &str) -> Option<String> {
        self.get(id).map(|a| a.presence.clone())
    }
    pub fn presence_since(&self, id: &str) -> Value {
        self.get(id).map_or(Value::Null, |a| opt(a.presence_since.or(Some(a.created_at))))
    }
    pub fn is_public(&self, id: &str) -> bool {
        self.demo || id != YOU
    }
    pub fn can(&self, id: &str, capability: &Value) -> bool {
        self.get(id).is_some_and(|a| a.capabilities.contains(capability))
    }
    pub fn enabled(&self, id: &str) -> bool {
        self.get(id).is_none_or(|a| a.enabled)
    }
    pub fn installed(&self, id: &str) -> Option<bool> {
        self.get(id).and_then(|a| a.installed)
    }
    pub fn is_registered(&self, id: &str) -> bool {
        self.get(id).is_some_and(|a| a.has_via("registered") && (a.allowed_at.is_some() || a.has_via("demo")))
    }
    pub fn only_observed(&self, id: &str) -> bool {
        self.get(id).is_some_and(|a| id != YOU && !self.is_registered(id) && a.via.iter().any(|v| v == "observed" || v == "attributed"))
    }
    pub fn targets_of(&self, id: &str) -> Vec<Value> {
        self.get(id).map_or_else(Vec::new, |a| a.targets.iter().map(|(_, t)| json!({ "id": t.id, "relation": t.relation })).collect())
    }
    /// The processes an actor named: the one it registered as, and its
    /// instances'.
    pub fn pids_of(&self, id: &str) -> Vec<f64> {
        let mut out: Vec<f64> = Vec::new();
        if let Some(a) = self.get(id) {
            for p in std::iter::once(a.pid).chain(a.instances.iter().map(|(_, i)| i.pid)).flatten() {
                if p.is_finite() && p.trunc() == p && !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        out
    }
    /// An actor as registering sees it, or null.
    pub fn info(&self, id: &str) -> Value {
        self.get(id).map_or(Value::Null, |a| {
            json!({
                "id": id,
                "name": a.name,
                "product": a.product,
                "via": a.via,
                "heartbeatMs": opt(a.heartbeat_ms),
                "capabilities": a.capabilities,
                "identity": a.identity,
                "allowedAt": opt(a.allowed_at),
                "presence": a.presence,
                "enabled": a.enabled,
            })
        })
    }
    pub fn registered_count(&self) -> usize {
        self.all.iter().filter(|a| a.allowed_at.is_some() && a.has_via("registered") && !a.has_via("demo")).count()
    }
    /// The actors of a product (by its id; None for undefined).
    pub fn of_product(&self, product: Option<&Value>) -> Vec<String> {
        self.all.iter().filter(|a| a.product.get("id") == product).map(|a| a.id.clone()).collect()
    }
    pub fn is_remote(&self, id: &str) -> bool {
        self.get(id).is_some_and(|a| a.has_via("remote"))
    }
    pub fn remote_of(&self, id: &str) -> Value {
        self.get(id).and_then(|a| a.remote.clone()).unwrap_or(Value::Null)
    }
    pub fn simulation_held(&self, id: &str) -> bool {
        self.demo && self.get(id).is_some_and(|a| a.simulated_offline)
    }

    /// Each actor as the twin shows `actors.actors` (instances and targets
    /// by name), for the tests that look in.
    pub fn records(&self) -> Value {
        Value::Array(
            self.all
                .iter()
                .map(|a| {
                    let instances: Vec<Value> = a
                        .instances
                        .iter()
                        .map(|(n, i)| json!([n, { "presence": i.presence, "at": number(i.at), "heartbeatMs": opt(i.heartbeat_ms), "pid": opt(i.pid) }]))
                        .collect();
                    let targets: Vec<Value> =
                        a.targets.iter().map(|(k, t)| json!([k, { "id": t.id, "relation": t.relation, "since": number(t.since), "lastAt": number(t.last_at) }])).collect();
                    json!({ "id": a.id, "slug": a.slug, "presence": a.presence, "status": a.status, "pid": opt(a.pid), "cwd": a.cwd, "instances": instances, "targets": targets })
                })
                .collect(),
        )
    }

    // ---------- what's kept ----------

    fn load(&mut self) -> Result<()> {
        let targets = self.store.actor_targets()?;
        let actor_rows = self.store.actor_rows()?;
        for r in actor_rows {
            let slug = template(Some(&col(&r, "slug")));
            let id = format!("actor:{slug}");
            if parse_actor_id(&id).is_none() {
                continue;
            }
            let mut ts: Vec<(String, Target)> = Vec::new();
            for t in targets.iter().filter(|t| text(t, "actor") == Some(id.as_str())) {
                let (target, relation) = (text(t, "target").unwrap_or("").to_string(), text(t, "relation").unwrap_or("").to_string());
                let key = format!("{relation}|{target}");
                let v = Target { id: target, relation, since: num(t, "since").unwrap_or(f64::NAN), last_at: num(t, "last_at").unwrap_or(f64::NAN) };
                match ts.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = v,
                    None => ts.push((key, v)),
                }
            }
            let recent: Vec<Value> = self.store.last_actor_events(&id, RECENT)?.iter().map(event_of).collect();
            let mut a = Actor {
                id: id.clone(),
                slug: slug.clone(),
                kind: text(&r, "kind").unwrap_or("").to_string(),
                name: col(&r, "name"),
                product: parse_json(text(&r, "product"), Value::Null),
                via: parse_json(text(&r, "via"), json!([])).as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                capabilities: parse_json(text(&r, "capabilities"), json!([])).as_array().cloned().unwrap_or_default(),
                heartbeat_ms: num(&r, "heartbeat_ms"),
                identity: col(&r, "identity"),
                enabled: num(&r, "enabled") != Some(0.0),
                created_at: num(&r, "created_at").unwrap_or(f64::NAN),
                allowed_at: num(&r, "allowed_at"),
                last_seen: num(&r, "last_seen"),
                presence: text(&r, "presence").unwrap_or("unknown").to_string(),
                presence_since: num(&r, "presence_at"),
                status: text(&r, "status").map(str::to_string),
                status_at: num(&r, "status_at"),
                needs_you: None,
                usage: None,
                seq: num(&r, "seq").unwrap_or(0.0),
                instances: Vec::new(),
                acted_at: None,
                cwd: Value::Null,
                pid: None,
                targets: ts,
                recent,
                dirty: false,
                settled_at: None,
                remote: None,
                installed: None,
                simulated_offline: false,
                simulated_remote: false,
            };
            // After a restart an actor keeps its last presence while it's
            // within two heartbeats of when it was last heard from, and the
            // rules take over from there. One only a remote connection
            // speaks for has no heartbeat to age by: its last presence lasts
            // a read's lease.
            if let Some(last) = a.last_seen {
                let lease = (a.heartbeat_ms.is_none() && a.has_via("remote")).then_some(REMOTE_LEASE_MS);
                let name = if id == YOU { "window" } else { "main" };
                a.instances.push((name.into(), Instance { presence: a.presence.clone(), at: last, heartbeat_ms: a.heartbeat_ms, pid: None, restored: true, lease_ms: lease }));
            }
            match self.all.iter_mut().find(|x| x.id == id) {
                Some(slot) => *slot = a,
                None => self.all.push(a),
            }
        }
        Ok(())
    }

    fn row(a: &Actor) -> Value {
        let json_text = |v: &Value| Value::String(xenonet_text::json::stringify(v, 0));
        json!({
            "slug": a.slug,
            "kind": a.kind,
            "name": a.name,
            "product": if a.product.is_null() { Value::Null } else { json_text(&a.product) },
            "via": json_text(&json!(a.via)),
            "capabilities": json_text(&Value::Array(a.capabilities.clone())),
            "heartbeat_ms": opt(a.heartbeat_ms),
            "identity": a.identity,
            "enabled": i32::from(a.enabled),
            "created_at": number(a.created_at),
            "allowed_at": opt(a.allowed_at),
            "last_seen": opt(a.last_seen),
            "presence": a.presence,
            "presence_at": opt(a.presence_since),
            "status": a.status,
            "status_at": opt(a.status_at),
            "seq": number(a.seq),
        })
    }

    fn save(&mut self, i: usize) -> Result<()> {
        let row = Self::row(&self.all[i]);
        self.store.save_actor(&row)?;
        self.all[i].dirty = false;
        Ok(())
    }

    // Rows that only heard a heartbeat are written on the clock.
    fn flush(&mut self) -> Result<()> {
        let dirty: Vec<usize> = (0..self.all.len()).filter(|&i| self.all[i].dirty).collect();
        if dirty.is_empty() {
            return Ok(());
        }
        self.transaction(|me| {
            for i in dirty {
                me.save(i)?;
            }
            Ok(())
        })
    }

    // ---------- the actors themselves ----------

    /// An actor XenoNet knows of, new or again (`def`: id, kind, name,
    /// product, via, heartbeatMs, capabilities, identity, allowedAt,
    /// enabled, at). Only you are human.
    pub fn upsert(&mut self, def: &Value, host: &mut dyn Host) -> Result<()> {
        let id_given = def.get("id");
        let parsed = id_given.and_then(Value::as_str).and_then(|id| parse_actor_id(id).map(|(h, s)| (id.to_string(), h.is_some(), s.to_string())));
        let Some((id, elsewhere, slug)) = parsed else {
            return err(format!("not an actor id of this machine: {}", template(id_given)));
        };
        if elsewhere {
            return err(format!("not an actor id of this machine: {id}"));
        }
        let kind = def.get("kind").map_or_else(|| "agent".to_string(), display);
        if (kind == "human") != (id == YOU) {
            return err(format!("only {YOU} is human"));
        }
        let at = match def.get("at") {
            None => host.now(),
            Some(a) => a.as_f64().unwrap_or(f64::NAN),
        };
        if self.get(&id).is_none() {
            self.all.push(Actor {
                id: id.clone(),
                slug,
                kind: kind.clone(),
                name: Value::Null,
                product: Value::Null,
                via: Vec::new(),
                capabilities: Vec::new(),
                heartbeat_ms: None,
                identity: Value::Null,
                enabled: true,
                created_at: at,
                allowed_at: None,
                last_seen: None,
                presence: "unknown".into(),
                presence_since: None,
                status: None,
                status_at: None,
                needs_you: None,
                usage: None,
                seq: 0.0,
                instances: Vec::new(),
                acted_at: None,
                cwd: Value::Null,
                pid: None,
                targets: Vec::new(),
                recent: Vec::new(),
                dirty: false,
                settled_at: None,
                remote: None,
                installed: None,
                simulated_offline: false,
                simulated_remote: false,
            });
        }
        let i = self.need_index(&id)?;
        let a = &mut self.all[i];
        a.kind = kind;
        a.name = col(def, "name");
        a.product = col(def, "product");
        a.via = def.get("via").and_then(Value::as_array).map(|l| l.iter().map(display).collect()).unwrap_or_default();
        a.heartbeat_ms = def.get("heartbeatMs").and_then(Value::as_f64);
        a.capabilities = def.get("capabilities").and_then(Value::as_array).cloned().unwrap_or_default();
        a.identity = col(def, "identity");
        if let Some(v) = def.get("allowedAt") {
            a.allowed_at = v.as_f64();
        }
        if let Some(v) = def.get("enabled") {
            a.enabled = truthy(Some(v));
        }
        self.save(i)?;
        self.mark_changed();
        Ok(())
    }

    /// How XenoNet knows an actor, one source added, and nothing else of
    /// it changed.
    pub fn add_via(&mut self, id: &str, v: &str) -> Result<()> {
        let i = self.need_index(id)?;
        if self.all[i].has_via(v) {
            return Ok(());
        }
        self.all[i].via.push(v.into());
        self.save(i)?;
        self.mark_changed();
        Ok(())
    }

    pub fn drop_via(&mut self, id: &str, v: &str) -> Result<()> {
        let i = self.need_index(id)?;
        if !self.all[i].has_via(v) {
            return Ok(());
        }
        self.all[i].via.retain(|x| x != v);
        self.save(i)?;
        self.mark_changed();
        Ok(())
    }

    /// What an actor's remote connections come to; null takes it away.
    pub fn set_remote(&mut self, id: &str, remote: &Value) -> Result<()> {
        let Some(a) = self.get_mut(id) else { return Ok(()) };
        // A made-up tunnel stays until clear.
        if a.simulated_remote && !truthy(remote.get("simulated")) {
            return Ok(());
        }
        let next = if truthy(Some(remote)) {
            let mut transports: Vec<String> = remote.get("transports").and_then(Value::as_array).map(|t| t.iter().map(|x| template(Some(x))).collect()).unwrap_or_default();
            transports.sort_by(|x, y| xenonet_text::sort::utf16_cmp(x, y));
            transports.dedup();
            Some(json!({
                "active": truthy(remote.get("active")),
                "connections": remote.get("connections").filter(|v| !v.is_null()).cloned().unwrap_or(json!(0)),
                "bridge": remote.get("bridge").filter(|v| !v.is_null()).cloned().unwrap_or(json!("not_connected")),
                "accountLink": remote.get("accountLink").filter(|v| !v.is_null()).cloned().unwrap_or(json!("unverified")),
                "permissions": remote.get("permissions").and_then(Value::as_array).cloned().unwrap_or_default(),
                "transports": transports,
            }))
        } else {
            None
        };
        let same = xenonet_text::json::stringify(&next.clone().unwrap_or(Value::Null), 0) == xenonet_text::json::stringify(&a.remote.clone().unwrap_or(Value::Null), 0);
        if same {
            return Ok(());
        }
        a.remote = next;
        self.mark_changed();
        Ok(())
    }

    /// Before an actor goes (remove), or leaves with its connections
    /// revoked (unregister): its slug, for what runs first.
    pub fn slug_of(&self, id: &str) -> Result<String> {
        Ok(self.need(id)?.slug.clone())
    }

    /// An actor gone, with its events and targets, and what it waits on.
    /// What runs before it goes (its remote connections revoked) has run.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let slug = self.need(id)?.slug.clone();
        self.store.remove_actor(&slug)?;
        self.all.retain(|a| a.id != id);
        self.waiting.retain(|w| w.actor != id);
        self.mark_changed();
        Ok(())
    }

    /// Your record deleted at once, when you turn off "Show me on the map".
    pub fn forget_you(&mut self) -> Result<()> {
        let i = self.need_index(YOU)?;
        let slug = self.all[i].slug.clone();
        self.transaction(|me| {
            me.store.forget_actor(&slug)?;
            let a = &mut me.all[i];
            a.instances.clear();
            a.targets.clear();
            a.presence = "unknown".into();
            a.presence_since = None;
            a.last_seen = None;
            a.status = None;
            a.status_at = None;
            a.needs_you = None;
            a.usage = None;
            a.acted_at = None;
            a.cwd = Value::Null;
            a.recent.clear();
            me.save(i)
        })?;
        self.mark_changed();
        Ok(())
    }

    /// Whether an unregister with `keep_remote` revokes the actor's remote
    /// connections first (what runs before it goes).
    pub fn unregister_revokes(&self, id: &str, keep_remote: bool) -> Result<bool> {
        Ok(!keep_remote && self.need(id)?.has_via("remote"))
    }

    /// A registered actor that no longer reports itself. `revoked`: its
    /// remote connections were revoked first (unregister_revokes). Whether
    /// it went for good, in which case what runs before it goes runs again
    /// (as remove() does).
    pub fn unregister(&mut self, id: &str, revoked: bool) -> Result<bool> {
        let i = self.need_index(id)?;
        if revoked {
            self.all[i].via.retain(|v| v != "remote");
        }
        let a = &self.all[i];
        let observed = !a.has_via("demo") && a.via.iter().any(|v| v == "observed" || v == "attributed");
        if !observed && !a.has_via("remote") {
            return Ok(true);
        }
        let a = &mut self.all[i];
        a.via.retain(|v| v != "registered");
        a.allowed_at = None;
        a.pid = None;
        a.cwd = Value::Null;
        self.save(i)?;
        self.mark_changed();
        Ok(false)
    }

    /// A new token for an actor, given from the window now.
    pub fn allow(&mut self, id: &str, at: f64) -> Result<()> {
        let i = self.need_index(id)?;
        if self.all[i].allowed_at == Some(at) {
            return Ok(());
        }
        self.all[i].allowed_at = Some(at);
        self.save(i)?;
        self.mark_changed();
        Ok(())
    }

    pub fn set_enabled(&mut self, id: &str, on: bool) -> Result<()> {
        let i = self.need_index(id)?;
        self.all[i].enabled = on;
        self.save(i)?;
        self.mark_changed();
        Ok(())
    }

    pub fn set_installed(&mut self, id: &str, installed: bool) -> Result<()> {
        let i = self.need_index(id)?;
        if self.all[i].installed == Some(installed) {
            return Ok(());
        }
        self.all[i].installed = Some(installed);
        self.mark_changed();
        Ok(())
    }

    /// A registered actor's spawned report that named no session: it waits
    /// up to two minutes for a session to start under the actor's own
    /// processes in that place. Whether it waits.
    pub fn await_session(&mut self, id: &str, app: Value, place: Value, own: Vec<f64>, at: f64, now: f64) -> bool {
        if own.is_empty() || !self.has(id) {
            return false;
        }
        let mut uniq: Vec<f64> = Vec::new();
        for p in own {
            if !uniq.contains(&p) {
                uniq.push(p);
            }
        }
        self.waiting.push(Waiting { actor: id.into(), app, place, own: uniq, at, until: now + AWAIT_MS });
        if self.waiting.len() > AWAIT_MAX {
            let extra = self.waiting.len() - AWAIT_MAX;
            self.waiting.drain(0..extra);
        }
        true
    }

    /// The spawned reports that still wait at `at`: those whose time is up,
    /// or whose actor is gone, let go (the attribution's first look).
    pub fn expire_waiting(&mut self, at: f64) -> Vec<Waiting> {
        let mut i = self.waiting.len();
        while i > 0 {
            i -= 1;
            let w = &self.waiting[i];
            if at >= w.until || !self.all.iter().any(|a| a.id == w.actor) {
                self.waiting.remove(i);
            }
        }
        self.waiting.clone()
    }

    /// A waiting report, now named by the session it waited for.
    pub fn drop_waiting_at(&mut self, i: usize) {
        if i < self.waiting.len() {
            self.waiting.remove(i);
        }
    }

    /// The process a registered actor said it runs as when it registered.
    pub fn watch_pid(&mut self, id: &str, pid: &Value) -> Result<()> {
        let i = self.need_index(id)?;
        self.all[i].pid = pid.as_f64().filter(|p| p.is_finite() && p.trunc() == *p && *p > 1.0);
        Ok(())
    }

    // ---------- presence ----------

    /// One of an actor's instances says it's there (`o`: instance,
    /// presence, status, needsYou, heartbeatMs, leaseMs, pid, cwd, via, at).
    pub fn report(&mut self, id: &str, o: &Value, host: &mut dyn Host) -> Result<()> {
        let i = self.need_index(id)?;
        let instance = o.get("instance").map_or_else(|| "main".to_string(), display);
        let presence = o.get("presence");
        let at = match o.get("at") {
            None => host.now(),
            Some(v) => v.as_f64().unwrap_or(f64::NAN),
        };
        let a = &mut self.all[i];
        // A presence kept across a restart gives way to the first source
        // that speaks: the same instance carries on from it, and any other
        // replaces it.
        a.instances.retain_mut(|(name, inst)| {
            if !inst.restored {
                return true;
            }
            if *name == instance {
                inst.restored = false;
                true
            } else {
                false
            }
        });
        let heartbeat = o.get("heartbeatMs");
        let pid = o.get("pid");
        if a.instance(&instance).is_none() {
            let inst = Instance {
                presence: presence.filter(|p| !p.is_null()).map_or_else(|| "idle".into(), display),
                at,
                heartbeat_ms: heartbeat.map_or(a.heartbeat_ms, Value::as_f64),
                pid: pid.and_then(Value::as_f64),
                restored: false,
                lease_ms: None,
            };
            a.instances.push((instance.clone(), inst));
            // At most INSTANCES_MAX: the one silent longest goes.
            if a.instances.len() > INSTANCES_MAX {
                let mut others: Vec<(usize, f64)> = a.instances.iter().enumerate().filter(|(_, (n, _))| *n != instance).map(|(j, (_, x))| (j, x.at)).collect();
                xenonet_text::sort::sort_by(&mut others, |x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((j, _)) = others.first() {
                    a.instances.remove(*j);
                }
            }
        }
        let inst = &mut a.instances.iter_mut().find(|(n, _)| *n == instance).expect("added above when it wasn't there").1;
        if let Some(p) = presence {
            inst.presence = display(p);
        }
        if let Some(h) = heartbeat {
            inst.heartbeat_ms = h.as_f64();
        }
        if let Some(v) = o.get("leaseMs") {
            inst.lease_ms = v.as_f64();
        }
        if let Some(p) = pid {
            inst.pid = p.as_f64();
        }
        inst.at = js_max(inst.at, at);
        if let Some(v) = o.get("cwd") {
            a.cwd = v.clone();
        }
        a.last_seen = Some(js_max(a.last_seen.unwrap_or(0.0), at));
        a.dirty = true;
        match o.get("needsYou") {
            None => {}
            Some(n) if truthy(Some(n)) => {
                let text = status_line(Some(&col(n, "text"))).ok().flatten().filter(|s| !s.is_empty()).unwrap_or_else(|| "Waiting for you".into());
                let since = match n.get("since") {
                    None | Some(Value::Null) => number(at),
                    Some(v) => v.clone(),
                };
                a.needs_you = Some(json!({ "kind": template(n.get("kind")), "text": text, "since": since }));
            }
            Some(_) => a.needs_you = None,
        }
        if let Some(u) = o.get("usage") {
            a.usage = clean_usage(u);
        }
        let via = via_of(o);
        let status = o.get("status").cloned();
        self.transaction(|me| {
            me.settle(i, at, via.as_deref(), host)?;
            if let Some(status) = &status {
                me.set_status_line(i, Some(status), via.as_deref(), at, &Value::Null)?;
            }
            Ok(())
        })?;
        self.mark_changed();
        Ok(())
    }

    /// What an actor's presence is at `now`, by section 1's rules: each
    /// instance's word, unless it has missed two heartbeats (away) or ten
    /// (offline), or its lease ran out (then it says nothing); the most
    /// present instance wins; an action it reported in the last 90 s is
    /// working; and with no word yet, unknown.
    pub fn presence_of(a: &Actor, now: f64) -> String {
        let mut best: Option<String> = None;
        for (_, inst) in &a.instances {
            let mut p = inst.presence.clone();
            if let Some(lease) = inst.lease_ms {
                if now - inst.at > lease {
                    continue;
                }
            }
            if p != "offline" && p != "unknown" {
                if let Some(hb) = inst.heartbeat_ms.filter(|h| *h != 0.0 && !h.is_nan()) {
                    let silent = now - inst.at;
                    if silent > 10.0 * hb {
                        p = "offline".into();
                    } else if silent > 2.0 * hb + GRACE_MS {
                        p = "away".into();
                    }
                }
            }
            if best.as_ref().is_none_or(|b| rank(&p) > rank(b)) {
                best = Some(p);
            }
        }
        let mut best = best.unwrap_or_else(|| "unknown".into());
        if best != "offline" && best != "working" && a.acted_at.is_some_and(|t| now - t < ACTED_MS) {
            best = "working".into();
        }
        best
    }

    // A change of presence is an event, and its time is presenceSince.
    fn settle(&mut self, i: usize, at: f64, via: Option<&str>, _host: &mut dyn Host) -> Result<()> {
        let a = &mut self.all[i];
        let t = js_max(at, a.settled_at.unwrap_or(f64::NEG_INFINITY));
        a.settled_at = Some(t);
        let p = Self::presence_of(a, t);
        if p == a.presence {
            return Ok(());
        }
        let from = std::mem::replace(&mut a.presence, p.clone());
        a.presence_since = Some(t);
        let detail = presence_words(Some(&Value::String(from)), Some(&p));
        self.event(i, "presence", &Value::Null, Some(detail.to_string()), via, t, None, &Value::Null)?;
        Ok(())
    }

    // A new status is an event; the same one again isn't, and one that
    // looks like it holds a key never gets in. A status given as undefined
    // reads as no status, as null does.
    fn set_status_line(&mut self, i: usize, text: Option<&Value>, via: Option<&str>, at: f64, target: &Value) -> Result<()> {
        let Ok(line) = status_line(Some(text.unwrap_or(&Value::Null))) else { return Ok(()) };
        if line == self.all[i].status {
            return Ok(());
        }
        let a = &mut self.all[i];
        a.status.clone_from(&line);
        a.status_at = Some(at);
        // A status cleared isn't news.
        if let Some(l) = line {
            self.event(i, "status", target, Some(l), via, at, None, &Value::Null)?;
        }
        Ok(())
    }

    // ---------- events ----------

    /// Something an actor did, kept and sent on (`o`: kind, target,
    /// relation, status, via, at, ref, and the facts).
    pub fn record(&mut self, id: &str, o: &Value, host: &mut dyn Host) -> Result<Value> {
        let kind_given = o.get("kind");
        let kind = template(kind_given);
        if kind_given.and_then(Value::as_str) == Some("presence") {
            return err("presence comes from reports");
        }
        let i = self.need_index(id)?;
        let target = col(o, "target");
        let at = match o.get("at") {
            None => host.now(),
            Some(v) => v.as_f64().unwrap_or(f64::NAN),
        };
        let r = col(o, "ref");
        let via = via_of(o);
        let mut facts = Map::new();
        for k in FACTS {
            if let Some(v) = o.get(k).filter(|v| !v.is_null()) {
                facts.insert(k.into(), v.clone());
            }
        }
        let facts = Value::Object(facts);
        let relation = o.get("relation").cloned();
        let status = o.get("status").cloned();
        let event = self.transaction(|me| {
            let event = if kind == "status" {
                let line = status_line(Some(status.as_ref().unwrap_or(&Value::Null))).ok().flatten().filter(|l| !l.is_empty());
                let Some(line) = line else { return err("a status event needs a line of its own") };
                me.all[i].status = Some(line.clone());
                me.all[i].status_at = Some(at);
                me.event(i, &kind, &target, Some(line), via.as_deref(), at, Some(&facts), &r)?
            } else {
                let at_where = me.where_of(&target, host);
                let detail = write_detail(&kind, Some(&facts), Some(&at_where)).map_err(Fail::Error)?;
                me.event(i, &kind, &target, detail, via.as_deref(), at, Some(&facts), &r)?
            };
            let rel = match &relation {
                None => relation_for(&kind, &target).map(str::to_string),
                Some(Value::Null) => None,
                Some(v) => Some(display(v)),
            };
            if let (Some(t), Some(rel)) = (target.as_str().filter(|t| !t.is_empty()), rel.filter(|r| !r.is_empty())) {
                if node_kind(t) != Some("actor") {
                    me.touch(i, t, &rel, at)?;
                }
            }
            // A registered actor that reports an action is working for a
            // while after it; an observed one is whatever its observer sees.
            if ACTIONS.contains(&kind.as_str()) && event.get("via").and_then(Value::as_str) == Some("registered") {
                let now = host.now();
                let a = &mut me.all[i];
                a.acted_at = Some(js_max(a.acted_at.unwrap_or(0.0), js_min(at, now)));
            }
            me.settle(i, at, via.as_deref(), host)?;
            Ok(event)
        })?;
        self.mark_changed();
        Ok(event)
    }

    #[expect(clippy::too_many_arguments, reason = "an event and what it records, one argument each")]
    fn event(&mut self, i: usize, kind: &str, target: &Value, detail: Option<String>, via: Option<&str>, at: f64, facts: Option<&Value>, r: &Value) -> Result<Value> {
        let a = &mut self.all[i];
        a.seq += 1.0;
        let via = via.map_or_else(|| a.via.iter().find(|v| *v != "demo").cloned().unwrap_or_else(|| "observed".into()), str::to_string);
        let detail = detail.map(|d| clip(&d, 200));
        let event = json!({
            "schema": "actor/event/v1",
            "id": format!("{}:{}", a.slug, xenonet_text::json::number_string(a.seq)),
            "actor": a.id,
            "at": number(at),
            "kind": kind,
            "target": target,
            "detail": detail,
            "via": via,
        });
        a.last_seen = Some(js_max(a.last_seen.unwrap_or(0.0), at));
        let facts = facts.filter(|f| f.as_object().is_some_and(|o| !o.is_empty())).map_or(Value::Null, |f| Value::String(xenonet_text::json::stringify(f, 0)));
        let row = json!({
            "schema": "actor/event/v1",
            "id": event["id"],
            "actor": a.id,
            "at": number(at),
            "kind": kind,
            "target": target,
            "detail": event["detail"],
            "via": event["via"],
            "facts": facts,
            "ref": r,
        });
        self.store.add_actor_event(&row)?;
        self.save(i)?;
        let a = &mut self.all[i];
        // Newest last, whatever order they came in.
        let mut k = a.recent.len();
        while k > 0 && a.recent[k - 1].get("at").and_then(Value::as_f64).is_some_and(|t| t > at) {
            k -= 1;
        }
        a.recent.insert(k, event.clone());
        if a.recent.len() > RECENT as usize {
            let extra = a.recent.len() - RECENT as usize;
            a.recent.drain(0..extra);
        }
        if self.is_public(&self.all[i].id) {
            self.emits.push(("event", event.clone()));
        }
        Ok(event)
    }

    /// What the map says of a target, for details: a session's app and the
    /// place it lives, or a place's name. Never a session's title.
    pub fn where_of(&mut self, target: &Value, host: &mut dyn Host) -> Value {
        let Some(k) = node_kind_of(target) else { return Value::Null };
        if k == "actor" {
            return json!({ "kind": "actor" });
        }
        let d = host.describe(&template(Some(target)));
        if !truthy(Some(&d)) {
            return json!({ "kind": k });
        }
        if d.get("kind").and_then(Value::as_str) == Some("session") {
            let place = d.get("place").filter(|p| !p.is_null()).map_or(Value::Null, |p| Value::String(place_name(&template(Some(p)), Some("project"))));
            return json!({ "kind": "session", "app": d.get("app").cloned().filter(|a| !a.is_null()).unwrap_or(Value::Null), "place": place });
        }
        let kind = d.get("kind").cloned().unwrap_or(Value::Null);
        let name = d.get("name").filter(|p| !p.is_null()).map_or(Value::Null, |n| Value::String(place_name(&template(Some(n)), kind.as_str())));
        json!({ "kind": kind, "name": name, "place": name })
    }

    // ---------- targets ----------

    /// Ties an actor to a node of the map it acts on.
    pub fn target(&mut self, id: &str, node: &str, relation: &str, at: f64) -> Result<()> {
        let i = self.need_index(id)?;
        if node_kind(node).is_none_or(|k| k == "actor") {
            return err(format!("not a node of the map: {node}"));
        }
        self.touch(i, node, relation, at)?;
        self.mark_changed();
        Ok(())
    }

    pub fn untarget(&mut self, id: &str, node: &str, relation: &str) -> Result<()> {
        let i = self.need_index(id)?;
        if self.all[i].drop_target(&format!("{relation}|{node}")).is_some() {
            self.store.drop_actor_target(id, node, relation)?;
        }
        self.mark_changed();
        Ok(())
    }

    fn touch(&mut self, i: usize, node: &str, relation: &str, at: f64) -> Result<()> {
        let key = format!("{relation}|{node}");
        let a = &mut self.all[i];
        let t = match a.target_mut(&key) {
            Some(t) => {
                t.since = js_min(t.since, at);
                t.last_at = js_max(t.last_at, at);
                t.clone()
            }
            None => {
                let t = Target { id: node.into(), relation: relation.into(), since: at, last_at: at };
                a.targets.push((key.clone(), t.clone()));
                t
            }
        };
        let actor = a.id.clone();
        let row = json!({ "actor": actor, "target": node, "relation": relation, "since": number(t.since), "last_at": number(t.last_at) });
        self.store.save_actor_target(&row)?;
        let a = &mut self.all[i];
        if a.targets.len() > TARGETS_KEPT {
            let mut order: Vec<(usize, f64)> = a.targets.iter().enumerate().map(|(j, (_, t))| (j, t.last_at)).collect();
            xenonet_text::sort::sort_by(&mut order, |x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
            let gone = a.targets.remove(order[0].0).1;
            self.store.drop_actor_target(&actor, &gone.id, &gone.relation)?;
        }
        Ok(())
    }

    fn map_ready(host: &mut dyn Host) -> bool {
        host.has_map() && host.ready() != Some(false)
    }

    fn known(t: &Target, now: f64, host: &mut dyn Host) -> bool {
        if !Self::map_ready(host) {
            return true;
        }
        host.has(&t.id) == Some(true) || (node_kind(&t.id) == Some("session") && now - t.since < SESSION_WAIT_MS)
    }

    // ---------- what the console and the API see ----------

    fn counts(&self, now: f64) -> Result<Vec<(String, [f64; 3])>> {
        let mut out: Vec<(String, [f64; 3])> = Vec::new();
        for r in self.store.actor_event_counts(now - DAY_MS)? {
            let actor = text(&r, "actor").unwrap_or("").to_string();
            let kind = text(&r, "kind").unwrap_or("");
            let n = num(&r, "n").unwrap_or(0.0);
            let i = match out.iter().position(|(a, _)| *a == actor) {
                Some(i) => i,
                None => {
                    out.push((actor, [0.0; 3]));
                    out.len() - 1
                }
            };
            let c = &mut out[i].1;
            c[0] += n;
            if kind == "spawned" {
                c[1] += n;
            }
            if kind == "messaged" || kind == "said" {
                c[2] += n;
            }
        }
        Ok(out)
    }

    fn counts_json(counts: &[(String, [f64; 3])], id: &str) -> Value {
        let c = counts.iter().find(|(a, _)| a == id).map_or([0.0; 3], |(_, c)| *c);
        json!({ "events24h": number(c[0]), "spawned24h": number(c[1]), "messages24h": number(c[2]) })
    }

    /// One actor as an actor/snapshot/v1, or null. Outside the window
    /// (`view` other than "window"), you are the placeholder.
    pub fn snapshot(&mut self, id: &str, view: &str, now: Option<f64>, counts: Option<&[(String, [f64; 3])]>, host: &mut dyn Host) -> Result<Value> {
        if self.get(id).is_none() {
            return Ok(Value::Null);
        }
        if view != "window" && !self.is_public(id) {
            return Ok(placeholder());
        }
        let now = now.unwrap_or_else(|| host.now());
        let own;
        let counts = match counts {
            Some(c) => c,
            None => {
                own = self.counts(now)?;
                &own
            }
        };
        let Some(a) = self.get(id).cloned() else { return Ok(Value::Null) };
        let mut targets: Vec<&Target> = Vec::new();
        for (_, t) in &a.targets {
            if now - t.last_at < DAY_MS && Self::known(t, now, host) {
                targets.push(t);
            }
        }
        xenonet_text::sort::sort_by(&mut targets, |x, y| y.last_at.partial_cmp(&x.last_at).unwrap_or(std::cmp::Ordering::Equal));
        let targets: Vec<Value> =
            targets.into_iter().take(TARGETS_SHOWN).map(|t| json!({ "id": t.id, "relation": t.relation, "since": number(t.since), "lastAt": number(t.last_at) })).collect();
        let mut o = Map::new();
        o.insert("schema".into(), json!("actor/snapshot/v1"));
        o.insert("id".into(), json!(a.id));
        o.insert("kind".into(), json!(a.kind));
        o.insert("name".into(), a.name.clone());
        o.insert("product".into(), a.product.clone());
        o.insert("host".into(), Value::Null);
        o.insert("via".into(), json!(a.via));
        o.insert("presence".into(), json!(a.presence));
        o.insert("presenceSince".into(), opt(a.presence_since));
        o.insert("lastSeen".into(), opt(a.last_seen));
        o.insert("heartbeatMs".into(), opt(a.heartbeat_ms));
        o.insert("status".into(), a.status.clone().map_or(Value::Null, Value::String));
        if let Some(n) = &a.needs_you {
            o.insert("needsYou".into(), n.clone());
        }
        if let Some(u) = &a.usage {
            o.insert("usage".into(), u.clone());
        }
        o.insert("capabilities".into(), Value::Array(a.capabilities.clone()));
        o.insert("targets".into(), Value::Array(targets));
        o.insert("recent".into(), Value::Array(a.recent.clone()));
        o.insert("counts".into(), Self::counts_json(counts, &a.id));
        o.insert("identity".into(), a.identity.clone());
        if let Some(r) = &a.remote {
            o.insert(
                "remote".into(),
                json!({ "connections": r["connections"], "active": r["active"], "bridge": r["bridge"], "accountLink": r["accountLink"], "permissions": r["permissions"], "transports": r["transports"] }),
            );
        }
        Ok(Value::Object(o))
    }

    fn listed(a: &Actor, now: f64) -> bool {
        if a.id == YOU || a.allowed_at.is_some() || a.remote.as_ref().is_some_and(|r| truthy(r.get("active"))) {
            return true;
        }
        if !a.enabled {
            return false;
        }
        a.has_via("demo") || a.installed == Some(true) || a.last_seen.is_some_and(|l| now - l < WEEK_MS)
    }

    fn source(a: &Actor) -> String {
        a.via.iter().find(|v| *v != "demo").cloned().unwrap_or_else(|| "observed".into())
    }

    /// Who started which sessions, for what each actor's work costs:
    /// `names` (each listed actor's name by its id) and `starts`.
    pub fn sessions_started(&self, now: f64) -> Result<Started> {
        let names: Vec<(String, Value)> = self.all.iter().filter(|a| self.is_public(&a.id) && Self::listed(a, now)).map(|a| (a.id.clone(), a.name.clone())).collect();
        let has = |id: &str| names.iter().any(|(n, _)| n == id);
        let mut starts = Vec::new();
        for r in self.store.actor_spawns(now - WEEK_MS)? {
            let actor = text(&r, "actor").unwrap_or("");
            if has(actor) && text(&r, "target").and_then(node_kind) == Some("session") {
                starts.push(json!({ "actor": actor, "session": col(&r, "target"), "at": col(&r, "at"), "via": col(&r, "via") }));
            }
        }
        for (id, _) in &names {
            let Some(a) = self.get(id) else { continue };
            let via = if a.has_via("registered") { "registered".to_string() } else { Self::source(a) };
            for (_, t) in &a.targets {
                if t.relation == "spawned" && node_kind(&t.id) == Some("session") {
                    starts.push(json!({ "actor": id, "session": t.id, "at": number(t.since), "via": via }));
                }
            }
        }
        Ok((names, starts))
    }

    // ---------- what the engine sees of actors ----------

    /// The project or folder holding an absolute path, as a node id, or
    /// null.
    pub fn place_of(&self, p: &Value, host: &mut dyn Host) -> Value {
        let Some(p) = p.as_str() else { return Value::Null };
        if !NATIVE.is_absolute(p) {
            return Value::Null;
        }
        let d = host.place_of(&style_of(p).resolve(&[p]));
        if d.is_null() {
            Value::Null
        } else {
            d
        }
    }

    /// What the engine needs of an actor, or null for you and for one that
    /// isn't here.
    pub fn engine_view(&self, id: &str, now: f64, since: Option<f64>, host: &mut dyn Host) -> Result<Value> {
        let Some(a) = self.get(id).filter(|_| id != YOU) else { return Ok(Value::Null) };
        let since = since.unwrap_or(now - EDIT_HOLD_MS);
        let mut latest: Option<&Target> = None;
        for (_, t) in &a.targets {
            if t.relation == "works-on" && node_kind(&t.id) != Some("session") && latest.is_none_or(|l| t.last_at > l.last_at) {
                latest = Some(t);
            }
        }
        let from_cwd = if truthy(Some(&a.cwd)) { self.place_of(&a.cwd, host) } else { Value::Null };
        let place_id = if truthy(Some(&from_cwd)) { from_cwd } else { latest.map_or(Value::Null, |t| json!(t.id)) };
        let d = if truthy(Some(&place_id)) { host.describe(&template(Some(&place_id))) } else { Value::Null };
        let mut edits: Vec<(String, f64, bool)> = Vec::new();
        for row in self.store.actor_facts_since(id, "edited", since)? {
            let facts = parse_json(text(&row, "facts"), json!({}));
            for p in facts.get("paths").and_then(Value::as_array).cloned().unwrap_or_default() {
                if let Some(p) = p.as_str() {
                    if !edits.iter().any(|(x, _, _)| x == p) {
                        edits.push((p.into(), num(&row, "at").unwrap_or(f64::NAN), text(&row, "via") != Some("registered")));
                    }
                }
            }
        }
        let place = if truthy(Some(&d)) {
            json!({ "id": place_id, "name": d.get("name").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null), "kind": d.get("kind").cloned().unwrap_or(Value::Null), "path": d.get("path").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null) })
        } else {
            Value::Null
        };
        Ok(json!({
            "id": id,
            "name": a.name,
            "via": a.via,
            "product": a.product,
            "presence": a.presence,
            "status": a.status,
            "lastSeen": opt(a.last_seen),
            "cwd": a.cwd,
            "place": place,
            "capabilities": a.capabilities,
            "edits": edits.into_iter().map(|(p, at, observed)| if observed { json!({ "path": p, "at": number(at), "observed": true }) } else { json!({ "path": p, "at": number(at) }) }).collect::<Vec<_>>(),
        }))
    }

    /// An actor's own status line, set through the engine's focus (`text`
    /// empty for none; `via` None for registered): Ok(status) or
    /// Err(reason).
    pub fn set_status(&mut self, id: &str, text: &str, via: Option<&str>, at: f64) -> Result<std::result::Result<Value, &'static str>> {
        let text = Value::String(text.to_string());
        if let Err(reason) = status_line(Some(&text)) {
            return Ok(Err(reason));
        }
        let i = self.need_index(id)?;
        let via = via.unwrap_or("registered");
        self.transaction(|me| me.set_status_line(i, Some(&text), Some(via), at, &Value::Null))?;
        self.all[i].dirty = true;
        self.mark_changed();
        Ok(Ok(self.all[i].status.clone().map_or(Value::Null, Value::String)))
    }

    /// A message an actor sent through the engine: a said event in
    /// XenoNet's words, never the text, and a messaged target for each
    /// session it reached.
    pub fn said(&mut self, from: &str, to: &[String], ts: Option<f64>, host: &mut dyn Host) -> Result<Value> {
        if from == YOU || !self.has(from) || to.is_empty() {
            return Ok(Value::Null);
        }
        let at = ts.unwrap_or_else(|| host.now());
        let sessions: Vec<&String> = to.iter().filter(|id| node_kind(id) == Some("session")).collect();
        let target = if sessions.len() == 1 {
            json!(sessions[0])
        } else {
            let v = self.engine_view(from, at, None, host)?;
            v.get("place").and_then(|p| p.get("id")).cloned().filter(|v| !v.is_null()).unwrap_or(Value::Null)
        };
        let event = self.transaction(|me| {
            let o = json!({ "kind": "said", "target": target, "count": number(to.len() as f64), "via": "registered", "at": number(at) });
            let event = me.record(from, &o, host)?;
            let i = me.need_index(from)?;
            for s in &sessions {
                me.touch(i, s, "messaged", at)?;
            }
            Ok(event)
        })?;
        Ok(event)
    }

    /// The actors the engine counts among the agents at work.
    pub fn engine_ids(&self, since: f64, now: f64) -> Vec<String> {
        self.all
            .iter()
            .filter(|a| a.id != YOU && a.last_seen.is_some_and(|l| l >= since) && ["working", "idle", "away"].contains(&a.presence.as_str()) && Self::listed(a, now))
            .map(|a| a.id.clone())
            .collect()
    }

    /// Each registered actor's token, for XenoNet's window alone.
    pub fn tokens(&self, now: f64) -> Result<Vec<Value>> {
        let rows = self.store.actor_token_times()?;
        let row = |slug: &str| rows.iter().rev().find(|r| r.get("slug").and_then(Value::as_str) == Some(slug));
        let mut list: Vec<&Actor> = self.all.iter().filter(|a| a.id != YOU && self.is_registered(&a.id)).collect();
        xenonet_text::sort::sort_by(&mut list, |x, y| x.created_at.partial_cmp(&y.created_at).unwrap_or(std::cmp::Ordering::Equal));
        Ok(list
            .into_iter()
            .map(|a| {
                let r = row(&a.slug);
                let has_token = r.is_some_and(|r| truthy(r.get("hasToken")));
                let r_at = r.and_then(|r| r.get("at")).and_then(Value::as_f64);
                // When the token was used, if it still works, and if it was
                // used since it was allowed.
                let works_at = r_at.filter(|_| has_token && !token_expired(r_at, now));
                let works = works_at.is_some();
                let used_at = r_at.filter(|at| has_token && a.allowed_at.is_some_and(|allowed| *at > allowed));
                let has_gateway = r.is_some_and(|r| truthy(r.get("hasGateway")));
                let state = if works {
                    "works"
                } else if a.allowed_at.is_some() && !has_gateway {
                    "expired"
                } else {
                    "none"
                };
                let gateway_at = r.and_then(|r| r.get("gatewayAt")).and_then(Value::as_f64);
                let mut o = Map::new();
                o.insert("actor".into(), json!(a.id));
                o.insert("state".into(), json!(state));
                o.insert("expiresAt".into(), works_at.map_or(Value::Null, |at| number(at + TOKEN_UNUSED_MS)));
                o.insert("usedAt".into(), used_at.map_or(Value::Null, number));
                if let Some(g) = gateway_at.filter(|_| has_gateway && !token_expired(gateway_at, now)) {
                    o.insert(
                        "gateway".into(),
                        json!({ "state": "works", "expiresAt": number(g + TOKEN_UNUSED_MS), "usedAt": if g > a.allowed_at.unwrap_or(0.0) { number(g) } else { Value::Null } }),
                    );
                }
                Value::Object(o)
            })
            .collect())
    }

    /// The actors as an actor/list/v1: you first, then the oldest first, at
    /// most 32 (those seen least lately go).
    pub fn list(&mut self, view: &str, now: Option<f64>, host: &mut dyn Host) -> Result<Value> {
        let now = now.unwrap_or_else(|| host.now());
        let counts = self.counts(now)?;
        let mut shown: Vec<(String, bool, f64, f64)> =
            self.all.iter().filter(|a| Self::listed(a, now)).map(|a| (a.id.clone(), a.id == YOU, a.last_seen.unwrap_or(0.0), a.created_at)).collect();
        if shown.len() > LIST_MAX {
            let mut by_seen = shown.clone();
            xenonet_text::sort::sort_by(&mut by_seen, |x, y| {
                let you = i32::from(y.1) - i32::from(x.1);
                if you != 0 {
                    return you.cmp(&0);
                }
                let d = y.2 - x.2;
                if d > 0.0 {
                    std::cmp::Ordering::Greater
                } else if d < 0.0 {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            let keep: Vec<String> = by_seen.into_iter().take(LIST_MAX).map(|s| s.0).collect();
            shown.retain(|s| keep.contains(&s.0));
        }
        xenonet_text::sort::sort_by(&mut shown, |x, y| {
            let you = i32::from(y.1) - i32::from(x.1);
            if you != 0 {
                return you.cmp(&0);
            }
            let d = x.3 - y.3;
            if d > 0.0 {
                std::cmp::Ordering::Greater
            } else if d < 0.0 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        });
        let mut actors = Vec::new();
        for (id, ..) in shown {
            actors.push(self.snapshot(&id, view, Some(now), Some(&counts), host)?);
        }
        Ok(json!({ "schema": "actor/list/v1", "at": number(now), "actors": actors }))
    }

    /// The list, when what it says has changed since the last one sent
    /// (and it's sent now), or null.
    pub fn list_if_changed(&mut self, host: &mut dyn Host) -> Result<Value> {
        let list = self.list("public", None, host)?;
        let said = xenonet_text::json::stringify(&list["actors"], 0);
        if self.last_list.as_deref() == Some(said.as_str()) {
            return Ok(Value::Null);
        }
        self.last_list = Some(said);
        Ok(list)
    }

    /// Events since `since` (the last 7 days at most), oldest first (`o`:
    /// since, limit, view, only).
    pub fn events(&self, o: &Value, now: f64) -> Result<Value> {
        let since = o.get("since").and_then(Value::as_f64).filter(|f| f.is_finite()).unwrap_or(now - DAY_MS);
        let from = js_max(since, now - WEEK_MS);
        let limit = match o.get("limit") {
            None => 1000.0,
            Some(v) => v.as_f64().unwrap_or(f64::NAN),
        };
        let floor = limit.floor();
        let n = js_min(js_max(1.0, if floor == 0.0 || floor.is_nan() { 1.0 } else { floor }), EVENTS_MAX);
        let view = o.get("view").map_or_else(|| "public".to_string(), display);
        let only = o.get("only").filter(|v| truthy(Some(v))).map(display);
        if let Some(only) = &only {
            if view != "window" && !self.is_public(only) {
                return Ok(json!({ "schema": "actor/events/v1", "at": number(now), "events": [] }));
            }
        }
        let except = if only.is_some() || view == "window" || self.demo { None } else { Some(YOU) };
        let rows = self.store.actor_events_since(from, n, only.as_deref(), except)?;
        Ok(json!({ "schema": "actor/events/v1", "at": number(now), "events": rows.iter().map(event_of).collect::<Vec<_>>() }))
    }

    // ---------- the clock ----------

    /// Every CLOCK_MS: processes that ended, presence from silence,
    /// sessions that never reached the map let go, heartbeats written down.
    pub fn tick(&mut self, now: f64, host: &mut dyn Host) -> Result<()> {
        self.transaction(|me| {
            for i in 0..me.all.len() {
                me.ended(i, now, host);
                me.settle(i, now, None, host)?;
                if !Self::map_ready(host) {
                    continue;
                }
                let gone: Vec<(String, Target)> =
                    me.all[i].targets.iter().filter(|(_, t)| node_kind(&t.id) == Some("session") && now - t.since >= SESSION_WAIT_MS).cloned().collect();
                for (key, t) in gone {
                    if host.has(&t.id) == Some(true) {
                        continue;
                    }
                    me.all[i].drop_target(&key);
                    let actor = me.all[i].id.clone();
                    me.store.drop_actor_target(&actor, &t.id, &t.relation)?;
                }
            }
            me.flush()
        })?;
        self.mark_changed();
        Ok(())
    }

    // The processes an actor named, watched.
    fn ended(&mut self, i: usize, now: f64, host: &mut dyn Host) {
        let pids: Vec<(usize, f64)> = self.all[i].instances.iter().enumerate().filter_map(|(j, (_, inst))| inst.pid.filter(|p| *p != 0.0 && !p.is_nan()).map(|p| (j, p))).collect();
        for (j, pid) in pids {
            if !host.alive(pid) {
                let inst = &mut self.all[i].instances[j].1;
                inst.presence = "offline".into();
                inst.pid = None;
            }
        }
        let Some(pid) = self.all[i].pid else { return };
        if host.alive(pid) {
            return;
        }
        let a = &mut self.all[i];
        a.pid = None;
        if a.instances.is_empty() {
            a.instances.push(("main".into(), Instance { presence: "offline".into(), at: now, heartbeat_ms: a.heartbeat_ms, pid: None, restored: false, lease_ms: None }));
        }
        for (_, inst) in &mut a.instances {
            inst.presence = "offline".into();
        }
    }

    /// What's kept (the store's prune), and what's in memory after it.
    pub fn prune(&mut self, now: f64) -> Result<()> {
        self.flush()?;
        self.store.prune(now)?;
        let targets: Vec<String> = self
            .store
            .actor_targets()?
            .iter()
            .map(|t| format!("{}|{}|{}", text(t, "actor").unwrap_or(""), text(t, "relation").unwrap_or(""), text(t, "target").unwrap_or("")))
            .collect();
        for i in 0..self.all.len() {
            let id = self.all[i].id.clone();
            let recent: Vec<Value> = self.store.last_actor_events(&id, RECENT)?.iter().map(event_of).collect();
            let a = &mut self.all[i];
            a.recent = recent;
            a.targets.retain(|(key, _)| targets.contains(&format!("{id}|{key}")));
            if id == YOU && a.status_at.is_some_and(|t| now - t >= DAY_MS) {
                a.status = None;
                a.status_at = None;
            }
        }
        self.mark_changed();
        Ok(())
    }

    /// Writes down what only heard a heartbeat: when the clock stops.
    pub fn stop(&mut self) -> Result<()> {
        self.flush()
    }

    // ---------- XenoNet Dev's simulations ----------

    /// A simulation (actor-spawn, actor-edit, …, clear): what it did, or
    /// null.
    pub fn simulate(&mut self, kind: &str, a: Value, b: Value, host: &mut dyn Host) -> Result<Value> {
        if kind == "clear" {
            if !self.demo {
                return Ok(Value::Null);
            }
            let held: Vec<String> = self.all.iter().filter(|x| x.simulated_offline).map(|x| x.id.clone()).collect();
            for id in held {
                if let Some(x) = self.get_mut(&id) {
                    x.simulated_offline = false;
                }
                self.report(&id, &json!({ "instance": "main", "presence": "working" }), host)?;
            }
            for i in 0..self.all.len() {
                if self.all[i].needs_you.is_some() {
                    self.all[i].needs_you = None;
                    self.all[i].dirty = true;
                    self.mark_changed();
                }
            }
            let tunnels: Vec<String> = self.all.iter().filter(|x| x.simulated_remote).map(|x| x.id.clone()).collect();
            for id in tunnels {
                if let Some(x) = self.get_mut(&id) {
                    x.simulated_remote = false;
                }
                self.set_remote(&id, &Value::Null)?;
            }
            return Ok(json!({ "ok": true, "kind": kind }));
        }
        let a =
            if a.is_null() { self.all.iter().find(|x| x.kind == "agent").map(|x| x.id.clone()).or_else(|| self.demo.then(|| YOU.to_string())) } else { Some(template(Some(&a))) };
        let Some(a) = a else { return Ok(Value::Null) };
        if !self.has(&a) || !self.is_public(&a) {
            return Ok(Value::Null);
        }
        let session = if b.is_null() { host.recent_session() } else { b.clone() };
        let rec = |kind: &str, target: &Value, count: bool| {
            let mut o = Map::new();
            o.insert("kind".into(), json!(kind));
            o.insert("target".into(), target.clone());
            if count {
                o.insert("count".into(), json!(1));
            }
            Value::Object(o)
        };
        match kind {
            "actor-spawn" => {
                self.record(&a, &rec("spawned", &session, false), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a, "b": session }))
            }
            "actor-edit" => {
                let mut b = b;
                if b.is_null() {
                    let d = if truthy(Some(&session)) { host.describe(&template(Some(&session))) } else { Value::Null };
                    b = if d.get("kind").and_then(Value::as_str) == Some("session") { d.get("placeId").filter(|p| !p.is_null()).cloned().unwrap_or(session) } else { session };
                }
                self.record(&a, &rec("edited", &b, true), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a, "b": b }))
            }
            "actor-message" => {
                self.record(&a, &rec("said", &session, true), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a, "b": session }))
            }
            "actor-online" => {
                if let Some(x) = self.get_mut(&a) {
                    x.simulated_offline = false;
                }
                self.report(&a, &json!({ "instance": "main", "presence": "working" }), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a }))
            }
            "actor-offline" => {
                let demo = self.demo;
                let now = host.now();
                let Some(x) = self.get_mut(&a) else { return Ok(Value::Null) };
                if demo {
                    x.simulated_offline = true;
                }
                if x.instances.is_empty() {
                    x.instances.push(("main".into(), Instance { presence: "offline".into(), at: now, heartbeat_ms: x.heartbeat_ms, pid: None, restored: false, lease_ms: None }));
                }
                for (_, inst) in &mut x.instances {
                    inst.presence = "offline".into();
                }
                let first = x.instances[0].0.clone();
                self.report(&a, &json!({ "instance": first }), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a }))
            }
            "actor-tunnel" | "actor-tunnel-down" => {
                if !self.demo || self.get(&a).is_none_or(|x| x.kind != "agent") {
                    return Ok(Value::Null);
                }
                if let Some(x) = self.get_mut(&a) {
                    x.simulated_remote = true;
                }
                let bridge = if kind == "actor-tunnel" { "connected" } else { "not_connected" };
                self.set_remote(&a, &json!({ "simulated": true, "active": true, "connections": 1, "bridge": bridge, "accountLink": "unverified", "permissions": ["read"], "transports": ["openai-secure-mcp-tunnel"] }))?;
                Ok(json!({ "ok": true, "kind": kind, "a": a }))
            }
            "actor-needs-you" => {
                if !self.demo || self.get(&a).is_none_or(|x| x.kind != "agent") {
                    return Ok(Value::Null);
                }
                let now = host.now();
                let Some(x) = self.get_mut(&a) else { return Ok(Value::Null) };
                x.needs_you = Some(json!({ "kind": "question", "text": "Should the refunds fix go out tonight, or wait for the review?", "since": number(now) }));
                x.dirty = true;
                self.mark_changed();
                Ok(json!({ "ok": true, "kind": kind, "a": a }))
            }
            _ => Ok(Value::Null),
        }
    }

    // ---------- you, at the console ----------

    fn you_settle(&mut self, at: f64, host: &mut dyn Host) -> Result<()> {
        if let Some(p) = self.you.presence() {
            let o = json!({ "instance": "window", "presence": p, "heartbeatMs": number(crate::you::YOU_HEARTBEAT_MS), "via": "observed", "at": number(at) });
            self.report(YOU, &o, host)?;
        }
        Ok(())
    }

    fn you_place_of(session: &Value, host: &mut dyn Host) -> Value {
        let d = host.describe(&template(Some(session)));
        d.get("placeId").filter(|p| !p.is_null()).cloned().unwrap_or(Value::Null)
    }

    // One thing you did, recorded; or why it isn't.
    fn you_act(&mut self, d: &Value, now: f64, host: &mut dyn Host) -> Result<Option<String>> {
        let what = match d.get("what").and_then(Value::as_str) {
            Some(w) if truthy(Some(d)) && crate::you::known_action(w) => w.to_string(),
            _ => return Ok(Some("XenoNet doesn't know that action".into())),
        };
        let mut at = d.get("at").and_then(Value::as_f64).filter(|f| f.is_finite()).unwrap_or(now);
        if now - at > crate::you::DAY_MS {
            return Ok(Some("that was more than a day ago".into()));
        }
        if at - now > crate::you::AHEAD_MS {
            at = now;
        }
        let node = col(d, "node");
        if host.has_map() && host.has(&template(Some(&node))) != Some(true) {
            return Ok(Some("XenoNet doesn't know that node".into()));
        }
        let at_where = self.where_of(&node, host);
        let kind = at_where.get("kind").and_then(Value::as_str).map(str::to_string);
        if at_where.is_null() || !kind.as_deref().is_some_and(|k| crate::you::on(&what, k)) {
            let k = at_where.get("kind").filter(|k| !k.is_null()).map_or_else(|| "node like that".into(), |k| template(Some(k)));
            return Ok(Some(format!("you can't do that to a {k}")));
        }
        // The apps you start and hand sessions to from the window: those
        // XenoNet hands sessions to, the first when it doesn't say.
        let asked = xenonet_adapter::apps::ASKED;
        let apps = |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(|a| asked.contains(&a));
        if what == "started" {
            let app = if apps(d.get("app")) { col(d, "app") } else { json!(asked[0]) };
            let o = json!({ "kind": "spawned", "target": node, "relation": "works-on", "app": app, "via": "observed", "at": number(at) });
            self.record(YOU, &o, host)?;
            return Ok(None);
        }
        if what == "handed-off" {
            if !apps(d.get("to")) {
                return Ok(Some("a handoff says which app it went to".into()));
            }
            let place = Self::you_place_of(&node, host);
            if !truthy(Some(&place)) {
                return Ok(Some("XenoNet doesn't know where that session lives".into()));
            }
            let app = at_where.get("app").cloned().unwrap_or(Value::Null);
            let o = json!({ "kind": "spawned", "target": place, "relation": "works-on", "app": app, "to": col(d, "to"), "via": "observed", "at": number(at) });
            self.record(YOU, &o, host)?;
            return Ok(None);
        }
        let target = if what == "terminal" && kind.as_deref() == Some("session") { Self::you_place_of(&node, host) } else { node };
        let line = crate::detail::console_status(Some(&what), Some(&at_where));
        let (true, Some(line)) = (truthy(Some(&target)), line) else {
            return Ok(Some("XenoNet can't say what that is".into()));
        };
        let mut o = Map::new();
        o.insert("kind".into(), json!("status"));
        o.insert("status".into(), json!(line));
        o.insert("target".into(), target);
        if let Some(rel) = crate::you::relation(&what) {
            o.insert("relation".into(), json!(rel));
        }
        o.insert("via".into(), json!("observed"));
        o.insert("at".into(), number(at));
        self.record(YOU, &Value::Object(o), host)?;
        Ok(None)
    }

    /// The page's report, an actor/console/v1: its presence (a report
    /// without one is a heartbeat that keeps the last), and what you did.
    /// Says how many actions were taken, and why the others weren't.
    pub fn you_report(&mut self, body: &Value, at: f64, host: &mut dyn Host) -> Result<Value> {
        self.you.off = false;
        let presence = body.get("presence");
        if truthy(presence) {
            self.you.page = presence.map(display);
        }
        let did = match body.get("did") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(l)) => l.clone(),
            Some(_) => return err("body.did.forEach is not a function"),
        };
        let (accepted, dropped) = self.transaction(|me| {
            me.you_settle(at, host)?;
            let mut accepted = 0;
            let mut dropped = Vec::new();
            for (index, d) in did.iter().enumerate() {
                match me.you_act(d, at, host)? {
                    Some(why) => dropped.push(json!({ "index": index, "reason": why })),
                    None => accepted += 1,
                }
            }
            Ok((accepted, dropped))
        })?;
        Ok(json!({ "accepted": accepted, "dropped": dropped }))
    }

    /// The desktop app says the window shows, hid (away) or closed
    /// (offline).
    pub fn you_window(&mut self, state: &str, at: f64, host: &mut dyn Host) -> Result<()> {
        if !matches!(state, "shown" | "hidden" | "closed") {
            return Ok(());
        }
        self.you.shown = state.into();
        self.you_settle(at, host)
    }

    /// You turned yourself off the map: your record goes at once, and
    /// nothing more is said of you until the page reports again.
    pub fn you_forget(&mut self) -> Result<()> {
        self.you.page = None;
        self.you.off = true;
        self.forget_you()
    }

    /// The record of an actor, for `you` and the demo.
    pub fn record_of(&self, id: &str) -> Option<&Actor> {
        self.get(id)
    }
}

/// The source a report or record names (`via`), or None for the actor's
/// own.
fn via_of(o: &Value) -> Option<String> {
    o.get("via").filter(|v| !v.is_null()).map(display)
}

// Math.max and Math.min, NaN when either side is.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}
