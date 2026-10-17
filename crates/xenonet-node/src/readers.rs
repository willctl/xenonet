//! The session readers (codex, claude, git) and the projects and identities
//! built on them, for their twins: core/git.js, core/projects.js,
//! core/codex.js, core/claude.js and core/identities.js.

use crate::js::{Json, Jv, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};
use xenonet_sources::codex::{self, Obj, F};
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

// ---------- codex.js ----------

pub(crate) fn jv(f: F) -> Jv {
    match f {
        F::Undef => Jv::Undef,
        F::Num(n) => Jv::Num(n),
        F::V(v) => Jv::Json(v),
        F::Arr(a) => Jv::Arr(a.into_iter().map(jv).collect()),
    }
}

pub(crate) fn obj(o: Obj) -> Jv {
    Jv::Obj(o.into_iter().map(|(k, f)| (k, jv(f))).collect())
}

pub(crate) fn objs(os: Vec<Obj>) -> Jv {
    Jv::Arr(os.into_iter().map(obj).collect())
}

#[napi(js_name = "codexSourceKinds")]
pub fn codex_source_kinds() -> Vec<&'static str> {
    codex::SOURCE_KINDS.to_vec()
}

#[napi(js_name = "codexStateDb")]
pub fn codex_state_db() -> u32 {
    codex::STATE_DB
}

#[napi(js_name = "codexRequestTimeoutMs")]
pub fn codex_request_timeout_ms() -> f64 {
    codex::REQUEST_TIMEOUT_MS
}

/// { parent, nickname, kind }
#[napi(js_name = "codexSubAgentOf")]
pub fn codex_sub_agent_of(env: &Env, t: MaybeJson) -> Result<Jv> {
    match codex::sub_agent_of(t.0.as_ref()) {
        Ok((parent, nickname, kind)) => Ok(Jv::Obj(vec![("parent", Jv::Json(parent)), ("nickname", Jv::Json(nickname)), ("kind", Jv::Json(kind))])),
        Err(e) => thrown(env, e),
    }
}

#[napi(js_name = "codexThreadToSession")]
pub fn codex_thread_to_session(env: &Env, t: MaybeJson, archived: bool) -> Result<Jv> {
    codex::thread_to_session(t.0.as_ref(), archived).map(obj).or_else(|e| thrown(env, e))
}

/// A page of the app-server's items, oldest first.
#[napi(js_name = "codexNormalizeItems")]
pub fn codex_normalize_items(env: &Env, data: MaybeJson) -> Result<Jv> {
    codex::normalize_items(data.0.as_ref()).map(objs).or_else(|e| thrown(env, e))
}

#[napi(js_name = "codexSaysUnsupported")]
pub fn codex_says_unsupported(rpc: MaybeJson) -> bool {
    codex::says_unsupported(rpc.0.as_ref())
}

/// A thread's items from its rollout, as its lines are read.
#[napi(js_name = "RolloutEntries")]
pub struct JsRolloutEntries {
    e: codex::RolloutEntries,
}

#[napi]
impl JsRolloutEntries {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsRolloutEntries { e: codex::RolloutEntries::default() }
    }

    /// A line's record, as lines.js read it.
    #[napi]
    pub fn add(&mut self, env: &Env, raw: MaybeJson) -> Result<()> {
        self.e.add(raw.0.unwrap_or(Value::Null)).or_else(|e| thrown(env, e))
    }

    /// { entries, cursor, total }: newest first, `cursor` continuing
    /// further back.
    #[napi]
    pub fn page(&self, cursor: MaybeJson, limit: f64) -> Jv {
        let all = self.e.list();
        let (a, b, next) = codex::rollout_page(all.len(), cursor.0.as_ref(), limit);
        Jv::Obj(vec![("entries", objs(all[a..b].to_vec())), ("cursor", opt_s(next)), ("total", Jv::Num(all.len() as f64))])
    }
}

/// One app-server's stdout, read into its messages.
#[napi(js_name = "CodexStdout")]
pub struct JsCodexStdout {
    io: codex::Stdout,
}

#[napi]
impl JsCodexStdout {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsCodexStdout { io: codex::Stdout::default() }
    }

    /// The messages a chunk completes: { decline } (a line to write back),
    /// { id, result }, or { id, error: { message, rpc } }.
    #[napi]
    pub fn take(&mut self, chunk: String) -> Jv {
        let mut out = Vec::new();
        for m in self.io.take(&chunk) {
            match m {
                codex::Message::Decline(line) => out.push(Jv::Obj(vec![("decline", Jv::Json(Value::String(line)))])),
                codex::Message::Answer { id, result, error: None } => out.push(Jv::Obj(vec![("id", Jv::Json(id)), ("result", jv(result))])),
                codex::Message::Answer { id, error: Some((message, rpc)), .. } => {
                    let error = Jv::Obj(vec![("message", Jv::Json(Value::String(message))), ("rpc", Jv::Json(rpc))]);
                    out.push(Jv::Obj(vec![("id", Jv::Json(id)), ("error", error)]));
                }
                codex::Message::Other => {}
            }
        }
        Jv::Arr(out)
    }
}

/// The last lines a client's app-servers wrote to stderr.
#[napi(js_name = "CodexStderr")]
pub struct JsCodexStderr {
    io: codex::Stderr,
}

#[napi]
impl JsCodexStderr {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsCodexStderr { io: codex::Stderr::default() }
    }

    /// Keeps a chunk's lines, and gives back the last 20.
    #[napi]
    pub fn push(&mut self, chunk: String) -> Vec<String> {
        self.io.push(&chunk);
        self.io.lines.clone()
    }

    #[napi]
    pub fn exited(&self, code: MaybeJson) -> String {
        self.io.exited(code.0.as_ref())
    }
}

/// The numbers of the other state databases in Codex's folder.
#[napi(js_name = "codexOtherStateDbs")]
pub fn codex_other_state_dbs(dir: String) -> Vec<String> {
    let names: Vec<String> =
        xenonet_base::paths::read_dir_sorted(std::path::Path::new(&dir)).map(|es| es.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    codex::other_state_dbs(&names)
}

fn cells(rows: Vec<Vec<(&'static str, codex::Cell)>>) -> Jv {
    let cell = |c: codex::Cell| match c {
        codex::Cell::V(v) => Jv::Json(v),
        codex::Cell::Bytes(b) => Jv::Bytes(b),
    };
    Jv::Arr(rows.into_iter().map(|r| Jv::Obj(r.into_iter().map(|(k, c)| (k, cell(c))).collect())).collect())
}

/// null (no database), { rows: { projects, project_roots } }, or
/// { missing: [...] } when a query failed.
#[napi(js_name = "codexReadStateDb")]
pub fn codex_read_state_db(file: String) -> Jv {
    match codex::read_state_db(&file) {
        None => Jv::Json(Value::Null),
        Some(codex::StateRows::Rows { projects, roots }) => Jv::Obj(vec![("rows", Jv::Obj(vec![("projects", cells(projects)), ("project_roots", cells(roots))]))]),
        Some(codex::StateRows::Missing(m)) => Jv::Obj(vec![("missing", Jv::Json(json!(m)))]),
    }
}

#[napi(js_name = "codexNativeProjects")]
pub fn codex_native_projects(env: &Env, rows: MaybeJson) -> Result<Jv> {
    codex::native_projects(rows.0.as_ref()).map(objs).or_else(|e| thrown(env, e))
}
