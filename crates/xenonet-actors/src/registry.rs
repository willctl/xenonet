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
//! goes through `Host`. What it tells (an event, the list changed) is
//! queued for whoever runs it (`take_emits`, `take_changed`).
//!
//! The person at the console is seen only by the window. Everywhere else
//! actor:you is a placeholder.

use crate::detail::{clip, place_name, presence_words, status_line, write_detail};
use crate::registration::{token_expired, TOKEN_UNUSED_MS};
use crate::you::You;
use serde_json::{json, Map, Value};
use xenonet_base::ids::{app_of_session, parse_actor_id, YOU};
use xenonet_base::json::number;
use xenonet_engine::js::truthy;
use xenonet_engine::sql::{Arg, Row, SqlError};
use xenonet_engine::store::Shared;

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
    Sql(SqlError),
}

impl From<SqlError> for Fail {
    fn from(e: SqlError) -> Self {
        Fail::Sql(e)
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
        other => node_kind(&xenonet_engine::js::template(Some(other))),
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
        Some(s) => xenonet_base::json::parse(s).unwrap_or(fallback),
    }
}

fn event_of(r: &Row) -> Value {
    json!({
        "schema": "actor/event/v1",
        "id": r.get("id").to_json(),
        "actor": r.get("actor").to_json(),
        "at": r.get("at").to_json(),
        "kind": r.get("kind").to_json(),
        "target": r.get("target").to_json(),
        "detail": r.get("detail").to_json(),
        "via": r.get("via").to_json(),
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
    pub store: Shared,
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
    pub fn new(store: Shared, demo: bool, host: &mut dyn Host) -> Result<Self> {
        let mut a = Actors { store, demo, all: Vec::new(), waiting: Vec::new(), last_list: None, changed: false, emits: Vec::new(), you: You::default() };
        a.load()?;
        let via: Vec<Arg> = if demo { vec!["demo".into(), "observed".into()] } else { vec!["observed".into()] };
        a.upsert(
            &Arg::Obj(vec![
                ("id".into(), Arg::text(YOU)),
                ("kind".into(), Arg::text("human")),
                ("name".into(), Arg::text("You")),
                ("via".into(), Arg::List(via)),
                ("heartbeatMs".into(), Arg::Num(crate::you::YOU_HEARTBEAT_MS)),
            ]),
            host,
        )?;
        Ok(a)
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
        let rows = self.store.with(|s| -> std::result::Result<_, SqlError> { Ok((s.actor_targets()?, s.actor_rows()?)) })?;
        let (targets, actor_rows) = rows;
        for r in actor_rows {
            let slug = r.get("slug").to_json();
            let slug = xenonet_engine::js::template(Some(&slug));
            let id = format!("actor:{slug}");
            if parse_actor_id(&id).is_none() {
                continue;
            }
            let mut ts: Vec<(String, Target)> = Vec::new();
            for t in targets.iter().filter(|t| t.text("actor") == Some(id.as_str())) {
                let (target, relation) = (t.text("target").unwrap_or("").to_string(), t.text("relation").unwrap_or("").to_string());
                let key = format!("{relation}|{target}");
                let v = Target { id: target, relation, since: t.num("since").unwrap_or(f64::NAN), last_at: t.num("last_at").unwrap_or(f64::NAN) };
                match ts.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = v,
                    None => ts.push((key, v)),
                }
            }
            let recent: Vec<Value> = self.store.with(|s| s.last_actor_events(Arg::text(&id), Arg::Num(RECENT)))?.iter().map(event_of).collect();
            let mut a = Actor {
                id: id.clone(),
                slug: slug.clone(),
                kind: r.text("kind").unwrap_or("").to_string(),
                name: r.get("name").to_json(),
                product: parse_json(r.text("product"), Value::Null),
                via: parse_json(r.text("via"), json!([])).as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default(),
                capabilities: parse_json(r.text("capabilities"), json!([])).as_array().cloned().unwrap_or_default(),
                heartbeat_ms: r.num("heartbeat_ms"),
                identity: r.get("identity").to_json(),
                enabled: r.num("enabled") != Some(0.0),
                created_at: r.num("created_at").unwrap_or(f64::NAN),
                allowed_at: r.num("allowed_at"),
                last_seen: r.num("last_seen"),
                presence: r.text("presence").unwrap_or("unknown").to_string(),
                presence_since: r.num("presence_at"),
                status: r.text("status").map(str::to_string),
                status_at: r.num("status_at"),
                needs_you: None,
                seq: r.num("seq").unwrap_or(0.0),
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

    fn row(a: &Actor) -> Arg {
        let num = |n: Option<f64>| n.map_or(Arg::Null, Arg::Num);
        let json_text = |v: &Value| Arg::Text(xenonet_base::json::stringify(v, 0));
        Arg::Obj(vec![
            ("slug".into(), Arg::text(&a.slug)),
            ("kind".into(), Arg::text(&a.kind)),
            ("name".into(), value_arg(&a.name)),
            ("product".into(), if a.product.is_null() { Arg::Null } else { json_text(&a.product) }),
            ("via".into(), json_text(&json!(a.via))),
            ("capabilities".into(), json_text(&Value::Array(a.capabilities.clone()))),
            ("heartbeat_ms".into(), num(a.heartbeat_ms)),
            ("identity".into(), value_arg(&a.identity)),
            ("enabled".into(), Arg::Num(if a.enabled { 1.0 } else { 0.0 })),
            ("created_at".into(), Arg::Num(a.created_at)),
            ("allowed_at".into(), num(a.allowed_at)),
            ("last_seen".into(), num(a.last_seen)),
            ("presence".into(), Arg::text(&a.presence)),
            ("presence_at".into(), num(a.presence_since)),
            ("status".into(), a.status.as_deref().map_or(Arg::Null, Arg::text)),
            ("status_at".into(), num(a.status_at)),
            ("seq".into(), Arg::Num(a.seq)),
        ])
    }

    fn save(&mut self, i: usize) -> Result<()> {
        let row = Self::row(&self.all[i]);
        self.store.with(|s| s.save_actor(&row))?;
        self.all[i].dirty = false;
        Ok(())
    }

    // Rows that only heard a heartbeat are written on the clock.
    fn flush(&mut self) -> Result<()> {
        let dirty: Vec<usize> = (0..self.all.len()).filter(|&i| self.all[i].dirty).collect();
        if dirty.is_empty() {
            return Ok(());
        }
        let store = self.store.clone();
        store.transaction(|| -> Result<()> {
            for i in dirty {
                self.save(i)?;
            }
            Ok(())
        })
    }

    // ---------- the actors themselves ----------

    /// An actor XenoNet knows of, new or again (`def`: id, kind, name,
    /// product, via, heartbeatMs, capabilities, identity, allowedAt,
    /// enabled, at). Only you are human.
    pub fn upsert(&mut self, def: &Arg, host: &mut dyn Host) -> Result<()> {
        let id_arg = def.field("id");
        let parsed = id_arg.as_str().and_then(|id| parse_actor_id(id).map(|(h, s)| (id.to_string(), h.is_some(), s.to_string())));
        let Some((id, elsewhere, slug)) = parsed else {
            return err(format!("not an actor id of this machine: {}", xenonet_engine::store::js_text(&id_arg)));
        };
        if elsewhere {
            return err(format!("not an actor id of this machine: {id}"));
        }
        let kind = match def.field("kind") {
            Arg::Undef => "agent".to_string(),
            k => xenonet_engine::store::js_text(&k),
        };
        if (kind == "human") != (id == YOU) {
            return err(format!("only {YOU} is human"));
        }
        let at = match def.field("at") {
            Arg::Undef => host.now(),
            a => a.as_f64().unwrap_or(f64::NAN),
        };
        if self.get(&id).is_none() {
            self.all.push(Actor {
                id: id.clone(),
                slug: slug.clone(),
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
        a.name = def.field("name").to_json();
        a.product = match def.field("product") {
            Arg::Undef => Value::Null,
            p => p.to_json(),
        };
        a.via = match def.field("via") {
            Arg::List(l) => l.iter().map(xenonet_engine::store::js_text).collect(),
            _ => Vec::new(),
        };
        a.heartbeat_ms = def.field("heartbeatMs").as_f64();
        a.capabilities = match def.field("capabilities") {
            Arg::List(l) => l.iter().map(Arg::to_json).collect(),
            _ => Vec::new(),
        };
        a.identity = match def.field("identity") {
            Arg::Undef => Value::Null,
            v => v.to_json(),
        };
        match def.field("allowedAt") {
            Arg::Undef => {}
            v => a.allowed_at = v.as_f64(),
        }
        match def.field("enabled") {
            Arg::Undef => {}
            v => a.enabled = v.truthy(),
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
            let mut transports: Vec<String> =
                remote.get("transports").and_then(Value::as_array).map(|t| t.iter().map(|x| xenonet_engine::js::template(Some(x))).collect()).unwrap_or_default();
            transports.sort_by(|x, y| xenonet_engine::js::utf16_cmp(x, y));
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
        let same = xenonet_base::json::stringify(&next.clone().unwrap_or(Value::Null), 0) == xenonet_base::json::stringify(&a.remote.clone().unwrap_or(Value::Null), 0);
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
        self.store.with(|s| s.remove_actor(Arg::text(slug)))?;
        self.all.retain(|a| a.id != id);
        self.waiting.retain(|w| w.actor != id);
        self.mark_changed();
        Ok(())
    }

    /// Your record deleted at once, when you turn off "Show me on the map".
    pub fn forget_you(&mut self) -> Result<()> {
        let i = self.need_index(YOU)?;
        let slug = self.all[i].slug.clone();
        let store = self.store.clone();
        store.transaction(|| -> Result<()> {
            self.store.with(|s| s.forget_actor(Arg::text(&slug)))?;
            let a = &mut self.all[i];
            a.instances.clear();
            a.targets.clear();
            a.presence = "unknown".into();
            a.presence_since = None;
            a.last_seen = None;
            a.status = None;
            a.status_at = None;
            a.needs_you = None;
            a.acted_at = None;
            a.cwd = Value::Null;
            a.recent.clear();
            self.save(i)
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
    pub fn watch_pid(&mut self, id: &str, pid: &Arg) -> Result<()> {
        let i = self.need_index(id)?;
        self.all[i].pid = pid.as_f64().filter(|p| p.is_finite() && p.trunc() == *p && *p > 1.0);
        Ok(())
    }

    // ---------- presence ----------

    /// One of an actor's instances says it's there (`o`: instance,
    /// presence, status, needsYou, heartbeatMs, leaseMs, pid, cwd, via, at).
    pub fn report(&mut self, id: &str, o: &Arg, host: &mut dyn Host) -> Result<()> {
        let i = self.need_index(id)?;
        let instance = match o.field("instance") {
            Arg::Undef => "main".to_string(),
            v => xenonet_engine::store::js_text(&v),
        };
        let presence = o.field("presence");
        let at = match o.field("at") {
            Arg::Undef => host.now(),
            v => v.as_f64().unwrap_or(f64::NAN),
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
        let heartbeat = o.field("heartbeatMs");
        let pid = o.field("pid");
        if a.instance(&instance).is_none() {
            let inst = Instance {
                presence: if presence.is_nullish() { "idle".into() } else { xenonet_engine::store::js_text(&presence) },
                at,
                heartbeat_ms: if heartbeat == Arg::Undef { a.heartbeat_ms } else { heartbeat.as_f64() },
                pid: pid.as_f64(),
                restored: false,
                lease_ms: None,
            };
            a.instances.push((instance.clone(), inst));
            // At most INSTANCES_MAX: the one silent longest goes.
            if a.instances.len() > INSTANCES_MAX {
                let mut others: Vec<(usize, f64)> = a.instances.iter().enumerate().filter(|(_, (n, _))| *n != instance).map(|(j, (_, x))| (j, x.at)).collect();
                xenonet_base::sort::sort_by(&mut others, |x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((j, _)) = others.first() {
                    a.instances.remove(*j);
                }
            }
        }
        let inst = &mut a.instances.iter_mut().find(|(n, _)| *n == instance).expect("added above when it wasn't there").1;
        if presence != Arg::Undef {
            inst.presence = xenonet_engine::store::js_text(&presence);
        }
        if heartbeat != Arg::Undef {
            inst.heartbeat_ms = heartbeat.as_f64();
        }
        match o.field("leaseMs") {
            Arg::Undef => {}
            v => inst.lease_ms = v.as_f64(),
        }
        if pid != Arg::Undef {
            inst.pid = pid.as_f64();
        }
        inst.at = js_max(inst.at, at);
        match o.field("cwd") {
            Arg::Undef => {}
            v => a.cwd = v.to_json(),
        }
        a.last_seen = Some(js_max(a.last_seen.unwrap_or(0.0), at));
        a.dirty = true;
        match o.field("needsYou") {
            Arg::Undef => {}
            n if n.truthy() => {
                let text = status_line(Some(&n.field("text").to_json())).ok().flatten().filter(|s| !s.is_empty()).unwrap_or_else(|| "Waiting for you".into());
                let since = match n.field("since") {
                    v if v.is_nullish() => number(at),
                    v => v.to_json(),
                };
                a.needs_you = Some(json!({ "kind": xenonet_engine::store::js_text(&n.field("kind")), "text": text, "since": since }));
            }
            _ => a.needs_you = None,
        }
        let via = o.field("via");
        let status = o.field("status");
        let store = self.store.clone();
        store.transaction(|| -> Result<()> {
            self.settle(i, at, &via, host)?;
            if status != Arg::Undef {
                self.set_status_line(i, &status, &via, at, &Value::Null)?;
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
    fn settle(&mut self, i: usize, at: f64, via: &Arg, _host: &mut dyn Host) -> Result<()> {
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
        self.event(i, "presence", &Value::Null, Some(detail.to_string()), via, t, None, Arg::Null)?;
        Ok(())
    }

    // A new status is an event; the same one again isn't, and one that
    // looks like it holds a key never gets in.
    fn set_status_line(&mut self, i: usize, text: &Arg, via: &Arg, at: f64, target: &Value) -> Result<()> {
        let Ok(line) = status_line(Some(&text.to_json_keep_undef())) else { return Ok(()) };
        if line == self.all[i].status {
            return Ok(());
        }
        let a = &mut self.all[i];
        a.status.clone_from(&line);
        a.status_at = Some(at);
        // A status cleared isn't news.
        if let Some(l) = line {
            self.event(i, "status", target, Some(l), via, at, None, Arg::Null)?;
        }
        Ok(())
    }

    // ---------- events ----------

    /// Something an actor did, kept and sent on (`o`: kind, target,
    /// relation, status, via, at, ref, and the facts).
    pub fn record(&mut self, id: &str, o: &Arg, host: &mut dyn Host) -> Result<Value> {
        let kind_arg = o.field("kind");
        let kind = xenonet_engine::store::js_text(&kind_arg);
        if kind_arg.as_str() == Some("presence") {
            return err("presence comes from reports");
        }
        let i = self.need_index(id)?;
        let target = def_null(o.field("target")).to_json();
        let at = match o.field("at") {
            Arg::Undef => host.now(),
            v => v.as_f64().unwrap_or(f64::NAN),
        };
        let r = def_null(o.field("ref"));
        let via = o.field("via");
        let mut facts = Map::new();
        for k in FACTS {
            let v = o.field(k);
            if !v.is_nullish() {
                facts.insert(k.into(), v.to_json());
            }
        }
        let facts = Value::Object(facts);
        let relation = o.field("relation");
        let status = o.field("status");
        let store = self.store.clone();
        let event = store.transaction(|| -> Result<Value> {
            let event = if kind == "status" {
                let line = status_line(Some(&status.to_json_keep_undef())).ok().flatten().filter(|l| !l.is_empty());
                let Some(line) = line else { return err("a status event needs a line of its own") };
                self.all[i].status = Some(line.clone());
                self.all[i].status_at = Some(at);
                self.event(i, &kind, &target, Some(line), &via, at, Some(&facts), r.clone())?
            } else {
                let at_where = self.where_of(&target, host);
                let detail = write_detail(&kind, Some(&facts), Some(&at_where)).map_err(Fail::Error)?;
                self.event(i, &kind, &target, detail, &via, at, Some(&facts), r.clone())?
            };
            let rel = match &relation {
                Arg::Undef => relation_for(&kind, &target).map(str::to_string),
                Arg::Null => None,
                v => Some(xenonet_engine::store::js_text(v)),
            };
            if let (Some(t), Some(rel)) = (target.as_str().filter(|t| !t.is_empty()), rel.filter(|r| !r.is_empty())) {
                if node_kind(t) != Some("actor") {
                    self.touch(i, t, &rel, at)?;
                }
            }
            // A registered actor that reports an action is working for a
            // while after it; an observed one is whatever its observer sees.
            if ACTIONS.contains(&kind.as_str()) && event.get("via").and_then(Value::as_str) == Some("registered") {
                let now = host.now();
                let a = &mut self.all[i];
                a.acted_at = Some(js_max(a.acted_at.unwrap_or(0.0), js_min(at, now)));
            }
            self.settle(i, at, &via, host)?;
            Ok(event)
        })?;
        self.mark_changed();
        Ok(event)
    }

    #[expect(clippy::too_many_arguments, reason = "the JS's event and what it records, one argument each")]
    fn event(&mut self, i: usize, kind: &str, target: &Value, detail: Option<String>, via: &Arg, at: f64, facts: Option<&Value>, r: Arg) -> Result<Value> {
        let a = &mut self.all[i];
        a.seq += 1.0;
        let via = if via.is_nullish() { a.via.iter().find(|v| *v != "demo").cloned().unwrap_or_else(|| "observed".into()) } else { xenonet_engine::store::js_text(via) };
        let detail = detail.map(|d| clip(&d, 200));
        let event = json!({
            "schema": "actor/event/v1",
            "id": format!("{}:{}", a.slug, xenonet_base::json::number_string(a.seq)),
            "actor": a.id,
            "at": number(at),
            "kind": kind,
            "target": target,
            "detail": detail,
            "via": via,
        });
        a.last_seen = Some(js_max(a.last_seen.unwrap_or(0.0), at));
        let facts = facts.filter(|f| f.as_object().is_some_and(|o| !o.is_empty())).map_or(Arg::Null, |f| Arg::Text(xenonet_base::json::stringify(f, 0)));
        let row = Arg::Obj(vec![
            ("schema".into(), Arg::text("actor/event/v1")),
            ("id".into(), value_arg(&event["id"])),
            ("actor".into(), Arg::text(&a.id)),
            ("at".into(), Arg::Num(at)),
            ("kind".into(), Arg::text(kind)),
            ("target".into(), value_arg(target)),
            ("detail".into(), value_arg(&event["detail"])),
            ("via".into(), value_arg(&event["via"])),
            ("facts".into(), facts),
            ("ref".into(), r),
        ]);
        self.store.with(|s| s.add_actor_event(&row))?;
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
        let d = host.describe(&xenonet_engine::js::template(Some(target)));
        if !truthy(Some(&d)) {
            return json!({ "kind": k });
        }
        if d.get("kind").and_then(Value::as_str) == Some("session") {
            let place = d.get("place").filter(|p| !p.is_null()).map_or(Value::Null, |p| Value::String(place_name(&xenonet_engine::js::template(Some(p)), Some("project"))));
            return json!({ "kind": "session", "app": d.get("app").cloned().filter(|a| !a.is_null()).unwrap_or(Value::Null), "place": place });
        }
        let kind = d.get("kind").cloned().unwrap_or(Value::Null);
        let name = d.get("name").filter(|p| !p.is_null()).map_or(Value::Null, |n| Value::String(place_name(&xenonet_engine::js::template(Some(n)), kind.as_str())));
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
            self.store.with(|s| s.drop_actor_target(Arg::text(id), Arg::text(node), Arg::text(relation)))?;
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
        let row = Arg::Obj(vec![
            ("actor".into(), Arg::text(&actor)),
            ("target".into(), Arg::text(node)),
            ("relation".into(), Arg::text(relation)),
            ("since".into(), Arg::Num(t.since)),
            ("last_at".into(), Arg::Num(t.last_at)),
        ]);
        self.store.with(|s| s.save_actor_target(&row))?;
        let a = &mut self.all[i];
        if a.targets.len() > TARGETS_KEPT {
            let mut order: Vec<(usize, f64)> = a.targets.iter().enumerate().map(|(j, (_, t))| (j, t.last_at)).collect();
            xenonet_base::sort::sort_by(&mut order, |x, y| x.1.partial_cmp(&y.1).unwrap_or(std::cmp::Ordering::Equal));
            let gone = a.targets.remove(order[0].0).1;
            self.store.with(|s| s.drop_actor_target(Arg::text(&actor), Arg::text(&gone.id), Arg::text(&gone.relation)))?;
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
        for r in self.store.with(|s| s.actor_event_counts(Arg::Num(now - DAY_MS)))? {
            let actor = r.text("actor").unwrap_or("").to_string();
            let kind = r.text("kind").unwrap_or("");
            let n = r.num("n").unwrap_or(0.0);
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
        xenonet_base::sort::sort_by(&mut targets, |x, y| y.last_at.partial_cmp(&x.last_at).unwrap_or(std::cmp::Ordering::Equal));
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
        for r in self.store.with(|s| s.actor_spawns(Arg::Num(now - WEEK_MS)))? {
            let actor = r.text("actor").unwrap_or("");
            if has(actor) && r.text("target").and_then(node_kind) == Some("session") {
                starts.push(json!({ "actor": actor, "session": r.get("target").to_json(), "at": r.get("at").to_json(), "via": r.get("via").to_json() }));
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
        if !is_absolute(p) {
            return Value::Null;
        }
        let d = host.place_of(&resolve(p));
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
        let d = if truthy(Some(&place_id)) { host.describe(&xenonet_engine::js::template(Some(&place_id))) } else { Value::Null };
        let mut edits: Vec<(String, f64, bool)> = Vec::new();
        for row in self.store.with(|s| s.actor_facts_since(Arg::text(id), Arg::text("edited"), Arg::Num(since)))? {
            let facts = parse_json(row.text("facts"), json!({}));
            for p in facts.get("paths").and_then(Value::as_array).cloned().unwrap_or_default() {
                if let Some(p) = p.as_str() {
                    if !edits.iter().any(|(x, _, _)| x == p) {
                        edits.push((p.into(), row.num("at").unwrap_or(f64::NAN), row.text("via") != Some("registered")));
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

    /// An actor's own status line, set through the engine's focus: Ok(status)
    /// or Err(reason).
    pub fn set_status(&mut self, id: &str, text: &Arg, via: &Arg, at: f64) -> Result<std::result::Result<Value, &'static str>> {
        let text = if text.is_nullish() { Arg::text("") } else { text.clone() };
        if let Err(reason) = status_line(Some(&text.to_json_keep_undef())) {
            return Ok(Err(reason));
        }
        let i = self.need_index(id)?;
        let via = if *via == Arg::Undef { Arg::text("registered") } else { via.clone() };
        let store = self.store.clone();
        store.transaction(|| self.set_status_line(i, &text, &via, at, &Value::Null))?;
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
        let store = self.store.clone();
        let event = store.transaction(|| -> Result<Value> {
            let o = Arg::Obj(vec![
                ("kind".into(), Arg::text("said")),
                ("target".into(), value_arg(&target)),
                ("count".into(), Arg::Num(to.len() as f64)),
                ("via".into(), Arg::text("registered")),
                ("at".into(), Arg::Num(at)),
            ]);
            let event = self.record(from, &o, host)?;
            let i = self.need_index(from)?;
            for s in &sessions {
                self.touch(i, s, "messaged", at)?;
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
        let rows = self.store.with(|s| s.actor_token_times())?;
        let row = |slug: &str| rows.iter().rev().find(|r| r.get("slug").and_then(Value::as_str) == Some(slug));
        let mut list: Vec<&Actor> = self.all.iter().filter(|a| a.id != YOU && self.is_registered(&a.id)).collect();
        xenonet_base::sort::sort_by(&mut list, |x, y| x.created_at.partial_cmp(&y.created_at).unwrap_or(std::cmp::Ordering::Equal));
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
            xenonet_base::sort::sort_by(&mut by_seen, |x, y| {
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
        xenonet_base::sort::sort_by(&mut shown, |x, y| {
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
        let said = xenonet_base::json::stringify(&list["actors"], 0);
        if self.last_list.as_deref() == Some(said.as_str()) {
            return Ok(Value::Null);
        }
        self.last_list = Some(said);
        Ok(list)
    }

    /// Events since `since` (the last 7 days at most), oldest first.
    pub fn events(&self, o: &Arg, now: f64) -> Result<Value> {
        let since = o.field("since").as_f64().filter(|f| f.is_finite()).unwrap_or(now - DAY_MS);
        let from = js_max(since, now - WEEK_MS);
        let limit = match o.field("limit") {
            Arg::Undef => 1000.0,
            v => v.as_f64().unwrap_or(f64::NAN),
        };
        let floor = limit.floor();
        let n = js_min(js_max(1.0, if floor == 0.0 || floor.is_nan() { 1.0 } else { floor }), EVENTS_MAX);
        let view = match o.field("view") {
            Arg::Undef => "public".to_string(),
            v => xenonet_engine::store::js_text(&v),
        };
        let only = def_null(o.field("only"));
        if only.truthy() && view != "window" && !self.is_public(&xenonet_engine::store::js_text(&only)) {
            return Ok(json!({ "schema": "actor/events/v1", "at": number(now), "events": [] }));
        }
        let except = if only.truthy() || view == "window" || self.demo { Arg::Null } else { Arg::text(YOU) };
        let rows =
            self.store.with(|s| s.actor_events_since(Arg::Num(from), &Arg::Obj(vec![("limit".into(), Arg::Num(n)), ("only".into(), only.clone()), ("except".into(), except)])))?;
        Ok(json!({ "schema": "actor/events/v1", "at": number(now), "events": rows.iter().map(event_of).collect::<Vec<_>>() }))
    }

    // ---------- the clock ----------

    /// Every CLOCK_MS: processes that ended, presence from silence,
    /// sessions that never reached the map let go, heartbeats written down.
    pub fn tick(&mut self, now: f64, host: &mut dyn Host) -> Result<()> {
        let store = self.store.clone();
        store.transaction(|| -> Result<()> {
            for i in 0..self.all.len() {
                self.ended(i, now, host);
                self.settle(i, now, &Arg::Undef, host)?;
                if !Self::map_ready(host) {
                    continue;
                }
                let gone: Vec<(String, Target)> =
                    self.all[i].targets.iter().filter(|(_, t)| node_kind(&t.id) == Some("session") && now - t.since >= SESSION_WAIT_MS).cloned().collect();
                for (key, t) in gone {
                    if host.has(&t.id) == Some(true) {
                        continue;
                    }
                    self.all[i].drop_target(&key);
                    let actor = self.all[i].id.clone();
                    self.store.with(|s| s.drop_actor_target(Arg::text(actor), Arg::text(&t.id), Arg::text(&t.relation)))?;
                }
            }
            self.flush()
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
        self.store.with(|s| s.prune(now))?;
        let targets: Vec<String> = self
            .store
            .with(|s| s.actor_targets())?
            .iter()
            .map(|t| format!("{}|{}|{}", t.text("actor").unwrap_or(""), t.text("relation").unwrap_or(""), t.text("target").unwrap_or("")))
            .collect();
        for i in 0..self.all.len() {
            let id = self.all[i].id.clone();
            let recent: Vec<Value> = self.store.with(|s| s.last_actor_events(Arg::text(&id), Arg::Num(RECENT)))?.iter().map(event_of).collect();
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
                self.report(&id, &Arg::Obj(vec![("instance".into(), Arg::text("main")), ("presence".into(), Arg::text("working"))]), host)?;
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
        let a = if a.is_null() {
            self.all.iter().find(|x| x.kind == "agent").map(|x| x.id.clone()).or_else(|| self.demo.then(|| YOU.to_string()))
        } else {
            Some(xenonet_engine::js::template(Some(&a)))
        };
        let Some(a) = a else { return Ok(Value::Null) };
        if !self.has(&a) || !self.is_public(&a) {
            return Ok(Value::Null);
        }
        let session = if b.is_null() { host.recent_session() } else { b.clone() };
        let rec = |kind: &str, target: &Value, count: bool| {
            let mut o = vec![("kind".to_string(), Arg::text(kind)), ("target".to_string(), value_arg(target))];
            if count {
                o.push(("count".into(), Arg::Num(1.0)));
            }
            Arg::Obj(o)
        };
        match kind {
            "actor-spawn" => {
                self.record(&a, &rec("spawned", &session, false), host)?;
                Ok(json!({ "ok": true, "kind": kind, "a": a, "b": session }))
            }
            "actor-edit" => {
                let mut b = b;
                if b.is_null() {
                    let d = if truthy(Some(&session)) { host.describe(&xenonet_engine::js::template(Some(&session))) } else { Value::Null };
                    b = if d.get("kind").and_then(Value::as_str) == Some("session") {
                        d.get("placeId").filter(|p| !p.is_null()).cloned().unwrap_or(session.clone())
                    } else {
                        session.clone()
                    };
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
                self.report(&a, &Arg::Obj(vec![("instance".into(), Arg::text("main")), ("presence".into(), Arg::text("working"))]), host)?;
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
                self.report(&a, &Arg::Obj(vec![("instance".into(), Arg::text(first))]), host)?;
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
            let o = Arg::Obj(vec![
                ("instance".into(), Arg::text("window")),
                ("presence".into(), Arg::text(p)),
                ("heartbeatMs".into(), Arg::Num(crate::you::YOU_HEARTBEAT_MS)),
                ("via".into(), Arg::text("observed")),
                ("at".into(), Arg::Num(at)),
            ]);
            self.report(YOU, &o, host)?;
        }
        Ok(())
    }

    fn you_place_of(session: &Value, host: &mut dyn Host) -> Value {
        let d = host.describe(&xenonet_engine::js::template(Some(session)));
        d.get("placeId").filter(|p| !p.is_null()).cloned().unwrap_or(Value::Null)
    }

    // One thing you did, recorded; or why it isn't.
    fn you_act(&mut self, d: &Arg, now: f64, host: &mut dyn Host) -> Result<Option<String>> {
        let what = d.field("what");
        let what = match &what {
            Arg::Text(w) if d.truthy() && crate::you::known_action(w) => w.clone(),
            _ => return Ok(Some("XenoNet doesn't know that action".into())),
        };
        let mut at = d.field("at").as_f64().filter(|f| f.is_finite()).unwrap_or(now);
        if now - at > crate::you::DAY_MS {
            return Ok(Some("that was more than a day ago".into()));
        }
        if at - now > crate::you::AHEAD_MS {
            at = now;
        }
        let node = d.field("node").to_json();
        if host.has_map() && host.has(&xenonet_engine::js::template(Some(&node))) != Some(true) {
            return Ok(Some("XenoNet doesn't know that node".into()));
        }
        let at_where = self.where_of(&node, host);
        let kind = at_where.get("kind").and_then(Value::as_str).map(str::to_string);
        if at_where.is_null() || !kind.as_deref().is_some_and(|k| crate::you::on(&what, k)) {
            let k = at_where.get("kind").filter(|k| !k.is_null()).map_or_else(|| "node like that".into(), |k| xenonet_engine::js::template(Some(k)));
            return Ok(Some(format!("you can't do that to a {k}")));
        }
        let apps = |v: &Arg| matches!(v.as_str(), Some("claude" | "codex"));
        if what == "started" {
            let app = if apps(&d.field("app")) { d.field("app") } else { Arg::text("claude") };
            let o = Arg::Obj(vec![
                ("kind".into(), Arg::text("spawned")),
                ("target".into(), value_arg(&node)),
                ("relation".into(), Arg::text("works-on")),
                ("app".into(), app),
                ("via".into(), Arg::text("observed")),
                ("at".into(), Arg::Num(at)),
            ]);
            self.record(YOU, &o, host)?;
            return Ok(None);
        }
        if what == "handed-off" {
            if !apps(&d.field("to")) {
                return Ok(Some("a handoff says which app it went to".into()));
            }
            let place = Self::you_place_of(&node, host);
            if !truthy(Some(&place)) {
                return Ok(Some("XenoNet doesn't know where that session lives".into()));
            }
            let app = at_where.get("app").cloned().unwrap_or(Value::Null);
            let o = Arg::Obj(vec![
                ("kind".into(), Arg::text("spawned")),
                ("target".into(), value_arg(&place)),
                ("relation".into(), Arg::text("works-on")),
                ("app".into(), value_arg(&app)),
                ("to".into(), d.field("to")),
                ("via".into(), Arg::text("observed")),
                ("at".into(), Arg::Num(at)),
            ]);
            self.record(YOU, &o, host)?;
            return Ok(None);
        }
        let target = if what == "terminal" && kind.as_deref() == Some("session") { Self::you_place_of(&node, host) } else { node };
        let line = crate::detail::console_status(Some(&what), Some(&at_where));
        let (true, Some(line)) = (truthy(Some(&target)), line) else {
            return Ok(Some("XenoNet can't say what that is".into()));
        };
        let o = Arg::Obj(vec![
            ("kind".into(), Arg::text("status")),
            ("status".into(), Arg::text(line)),
            ("target".into(), value_arg(&target)),
            ("relation".into(), crate::you::relation(&what).map_or(Arg::Undef, Arg::text)),
            ("via".into(), Arg::text("observed")),
            ("at".into(), Arg::Num(at)),
        ]);
        self.record(YOU, &o, host)?;
        Ok(None)
    }

    /// The page's report, an actor/console/v1: its presence (a report
    /// without one is a heartbeat that keeps the last), and what you did.
    /// Says how many actions were taken, and why the others weren't.
    pub fn you_report(&mut self, body: &Arg, at: f64, host: &mut dyn Host) -> Result<Value> {
        self.you.off = false;
        let presence = body.field("presence");
        if presence.truthy() {
            self.you.page = Some(xenonet_engine::store::js_text(&presence));
        }
        let did = match body.field("did") {
            Arg::Undef | Arg::Null => Vec::new(),
            Arg::List(l) => l,
            _ => return err("body.did.forEach is not a function"),
        };
        let store = self.store.clone();
        let (accepted, dropped) = store.transaction(|| -> Result<(usize, Vec<Value>)> {
            self.you_settle(at, host)?;
            let mut accepted = 0;
            let mut dropped = Vec::new();
            for (index, d) in did.iter().enumerate() {
                match self.you_act(d, at, host)? {
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

fn def_null(a: Arg) -> Arg {
    if a == Arg::Undef {
        Arg::Null
    } else {
        a
    }
}

/// A JSON value to bind as SQL would bind the JS value.
pub fn value_arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::Text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(value_arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), value_arg(v))).collect()),
    }
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

/// path.isAbsolute on this machine.
pub fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        let b = p.as_bytes();
        p.starts_with('\\') || p.starts_with('/') || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
    } else {
        p.starts_with('/')
    }
}

fn resolve(p: &str) -> String {
    match xenonet_base::paths::path_style(p) {
        xenonet_base::paths::PathStyle::Posix => {
            let n = xenonet_base::paths::normalize_posix(p);
            if n.len() > 1 {
                n.trim_end_matches('/').to_string()
            } else {
                n
            }
        }
        _ => xenonet_base::paths::resolve(p),
    }
}

trait KeepUndef {
    fn to_json_keep_undef(&self) -> Value;
}

impl KeepUndef for Arg {
    // A status given as undefined reads as no status, as null does; a
    // value that isn't text stays one.
    fn to_json_keep_undef(&self) -> Value {
        match self {
            Arg::Undef => Value::Null,
            other => other.to_json(),
        }
    }
}
