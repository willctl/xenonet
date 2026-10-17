//! xenonet-sources' Claude usage and refusal signals, for the twins of
//! core/claude-usage.js and core/refusals/detect.js.

use crate::js::{Json, Jv, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{Map, Value};
use xenonet_sources::claude_usage;
use xenonet_sources::lines::TypeError;
use xenonet_sources::refusals::detect;

fn maybe(v: Option<Value>) -> Jv {
    v.map_or(Jv::Undef, Jv::Json)
}

fn type_error<T>(env: &Env, what: &str) -> Result<T> {
    thrown(env, TypeError(what.into()))
}

// ---------- claude-usage.js ----------

#[napi(js_name = "claudeUsageConstants")]
pub fn claude_usage_constants() -> Json {
    let mut m = Map::new();
    m.insert("url".into(), claude_usage::USAGE_URL.into());
    m.insert("everyMs".into(), xenonet_base::json::number(claude_usage::USAGE_EVERY_MS));
    m.insert("keychainService".into(), claude_usage::KEYCHAIN_SERVICE.into());
    Json(Value::Object(m))
}

/// { token, expiresAt } from a credentials text, or null.
#[napi(js_name = "claudeUsageTokenOf")]
pub fn claude_usage_token_of(text: Option<String>) -> Json {
    Json(match claude_usage::token_of(text.as_deref()) {
        Some((token, expires)) => {
            let mut m = Map::new();
            m.insert("token".into(), Value::String(token));
            m.insert("expiresAt".into(), expires.map_or(Value::Null, xenonet_base::json::number));
            Value::Object(m)
        }
        None => Value::Null,
    })
}

#[napi(js_name = "claudeUsageParse")]
pub fn claude_usage_parse(body: MaybeJson, now: f64) -> Json {
    Json(claude_usage::parse_usage(body.0.as_ref(), now).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageSignInProblem")]
pub fn claude_usage_sign_in_problem(cred: MaybeJson, now: f64) -> Json {
    Json(claude_usage::sign_in_problem(cred.0.as_ref(), now).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageUnreachable")]
pub fn claude_usage_unreachable(name: MaybeJson, message: MaybeJson) -> Json {
    Json(claude_usage::unreachable(name.0.as_ref(), message.0.as_ref()))
}

#[napi(js_name = "claudeUsageStatusProblem")]
pub fn claude_usage_status_problem(status: MaybeJson, ok: bool, retry_after: MaybeJson) -> Json {
    Json(claude_usage::status_problem(status.0.as_ref(), ok, retry_after.0.as_ref()).unwrap_or(Value::Null))
}

#[napi(js_name = "claudeUsageReading")]
pub fn claude_usage_reading(body: MaybeJson, now: f64) -> Json {
    Json(claude_usage::reading(body.0.as_ref(), now))
}

#[napi(js_name = "claudeUsageNextIn")]
pub fn claude_usage_next_in(r: MaybeJson) -> Jv {
    maybe(claude_usage::next_in(r.0.as_ref()))
}

#[napi(js_name = "claudeUsageDesktopVersion")]
pub fn claude_usage_desktop_version(raw: MaybeJson) -> Option<String> {
    claude_usage::desktop_version(raw.0.as_ref())
}

#[napi(js_name = "claudeUsageDesktopSamples")]
pub fn claude_usage_desktop_samples(env: &Env, raw: MaybeJson, checked: MaybeJson) -> Result<Json> {
    match claude_usage::desktop_samples(raw.0.as_ref(), checked.0.as_ref()) {
        Ok(v) => Ok(Json(v)),
        Err(claude_usage::DesktopError::Plain(m)) => Err(Error::new(Status::GenericFailure, m)),
        Err(claude_usage::DesktopError::Type(m)) => thrown(env, TypeError(m)),
    }
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
