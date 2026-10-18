//! Registering actors (core/actors/registration.js over
//! crates/xenonet-actors/src/registration.rs): their requests, tokens, the
//! gateway's asks and the limits. The actors and the clock are called as
//! the JS called them; what it tells (requests, asked, asked-gateway) goes
//! to the page once the call is done.

use crate::actors::{collab_call, collab_get, collab_kind};
use crate::js::{arg, at, json, no_method, now_ms, obj, Locked};
use crate::world::World;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_actors::registration::{Fail, Registration as Core};
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;
use xenonet_service::cores::{CoreError, Kind, Registration, R};
use xenonet_service::glue::From;

/// The registration's collaborators: the actors, the clock, and its own
/// observing().
struct RegCalls<'a> {
    w: &'a World,
}

impl Calls for RegCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("", "now") => Ok(Arg::Num(now_ms())),
            ("", "observing") => Core::observing(self, &at(&args, 0)).map(Arg::Bool).map_err(fail_thrown),
            (p, m) if p == "actors" || p.starts_with("actors.") => collab_call(self.w, p, m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(collab_get(self.w, path))
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "observing" | "now" => "function",
            p => collab_kind(self.w, p),
        }
        .into())
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

fn fail_thrown(f: Fail) -> xenonet_engine::peer::Thrown {
    match f {
        Fail::Actor(e) => xenonet_engine::peer::Thrown::coded(e.message, e.code),
        Fail::Thrown(t) => t,
    }
}

/// An ActorError as the routes answer it: its status, code and extra
/// fields; anything else an Error.
pub fn actor_error(f: Fail) -> CoreError {
    match f {
        Fail::Actor(e) => CoreError::of(Kind::Actor, e.status as u16, e.code, e.message).with_extra(e.extra),
        Fail::Thrown(t) => crate::actors::core_error(t),
    }
}

pub struct RegSide {
    core: Mutex<Core>,
}

impl RegSide {
    pub fn new(store: Shared, home: &str) -> RegSide {
        RegSide { core: Mutex::new(Core::new(store, home)) }
    }

    /// A call that holds what's kept, with what it told told once it's done.
    fn with<T>(&self, w: &World, f: impl FnOnce(&mut Core, &mut RegCalls) -> std::result::Result<T, Fail>) -> std::result::Result<T, Fail> {
        let (out, emits) = {
            let mut core = self.core.locked();
            let out = f(&mut core, &mut RegCalls { w });
            (out, core.take_emits())
        };
        for (name, v) in emits {
            w.emit(From::Registration, name, v);
        }
        out
    }

    /// A call that needs none of what's kept (observers asks observing
    /// while another runs).
    fn free<T>(&self, w: &World, f: impl FnOnce(&mut RegCalls) -> std::result::Result<T, Fail>) -> std::result::Result<T, Fail> {
        f(&mut RegCalls { w })
    }

    pub fn tick(&self, w: &World) {
        let _ = self.with(w, |r, _| r.tick(now_ms()));
    }

    /// Whether the observer of a product is on, as the observers ask.
    pub fn observing(&self, w: &World, product: &str) -> bool {
        self.free(w, |c| Core::observing(c, &Arg::text(product))).unwrap_or(true)
    }
}

/// `{ checked = false, chain = null, cwd = null }` of a verdict.
fn verdict_parts(v: &Value) -> (Arg, Arg, Arg) {
    let a = arg(v);
    let or = |x: Arg, d: Arg| if matches!(x, Arg::Undef) { d } else { x };
    (or(a.field("checked"), Arg::Bool(false)), or(a.field("chain"), Arg::Null), or(a.field("cwd"), Arg::Null))
}

pub struct RegistrationDriver(pub Arc<World>);

impl RegistrationDriver {
    fn side(&self) -> R<&RegSide> {
        self.0.registration.as_ref().ok_or_else(|| CoreError::pending("core/actors/registration.js"))
    }

    fn run(&self, f: impl FnOnce(&mut Core, &mut RegCalls) -> std::result::Result<Value, Fail>) -> R<Value> {
        self.side()?.with(&self.0, f).map_err(actor_error)
    }
}

/// What authenticate gives for a call that failed: a refusal, never a pass.
fn refusal(f: Fail) -> Value {
    match f {
        Fail::Actor(e) => json!({ "ok": false, "status": crate::js::number(e.status), "code": e.code, "error": e.message }),
        Fail::Thrown(t) => json!({ "ok": false, "status": 500, "code": "error", "error": t.message }),
    }
}

impl Registration for RegistrationDriver {
    fn authenticate(&self, token: &str) -> Value {
        let t = Arg::text(token);
        match self.side() {
            Ok(s) => s.with(&self.0, |r, c| r.authenticate(c, &t)).unwrap_or_else(refusal),
            Err(e) => json!({ "ok": false, "code": "pending", "error": e.message }),
        }
    }
    fn authenticate_gateway(&self, token: &str) -> Value {
        let t = Arg::text(token);
        match self.side() {
            Ok(s) => s.with(&self.0, |r, c| r.authenticate_gateway(c, &t)).unwrap_or_else(refusal),
            Err(e) => json!({ "ok": false, "status": 501, "code": "pending", "error": e.message }),
        }
    }
    fn product_of(&self, auth: &Value) -> Value {
        let a = arg(auth);
        match self.side() {
            Ok(s) => s.free(&self.0, |c| Core::product_of(c, &a)).unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    }
    fn register(&self, body: &Value, verdict: &Value) -> R<Value> {
        let b = arg(body);
        let (checked, chain, cwd) = verdict_parts(verdict);
        self.run(|r, c| r.register(c, &b, &checked, &chain, &cwd))
    }
    fn me(&self, auth: &Value, extra: Option<Value>) -> R<Value> {
        let (a, e) = (arg(auth), extra.as_ref().map_or(Arg::Obj(vec![]), arg));
        self.run(|r, c| r.me(c, &a, &e))
    }
    fn remove(&self, slug: &str, reason: Option<&str>) -> R<Value> {
        let s = Arg::text(slug);
        let reason = reason.unwrap_or("removed").to_string();
        self.run(|r, c| r.remove(c, &s, &reason))
    }
    fn add(&self, body: &Value) -> R<Value> {
        let b = arg(body);
        self.run(|r, c| r.add(c, &b))
    }
    fn decide(&self, request: &str, decision: &Value) -> R<Value> {
        let (q, d) = (Arg::text(request), arg(decision));
        self.run(|r, c| r.decide(c, &q, &d))
    }
    fn rotate(&self, slug: &str) -> R<Value> {
        let s = Arg::text(slug);
        self.run(|r, c| r.rotate(c, &s))
    }
    fn drop_gateway(&self, slug: &str) -> R<Value> {
        let s = Arg::text(slug);
        self.run(|r, c| r.drop_gateway(c, &s))
    }
    fn gateway(&self, slug: &str, capabilities: Option<Value>) -> R<Value> {
        let s = Arg::text(slug);
        let body = match capabilities {
            Some(c) => obj(vec![("capabilities", arg(&c))]),
            None => Arg::Obj(vec![]),
        };
        self.run(|r, c| r.gateway(c, &s, &body))
    }
    fn ask_gateway(&self, slug: &str, verdict: &Value) -> R<Value> {
        let s = Arg::text(slug);
        let (checked, chain, cwd) = verdict_parts(verdict);
        self.run(|r, c| r.ask_gateway(c, &s, &checked, &chain, &cwd))
    }
    fn requests(&self) -> R<Value> {
        self.run(|r, c| r.requests(c))
    }
    fn observers(&self) -> R<Value> {
        self.side()?.free(&self.0, |c| Core::observers(c)).map_err(actor_error)
    }
    fn observe(&self, product: &Value, on: &Value) -> R<Value> {
        let (p, o) = (arg(product), arg(on));
        self.side()?.free(&self.0, |c| Core::observe(c, &p, &o)).map_err(actor_error)
    }
    fn waiting(&self) -> usize {
        self.side().ok().and_then(|s| s.with(&self.0, |r, _| r.waiting(now_ms())).ok()).unwrap_or(0)
    }
    fn simulate(&self, kind: &str, a: Option<String>) -> R<Option<Value>> {
        let (k, a) = (Arg::text(kind), a.map_or(Arg::Undef, Arg::Text));
        let v = self.run(|r, c| r.simulate(c, &k, &a))?;
        Ok((!v.is_null()).then_some(v))
    }
    fn tick(&self) {
        if let Ok(s) = self.side() {
            s.tick(&self.0);
        }
    }
}

/// A Value's JSON, for the routes.
pub fn as_json(a: &Arg) -> Value {
    json(a)
}
