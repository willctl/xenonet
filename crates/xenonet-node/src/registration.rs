//! xenonet-actors' registration for the twin of
//! core/actors/registration.js: what's asked, the tokens and the limits are
//! Rust's; the actors and the clock are called through the twin's deps.
//! An ActorError is thrown marked, for the twin to throw as its own class.

use crate::args::JsArg;
use crate::js::Json;
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::ptr;
use xenonet_actors::registration::{self, ActorError, Fail, Registration};
use xenonet_engine::sql::Arg;

#[napi(js_name = "registrationConstants")]
pub fn registration_constants() -> Json {
    Json(json!({
        "requestMs": registration::REQUEST_MS,
        "tokenUnusedMs": registration::TOKEN_UNUSED_MS,
        "registeredMax": registration::REGISTERED_MAX,
        "waitingMax": registration::WAITING_MAX,
        "requestsPerHour": registration::REQUESTS_PER_HOUR,
        "capabilities": registration::CAPABILITIES,
        "revoked": registration::REVOKED_REASONS.iter().map(|r| (r.to_string(), json!(registration::revoked_words(r)))).collect::<serde_json::Map<_, _>>(),
    }))
}

#[napi(js_name = "tokenExpired")]
pub fn token_expired(at: Option<f64>, t: f64) -> bool {
    registration::token_expired(at, t)
}

/// An ActorError thrown for the twin: an Error marked xenonetActor, with
/// its status, code and extra fields.
pub fn throw_actor(env: &Env, e: &ActorError) -> Error {
    let env_raw = env.raw();
    unsafe {
        let mut msg = ptr::null_mut();
        sys::napi_create_string_utf8(env_raw, e.message.as_ptr().cast(), e.message.len() as isize, &raw mut msg);
        let mut err = ptr::null_mut();
        sys::napi_create_error(env_raw, ptr::null_mut(), msg, &raw mut err);
        let set = |k: &std::ffi::CStr, v: &Value| {
            if let Ok(js) = crate::js::to_js(env_raw, v, 0) {
                sys::napi_set_named_property(env_raw, err, k.as_ptr(), js);
            }
        };
        set(c"xenonetActor", &json!(true));
        set(c"status", &xenonet_base::json::number(e.status));
        set(c"code", &json!(e.code));
        set(c"extra", &e.extra);
        sys::napi_throw(env_raw, err);
    }
    Error::new(Status::PendingException, e.message.clone())
}

pub fn fail(env: &Env, f: Fail) -> Error {
    match f {
        Fail::Actor(e) => {
            settle(env);
            throw_actor(env, &e)
        }
        Fail::Thrown(t) => rethrow(env, t),
    }
}

#[napi]
pub struct RegistrationCore {
    inner: RefCell<Registration>,
}

fn busy() -> Error {
    Error::new(Status::GenericFailure, "the registration was asked again while it was busy".to_string())
}

impl RegistrationCore {
    // A call that needs none of what's kept, so it may come back in
    // during another (observers asks observing).
    fn free<T>(&self, env: &Env, deps: &Object<'_>, f: impl FnOnce(&mut SyncCalls) -> registration::Result<T>) -> Result<T> {
        let mut calls = SyncCalls::new(env, deps);
        match f(&mut calls) {
            Ok(v) => Ok(v),
            Err(e) => Err(fail(env, e)),
        }
    }

    fn with<T>(&self, env: &Env, deps: &Object<'_>, f: impl FnOnce(&mut Registration, &mut SyncCalls) -> registration::Result<T>) -> Result<T> {
        let mut r = self.inner.try_borrow_mut().map_err(|_| busy())?;
        let mut calls = SyncCalls::new(env, deps);
        match f(&mut r, &mut calls) {
            Ok(v) => {
                settle(env);
                Ok(v)
            }
            Err(e) => Err(fail(env, e)),
        }
    }
}

#[napi]
impl RegistrationCore {
    #[napi(constructor)]
    pub fn new(store: &crate::store::EngineStoreCore, home: String) -> Self {
        RegistrationCore { inner: RefCell::new(Registration::new(store.inner.clone(), &home)) }
    }

    /// [name, what it says] for each event told since.
    #[napi(js_name = "takeEmits")]
    pub fn take_emits(&self) -> Result<Json> {
        let mut r = self.inner.try_borrow_mut().map_err(|_| busy())?;
        Ok(Json(Value::Array(r.take_emits().into_iter().map(|(n, v)| json!([n, v])).collect())))
    }

    #[napi]
    pub fn waiting(&self, env: Env, deps: Object<'_>, t: f64) -> Result<u32> {
        self.with(&env, &deps, |r, _| r.waiting(t).map(|n| n as u32))
    }
    #[napi]
    pub fn register(&self, env: Env, deps: Object<'_>, body: JsArg, checked: JsArg, chain: JsArg, cwd: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.register(c, &body.0, &checked.0, &chain.0, &cwd.0)).map(Json)
    }
    #[napi]
    pub fn requests(&self, env: Env, deps: Object<'_>) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.requests(c)).map(Json)
    }
    #[napi]
    pub fn decide(&self, env: Env, deps: Object<'_>, request: JsArg, decision: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.decide(c, &request.0, &decision.0)).map(Json)
    }
    #[napi]
    pub fn add(&self, env: Env, deps: Object<'_>, body: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.add(c, &body.0)).map(Json)
    }
    #[napi]
    pub fn rotate(&self, env: Env, deps: Object<'_>, slug: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.rotate(c, &slug.0)).map(Json)
    }
    #[napi]
    pub fn remove(&self, env: Env, deps: Object<'_>, slug: JsArg, reason: String) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.remove(c, &slug.0, &reason)).map(Json)
    }
    #[napi]
    pub fn authenticate(&self, env: Env, deps: Object<'_>, token: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.authenticate(c, &token.0)).map(Json)
    }
    #[napi]
    pub fn gateway(&self, env: Env, deps: Object<'_>, slug: JsArg, body: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.gateway(c, &slug.0, &body.0)).map(Json)
    }
    #[napi(js_name = "askGateway")]
    pub fn ask_gateway(&self, env: Env, deps: Object<'_>, slug: JsArg, checked: JsArg, chain: JsArg, cwd: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.ask_gateway(c, &slug.0, &checked.0, &chain.0, &cwd.0)).map(Json)
    }
    #[napi(js_name = "dropGateway")]
    pub fn drop_gateway(&self, env: Env, deps: Object<'_>, slug: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.drop_gateway(c, &slug.0)).map(Json)
    }
    #[napi(js_name = "authenticateGateway")]
    pub fn authenticate_gateway(&self, env: Env, deps: Object<'_>, token: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.authenticate_gateway(c, &token.0)).map(Json)
    }
    #[napi(js_name = "productOf")]
    pub fn product_of(&self, env: Env, deps: Object<'_>, auth: JsArg) -> Result<Json> {
        self.free(&env, &deps, |c| Registration::product_of(c, &auth.0)).map(Json)
    }
    #[napi]
    pub fn me(&self, env: Env, deps: Object<'_>, auth: JsArg, extra: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.me(c, &auth.0, &extra.0)).map(Json)
    }
    #[napi]
    pub fn tick(&self, env: Env, deps: Object<'_>, t: f64) -> Result<()> {
        self.with(&env, &deps, |r, _| r.tick(t))
    }
    #[napi]
    pub fn observers(&self, env: Env, deps: Object<'_>) -> Result<Json> {
        self.free(&env, &deps, |c| Registration::observers(c)).map(Json)
    }
    #[napi]
    pub fn observing(&self, env: Env, deps: Object<'_>, product: JsArg) -> Result<bool> {
        self.free(&env, &deps, |c| Registration::observing(c, &product.0))
    }
    #[napi]
    pub fn observe(&self, env: Env, deps: Object<'_>, product: JsArg, on: JsArg) -> Result<Json> {
        self.free(&env, &deps, |c| Registration::observe(c, &product.0, &on.0)).map(Json)
    }
    #[napi]
    pub fn simulate(&self, env: Env, deps: Object<'_>, kind: JsArg, a: JsArg) -> Result<Json> {
        self.with(&env, &deps, |r, c| r.simulate(c, &kind.0, &a.0)).map(Json)
    }
}

#[allow(dead_code)]
fn unused(_: Arg) {}
