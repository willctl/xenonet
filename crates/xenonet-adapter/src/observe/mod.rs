//! Observing an actor product (docs/design/actors/interface.md, sections 1
//! and 7). An adapter whose product XenoNet observes brings an `Observer`:
//! one look every 5 seconds, with the process table read once for every
//! observer, that keeps its product's actors in the registry (`Core`:
//! keep, say, gather, touch, happened), matches the sessions they start
//! to processes (`attribution`), and reads a product's Mac app
//! (`AppBundle`). What an observer needs of the machine and the registry
//! comes through `ObserverHost`, which the engine implements.

pub mod app_bundle;
pub mod attribution;
pub mod core;

pub use app_bundle::AppBundle;
pub use attribution::{Census, Spawns, SpawnsState};
pub use core::{Core, Gather, Keep, Say, CAPABILITIES, KEEPALIVE_MS, LOOK_MS};

use crate::apps::App;
use crate::products::Product;
use serde_json::Value;
use xenonet_base::text::js_string;

/// What a call on the host threw: its message and, for a file or a
/// database, the error's code (ENOENT, EACCES, SQLITE_BUSY).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail {
    pub message: String,
    pub code: Option<String>,
}

impl Fail {
    pub fn new(message: impl Into<String>) -> Self {
        Fail { message: message.into(), code: None }
    }
    pub fn coded(message: impl Into<String>, code: impl Into<String>) -> Self {
        Fail { message: message.into(), code: Some(code.into()) }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

/// A file's size and modification time, for telling whether it changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    pub mtime_ms: f64,
    pub size: f64,
}

/// The files an observer reads: whether one is there, its bytes or text,
/// a folder's names, and a file's stamp. Errors carry the system's code.
pub trait Files {
    fn exists(&self, path: &str) -> bool;
    fn read(&self, path: &str) -> Result<Vec<u8>>;
    fn read_text(&self, path: &str) -> Result<String>;
    fn readdir(&self, path: &str) -> Result<Vec<String>>;
    /// The folders in a folder, by name.
    fn dirs(&self, path: &str) -> Result<Vec<String>>;
    fn stat(&self, path: &str) -> Result<Stat>;
}

/// The process table, as the observers read it.
pub trait Procs {
    /// Whether the table can be read on this system at all.
    fn supported(&self) -> bool;
    /// The table, with what each process is.
    fn census(&self) -> Result<Census>;
    /// Whether a process runs now, asked of the system.
    fn alive(&self, pid: f64) -> bool;
    /// The folders these processes work in, for those the system says.
    fn cwds(&self, pids: &[f64]) -> Vec<(f64, String)>;
    /// The children of these processes.
    fn children(&self, pids: &[f64]) -> Vec<f64>;
    /// These processes' rows (pid, ppid, start, program).
    fn describe(&self, pids: &[f64]) -> Vec<Value>;
    /// The agent app a program is, from its path alone.
    fn agent_of(&self, program: &str) -> Option<String>;
    /// Whether a program's arguments must be read to say what it is.
    fn wants_args(&self, program: &str) -> bool;
}

/// The actors' registry, as the observers keep their actors in it. The
/// definitions, reports and records are the JSON the registry takes.
pub trait Actors {
    fn has(&self, id: &str) -> bool;
    /// The actor's record (name, product, via, heartbeatMs, capabilities,
    /// identity), or null.
    fn info(&self, id: &str) -> Value;
    fn presence(&self, id: &str) -> Option<String>;
    /// Whether its product is installed, when known.
    fn installed(&self, id: &str) -> Option<bool>;
    fn enabled(&self, id: &str) -> bool;
    fn is_registered(&self, id: &str) -> bool;
    /// The actors of a product, by id.
    fn of_product(&self, product: &str) -> Vec<String>;
    /// The processes a registered actor reports from.
    fn pids_of(&self, id: &str) -> Vec<f64>;
    fn targets_of(&self, id: &str) -> Vec<Value>;
    fn upsert(&self, def: &Value) -> Result<()>;
    fn set_installed(&self, id: &str, installed: bool) -> Result<()>;
    fn report(&self, id: &str, report: &Value) -> Result<()>;
    /// The event recorded, or null.
    fn record(&self, id: &str, event: &Value) -> Result<Value>;
    fn target(&self, id: &str, node: &str, relation: &str, at: f64) -> Result<()>;
    /// What a target is on the map ({ kind, name, … }), or null.
    fn where_of(&self, target: &Value) -> Value;
    /// The deepest project or folder holding a path: its id, or null.
    fn place_of(&self, path: &str) -> Value;
    /// Whether an event with this ref was recorded for the actor since.
    fn has_ref(&self, id: &str, r: &str, since: f64) -> Result<bool>;
    /// The spawned reports still waiting for their session, the lapsed
    /// ones let go.
    fn expire_waiting(&self, at: f64) -> Vec<Value>;
    fn waiting(&self) -> Vec<Value>;
    fn drop_waiting_at(&self, index: usize);
}

/// The engine's pins and the map's sessions, for matching a process to
/// the session it holds.
pub trait Pins {
    /// Whether the map has been made; None when there's no map at all.
    fn map_ready(&self) -> Option<bool>;
    /// The map's sessions ({ id, app, cwd, worktree, createdAt,
    /// originator }), none without a map.
    fn sessions(&self) -> Result<Vec<Value>>;
    /// The processes a session is pinned to ({ pid, start, sure, at }).
    fn pins(&self, session: &str) -> Result<Vec<Value>>;
    /// The sessions pinned to a process ({ agent, start, sure, at }).
    fn pinned_to(&self, pid: f64) -> Result<Vec<Value>>;
}

/// A product's SQLite database, opened read only for a moment.
pub trait Db {
    fn open(&self, file: &str) -> Result<()>;
    fn all(&self, sql: &str, args: &[Value]) -> Result<Vec<Value>>;
    /// The first row, or null.
    fn get(&self, sql: &str, args: &[Value]) -> Result<Value>;
    fn close(&self);
}

/// The vendor readers: a record checked against its source's schema, and
/// the drift recorded when a file can't be read at all or lacks a field.
pub trait Vendor {
    /// The record as the reader keeps it, or null when it's dropped.
    fn check(&self, source: &str, record: &Value, version: Option<&str>) -> Value;
    fn unparsable(&self, source: &str, version: Option<&str>);
    fn missing(&self, source: &str, path: &str, version: Option<&str>);
}

/// What an observer is given: the clock, the home folder, whether its
/// product's observer is on, its block's health, and its collaborators.
pub trait ObserverHost {
    fn now(&self) -> f64;
    fn home(&self) -> &str;
    /// Whether this product's observer is on (every actor of it enabled).
    fn observing(&self) -> bool;
    /// A look that went well, for the observer's block.
    fn ok(&self, at: f64);
    /// A look that failed, for the observer's block.
    fn problem(&self, message: &str, at: f64);
    fn actors(&self) -> &dyn Actors;
    fn procs(&self) -> &dyn Procs;
    fn files(&self) -> &dyn Files;
    fn spawns(&self) -> &dyn Pins;
    fn db(&self) -> &dyn Db;
    fn vendor(&self) -> &dyn Vendor;
    /// A key of a binary Info.plist, read by the system's tools.
    fn plist_key(&self, file: &str, key: &str) -> Option<String>;
}

/// A product's observer, as its adapter brings it.
pub trait Observer: Send + Sync {
    /// Its block's name: `hermes-observer`.
    fn name(&self) -> &'static str;
    fn product(&self) -> &'static Product;
    /// What it shares with every observer.
    fn core(&self) -> &Core;
    /// One look at the product, at `at`.
    fn look(&self, h: &dyn ObserverHost, at: f64) -> Result<()>;
    /// What the doctor shows: { product, name, installed, sources,
    /// recognized }.
    fn status(&self, h: &dyn ObserverHost) -> Result<Value>;
    /// How often it polls between looks, for one that does.
    fn poll_ms(&self) -> Option<f64> {
        None
    }
    fn may_poll(&self, _h: &dyn ObserverHost) -> bool {
        false
    }
    fn poll(&self, _h: &dyn ObserverHost, _at: f64) -> Result<()> {
        Ok(())
    }
}

/// Whether an observer may look now: it's on, and the map is there (before
/// XenoNet's first look at it, no place could be named).
pub fn may_look(h: &dyn ObserverHost) -> bool {
    h.observing() && h.spawns().map_ready() != Some(false)
}

/// One look: the observer's own, then the minutes over recorded, and the
/// block ok; a look that fails leaves it degraded.
pub fn tick(o: &dyn Observer, h: &dyn ObserverHost, at: f64) {
    match o.look(h, at).and_then(|()| o.core().flush(h, at)) {
        Ok(()) => h.ok(at),
        Err(e) => h.problem(&e.message, at),
    }
}

/// Whether an actor is known, and not the demo's.
pub fn known(h: &dyn ObserverHost, id: &str) -> bool {
    let a = h.actors();
    a.has(id) && !a.info(id).get("via").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| v.as_str() == Some("demo")))
}

/// A product's entry for the registry of adapters: its id and name, with
/// nothing of an app's (no folder, hooks or capabilities), since the
/// registry holds adapters by their `App`.
pub fn product_app(p: &Product) -> App {
    App { id: p.id.into(), name: p.name.into(), ..App::default() }
}

/// A value as `${v}` writes it.
pub(crate) fn text(v: &Value) -> String {
    js_string(v)
}

/// `${v}`, or "" for null.
pub(crate) fn text_or_empty(v: &Value) -> String {
    if v.is_null() {
        String::new()
    } else {
        js_string(v)
    }
}
