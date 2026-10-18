//! Actors (core/actors/index.js over crates/xenonet-actors/src/registry.rs,
//! with you.js and demo.js): the registry, its host (the clock, the map,
//! whether a process runs), the list's throttle, the clock that ticks it,
//! the demo's made-up actors and their scripts, and the JS API the other
//! cores call it by ("actors.has", "actors.upsert"…). What it tells goes
//! to the page once the call is done, never while it's held.

use crate::js::{arg, at, json, list, no_method, now_ms, obj, text, Locked};
use crate::map::Map;
use crate::world::World;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use xenonet_actors::demo::{Demo, Resolve, STEP_MS};
use xenonet_actors::registry::{self, Actors as Registry, Fail, Host, CLOCK_MS, LIST_THROTTLE_MS, PRUNE_MS};
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;
use xenonet_service::cores::{Actors, CoreError, R};
use xenonet_service::glue::From;

/// The demo world's nodes, as resolveDemo finds them: its projects folder,
/// and its sessions by title.
#[derive(Clone, Default)]
pub struct DemoNames {
    projects_dir: String,
    sessions: Vec<(String, String)>,
}

impl DemoNames {
    pub fn new(projects_dir: &str) -> DemoNames {
        let list = xenonet_app::demo::Demo::new("", "0", "1970-01-01").session_list();
        let sessions = list.iter().filter_map(|s| Some((s.get("title")?.as_str()?.to_string(), s.get("id")?.as_str()?.to_string()))).collect();
        DemoNames { projects_dir: projects_dir.to_string(), sessions }
    }

    fn resolve(&self, kind: &str, what: &str) -> Value {
        let file = || what.split('/').fold(self.projects_dir.clone(), |p, part| xenonet_base::paths::join(&p, part));
        match kind {
            "project" => json!(format!("project:{}", xenonet_base::paths::path_key(&file()).unwrap_or_default())),
            "session" => self.sessions.iter().find(|(t, _)| t == what).map_or(Value::Null, |(_, id)| json!(id)),
            "file" => json!(file()),
            "identity" => {
                let a = xenonet_app::demo::demo_claude_account(Some(what));
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
        crate::js::alive(pid)
    }
}

impl Resolve for ActorHost<'_> {
    fn resolve(&mut self, kind: &str, what: &str) -> Value {
        self.names.resolve(kind, what)
    }
}

fn thrown(f: Fail) -> Thrown {
    match f {
        Fail::Error(m) => Thrown::new(m),
        Fail::Sql(e) => e.into(),
    }
}

pub fn core_error(t: Thrown) -> CoreError {
    CoreError { code: t.code.as_ref().and_then(Value::as_str).map(str::to_string), ..CoreError::new(t.message) }
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
        let mut reg = Registry::new(store.clone(), demo, &mut host).map_err(|f| thrown(f).message)?;
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
    pub fn with<T>(&self, w: &World, f: impl FnOnce(&mut Registry, &mut ActorHost) -> registry::Result<T>) -> Result<T> {
        let (out, told) = {
            let mut reg = self.reg.locked();
            let mut host = ActorHost { map: &w.map, names: &self.names };
            let out = f(&mut reg, &mut host);
            let told: Told = (reg.take_emits(), reg.take_changed());
            (out, told)
        };
        self.tell(w, told);
        out.map_err(thrown)
    }

    fn read<T>(&self, f: impl FnOnce(&Registry) -> T) -> T {
        f(&self.reg.locked())
    }

    fn tell(&self, w: &World, (emits, changed): Told) {
        for (name, v) in emits {
            w.emit(From::Actors, name, v);
        }
        if changed {
            self.changed(w);
        }
    }

    /// The list goes out at most every LIST_THROTTLE_MS after a change,
    /// and only when what it says has changed.
    pub fn changed(&self, w: &World) {
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

    fn remove(&self, w: &World, id: &str) -> Result<()> {
        let slug = self.slug_of(id)?;
        let hooks = self.before_remove.locked().clone();
        for f in hooks {
            f(&slug);
        }
        self.with(w, |r, _| r.remove(id))
    }

    /// `actors.<method>(...args)`, as the Actors class answers it.
    pub fn call(&self, w: &World, method: &str, args: &[Arg]) -> Result<Arg> {
        let x = |i| at(args, i);
        let s = |i| text(&at(args, i));
        let now_or = |a: Arg| match a {
            Arg::Undef => now_ms(),
            a => crate::js::num(&a),
        };
        Ok(match method {
            "has" => Arg::Bool(self.read(|r| r.has(&s(0)))),
            "ids" => Arg::List(self.read(|r| r.ids()).into_iter().map(Arg::Text).collect()),
            "presence" => self.read(|r| r.presence(&s(0))).map_or(Arg::Null, Arg::Text),
            "presenceSince" => arg(&self.read(|r| r.presence_since(&s(0)))),
            "isPublic" => Arg::Bool(self.read(|r| r.is_public(&s(0)))),
            "can" => Arg::Bool(self.read(|r| r.can(&s(0), &json(&x(1))))),
            "enabled" => Arg::Bool(self.read(|r| r.enabled(&s(0)))),
            "installed" => self.read(|r| r.installed(&s(0))).map_or(Arg::Undef, Arg::Bool),
            "isRegistered" => Arg::Bool(self.read(|r| r.is_registered(&s(0)))),
            "onlyObserved" => Arg::Bool(self.read(|r| r.only_observed(&s(0)))),
            "targetsOf" => Arg::List(self.read(|r| r.targets_of(&s(0))).iter().map(arg).collect()),
            "pidsOf" => Arg::List(self.read(|r| r.pids_of(&s(0))).into_iter().map(Arg::Num).collect()),
            "info" => arg(&self.read(|r| r.info(&s(0)))),
            "registeredCount" => Arg::Num(self.read(|r| r.registered_count()) as f64),
            "ofProduct" => Arg::List(self.read(|r| r.of_product(crate::js::opt_json(&x(0)).as_ref())).into_iter().map(Arg::Text).collect()),
            "isRemote" => Arg::Bool(self.read(|r| r.is_remote(&s(0)))),
            "remoteOf" => arg(&self.read(|r| r.remote_of(&s(0)))),
            "simulationHeld" => Arg::Bool(self.read(|r| r.simulation_held(&s(0)))),
            "upsert" => {
                let def = x(0);
                self.with(w, |r, h| r.upsert(&def, h))?;
                Arg::Undef
            }
            "addVia" => {
                self.with(w, |r, _| r.add_via(&s(0), &s(1)))?;
                Arg::Undef
            }
            "dropVia" => {
                self.with(w, |r, _| r.drop_via(&s(0), &s(1)))?;
                Arg::Undef
            }
            "setRemote" => {
                let remote = json(&x(1));
                self.with(w, |r, _| r.set_remote(&s(0), &remote))?;
                Arg::Undef
            }
            "remove" => {
                self.remove(w, &s(0))?;
                Arg::Undef
            }
            "forgetYou" => {
                self.with(w, |r, _| r.forget_you())?;
                Arg::Undef
            }
            "unregister" => {
                let id = s(0);
                let keep = match x(1).field("keepRemote") {
                    Arg::Undef => true,
                    k => k.truthy(),
                };
                let revokes = self.read(|r| r.unregister_revokes(&id, keep)).map_err(thrown)?;
                if revokes {
                    let slug = self.slug_of(&id)?;
                    for f in self.before_remove.locked().clone() {
                        f(&slug);
                    }
                }
                if self.with(w, |r, _| r.unregister(&id, revokes))? {
                    self.remove(w, &id)?;
                }
                Arg::Undef
            }
            "allow" => {
                let at = now_or(x(1));
                self.with(w, |r, _| r.allow(&s(0), at))?;
                Arg::Undef
            }
            "setEnabled" => {
                let on = x(1).truthy();
                self.with(w, |r, _| r.set_enabled(&s(0), on))?;
                Arg::Undef
            }
            "setInstalled" => {
                let on = x(1).truthy();
                self.with(w, |r, _| r.set_installed(&s(0), on))?;
                Arg::Undef
            }
            "awaitSession" => {
                let o = x(1);
                let own: Vec<f64> = list(o.field("own")).iter().filter_map(Arg::as_f64).collect();
                let (app, place, at) = (json(&o.field("app")), json(&o.field("place")), now_or(o.field("at")));
                Arg::Bool(self.reg.locked().await_session(&s(0), app, place, own, at, now_ms()))
            }
            "expireWaiting" => {
                let at = now_or(x(0));
                let list = self.reg.locked().expire_waiting(at);
                waiting_arg(&list)
            }
            "dropWaitingAt" => {
                self.reg.locked().drop_waiting_at(crate::js::num(&x(0)).max(0.0) as usize);
                Arg::Undef
            }
            "watchPid" => {
                let pid = x(1);
                self.reg.locked().watch_pid(&s(0), &pid).map_err(thrown)?;
                Arg::Undef
            }
            "report" => {
                let o = match x(1) {
                    Arg::Undef => Arg::Obj(vec![]),
                    o => o,
                };
                self.with(w, |r, h| r.report(&s(0), &o, h))?;
                Arg::Undef
            }
            "record" => {
                let o = x(1);
                arg(&self.with(w, |r, h| r.record(&s(0), &o, h))?)
            }
            "where" => {
                let t = json(&x(0));
                arg(&self.with(w, |r, h| Ok(r.where_of(&t, h)))?)
            }
            "target" => {
                let at = now_or(x(3));
                self.with(w, |r, _| r.target(&s(0), &s(1), &s(2), at))?;
                Arg::Undef
            }
            "untarget" => {
                self.with(w, |r, _| r.untarget(&s(0), &s(1), &s(2)))?;
                Arg::Undef
            }
            "snapshot" => {
                let o = x(1);
                let view = match o.field("view") {
                    Arg::Undef => "public".to_string(),
                    v => text(&v),
                };
                let now = now_or(o.field("now"));
                arg(&self.with(w, |r, h| r.snapshot(&s(0), &view, Some(now), None, h))?)
            }
            "sessionsStarted" => {
                let now = now_or(x(0).field("now"));
                let (names, starts) = self.read(|r| r.sessions_started(now)).map_err(thrown)?;
                obj(vec![
                    ("names", Arg::List(names.into_iter().map(|(k, v)| Arg::List(vec![Arg::Text(k), arg(&v)])).collect())),
                    ("starts", Arg::List(starts.iter().map(arg).collect())),
                ])
            }
            "placeOf" => {
                let p = json(&x(0));
                arg(&self.with(w, |r, h| Ok(r.place_of(&p, h)))?)
            }
            "engineView" => {
                let o = x(1);
                let now = now_or(o.field("now"));
                let since = o.field("since").as_f64();
                arg(&self.with(w, |r, h| r.engine_view(&s(0), now, since, h))?)
            }
            "setStatus" => {
                let o = x(2);
                let (text_, via, at) = (x(1), o.field("via"), now_or(o.field("at")));
                match self.with(w, |r, _| r.set_status(&s(0), &text_, &via, at))? {
                    Ok(status) => obj(vec![("ok", Arg::Bool(true)), ("status", arg(&status))]),
                    Err(reason) => obj(vec![("ok", Arg::Bool(false)), ("reason", Arg::text(reason))]),
                }
            }
            "said" => {
                let m = x(0);
                let to: Vec<String> = list(m.field("to")).iter().map(text).collect();
                let ts = m.field("ts").as_f64();
                arg(&self.with(w, |r, h| r.said(&text(&m.field("from")), &to, ts, h))?)
            }
            "engineIds" => {
                let now = now_or(x(1));
                let since = crate::js::num(&x(0));
                Arg::List(self.read(|r| r.engine_ids(since, now)).into_iter().map(Arg::Text).collect())
            }
            "tokens" => Arg::List(self.read(|r| r.tokens(now_or(x(0)))).map_err(thrown)?.iter().map(arg).collect()),
            "list" => {
                let o = x(0);
                let view = match o.field("view") {
                    Arg::Undef => "public".to_string(),
                    v => text(&v),
                };
                let now = now_or(o.field("now"));
                arg(&self.with(w, |r, h| r.list(&view, Some(now), h))?)
            }
            "events" => {
                let o = x(0);
                let now = now_or(o.field("now"));
                arg(&self.read(|r| r.events(&o, now)).map_err(thrown)?)
            }
            "tick" => {
                let now = now_or(x(0));
                self.with(w, |r, h| r.tick(now, h))?;
                Arg::Undef
            }
            "prune" => {
                let now = now_or(x(0));
                self.with(w, |r, _| r.prune(now))?;
                Arg::Undef
            }
            "refresh" => {
                self.changed(w);
                Arg::Undef
            }
            "now" => Arg::Num(now_ms()),
            _ => return Err(no_method("actors", method)),
        })
    }

    /// The registry's clock, its prune, the registration's tick and the
    /// demo's scripts, while the service runs.
    pub fn start(&self, w: &World) {
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
    fn step(&self, w: &World) {
        let mut d = self.demo.locked();
        let Some(d) = d.as_mut() else { return };
        let _ = self.with(w, |r, h| d.step(r, h));
    }

    pub fn stop(&self, w: &World) {
        for t in self.timers.locked().drain(..) {
            t.abort();
        }
        let _ = self.with(w, |r, _| r.stop());
    }
}

/// The spawned reports waiting for their session, as `actors.waiting`
/// gives them.
fn waiting_arg(list: &[registry::Waiting]) -> Arg {
    Arg::List(
        list.iter()
            .map(|w| {
                obj(vec![
                    ("actor", Arg::text(&w.actor)),
                    ("app", arg(&w.app)),
                    ("place", arg(&w.place)),
                    ("own", Arg::List(w.own.iter().map(|p| Arg::Num(*p)).collect())),
                    ("at", Arg::Num(w.at)),
                    ("until", Arg::Num(w.until)),
                ])
            })
            .collect(),
    )
}

/// What `actors.<method>` is.
pub fn kind(method: &str) -> &'static str {
    match method {
        "" => "object",
        "demo" => "boolean",
        "store" | "map" => "object",
        "has" | "ids" | "presence" | "presenceSince" | "isPublic" | "can" | "enabled" | "installed" | "isRegistered" | "onlyObserved" | "targetsOf" | "pidsOf" | "info"
        | "registeredCount" | "ofProduct" | "isRemote" | "remoteOf" | "simulationHeld" | "upsert" | "addVia" | "dropVia" | "setRemote" | "remove" | "forgetYou" | "unregister"
        | "allow" | "setEnabled" | "setInstalled" | "awaitSession" | "watchPid" | "report" | "record" | "where" | "target" | "untarget" | "snapshot" | "sessionsStarted"
        | "placeOf" | "engineView" | "setStatus" | "said" | "engineIds" | "tokens" | "list" | "events" | "tick" | "prune" | "refresh" | "now" | "expireWaiting"
        | "dropWaitingAt" => "function",
        "waiting" => "object",
        _ => "undefined",
    }
}

/// The "actors" collaborator as a core sees it: the registry's calls, its
/// store, its map and whether it's the demo's.
pub fn collab_call(w: &World, path: &str, method: &str, args: &[Arg]) -> Result<Arg> {
    let Some(a) = &w.actors else { return Err(no_method(path, method)) };
    match path {
        "actors" => a.call(w, method, args),
        "actors.store" => crate::store::call(&a.store, method, args),
        "actors.map" => w.map.call(method, args),
        _ => Err(no_method(path, method)),
    }
}

pub fn collab_get(w: &World, path: &str) -> Arg {
    match (&w.actors, path) {
        (None, _) => Arg::Undef,
        (Some(a), "actors.demo") => Arg::Bool(a.is_demo),
        (Some(a), "actors.waiting") => waiting_arg(&a.reg.locked().waiting),
        _ => Arg::Undef,
    }
}

pub fn collab_kind(w: &World, path: &str) -> &'static str {
    if w.actors.is_none() {
        return if path == "actors" { "null" } else { "undefined" };
    }
    match path {
        "actors" => "object",
        p => match p.strip_prefix("actors.") {
            Some(rest) => match rest.split_once('.') {
                Some(("store", m)) => crate::store::kind(m),
                Some(("map", m)) => Map::kind(m),
                Some(_) => "undefined",
                None => kind(rest),
            },
            None => "undefined",
        },
    }
}

/// The actors, as the routes call them.
pub struct ActorsDriver(pub Arc<World>);

impl ActorsDriver {
    fn side(&self) -> R<&ActorSide> {
        self.0.actors.as_ref().ok_or_else(|| CoreError::pending("core/actors/index.js"))
    }

    fn call(&self, method: &str, args: &[Arg]) -> R<Value> {
        self.side()?.call(&self.0, method, args).map(|a| json(&a)).map_err(core_error)
    }
}

#[async_trait]
impl Actors for ActorsDriver {
    fn list(&self) -> R<Value> {
        self.call("list", &[])
    }
    fn snapshot(&self, id: &str, view: Option<&str>, counts: bool) -> R<Option<Value>> {
        let a = self.side()?;
        let view = view.unwrap_or("public").to_string();
        let none: Vec<(String, [f64; 3])> = Vec::new();
        let v = a.with(&self.0, |r, h| r.snapshot(id, &view, Some(now_ms()), (!counts).then_some(&none[..]), h)).map_err(core_error)?;
        Ok((!v.is_null()).then_some(v))
    }
    fn events(&self, since: Option<f64>, only: Option<&str>, view: Option<&str>, limit: f64) -> R<Value> {
        let o = obj(vec![
            ("since", since.map_or(Arg::Undef, Arg::Num)),
            ("limit", Arg::Num(limit)),
            ("view", view.map_or(Arg::Undef, Arg::text)),
            ("only", only.map_or(Arg::Null, Arg::text)),
        ]);
        self.call("events", &[o])
    }
    fn you_forget(&self) -> R<()> {
        self.side()?.with(&self.0, |r, _| r.you_forget()).map_err(core_error)
    }
    fn you_report(&self, body: &Value) -> R<Value> {
        let b = arg(body);
        self.side()?.with(&self.0, |r, h| r.you_report(&b, now_ms(), h)).map_err(core_error)
    }
    fn you_window(&self, state: &Value) {
        if let Ok(a) = self.side() {
            let s = text(&arg(state));
            let _ = a.with(&self.0, |r, h| r.you_window(&s, now_ms(), h));
        }
    }
    fn pids_of(&self, id: &str) -> R<Vec<u32>> {
        Ok(self.side()?.read(|r| r.pids_of(id)).into_iter().filter(|p| p.is_finite() && *p >= 0.0).map(|p| p as u32).collect())
    }
    fn tokens(&self, now: f64) -> R<Value> {
        self.call("tokens", &[Arg::Num(now)])
    }
    fn can(&self, id: &str, capability: &str) -> bool {
        self.side().is_ok_and(|a| a.read(|r| r.can(id, &json!(capability))))
    }
    fn info(&self, id: &str) -> Option<Value> {
        let v = self.side().ok()?.read(|r| r.info(id));
        (!v.is_null()).then_some(v)
    }
    fn said(&self, message: &Value) {
        let _ = self.call("said", &[arg(message)]);
    }
    fn refresh(&self) {
        if let Ok(a) = self.side() {
            a.changed(&self.0);
        }
    }
    fn target(&self, actor: &str, id: &str, relation: &str) -> R<()> {
        self.call("target", &[Arg::text(actor), Arg::text(id), Arg::text(relation)]).map(|_| ())
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
        self.side().map(|a| a.read(|r| r.ids())).unwrap_or_default()
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
    fn registry(&self, method: &str, args: Vec<Value>) -> R<Value> {
        let args: Vec<Arg> = args.iter().map(arg).collect();
        self.call(method, &args)
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
