//! The session readers (codex, claude, git) and the projects and identities
//! built on them, for their twins: core/git.js, core/projects.js,
//! core/codex.js, core/claude.js and core/identities.js.

use crate::js::{Json, Jv, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};
use xenonet_sources::git;
use xenonet_state::projects::{self, FactsCache, Scan};

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

// ---------- projects.js ----------

// What the bottom-up pass has seen, for a while (forgetScans clears it).
static FACTS: LazyLock<Mutex<FactsCache>> = LazyLock::new(|| Mutex::new(FactsCache::default()));

#[napi(js_name = "forgetProjectScans")]
pub fn forget_project_scans() {
    if let Ok(mut c) = FACTS.lock() {
        c.clear();
    }
}

#[napi(js_name = "projectVcsMarkers")]
pub fn project_vcs_markers() -> Vec<&'static str> {
    projects::VCS_MARKERS.to_vec()
}

#[napi(js_name = "projectScanLimits")]
pub fn project_scan_limits() -> Jv {
    Jv::Obj(vec![("depth", Jv::Num(projects::SCAN_DEPTH)), ("maxDirs", Jv::Num(projects::SCAN_MAX_DIRS)), ("maxSubmodules", Jv::Num(projects::SCAN_MAX_SUBMODULES))])
}

/// One scan of a projects folder: the walk hands it each folder read
/// (look) and asks it about each subfolder (enter); finish does the rest.
#[napi(js_name = "ProjectScan")]
pub struct JsProjectScan {
    scan: Scan,
}

#[napi]
impl JsProjectScan {
    #[napi(constructor)]
    pub fn new(root: String, rules: Json) -> Self {
        JsProjectScan { scan: Scan::new(&root, &rules.0) }
    }

    #[napi(getter, js_name = "rootRead")]
    pub fn root_read(&self) -> bool {
        self.scan.root_read
    }

    #[napi]
    pub fn look(&mut self, dir: String, names: Vec<String>, dirs: Vec<bool>, level: f64, aborted: bool) {
        let ents: Vec<(String, bool)> = names.into_iter().zip(dirs).collect();
        self.scan.look(&dir, &ents, level, aborted);
    }

    #[napi]
    pub fn enter(&self, name: String, dir: String, level: f64) -> bool {
        self.scan.enter(&name, &dir, level)
    }

    /// { projects, folders, groups, count }.
    #[napi]
    pub fn finish(&mut self, env: &Env, sessions: Json, max_submodules: MaybeJson, now: f64) -> Result<Jv> {
        let sessions = match sessions.0 {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        let mut cache = FACTS.lock().map_err(|_| Error::new(Status::GenericFailure, "the projects' cache is poisoned".to_string()))?;
        match self.scan.finish(&sessions, max_submodules.0.as_ref(), &mut cache, now) {
            Ok(f) => Ok(Jv::Obj(vec![
                ("projects", Jv::Json(Value::Array(f.projects))),
                ("folders", Jv::Json(Value::Array(f.folders))),
                ("groups", Jv::Num(f.groups as f64)),
                ("count", Jv::Num(f.count as f64)),
            ])),
            Err(e) => thrown(env, e),
        }
    }
}

/// { parentOf: [[key, parent]…], sameRemote: [[a, b]…] }.
#[napi(js_name = "projectRelations")]
pub fn project_relations(env: &Env, projects: Json) -> Result<Jv> {
    let Value::Array(ps) = projects.0 else {
        return thrown(env, xenonet_sources::lines::TypeError("projects.filter is not a function".into()));
    };
    match projects::relations(&ps) {
        Ok((parents, same)) => Ok(Jv::Obj(vec![("parentOf", Jv::Json(json!(parents))), ("sameRemote", Jv::Json(json!(same)))])),
        Err(e) => thrown(env, e),
    }
}
