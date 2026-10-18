//! xenonet-integrations for the twins of core/integrations/.

use crate::app::{s, AppTask};
use crate::js::{Json, Jv, MaybeJson};
use napi_derive::napi;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;
use xenonet_app::task;
use xenonet_integrations::common::{self, Fail};
use xenonet_integrations::{index, openshell, pi, reviews, routing};

fn arr(v: Option<Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

fn obj(v: Option<Value>) -> Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

/// { ok } or { error: { status, code, message } }; status 0 for an error
/// the JS let through as it was.
fn res(r: Result<Value, Fail>) -> Jv {
    match r {
        Ok(v) => Jv::Obj(vec![("ok", Jv::Json(v))]),
        Err(f) => Jv::Obj(vec![("error", Jv::Obj(vec![("status", Jv::Num(f64::from(f.status))), ("code", s(f.code)), ("message", s(f.message))]))]),
    }
}

#[napi(js_name = "intgConstants")]
pub fn intg_constants() -> Json {
    Json(json!({ "reviewLimits": reviews::limits(), "reviewStorage": reviews::storage(), "routingNote": routing::NOTE }))
}

// ----- common -----

#[napi(js_name = "intgClean")]
pub fn intg_clean(value: MaybeJson, max: Option<f64>) -> String {
    common::clean(value.0.as_ref(), max.map_or(160, |m| m.max(0.0) as usize))
}

#[napi(js_name = "intgText")]
pub fn intg_text(value: MaybeJson, name: String, max: Option<f64>) -> Jv {
    res(common::text(value.0.as_ref(), &name, max.map_or(2000, |m| m.max(0.0) as usize)).map(Value::String))
}

#[napi(js_name = "intgSandboxGone")]
pub fn intg_sandbox_gone(r: Json) -> bool {
    common::sandbox_gone(&r.0)
}

#[napi(js_name = "intgHasCredential")]
pub fn intg_has_credential(text: String) -> bool {
    common::has_credential(&text)
}

/// Previews' tickets, for createTickets.
#[napi(js_name = "IntgTickets")]
pub struct IntgTickets {
    inner: common::Tickets,
}

#[napi]
impl IntgTickets {
    #[napi(constructor)]
    pub fn new() -> Self {
        IntgTickets { inner: Default::default() }
    }

    #[napi]
    pub fn check(&mut self, now: f64) -> Jv {
        res(self.inner.check(now).map(|()| Value::Null))
    }

    #[napi]
    pub fn issue(&mut self, value: MaybeJson, now: f64) -> Jv {
        res(self.inner.issue(value.0.unwrap_or(Value::Null), now))
    }

    #[napi]
    pub fn take(&mut self, id: String, now: f64) -> Jv {
        res(self.inner.take(&id, now))
    }
}

/// The records of decisions and sandbox jobs, for createRecords.
#[napi(js_name = "IntgRecords")]
pub struct IntgRecords {
    inner: common::Records,
}

#[napi]
impl IntgRecords {
    #[napi(factory)]
    pub fn of(dir: Option<String>, limit: f64) -> napi::Result<Self> {
        common::Records::new(dir, if limit.is_finite() { limit.max(0.0) as usize } else { usize::MAX })
            .map(|inner| IntgRecords { inner })
            .map_err(|f| napi::Error::from_reason(f.message))
    }

    #[napi]
    pub fn list(&self) -> Jv {
        res(self.inner.list().map(Value::Array))
    }

    #[napi]
    pub fn add(&mut self, value: Json, now: f64) -> Jv {
        res(self.inner.add(&value.0, now))
    }

    #[napi]
    pub fn update(&mut self, id: MaybeJson, fields: Json) -> Jv {
        res(self.inner.update(&id.0.unwrap_or(Value::Null), &fields.0))
    }
}

#[napi(js_name = "intgQuery")]
pub fn intg_query(records: Json, params: MaybeJson) -> Json {
    Json(common::query(&arr(Some(records.0)), &params.0.unwrap_or_else(|| json!({}))))
}

// ----- routing -----

#[napi(js_name = "intgInventory")]
pub fn intg_inventory(nodes: MaybeJson) -> Json {
    Json(routing::inventory(&arr(nodes.0)))
}

#[napi(js_name = "intgRoutingRequest")]
pub fn intg_routing_request(body: MaybeJson) -> Jv {
    res(routing::request_of(body.0.as_ref()).map(Value::String))
}

#[napi(js_name = "intgOutgoing")]
pub fn intg_outgoing(model: MaybeJson, request: String, candidates: Json) -> Json {
    Json(routing::outgoing(&model.0.unwrap_or(Value::Null), &request, &arr(Some(candidates.0))))
}

#[napi(js_name = "intgPayload")]
pub fn intg_payload(request: String, candidates: Json) -> Json {
    Json(routing::payload(&request, &arr(Some(candidates.0))))
}

#[napi(js_name = "intgAnswerOk")]
pub fn intg_answer_ok(answers: MaybeJson, candidates: Json) -> bool {
    routing::answer_ok(&answers.0.unwrap_or(Value::Null), &arr(Some(candidates.0)))
}

#[napi(js_name = "intgDecisionRecord")]
pub fn intg_decision_record(result: Json, request: String, metrics: MaybeJson) -> Json {
    Json(routing::decision_record(&result.0, &request, &metrics.0.unwrap_or_else(|| json!({}))))
}

#[napi(js_name = "intgOutcomeOk")]
pub fn intg_outcome_ok(outcome: MaybeJson) -> Jv {
    res(routing::outcome_ok(&outcome.0.unwrap_or(Value::Null)).map(|()| Value::Null))
}

// ----- pi -----

#[napi(js_name = "intgPiExport")]
pub fn intg_pi_export(value: MaybeJson) -> Jv {
    res(pi::export_metadata(&value.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "intgPiSession")]
pub fn intg_pi_session(content: String, file: String) -> Jv {
    res(pi::session_metadata(&content, &file))
}

#[napi(js_name = "intgScanPi")]
pub fn intg_scan_pi(root: String, max_files: f64, max_bytes: f64) -> Json {
    Json(pi::scan(&root, max_files.max(0.0) as usize, max_bytes.max(0.0) as u64))
}

// ----- reviews -----

#[napi(js_name = "intgExcludedPath")]
pub fn intg_excluded_path(name: String) -> bool {
    reviews::excluded_path(&name)
}

#[napi(js_name = "intgReviewPath")]
pub fn intg_review_path(name: MaybeJson) -> bool {
    reviews::review_path(&name.0.unwrap_or(Value::Null))
}

#[napi(js_name = "intgTextPatch")]
pub fn intg_text_patch(before: Json, after: Json) -> Json {
    Json(reviews::text_patch(&arr(Some(before.0)), &arr(Some(after.0))))
}

#[napi(js_name = "intgValidateResult")]
pub fn intg_validate_result(value: MaybeJson) -> Jv {
    res(reviews::validate_result(&value.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "intgDigest")]
pub fn intg_digest(files: Json) -> String {
    reviews::digest(&arr(Some(files.0)))
}

/// sourceSnapshot, as a task asking for git.
#[napi(js_name = "intgSourceSnapshot")]
pub fn intg_source_snapshot(source: String, env: Json) -> AppTask {
    let env = obj(Some(env.0));
    AppTask::of(task::Task::new(move |io| async move { reviews::source_snapshot(io, source, env).await }))
}

/// The local reviews, for createReviews.
#[napi(js_name = "IntgReviews")]
pub struct IntgReviews {
    inner: reviews::Shared,
}

#[napi]
impl IntgReviews {
    #[napi(constructor)]
    pub fn new(dir: Option<String>, demo: bool) -> Self {
        IntgReviews { inner: Rc::new(RefCell::new(reviews::Reviews::new(dir, demo))) }
    }

    #[napi]
    pub fn seed(&self, options: MaybeJson, now: f64) -> Jv {
        res(self.inner.borrow_mut().seed(&options.0.unwrap_or_else(|| json!({})), now))
    }

    #[napi]
    pub fn view(&self, id: MaybeJson) -> Jv {
        res(self.inner.borrow_mut().view(&id.0.unwrap_or(Value::Null)))
    }

    #[napi(js_name = "verifyUploaded")]
    pub fn verify_uploaded(&self, id: MaybeJson, input: MaybeJson) -> Jv {
        res(self.inner.borrow_mut().verify_uploaded(&id.0.unwrap_or(Value::Null), &input.0.unwrap_or(Value::Null)).map(|()| Value::Null))
    }

    #[napi]
    pub fn export(&self, id: MaybeJson, revision: MaybeJson) -> Jv {
        res(self.inner.borrow_mut().export(&id.0.unwrap_or(Value::Null), &revision.0.unwrap_or(Value::Null)))
    }

    #[napi]
    pub fn forget(&self, id: MaybeJson) -> Jv {
        res(self.inner.borrow_mut().forget(&id.0.unwrap_or(Value::Null)))
    }

    #[napi]
    pub fn prepare(&self, source: MaybeJson, env: Json) -> AppTask {
        let (r, env, source) = (self.inner.clone(), obj(Some(env.0)), source.0.unwrap_or(Value::Null));
        AppTask::of(task::Task::new(move |io| async move { reviews::prepare(io, r, source, env).await }))
    }

    #[napi]
    pub fn launch(&self, id: MaybeJson) -> AppTask {
        let (r, id) = (self.inner.clone(), id.0.unwrap_or(Value::Null));
        AppTask::of(task::Task::new(move |io| async move { reviews::launch(io, r, id).await }))
    }

    #[napi]
    pub fn accept(&self, id: MaybeJson, input: MaybeJson, env: Json) -> AppTask {
        let (r, id, input, env) = (self.inner.clone(), id.0.unwrap_or(Value::Null), input.0.unwrap_or(Value::Null), obj(Some(env.0)));
        AppTask::of(task::Task::new(move |io| async move { reviews::accept(io, r, id, input, env).await }))
    }
}

// ----- openshell -----

#[napi(js_name = "intgWslFile")]
pub fn intg_wsl_file(system_root: Option<String>) -> String {
    openshell::wsl_file(system_root.as_deref())
}

#[napi(js_name = "intgWslProfiles")]
pub fn intg_wsl_profiles(stdout: String) -> Json {
    Json(openshell::wsl_profiles(&stdout))
}

#[napi(js_name = "intgCommand")]
pub fn intg_command(profile: MaybeJson, args: Json, platform: String, system_root: Option<String>) -> Jv {
    res(openshell::command(&profile.0.unwrap_or(Value::Null), &arr(Some(args.0)), &platform, system_root.as_deref()))
}

#[napi(js_name = "intgSourceDir")]
pub fn intg_source_dir(value: MaybeJson) -> Jv {
    res(openshell::source_dir(value.0.as_ref()).map(Value::String))
}

#[napi(js_name = "intgPolicyFile")]
pub fn intg_policy_file(value: MaybeJson) -> Jv {
    res(openshell::policy_file(value.0.as_ref()))
}

/// One OpenShell integration; each action a task asking for its commands,
/// records and reviews.
#[napi(js_name = "IntgOpenShell")]
pub struct IntgOpenShell {
    inner: openshell::Shared,
}

macro_rules! shell_task {
    ($self:ident, |$io:ident, $sh:ident| $body:expr) => {{
        let $sh = $self.inner.clone();
        AppTask::of(task::Task::new(move |$io| async move { $body.await }))
    }};
}

#[napi]
impl IntgOpenShell {
    #[napi(constructor)]
    pub fn new(platform: String, system_root: Option<String>, collect_text: String) -> Self {
        IntgOpenShell { inner: Rc::new(RefCell::new(openshell::OpenShell { tickets: Default::default(), platform, system_root, collect_text })) }
    }

    #[napi]
    pub fn status(&self, profile: MaybeJson) -> AppTask {
        let p = profile.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::status(io, sh, p))
    }

    #[napi]
    pub fn preview(&self, body: MaybeJson) -> AppTask {
        let b = body.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::preview(io, sh, b))
    }

    #[napi]
    pub fn launch(&self, ticket: MaybeJson) -> AppTask {
        let t = ticket.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::launch(io, sh, t))
    }

    #[napi]
    pub fn inspect(&self, id: MaybeJson) -> AppTask {
        let id = id.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::inspect(io, sh, id))
    }

    #[napi]
    pub fn review(&self, id: MaybeJson) -> AppTask {
        let id = id.0.unwrap_or(Value::Null);
        AppTask::of(task::Task::new(move |io| async move { openshell::review(io, id).await }))
    }

    #[napi]
    pub fn retrieve(&self, id: MaybeJson) -> AppTask {
        let id = id.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::retrieve(io, sh, id))
    }

    #[napi(js_name = "exportReview")]
    pub fn export_review(&self, id: MaybeJson, revision: MaybeJson) -> AppTask {
        let (id, rev) = (id.0.unwrap_or(Value::Null), revision.0.unwrap_or(Value::Null));
        AppTask::of(task::Task::new(move |io| async move { openshell::export_review(io, id, rev).await }))
    }

    #[napi(js_name = "forgetReview")]
    pub fn forget_review(&self, id: MaybeJson) -> AppTask {
        let id = id.0.unwrap_or(Value::Null);
        AppTask::of(task::Task::new(move |io| async move { openshell::forget_review(io, id).await }))
    }

    #[napi]
    pub fn remove(&self, id: MaybeJson) -> AppTask {
        let id = id.0.unwrap_or(Value::Null);
        shell_task!(self, |io, sh| openshell::remove(io, sh, id))
    }
}

// ----- index -----

#[napi(js_name = "intgPiOn")]
pub fn intg_pi_on(file: Option<String>) -> bool {
    index::pi_on(file.as_deref())
}

#[napi(js_name = "intgSetPi")]
pub fn intg_set_pi(file: Option<String>, on: MaybeJson) -> Jv {
    res(index::set_pi(file.as_deref(), &on.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "intgPiRoot")]
pub fn intg_pi_root(home: String) -> String {
    index::pi_root(&home)
}

#[napi(js_name = "intgPiRootShown")]
pub fn intg_pi_root_shown(home: String) -> String {
    index::pi_root_shown(&home)
}

#[napi(js_name = "intgHistory")]
pub fn intg_history(records: Json, params: MaybeJson) -> Json {
    Json(index::history(&arr(Some(records.0)), &params.0.unwrap_or_else(|| json!({}))))
}

#[napi(js_name = "intgPi")]
pub fn intg_pi(records: Json, scanned: Json, enabled: bool, params: MaybeJson) -> Json {
    Json(index::pi(&arr(Some(records.0)), &scanned.0, enabled, &params.0.unwrap_or_else(|| json!({}))))
}

#[napi(js_name = "intgImportPlan")]
pub fn intg_import_plan(records: Json, value: MaybeJson) -> Jv {
    res(index::import_plan(&arr(Some(records.0)), &value.0.unwrap_or(Value::Null)))
}

#[napi(js_name = "intgDemoDecision")]
pub fn intg_demo_decision() -> Json {
    Json(index::demo_decision())
}

#[napi(js_name = "intgDemoRun")]
pub fn intg_demo_run(args: Json) -> Jv {
    match index::demo_run(&arr(Some(args.0))) {
        Ok(out) => Jv::Obj(vec![("ok", s(out))]),
        Err(m) => Jv::Obj(vec![("error", s(m))]),
    }
}

#[napi(js_name = "intgDemoRecords")]
pub fn intg_demo_records() -> Json {
    Json(index::demo_records())
}

#[napi(js_name = "intgDemoSandbox")]
pub fn intg_demo_sandbox(name: String, options: Json, review_id: MaybeJson) -> Json {
    Json(index::demo_sandbox(&name, &options.0, &review_id.0.unwrap_or(Value::Null)))
}
