//! xenonet-actors' account links for the twin of core/actors/account-links.js,
//! over the engine store the twin's store holds.

use crate::args::JsArg;
use crate::js::Json;
use crate::peer::rethrow;
use crate::store::EngineStoreCore;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::json;
use std::ptr;
use xenonet_actors::account_links::{self as links, LinkFail};
use xenonet_engine::peer::list;

/// A LinkError for the twin: an Error marked xenonetLink, with its code.
fn fail(env: &Env, f: LinkFail) -> Error {
    match f {
        LinkFail::Thrown(t) => rethrow(env, t),
        LinkFail::Link { code, message } => {
            let raw = env.raw();
            unsafe {
                let mut msg = ptr::null_mut();
                sys::napi_create_string_utf8(raw, message.as_ptr().cast(), message.len() as isize, &mut msg);
                let mut err = ptr::null_mut();
                sys::napi_create_error(raw, ptr::null_mut(), msg, &mut err);
                for (k, v) in [(c"xenonetLink", json!(true)), (c"code", json!(code))] {
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

#[napi(js_name = "linkAssert")]
pub fn assert_link(env: Env, store: &EngineStoreCore, slug: JsArg, identity: JsArg, provider: JsArg, known: JsArg, now: JsArg) -> Result<JsArg> {
    let known = (!known.0.is_nullish()).then(|| list(known.0));
    links::assert_link(&store.inner, &slug.0, &identity.0, &provider.0, known.as_deref(), &now.0).map(JsArg).map_err(|f| fail(&env, f))
}

#[napi(js_name = "linkRecordVerified")]
pub fn record_verified(env: Env, store: &EngineStoreCore, slug: JsArg, facts: JsArg, known: JsArg, now: f64) -> Result<Json> {
    let known = list(known.0);
    let (identity, switched) = links::record_verified(&store.inner, &slug.0, &facts.0, &known, now).map_err(|f| fail(&env, f))?;
    Ok(Json(json!({ "identity": identity, "switched": switched })))
}

#[napi(js_name = "linkState")]
pub fn link_state(env: Env, store: &EngineStoreCore, slug: JsArg, now: f64) -> Result<Json> {
    let (state, identity) = links::link_state(&store.inner, &slug.0, now).map_err(|e| crate::args::throw_sql(&env, &e))?;
    Ok(Json(json!({ "state": state, "identity": identity })))
}

#[napi(js_name = "linkAddExternalRef")]
pub fn add_external_ref(env: Env, store: &EngineStoreCore, slug: JsArg, adapter_id: JsArg, facts: JsArg, now: JsArg) -> Result<Json> {
    links::add_external_ref(&store.inner, &slug.0, &adapter_id.0, &facts.0, &now.0).map(Json).map_err(|f| fail(&env, f))
}
