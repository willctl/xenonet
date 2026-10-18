//! xenonet-actors' readers of Hermes's files for the twin of
//! core/actors/hermes-files.js: the twin hands over its files, its folders,
//! the vendor readers, Date.parse and Hermes's own database handle.

use crate::args::JsArg;
use crate::js::{Json, MaybeJson};
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use xenonet_actors::hermes_files as hf;
use xenonet_engine::peer::list;

fn done<T>(env: &Env, r: xenonet_engine::peer::Result<T>) -> Result<T> {
    let out = r.map_err(|t| rethrow(env, t));
    settle(env);
    out
}

fn version(v: MaybeJson) -> Option<String> {
    v.0.and_then(|v| v.as_str().map(str::to_string))
}

#[napi(js_name = "hermesConstants")]
pub fn hermes_constants() -> Json {
    Json(json!({ "source": hf::SOURCE.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(), "editTools": hf::EDIT_TOOLS }))
}

#[napi(js_name = "hermesMsOf")]
pub fn ms_of(env: Env, deps: Object<'_>, v: MaybeJson) -> Result<Option<f64>> {
    let mut c = SyncCalls::new(&env, &deps);
    let mut p = hf::parse_date(&mut c);
    let out = v.0.as_ref().and_then(|v| hf::ms_of(v, &mut p));
    settle(&env);
    Ok(out)
}

#[napi(js_name = "hermesSameStart")]
pub fn same_start(recorded: MaybeJson, table_start: MaybeJson, slack: f64) -> bool {
    hf::same_start(&recorded.0.unwrap_or(Value::Null), &table_start.0.unwrap_or(Value::Null), slack)
}

/// HERMES_HOME (resolved) when it's set, else <home>/.hermes.
#[napi(js_name = "hermesRoot")]
pub fn hermes_root(home: String, hermes_home: Option<String>) -> String {
    match hermes_home.filter(|h| !h.is_empty()) {
        Some(h) => xenonet_base::paths::resolve(&h),
        None => xenonet_base::paths::join(&home, ".hermes"),
    }
}

#[napi(js_name = "hermesListProfiles")]
pub fn list_profiles(env: Env, deps: Object<'_>, root: String) -> Result<Json> {
    done(&env, hf::list_profiles(&mut SyncCalls::new(&env, &deps), &root)).map(Json)
}

#[napi(js_name = "hermesInstalled")]
pub fn installed(env: Env, deps: Object<'_>, root: String) -> Result<bool> {
    done(&env, hf::installed(&mut SyncCalls::new(&env, &deps), &root))
}

#[napi(js_name = "hermesReadStamp")]
pub fn read_stamp(env: Env, deps: Object<'_>, root: String) -> Result<Json> {
    done(&env, hf::read_stamp(&mut SyncCalls::new(&env, &deps), &root)).map(Json)
}

#[napi(js_name = "hermesRead")]
pub fn read(env: Env, deps: Object<'_>, what: String, home: String, v: MaybeJson) -> Result<Json> {
    let mut c = SyncCalls::new(&env, &deps);
    let v = version(v);
    let out = match what.as_str() {
        "ledger" => hf::read_ledger(&mut c, &home, v),
        "gateway" => hf::read_gateway(&mut c, &home, v),
        "ticker" => hf::read_ticker(&mut c, &home, v),
        "processes" => hf::read_processes(&mut c, &home, v),
        "jobs" => hf::read_jobs(&mut c, &home, v),
        other => Err(xenonet_engine::peer::Thrown::new(format!("no reader {other}"))),
    };
    done(&env, out).map(Json)
}

#[napi(js_name = "hermesReadStateDb")]
pub fn read_state_db(env: Env, deps: Object<'_>, file: String, after_id: MaybeJson, since_ms: MaybeJson, session_ids: JsArg) -> Result<Json> {
    let ids: Vec<Value> = list(session_ids.0).iter().map(xenonet_engine::sql::Arg::to_json).collect();
    done(&env, hf::read_state_db(&mut SyncCalls::new(&env, &deps), &file, &after_id.0.unwrap_or(Value::Null), &since_ms.0.unwrap_or(Value::Null), &ids)).map(Json)
}
