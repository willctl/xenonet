//! xenonet-sources' live helpers, xenolith, refusals and OpenRouter usage, for
//! their twins.

use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use xenonet_sources::lines::TypeError;
use xenonet_sources::{live, xenolith};

// What JS would throw, thrown as JS throws it: a TypeError.
pub(crate) fn thrown<T>(env: &Env, e: TypeError) -> Result<T> {
    let _ = env.throw_type_error(&e.0, None);
    Err(Error::new(Status::PendingException, e.0))
}

// ---------- live.js's helpers ----------

#[napi(js_name = "liveMcpServerOf")]
pub fn live_mcp_server_of(tool_name: String) -> Option<String> {
    live::mcp_server_of(&tool_name)
}

#[napi(js_name = "liveCategorize")]
pub fn live_categorize(name: String) -> &'static str {
    live::categorize(&name)
}

#[napi(js_name = "liveClip")]
pub fn live_clip(s: String, n: f64) -> String {
    live::clip(&s, n.max(0.0) as usize)
}

#[napi(js_name = "liveDayKey")]
pub fn live_day_key(ts: f64) -> String {
    live::day_key(ts)
}

#[napi(js_name = "liveLooksLikeQuestion")]
pub fn live_looks_like_question(text: String) -> bool {
    live::looks_like_question(&text)
}

#[napi(js_name = "liveCommandText")]
pub fn live_command_text(command: MaybeJson) -> String {
    live::command_text(command.0.as_ref())
}

#[napi(js_name = "liveToolDetail")]
pub fn live_tool_detail(input: MaybeJson, home: Option<String>) -> String {
    live::tool_detail(input.0.as_ref(), home.as_deref())
}

#[napi(js_name = "liveFilesOf")]
pub fn live_files_of(input: MaybeJson) -> Vec<String> {
    live::files_of(input.0.as_ref())
}

#[napi(js_name = "livePatchFiles")]
pub fn live_patch_files(text: MaybeJson) -> Vec<String> {
    live::patch_files(text.0.as_ref())
}

#[napi(js_name = "liveClaudeEdits")]
pub fn live_claude_edits(env: &Env, name: String, input: MaybeJson) -> Result<Json> {
    match live::claude_edits(&name, input.0.as_ref()) {
        Ok(e) => Ok(Json(Value::Array(e))),
        Err(e) => thrown(env, e),
    }
}

#[napi(js_name = "liveCodexEdits")]
pub fn live_codex_edits(changes: MaybeJson) -> Json {
    Json(Value::Array(live::codex_edits(changes.0.as_ref())))
}

#[napi(js_name = "liveToolKey")]
pub fn live_tool_key(name: String, input: MaybeJson) -> String {
    live::tool_key(&name, input.0.as_ref())
}

#[napi(js_name = "liveWikiPagesOf")]
pub fn live_wiki_pages_of(server: MaybeJson, input: MaybeJson) -> Option<Vec<String>> {
    live::wiki_pages_of(server.0.as_ref().and_then(Value::as_str), input.0.as_ref())
}

#[napi(js_name = "liveFileKey")]
pub fn live_file_key(p: String) -> String {
    live::file_key(&p)
}

// ---------- xenolith.js ----------

#[napi(js_name = "contextServers")]
pub fn context_servers() -> Vec<&'static str> {
    xenolith::CONTEXT_SERVERS.to_vec()
}

/// { dir, server }, or null.
#[napi(js_name = "findContextDir")]
pub fn find_context_dir(env: Json, home: String, platform: String) -> Jv {
    match xenolith::find_context_dir(&xenolith::env_of(&env.0), &home, &platform) {
        Some((dir, server)) => Jv::Obj(vec![("dir", Jv::Json(Value::String(dir))), ("server", Jv::Json(Value::String(server.into())))]),
        None => Jv::Json(Value::Null),
    }
}

#[napi(js_name = "contextServerOf")]
pub fn context_server_of(dir: String) -> &'static str {
    xenolith::context_server_of(&dir)
}

#[napi(js_name = "contextName")]
pub fn context_name(server: Option<String>) -> &'static str {
    xenolith::context_name(server.as_deref())
}
