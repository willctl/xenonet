//! xenonet-state's stats and search, and xenonet-sources' wiki, for their
//! twins (core/stats.js, core/search.js, core/wiki.js).

use crate::js::{Json, MaybeJson};
use crate::sources::thrown;
use napi::bindgen_prelude::*;
use napi::sys;
use napi_derive::napi;
use serde_json::Value;
use std::ptr;
use xenonet_sources::lines::TypeError;
use xenonet_sources::wiki::{self, Out};
use xenonet_state::search::{self, Begun, Plan, Root, Run, Search};
use xenonet_state::stats::{self, Event, EventIds, File, Kept, Transcripts};

// ---------- answers ----------

/// An answer as the crates build it, made a JS value.
pub struct OutJs(pub Out);

fn check(status: sys::napi_status, what: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Error::new(Status::GenericFailure, format!("couldn't {what} ({status})")))
    }
}

unsafe fn string(env: sys::napi_env, s: &str) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &raw mut out) }, "make a string")?;
    Ok(out)
}

unsafe fn build(env: sys::napi_env, v: &Out) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    match v {
        Out::Undef => check(unsafe { sys::napi_get_undefined(env, &raw mut out) }, "make undefined")?,
        Out::Null => check(unsafe { sys::napi_get_null(env, &raw mut out) }, "make null")?,
        Out::Bool(b) => check(unsafe { sys::napi_get_boolean(env, *b, &raw mut out) }, "make a boolean")?,
        Out::Num(n) => check(unsafe { sys::napi_create_double(env, *n, &raw mut out) }, "make a number")?,
        Out::Str(s) => out = unsafe { string(env, s)? },
        Out::Json(j) => out = unsafe { crate::js::to_js(env, j, 0)? },
        // A Uint8Array, as node:sqlite gives a BLOB.
        Out::Bytes(b) => {
            let mut data = ptr::null_mut();
            let mut buf = ptr::null_mut();
            check(unsafe { sys::napi_create_arraybuffer(env, b.len(), &raw mut data, &raw mut buf) }, "make an array buffer")?;
            if !b.is_empty() {
                unsafe { ptr::copy_nonoverlapping(b.as_ptr(), data.cast::<u8>(), b.len()) };
            }
            check(unsafe { sys::napi_create_typedarray(env, sys::TypedarrayType::uint8_array, b.len(), buf, 0, &raw mut out) }, "make a Uint8Array")?;
        }
        Out::Arr(items) => {
            check(unsafe { sys::napi_create_array_with_length(env, items.len(), &raw mut out) }, "make an array")?;
            for (i, x) in items.iter().enumerate() {
                let item = unsafe { build(env, x)? };
                check(unsafe { sys::napi_set_element(env, out, i as u32, item) }, "fill an array")?;
            }
        }
        Out::Obj(fields) => {
            check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
            for (k, x) in fields {
                let key = unsafe { string(env, k)? };
                let value = unsafe { build(env, x)? };
                if k == "__proto__" {
                    // Defined, not set: setting __proto__ would change the
                    // object's prototype.
                    let attributes = sys::PropertyAttributes::writable | sys::PropertyAttributes::enumerable | sys::PropertyAttributes::configurable;
                    let d = sys::napi_property_descriptor { utf8name: ptr::null(), name: key, method: None, getter: None, setter: None, value, attributes, data: ptr::null_mut() };
                    check(unsafe { sys::napi_define_properties(env, out, 1, &raw const d) }, "define a key")?;
                } else {
                    check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill an object")?;
                }
            }
        }
        Out::Assigned(fields) => {
            check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
            for (k, x) in fields {
                let key = unsafe { string(env, k)? };
                let value = unsafe { build(env, x)? };
                check(unsafe { sys::napi_set_property(env, out, key, value) }, "set a key")?;
            }
        }
    }
    Ok(out)
}

impl ToNapiValue for OutJs {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        unsafe { build(env, &val.0) }
    }
}

impl TypeName for OutJs {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

fn answer(env: Env, r: std::result::Result<Out, TypeError>) -> Result<OutJs> {
    r.map(OutJs).or_else(|e| thrown(env, e))
}

// What the index's errors throw: a TypeError where JS would throw one, and
// SQLite's message as node:sqlite throws it.
fn failed<T>(env: Env, e: search::Error) -> Result<T> {
    match e {
        search::Error::Type(e) => thrown(env, e),
        search::Error::Sql(m) => {
            let _ = env.throw_error(&m, Some("ERR_SQLITE_ERROR"));
            Err(Error::new(Status::PendingException, m))
        }
    }
}

fn values(v: Json) -> Vec<Value> {
    match v.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

// ---------- wiki.js ----------

/// { meta, end }: the frontmatter's keys, and where the text after it
/// starts, in UTF-16 units (null without frontmatter).
#[napi(js_name = "wikiParseFrontmatter")]
pub fn wiki_parse_frontmatter(text: String) -> OutJs {
    let (meta, after) = wiki::frontmatter_at(&text);
    let meta = Out::Assigned(meta.into_iter().map(|(k, v)| (k, Out::Json(v))).collect());
    let end = after.map_or(Out::Null, |a| Out::Num(xenonet_base::jsstr::len(&text[..a]) as f64));
    OutJs(Out::obj(vec![("meta", meta), ("end", end)]))
}

#[napi(js_name = "wikiLinks")]
pub fn wiki_links(text: String) -> Vec<String> {
    wiki::wiki_links(&text)
}

#[napi(js_name = "wikiMentionedProjects")]
pub fn wiki_mentioned_projects(env: Env, text: String, tags: MaybeJson, projects: MaybeJson, root: MaybeJson) -> Result<OutJs> {
    let keys = wiki::mentioned_projects(&text, tags.0.as_ref(), projects.0.as_ref(), root.0.as_ref());
    answer(env, keys.map(|ks| Out::Arr(ks.into_iter().map(|k| Out::of(k.as_ref())).collect())))
}

#[napi(js_name = "wikiParseLog")]
pub fn wiki_parse_log(env: Env, text: String, date: MaybeJson) -> Result<OutJs> {
    answer(env, wiki::parse_log(&text, date.0.as_ref()).map(|es| Out::Arr(es.iter().map(wiki::LogEntry::out).collect())))
}

/// The folder read, or null without one.
#[napi(js_name = "readWiki")]
pub fn read_wiki(env: Env, dir: MaybeJson, projects: MaybeJson, root: MaybeJson, now: f64, server: MaybeJson) -> Result<OutJs> {
    let dir = match dir.0 {
        None | Some(Value::Null) => return Ok(OutJs(Out::Null)),
        Some(v) if !xenonet_base::text::truthy(Some(&v)) => return Ok(OutJs(Out::Null)),
        Some(Value::String(s)) => s,
        Some(_) => return thrown(env, TypeError("The \"path\" argument must be of type string".into())),
    };
    answer(env, wiki::read_wiki(&dir, projects.0.as_ref(), root.0.as_ref(), now, Out::of(server.0.as_ref())))
}

// ---------- stats.js ----------

#[napi(js_name = "TranscriptsCore")]
#[derive(Default)]
pub struct JsTranscripts {
    t: Transcripts,
}

#[napi]
impl JsTranscripts {
    #[napi(constructor)]
    pub fn new() -> Self {
        JsTranscripts::default()
    }
    /// [{ file, agentId }].
    #[napi]
    pub fn files(&mut self, session_id: String, claude_root: Option<String>, codex_roots: Vec<String>, now: f64) -> OutJs {
        let files = self.t.files(&session_id, claude_root.as_deref(), &codex_roots, now);
        OutJs(Out::Arr(files.into_iter().map(|(file, agent)| Out::obj(vec![("file", Out::Str(file)), ("agentId", agent.map_or(Out::Null, Out::Str))])).collect()))
    }
    #[napi]
    pub fn list(&mut self, codex_roots: Vec<String>, now: f64) {
        self.t.list(&codex_roots, now);
    }
    /// [[thread id, file]].
    #[napi(getter)]
    pub fn rollouts(&self) -> Vec<Vec<String>> {
        self.t.rollouts.iter().map(|(k, f)| vec![k.clone(), f.clone()]).collect()
    }
    #[napi(getter, js_name = "listedAt")]
    pub fn listed_at(&self) -> f64 {
        self.t.listed_at
    }
}

#[napi(js_name = "StatsKept")]
#[derive(Default)]
pub struct JsKept {
    k: Kept,
}

#[napi]
impl JsKept {
    #[napi(constructor)]
    pub fn new() -> Self {
        JsKept::default()
    }
    #[napi]
    pub fn hit(&self, session_id: String, key: String) -> bool {
        self.k.hit(&session_id, &key)
    }
    /// The session let go to make room, if one was.
    #[napi]
    pub fn keep(&mut self, session_id: String, key: String) -> Option<String> {
        self.k.keep(&session_id, &key)
    }
}

/// What a session's files are now, as one key.
#[napi(js_name = "statsStamp")]
pub fn stats_stamp(files: Vec<String>) -> String {
    Kept::stamp(&files)
}

/// The app a session id names.
#[napi(js_name = "statsAppOf")]
pub fn stats_app_of(session_id: String) -> String {
    session_id.split(':').next().unwrap_or_default().to_string()
}

/// A session's stats from its files read: [{ agentId, lines: [record] }].
#[napi(js_name = "sessionStats")]
pub fn session_stats(env: Env, session_id: String, files: Json) -> Result<OutJs> {
    let files = values(files)
        .into_iter()
        .map(|f| {
            let mut f = match f {
                Value::Object(m) => m,
                _ => Default::default(),
            };
            File { agent: f.remove("agentId"), lines: f.remove("lines").map(|l| values(Json(l))).unwrap_or_default() }
        })
        .collect();
    answer(env, stats::session_stats(&session_id, files))
}

#[napi(js_name = "statsSummarize")]
pub fn stats_summarize(env: Env, events: Json, app: MaybeJson) -> Result<OutJs> {
    let mut ids = EventIds::default();
    let events: std::result::Result<Vec<Event>, TypeError> = values(events).iter().map(|e| Event::from_js(e, &mut ids)).collect();
    answer(env, events.and_then(|es| stats::summarize(&es, app.0.as_ref())))
}

// ---------- search.js ----------

#[napi(js_name = "SearchPlan")]
pub struct JsPlan {
    p: Plan,
}

#[napi]
impl JsPlan {
    #[napi(getter)]
    pub fn whole(&self) -> bool {
        self.p.whole
    }
    #[napi(getter)]
    pub fn offset(&self) -> f64 {
        self.p.offset
    }
    #[napi(getter)]
    pub fn size(&self) -> f64 {
        self.p.size
    }
    #[napi(getter, js_name = "sessionId")]
    pub fn session_id(&self) -> String {
        self.p.session_id.clone()
    }
    /// Whether its prompts are its user_message events (see docsFromLine).
    #[napi(getter, js_name = "userEvents")]
    pub fn user_events(&self) -> bool {
        self.p.app == "codex" && !self.p.items
    }
}

#[napi(js_name = "SearchRun")]
pub struct JsRun {
    r: Run,
}

#[napi]
impl JsRun {
    #[napi(constructor)]
    pub fn new(file: String, app: String, session_id: String, user_events: bool) -> Self {
        JsRun { r: Search::run(&file, &app, &session_id, user_events) }
    }
    /// How many documents went in.
    #[napi(getter)]
    pub fn n(&self) -> f64 {
        self.r.n
    }
}

#[napi(js_name = "SearchCore")]
pub struct JsSearch {
    s: Search,
}

#[napi]
impl JsSearch {
    #[napi(constructor)]
    pub fn new(db_file: Option<String>, claude_root: Option<String>, codex_roots: Vec<Option<String>>) -> Self {
        JsSearch { s: Search::new(db_file.as_deref(), claude_root.as_deref(), &codex_roots) }
    }
    /// [{ dir, app }]: the folders that are there.
    #[napi(getter)]
    pub fn roots(&self) -> OutJs {
        OutJs(Out::Arr(self.s.roots.iter().map(|Root { dir, app }| Out::obj(vec![("dir", Out::Str(dir.clone())), ("app", Out::Str(app.clone()))])).collect()))
    }
    #[napi(getter)]
    pub fn generation(&self) -> f64 {
        self.s.generation as f64
    }
    #[napi]
    pub fn connect(&mut self, env: Env) -> Result<()> {
        self.s.connect().or_else(|e| failed(env, e))
    }
    #[napi]
    pub fn prepare(&mut self, env: Env) -> Result<()> {
        self.s.prepare().or_else(|e| failed(env, e))
    }
    #[napi]
    pub fn close(&mut self) {
        self.s.close();
    }
    #[napi]
    pub fn plan(&self, env: Env, file: String, app: String) -> Result<Option<JsPlan>> {
        match self.s.plan(&file, &app) {
            Ok(p) => Ok(p.map(|p| JsPlan { p })),
            Err(e) => failed(env, e),
        }
    }
    /// The lines to index, or null when the file must be read whole first.
    #[napi]
    pub fn begin(&self, env: Env, plan: &mut JsPlan, text: String) -> Result<Option<Vec<String>>> {
        match self.s.begin(&mut plan.p, &text) {
            Ok(Begun::ReadWhole) => Ok(None),
            Ok(Begun::Lines(lines)) => Ok(Some(lines)),
            Err(e) => failed(env, e),
        }
    }
    #[napi]
    pub fn add(&self, env: Env, run: &mut JsRun, raw: Json) -> Result<()> {
        self.s.add(&mut run.r, raw.0).or_else(|e| failed(env, e))
    }
    #[napi]
    pub fn commit(&self, env: Env, plan: &JsPlan) -> Result<()> {
        self.s.commit(&plan.p).or_else(|e| failed(env, e))
    }
    #[napi]
    pub fn rollback(&self, env: Env) -> Result<()> {
        self.s.rollback().or_else(|e| failed(env, e))
    }
    /// [files, documents].
    #[napi]
    pub fn counts(&self, env: Env) -> Result<Vec<f64>> {
        match self.s.counts() {
            Ok((files, docs)) => Ok(vec![files, docs]),
            Err(e) => failed(env, e),
        }
    }
    #[napi]
    pub fn search(&self, env: Env, q: String, limit: f64) -> Result<OutJs> {
        match self.s.search(&q, limit) {
            Ok(hits) => Ok(OutJs(Out::Arr(hits))),
            Err(e) => failed(env, e),
        }
    }

    // ----- the handle -----

    #[napi]
    pub fn check(&self, env: Env, generation: f64, sql: String) -> Result<()> {
        self.s.check(generation as u64, &sql).or_else(|e| failed(env, e))
    }
    #[napi(js_name = "closeHandle")]
    pub fn close_handle(&mut self, env: Env, generation: f64) -> Result<()> {
        self.s.close_handle(generation as u64).or_else(|e| failed(env, e))
    }
    /// The rows, each an object of its columns.
    #[napi]
    pub fn query(&self, env: Env, generation: f64, sql: String, params: Json, all: bool) -> Result<OutJs> {
        match self.s.query(generation as u64, &sql, &values(params), all) {
            Ok(rows) => Ok(OutJs(Out::Arr(rows.into_iter().map(Out::Obj).collect()))),
            Err(e) => failed(env, e),
        }
    }
    /// { changes, lastInsertRowid }.
    #[napi(js_name = "runSql")]
    pub fn run_sql(&self, env: Env, generation: f64, sql: String, params: Json) -> Result<OutJs> {
        match self.s.run_sql(generation as u64, &sql, &values(params)) {
            Ok((changes, rowid)) => Ok(OutJs(Out::obj(vec![("changes", Out::Num(changes)), ("lastInsertRowid", Out::Num(rowid))]))),
            Err(e) => failed(env, e),
        }
    }
    #[napi]
    pub fn exec(&self, env: Env, generation: f64, sql: String) -> Result<()> {
        self.s.exec(generation as u64, &sql).or_else(|e| failed(env, e))
    }
}

/// [{ role, text, ts }].
#[napi(js_name = "searchDocsFromLine")]
pub fn search_docs_from_line(env: Env, raw: Json, app: String, user_events: bool) -> Result<OutJs> {
    let docs = search::docs_from_line(raw.0, &app, user_events);
    answer(env, docs.map(|ds| Out::Arr(ds.into_iter().map(|d| Out::obj(vec![("role", Out::Str(d.role.into())), ("text", Out::Str(d.text)), ("ts", Out::Num(d.ts))])).collect())))
}

#[napi(js_name = "searchFtsQuery")]
pub fn search_fts_query(q: String) -> String {
    search::fts_query(&q)
}

#[napi(js_name = "searchSessionOf")]
pub fn search_session_of(file: String, app: String) -> Option<String> {
    search::session_of(&file, &app)
}

#[napi(js_name = "searchListFiles")]
pub fn search_list_files(dir: String) -> Vec<String> {
    search::list_transcripts(&dir)
}

#[napi(js_name = "searchLines")]
pub fn search_lines(text: String) -> Vec<String> {
    search::lines_of(&text)
}
