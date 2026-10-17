//! xenonet-sources' native readers, numbat and reported runs, for their
//! twins (js/core/native/*.js, js/core/numbat.js, js/core/reported-runs.js).
//! The readers' cores live here as classes; what a twin passes in is called
//! back during the call: vendor.js's checks, a test's own `files`, a
//! node:sqlite handle a test opened. Those come guarded, so they never
//! throw into Rust: a check that threw returns `failed`, and `files` and a
//! handle answer [value] or { code, message }.

use crate::js::{from_js, to_js, Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use xenonet_base::fs::FsError;
use xenonet_sources::native::files::{Entry, Files, Stat, StdFiles};
use xenonet_sources::native::opencode::{self, Db, OpenCode, Sqlite, Vendor};
use xenonet_sources::native::{copilot_cli, cursor, gemini_cli, Checked, Out, Threw};
use xenonet_sources::{numbat, reported_runs};

fn jv(o: Out) -> Jv {
    match o {
        Out::Undef => Jv::Undef,
        Out::Num(n) => Jv::Num(n),
        Out::Json(v) => Jv::Json(v),
        Out::Obj(fields) => Jv::Obj(fields.into_iter().map(|(k, v)| (k, jv(v))).collect()),
        Out::Arr(items) => Jv::Arr(items.into_iter().map(jv).collect()),
    }
}

fn list(items: Vec<Out>) -> Jv {
    Jv::Arr(items.into_iter().map(jv).collect())
}

fn same(env: &Env, a: &Unknown, b: &Unknown) -> bool {
    let mut is = false;
    unsafe { sys::napi_strict_equals(env.raw(), a.raw(), b.raw(), &mut is) };
    is
}

// ---------- what the twin passes ----------

/// A check the twin guarded: (record, version) → what vendor.js's reader
/// gave back, or `failed` when it threw.
type CheckFn<'a> = Function<'a, FnArgs<(Unknown<'a>, Option<String>)>, Unknown<'a>>;

fn checked(env: &Env, check: &CheckFn, failed: &Unknown, record: &Value, version: Option<&str>) -> std::result::Result<Checked, Threw> {
    let raw = unsafe { to_js(env.raw(), record, 0) }.map_err(|_| Threw)?;
    let arg = unsafe { Unknown::from_raw_unchecked(env.raw(), raw) };
    let out = check.call(FnArgs::from((arg, version.map(str::to_string)))).map_err(|_| Threw)?;
    if same(env, &out, failed) {
        return Err(Threw);
    }
    if same(env, &out, &arg) {
        return Ok(Checked::Same);
    }
    if matches!(out.get_type(), Ok(ValueType::Null | ValueType::Undefined)) {
        return Ok(Checked::Null);
    }
    Ok(Checked::Other(unsafe { from_js(env.raw(), out.raw(), 0) }.map_err(|_| Threw)?))
}

/// A test's own `files`, through the twin: io(op, path, a, b) answers
/// [value], or { code, message } for what the call threw.
type IoFn<'a> = Function<'a, FnArgs<(String, String, f64, f64)>, Json>;

struct JsFiles<'a> {
    io: &'a IoFn<'a>,
}

impl JsFiles<'_> {
    fn call(&self, op: &str, path: &str, a: f64, b: f64) -> std::result::Result<Value, FsError> {
        match self.io.call(FnArgs::from((op.to_string(), path.to_string(), a, b))) {
            Ok(Json(Value::Array(mut v))) if v.len() == 1 => Ok(v.remove(0)),
            Ok(Json(e)) => {
                Err(FsError { code: e.get("code").and_then(Value::as_str).map(Into::into), message: e.get("message").and_then(Value::as_str).unwrap_or_default().to_string() })
            }
            Err(e) => Err(FsError { code: None, message: e.reason.clone() }),
        }
    }
}

impl Files for JsFiles<'_> {
    fn read_dir(&mut self, dir: &str) -> std::result::Result<Vec<Entry>, FsError> {
        let v = self.call("readdir", dir, 0.0, 0.0)?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .map(|e| Entry { name: e[0].as_str().unwrap_or_default().to_string(), is_dir: e[1].as_bool().unwrap_or(false), is_file: e[2].as_bool().unwrap_or(false) })
                    .collect()
            })
            .unwrap_or_default())
    }
    fn stat(&mut self, file: &str) -> std::result::Result<Stat, FsError> {
        let v = self.call("stat", file, 0.0, 0.0)?;
        let num = |k: &str| xenonet_base::text::js_number(v.get(k));
        let ino = v.get("ino").filter(|x| !x.is_null()).map(|x| xenonet_base::text::js_number(Some(x)));
        Ok(Stat { mtime_ms: num("mtimeMs"), size: num("size"), ino })
    }
    fn read_text(&mut self, file: &str) -> std::result::Result<String, FsError> {
        match self.call("text", file, 0.0, 0.0)? {
            Value::String(s) => Ok(s),
            _ => Err(FsError { code: None, message: "not text".into() }),
        }
    }
    fn read_at(&mut self, file: &str, offset: f64, len: usize) -> std::result::Result<Vec<u8>, FsError> {
        // The bytes come as latin1 text: one character a byte.
        match self.call("read", file, offset, len as f64)? {
            Value::String(s) => Ok(s.chars().map(|c| c as u32 as u8).collect()),
            _ => Err(FsError { code: None, message: "not bytes".into() }),
        }
    }
    fn exists(&mut self, file: &str) -> bool {
        self.call("exists", file, 0.0, 0.0).is_ok_and(|v| v.as_bool() == Some(true))
    }
}

fn files_of<'a>(io: &'a Option<IoFn<'a>>) -> Box<dyn Files + 'a> {
    match io {
        Some(io) => Box::new(JsFiles { io }),
        None => Box::new(StdFiles),
    }
}

// ---------- core/native/copilot-cli.js ----------

#[napi(js_name = "copilotSessionIdOf")]
pub fn copilot_session_id_of(id: String) -> Option<String> {
    copilot_cli::session_id_of(&id)
}

#[napi(js_name = "copilotEditedBy")]
pub fn copilot_edited_by(tool: MaybeJson, args: MaybeJson) -> Vec<String> {
    copilot_cli::edited_by(tool.0.as_ref(), args.0.as_ref())
}

/// The log, null, or `failed` when the check threw.
#[napi(js_name = "copilotReadEvents")]
pub fn copilot_read_events(env: &Env, text: String, check: CheckFn, failed: Unknown) -> Jv {
    match copilot_cli::read_events(&text, &mut |r| checked(env, &check, &failed, r, None)) {
        Ok(Some(l)) => jv(copilot_cli::log_out(&l)),
        Ok(None) => Jv::Json(Value::Null),
        Err(Threw) => Jv::Raw(failed.raw()),
    }
}

#[napi(js_name = "copilotRoot")]
pub fn copilot_root(home: String) -> String {
    copilot_cli::copilot_root(&home)
}

#[napi(js_name = "CopilotCliCore")]
pub struct JsCopilotCli {
    c: copilot_cli::CopilotCli,
}

#[napi]
impl JsCopilotCli {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsCopilotCli { c: copilot_cli::CopilotCli::new() }
    }
    #[napi]
    pub fn read(&mut self, env: &Env, root: Option<String>, now: f64, check: CheckFn, failed: Unknown, io: Option<IoFn>) {
        let mut files = files_of(&io);
        self.c.read(root.as_deref(), now, files.as_mut(), &mut |r, v| checked(env, &check, &failed, r, v));
    }
    #[napi(getter, js_name = "rootState")]
    pub fn root_state(&self) -> &'static str {
        self.c.root_state
    }
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.c.runs.len() as u32
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.c.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn sessions(&self) -> Jv {
        list(self.c.sessions())
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.c.runs_out())
    }
}

// ---------- core/native/gemini-cli.js ----------

#[napi(js_name = "geminiSessionIdOf")]
pub fn gemini_session_id_of(id: String) -> Option<String> {
    gemini_cli::session_id_of(&id)
}

/// The chat, null, or `failed` when the check threw.
#[napi(js_name = "geminiReadChat")]
pub fn gemini_read_chat(env: &Env, text: String, check: CheckFn, failed: Unknown) -> Jv {
    match gemini_cli::read_chat(&text, &mut |r| checked(env, &check, &failed, r, None)) {
        Ok(Some(c)) => jv(gemini_cli::chat_out(&c)),
        Ok(None) => Jv::Json(Value::Null),
        Err(Threw) => Jv::Raw(failed.raw()),
    }
}

#[napi(js_name = "geminiRoot")]
pub fn gemini_root(cli_home: Option<String>, home: String) -> String {
    gemini_cli::gemini_root(cli_home.as_deref(), &home)
}

#[napi(js_name = "GeminiCliCore")]
pub struct JsGeminiCli {
    g: gemini_cli::GeminiCli,
}

#[napi]
impl JsGeminiCli {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsGeminiCli { g: gemini_cli::GeminiCli::new() }
    }
    #[napi]
    pub fn read(&mut self, env: &Env, root: Option<String>, now: f64, check: CheckFn, failed: Unknown, io: Option<IoFn>) {
        let mut files = files_of(&io);
        self.g.read(root.as_deref(), now, files.as_mut(), &mut |r, v| checked(env, &check, &failed, r, v));
    }
    #[napi(getter, js_name = "rootState")]
    pub fn root_state(&self) -> &'static str {
        self.g.root_state
    }
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.g.runs.len() as u32
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.g.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn sessions(&self) -> Jv {
        list(self.g.sessions())
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.g.runs_out())
    }
}

// ---------- core/native/cursor.js ----------

#[napi(js_name = "cursorSessionIdOf")]
pub fn cursor_session_id_of(id: String) -> Option<String> {
    cursor::session_id_of(&id)
}

#[napi(js_name = "cursorEventsFile")]
pub fn cursor_events_file(named: Option<String>, state_dir: String) -> String {
    cursor::cursor_events_file(named.as_deref(), &state_dir)
}

#[napi(js_name = "CursorCore")]
pub struct JsCursor {
    c: cursor::Cursor,
}

#[napi]
impl JsCursor {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsCursor { c: cursor::Cursor::new() }
    }
    /// True when the vendor check threw (the twin has what it threw).
    #[napi]
    pub fn read(&mut self, env: &Env, file: Option<String>, now: f64, check: CheckFn, failed: Unknown, io: Option<IoFn>) -> bool {
        let mut files = files_of(&io);
        self.c.read(file.as_deref(), now, files.as_mut(), &mut |r, v| checked(env, &check, &failed, r, v)).is_err()
    }
    #[napi(getter, js_name = "fileState")]
    pub fn file_state(&self) -> &'static str {
        self.c.file_state
    }
    #[napi(getter)]
    pub fn seen(&self) -> Option<String> {
        self.c.seen.clone()
    }
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.c.runs.len() as u32
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.c.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn sessions(&self) -> Jv {
        list(self.c.sessions())
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.c.runs_out())
    }
}

// ---------- core/native/opencode.js ----------

#[napi(js_name = "opencodeLocalId")]
pub fn opencode_local_id(id: String) -> String {
    opencode::local_id(&id)
}

#[napi(js_name = "opencodeSessionIdOf")]
pub fn opencode_session_id_of(id: String) -> String {
    opencode::session_id_of(&id)
}

#[napi(js_name = "opencodePatchFiles")]
pub fn opencode_patch_files(text: MaybeJson) -> Vec<String> {
    opencode::patch_files(text.0.as_ref())
}

#[napi(js_name = "opencodeDbPath")]
pub fn opencode_db_path(home: String, xdg_data_home: Option<String>, named: Option<String>) -> String {
    opencode::opencode_db_path(&home, xdg_data_home.as_deref(), named.as_deref())
}

/// A database handle a test opened, through the twin: query(kind, sql,
/// param) for "open", "all", "get" and "close".
type QueryFn<'a> = Function<'a, FnArgs<(String, String, Option<f64>)>, Json>;

struct JsDb<'a> {
    query: &'a QueryFn<'a>,
}

impl JsDb<'_> {
    fn call(&self, kind: &str, sql: &str, param: Option<f64>) -> std::result::Result<Value, String> {
        match self.query.call(FnArgs::from((kind.to_string(), sql.to_string(), param))) {
            Ok(Json(Value::Array(mut v))) if v.len() == 1 => Ok(v.remove(0)),
            Ok(Json(e)) => Err(e.get("message").and_then(Value::as_str).unwrap_or_default().to_string()),
            Err(e) => Err(e.reason.clone()),
        }
    }
}

impl Db for JsDb<'_> {
    fn all(&mut self, sql: &str, param: Option<f64>) -> std::result::Result<Vec<Value>, String> {
        match self.call("all", sql, param)? {
            Value::Array(rows) => Ok(rows),
            _ => Err("not rows".into()),
        }
    }
    fn get(&mut self, sql: &str) -> std::result::Result<Option<Value>, String> {
        Ok(Some(self.call("get", sql, None)?).filter(|v| !v.is_null()))
    }
    fn close(&mut self) {
        let _ = self.call("close", "", None);
    }
}

/// vendor.js's reader, through the twin: ready(), missing(path, version)
/// and check(record, version), each giving `failed` when it threw.
struct JsVendor<'a> {
    env: &'a Env,
    ready: &'a Function<'a, (), Unknown<'a>>,
    missing: &'a Function<'a, FnArgs<(String, Option<String>)>, Unknown<'a>>,
    check: &'a CheckFn<'a>,
    failed: &'a Unknown<'a>,
}

impl Vendor for JsVendor<'_> {
    fn ready(&mut self) -> std::result::Result<(), Threw> {
        let v = self.ready.call(()).map_err(|_| Threw)?;
        if same(self.env, &v, self.failed) {
            Err(Threw)
        } else {
            Ok(())
        }
    }
    fn missing(&mut self, path: &str, version: Option<&str>) -> std::result::Result<(), Threw> {
        let v = self.missing.call(FnArgs::from((path.to_string(), version.map(str::to_string)))).map_err(|_| Threw)?;
        if same(self.env, &v, self.failed) {
            Err(Threw)
        } else {
            Ok(())
        }
    }
    fn check(&mut self, record: &Value, version: Option<&str>) -> std::result::Result<Checked, Threw> {
        checked(self.env, self.check, self.failed, record, version)
    }
}

#[allow(clippy::too_many_arguments)]
fn read_db(
    env: &Env,
    exists: bool,
    file: &str,
    since_ms: f64,
    edits_since_ms: f64,
    query: &Option<QueryFn>,
    ready: &Function<(), Unknown>,
    missing: &Function<FnArgs<(String, Option<String>)>, Unknown>,
    check: &CheckFn,
    failed: &Unknown,
) -> std::result::Result<opencode::Read, Threw> {
    let mut vendor = JsVendor { env, ready, missing, check, failed };
    let mut open = || -> std::result::Result<Box<dyn Db + '_>, String> {
        match query {
            Some(q) => {
                let db = JsDb { query: q };
                db.call("open", file, None)?;
                Ok(Box::new(db))
            }
            None => Ok(Box::new(Sqlite::open(file)?)),
        }
    };
    opencode::read_db(exists, &mut open, &mut vendor, since_ms, edits_since_ms)
}

/// readDb's answer, or true when getting the vendor reader threw.
#[allow(clippy::too_many_arguments)]
#[napi(js_name = "opencodeReadDb")]
pub fn opencode_read_db(
    env: &Env,
    exists: bool,
    file: String,
    since_ms: f64,
    edits_since_ms: f64,
    query: Option<QueryFn>,
    ready: Function<(), Unknown>,
    missing: Function<FnArgs<(String, Option<String>)>, Unknown>,
    check: CheckFn,
    failed: Unknown,
) -> Jv {
    match read_db(env, exists, &file, since_ms, edits_since_ms, &query, &ready, &missing, &check, &failed) {
        Ok(r) => jv(r.out()),
        Err(Threw) => Jv::Bool(true),
    }
}

#[napi(js_name = "OpenCodeCore")]
pub struct JsOpenCode {
    o: OpenCode,
}

#[napi]
impl JsOpenCode {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsOpenCode { o: OpenCode::new() }
    }
    /// undefined, true when getting the vendor reader threw, or a
    /// TypeError's message.
    #[allow(clippy::too_many_arguments)]
    #[napi]
    pub fn read(
        &mut self,
        env: &Env,
        now: f64,
        exists: bool,
        file: String,
        query: Option<QueryFn>,
        ready: Function<(), Unknown>,
        missing: Function<FnArgs<(String, Option<String>)>, Unknown>,
        check: CheckFn,
        failed: Unknown,
    ) -> Jv {
        const DAY_MS: f64 = 24.0 * 3600e3;
        const EDIT_KEEP_MS: f64 = 30.0 * 60e3;
        match read_db(env, exists, &file, now - DAY_MS, now - EDIT_KEEP_MS, &query, &ready, &missing, &check, &failed) {
            Err(Threw) => Jv::Bool(true),
            Ok(got) => match self.o.take(&got, now) {
                Ok(()) => Jv::Undef,
                Err(m) => Jv::Json(Value::String(m)),
            },
        }
    }
    #[napi(getter, js_name = "fileState")]
    pub fn file_state(&self) -> &'static str {
        self.o.file_state
    }
    #[napi(getter)]
    pub fn version(&self) -> Option<String> {
        self.o.version.clone()
    }
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.o.runs.len() as u32
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.o.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn sessions(&self) -> Jv {
        list(self.o.sessions())
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.o.runs_out())
    }
}

// ---------- core/numbat.js ----------

#[napi(js_name = "numbatRunId")]
pub fn numbat_run_id(sid: String) -> String {
    numbat::run_id(&sid)
}

#[napi(js_name = "numbatSessionIdOf")]
pub fn numbat_session_id_of(app: String, sid: String) -> String {
    numbat::session_id_of(&app, &sid)
}

/// { ok: true, event } or { ok: false, why }.
#[napi(js_name = "numbatReadRecord")]
pub fn numbat_read_record(line: String) -> Jv {
    match numbat::read_record(&line) {
        Ok(e) => Jv::Obj(vec![("ok", Jv::Bool(true)), ("event", jv(numbat::event_out(&e)))]),
        Err(why) => Jv::Obj(vec![("ok", Jv::Bool(false)), ("why", Jv::Json(Value::String(why.into())))]),
    }
}

#[napi(js_name = "NumbatCore")]
pub struct JsNumbat {
    n: numbat::Numbat,
}

#[napi]
impl JsNumbat {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsNumbat { n: numbat::Numbat::new() }
    }
    /// The message of a read that failed, or null.
    #[napi]
    pub fn read(&mut self, file: Option<String>, now: f64, io: Option<IoFn>) -> Option<String> {
        let mut files = files_of(&io);
        self.n.read(file.as_deref(), now, files.as_mut()).err()
    }
    #[napi(getter, js_name = "fileState")]
    pub fn file_state(&self) -> &'static str {
        self.n.file_state
    }
    #[napi(getter)]
    pub fn offset(&self) -> Option<f64> {
        self.n.offset
    }
    #[napi(getter)]
    pub fn ino(&self) -> Option<f64> {
        self.n.ino
    }
    #[napi(getter)]
    pub fn partial(&self) -> String {
        self.n.partial.clone()
    }
    #[napi(getter)]
    pub fn skipped(&self) -> f64 {
        self.n.skipped
    }
    #[napi(setter)]
    pub fn set_skipped(&mut self, n: f64) {
        self.n.skipped = n;
    }
    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.n.runs.len() as u32
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.n.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn sessions(&self) -> Jv {
        list(self.n.sessions())
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.n.runs_out())
    }
}

// ---------- core/reported-runs.js ----------

#[napi(js_name = "reportedRunId")]
pub fn reported_run_id(actor: String, app: String, run: String) -> String {
    reported_runs::id_of(&actor, &app, &run)
}

/// observedSessions: which of the sessions' ids are the first of theirs.
#[napi(js_name = "observedFirsts")]
pub fn observed_firsts(ids: Vec<MaybeJson>) -> Vec<bool> {
    let ids: Vec<Option<Value>> = ids.into_iter().map(|m| m.0).collect();
    reported_runs::firsts(&ids)
}

#[napi(js_name = "ReportedRunsCore")]
pub struct JsReportedRuns {
    r: reported_runs::ReportedRuns,
}

#[napi]
impl JsReportedRuns {
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        JsReportedRuns { r: reported_runs::ReportedRuns::new() }
    }
    /// { ok, id } or { ok: false, error }. `now` is asked only for a
    /// report that's good, and what it throws goes on to the caller.
    #[napi]
    pub fn report(&mut self, actor: String, run: Option<String>, app: String, title: Option<String>, presence: Option<String>, now: Function<(), f64>) -> Result<Jv> {
        let answer = |r: std::result::Result<String, String>| match r {
            Ok(id) => Jv::Obj(vec![("ok", Jv::Bool(true)), ("id", Jv::Json(Value::String(id)))]),
            Err(error) => Jv::Obj(vec![("ok", Jv::Bool(false)), ("error", Jv::Json(Value::String(error)))]),
        };
        if let Some(e) = reported_runs::refusal(run.as_deref(), &app, presence.as_deref()) {
            return Ok(answer(Err(e)));
        }
        let t = now.call(())?;
        Ok(answer(self.r.report(&actor, run.as_deref(), &app, title.as_deref(), presence.as_deref(), &mut || t)))
    }
    #[napi]
    pub fn forget(&mut self, actor: String) {
        self.r.forget(&actor);
    }
    #[napi]
    pub fn sessions(&mut self, now: f64) -> Jv {
        list(self.r.sessions(now))
    }
    /// { updatedAt, endedAt, serial } of a run, or null.
    #[napi]
    pub fn live(&self, id: String) -> Jv {
        match self.r.runs.get(&id) {
            Some(r) => Jv::Obj(vec![("updatedAt", Jv::Num(r.updated_at)), ("endedAt", r.ended_at.map_or(Jv::Json(Value::Null), Jv::Num)), ("serial", Jv::Num(r.serial as f64))]),
            None => Jv::Json(Value::Null),
        }
    }
    #[napi(js_name = "statusOf")]
    pub fn status_of(&self, id: String, serial: f64, ended_at: Option<f64>, updated_at: f64, now: f64) -> &'static str {
        self.r.status_of(&id, serial as u64, (ended_at, updated_at), now)
    }
    #[napi]
    pub fn ids(&self) -> Vec<String> {
        self.r.runs.keys().map(str::to_string).collect()
    }
    #[napi]
    pub fn runs(&self) -> Jv {
        list(self.r.runs_out())
    }
}
