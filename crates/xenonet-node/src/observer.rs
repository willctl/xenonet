//! xenonet-actors' observer base for the twin of core/actors/observer.js:
//! what it said, touched and recorded, and the minutes it gathered, are
//! Rust's; it calls the actors, its product, its clock and its own
//! observing(), look(), ok() and problem() through the observer itself.

use crate::args::JsArg;
use crate::js::Json;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::json;
use std::sync::Arc;
use xenonet_actors::observer::{self, Observer};
use xenonet_engine::sql::Arg;

#[napi(js_name = "observerConstants")]
pub fn observer_constants() -> Json {
    Json(json!({ "lookMs": observer::LOOK_MS, "keepaliveMs": observer::KEEPALIVE_MS }))
}

#[napi]
pub struct ObserverCore {
    inner: Arc<Observer>,
}

fn done<T>(env: Env, r: xenonet_engine::peer::Result<T>) -> Result<T> {
    let out = r.map_err(|t| rethrow(&env, t));
    settle(&env);
    out
}

fn s(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

#[napi]
impl ObserverCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        ObserverCore { inner: Arc::new(Observer::default()) }
    }

    #[napi]
    pub fn keep(&self, env: Env, me: Object<'_>, id: JsArg, o: JsArg) -> Result<()> {
        done(env, self.inner.keep(&mut SyncCalls::new(&env, &me), &s(&id.0), &o.0))
    }
    #[napi]
    pub fn say(&self, env: Env, me: Object<'_>, id: JsArg, o: JsArg) -> Result<bool> {
        done(env, self.inner.say(&mut SyncCalls::new(&env, &me), &s(&id.0), &o.0))
    }
    #[napi]
    pub fn happened(&self, env: Env, me: Object<'_>, id: JsArg, o: JsArg) -> Result<JsArg> {
        done(env, self.inner.happened(&mut SyncCalls::new(&env, &me), &s(&id.0), &o.0)).map(JsArg)
    }
    #[napi]
    pub fn gather(&self, id: JsArg, o: JsArg) {
        self.inner.gather(&s(&id.0), &o.0);
    }
    #[napi]
    pub fn flush(&self, env: Env, me: Object<'_>, now: JsArg) -> Result<()> {
        done(env, self.inner.flush(&mut SyncCalls::new(&env, &me), &now.0))
    }
    #[napi]
    pub fn touch(&self, env: Env, me: Object<'_>, id: JsArg, node: JsArg, relation: JsArg, at: JsArg) -> Result<()> {
        done(env, self.inner.touch(&mut SyncCalls::new(&env, &me), &s(&id.0), &node.0, &relation.0, &at.0))
    }
    #[napi(js_name = "mayLook")]
    pub fn may_look(&self, env: Env, me: Object<'_>) -> Result<bool> {
        done(env, Observer::may_look(&mut SyncCalls::new(&env, &me)))
    }
    /// One look, as a machine: the observer's look(at), then flush, ok or
    /// problem, through the observer.
    #[napi]
    pub fn tick(&self, at: f64) -> Machine {
        let o = self.inner.clone();
        start(move |asker| o.tick(&mut AskCalls(asker), at).map(Arg::Bool))
    }
}
