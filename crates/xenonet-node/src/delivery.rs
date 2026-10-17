//! xenonet-engine's live delivery for the twins of core/delivery/: the
//! selectors, plan usage, and the policy's guards and what it keeps. The
//! policy's clock, limits and files are called through the twin.

use crate::args::JsArg;
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::cell::RefCell;
use std::ptr;
use xenonet_engine::delivery::policy::{self, Fail, Policy};
use xenonet_engine::delivery::selector::{self, Selector};
use xenonet_engine::delivery::usage;
use xenonet_engine::peer::{list, Thrown};
use xenonet_engine::sql::Arg;

// ----- selector -----

#[napi(js_name = "selParse")]
pub fn sel_parse(text: JsArg) -> JsArg {
    JsArg(selector::parse_selector(&text.0).map_or(Arg::Null, |s| s.to_arg()))
}

#[napi(js_name = "selIs")]
pub fn sel_is(text: JsArg) -> bool {
    selector::is_selector(&text.0)
}

#[napi(js_name = "selFormat")]
pub fn sel_format(sel: JsArg) -> String {
    selector::format_selector(Selector::of(&sel.0).filter(|_| !sel.0.field("terms").is_nullish()).as_ref())
}

#[napi(js_name = "selMatches")]
pub fn sel_matches(sel: JsArg, facts: JsArg) -> bool {
    selector::matches(Selector::of(&sel.0).as_ref(), &facts.0)
}

#[napi(js_name = "selWithin")]
pub fn sel_within(inner: JsArg, outer: JsArg) -> bool {
    match Selector::of(&inner.0) {
        Some(i) => selector::within(&i, Selector::of(&outer.0).as_ref()),
        None => false,
    }
}

#[napi(js_name = "planUsage")]
pub fn plan_usage(limits: JsArg, app: JsArg, now: f64) -> Option<f64> {
    usage::plan_usage(&limits.0, &app.0, now)
}

// ----- policy -----

#[napi(js_name = "policyLimits")]
pub fn policy_limits() -> crate::js::Json {
    crate::js::Json(policy::limits())
}

#[napi(js_name = "policyIsYou")]
pub fn policy_is_you(id: JsArg) -> bool {
    policy::is_you(&id.0)
}

#[napi(js_name = "policyRedact")]
pub fn policy_redact(text: String) -> JsArg {
    let (t, n) = policy::redact_secrets(&text);
    JsArg(Arg::Obj(vec![("text".into(), Arg::text(t)), ("redacted".into(), Arg::num(n as f64))]))
}

#[napi(js_name = "policySanitize")]
pub fn policy_sanitize(text: String, one_line: bool, max: f64) -> String {
    policy::sanitize(&text, one_line, max)
}

#[napi(js_name = "policyFramed")]
pub fn policy_framed(id: JsArg, from: JsArg, verified: JsArg, text: JsArg, user: bool) -> String {
    policy::framed(&id.0, &from.0, &verified.0, &text.0, user)
}

#[napi(js_name = "policyHashText")]
pub fn policy_hash_text(text: String) -> String {
    policy::hash_text(&text)
}

#[napi(js_name = "policyTextFor")]
pub fn policy_text_for(text: JsArg, sender: JsArg, sender_facts: JsArg, recipient_facts: JsArg) -> JsArg {
    let (t, n) = policy::text_for(&text.0, &sender.0, &sender_facts.0, &recipient_facts.0);
    JsArg(Arg::Obj(vec![("text".into(), t), ("redacted".into(), Arg::num(n as f64))]))
}

/// A refusal thrown as the JS threw it: an Error with its code and status
/// 429; an Error with its sentence; or what a collaborator threw.
fn throw(env: &Env, f: Fail) -> Error {
    match f {
        Fail::Thrown(t) => rethrow(env, t),
        Fail::Error(message) => rethrow(env, Thrown::new(message)),
        Fail::Refused { why, code } => {
            settle(env);
            let raw = env.raw();
            unsafe {
                let mut msg = ptr::null_mut();
                sys::napi_create_string_utf8(raw, why.as_ptr().cast(), why.len() as isize, &mut msg);
                let mut err = ptr::null_mut();
                sys::napi_create_error(raw, ptr::null_mut(), msg, &mut err);
                for (k, v) in [(c"code", serde_json::json!(code)), (c"status", serde_json::json!(429))] {
                    if let Ok(js) = crate::js::to_js(raw, &v, 0) {
                        sys::napi_set_named_property(raw, err, k.as_ptr(), js);
                    }
                }
                sys::napi_throw(raw, err);
            }
            Error::new(Status::PendingException, why)
        }
    }
}

#[napi]
pub struct PolicyCore {
    inner: RefCell<Policy>,
}

fn busy() -> Error {
    Error::new(Status::GenericFailure, "the delivery policy was asked again while it was busy".to_string())
}

fn opt(a: JsArg) -> Option<Arg> {
    (!matches!(a.0, Arg::Undef)).then_some(a.0)
}

impl PolicyCore {
    fn run<T>(&self, env: &Env, deps: &Object, f: impl FnOnce(&mut Policy, &mut SyncCalls) -> policy::Result<T>) -> Result<T> {
        let mut p = self.inner.try_borrow_mut().map_err(|_| busy())?;
        let mut c = SyncCalls::new(env, deps);
        match f(&mut p, &mut c) {
            Ok(v) => {
                settle(env);
                Ok(v)
            }
            Err(e) => Err(throw(env, e)),
        }
    }
}

#[napi]
impl PolicyCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        PolicyCore { inner: RefCell::new(Policy::default()) }
    }

    /// What the policy's file keeps, read.
    #[napi]
    pub fn load(&self, env: Env, deps: Object) -> Result<()> {
        self.run(&env, &deps, |p, c| {
            *p = Policy::load(c)?;
            Ok(())
        })
    }

    #[napi]
    pub fn data(&self) -> Result<JsArg> {
        Ok(JsArg(self.inner.try_borrow().map_err(|_| busy())?.data_arg()))
    }
    #[napi(js_name = "memoryAudit")]
    pub fn memory_audit(&self) -> Result<JsArg> {
        Ok(JsArg(Arg::List(self.inner.try_borrow().map_err(|_| busy())?.memory_audit.clone())))
    }
    #[napi]
    pub fn paused(&self) -> Result<bool> {
        Ok(self.inner.try_borrow().map_err(|_| busy())?.paused())
    }
    #[napi(js_name = "setPaused")]
    pub fn set_paused(&self, env: Env, deps: Object, on: bool, by: JsArg) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.set_paused(c, on, &by.0)).map(JsArg)
    }
    #[napi]
    pub fn grant(&self, env: Env, deps: Object, actor: JsArg, selector: JsArg, ttl_ms: JsArg, by: JsArg) -> Result<JsArg> {
        let ttl = opt(ttl_ms);
        self.run(&env, &deps, |p, c| p.grant(c, &actor.0, &selector.0, ttl.as_ref(), &by.0)).map(JsArg)
    }
    #[napi]
    pub fn revoke(&self, env: Env, deps: Object, actor: JsArg, by: JsArg) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.revoke(c, &actor.0, &by.0)).map(JsArg)
    }
    #[napi]
    pub fn grants(&self, env: Env, deps: Object) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.grants(c)).map(JsArg)
    }
    #[napi(js_name = "scopeOf")]
    pub fn scope_of(&self, env: Env, deps: Object, id: JsArg) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.scope_of(c, &id.0).map(|s| s.to_arg())).map(JsArg)
    }
    #[napi]
    pub fn visible(&self, env: Env, deps: Object, caller: JsArg, facts: JsArg) -> Result<bool> {
        self.run(&env, &deps, |p, c| p.visible(c, &caller.0, &facts.0))
    }
    #[napi(js_name = "checkSend")]
    pub fn check_send(&self, env: Env, deps: Object, sender: JsArg, recipients: JsArg, text: JsArg, depth: f64) -> Result<()> {
        let recipients = list(recipients.0);
        self.run(&env, &deps, |p, c| p.check_send(c, &sender.0, &recipients, &text.0, depth))
    }
    #[napi]
    pub fn allow(&self, env: Env, deps: Object, session: JsArg, remote: bool, ttl_ms: JsArg, by: JsArg) -> Result<JsArg> {
        let ttl = opt(ttl_ms);
        self.run(&env, &deps, |p, c| p.allow(c, &session.0, remote, ttl.as_ref(), &by.0)).map(JsArg)
    }
    #[napi]
    pub fn disallow(&self, env: Env, deps: Object, session: JsArg, by: JsArg) -> Result<()> {
        self.run(&env, &deps, |p, c| p.disallow(c, &session.0, &by.0))
    }
    #[napi]
    pub fn consented(&self, env: Env, deps: Object, session: JsArg, remote: bool) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.consented(c, &session.0, remote)).map(JsArg)
    }
    #[napi(js_name = "wakeAllowed")]
    pub fn wake_allowed(&self, env: Env, deps: Object, sender: JsArg, target: JsArg, usage: JsArg) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.wake_allowed(c, &sender.0, &target.0, &usage.0)).map(JsArg)
    }
    #[napi(js_name = "spendWake")]
    pub fn spend_wake(&self, env: Env, deps: Object, sender: JsArg, target: JsArg) -> Result<()> {
        self.run(&env, &deps, |p, c| p.spend_wake(c, &sender.0, &target.0))
    }
    #[napi]
    pub fn audit(&self, env: Env, deps: Object, entry: JsArg) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.audit(c, entry.0)).map(JsArg)
    }
    #[napi(js_name = "readAudit")]
    pub fn read_audit(&self, env: Env, deps: Object, limit: f64) -> Result<JsArg> {
        self.run(&env, &deps, |p, c| p.read_audit(c, limit)).map(JsArg)
    }
}
