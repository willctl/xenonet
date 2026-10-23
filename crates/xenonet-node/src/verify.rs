//! xenonet-engine's verifier for the twin of core/engine/verify.js: whether
//! a request comes from the process it says, over the engine store's pins.
//! procs and the clock are called through the JS objects the twin was
//! given.

use crate::args::JsArg;
use crate::machine::{start, AskCalls, Machine};
use crate::store::EngineStoreCore;
use napi_derive::napi;
use std::sync::Arc;
use xenonet_engine::peer::list;
use xenonet_engine::sql::Arg;
use xenonet_engine::verify::{self, Verifier};

/// How many of the chain's processes an actor's request may call its own.
#[napi(js_name = "verifyOwnOf")]
pub fn own_of(chain: JsArg) -> u32 {
    verify::own_of(&list(chain.0)).len() as u32
}

#[napi(js_name = "verifyActorRefusal")]
pub fn actor_refusal(chain: JsArg, product: JsArg, pid: JsArg, registering: bool) -> Option<String> {
    verify::actor_refusal(&list(chain.0), &product.0, &pid.0, registering)
}

#[napi(js_name = "verifyBundleOf")]
pub fn bundle_of(program: JsArg) -> String {
    verify::bundle_of(&program.0)
}

/// The tray app's bundle, as it's installed.
#[napi(js_name = "verifyTrayApp")]
pub fn tray_app() -> String {
    verify::TRAY_APP.to_string()
}

#[napi]
pub struct VerifierCore {
    inner: Arc<Verifier>,
}

#[napi]
impl VerifierCore {
    #[napi(constructor)]
    pub fn new(store: &EngineStoreCore, home: JsArg) -> Self {
        VerifierCore { inner: Arc::new(Verifier::new(store.inner.clone(), home.0)) }
    }

    /// { checked, refused, unchecked, unseen, lastRefusal }.
    #[napi]
    pub fn state(&self) -> JsArg {
        let c = self.inner.counts();
        JsArg(Arg::Obj(vec![
            ("checked".into(), Arg::num(c.checked)),
            ("refused".into(), Arg::num(c.refused)),
            ("unchecked".into(), Arg::num(c.unchecked)),
            ("unseen".into(), Arg::num(c.unseen)),
            ("lastRefusal".into(), c.last_refusal.unwrap_or(Arg::Null)),
        ]))
    }

    #[napi]
    pub fn check(&self, agent: JsArg, hook: bool) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.check(&mut AskCalls(asker), &agent.0, hook))
    }
    #[napi]
    pub fn person(&self) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.person(&mut AskCalls(asker)))
    }
    #[napi]
    pub fn window(&self) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.window(&mut AskCalls(asker)))
    }
    #[napi]
    pub fn desktop(&self, connected: bool) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.desktop(&mut AskCalls(asker), connected))
    }
    #[napi]
    pub fn tray(&self, apps: Vec<String>) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.tray(&mut AskCalls(asker), &apps))
    }
    #[napi]
    pub fn actor(&self, who: JsArg, pid: JsArg) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.actor(&mut AskCalls(asker), &who.0, &pid.0))
    }
    #[napi]
    pub fn register(&self, product: JsArg, pid: JsArg) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.register(&mut AskCalls(asker), &product.0, &pid.0))
    }
    #[napi]
    pub fn asker(&self) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.asker(&mut AskCalls(asker)))
    }
    #[napi]
    pub fn peer(&self) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.peer(&mut AskCalls(asker)))
    }
    #[napi]
    pub fn lineage(&self, pid: JsArg) -> Machine {
        let v = self.inner.clone();
        start(move |asker| v.lineage(&mut AskCalls(asker), &pid.0))
    }
}
