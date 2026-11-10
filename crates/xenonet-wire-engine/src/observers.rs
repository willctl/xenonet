//! The observers of the actor products XenoNet observes, run over the
//! world: each adapter's observer (`Platform::observer`), on unless you
//! turned its product's observer off, never in the demo, whose actors are
//! made up and whose machine isn't read. They look together every 5
//! seconds, then the registered actors' spawned reports look for their
//! sessions; an observer that polls between looks (Grok Bot's, what its
//! app runs) does so on its own timer. The world answers what an observer
//! asks (`ObserverHost`): the actors, the process table, the files, the
//! engine's pins and the map, a product's database, and the vendor
//! readers.

use crate::runtime::Runtime;
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_adapter::observe::{self, Actors, Census, Db, Fail, Files, Observer, ObserverHost, Pins, Procs, SpawnsState, Stat, Vendor, LOOK_MS};
use xenonet_base::sync::Locked;
use xenonet_base::time::now_ms;
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_engine::engine::Thrown;
use xenonet_engine::sql::{self, Bind};
use xenonet_engine::store::row_json;
use xenonet_schema::drift::{DriftLog, Piece};
use xenonet_schema::vendor::{Checked, Readers};

/// One observer: a building block with its lifecycle.
pub struct One {
    obs: Box<dyn Observer>,
    life: Mutex<Lifecycle>,
    looking: Mutex<bool>,
    polling: Mutex<bool>,
    db: Mutex<Option<rusqlite::Connection>>,
}

pub struct Observers {
    list: Vec<One>,
    reported: SpawnsState,
    timers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

/// The observers the adapters bring, made once the world is, and their
/// blocks listed with the map's.
pub fn make(w: &std::sync::Arc<Runtime>) {
    let list: Vec<One> = xenonet_adapter::platforms()
        .observers()
        .into_iter()
        .map(|obs| One { obs, life: Mutex::new(Lifecycle::new(now_ms())), looking: Mutex::new(false), polling: Mutex::new(false), db: Mutex::new(None) })
        .collect();
    let n = list.len();
    if w.observers.set(Observers { list, reported: SpawnsState::default(), timers: Mutex::new(Vec::new()) }).is_err() {
        return;
    }
    for i in 0..n {
        let weak = std::sync::Arc::downgrade(w);
        let _ = w.state().add_block(
            std::sync::Arc::new(move || {
                let Some(w) = weak.upgrade() else { return Value::Null };
                let health = w.observers.get().and_then(|o| o.list.get(i)).map(|o| crate::store::block_health(o.obs.name(), "observer", &o.life.locked()));
                health.unwrap_or(Value::Null)
            }),
            None,
        );
    }
}

fn fail(t: Thrown) -> Fail {
    Fail { message: t.message, code: t.code }
}

/// What the world answers an observer, or the reports' matcher (which has
/// no observer of its own: it's always on, and keeps no block).
struct Host<'a> {
    w: &'a Runtime,
    o: Option<&'a One>,
}

impl Host<'_> {
    /// The service's vendor readers, which the map's readers share, so the
    /// drift report counts what the observers read (Hermes's files). Made
    /// here only when the map isn't wired, as in a test of this area alone.
    fn readers(&self) -> std::sync::Arc<Readers> {
        self.w.env.readers.get_or_make(|| Readers::new(self.w.setup.root.join("schemas"), self.w.env.drift.clone()))
    }

    fn drift(&self, p: &Piece) {
        let mut log = self.w.env.drift.locked();
        let log: &mut DriftLog = &mut log;
        log.record(p, now_ms());
    }

    fn piece(source: &str, kind: &str, path: Value, version: Option<&str>) -> Piece {
        Piece { source: source.into(), kind: kind.into(), path, vendor_version: version.map_or(Value::Null, |v| Value::String(v.into())), ..Default::default() }
    }

    fn actors(&self) -> Option<&crate::actors::ActorSide> {
        self.w.actors.as_ref()
    }

    fn with<T>(&self, f: impl FnOnce(&mut xenonet_actors::registry::Actors, &mut crate::actors::ActorHost<'_>) -> xenonet_actors::registry::Result<T>) -> observe::Result<T> {
        let Some(a) = self.actors() else { return Err(Fail::new("no actors")) };
        a.with(self.w, f).map_err(fail)
    }

    /// A read of the registry, or the empty answer without actors.
    fn reg<T: Default>(&self, f: impl FnOnce(&xenonet_actors::registry::Actors) -> T) -> T {
        self.actors().map_or_else(T::default, |a| f(&a.reg.locked()))
    }

    fn waiting_json(list: &[xenonet_actors::registry::Waiting]) -> Vec<Value> {
        list.iter().map(|w| json!({ "actor": w.actor, "app": w.app, "place": w.place, "own": w.own.iter().map(|p| xenonet_base::json::number(*p)).collect::<Vec<_>>(), "at": xenonet_base::json::number(w.at), "until": xenonet_base::json::number(w.until) })).collect()
    }

    fn sql(e: sql::SqlError) -> Fail {
        fail(Thrown::from(e))
    }
}

fn fs_fail(e: xenonet_base::fs::FsError) -> Fail {
    Fail { message: e.message, code: e.code }
}

fn io_fail(e: std::io::Error, syscall: &str, path: &str) -> Fail {
    fs_fail(xenonet_base::fs::FsError::of(e, syscall, path))
}

impl Files for Host<'_> {
    fn exists(&self, path: &str) -> bool {
        std::path::Path::new(path).exists()
    }
    fn read(&self, path: &str) -> observe::Result<Vec<u8>> {
        std::fs::read(path).map_err(|e| io_fail(e, "open", path))
    }
    fn read_text(&self, path: &str) -> observe::Result<String> {
        self.read(path).map(|b| String::from_utf8_lossy(&b).into_owned())
    }
    fn readdir(&self, path: &str) -> observe::Result<Vec<String>> {
        let ents = xenonet_base::paths::read_dir_sorted(std::path::Path::new(path)).map_err(|e| io_fail(e, "scandir", path))?;
        Ok(ents.into_iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    }
    fn dirs(&self, path: &str) -> observe::Result<Vec<String>> {
        let ents = xenonet_base::paths::read_dir_sorted(std::path::Path::new(path)).map_err(|e| io_fail(e, "scandir", path))?;
        Ok(ents.into_iter().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    }
    fn stat(&self, path: &str) -> observe::Result<Stat> {
        let m = std::fs::metadata(path).map_err(|e| io_fail(e, "stat", path))?;
        Ok(Stat { mtime_ms: xenonet_base::fs::mtime_ms_of(&m).unwrap_or(f64::NAN), size: m.len() as f64 })
    }
}

impl Procs for Host<'_> {
    fn supported(&self) -> bool {
        self.w.sys.procs.supported
    }
    fn census(&self) -> observe::Result<Census> {
        crate::procs::census(&self.w.sys).map(|v| Census::of(&v)).map_err(Fail::new)
    }
    fn alive(&self, pid: f64) -> bool {
        crate::runtime::alive(pid)
    }
    fn cwds(&self, l: &[f64]) -> Vec<(f64, String)> {
        let sys = &self.w.sys;
        sys.procs.cwds(&mut sys.io(), l).iter().map(|(pid, s)| (*pid, s.clone())).collect()
    }
    fn children(&self, l: &[f64]) -> Vec<f64> {
        let sys = &self.w.sys;
        sys.procs.children(&mut sys.io(), l)
    }
    fn describe(&self, l: &[f64]) -> Vec<Value> {
        let sys = &self.w.sys;
        sys.procs.describe(&mut sys.io(), l).iter().map(|(_, r)| Value::Object(r.json())).collect()
    }
    fn agent_of(&self, program: &str) -> Option<String> {
        xenonet_engine::processes::agent_of(program, "").map(str::to_string)
    }
    fn wants_args(&self, program: &str) -> bool {
        xenonet_engine::procs::wants_args(program)
    }
}

impl Actors for Host<'_> {
    fn has(&self, id: &str) -> bool {
        self.reg(|r| r.has(id))
    }
    fn info(&self, id: &str) -> Value {
        self.reg(|r| r.info(id))
    }
    fn presence(&self, id: &str) -> Option<String> {
        self.reg(|r| r.presence(id))
    }
    fn installed(&self, id: &str) -> Option<bool> {
        self.reg(|r| r.installed(id))
    }
    fn enabled(&self, id: &str) -> bool {
        self.reg(|r| r.enabled(id))
    }
    fn is_registered(&self, id: &str) -> bool {
        self.reg(|r| r.is_registered(id))
    }
    fn of_product(&self, product: &str) -> Vec<String> {
        self.reg(|r| r.of_product(Some(&Value::String(product.into()))))
    }
    fn pids_of(&self, id: &str) -> Vec<f64> {
        self.reg(|r| r.pids_of(id))
    }
    fn targets_of(&self, id: &str) -> Vec<Value> {
        self.reg(|r| r.targets_of(id))
    }
    fn upsert(&self, def: &Value) -> observe::Result<()> {
        self.with(|r, h| r.upsert(def, h))
    }
    fn set_installed(&self, id: &str, installed: bool) -> observe::Result<()> {
        self.with(|r, _| r.set_installed(id, installed))
    }
    fn report(&self, id: &str, report: &Value) -> observe::Result<()> {
        self.with(|r, h| r.report(id, report, h))
    }
    fn record(&self, id: &str, event: &Value) -> observe::Result<Value> {
        self.with(|r, h| r.record(id, event, h))
    }
    fn target(&self, id: &str, node: &str, relation: &str, at: f64) -> observe::Result<()> {
        self.with(|r, _| r.target(id, node, relation, at))
    }
    fn where_of(&self, target: &Value) -> Value {
        self.with(|r, h| Ok(r.where_of(target, h))).unwrap_or(Value::Null)
    }
    fn place_of(&self, path: &str) -> Value {
        let p = Value::String(path.into());
        self.with(|r, h| Ok(r.place_of(&p, h))).unwrap_or(Value::Null)
    }
    fn has_ref(&self, id: &str, r: &str, since: f64) -> observe::Result<bool> {
        let Some(a) = self.actors() else { return Err(Fail::new("no actors")) };
        a.store.with(|s| s.has_actor_ref(id, r, since)).map_err(Host::sql)
    }
    fn expire_waiting(&self, at: f64) -> Vec<Value> {
        self.actors().map_or_else(Vec::new, |a| Host::waiting_json(&a.reg.locked().expire_waiting(at)))
    }
    fn waiting(&self) -> Vec<Value> {
        self.actors().map_or_else(Vec::new, |a| Host::waiting_json(&a.reg.locked().waiting))
    }
    fn drop_waiting_at(&self, index: usize) {
        if let Some(a) = self.actors() {
            a.reg.locked().drop_waiting_at(index);
        }
    }
}

impl Pins for Host<'_> {
    fn map_ready(&self) -> Option<bool> {
        Some(self.w.map.ready())
    }
    fn sessions(&self) -> observe::Result<Vec<Value>> {
        Ok(self.w.map.sessions().map_err(fail)?.as_array().cloned().unwrap_or_default())
    }
    fn pins(&self, session: &str) -> observe::Result<Vec<Value>> {
        self.w.store.with(|s| s.pins(session)).map(|rows| rows.iter().map(row_json).collect()).map_err(Host::sql)
    }
    fn pinned_to(&self, pid: f64) -> observe::Result<Vec<Value>> {
        self.w.store.with(|s| s.pinned_to(pid)).map(|rows| rows.iter().map(row_json).collect()).map_err(Host::sql)
    }
}

impl Db for Host<'_> {
    fn open(&self, file: &str) -> observe::Result<()> {
        let Some(o) = self.o else { return Err(Fail::new("no observer")) };
        let c = rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| Host::sql(sql::SqlError::from(e)))?;
        *o.db.locked() = Some(c);
        Ok(())
    }
    fn all(&self, sql: &str, args: &[Value]) -> observe::Result<Vec<Value>> {
        let Some(o) = self.o else { return Err(Fail::new("no observer")) };
        let db = o.db.locked();
        let Some(c) = db.as_ref() else { return Err(Fail::new("the database isn't open")) };
        let params = args.iter().enumerate().map(|(i, v)| Bind::scalar(v).ok_or_else(|| sql::unbindable(i + 1))).collect::<sql::Result<Vec<Bind>>>().map_err(Host::sql)?;
        let rows = sql::all(c, sql, &params).map_err(Host::sql)?;
        Ok(rows.iter().map(row_json).collect())
    }
    fn get(&self, sql: &str, args: &[Value]) -> observe::Result<Value> {
        Ok(self.all(sql, args)?.into_iter().next().unwrap_or(Value::Null))
    }
    fn close(&self) {
        if let Some(o) = self.o {
            *o.db.locked() = None;
        }
    }
}

impl Vendor for Host<'_> {
    // The readers lock the drift log themselves, after their own lock.
    fn check(&self, source: &str, record: &Value, version: Option<&str>) -> Value {
        match self.readers().check(source, record, version, None, now_ms()) {
            Checked::Same => record.clone(),
            Checked::Stripped(x) => x,
            Checked::Dropped => Value::Null,
        }
    }
    fn unparsable(&self, source: &str, version: Option<&str>) {
        self.drift(&Host::piece(source, "unparsable", Value::Null, version));
    }
    fn missing(&self, source: &str, path: &str, version: Option<&str>) {
        self.drift(&Host::piece(source, "missing-field", Value::String(path.into()), version));
    }
}

impl ObserverHost for Host<'_> {
    fn now(&self) -> f64 {
        now_ms()
    }
    fn home(&self) -> &str {
        &self.w.setup.home
    }
    fn observing(&self) -> bool {
        match self.o {
            Some(o) => self.w.registration.as_ref().is_none_or(|r| r.observing(self.w, o.obs.product().id)),
            None => true,
        }
    }
    fn ok(&self, at: f64) {
        if let Some(o) = self.o {
            o.life.locked().ok(at);
        }
    }
    fn problem(&self, message: &str, at: f64) {
        if let Some(o) = self.o {
            o.life.locked().problem(message, at);
        }
    }
    fn actors(&self) -> &dyn Actors {
        self
    }
    fn procs(&self) -> &dyn Procs {
        self
    }
    fn files(&self) -> &dyn Files {
        self
    }
    fn spawns(&self) -> &dyn Pins {
        self
    }
    fn db(&self) -> &dyn Db {
        self
    }
    fn vendor(&self) -> &dyn Vendor {
        self
    }
    // A key of a binary Info.plist, through plutil.
    fn plist_key(&self, file: &str, key: &str) -> Option<String> {
        let a = ["-extract".to_string(), key.to_string(), "raw".into(), "-o".into(), "-".into(), file.to_string()];
        crate::exec::run(&crate::exec::Spec::new("plutil", &a, 3000)).ok().map(|out| xenonet_base::text::js_trim(&out).to_string())
    }
}

impl Observers {
    /// One observer's look, unless it's off or the map isn't there yet.
    fn tick(w: &Runtime, o: &One) {
        let h = Host { w, o: Some(o) };
        if !observe::may_look(&h) {
            return;
        }
        {
            let mut busy = o.looking.locked();
            if *busy {
                return;
            }
            *busy = true;
        }
        observe::tick(o.obs.as_ref(), &h, now_ms());
        *o.looking.locked() = false;
    }

    /// Every observer's look, one after the other, then the waiting reports.
    fn look(&self, w: &Runtime) {
        for o in &self.list {
            Self::tick(w, o);
        }
        // Before XenoNet's first look at the map, no session could be named.
        // A look that fails leaves the reports as they were told.
        if !w.map.ready() {
            return;
        }
        let _ = observe::attribution::reported_look(&self.reported, &Host { w, o: None }, now_ms());
    }

    /// The polls of the observers that poll, between looks.
    fn poll(&self, w: &Runtime, ms: f64) {
        for o in &self.list {
            if o.obs.poll_ms() != Some(ms) {
                continue;
            }
            let h = Host { w, o: Some(o) };
            if !o.obs.may_poll(&h) {
                continue;
            }
            {
                let mut busy = o.polling.locked();
                if *busy {
                    continue;
                }
                *busy = true;
            }
            if let Err(e) = o.obs.poll(&h, now_ms()) {
                o.life.locked().problem(&e.message, now_ms());
            }
            *o.polling.locked() = false;
        }
    }

    fn status(&self, w: &Runtime) -> Value {
        Value::Array(
            self.list
                .iter()
                .map(|o| {
                    let h = Host { w, o: Some(o) };
                    let p = o.obs.product();
                    let mut s = o.obs.status(&h).unwrap_or_else(|e| json!({ "product": p.id, "name": p.name, "error": e.message }));
                    if let Value::Object(m) = &mut s {
                        m.insert("on".into(), Value::Bool(h.observing()));
                    }
                    s
                })
                .collect(),
        )
    }
}

/// What each observer reads, for the doctor: none in the demo.
pub fn status(w: &Runtime) -> Value {
    w.observers.get().map_or_else(|| Value::Array(Vec::new()), |o| o.status(w))
}

/// The observers' looks and polls, while the service runs.
pub fn start(w: &Runtime) {
    let Some(obs) = w.observers.get() else { return };
    let mut timers = obs.timers.locked();
    if !timers.is_empty() {
        return;
    }
    for o in &obs.list {
        let mut l = o.life.locked();
        if let Some(run) = l.begin_start(now_ms()) {
            l.start_done(run, now_ms());
        }
    }
    let looks = w.me.get().cloned().unwrap_or_default();
    timers.push(w.rt.spawn(async move {
        loop {
            let Some(w) = looks.upgrade() else { return };
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(o) = w.observers.get() {
                    o.look(&w);
                }
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs_f64(LOOK_MS / 1000.0)).await;
        }
    }));
    let mut periods: Vec<f64> = Vec::new();
    for ms in obs.list.iter().filter_map(|o| o.obs.poll_ms()) {
        if !periods.contains(&ms) {
            periods.push(ms);
        }
    }
    for ms in periods {
        timers.push(w.every(ms, move |w| {
            if let Some(o) = w.observers.get() {
                o.poll(w, ms);
            }
        }));
    }
}

pub fn stop(w: &Runtime) {
    let Some(obs) = w.observers.get() else { return };
    for t in obs.timers.locked().drain(..) {
        t.abort();
    }
    for o in &obs.list {
        o.life.locked().stop(now_ms());
        *o.db.locked() = None;
    }
}

/// An observer's name and what it observes, for a test.
pub fn names(w: &Runtime) -> Vec<(String, String)> {
    w.observers.get().map(|o| o.list.iter().map(|x| (x.obs.name().to_string(), x.obs.product().id.to_string())).collect()).unwrap_or_default()
}
