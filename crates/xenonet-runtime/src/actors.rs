//! Actors (crates/xenonet-actors/src/registry.rs, with you.rs and
//! demo.rs): the registry, its host (the clock, the map, whether a process
//! runs), its rows in the engine's store, the list's throttle, the clock
//! that ticks it, and the demo's made-up actors and their scripts. What it
//! tells goes to the page once the call is done, never while it's held.

use crate::map::Map;
use crate::runtime::Runtime;
use async_trait::async_trait;
use serde_json::{json, Map as JsonMap, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use xenonet_actors::demo::{Demo, Resolve, STEP_MS};
use xenonet_actors::registry::{self, Actors as Registry, Fail, Host, CLOCK_MS, LIST_THROTTLE_MS, PRUNE_MS};
use xenonet_actors::store::{self as rows, Tx};
use xenonet_api::cores::{Actors, CoreError, R};
use xenonet_api::From;
use xenonet_engine::engine::Thrown;
use xenonet_os::sync::Locked;
use xenonet_store::engine::{row_json, EngineStore, Shared};
use xenonet_store::sql::{self, Row, SqlError};
use xenonet_text::time::now_ms;

type Result<T> = std::result::Result<T, Thrown>;

/// The actors' rows in the engine's store, as the registry, registration
/// and the wakes keep them, as JSON.
pub struct Rows(pub Shared);

/// What a store call failed with, kept whole: its message and code.
pub fn store_error(e: SqlError) -> rows::Error {
    rows::Error { message: e.message().to_string(), code: e.code().map(str::to_string) }
}

/// Rows read, as JSON (every integer a number).
pub fn rows_json(r: sql::Result<Vec<Row>>) -> rows::Result<Vec<Value>> {
    r.map(|rows| rows.iter().map(row_json).collect()).map_err(store_error)
}

/// A row read, if any, as JSON.
pub fn row_of(r: sql::Result<Option<Row>>) -> rows::Result<Option<Value>> {
    r.map(|row| row.as_ref().map(row_json)).map_err(store_error)
}

impl Rows {
    /// A call on the store, its failure as the actors' cores take it.
    pub fn direct<T>(&self, f: impl FnOnce(&EngineStore) -> sql::Result<T>) -> rows::Result<T> {
        self.0.with(f).map_err(store_error)
    }
}

impl Tx for Rows {
    fn in_transaction(&self) -> bool {
        self.0.with(EngineStore::in_transaction)
    }
    fn begin(&self) -> rows::Result<()> {
        self.direct(EngineStore::begin)
    }
    fn commit(&self) -> rows::Result<()> {
        self.direct(EngineStore::commit)
    }
    fn rollback(&self) {
        self.0.with(EngineStore::rollback);
    }
}

impl registry::Store for Rows {
    fn actor_rows(&self) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(EngineStore::actor_rows))
    }
    fn actor_targets(&self) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(EngineStore::actor_targets))
    }
    fn last_actor_events(&self, actor: &str, limit: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.last_actor_events(actor, limit)))
    }
    fn save_actor(&self, row: &Value) -> rows::Result<()> {
        self.direct(|s| s.save_actor(row))
    }
    fn remove_actor(&self, slug: &str) -> rows::Result<()> {
        self.direct(|s| s.remove_actor(slug))
    }
    fn forget_actor(&self, slug: &str) -> rows::Result<()> {
        self.direct(|s| s.forget_actor(slug))
    }
    fn add_actor_event(&self, event: &Value) -> rows::Result<()> {
        self.direct(|s| s.add_actor_event(event))
    }
    fn save_actor_target(&self, target: &Value) -> rows::Result<()> {
        self.direct(|s| s.save_actor_target(target))
    }
    fn drop_actor_target(&self, actor: &str, target: &str, relation: &str) -> rows::Result<()> {
        self.direct(|s| s.drop_actor_target(actor, target, relation))
    }
    fn actor_event_counts(&self, since: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.actor_event_counts(since)))
    }
    fn actor_spawns(&self, since: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.actor_spawns(since)))
    }
    fn actor_facts_since(&self, actor: &str, kind: &str, since: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.actor_facts_since(actor, kind, since)))
    }
    fn actor_events_since(&self, since: f64, limit: f64, only: Option<&str>, except: Option<&str>) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.actor_events_since(since, limit, only, except)))
    }
    fn actor_token_times(&self) -> rows::Result<Vec<Value>> {
        self.direct(EngineStore::actor_token_times)
    }
    fn prune(&self, now: f64) -> rows::Result<()> {
        self.direct(|s| s.prune(now))
    }
}

/// The demo world's nodes, as resolveDemo finds them: its projects folder,
/// and its sessions by title; a project by the checkout the store keeps
/// for its folder, which the map names it by.
#[derive(Clone)]
pub struct DemoNames {
    projects_dir: String,
    sessions: Vec<(String, String)>,
    claude_account: fn(Option<&str>) -> Value,
    db: Option<Arc<xenonet_store::Db>>,
}

/// The demo's made-up world, as the actors' requests name its nodes: its
/// sessions by title and id, and the Claude account a project is in. The
/// composition root gives it, from the demo's world.
#[derive(Clone, Debug)]
pub struct MadeUp {
    pub sessions: Vec<(String, String)>,
    pub claude_account: fn(Option<&str>) -> Value,
}

impl MadeUp {
    /// A world with no sessions and no accounts, for a runtime with no
    /// demo to name.
    pub fn none() -> MadeUp {
        MadeUp { sessions: Vec::new(), claude_account: |_| Value::Null }
    }
}

impl DemoNames {
    pub fn new(projects_dir: &str, made_up: &MadeUp, db: Option<Arc<xenonet_store::Db>>) -> DemoNames {
        DemoNames { projects_dir: projects_dir.to_string(), sessions: made_up.sessions.clone(), claude_account: made_up.claude_account, db }
    }

    // The node a project of the demo's folder is: its checkout's.
    fn project(&self, dir: &str) -> String {
        let checkout = self.db.as_ref().and_then(|db| {
            let found = crate::checkouts::found_of(dir, crate::checkouts::Why::Repo)?;
            db.ensure(&found, now_ms() as i64).ok().map(|(c, _)| c.id.as_str().to_string())
        });
        format!("project:{}", checkout.unwrap_or_else(|| xenonet_os::paths::path_key(dir).unwrap_or_default()))
    }

    fn resolve(&self, kind: &str, what: &str) -> Value {
        let file = || what.split('/').fold(self.projects_dir.clone(), |p, part| xenonet_os::paths::join(&p, part));
        match kind {
            "project" => json!(self.project(&file())),
            "session" => self.sessions.iter().find(|(t, _)| t == what).map_or(Value::Null, |(_, id)| json!(id)),
            "file" => json!(file()),
            "identity" => {
                let a = (self.claude_account)(Some(what));
                json!(format!("claude:{}:{}", a["account"].as_str().unwrap_or(""), a["org"].as_str().unwrap_or("")))
            }
            _ => Value::Null,
        }
    }
}

/// What the registry asks of the world: the clock, the map, processes,
/// and the demo's nodes.
pub struct ActorHost<'a> {
    pub map: &'a Map,
    pub names: &'a DemoNames,
}

impl Host for ActorHost<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn describe(&mut self, id: &str) -> Value {
        self.map.describe(id).unwrap_or(Value::Null)
    }
    fn has_map(&mut self) -> bool {
        true
    }
    fn has(&mut self, id: &str) -> Option<bool> {
        Some(self.map.has(id).unwrap_or(false))
    }
    fn ready(&mut self) -> Option<bool> {
        (!self.map.ready()).then_some(false)
    }
    fn place_of(&mut self, p: &str) -> Value {
        self.map.place_of(p).unwrap_or(Value::Null)
    }
    fn recent_session(&mut self) -> Value {
        self.map.recent_session().unwrap_or(Value::Null)
    }
    fn alive(&mut self, pid: f64) -> bool {
        crate::runtime::alive(pid)
    }
}

impl Resolve for ActorHost<'_> {
    fn resolve(&mut self, kind: &str, what: &str) -> Value {
        self.names.resolve(kind, what)
    }
}

/// What the registry failed with, as the engine's calls pass it on.
pub fn thrown(f: Fail) -> Thrown {
    match f {
        Fail::Error(m) => Thrown::new(m),
        Fail::Store(e) => Thrown { message: e.message, code: e.code },
    }
}

pub fn core_error(t: Thrown) -> CoreError {
    CoreError { code: t.code, ..CoreError::new(t.message) }
}

/// What's run before an actor's record goes: its remote connections are
/// revoked first, while their grants can still be named.
pub type BeforeRemove = Arc<dyn Fn(&str) + Send + Sync>;

pub struct ActorSide {
    pub reg: Mutex<Registry>,
    pub store: Shared,
    pub names: DemoNames,
    demo: Mutex<Option<Demo>>,
    pub is_demo: bool,
    list_armed: AtomicBool,
    timers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    pub before_remove: Mutex<Vec<BeforeRemove>>,
    pub tokens: crate::tokens::TokenScan,
}

/// What a call told, and whether the list changed, taken while it's held.
type Told = (Vec<(&'static str, Value)>, bool);

impl ActorSide {
    /// The actors kept in `store`: you, and in the demo its made-up ones,
    /// with three hours of their scripts behind them.
    pub fn new(store: Shared, demo: bool, map: &Map, names: DemoNames) -> std::result::Result<ActorSide, String> {
        let mut host = ActorHost { map, names: &names };
        let mut reg = Registry::new(Arc::new(Rows(store.clone())), demo, &mut host).map_err(|f| thrown(f).message)?;
        let d = if demo { Some(Demo::add(&mut reg, 3.0 * 3600e3, &mut host).map_err(|f| thrown(f).message)?) } else { None };
        // Nothing listens yet: what the demo's history told is in the store.
        let _ = (reg.take_emits(), reg.take_changed());
        Ok(ActorSide {
            reg: Mutex::new(reg),
            store,
            names,
            demo: Mutex::new(d),
            is_demo: demo,
            list_armed: AtomicBool::new(false),
            timers: Mutex::new(Vec::new()),
            before_remove: Mutex::new(Vec::new()),
            tokens: crate::tokens::TokenScan::default(),
        })
    }

    /// A call on the registry, with what it told told once it's let go.
    pub fn with<T>(&self, w: &Runtime, f: impl FnOnce(&mut Registry, &mut ActorHost<'_>) -> registry::Result<T>) -> Result<T> {
        let (out, told) = self.in_turn(|| {
            let mut reg = self.reg.locked();
            let mut host = ActorHost { map: &w.map, names: &self.names };
            let out = f(&mut reg, &mut host);
            let told: Told = (reg.take_emits(), reg.take_changed());
            (out, told)
        });
        self.tell(w, told);
        out.map_err(thrown)
    }

    /// A read of the registry.
    pub fn read<T>(&self, f: impl FnOnce(&Registry) -> T) -> T {
        self.in_turn(|| f(&self.reg.locked()))
    }

    /// `f` with the store's turn taken before the registry's lock: the
    /// order an entry from another machine takes them in, since it's
    /// applied in a store transaction that reads the registry. Taken the
    /// other way round, a thread holding the registry would wait for the
    /// store while the store's holder waits for the registry. Inside a
    /// transaction this thread holds already, `f` is part of it.
    pub(crate) fn in_turn<T>(&self, f: impl FnOnce() -> T) -> T {
        let began = !self.store.with(EngineStore::in_transaction) && self.store.with(EngineStore::begin).is_ok();
        let out = f();
        if began {
            // What the registry wrote stays, as each of its writes did on
            // its own before.
            if self.store.with(EngineStore::commit).is_err() {
                self.store.with(EngineStore::rollback);
            }
        }
        out
    }

    fn tell(&self, w: &Runtime, (emits, changed): Told) {
        for (name, v) in emits {
            w.emit(From::Actors, name, v);
        }
        if changed {
            self.changed(w);
        }
    }

    /// The list goes out at most every LIST_THROTTLE_MS after a change,
    /// and only when what it says has changed.
    pub fn changed(&self, w: &Runtime) {
        if self.list_armed.swap(true, Ordering::SeqCst) {
            return;
        }
        w.after(LIST_THROTTLE_MS, |w| {
            let Some(a) = &w.actors else { return };
            a.list_armed.store(false, Ordering::SeqCst);
            if let Ok(list) = a.with(w, |r, h| r.list_if_changed(h)) {
                if !list.is_null() {
                    w.emit(From::Actors, "actors", list);
                }
            }
        });
    }

    fn slug_of(&self, id: &str) -> Result<String> {
        self.read(|r| r.slug_of(id)).map_err(thrown)
    }

    /// An actor gone: what runs before it goes (its remote connections
    /// revoked) first, then its record.
    pub fn remove(&self, w: &Runtime, id: &str) -> Result<()> {
        let slug = self.slug_of(id)?;
        let hooks = self.before_remove.locked().clone();
        for f in hooks {
            f(&slug);
        }
        self.with(w, |r, _| r.remove(id))
    }

    /// A registered actor that no longer reports itself: its remote
    /// connections revoked first unless kept, and its record gone when
    /// nothing else speaks for it.
    pub fn unregister(&self, w: &Runtime, id: &str, keep_remote: bool) -> Result<()> {
        let revokes = self.read(|r| r.unregister_revokes(id, keep_remote)).map_err(thrown)?;
        if revokes {
            let slug = self.slug_of(id)?;
            for f in self.before_remove.locked().clone() {
                f(&slug);
            }
        }
        if self.with(w, |r, _| r.unregister(id, revokes))? {
            self.remove(w, id)?;
        }
        Ok(())
    }

    /// The registry's clock, its prune, the registration's tick and the
    /// demo's scripts, while the service runs.
    pub fn start(&self, w: &Runtime) {
        let mut t = self.timers.locked();
        if !t.is_empty() {
            return;
        }
        if w.registration.is_some() {
            t.push(w.every(CLOCK_MS, |w| {
                if let Some(r) = &w.registration {
                    r.tick(w);
                }
            }));
        }
        if self.is_demo {
            t.push(w.every(STEP_MS, |w| {
                if let Some(a) = &w.actors {
                    a.step(w);
                }
            }));
        }
        t.push(w.every(CLOCK_MS, |w| {
            if let Some(a) = &w.actors {
                let _ = a.with(w, |r, h| r.tick(now_ms(), h));
            }
        }));
        t.push(w.every(PRUNE_MS, |w| {
            if let Some(a) = &w.actors {
                let _ = a.with(w, |r, _| r.prune(now_ms()));
            }
        }));
    }

    /// The demo's next turn.
    fn step(&self, w: &Runtime) {
        let mut d = self.demo.locked();
        let Some(d) = d.as_mut() else { return };
        let _ = self.with(w, |r, h| d.step(r, h));
    }

    pub fn stop(&self, w: &Runtime) {
        for t in self.timers.locked().drain(..) {
            t.abort();
        }
        let _ = self.with(w, |r, _| r.stop());
    }
}

/// The actors, as the routes call them.
pub struct ActorsDriver(pub Arc<Runtime>);

impl ActorsDriver {
    fn side(&self) -> R<&ActorSide> {
        self.0.actors.as_ref().ok_or_else(|| CoreError::pending("core/actors/index.js"))
    }

    fn with<T>(&self, f: impl FnOnce(&mut Registry, &mut ActorHost<'_>) -> registry::Result<T>) -> R<T> {
        self.side()?.with(&self.0, f).map_err(core_error)
    }
}

#[async_trait]
impl Actors for ActorsDriver {
    fn list(&self) -> R<Value> {
        self.side()?.with(&self.0, |r, h| r.list("public", Some(now_ms()), h)).map_err(core_error)
    }
    fn snapshot(&self, id: &str, view: Option<&str>, counts: bool) -> R<Option<Value>> {
        let a = self.side()?;
        let view = view.unwrap_or("public").to_string();
        let none: Vec<(String, [f64; 3])> = Vec::new();
        let v = a.with(&self.0, |r, h| r.snapshot(id, &view, Some(now_ms()), (!counts).then_some(&none[..]), h)).map_err(core_error)?;
        Ok((!v.is_null()).then_some(v))
    }
    fn events(&self, since: Option<f64>, only: Option<&str>, view: Option<&str>, limit: f64) -> R<Value> {
        let mut o = JsonMap::new();
        if let Some(since) = since {
            o.insert("since".into(), xenonet_text::json::number(since));
        }
        o.insert("limit".into(), xenonet_text::json::number(limit));
        if let Some(view) = view {
            o.insert("view".into(), json!(view));
        }
        o.insert("only".into(), only.map_or(Value::Null, |o| json!(o)));
        let o = Value::Object(o);
        self.side()?.read(|r| r.events(&o, now_ms())).map_err(|f| core_error(thrown(f)))
    }
    fn you_forget(&self) -> R<()> {
        self.side()?.with(&self.0, |r, _| r.you_forget()).map_err(core_error)
    }
    fn you_report(&self, body: &Value) -> R<Value> {
        self.side()?.with(&self.0, |r, h| r.you_report(body, now_ms(), h)).map_err(core_error)
    }
    fn you_window(&self, state: &Value) {
        if let Ok(a) = self.side() {
            let s = xenonet_text::text::display(state);
            let _ = a.with(&self.0, |r, h| r.you_window(&s, now_ms(), h));
        }
    }
    fn pids_of(&self, id: &str) -> R<Vec<u32>> {
        Ok(self.side()?.read(|r| r.pids_of(id)).into_iter().filter(|p| p.is_finite() && *p >= 0.0).map(|p| p as u32).collect())
    }
    /// Each registered actor's token, with its allow scope when it has
    /// one: what the Actors panel shows and narrows.
    fn tokens(&self, now: f64) -> R<Value> {
        let mut list = self.side()?.read(|r| r.tokens(now)).map_err(|f| core_error(thrown(f)))?;
        for t in &mut list {
            let slug = t.get("actor").and_then(Value::as_str).and_then(|a| a.strip_prefix("actor:")).map(str::to_string);
            if let Some(allows) = slug.and_then(|s| crate::grants::allows_of(&self.0, &s)) {
                t["allows"] = allows;
            }
        }
        Ok(Value::Array(list))
    }
    fn info(&self, id: &str) -> Option<Value> {
        let v = self.side().ok()?.read(|r| r.info(id));
        (!v.is_null()).then_some(v)
    }
    fn said(&self, message: &Value) {
        let Ok(a) = self.side() else { return };
        let to: Vec<String> = message.get("to").and_then(Value::as_array).map(|l| l.iter().map(xenonet_text::text::display).collect()).unwrap_or_default();
        let from = xenonet_text::text::display(message.get("from").unwrap_or(&Value::Null));
        let ts = message.get("ts").and_then(Value::as_f64);
        let _ = a.with(&self.0, |r, h| r.said(&from, &to, ts, h));
    }
    fn refresh(&self) {
        if let Ok(a) = self.side() {
            a.changed(&self.0);
        }
    }
    fn target(&self, actor: &str, id: &str, relation: &str) -> R<()> {
        self.side()?.with(&self.0, |r, _| r.target(actor, id, relation, now_ms())).map_err(core_error)
    }
    fn simulate(&self, kind: &str, a: Option<String>, b: Option<String>) -> R<Option<Value>> {
        let side = self.side()?;
        let (a, b) = (a.map_or(Value::Null, Value::String), b.map_or(Value::Null, Value::String));
        let v = side.with(&self.0, |r, h| r.simulate(kind, a, b, h)).map_err(core_error)?;
        Ok((!v.is_null()).then_some(v))
    }
    fn is_demo(&self) -> bool {
        self.side().is_ok_and(|a| a.is_demo)
    }
    fn ids(&self) -> Vec<String> {
        self.side().map(|a| a.read(registry::Actors::ids)).unwrap_or_default()
    }
    fn remote_of(&self, id: &str) -> Option<Value> {
        let v = self.side().ok()?.read(|r| r.remote_of(id));
        (!v.is_null()).then_some(v)
    }
    fn targets_of(&self, id: &str) -> Vec<Value> {
        self.side().map(|a| a.read(|r| r.targets_of(id))).unwrap_or_default()
    }
    fn observers_status(&self) -> Value {
        crate::observers::status(&self.0)
    }
    async fn token_files(&self, by_hand: bool) -> R<Value> {
        let w = self.0.clone();
        tokio::task::spawn_blocking(move || match &w.actors {
            Some(a) => Ok(a.tokens.look(&w, by_hand)),
            None => Err(CoreError::pending("core/actors/token-file.js")),
        })
        .await
        .map_err(|e| CoreError::new(e.to_string()))?
    }
    fn has(&self, id: &str) -> bool {
        self.side().is_ok_and(|a| a.read(|r| r.has(id)))
    }
    fn is_remote(&self, id: &str) -> bool {
        self.side().is_ok_and(|a| a.read(|r| r.is_remote(id)))
    }
    fn is_registered(&self, id: &str) -> bool {
        self.side().is_ok_and(|a| a.read(|r| r.is_registered(id)))
    }
    fn upsert(&self, def: &Value) -> R<()> {
        self.with(|r, h| r.upsert(def, h))
    }
    fn add_via(&self, id: &str, via: &str) -> R<()> {
        self.with(|r, _| r.add_via(id, via))
    }
    fn set_remote(&self, id: &str, remote: &Value) -> R<()> {
        self.with(|r, _| r.set_remote(id, remote))
    }
    fn report(&self, id: &str, report: &Value) -> R<()> {
        self.with(|r, h| r.report(id, report, h))
    }
    fn target_at(&self, actor: &str, id: &str, relation: &str, at: f64) -> R<()> {
        self.with(|r, _| r.target(actor, id, relation, at))
    }
    fn before_remove(&self, hook: Arc<dyn Fn(&str) + Send + Sync>) {
        if let Ok(a) = self.side() {
            a.before_remove.locked().push(hook);
        }
    }
    fn start(&self) {
        if let Some(a) = &self.0.actors {
            a.start(&self.0);
        }
        crate::observers::start(&self.0);
    }
    fn stop(&self) {
        crate::observers::stop(&self.0);
        if let Some(a) = &self.0.actors {
            a.stop(&self.0);
        }
    }
}
