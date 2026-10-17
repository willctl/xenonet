//! xenonet-sources' refusal signals, for the twin of
//! core/refusals/detect.js.

use crate::js::{Json, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use xenonet_sources::lines::TypeError;
use xenonet_sources::refusals::detect;

fn type_error<T>(env: &Env, what: &str) -> Result<T> {
    thrown(env, TypeError(what.into()))
}

// ---------- refusals/detect.js ----------

#[napi(js_name = "refusalClaudeSignal")]
pub fn refusal_claude_signal(raw: MaybeJson) -> Json {
    Json(detect::claude_signal(raw.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "refusalCodexSignal")]
pub fn refusal_codex_signal(raw: MaybeJson) -> Json {
    Json(detect::codex_signal(raw.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "refusalKeyOf")]
pub fn refusal_key_of(env: &Env, sig: MaybeJson, session_id: MaybeJson) -> Result<String> {
    let session = match &session_id.0 {
        None => "undefined".to_string(),
        Some(v) => xenonet_base::text::js_string(v),
    };
    match &sig.0 {
        None => type_error(env, "Cannot read properties of undefined (reading 'provider')"),
        Some(Value::Null) => type_error(env, "Cannot read properties of null (reading 'provider')"),
        Some(s) => Ok(detect::key_of(s, &session)),
    }
}
