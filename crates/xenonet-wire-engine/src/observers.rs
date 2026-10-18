//! The observers of the actor products XenoNet observes
//! (core/actors/observers.js, observer.js, hermes.js, grok-bot.js, muse.js,
//! app-bundle.js, attribution.js over their Rust cores): Hermes Agent's,
//! Grok Bot's and Muse's, each on unless you turned its product's observer
//! off, never in the demo, whose actors are made up and whose machine isn't
//! read. They look together every 5 seconds, then the registered actors'
//! spawned reports look for their sessions; Grok Bot's also polls what its
//! app runs every 3 seconds while the app runs. Each observer's
//! collaborators are its own fields as the JS read them ("actors", "app",
//! "procs", "files", "spawns", "vendor", "db", keep, say…), answered here.

use crate::actors::{collab_call, collab_get, collab_kind};
use crate::js::{arg, at, json, no_method, now_ms, text, Locked};
use crate::world::World;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_actors::app_bundle::AppBundle;
use xenonet_actors::attribution::{self, Paths, SpawnsState};
use xenonet_actors::grok_bot::{GrokBot, POLL_MS};
use xenonet_actors::hermes::Hermes;
use xenonet_actors::muse::Muse;
use xenonet_actors::observer::{Observer as Core, LOOK_MS};
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::products::{product_of, Product};
use xenonet_engine::sql::Arg;
use xenonet_schema::drift::{DriftLog, Piece};
use xenonet_schema::vendor::{Checked, VendorReader};

/// Each kind's own look.
enum Kind {
    Hermes(Hermes),
    Grok(GrokBot),
    Muse(Muse),
}

/// A product's Mac app, as its observer reads it.
struct App {
    bundle: AppBundle,
    apps: Vec<String>,
    id: regex::Regex,
}

/// One observer: a building block with its lifecycle.
pub struct One {
    name: &'static str,
    product: &'static Product,
    core: Core,
    kind: Kind,
    app: Option<App>,
    life: Mutex<Lifecycle>,
    looking: Mutex<bool>,
    polling: Mutex<bool>,
    db: Mutex<Option<rusqlite::Connection>>,
    home: String,
}

/// The lenient readers of the vendor sources the observers read (Hermes's
/// files), recording drift in the service's one log.
#[derive(Default)]
struct Vendor {
    dir: std::path::PathBuf,
    readers: Mutex<HashMap<String, VendorReader>>,
}

pub struct Observers {
    list: Vec<One>,
    reported: Arc<SpawnsState>,
    vendor: Vendor,
    timers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

fn spawns_paths() -> Paths {
    Paths { store: "store".into(), map: "map".into(), procs: "procs".into() }
}

fn apps_of(home: &str, bundle: &str) -> Vec<String> {
    ["/Applications".to_string(), xenonet_base::paths::join(home, "Applications")].iter().map(|d| xenonet_base::paths::join(d, bundle)).collect()
}

/// The observers of Hermes Agent, Grok Bot and Muse, made once the world
/// is, and their blocks listed with the map's.
pub fn make(w: &Arc<World>) {
    let home = w.setup.home.clone();
    let product = |id: &str| product_of(id).expect("the observed products are in the table");
    let one = |name: &'static str, id: &str, kind: Kind, app: Option<App>| One {
        name,
        product: product(id),
        core: Core::default(),
        kind,
        app,
        life: Mutex::new(Lifecycle::new(now_ms())),
        looking: Mutex::new(false),
        polling: Mutex::new(false),
        db: Mutex::new(None),
        home: home.clone(),
    };
    let root = match std::env::var("HERMES_HOME").ok().filter(|h| !h.is_empty()) {
        Some(h) => xenonet_base::paths::resolve(&h),
        None => xenonet_base::paths::join(&home, ".hermes"),
    };
    let id = |pattern: &str| regex::Regex::new(pattern).expect("a bundle id pattern that compiles");
    let list = vec![
        one("hermes-observer", "hermes-agent", Kind::Hermes(Hermes::new(Arc::new(SpawnsState::default()), &root, &home)), None),
        one(
            "grok-bot-observer",
            "grok-bot",
            Kind::Grok(GrokBot::new(Arc::new(SpawnsState::default()))),
            Some(App { bundle: AppBundle::default(), apps: apps_of(&home, "Grok Bot.app"), id: id(r"^com\.anysphere\.sand(\.|$)") }),
        ),
        one("muse-observer", "muse", Kind::Muse(Muse::default()), Some(App { bundle: AppBundle::default(), apps: apps_of(&home, "Muse.app"), id: id(r"^com\.meta\.endo(\.|$)") })),
    ];
    let vendor = Vendor { dir: w.setup.root.join("schemas"), ..Vendor::default() };
    if w.observers.set(Observers { list, reported: Arc::new(SpawnsState::default()), vendor, timers: Mutex::new(Vec::new()) }).is_err() {
        return;
    }
    for i in 0..3 {
        let weak = Arc::downgrade(w);
        let _ = w.state().add_block(
            Arc::new(move || {
                let Some(w) = weak.upgrade() else { return Value::Null };
                let health = w.observers.get().and_then(|o| o.list.get(i)).map(|o| crate::store::block_health(o.name, "observer", &o.life.locked()));
                health.unwrap_or(Value::Null)
            }),
            None,
        );
    }
}

/// One observer's collaborators: its own fields and methods.
struct ObsCalls<'a> {
    w: &'a World,
    obs: &'a Observers,
    o: &'a One,
}

/// An app bundle's collaborators: "apps", "id", "files" and "binary".
struct AppCalls<'a> {
    app: &'a App,
}

impl Calls for AppCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("files", m) => crate::fsx::call(m, &args),
            ("id", "test") => Ok(Arg::Bool(self.app.id.is_match(&text(&at(&args, 0))))),
            // A key of a binary Info.plist, through plutil.
            ("", "binary") => {
                let a = ["-extract".to_string(), text(&at(&args, 1)), "raw".into(), "-o".into(), "-".into(), text(&at(&args, 0))];
                Ok(match crate::exec::run(&crate::exec::Spec::new("plutil", &a, 3000)) {
                    Ok(out) => Arg::Text(xenonet_base::text::js_trim(&out).to_string()),
                    Err(_) => Arg::Null,
                })
            }
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "apps" => Arg::List(self.app.apps.iter().map(Arg::text).collect()),
            "id" => Arg::text(self.app.id.as_str()),
            _ => Arg::Undef,
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "apps" | "id" | "files" => "object",
            _ => "function",
        }
        .into())
    }
}

/// A row as node:sqlite's prepare(sql).all(...args) reads it.
fn db_rows(db: &rusqlite::Connection, sql: &str, args: &[Arg]) -> Result<Arg> {
    xenonet_engine::sql::all(db, sql, args).map(|r| crate::js::rows(&r)).map_err(Thrown::from)
}

impl ObsCalls<'_> {
    fn app(&self) -> Result<&App> {
        self.o.app.as_ref().ok_or_else(|| Thrown::coded("Cannot read properties of undefined (reading 'read')", "TypeError"))
    }

    fn observing(&self) -> bool {
        self.w.registration.as_ref().is_none_or(|r| r.observing(self.w, self.o.product.id))
    }

    fn vendor(&self, op: &str, args: &[Arg]) -> Result<Arg> {
        let source = text(&at(args, 0));
        let version = |i: usize| at(args, i).as_str().map(str::to_string);
        let v = &self.obs.vendor;
        let mut log = self.w.env.drift.locked();
        let log: &mut DriftLog = &mut log;
        let piece = |kind: &str, path: Value, version: Option<String>| Piece {
            source: source.clone(),
            kind: kind.into(),
            path,
            vendor_version: version.map_or(Value::Null, Value::String),
            ..Default::default()
        };
        match op {
            "check" => {
                let record = json(&at(args, 1));
                let mut readers = v.readers.locked();
                let r = readers.entry(source.clone()).or_insert_with(|| VendorReader::new(&source, v.dir.clone(), None));
                r.stats.records += 1;
                Ok(match r.inspect(&record, version(2).as_deref(), None, log, now_ms()) {
                    Checked::Same => at(args, 1),
                    Checked::Stripped(x) => arg(&x),
                    Checked::Dropped => Arg::Null,
                })
            }
            "unparsable" => {
                log.record(&piece("unparsable", Value::Null, version(1)), now_ms());
                Ok(Arg::Undef)
            }
            "missing" => {
                log.record(&piece("missing-field", json(&at(args, 1)), version(2)), now_ms());
                Ok(Arg::Undef)
            }
            _ => Err(no_method("vendor", op)),
        }
    }

    fn db(&self, op: &str, args: &[Arg]) -> Result<Arg> {
        let mut db = self.o.db.locked();
        match op {
            "open" => {
                let file = text(&at(args, 0));
                let c = rusqlite::Connection::open_with_flags(&file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
                    .map_err(|e| Thrown::from(xenonet_engine::sql::SqlError::from(e)))?;
                *db = Some(c);
                Ok(Arg::Undef)
            }
            "all" | "get" => {
                let Some(c) = db.as_ref() else { return Err(Thrown::coded("Cannot read properties of null (reading 'prepare')", "TypeError")) };
                let params = crate::js::list(at(args, 1));
                let rows = db_rows(c, &text(&at(args, 0)), &params)?;
                Ok(if op == "all" { rows } else { crate::js::list(rows).into_iter().next().unwrap_or(Arg::Null) })
            }
            "close" => {
                *db = None;
                Ok(Arg::Undef)
            }
            _ => Err(no_method("db", op)),
        }
    }

    fn look(&mut self, at_: f64) -> Result<()> {
        let o = self.o;
        match &o.kind {
            Kind::Hermes(h) => h.look(self, at_),
            Kind::Grok(g) => g.look(self, at_),
            Kind::Muse(m) => m.look(self, at_),
        }
    }
}

impl Calls for ObsCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let x = |i| at(&args, i);
        let (w, o) = (self.w, self.o);
        match (path, method) {
            ("clock", "now") => Ok(Arg::Num(now_ms())),
            ("", "observing") => Ok(Arg::Bool(self.observing())),
            ("", "look") => {
                let t = x(0).as_f64().unwrap_or_else(now_ms);
                self.look(t).map(|_| Arg::Undef)
            }
            ("", "ok") => {
                o.life.locked().ok(x(0).as_f64().unwrap_or_else(now_ms));
                Ok(Arg::Undef)
            }
            ("", "problem") => {
                o.life.locked().problem(&text(&x(0)), x(1).as_f64().unwrap_or_else(now_ms));
                Ok(Arg::Undef)
            }
            ("", "keep") => o.core.keep(self, &text(&x(0)), &x(1)).map(|_| Arg::Undef),
            ("", "say") => o.core.say(self, &text(&x(0)), &x(1)).map(Arg::Bool),
            ("", "happened") => o.core.happened(self, &text(&x(0)), &x(1)),
            ("", "gather") => {
                o.core.gather(&text(&x(0)), &x(1));
                Ok(Arg::Undef)
            }
            ("", "touch") => {
                let t = match x(3) {
                    Arg::Undef => Arg::Num(now_ms()),
                    t => t,
                };
                o.core.touch(self, &text(&x(0)), &x(1), &x(2), &t).map(|_| Arg::Undef)
            }
            ("", "__census" | "census") => crate::procs::census(&w.sys).map(|v| arg(&v)),
            ("", "alive") => Ok(Arg::Bool(x(0).as_f64().is_some_and(crate::js::alive))),
            ("", "dirs") => crate::fsx::dirs(&text(&x(0))),
            ("", "dateParse") => Ok(Arg::Num(xenonet_base::time::parse_iso(&text(&x(0))).unwrap_or(f64::NAN))),
            ("files", m) => crate::fsx::call(m, &args),
            ("procs" | "spawns.procs", m) => crate::procs::call(&w.sys, m, &args),
            ("spawns.store", m) => crate::store::call(&w.store, m, &args),
            ("spawns.map", m) => w.map.call(m, &args),
            ("vendor", m) => self.vendor(m, &args),
            ("db", m) => self.db(m, &args),
            ("app", m) => {
                let app = self.app()?;
                let c = &mut AppCalls { app };
                match m {
                    "installed" => app.bundle.installed(c).map(Arg::Bool),
                    "ours" => app.bundle.ours(c).map(Arg::Bool),
                    "read" => app.bundle.read(c).map(|p| arg(&p.json())),
                    "where" => app.bundle.where_(c).map(|p| p.map_or(Arg::Null, Arg::Text)),
                    "sources" => app.bundle.sources(c, &text(&x(0))).map(|v| arg(&v)),
                    _ => Err(no_method("app", m)),
                }
            }
            (p, m) if p == "actors" || p.starts_with("actors.") => collab_call(w, p, m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        let p = self.o.product;
        Ok(match path {
            "product.id" => Arg::text(p.id),
            "product.vendor" => Arg::text(p.vendor),
            "product.name" => Arg::text(p.name),
            "product.slug" => Arg::text(p.slug),
            "home" => Arg::text(&self.o.home),
            "procs.supported" | "spawns.procs.supported" => crate::procs::get(&self.w.sys, "supported"),
            "app.plist" => self.o.app.as_ref().and_then(|a| a.bundle.plist()).map_or(Arg::Null, |pl| arg(&pl.json())),
            other => collab_get(self.w, other),
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "product" | "procs" | "files" | "vendor" | "db" | "clock" | "spawns" | "spawns.store" | "spawns.map" | "spawns.procs" => "object",
            "app" => {
                if self.o.app.is_some() {
                    "object"
                } else {
                    "undefined"
                }
            }
            "home" => "string",
            p => match p.strip_prefix("spawns.map.") {
                Some(m) => crate::map::Map::kind(m),
                None => match p.strip_prefix("spawns.store.") {
                    Some(m) => crate::store::kind(m),
                    None => match p.strip_prefix("spawns.procs.").or_else(|| p.strip_prefix("procs.")) {
                        Some(m) => crate::procs::kind(m),
                        None if p.starts_with("actors") => collab_kind(self.w, p),
                        None => "function",
                    },
                },
            },
        }
        .into())
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

/// The reports' matcher's collaborators (ReportedSpawns.look): the actors,
/// procs, the store and the map, and the census.
struct ReportedCalls<'a> {
    w: &'a World,
}

impl Calls for ReportedCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let w = self.w;
        match (path, method) {
            ("", "census") => crate::procs::census(&w.sys).map(|v| arg(&v)),
            ("procs", m) => crate::procs::call(&w.sys, m, &args),
            ("store", m) => crate::store::call(&w.store, m, &args),
            ("map", m) => w.map.call(m, &args),
            (p, m) if p == "actors" || p.starts_with("actors.") => collab_call(w, p, m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "procs.supported" => crate::procs::get(&self.w.sys, "supported"),
            p => collab_get(self.w, p),
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "store" | "map" | "procs" => "object",
            p => match p.strip_prefix("map.") {
                Some(m) => crate::map::Map::kind(m),
                None => match p.strip_prefix("store.") {
                    Some(m) => crate::store::kind(m),
                    None => match p.strip_prefix("procs.") {
                        Some(m) => crate::procs::kind(m),
                        None => collab_kind(self.w, p),
                    },
                },
            },
        }
        .into())
    }
}

impl Observers {
    /// One observer's look, unless it's off or the map isn't there yet.
    fn tick(&self, w: &World, o: &One) {
        let mut c = ObsCalls { w, obs: self, o };
        if !Core::may_look(&mut c).unwrap_or(false) {
            return;
        }
        {
            let mut busy = o.looking.locked();
            if *busy {
                return;
            }
            *busy = true;
        }
        let _ = o.core.tick(&mut c, now_ms());
        *o.looking.locked() = false;
    }

    /// Every observer's look, one after the other, then the waiting reports.
    fn look(&self, w: &World) {
        for o in &self.list {
            self.tick(w, o);
        }
        if !w.map.ready() {
            return;
        }
        let _ = attribution::reported_look(&self.reported, &mut ReportedCalls { w }, &spawns_paths(), now_ms());
    }

    /// Grok Bot's poll of what its app runs.
    fn poll(&self, w: &World) {
        for o in &self.list {
            let Kind::Grok(g) = &o.kind else { continue };
            let mut c = ObsCalls { w, obs: self, o };
            if !g.may_poll(&mut c).unwrap_or(false) {
                continue;
            }
            {
                let mut busy = o.polling.locked();
                if *busy {
                    continue;
                }
                *busy = true;
            }
            if let Err(e) = g.poll(&mut c, now_ms()) {
                o.life.locked().problem(&e.message, now_ms());
            }
            *o.polling.locked() = false;
        }
    }

    fn status(&self, w: &World) -> Value {
        Value::Array(
            self.list
                .iter()
                .map(|o| {
                    let mut c = ObsCalls { w, obs: self, o };
                    let s = match &o.kind {
                        Kind::Hermes(h) => h.status(&mut c),
                        Kind::Grok(g) => g.status(&mut c, &o.home),
                        Kind::Muse(m) => m.status(&mut c, &o.home),
                    };
                    let mut s = s.unwrap_or_else(|e| json!({ "product": o.product.id, "name": o.product.name, "error": e.message }));
                    if let Value::Object(m) = &mut s {
                        m.insert("on".into(), Value::Bool(c.observing()));
                    }
                    s
                })
                .collect(),
        )
    }
}

/// What each observer reads, for the doctor: none in the demo.
pub fn status(w: &World) -> Value {
    w.observers.get().map_or_else(|| Value::Array(Vec::new()), |o| o.status(w))
}

/// The observers' looks and Grok Bot's poll, while the service runs.
pub fn start(w: &World) {
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
    let weak = w.me.get().cloned().unwrap_or_default();
    let looks = weak.clone();
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
    timers.push(w.every(POLL_MS, |w| {
        if let Some(o) = w.observers.get() {
            o.poll(w);
        }
    }));
}

pub fn stop(w: &World) {
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
pub fn names(w: &World) -> Vec<(String, String)> {
    w.observers.get().map(|o| o.list.iter().map(|x| (x.name.to_string(), x.product.id.to_string())).collect()).unwrap_or_default()
}
