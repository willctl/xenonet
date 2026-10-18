//! xenonet-actors' connections for the twin of core/actors/connections.js:
//! remote actors' connections and their grants, over the engine store the
//! twin's store holds. The actors, the credentials' files and the rest are
//! called through the JS objects the twin was given.

use crate::args::JsArg;
use crate::js::Json;
use crate::peer::{rethrow, settle, SyncCalls};
use crate::store::EngineStoreCore;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::ptr;
use std::sync::Arc;
use xenonet_actors::connections::{self as conn, Connections, Fail};

/// A refusal thrown for the twin: an Error marked as a ConnectionError or
/// a LinkError, with its status, code and extra fields.
fn throw(env: &Env, f: Fail) -> Error {
    let (message, fields): (String, Vec<(&std::ffi::CStr, Value)>) = match f {
        Fail::Thrown(t) => return rethrow(env, t),
        Fail::Connection { status, code, message, extra } => {
            (message, vec![(c"xenonetConnection", json!(true)), (c"status", xenonet_base::json::number(status)), (c"code", json!(code)), (c"extra", extra)])
        }
        Fail::Link { code, message } => (message, vec![(c"xenonetLink", json!(true)), (c"code", json!(code))]),
    };
    settle(env);
    let raw = env.raw();
    unsafe {
        let mut msg = ptr::null_mut();
        sys::napi_create_string_utf8(raw, message.as_ptr().cast(), message.len() as isize, &raw mut msg);
        let mut err = ptr::null_mut();
        sys::napi_create_error(raw, ptr::null_mut(), msg, &raw mut err);
        for (k, v) in fields {
            if let Ok(js) = crate::js::to_js(raw, &v, 0) {
                sys::napi_set_named_property(raw, err, k.as_ptr(), js);
            }
        }
        sys::napi_throw(raw, err);
    }
    Error::new(Status::PendingException, message)
}

fn done<T>(env: Env, r: conn::Result<T>) -> Result<T> {
    match r {
        Ok(v) => {
            settle(&env);
            Ok(v)
        }
        Err(f) => Err(throw(&env, f)),
    }
}

#[napi(js_name = "connectionsConstants")]
pub fn constants() -> Json {
    Json(json!({
        "grantDays": conn::GRANT_DAYS.iter().map(|d| xenonet_base::json::number(*d)).collect::<Vec<_>>(),
        "defaultGrantDays": xenonet_base::json::number(conn::DEFAULT_GRANT_DAYS),
        "lookbackDays": xenonet_base::json::number(conn::LOOKBACK_DAYS),
        "remoteActorsMax": conn::REMOTE_ACTORS_MAX,
        "connectionsMax": conn::CONNECTIONS_MAX,
        "projectsMax": conn::PROJECTS_MAX,
    }))
}

#[napi(js_name = "connectionsFingerprint")]
pub fn fingerprint(host: JsArg, user: JsArg, state_dir: JsArg) -> String {
    conn::fingerprint_of(&host.0, &user.0, &state_dir.0)
}

#[napi(js_name = "connectionsLaunch")]
pub fn launch(grant: JsArg, port: JsArg) -> JsArg {
    JsArg(conn::launch_for(&grant.0, &port.0))
}

#[napi]
pub struct ConnectionsCore {
    inner: Arc<Connections>,
}

#[napi]
impl ConnectionsCore {
    #[napi(constructor)]
    pub fn new(store: &EngineStoreCore, dir: JsArg, fingerprint: JsArg, port: JsArg) -> Self {
        ConnectionsCore { inner: Arc::new(Connections::new(store.inner.clone(), dir.0, fingerprint.0, port.0)) }
    }

    #[napi]
    pub fn start(&self, env: Env, deps: Object<'_>) -> Result<()> {
        done(env, self.inner.start(&mut SyncCalls::new(&env, &deps)))
    }
    #[napi]
    pub fn device(&self, env: Env, deps: Object<'_>) -> Result<Json> {
        done(env, self.inner.device(&mut SyncCalls::new(&env, &deps))).map(Json)
    }
    #[napi(js_name = "idKey")]
    pub fn id_key(&self, env: Env, deps: Object<'_>) -> Result<Json> {
        done(env, self.inner.id_key(&mut SyncCalls::new(&env, &deps))).map(Json)
    }
    #[napi]
    pub fn disabled(&self, env: Env, deps: Object<'_>) -> Result<bool> {
        done(env, self.inner.disabled(&mut SyncCalls::new(&env, &deps)))
    }
    #[napi]
    pub fn pair(&self, env: Env, deps: Object<'_>, ask: JsArg) -> Result<JsArg> {
        done(env, self.inner.pair(&mut SyncCalls::new(&env, &deps), &ask.0)).map(JsArg)
    }
    #[napi]
    pub fn update(&self, env: Env, deps: Object<'_>, grant_id: JsArg, change: JsArg) -> Result<JsArg> {
        done(env, self.inner.update(&mut SyncCalls::new(&env, &deps), &grant_id.0, &change.0)).map(JsArg)
    }
    #[napi]
    pub fn rotate(&self, env: Env, deps: Object<'_>, grant_id: JsArg) -> Result<JsArg> {
        done(env, self.inner.rotate(&mut SyncCalls::new(&env, &deps), &grant_id.0)).map(JsArg)
    }
    #[napi]
    pub fn revoke(&self, env: Env, deps: Object<'_>, grant_id: JsArg, reason: JsArg) -> Result<JsArg> {
        done(env, self.inner.revoke(&mut SyncCalls::new(&env, &deps), &grant_id.0, &reason.0)).map(JsArg)
    }
    #[napi]
    pub fn disable(&self, env: Env, deps: Object<'_>) -> Result<JsArg> {
        done(env, self.inner.disable(&mut SyncCalls::new(&env, &deps))).map(JsArg)
    }
    #[napi]
    pub fn enable(&self, env: Env, deps: Object<'_>) -> Result<JsArg> {
        done(env, self.inner.enable(&mut SyncCalls::new(&env, &deps))).map(JsArg)
    }
    #[napi]
    pub fn authenticate(&self, env: Env, deps: Object<'_>, ask: JsArg) -> Result<JsArg> {
        done(env, self.inner.authenticate(&mut SyncCalls::new(&env, &deps), &ask.0).map(|s| s.to_arg())).map(JsArg)
    }
    #[napi]
    pub fn recheck(&self, env: Env, deps: Object<'_>, grant_id: JsArg, revision: JsArg) -> Result<JsArg> {
        done(env, self.inner.recheck(&mut SyncCalls::new(&env, &deps), &grant_id.0, &revision.0)).map(JsArg)
    }
    #[napi]
    pub fn policy(&self, env: Env, deps: Object<'_>, standing: JsArg) -> Result<JsArg> {
        done(env, self.inner.policy(&mut SyncCalls::new(&env, &deps), &standing.0)).map(JsArg)
    }
    #[napi]
    pub fn used(&self, env: Env, grant_id: JsArg, t: f64) -> Result<()> {
        done(env, self.inner.used(&grant_id.0, t))
    }
    #[napi]
    pub fn bridge(&self, env: Env, deps: Object<'_>, connection_id: JsArg, delta: f64) -> Result<()> {
        done(env, self.inner.bridge(&mut SyncCalls::new(&env, &deps), &connection_id.0, delta))
    }
    #[napi]
    pub fn list(&self, env: Env, deps: Object<'_>) -> Result<JsArg> {
        done(env, self.inner.list(&mut SyncCalls::new(&env, &deps))).map(JsArg)
    }
    #[napi]
    pub fn view(&self, env: Env, deps: Object<'_>, grant_id: JsArg) -> Result<JsArg> {
        done(env, self.inner.view(&mut SyncCalls::new(&env, &deps), &grant_id.0)).map(JsArg)
    }
    #[napi]
    pub fn tick(&self, env: Env, deps: Object<'_>, t: f64) -> Result<()> {
        done(env, self.inner.tick(&mut SyncCalls::new(&env, &deps), t))
    }
    #[napi]
    pub fn sync(&self, env: Env, deps: Object<'_>, t: f64) -> Result<()> {
        done(env, self.inner.sync(&mut SyncCalls::new(&env, &deps), t))
    }
    #[napi(js_name = "beforeRemove")]
    pub fn before_remove(&self, env: Env, deps: Object<'_>, slug: JsArg) -> Result<()> {
        done(env, self.inner.before_remove(&mut SyncCalls::new(&env, &deps), &slug.0))
    }
    #[napi(js_name = "anyActive")]
    pub fn any_active(&self, env: Env, deps: Object<'_>) -> Result<bool> {
        done(env, self.inner.any_active(&mut SyncCalls::new(&env, &deps)))
    }
}
