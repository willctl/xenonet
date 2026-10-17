//! The session readers (codex, claude, git) and the projects and identities
//! built on them, for their twins: core/git.js, core/projects.js,
//! core/codex.js, core/claude.js and core/identities.js.

use crate::js::Jv;
use napi_derive::napi;
use serde_json::{json, Value};
use xenonet_sources::git;

fn opt_s(v: Option<String>) -> Jv {
    Jv::Json(v.map_or(Value::Null, Value::String))
}

// ---------- git.js ----------

#[napi(js_name = "gitParseStatus")]
pub fn git_parse_status(out: String) -> Jv {
    let st = git::parse_status(&out);
    Jv::Obj(vec![
        ("branch", opt_s(st.branch)),
        ("ahead", Jv::Num(st.ahead)),
        ("behind", Jv::Num(st.behind)),
        ("staged", Jv::Num(st.staged)),
        ("modified", Jv::Num(st.modified)),
        ("untracked", Jv::Num(st.untracked)),
        ("dirty", Jv::Num(st.dirty)),
    ])
}

/// The -c settings that empty the repo's own drivers, or null.
#[napi(js_name = "gitFilterOverrides")]
pub fn git_filter_overrides(listing: String, scoped: bool) -> Option<Vec<String>> {
    git::filter_overrides(&listing, scoped)
}

#[napi(js_name = "gitDriversArgs")]
pub fn git_drivers_args(scoped: bool) -> Vec<&'static str> {
    git::drivers_args(scoped)
}

/// What comes after git config listed the drivers: { unscoped: true },
/// { args } for the status, or {} for no status.
#[napi(js_name = "gitAfterDrivers")]
pub fn git_after_drivers(code: Option<f64>, err: bool, stdout: String, scoped: bool) -> Jv {
    match git::after_drivers(code, err, &stdout, scoped) {
        git::AfterDrivers::Unscoped => Jv::Obj(vec![("unscoped", Jv::Bool(true))]),
        git::AfterDrivers::Nothing => Jv::Obj(vec![]),
        git::AfterDrivers::Status(args) => Jv::Obj(vec![("args", Jv::Json(json!(args)))]),
    }
}

#[napi(js_name = "gitNeedsDevTools")]
pub fn git_needs_dev_tools(platform: String, git: Option<String>) -> bool {
    git::needs_dev_tools(&platform, git.as_deref())
}

#[napi(js_name = "gitNoDevTools")]
pub fn git_no_dev_tools() -> &'static str {
    git::NO_DEV_TOOLS
}

#[napi(js_name = "gitUnreadable")]
pub fn git_unreadable(path: String) -> String {
    git::unreadable(&path)
}

#[napi(js_name = "gitProblem")]
pub fn git_problem(failed: Vec<String>) -> Option<String> {
    git::problem_of(&failed)
}
