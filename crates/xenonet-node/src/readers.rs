//! The session readers (codex, claude, git) and the projects and identities
//! built on them, for their twins: core/git.js, core/projects.js,
//! core/codex.js, core/claude.js and core/identities.js.

use crate::js::{Json, Jv, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::{LazyLock, Mutex};
use xenonet_sources::claude;
use xenonet_sources::codex;
use xenonet_sources::git;
use xenonet_sources::record::{Obj, F};
use xenonet_state::identities;
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

#[napi(js_name = "gitStatusIdleMs")]
pub fn git_status_idle_ms() -> f64 {
    git::STATUS_IDLE_MS
}

#[napi(js_name = "gitConfigTtlMs")]
pub fn git_config_ttl_ms() -> f64 {
    git::CONFIG_TTL_MS
}

/// { git, common }, or null.
#[napi(js_name = "gitDirs")]
pub fn git_dirs(dir: String) -> Json {
    Json(git::git_dirs(&dir).map_or(Value::Null, |(g, c)| json!({ "git": g, "common": c })))
}

/// { status, config }, or null.
#[napi(js_name = "gitRepoStamps")]
pub fn git_repo_stamps(dir: String, home: String, xdg: Option<String>) -> Json {
    Json(git::repo_stamps(&dir, &home, xdg.as_deref()).map_or(Value::Null, |s| json!({ "status": s.status, "config": s.config })))
}

// { stamp, at } as the watch keeps it: a stamp that isn't text is one
// that couldn't be told.
fn kept(v: &Value) -> Option<(&str, f64)> {
    Some((v.get("stamp")?.as_str()?, v.get("at")?.as_f64()?))
}

#[napi(js_name = "gitStatusDue")]
pub fn git_status_due(last: Json, stamp: Option<String>, now: f64) -> bool {
    git::status_due(kept(&last.0), stamp.as_deref(), now)
}

#[napi(js_name = "gitDriversFresh")]
pub fn git_drivers_fresh(cached: Json, stamp: Option<String>, now: f64) -> bool {
    git::drivers_fresh(kept(&cached.0), stamp.as_deref(), now)
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
    pub fn finish(&mut self, env: Env, sessions: Json, max_submodules: MaybeJson, now: f64) -> Result<Jv> {
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
pub fn project_relations(env: Env, projects: Json) -> Result<Jv> {
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
pub fn codex_sub_agent_of(env: Env, t: MaybeJson) -> Result<Jv> {
    match codex::sub_agent_of(t.0.as_ref()) {
        Ok((parent, nickname, kind)) => Ok(Jv::Obj(vec![("parent", Jv::Json(parent)), ("nickname", Jv::Json(nickname)), ("kind", Jv::Json(kind))])),
        Err(e) => thrown(env, e),
    }
}

#[napi(js_name = "codexThreadToSession")]
pub fn codex_thread_to_session(env: Env, t: MaybeJson, archived: bool) -> Result<Jv> {
    codex::thread_to_session(t.0.as_ref(), archived).map(obj).or_else(|e| thrown(env, e))
}

/// A page of the app-server's items, oldest first.
#[napi(js_name = "codexNormalizeItems")]
pub fn codex_normalize_items(env: Env, data: MaybeJson) -> Result<Jv> {
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
    pub fn new() -> Self {
        JsRolloutEntries { e: codex::RolloutEntries::default() }
    }

    /// A line's record, as lines.js read it.
    #[napi]
    pub fn add(&mut self, env: Env, raw: MaybeJson) -> Result<()> {
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
                codex::Message::Answer { id, result, error: None } => out.push(Jv::Obj(vec![("id", Jv::Json(id)), ("result", jv(*result))])),
                codex::Message::Answer { id, error: Some(e), .. } => {
                    let (message, rpc) = *e;
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
pub fn codex_native_projects(env: Env, rows: MaybeJson) -> Result<Jv> {
    codex::native_projects(rows.0.as_ref()).map(objs).or_else(|e| thrown(env, e))
}

// ---------- claude.js ----------

/// Claude desktop's session files under its claude-code-sessions folder:
/// [{ file, parts, data }], data undefined when it isn't JSON.
#[napi(js_name = "claudeDesktopFiles")]
pub fn claude_desktop_files(root: String) -> Jv {
    Jv::Arr(
        claude::desktop_files(&root)
            .into_iter()
            .map(|f| Jv::Obj(vec![("file", Jv::Json(Value::String(f.file))), ("parts", Jv::Json(json!(f.parts))), ("data", f.data.into())]))
            .collect(),
    )
}

/// From [[file, parts, record]…] (the records after the lenient reader):
/// { desktop: the records by CLI session, files: [[desktop id, file]…] }.
#[napi(js_name = "claudeDesktopIndex")]
pub fn claude_desktop_index(records: Json) -> Jv {
    let records: Vec<(String, Vec<String>, Value)> = match records.0 {
        Value::Array(rs) => rs
            .into_iter()
            .filter_map(|r| {
                let Value::Array(mut r) = r else { return None };
                let d = r.pop()?;
                let parts = r.pop().and_then(|p| p.as_array().map(|a| a.iter().map(xenonet_base::text::js_string).collect())).unwrap_or_default();
                let file = r.pop().map(|f| xenonet_base::text::js_string(&f)).unwrap_or_default();
                Some((file, parts, d))
            })
            .collect(),
        _ => Vec::new(),
    };
    let (desktop, files) = claude::desktop_index(records);
    Jv::Obj(vec![("desktop", Jv::Json(Value::Array(desktop))), ("files", Jv::Json(json!(files)))])
}

#[napi(js_name = "claudeSessions")]
pub fn claude_sessions(env: Env, listed: Json, desktop: Json) -> Result<Jv> {
    let listed = match listed.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let desktop = match desktop.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    claude::sessions(&listed, &desktop).map(objs).or_else(|e| thrown(env, e))
}

/// { start, end, cursor } of a page of `len` messages.
#[napi(js_name = "claudePage")]
pub fn claude_page(len: f64, cursor: MaybeJson, limit: f64) -> Jv {
    let (start, end, next) = claude::page(len as usize, cursor.0.as_ref(), limit);
    Jv::Obj(vec![("start", Jv::Num(start as f64)), ("end", Jv::Num(end as f64)), ("cursor", opt_s(next))])
}

#[napi(js_name = "claudeEntries")]
pub fn claude_entries(env: Env, messages: Json) -> Result<Jv> {
    let ms = match messages.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    claude::entries(&ms).map(objs).or_else(|e| thrown(env, e))
}

#[napi(js_name = "claudeNoRecord")]
pub fn claude_no_record() -> &'static str {
    claude::NO_RECORD
}

#[napi(js_name = "claudeOtherSession")]
pub fn claude_other_session() -> &'static str {
    claude::OTHER_SESSION
}

/// The record's text with the new title, or null when it's another
/// session's now.
#[napi(js_name = "claudeRenamedRecord")]
pub fn claude_renamed_record(record: MaybeJson, desktop_id: MaybeJson, cli_id: MaybeJson, title: MaybeJson) -> Option<String> {
    claude::renamed_record(record.0.unwrap_or(Value::Null), desktop_id.0.as_ref(), cli_id.0.as_ref(), title.0.as_ref())
}

// ---------- identities.js ----------

fn pairs(v: &Value) -> Vec<(String, String)> {
    let s = |x: Option<&Value>| x.map_or_else(String::new, xenonet_base::text::js_string);
    v.as_array().map(|a| a.iter().map(|p| (s(p.get(0)), s(p.get(1)))).collect()).unwrap_or_default()
}

fn value_pairs(v: Value) -> Vec<(Value, Value)> {
    match v {
        Value::Array(a) => a
            .into_iter()
            .map(|p| match p {
                Value::Array(mut kv) if !kv.is_empty() => {
                    let v = if kv.len() > 1 { kv.remove(1) } else { Value::Null };
                    (kv.remove(0), v)
                }
                _ => (Value::Null, Value::Null),
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[napi(js_name = "idClaudeAccount")]
pub fn id_claude_account(data: MaybeJson) -> Json {
    Json(identities::claude_account(data.0.as_ref()))
}

#[napi(js_name = "idTokenClaims")]
pub fn id_token_claims(auth: MaybeJson) -> Json {
    Json(identities::id_token_claims(auth.0.as_ref()))
}

#[napi(js_name = "idChatgptAccount")]
pub fn id_chatgpt_account(auth: MaybeJson, claims: MaybeJson) -> Json {
    Json(identities::chatgpt_account(auth.0.as_ref(), claims.0.as_ref()))
}

#[napi(js_name = "idParseGhHosts")]
pub fn id_parse_gh_hosts(text: String) -> Json {
    Json(Value::Array(identities::parse_gh_hosts(&text)))
}

#[napi(js_name = "idGhHostsShape")]
pub fn id_gh_hosts_shape(text: String) -> Json {
    Json(identities::gh_hosts_shape(&text))
}

/// [[alias, host]…]
#[napi(js_name = "idParseSshAliases")]
pub fn id_parse_ssh_aliases(text: String) -> Json {
    Json(json!(identities::parse_ssh_aliases(&text)))
}

#[napi(js_name = "idParseRemote")]
pub fn id_parse_remote(url: String, aliases: Json) -> Json {
    Json(identities::parse_remote(&url, &pairs(&aliases.0)).map_or(Value::Null, |r| r.to_json()))
}

#[napi(js_name = "idParseGitConfig")]
pub fn id_parse_git_config(out: String) -> Json {
    Json(identities::parse_git_config(&out))
}

#[napi(js_name = "idPushAccount")]
pub fn id_push_account(env: Env, url: MaybeJson, cfg: MaybeJson, aliases: Json) -> Result<Json> {
    identities::push_account(url.0.as_ref(), cfg.0.as_ref(), &pairs(&aliases.0)).map(Json).or_else(|e| thrown(env, e))
}

fn signin_op(op: &identities::Signin) -> Jv {
    match op {
        identities::Signin::Same => Jv::Obj(vec![]),
        identities::Signin::Extend(to) => Jv::Obj(vec![("extend", Jv::Num(*to))]),
        identities::Signin::Push { span, drop } => Jv::Obj(vec![("span", Jv::Json(span.clone())), ("drop", Jv::Num(*drop as f64))]),
    }
}

/// {} (nothing changed), { extend: to }, or { span, drop }.
#[napi(js_name = "idNoteSignin")]
pub fn id_note_signin(spans: Json, id: MaybeJson, now: f64, since: MaybeJson) -> Jv {
    let spans = match spans.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    signin_op(&identities::note_signin(&spans, id.0.as_ref().unwrap_or(&Value::Null), now, since.0.as_ref()))
}

#[napi(js_name = "idAccountAt")]
pub fn id_account_at(spans: Json, current: MaybeJson, t: MaybeJson, known: Json) -> Json {
    let spans = match spans.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let known = match known.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    Json(identities::account_at(&spans, current.0.as_ref(), t.0.as_ref(), &known))
}

#[napi(js_name = "idEmptySaved")]
pub fn id_empty_saved() -> Json {
    Json(identities::empty_saved())
}

#[napi(js_name = "idGithubId")]
pub fn id_github_id(host: String, login: String) -> String {
    identities::github_id(&host, &login)
}

#[napi(js_name = "idGitId")]
pub fn id_git_id(email: String) -> String {
    identities::git_id(&email)
}

#[napi(js_name = "idClaudeId")]
pub fn id_claude_id(account: String, org: String) -> String {
    identities::claude_id(&account, &org)
}

/// From { nodes, links, accounts, ssh, repos, owners, saved, home, now }:
/// { ranAs, links, identities, signins, known, dirty }.
#[napi(js_name = "idLinkIdentities")]
pub fn id_link_identities(env: Env, input: Json) -> Result<Jv> {
    let mut m = match input.0 {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    let mut take = |k: &str| m.remove(k).unwrap_or(Value::Null);
    let nodes = match take("nodes") {
        Value::Array(a) => a,
        _ => Vec::new(),
    };
    let links: Vec<(Value, Value, Value)> = match take("links") {
        Value::Array(a) => a
            .into_iter()
            .map(|l| {
                let at = |i: usize| l.get(i).cloned().unwrap_or(Value::Null);
                (at(0), at(1), at(2))
            })
            .collect(),
        _ => Vec::new(),
    };
    let accounts = take("accounts");
    let ssh = pairs(&take("ssh"));
    let repos = value_pairs(take("repos"));
    let owners = value_pairs(take("owners"));
    let saved = take("saved");
    let home = take("home");
    let now = xenonet_base::text::js_number(Some(&take("now")));
    let home = home.as_str().filter(|h| !h.is_empty());
    let out = identities::link_identities(identities::LinkInput {
        nodes: &nodes,
        links: &links,
        accounts: &accounts,
        ssh: &ssh,
        repos: &repos,
        owners: &owners,
        saved: &saved,
        home,
        now,
    });
    let out = match out {
        Ok(o) => o,
        Err(e) => return thrown(env, e),
    };
    let indexed = |xs: Vec<(usize, Value)>| Jv::Arr(xs.into_iter().map(|(i, v)| Jv::Arr(vec![Jv::Num(i as f64), Jv::Json(v)])).collect());
    Ok(Jv::Obj(vec![
        ("ranAs", indexed(out.ran_as)),
        ("links", indexed(out.identities)),
        ("identities", Jv::Json(Value::Array(out.list))),
        ("signins", Jv::Arr(out.signins.iter().map(|(app, op)| Jv::Arr(vec![Jv::Json(Value::String((*app).into())), signin_op(op)])).collect())),
        ("known", Jv::Arr(out.known.into_iter().map(|(id, whole, v)| Jv::Arr(vec![Jv::Json(Value::String(id)), Jv::Bool(whole), Jv::Json(v)])).collect())),
        ("dirty", Jv::Bool(out.dirty)),
    ]))
}

#[napi(js_name = "idRepoConfigMtime")]
pub fn id_repo_config_mtime(dir: String) -> Option<f64> {
    identities::repo_config_mtime(&dir)
}

#[napi(js_name = "idGlobalStamp")]
pub fn id_global_stamp(home: String, xdg_config_home: Option<String>) -> String {
    identities::global_stamp(&home, xdg_config_home.as_deref())
}

#[napi(js_name = "idUnreadableConfig")]
pub fn id_unreadable_config(path: String, home: String) -> String {
    identities::unreadable_config(&path, &home)
}

/// { say: "ok" | "problem" | "", problem }
#[napi(js_name = "idReport")]
pub fn id_report(file: Option<Vec<String>>, repo: Option<String>, home: String, state: String, last_error: Option<String>) -> Jv {
    let file = file.as_ref().map(|f| (f.first().map_or("", String::as_str), f.get(1).map_or("", String::as_str)));
    let (say, problem) = identities::report(file, repo.as_deref(), &home, &state, last_error.as_deref());
    Jv::Obj(vec![("say", Jv::Json(Value::String(say.into()))), ("problem", opt_s(problem))])
}

/// The environment git config runs in away from your home folder, or null.
#[napi(js_name = "idGitEnv")]
pub fn id_git_env(env: Json, home: String, your_home: String) -> Json {
    let env = env.0.as_object().cloned().unwrap_or_default();
    Json(identities::git_env(&env, &home, &your_home).map_or(Value::Null, Value::Object))
}

/// Where each Claude Code transcript is, and the account a Remote Control
/// session names in its first lines.
#[napi(js_name = "TranscriptOwners")]
pub struct JsTranscriptOwners {
    o: identities::TranscriptOwners,
}

#[napi]
impl JsTranscriptOwners {
    #[napi(constructor)]
    pub fn new(dir: String) -> Self {
        JsTranscriptOwners { o: identities::TranscriptOwners::new(&dir) }
    }

    /// { account, org }, or null.
    #[napi]
    pub fn owner(&mut self, id: String, now: f64) -> Result<Json> {
        self.o.owner(&id, now).map(Json).map_err(|e| Error::new(Status::GenericFailure, e.message))
    }
}
