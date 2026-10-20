//! xenonet-sources' live helpers, xenolith, refusals and OpenRouter usage, for
//! their twins.

use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use xenonet_sources::lines::TypeError;
use xenonet_sources::{live, xenolith};

// What JS would throw, thrown as JS throws it: a TypeError.
pub(crate) fn thrown<T>(env: Env, e: TypeError) -> Result<T> {
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
pub fn live_claude_edits(env: Env, name: String, input: MaybeJson) -> Result<Json> {
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

// ---------- refusals/ ----------

use xenonet_sources::refusals::{demo, query, store};

// { ok } or { error: { message, code } }: the twin throws the error as
// Node would have, with its code, or a RangeError for a time JS can't
// write.
fn answer(r: std::result::Result<Value, store::StoreError>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
        Err(e) => Jv::Obj(vec![("error", Jv::Obj(vec![("message", Jv::Json(Value::String(e.message))), ("code", e.code.map_or(Jv::Undef, |c| Jv::Json(Value::String(c))))]))]),
    }
}

#[napi(js_name = "demoRefusals")]
pub fn demo_refusals(now: f64) -> Json {
    Json(Value::Array(demo::demo_refusals(now)))
}

#[napi(js_name = "refusalSorts")]
pub fn refusal_sorts() -> Vec<&'static str> {
    query::REFUSAL_SORTS.to_vec()
}

#[napi(js_name = "refusalPublicRecord")]
pub fn refusal_public_record(r: Json) -> Json {
    Json(query::public_record(&r.0))
}

#[napi(js_name = "refusalModelOf")]
pub fn refusal_model_of(r: Json) -> Json {
    Json(query::model_of(&r.0))
}

#[napi(js_name = "refusalCategoryOf")]
pub fn refusal_category_of(r: Json) -> Json {
    Json(query::category_of(&r.0))
}

#[napi(js_name = "queryRefusals")]
pub fn query_refusals(records: Json, q: Json) -> Json {
    Json(query::query_refusals(records.0.as_array().map_or(&[], Vec::as_slice), &q.0))
}

#[napi(js_name = "refusalSchema")]
pub fn refusal_schema() -> &'static str {
    store::SCHEMA
}

#[napi(js_name = "refusalClaudeContext")]
pub fn refusal_claude_context(all: Json, sig: Json) -> Json {
    Json(store::claude_context(all.0.as_array().map_or(&[], Vec::as_slice), &sig.0).map_or(Value::Null, |c| c.to_json()))
}

#[napi(js_name = "refusalCodexContext")]
pub fn refusal_codex_context(env: Env, all: Json, sig: Json) -> Result<Json> {
    match store::codex_context(all.0.as_array().map_or(&[], Vec::as_slice), &sig.0) {
        Ok(c) => Ok(Json(c.to_json())),
        Err(m) => thrown(env, TypeError(m)),
    }
}

#[napi(js_name = "refusalContextOn")]
pub fn refusal_context_on(dir: String, env_value: Option<String>) -> bool {
    store::context_on(std::path::Path::new(&dir), env_value.as_deref())
}

#[napi(js_name = "refusalSetContext")]
pub fn refusal_set_context(dir: String, on: bool) -> Jv {
    answer(store::set_context(std::path::Path::new(&dir), on).map(Value::Bool))
}

#[napi(js_name = "RefusalStore")]
pub struct JsRefusalStore {
    r: store::Refusals,
}

#[napi]
impl JsRefusalStore {
    #[napi(constructor)]
    pub fn new(dir: String, home: String) -> Self {
        JsRefusalStore { r: store::Refusals::new(&dir, &home) }
    }
    #[napi]
    pub fn known(&mut self, sig: Json, at: Json) -> bool {
        self.r.known(&sig.0, &at.0)
    }
    #[napi]
    pub fn record(&mut self, sig: Json, at: Json, now: f64, context: bool) -> Jv {
        answer(self.r.record(&sig.0, &at.0, now, context).map(|r| r.unwrap_or(Value::Null)))
    }
    #[napi]
    pub fn get(&self, id: String) -> Json {
        Json(self.r.get(&id).unwrap_or(Value::Null))
    }
    #[napi]
    pub fn list(&self, since: f64, provider: Option<String>, kind: Option<String>) -> Json {
        Json(Value::Array(self.r.list(since, provider.as_deref(), kind.as_deref())))
    }
    #[napi]
    pub fn capture(&self, id: String) -> Jv {
        answer(self.r.capture(&id))
    }
    #[napi(js_name = "exportLines")]
    pub fn export_lines(&self, since: f64, provider: Option<String>, kind: Option<String>, with_context: bool, scrubbed: bool) -> Json {
        Json(Value::Array(self.r.export_lines(since, provider.as_deref(), kind.as_deref(), with_context, scrubbed)))
    }
    #[napi(js_name = "contextFile")]
    pub fn context_file(&self, id: String) -> String {
        self.r.context_file(&id).to_string_lossy().into_owned()
    }
}

// ---------- openrouter-usage.js ----------

use xenonet_sources::openrouter_usage as openrouter;

#[napi(js_name = "openrouterKeyUrl")]
pub fn openrouter_key_url() -> &'static str {
    openrouter::KEY_URL
}

#[napi(js_name = "openrouterUsageOf")]
pub fn openrouter_usage_of(body: MaybeJson) -> Json {
    Json(openrouter::usage_of(body.0.as_ref()))
}

/// `reached` false when OpenRouter couldn't be asked; `status` and `ok`
/// are its answer's.
#[napi(js_name = "openrouterReading")]
pub fn openrouter_reading(has_key: bool, reached: bool, status: MaybeJson, ok: bool, body: MaybeJson, at: f64) -> Json {
    let status = status.0.unwrap_or(Value::Null);
    Json(openrouter::reading(has_key, reached.then_some((&status, ok)), body.0.as_ref(), at))
}

#[napi(js_name = "openrouterFresh")]
pub fn openrouter_fresh(at: f64, now: f64) -> bool {
    openrouter::fresh(at, now)
}

#[napi(js_name = "openrouterDemo")]
pub fn openrouter_demo(now: f64) -> Json {
    Json(openrouter::demo(now))
}
