//! xenonet-actors' app bundles and the Muse and Grok Bot observers, for
//! their twins: what's kept and decided is Rust's; the files, plutil, the
//! census and the actors are called through the twin objects themselves.

use crate::args::JsArg;
use crate::js::Json;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_actors::app_bundle::{self, AppBundle};
use xenonet_actors::grok_bot::{self, GrokBot};
use xenonet_actors::muse::Muse;
use xenonet_engine::peer::Calls;
use xenonet_engine::sql::Arg;

fn done<T>(env: &Env, r: xenonet_engine::peer::Result<T>) -> Result<T> {
    let out = r.map_err(|t| rethrow(env, t));
    settle(env);
    out
}

fn home_of(c: &mut dyn Calls) -> String {
    match c.get("home") {
        Ok(Arg::Text(h)) => h,
        _ => String::new(),
    }
}

#[napi(js_name = "observerProducts")]
pub fn observer_products() -> Json {
    Json(json!({ "museBundleId": r"^com\.meta\.endo(\.|$)", "grokBundleId": r"^com\.anysphere\.sand(\.|$)", "grokPollMs": grok_bot::POLL_MS }))
}

#[napi(js_name = "plistString")]
pub fn plist_string(text: String, key: String) -> Option<String> {
    app_bundle::plist_string(&text, &key)
}

#[napi]
pub struct AppBundleCore {
    inner: Arc<AppBundle>,
}

#[napi]
impl AppBundleCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        AppBundleCore { inner: Arc::new(AppBundle::default()) }
    }
    #[napi]
    pub fn plist(&self) -> Json {
        Json(self.inner.plist().map_or(Value::Null, |p| p.json()))
    }
    #[napi(js_name = "where")]
    pub fn where_(&self, env: Env, me: Object) -> Result<Option<String>> {
        done(&env, self.inner.where_(&mut SyncCalls::new(&env, &me)))
    }
    #[napi]
    pub fn installed(&self, env: Env, me: Object) -> Result<bool> {
        done(&env, self.inner.installed(&mut SyncCalls::new(&env, &me)))
    }
    #[napi]
    pub fn ours(&self, env: Env, me: Object) -> Result<bool> {
        done(&env, self.inner.ours(&mut SyncCalls::new(&env, &me)))
    }
    #[napi]
    pub fn read(&self) -> Machine {
        let a = self.inner.clone();
        start(move |asker| a.read(&mut AskCalls(asker)).map(|p| crate::args::value_to_arg(&p.json())))
    }
    #[napi]
    pub fn sources(&self, env: Env, me: Object, home: JsArg) -> Result<Json> {
        let home = match home.0 {
            Arg::Text(h) => h,
            _ => String::new(),
        };
        done(&env, self.inner.sources(&mut SyncCalls::new(&env, &me), &home)).map(Json)
    }
}

#[napi]
pub struct MuseCore {
    inner: Arc<Muse>,
}

#[napi]
impl MuseCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        MuseCore { inner: Arc::new(Muse::default()) }
    }
    #[napi]
    pub fn look(&self, at: f64) -> Machine {
        let m = self.inner.clone();
        start(move |asker| m.look(&mut AskCalls(asker), at).map(|_| Arg::Undef))
    }
    #[napi]
    pub fn status(&self, env: Env, me: Object) -> Result<Json> {
        let mut c = SyncCalls::new(&env, &me);
        let home = home_of(&mut c);
        done(&env, self.inner.status(&mut c, &home)).map(Json)
    }
}

#[napi]
pub struct GrokBotCore {
    inner: Arc<GrokBot>,
}

#[napi]
impl GrokBotCore {
    #[napi(constructor)]
    pub fn new(spawns: &crate::attribution::SpawnsCore) -> Self {
        GrokBotCore { inner: Arc::new(GrokBot::new(spawns.state.clone())) }
    }
    #[napi]
    pub fn look(&self, at: f64) -> Machine {
        let g = self.inner.clone();
        start(move |asker| g.look(&mut AskCalls(asker), at).map(|_| Arg::Undef))
    }
    /// [[pid, entry]] tracked.
    #[napi]
    pub fn tracked(&self) -> Json {
        Json(Value::Array(self.inner.tracked().0))
    }
    #[napi(js_name = "appPids")]
    pub fn app_pids(&self) -> Vec<f64> {
        self.inner.tracked().1
    }
    #[napi(js_name = "mayPoll")]
    pub fn may_poll(&self, env: Env, me: Object) -> Result<bool> {
        done(&env, self.inner.may_poll(&mut SyncCalls::new(&env, &me)))
    }
    #[napi]
    pub fn poll(&self, at: f64) -> Machine {
        let g = self.inner.clone();
        start(move |asker| g.poll(&mut AskCalls(asker), at).map(|_| Arg::Undef))
    }
    #[napi]
    pub fn status(&self, env: Env, me: Object) -> Result<Json> {
        let mut c = SyncCalls::new(&env, &me);
        let home = home_of(&mut c);
        done(&env, self.inner.status(&mut c, &home)).map(Json)
    }
}
