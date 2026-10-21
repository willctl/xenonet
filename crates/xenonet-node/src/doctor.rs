//! xenonet-doctor for the twins of core/doctor/.

use crate::app::AppTask;
use crate::js::{Json, MaybeJson};
use napi_derive::napi;
use serde_json::{json, Value};
use xenonet_app::task;
use xenonet_doctor::{checks, run};

#[napi(js_name = "doctorConstants")]
pub fn doctor_constants() -> Json {
    Json(json!({ "statuses": checks::STATUSES, "checks": checks::all().iter().map(|(id, group, title)| json!([id, group, title])).collect::<Vec<_>>() }))
}

/// One check, as a task asking the twin's ctx for what it reads.
#[napi(js_name = "doctorCheck")]
pub fn doctor_check(id: String, facts: Json) -> AppTask {
    let f = checks::Facts { data: facts.0 };
    AppTask::of(task::Task::new(move |io| async move { checks::run(io, id, f).await }))
}

#[napi(js_name = "doctorHostCheck")]
pub fn doctor_host_check(h: Json, now: f64) -> Json {
    Json(run::host_check(&h.0, now))
}

#[napi(js_name = "doctorKept")]
pub fn doctor_kept(id: String, only: Vec<String>) -> bool {
    run::kept(&id, &only)
}

#[napi(js_name = "doctorSummarize")]
pub fn doctor_summarize(results: Json) -> Json {
    Json(run::summarize(results.0.as_array().map_or(&[][..], Vec::as_slice)))
}

#[napi(js_name = "doctorNewProblems")]
pub fn doctor_new_problems(prev: MaybeJson, next: Json) -> Json {
    Json(Value::Array(run::new_problems(&prev.0.unwrap_or(Value::Null), &next.0)))
}

#[napi(js_name = "doctorReadRun")]
pub fn doctor_read_run(file: Option<String>) -> Json {
    Json(run::read_run(file.as_deref()).unwrap_or(Value::Null))
}

#[napi(js_name = "doctorWriteRun")]
pub fn doctor_write_run(file: Option<String>, result: Json) {
    run::write_run(file.as_deref(), &result.0);
}
