//! xenonet-engine's engine for the twin of core/engine/engine.js: the
//! agents on this machine, and messages, claims, focus and hooks between
//! them. The store, the actors, the policy, delivery, repos and the map's
//! state are called through the JS objects the twin was given.

use crate::args::JsArg;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::ptr;
use std::sync::Arc;
use xenonet_engine::engine::{self as eng, Engine, Fail};
use xenonet_engine::peer::{list, Thrown};
use xenonet_engine::sql::Arg;

/// What a machine came to: { done }, or { refused: { message, status,
/// code } } for the twin to throw as the JS threw it.
fn outcome(r: eng::Result<Arg>) -> std::result::Result<Arg, Thrown> {
    match r {
        Ok(v) => Ok(Arg::Obj(vec![("done".into(), v)])),
        Err(Fail::Bad(message)) => Ok(Arg::Obj(vec![(
            "refused".into(),
            Arg::Obj(vec![("message".into(), Arg::text(message)), ("status".into(), Arg::num(400.0)), ("code".into(), Arg::text("bad-request"))]),
        )])),
        Err(Fail::Error(message)) => Err(Thrown::new(message)),
        Err(Fail::Thrown(t)) => Err(t),
    }
}

fn throw(env: &Env, f: Fail) -> Error {
    match f {
        Fail::Thrown(t) => rethrow(env, t),
        Fail::Error(m) => rethrow(env, Thrown::new(m)),
        Fail::Bad(message) => {
            settle(env);
            let raw = env.raw();
            unsafe {
                let mut msg = ptr::null_mut();
                sys::napi_create_string_utf8(raw, message.as_ptr().cast(), message.len() as isize, &raw mut msg);
                let mut err = ptr::null_mut();
                sys::napi_create_error(raw, ptr::null_mut(), msg, &raw mut err);
                for (k, v) in [(c"status", serde_json::json!(400)), (c"code", serde_json::json!("bad-request"))] {
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

fn done<T>(env: &Env, r: eng::Result<T>) -> Result<T> {
    match r {
        Ok(v) => {
            settle(env);
            Ok(v)
        }
        Err(f) => Err(throw(env, f)),
    }
}

#[napi(js_name = "engineIsAgentId")]
pub fn is_agent_id(id: JsArg) -> bool {
    eng::is_agent_id(&id.0)
}

#[napi(js_name = "engineIsActorAgent")]
pub fn is_actor_agent(id: JsArg) -> bool {
    eng::is_actor_agent(&id.0)
}

#[napi(js_name = "engineMatches")]
pub fn matches(pattern: JsArg, file: JsArg) -> bool {
    eng::matches(&pattern.0, &file.0)
}

#[napi(js_name = "engineEditPaths")]
pub fn edit_paths(tool: JsArg, input: JsArg, cwd: JsArg) -> Vec<String> {
    eng::edit_paths(&tool.0, &input.0, &cwd.0)
}

#[napi(js_name = "engineParseRepo")]
pub fn parse_repo(stdout: String) -> JsArg {
    JsArg(eng::parse_repo(&stdout).unwrap_or(Arg::Null))
}

#[napi(js_name = "engineTogether")]
pub fn together(a: JsArg, b: JsArg) -> bool {
    eng::together(&a.0, &b.0)
}

#[napi(js_name = "engineReplyTarget")]
pub fn reply_target(me: JsArg, m: JsArg) -> JsArg {
    JsArg(eng::reply_target(&me.0, &m.0))
}

#[napi]
pub struct EngineCore {
    inner: Arc<Engine>,
}

macro_rules! machine {
    ($self:ident, |$e:ident, $c:ident| $body:expr) => {{
        let $e = $self.inner.clone();
        start(move |asker| {
            let $c = &mut AskCalls(asker);
            outcome($body)
        })
    }};
}

#[napi]
impl EngineCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        EngineCore { inner: Arc::new(Engine::default()) }
    }

    #[napi]
    pub fn agents(&self, within_ms: f64) -> Machine {
        machine!(self, |e, c| e.agents(c, within_ms).map(Arg::List))
    }
    #[napi]
    pub fn view(&self, id: JsArg) -> Machine {
        machine!(self, |e, c| e.view(c, &id.0))
    }
    #[napi]
    pub fn whoami(&self, me: JsArg) -> Machine {
        machine!(self, |e, c| e.whoami(c, &me.0))
    }
    #[napi]
    pub fn peers(&self, me: JsArg, scope: JsArg, within_ms: f64) -> Machine {
        machine!(self, |e, c| e.peers(c, &me.0, &scope.0, within_ms).map(Arg::List))
    }
    #[napi]
    pub fn conflicts(&self, me: JsArg, files: JsArg) -> Machine {
        let files = list(files.0);
        machine!(self, |e, c| e.conflicts(c, &me.0, &files).map(Arg::List))
    }
    #[napi(js_name = "whoHas")]
    pub fn who_has(&self, me: JsArg, file: JsArg) -> Machine {
        machine!(self, |e, c| e.who_has(c, &me.0, &file.0).map(Arg::List))
    }
    #[napi]
    pub fn send(&self, me: JsArg, ask: JsArg, envelope: JsArg) -> Machine {
        machine!(self, |e, c| e.send(c, &me.0, &ask.0, &envelope.0))
    }
    #[napi]
    pub fn inbox(&self, me: JsArg, wait_ms: JsArg) -> Machine {
        machine!(self, |e, c| e.inbox(c, &me.0, &wait_ms.0).map(Arg::List))
    }
    #[napi]
    pub fn listen(&self, me: JsArg, ms: JsArg) -> Machine {
        machine!(self, |e, c| e.listen(c, &me.0, &ms.0))
    }
    #[napi]
    pub fn claim(&self, me: JsArg, ask: JsArg) -> Machine {
        machine!(self, |e, c| e.claim(c, &me.0, &ask.0))
    }
    #[napi]
    pub fn hook(&self, app: JsArg, event: JsArg, input: JsArg) -> Machine {
        machine!(self, |e, c| e.hook(c, &app.0, &event.0, &input.0))
    }
    #[napi]
    pub fn briefing(&self, me: JsArg) -> Machine {
        machine!(self, |e, c| e.briefing(c, &me.0).map(Arg::text))
    }
    #[napi]
    pub fn snapshot(&self) -> Machine {
        machine!(self, |e, c| e.snapshot(c))
    }

    #[napi]
    pub fn release(&self, env: Env, deps: Object<'_>, me: JsArg, paths: JsArg) -> Result<JsArg> {
        done(&env, self.inner.release(&mut SyncCalls::new(&env, &deps), &me.0, &paths.0)).map(JsArg)
    }
    #[napi(js_name = "setFocus")]
    pub fn set_focus(&self, env: Env, deps: Object<'_>, me: JsArg, text: JsArg) -> Result<JsArg> {
        done(&env, self.inner.set_focus(&mut SyncCalls::new(&env, &deps), &me.0, &text.0)).map(JsArg)
    }
    #[napi]
    pub fn touch(&self, env: Env, deps: Object<'_>, me: JsArg, ask: JsArg) -> Result<()> {
        done(&env, self.inner.touch(&mut SyncCalls::new(&env, &deps), &me.0, &ask.0))
    }
    #[napi]
    pub fn facts(&self, env: Env, deps: Object<'_>, a: JsArg) -> Result<JsArg> {
        done(&env, self.inner.facts(&mut SyncCalls::new(&env, &deps), &a.0)).map(JsArg)
    }
    #[napi(js_name = "visibleTo")]
    pub fn visible_to(&self, env: Env, deps: Object<'_>, me: JsArg, a: JsArg) -> Result<bool> {
        done(&env, self.inner.visible_to(&mut SyncCalls::new(&env, &deps), &me.0, &a.0))
    }
}
