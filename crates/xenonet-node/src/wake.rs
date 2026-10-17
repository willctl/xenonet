//! xenonet-actors' wakes for the twin of core/actors/wake.js: when and how
//! an actor is woken for its messages, over the engine store the twin's
//! store holds. Timers, sinks, the credential store, DNS and fetch are the
//! twin's, called through the JS objects it was given.

use crate::args::JsArg;
use crate::js::Json;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle, SyncCalls};
use crate::store::EngineStoreCore;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::json;
use std::ptr;
use std::sync::Arc;
use xenonet_actors::wake::{self, Fail, Wakes};
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::Arg;

/// A WakeError thrown for the twin: an Error marked xenonetWake, with its
/// status and code.
fn throw(env: &Env, f: Fail) -> Error {
    match f {
        Fail::Thrown(t) => rethrow(env, t),
        Fail::Wake { status, code, message } => {
            settle(env);
            let raw = env.raw();
            unsafe {
                let mut msg = ptr::null_mut();
                sys::napi_create_string_utf8(raw, message.as_ptr().cast(), message.len() as isize, &mut msg);
                let mut err = ptr::null_mut();
                sys::napi_create_error(raw, ptr::null_mut(), msg, &mut err);
                for (k, v) in [(c"xenonetWake", json!(true)), (c"status", xenonet_base::json::number(status)), (c"code", json!(code))] {
                    if let Ok(js) = crate::js::to_js(raw, &v, 0) {
                        sys::napi_set_named_property(raw, err, k.as_ptr(), js);
                    }
                }
                sys::napi_throw(raw, err);
            }
            Error::new(Status::PendingException, message)
        }
    }
}

fn done<T>(env: &Env, r: wake::Result<T>) -> Result<T> {
    match r {
        Ok(v) => {
            settle(env);
            Ok(v)
        }
        Err(f) => Err(throw(env, f)),
    }
}

/// What a machine came to: { done } or { refused: { status, code,
/// message } }, which the twin throws as a WakeError.
fn outcome(r: wake::Result<Arg>) -> std::result::Result<Arg, Thrown> {
    match r {
        Ok(v) => Ok(Arg::Obj(vec![("done".into(), v)])),
        Err(Fail::Wake { status, code, message }) => Ok(Arg::Obj(vec![("refused".into(), Arg::Obj(vec![("status".into(), Arg::num(status)), ("code".into(), Arg::text(code)), ("message".into(), Arg::text(message))]))])),
        Err(Fail::Thrown(t)) => Err(t),
    }
}

#[napi(js_name = "wakeConstants")]
pub fn constants() -> Json {
    Json(json!({
        "kinds": wake::WAKE_KINDS,
        "coalesceMs": wake::COALESCE_MS,
        "minGapMs": wake::MIN_GAP_MS,
        "timeoutMs": wake::TIMEOUT_MS,
        "backoffMs": wake::BACKOFF_MS,
        "notifyAfterMs": wake::NOTIFY_AFTER_MS,
        "notifyEveryMs": wake::NOTIFY_EVERY_MS,
        "streamsPerActor": wake::STREAMS_PER_ACTOR,
        "perActor": { "rate": wake::PER_ACTOR_RATE, "burst": wake::PER_ACTOR_BURST },
        "unreadHoldMs": wake::UNREAD_HOLD_MS,
    }))
}

#[napi(js_name = "wakeTarget")]
pub fn target(slug: JsArg, part: JsArg) -> String {
    wake::wake_target(&xenonet_engine::store::js_text(&slug.0), &xenonet_engine::store::js_text(&part.0))
}

#[napi(js_name = "wakeLinkLocal")]
pub fn link_local(ip: JsArg) -> bool {
    wake::link_local(&xenonet_engine::store::js_text(&ip.0))
}

#[napi(js_name = "wakeCheckUrl")]
pub fn check_url(env: Env, deps: Object, raw: JsArg) -> Result<Json> {
    let out = wake::check_wake_url(&mut SyncCalls::new(&env, &deps), &raw.0);
    done(&env, out).map(|(url, host)| Json(json!({ "url": url, "host": host })))
}

#[napi(js_name = "wakeSignature")]
pub fn signature(secret: String, id: String, timestamp: String, body: String) -> String {
    wake::wake_signature(&secret, &id, &timestamp, &body)
}

#[napi(js_name = "wakeRequest")]
pub fn request(env: Env, key: String, actor: JsArg, unread: JsArg, version: JsArg, now: f64, id: Option<String>) -> Result<JsArg> {
    wake::wake_request(&key, &actor.0, &unread.0, &version.0, now, id).map(JsArg).map_err(|t| rethrow(&env, t))
}

#[napi]
pub struct WakesCore {
    inner: Arc<Wakes>,
}

#[napi]
impl WakesCore {
    #[napi(constructor)]
    pub fn new(store: &EngineStoreCore, version: JsArg, now: f64) -> Self {
        WakesCore { inner: Arc::new(Wakes::new(store.inner.clone(), version.0, now)) }
    }

    #[napi]
    pub fn arrived(&self, env: Env, deps: Object, id: JsArg) -> Result<()> {
        done(&env, self.inner.arrived(&mut SyncCalls::new(&env, &deps), &id.0))
    }
    #[napi]
    pub fn view(&self, env: Env, id: JsArg) -> Result<JsArg> {
        done(&env, self.inner.view(&id.0)).map(JsArg)
    }
    #[napi]
    pub fn list(&self, env: Env, deps: Object) -> Result<JsArg> {
        done(&env, self.inner.list(&mut SyncCalls::new(&env, &deps))).map(JsArg)
    }
    /// The set the sink joined.
    #[napi]
    pub fn subscribe(&self, env: Env, deps: Object, id: JsArg, sink: u32) -> Result<u32> {
        done(&env, self.inner.subscribe(&mut SyncCalls::new(&env, &deps), &id.0, u64::from(sink))).map(|n| n as u32)
    }
    #[napi]
    pub fn unsubscribe(&self, id: JsArg, set: u32, sink: u32) {
        self.inner.unsubscribe(&id.0, u64::from(set), u64::from(sink));
    }
    #[napi]
    pub fn stop(&self, env: Env, deps: Object) -> Result<()> {
        done(&env, self.inner.stop(&mut SyncCalls::new(&env, &deps)))
    }
    #[napi(js_name = "wakeDue")]
    pub fn wake_due(&self, id: JsArg) -> bool {
        self.inner.wake_due(&id.0)
    }
    #[napi(js_name = "notifyDue")]
    pub fn notify_due(&self, env: Env, deps: Object, id: JsArg) -> Result<()> {
        done(&env, self.inner.notify_due(&mut SyncCalls::new(&env, &deps), &id.0))
    }

    #[napi]
    pub fn fire(&self, id: JsArg) -> Machine {
        let w = self.inner.clone();
        start(move |asker| outcome(w.fire(&mut AskCalls(asker), &id.0).map(|()| Arg::Undef)))
    }
    #[napi]
    pub fn set(&self, slug: JsArg, ask: JsArg) -> Machine {
        let w = self.inner.clone();
        start(move |asker| outcome(w.set(&mut AskCalls(asker), &slug.0, &ask.0)))
    }
    #[napi]
    pub fn test(&self, slug: JsArg) -> Machine {
        let w = self.inner.clone();
        start(move |asker| outcome(w.test(&mut AskCalls(asker), &slug.0)))
    }
    #[napi]
    pub fn forget(&self, slug: JsArg) -> Machine {
        let w = self.inner.clone();
        start(move |asker| outcome(w.forget(&mut AskCalls(asker), &slug.0).map(|()| Arg::Undef)))
    }
}

