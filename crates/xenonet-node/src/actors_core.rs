//! xenonet-actors' registry for the twin of core/actors/index.js (with
//! you.js and demo.js): the actors, their presence, events and targets are
//! Rust's. The twin hands each call its host, a JS function that answers
//! what the registry asks of the world (the clock, the map, whether a
//! process runs) and hears what it tells (an event, the list changed),
//! which is told once the call is done.

use crate::args::{throw_sql, JsArg};
use crate::js::{Json, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::cell::RefCell;
use xenonet_actors::demo::{self, Demo, Resolve};
use xenonet_actors::registry::{self, Actors, Fail, Host};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;

pub type HostFn<'a> = Function<'a, FnArgs<(String, JsArg)>, JsArg>;

/// The twin's host, through its function; the first error it threw is
/// kept, and thrown once the call is done.
pub struct JsHost<'a, 'f> {
    pub f: &'a HostFn<'f>,
    pub thrown: Option<Error>,
}

impl JsHost<'_, '_> {
    fn ask(&mut self, op: &str, arg: Arg) -> Arg {
        if self.thrown.is_some() {
            return Arg::Undef;
        }
        match self.f.call(FnArgs::from((op.to_string(), JsArg(arg)))) {
            Ok(v) => v.0,
            Err(e) => {
                self.thrown = Some(e);
                Arg::Undef
            }
        }
    }
}

impl Host for JsHost<'_, '_> {
    fn now(&mut self) -> f64 {
        self.ask("now", Arg::Undef).as_f64().unwrap_or(f64::NAN)
    }
    fn describe(&mut self, id: &str) -> Value {
        self.ask("describe", Arg::text(id)).to_json()
    }
    fn has_map(&mut self) -> bool {
        self.ask("hasMap", Arg::Undef).truthy()
    }
    fn has(&mut self, id: &str) -> Option<bool> {
        match self.ask("has", Arg::text(id)) {
            Arg::Undef => None,
            v => Some(v.truthy()),
        }
    }
    fn ready(&mut self) -> Option<bool> {
        match self.ask("ready", Arg::Undef) {
            Arg::Bool(false) => Some(false),
            _ => None,
        }
    }
    fn place_of(&mut self, p: &str) -> Value {
        self.ask("placeOf", Arg::text(p)).to_json()
    }
    fn recent_session(&mut self) -> Value {
        self.ask("recentSession", Arg::Undef).to_json()
    }
    fn alive(&mut self, pid: f64) -> bool {
        self.ask("alive", Arg::Num(pid)).truthy()
    }
}

impl Resolve for JsHost<'_, '_> {
    fn resolve(&mut self, kind: &str, what: &str) -> Value {
        self.ask("resolve", Arg::List(vec![Arg::text(kind), Arg::text(what)])).to_json()
    }
}

#[napi]
pub struct ActorsCore {
    inner: RefCell<Actors>,
    demo: RefCell<Option<Demo>>,
}

fn fail(env: Env, f: Fail) -> Error {
    match f {
        Fail::Error(m) => Error::new(Status::GenericFailure, m),
        Fail::Sql(e) => throw_sql(&env, &e),
    }
}

fn busy() -> Error {
    Error::new(Status::GenericFailure, "the actors were asked again while they were busy".to_string())
}

fn store_of(store: &crate::store::EngineStoreCore) -> Shared {
    store.inner.clone()
}

impl ActorsCore {
    // A call on the registry: its answer, then what it told, told.
    fn call<T>(&self, env: Env, host: &HostFn<'_>, f: impl FnOnce(&mut Actors, &mut JsHost<'_, '_>) -> registry::Result<T>) -> Result<T> {
        let mut h = JsHost { f: host, thrown: None };
        let (out, emits, changed) = {
            let mut a = self.inner.try_borrow_mut().map_err(|_| busy())?;
            let out = f(&mut a, &mut h);
            (out, a.take_emits(), a.take_changed())
        };
        if let Some(e) = h.thrown.take() {
            return Err(e);
        }
        let out = out.map_err(|e| fail(env, e))?;
        for (name, v) in emits {
            host.call(FnArgs::from(("emit".to_string(), JsArg(Arg::List(vec![Arg::text(name), registry::value_arg(&v)])))))?;
        }
        if changed {
            host.call(FnArgs::from(("changed".to_string(), JsArg(Arg::Undef))))?;
        }
        Ok(out)
    }

    fn read<T>(&self, f: impl FnOnce(&Actors) -> T) -> Result<T> {
        Ok(f(&*self.inner.try_borrow().map_err(|_| busy())?))
    }
}

fn s(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

#[napi]
impl ActorsCore {
    #[napi(constructor)]
    pub fn new(env: Env, store: &crate::store::EngineStoreCore, demo: bool, host: HostFn<'_>) -> Result<Self> {
        let mut h = JsHost { f: &host, thrown: None };
        let a = Actors::new(store_of(store), demo, &mut h);
        if let Some(e) = h.thrown.take() {
            return Err(e);
        }
        Ok(ActorsCore { inner: RefCell::new(a.map_err(|e| fail(env, e))?), demo: RefCell::new(None) })
    }

    #[napi]
    pub fn has(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.has(&s(&id.0)))
    }
    #[napi]
    pub fn ids(&self) -> Result<Vec<String>> {
        self.read(Actors::ids)
    }
    #[napi]
    pub fn presence(&self, id: JsArg) -> Result<Option<String>> {
        self.read(|a| a.presence(&s(&id.0)))
    }
    #[napi(js_name = "presenceSince")]
    pub fn presence_since(&self, id: JsArg) -> Result<Json> {
        self.read(|a| Json(a.presence_since(&s(&id.0))))
    }
    #[napi(js_name = "isPublic")]
    pub fn is_public(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.is_public(&s(&id.0)))
    }
    #[napi]
    pub fn can(&self, id: JsArg, capability: JsArg) -> Result<bool> {
        self.read(|a| a.can(&s(&id.0), &capability.0.to_json()))
    }
    #[napi]
    pub fn enabled(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.enabled(&s(&id.0)))
    }
    /// null for unsaid; the twin gives undefined.
    #[napi]
    pub fn installed(&self, id: JsArg) -> Result<Option<bool>> {
        self.read(|a| a.installed(&s(&id.0)))
    }
    #[napi(js_name = "isRegistered")]
    pub fn is_registered(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.is_registered(&s(&id.0)))
    }
    #[napi(js_name = "onlyObserved")]
    pub fn only_observed(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.only_observed(&s(&id.0)))
    }
    #[napi(js_name = "targetsOf")]
    pub fn targets_of(&self, id: JsArg) -> Result<Json> {
        self.read(|a| Json(Value::Array(a.targets_of(&s(&id.0)))))
    }
    #[napi(js_name = "pidsOf")]
    pub fn pids_of(&self, id: JsArg) -> Result<Vec<f64>> {
        self.read(|a| a.pids_of(&s(&id.0)))
    }
    #[napi]
    pub fn info(&self, id: JsArg) -> Result<Json> {
        self.read(|a| Json(a.info(&s(&id.0))))
    }
    #[napi(js_name = "registeredCount")]
    pub fn registered_count(&self) -> Result<u32> {
        self.read(|a| a.registered_count() as u32)
    }
    #[napi(js_name = "ofProduct")]
    pub fn of_product(&self, product: MaybeJson) -> Result<Vec<String>> {
        self.read(|a| a.of_product(product.0.as_ref()))
    }
    #[napi(js_name = "isRemote")]
    pub fn is_remote(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.is_remote(&s(&id.0)))
    }
    #[napi(js_name = "remoteOf")]
    pub fn remote_of(&self, id: JsArg) -> Result<Json> {
        self.read(|a| Json(a.remote_of(&s(&id.0))))
    }
    /// What an actor's presence comes to at `now`, by the rules.
    #[napi(js_name = "presenceAt")]
    pub fn presence_at(&self, id: JsArg, now: f64) -> Result<Option<String>> {
        self.read(|a| a.record_of(&s(&id.0)).map(|r| Actors::presence_of(r, now)))
    }
    #[napi(js_name = "simulationHeld")]
    pub fn simulation_held(&self, id: JsArg) -> Result<bool> {
        self.read(|a| a.simulation_held(&s(&id.0)))
    }
    /// The records, as `actors.actors` shows them.
    #[napi]
    pub fn records(&self) -> Result<Json> {
        self.read(|a| Json(a.records()))
    }
    /// The spawned reports waiting for their session.
    #[napi]
    pub fn waiting(&self) -> Result<Json> {
        self.read(|a| {
            Json(Value::Array(
                a.waiting.iter().map(|w| json!({ "actor": w.actor, "app": w.app, "place": w.place, "own": w.own.iter().map(|p| xenonet_base::json::number(*p)).collect::<Vec<_>>(), "at": xenonet_base::json::number(w.at), "until": xenonet_base::json::number(w.until) })).collect(),
            ))
        })
    }
    #[napi(js_name = "dropWaitingAt")]
    pub fn drop_waiting_at(&self, index: u32) -> Result<()> {
        self.inner.try_borrow_mut().map_err(|_| busy())?.drop_waiting_at(index as usize);
        Ok(())
    }
    /// The spawned reports that still wait at `at`.
    #[napi(js_name = "expireWaiting")]
    pub fn expire_waiting(&self, at: f64) -> Result<()> {
        self.inner.try_borrow_mut().map_err(|_| busy())?.expire_waiting(at);
        Ok(())
    }

    #[napi]
    pub fn upsert(&self, env: Env, host: HostFn<'_>, def: JsArg) -> Result<()> {
        self.call(env, &host, |a, h| a.upsert(&def.0, h))
    }
    #[napi(js_name = "addVia")]
    pub fn add_via(&self, env: Env, host: HostFn<'_>, id: JsArg, v: JsArg) -> Result<()> {
        self.call(env, &host, |a, _| a.add_via(&s(&id.0), &s(&v.0)))
    }
    #[napi(js_name = "dropVia")]
    pub fn drop_via(&self, env: Env, host: HostFn<'_>, id: JsArg, v: JsArg) -> Result<()> {
        self.call(env, &host, |a, _| a.drop_via(&s(&id.0), &s(&v.0)))
    }
    #[napi(js_name = "setRemote")]
    pub fn set_remote(&self, env: Env, host: HostFn<'_>, id: JsArg, remote: JsArg) -> Result<()> {
        self.call(env, &host, |a, _| a.set_remote(&s(&id.0), &remote.0.to_json()))
    }
    #[napi(js_name = "slugOf")]
    pub fn slug_of(&self, env: Env, id: JsArg) -> Result<String> {
        self.read(|a| a.slug_of(&s(&id.0)))?.map_err(|e| fail(env, e))
    }
    #[napi]
    pub fn remove(&self, env: Env, host: HostFn<'_>, id: JsArg) -> Result<()> {
        self.call(env, &host, |a, _| a.remove(&s(&id.0)))
    }
    #[napi(js_name = "forgetYou")]
    pub fn forget_you(&self, env: Env, host: HostFn<'_>) -> Result<()> {
        self.call(env, &host, |a, _| a.forget_you())
    }
    #[napi(js_name = "unregisterRevokes")]
    pub fn unregister_revokes(&self, env: Env, id: JsArg, keep_remote: bool) -> Result<bool> {
        self.read(|a| a.unregister_revokes(&s(&id.0), keep_remote))?.map_err(|e| fail(env, e))
    }
    /// Whether it went for good (the twin then removes it).
    #[napi]
    pub fn unregister(&self, env: Env, host: HostFn<'_>, id: JsArg, revoked: bool) -> Result<bool> {
        self.call(env, &host, |a, _| a.unregister(&s(&id.0), revoked))
    }
    #[napi]
    pub fn allow(&self, env: Env, host: HostFn<'_>, id: JsArg, at: f64) -> Result<()> {
        self.call(env, &host, |a, _| a.allow(&s(&id.0), at))
    }
    #[napi(js_name = "setEnabled")]
    pub fn set_enabled(&self, env: Env, host: HostFn<'_>, id: JsArg, on: bool) -> Result<()> {
        self.call(env, &host, |a, _| a.set_enabled(&s(&id.0), on))
    }
    #[napi(js_name = "setInstalled")]
    pub fn set_installed(&self, env: Env, host: HostFn<'_>, id: JsArg, installed: bool) -> Result<()> {
        self.call(env, &host, |a, _| a.set_installed(&s(&id.0), installed))
    }
    #[napi(js_name = "awaitSession")]
    pub fn await_session(&self, id: JsArg, app: MaybeJson, place: MaybeJson, own: Vec<f64>, at: f64, now: f64) -> Result<bool> {
        let mut a = self.inner.try_borrow_mut().map_err(|_| busy())?;
        Ok(a.await_session(&s(&id.0), app.0.unwrap_or(Value::Null), place.0.unwrap_or(Value::Null), own, at, now))
    }
    #[napi(js_name = "watchPid")]
    pub fn watch_pid(&self, env: Env, id: JsArg, pid: JsArg) -> Result<()> {
        let mut a = self.inner.try_borrow_mut().map_err(|_| busy())?;
        a.watch_pid(&s(&id.0), &pid.0).map_err(|e| fail(env, e))
    }
    #[napi]
    pub fn report(&self, env: Env, host: HostFn<'_>, id: JsArg, o: JsArg) -> Result<()> {
        self.call(env, &host, |a, h| a.report(&s(&id.0), &o.0, h))
    }
    #[napi]
    pub fn record(&self, env: Env, host: HostFn<'_>, id: JsArg, o: JsArg) -> Result<Json> {
        self.call(env, &host, |a, h| a.record(&s(&id.0), &o.0, h)).map(Json)
    }
    #[napi(js_name = "whereOf")]
    pub fn where_of(&self, env: Env, host: HostFn<'_>, target: MaybeJson) -> Result<Json> {
        self.call(env, &host, |a, h| Ok(a.where_of(&target.0.unwrap_or(Value::Null), h))).map(Json)
    }
    #[napi]
    pub fn target(&self, env: Env, host: HostFn<'_>, id: JsArg, node: JsArg, relation: JsArg, at: f64) -> Result<()> {
        self.call(env, &host, |a, _| a.target(&s(&id.0), &s(&node.0), &s(&relation.0), at))
    }
    #[napi]
    pub fn untarget(&self, env: Env, host: HostFn<'_>, id: JsArg, node: JsArg, relation: JsArg) -> Result<()> {
        self.call(env, &host, |a, _| a.untarget(&s(&id.0), &s(&node.0), &s(&relation.0)))
    }
    #[napi]
    pub fn snapshot(&self, env: Env, host: HostFn<'_>, id: JsArg, view: String, now: f64) -> Result<Json> {
        self.call(env, &host, |a, h| a.snapshot(&s(&id.0), &view, Some(now), None, h)).map(Json)
    }
    /// {names: [[id, name]], starts}.
    #[napi(js_name = "sessionsStarted")]
    pub fn sessions_started(&self, env: Env, now: f64) -> Result<Json> {
        let (names, starts) = self.read(|a| a.sessions_started(now))?.map_err(|e| fail(env, e))?;
        Ok(Json(json!({ "names": names.into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(), "starts": starts })))
    }
    #[napi(js_name = "placeOf")]
    pub fn place_of(&self, env: Env, host: HostFn<'_>, p: MaybeJson) -> Result<Json> {
        self.call(env, &host, |a, h| Ok(a.place_of(&p.0.unwrap_or(Value::Null), h))).map(Json)
    }
    #[napi(js_name = "engineView")]
    pub fn engine_view(&self, env: Env, host: HostFn<'_>, id: JsArg, now: f64, since: Option<f64>) -> Result<Json> {
        self.call(env, &host, |a, h| a.engine_view(&s(&id.0), now, since, h)).map(Json)
    }
    /// {ok, status} or {ok: false, reason}.
    #[napi(js_name = "setStatus")]
    pub fn set_status(&self, env: Env, host: HostFn<'_>, id: JsArg, text: JsArg, via: JsArg, at: f64) -> Result<Json> {
        self.call(env, &host, |a, _| a.set_status(&s(&id.0), &text.0, &via.0, at)).map(|r| match r {
            Ok(status) => Json(json!({ "ok": true, "status": status })),
            Err(reason) => Json(json!({ "ok": false, "reason": reason })),
        })
    }
    #[napi]
    pub fn said(&self, env: Env, host: HostFn<'_>, from: JsArg, to: Vec<String>, ts: Option<f64>) -> Result<Json> {
        self.call(env, &host, |a, h| a.said(&s(&from.0), &to, ts, h)).map(Json)
    }
    #[napi(js_name = "engineIds")]
    pub fn engine_ids(&self, since: f64, now: f64) -> Result<Vec<String>> {
        self.read(|a| a.engine_ids(since, now))
    }
    #[napi]
    pub fn tokens(&self, env: Env, now: f64) -> Result<Json> {
        self.read(|a| a.tokens(now))?.map(|t| Json(Value::Array(t))).map_err(|e| fail(env, e))
    }
    #[napi]
    pub fn list(&self, env: Env, host: HostFn<'_>, view: String, now: f64) -> Result<Json> {
        self.call(env, &host, |a, h| a.list(&view, Some(now), h)).map(Json)
    }
    #[napi(js_name = "listIfChanged")]
    pub fn list_if_changed(&self, env: Env, host: HostFn<'_>) -> Result<Json> {
        self.call(env, &host, |a, h| a.list_if_changed(h)).map(Json)
    }
    #[napi]
    pub fn events(&self, env: Env, o: JsArg, now: f64) -> Result<Json> {
        self.read(|a| a.events(&o.0, now))?.map(Json).map_err(|e| fail(env, e))
    }
    #[napi]
    pub fn tick(&self, env: Env, host: HostFn<'_>, now: f64) -> Result<()> {
        self.call(env, &host, |a, h| a.tick(now, h))
    }
    #[napi]
    pub fn prune(&self, env: Env, host: HostFn<'_>, now: f64) -> Result<()> {
        self.call(env, &host, |a, _| a.prune(now))
    }
    #[napi]
    pub fn stop(&self, env: Env, host: HostFn<'_>) -> Result<()> {
        self.call(env, &host, |a, _| a.stop())
    }
    #[napi]
    pub fn simulate(&self, env: Env, host: HostFn<'_>, kind: JsArg, a: MaybeJson, b: MaybeJson) -> Result<Json> {
        self.call(env, &host, |x, h| x.simulate(&s(&kind.0), a.0.unwrap_or(Value::Null), b.0.unwrap_or(Value::Null), h)).map(Json)
    }

    // ----- you -----

    #[napi(js_name = "youReport")]
    pub fn you_report(&self, env: Env, host: HostFn<'_>, body: JsArg, at: f64) -> Result<Json> {
        self.call(env, &host, |a, h| a.you_report(&body.0, at, h)).map(Json)
    }
    #[napi(js_name = "youWindow")]
    pub fn you_window(&self, env: Env, host: HostFn<'_>, state: JsArg, at: f64) -> Result<()> {
        self.call(env, &host, |a, h| a.you_window(&s(&state.0), at, h))
    }
    #[napi(js_name = "youForget")]
    pub fn you_forget(&self, env: Env, host: HostFn<'_>) -> Result<()> {
        self.call(env, &host, |a, _| a.you_forget())
    }
    #[napi(js_name = "youOff")]
    pub fn you_off(&self) -> Result<bool> {
        self.read(|a| a.you.off)
    }
    #[napi(js_name = "youPresence")]
    pub fn you_presence(&self) -> Result<Option<String>> {
        self.read(|a| a.you.presence())
    }

    // ----- the demo -----

    /// The demo's actors added, with `history` ms of their scripts behind
    /// them; demoStep() plays the next turn.
    #[napi(js_name = "addDemo")]
    pub fn add_demo(&self, env: Env, host: HostFn<'_>, history: f64) -> Result<()> {
        let mut d = None;
        self.call(env, &host, |a, h| {
            d = Some(Demo::add(a, history, h)?);
            Ok(())
        })?;
        *self.demo.borrow_mut() = d;
        Ok(())
    }
    #[napi(js_name = "demoStep")]
    pub fn demo_step(&self, env: Env, host: HostFn<'_>) -> Result<Json> {
        let mut d = self.demo.try_borrow_mut().map_err(|_| busy())?;
        let Some(d) = d.as_mut() else { return Ok(Json(Value::Null)) };
        self.call(env, &host, |a, h| d.step(a, h)).map(Json)
    }
}

#[napi(js_name = "actorConstants")]
pub fn actor_constants() -> Json {
    Json(json!({
        "listThrottleMs": registry::LIST_THROTTLE_MS,
        "clockMs": registry::CLOCK_MS,
        "pruneMs": registry::PRUNE_MS,
        "remoteLeaseMs": registry::REMOTE_LEASE_MS,
        "youHeartbeatMs": xenonet_actors::you::YOU_HEARTBEAT_MS,
        "stepMs": demo::STEP_MS,
    }))
}

#[napi(js_name = "nodeKind")]
pub fn node_kind(id: MaybeJson) -> Option<&'static str> {
    let text = match id.0 {
        None | Some(Value::Null) => String::new(),
        Some(v) => xenonet_engine::js::template(Some(&v)),
    };
    registry::node_kind(&text)
}

/// The demo's actors and scripts, as data, their nodes resolved by the
/// host's "resolve".
#[napi(js_name = "demoActors")]
pub fn demo_actors(host: HostFn<'_>) -> Result<Json> {
    let mut h = JsHost { f: &host, thrown: None };
    let out = demo::demo_actors(&mut h);
    match h.thrown {
        Some(e) => Err(e),
        None => Ok(Json(out)),
    }
}

#[napi(js_name = "demoScripts")]
pub fn demo_scripts(host: HostFn<'_>) -> Result<Json> {
    let mut h = JsHost { f: &host, thrown: None };
    let out = demo::demo_scripts(&mut h);
    match h.thrown {
        Some(e) => Err(e),
        None => Ok(Json(out)),
    }
}
