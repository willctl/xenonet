//! xenonet-engine's envelopes for the twin of core/engine/attest.js: how
//! XenoNet knows who sent a message, written from its own checks.

use crate::args::JsArg;
use crate::peer::rethrow;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use xenonet_engine::attest;

#[napi(js_name = "attestEnvelope")]
pub fn envelope(env: Env, sender: JsArg, method: JsArg, verdict: JsArg, home: JsArg, now: JsArg, platform: String) -> Result<JsArg> {
    attest::attest(&sender.0, &method.0, &verdict.0, &home.0, &now.0, &platform).map(JsArg).map_err(|t| rethrow(&env, t))
}

#[napi(js_name = "attestLine")]
pub fn line(envelope: JsArg, platform: String) -> String {
    attest::verified_line(&envelope.0, &platform)
}

#[napi(js_name = "attestQuoted")]
pub fn quoted(text: String) -> String {
    attest::quoted(&text)
}
