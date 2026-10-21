//! core/search.js: full-text search across both apps' transcripts, in a
//! local SQLite FTS5 index (search.sqlite in the cache dir). Indexing is
//! incremental: appended lines are read from the last offset, and Codex's
//! compressed .jsonl.zst archives are read whole.
//!
//! The index is the one core/search.js keeps, on the same file and schema,
//! so either side opens what the other wrote: the same statements make it,
//! numbers are bound as doubles (as node:sqlite binds every JS number), and
//! the connection is set up as node:sqlite sets one up (foreign keys on,
//! double-quoted strings off, no busy timeout).
//!
//! The index has a version (PRAGMA user_version). 1: the files table says
//! which Codex rollouts have a completed user message, since one with none
//! at all has its prompts in its user_message events (see lines.rs); an
//! index from before is made again for Codex's rollouts.
//!
//! Reading the files stays with the caller (a compressed archive is
//! unpacked there), and so does each line's way through the transcripts'
//! normalizer: this says what to read, takes the text and the records, and
//! makes every decision.

use regex::Regex;
use rusqlite::types::Value as Sql;
use rusqlite::{params_from_iter, Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;
use xenonet_base::text::{js_number, js_trim, truthy};
use xenonet_base::{fs as xfs, json, jsstr};
use xenonet_sources::line_events::{self, Broken, Field, Kind, LineEvent, LineFile};
use xenonet_sources::lines::{prop, ClaudeLine, CodexLine, Js, TypeError};
use xenonet_sources::wiki::{to_str_or_empty, Out};

const MAX_TEXT: usize = 6000;
const VERSION: i64 = 1;

const SCHEMA: &str = "
      create virtual table if not exists docs using fts5(text, session_id unindexed, app unindexed, role unindexed, ts unindexed, file unindexed, tokenize = 'porter unicode61 remove_diacritics 2');
      create table if not exists files(path text primary key, size integer, mtime integer, offset integer, items integer not null default 0);
    ";

/// What went wrong: what JS would throw reading a record, or SQLite's
/// error, by its message.
#[derive(Debug, PartialEq)]
pub enum Error {
    Type(TypeError),
    Sql(String),
}

impl From<TypeError> for Error {
    fn from(e: TypeError) -> Self {
        Error::Type(e)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(message(&e))
    }
}

/// SQLite's own message for an error, as node:sqlite gives it.
pub fn message(e: &rusqlite::Error) -> String {
    match e {
        rusqlite::Error::SqliteFailure(_, Some(m)) => m.clone(),
        e => e.to_string(),
    }
}

// ---------- records to documents ----------

fn is_str(v: Js<'_>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

/// A searchable record: whose words (user, assistant or subagent), what
/// they said, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub role: &'static str,
    pub text: String,
    pub ts: f64,
}

// `[...].join(sep)` of values: null and undefined as "", the rest as
// String() writes them.
fn join(items: &[Js<'_>], sep: &str) -> Result<String, TypeError> {
    let mut parts = Vec::with_capacity(items.len());
    for v in items {
        parts.push(to_str_or_empty(*v)?);
    }
    Ok(parts.join(sep))
}

fn text_of(content: Js<'_>) -> Result<String, TypeError> {
    match content {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(a)) => {
            let mut parts = Vec::with_capacity(a.len());
            for c in a {
                parts.push(if c.is_string() { Some(c) } else { nn(prop(Some(c), "text")?) });
            }
            join(&parts, "\n")
        }
        _ => Ok(String::new()),
    }
}

fn skippable(t: &str) -> bool {
    js_trim(t).is_empty() || t.starts_with('<') || t.starts_with("# AGENTS.md")
}

// A record, unless there's nothing worth searching in it, or the line
// doesn't say when it was written (as stats leave such a line out).
fn doc(role: &'static str, text: String, ts: f64) -> Vec<Doc> {
    if skippable(&text) || !ts.is_finite() {
        Vec::new()
    } else {
        vec![Doc { role, text, ts }]
    }
}

// Whether search reads the field a line broke at: a user line only as far
// as its first tool result, and not a meta one; its text unless it's a
// compacted summary; and neither the refusal, a usage nor the type stats
// reads a Codex call by.
fn reaches(b: &Broken<'_>, origin: &str) -> bool {
    match b.at {
        Field::Content => !truthy(b.meta) && !b.after_result,
        Field::Text => origin == "user" && !truthy(b.meta) && !b.results && !truthy(b.compact_summary),
        Field::Line | Field::Blocks => true,
        Field::Refusal | Field::Usage | Field::Type => false,
    }
}

/// One searchable record per human or agent message, from a line's events
/// (line_events.rs), each joined with newlines. Tool results, a compacted
/// summary and an interruption aren't anyone's message.
pub fn docs_of(events: &[LineEvent<'_>]) -> Result<Vec<Doc>, TypeError> {
    for e in events {
        match &e.kind {
            // A line with tool results is passed over whole.
            Kind::Result { .. } => return Ok(Vec::new()),
            Kind::Prompt(p) => {
                // A Codex rollout's user_message events are its prompts while
                // it has no completed user message (those are read as items,
                // below).
                if &*e.origin == "event_msg:user_message" {
                    return Ok(doc("user", p.text.clone(), e.t));
                }
                // Claude Code's: not a meta line.
                let Some(c) = &p.claude else { continue };
                if truthy(c.meta) || truthy(c.compact_summary) || c.interruption {
                    return Ok(Vec::new());
                }
                let role = if truthy(c.sidechain) { "subagent" } else { "user" };
                return Ok(doc(role, text_of(p.parts)?, e.t));
            }
            Kind::Said { parts, sidechain } => match &*e.origin {
                "event_msg:agent_message" => return Ok(doc("assistant", to_str_or_empty(*parts)?, e.t)),
                "response_item:message" => return Ok(doc("assistant", text_of(*parts)?, e.t)),
                "assistant" => {
                    let Some(Value::Array(blocks)) = parts else { return Err(TypeError("o.blocks.filter is not a function".into())) };
                    let mut texts = Vec::new();
                    for b in blocks {
                        if is_str(prop(Some(b), "type")?, "text") {
                            texts.push(prop(Some(b), "text")?);
                        }
                    }
                    let t = join(&texts, "\n")?;
                    if js_trim(&t).is_empty() {
                        return Ok(Vec::new());
                    }
                    let role = if truthy(*sidechain) { "subagent" } else { "assistant" };
                    return Ok(doc(role, t, e.t));
                }
                _ => {}
            },
            // What you said in Codex is a rollout's completed user messages,
            // whatever else may say it again.
            Kind::Item(it) => {
                let t = field(it, "type");
                if is_str(t, "UserMessage") || is_str(t, "AgentMessage") {
                    let role = if is_str(t, "UserMessage") { "user" } else { "assistant" };
                    return Ok(doc(role, text_of(field(it, "content"))?, e.t));
                }
            }
            Kind::Broken(b) if reaches(b, &e.origin) => return Err(b.error()),
            _ => {}
        }
    }
    Ok(Vec::new())
}

fn field<'a>(v: &'a Value, k: &str) -> Js<'a> {
    v.as_object().and_then(|m| m.get(k))
}

/// One searchable record per human or agent message, from a line's record.
/// `user_events`: a Codex rollout with no completed user messages at all,
/// whose prompts are its user_message events. An app XenoNet doesn't tail
/// is refused, not read as Codex.
pub fn docs_from_line(raw: Value, app: &str, user_events: bool) -> Result<Vec<Doc>, TypeError> {
    let file = LineFile { items: !user_events, ..LineFile::default() };
    let Some(tailed) = xenonet_base::apps::tailed_or_text(Some(app), None) else {
        return Err(TypeError(xenonet_base::apps::untailed(app)));
    };
    // A record that isn't an object fails where search read it first.
    if tailed == "claude" {
        let o = ClaudeLine { raw };
        o.kind()?;
        return docs_of(&line_events::claude(&o, &file));
    }
    let o = CodexLine { raw };
    o.payload()?;
    docs_of(&line_events::codex(&o, &file))
}

/// Whether a rollout's text has a completed user message: only the lines
/// that name one are read, as rollouts can be hundreds of megabytes.
pub fn holds_user_messages(text: &str) -> bool {
    const NAME: &str = "\"UserMessage\"";
    let mut at = text.find(NAME);
    while let Some(i) = at {
        let start = text[..i].rfind('\n').map_or(0, |n| n + 1);
        let end = text[i..].find('\n').map_or(text.len(), |n| i + n);
        if let Ok(raw) = json::parse(&text[start..end]) {
            if matches!((CodexLine { raw }).user_message(), Ok(Some(_))) {
                return true;
            }
        }
        at = text[end..].find(NAME).map(|n| end + n);
    }
    false
}

static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}_]+").expect("a fixed pattern"));

/// FTS5 query from free text: every word must match, as a prefix.
pub fn fts_query(q: &str) -> String {
    WORD.find_iter(q).take(8).map(|w| format!("\"{}\"*", w.as_str())).collect::<Vec<_>>().join(" AND ")
}

// Matched on the path in ASCII lowercase, as JS's /i matches it.
static SUBAGENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/([0-9a-f-]{36})/subagents/[^/]+\.jsonl\z").expect("a fixed pattern"));
static UUID_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl(\.zst)?\z").expect("a fixed pattern"));

/// The session a transcript is of: a Claude Code subagent's file is its
/// session's, and a Codex file is one only when it's a rollout. None for an
/// app XenoNet doesn't tail.
pub fn session_of(file: &str, app: &str) -> Option<String> {
    let norm = file.replace('\\', "/");
    let low = norm.to_ascii_lowercase();
    if xenonet_base::apps::tailed_or_text(Some(app), None)? == "claude" {
        if let Some(m) = SUBAGENT.captures(&low).and_then(|c| c.get(1)) {
            return Some(format!("claude:{}", &norm[m.range()]));
        }
    } else if !norm.contains("rollout-") {
        return None;
    }
    UUID_FILE.captures(&low).and_then(|c| c.get(1)).map(|m| format!("{app}:{}", &norm[m.range()]))
}

/// The transcripts under a folder the index reads: *.jsonl and
/// *.jsonl.zst, six folders down.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
pub fn list_transcripts(dir: &str) -> Vec<String> {
    xenonet_base::paths::list_files(dir, &mut |name| name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"), 6)
}

/// `text.split("\n")`, without the blank lines.
pub fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n').filter(|l| !js_trim(l).is_empty()).collect()
}

// ---------- SQLite's values ----------

/// A SQLite value as node:sqlite hands it to JS.
pub fn sql_out(v: &Sql) -> Out {
    match v {
        Sql::Null => Out::Null,
        Sql::Integer(i) => Out::Num(*i as f64),
        Sql::Real(r) => Out::Num(*r),
        Sql::Text(s) => Out::Str(s.clone()),
        Sql::Blob(b) => Out::Bytes(b.clone()),
    }
}

// Number(v) of what node:sqlite gives JS.
fn sql_number(v: &Sql) -> f64 {
    match v {
        Sql::Null => 0.0,
        Sql::Integer(i) => *i as f64,
        Sql::Real(r) => *r,
        Sql::Text(s) => js_number(Some(&Value::String(s.clone()))),
        Sql::Blob(_) => f64::NAN,
    }
}

// `v === n`, for a number n.
fn sql_is(v: &Sql, n: f64) -> bool {
    match v {
        Sql::Integer(i) => *i as f64 == n,
        Sql::Real(r) => *r == n,
        _ => false,
    }
}

/// A JS value bound as node:sqlite binds it: a number as a double, a
/// string as text, null as NULL; anything else it refuses.
pub fn sql_param(v: &Value, i: usize) -> Result<Sql, Error> {
    match v {
        Value::Null => Ok(Sql::Null),
        Value::Number(n) => Ok(Sql::Real(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => Ok(Sql::Text(s.clone())),
        _ => Err(Error::Sql(format!("Provided value cannot be bound to SQLite parameter {}.", i + 1))),
    }
}

// How a JS Map tells two values apart, for the rows' session ids.
fn sql_key(v: &Sql, row: usize) -> String {
    match v {
        Sql::Null => "null".into(),
        Sql::Integer(i) => format!("n{}", *i as f64),
        Sql::Real(r) => format!("n{}", if *r == 0.0 { 0.0 } else { *r }),
        Sql::Text(s) => format!("s{s}"),
        Sql::Blob(_) => format!("b{row}"),
    }
}

// ---------- the index ----------

/// A folder the index reads, and whose transcripts are in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub dir: String,
    pub app: String,
}

/// What reading a file for the index takes: where its text starts, and
/// what's decided as it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub file: String,
    pub app: String,
    pub session_id: String,
    pub size: f64,
    pub mtime: f64,
    pub zst: bool,
    /// Read from the start (and indexed again), or from `offset`.
    pub whole: bool,
    pub offset: f64,
    /// Whether the rollout has a completed user message.
    pub items: bool,
    /// Whether the index has read the file before. One it hasn't has no
    /// documents in it yet (a file's documents and its row go in
    /// together), so reading it whole has none to take out first.
    known: bool,
    checked: bool,
    next_offset: f64,
}

/// What `begin` says: read the file whole first, or index these lines.
#[derive(Clone, Debug, PartialEq)]
pub enum Begun<'a> {
    ReadWhole,
    Lines(Vec<&'a str>),
}

/// One text being indexed: its records' documents go in once each.
#[derive(Clone, Debug)]
pub struct Run {
    file: String,
    app: String,
    session_id: String,
    user_events: bool,
    seen: HashSet<String>,
    pub n: f64,
}

pub struct Search {
    pub db_file: Option<String>,
    pub roots: Vec<Root>,
    db: Option<Connection>,
    prepared: bool,
    /// Counts the connections opened, so a handle to a closed one stays closed.
    pub generation: u64,
}

const NOT_OPEN: &str = "database is not open";

impl Search {
    /// The index in `db_file`, of the transcripts under the folders that
    /// are there.
    pub fn new(db_file: Option<&str>, claude_root: Option<&str>, codex_roots: &[Option<String>]) -> Search {
        let mut roots = Vec::new();
        if let Some(dir) = claude_root.filter(|d| !d.is_empty()) {
            roots.push(Root { dir: dir.to_string(), app: "claude".into() });
        }
        for dir in codex_roots {
            roots.push(Root { dir: dir.clone().unwrap_or_default(), app: "codex".into() });
        }
        roots.retain(|r| !r.dir.is_empty() && Path::new(&r.dir).exists());
        Search { db_file: db_file.map(String::from), roots, db: None, prepared: false, generation: 0 }
    }

    pub fn is_open(&self) -> bool {
        self.db.is_some()
    }

    fn db(&self) -> Result<&Connection, Error> {
        self.db.as_ref().ok_or_else(|| Error::Sql(NOT_OPEN.into()))
    }

    /// Opens the file, making its folder first.
    pub fn connect(&mut self) -> Result<(), Error> {
        let Some(file) = &self.db_file else { return Err(Error::Type(TypeError("The \"path\" argument must be of type string. Received undefined".into()))) };
        if let Some(dir) = Path::new(file).parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| Error::Sql(e.to_string()))?;
        }
        let db = Connection::open(file)?;
        db.busy_timeout(Duration::ZERO)?;
        use rusqlite::config::DbConfig;
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)?;
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML, false)?;
        db.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL, false)?;
        self.db = Some(db);
        self.generation += 1;
        Ok(())
    }

    /// Makes the tables when they aren't there, and brings an older index
    /// up to date.
    pub fn prepare(&mut self) -> Result<(), Error> {
        let db = self.db()?;
        db.execute_batch(SCHEMA)?;
        self.migrate()?;
        self.prepared = true;
        Ok(())
    }

    // An index from before version 1 has no `items`, and took a Codex
    // rollout's prompts from its completed user messages alone: Codex's
    // rollouts are indexed again, once.
    fn migrate(&self) -> Result<(), Error> {
        let db = self.db()?;
        let version: Sql = db.query_row("pragma user_version", [], |r| r.get(0))?;
        if sql_number(&version) >= VERSION as f64 {
            return Ok(());
        }
        let columns: Vec<Sql> = db.prepare("pragma table_info(files)")?.query_map([], |r| r.get("name"))?.collect::<Result<_, _>>()?;
        db.execute_batch("begin")?;
        let step = || -> rusqlite::Result<()> {
            if !columns.iter().any(|c| matches!(c, Sql::Text(n) if n == "items")) {
                db.execute_batch("alter table files add column items integer not null default 0")?;
            }
            // The rollouts it found something in, and those under Codex's
            // folders, which it may have found nothing in.
            let found: Vec<Sql> = db.prepare("select distinct file from docs where app = 'codex'")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            db.execute_batch("delete from docs where app = 'codex'")?;
            let mut drop = db.prepare("delete from files where path = ?")?;
            for f in found {
                drop.execute([f])?;
            }
            let mut under = db.prepare("delete from files where substr(path, 1, ?) = ?")?;
            for r in self.roots.iter().filter(|r| r.app == "codex") {
                let sep = std::path::MAIN_SEPARATOR;
                let prefix = if r.dir.ends_with(sep) { r.dir.clone() } else { format!("{}{sep}", r.dir) };
                under.execute((jsstr::len(&prefix) as f64, prefix))?;
            }
            db.execute_batch(&format!("pragma user_version = {VERSION}"))?;
            db.execute_batch("commit")
        };
        if let Err(e) = step() {
            db.execute_batch("rollback")?;
            return Err(e.into());
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.db = None;
    }

    // The statements an open index has: there only once it was prepared.
    fn ready(&self, reading: &str) -> Result<&Connection, Error> {
        if !self.prepared {
            return Err(Error::Type(TypeError(format!("Cannot read properties of undefined (reading '{reading}')"))));
        }
        self.db()
    }

    /// What reading a file takes, or None when there's nothing to index:
    /// it's no session's, it's gone, or it's as it was.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
    pub fn plan(&self, file: &str, app: &str) -> Result<Option<Plan>, Error> {
        let Some(session_id) = session_of(file, app) else { return Ok(None) };
        let Ok(st) = std::fs::metadata(file) else { return Ok(None) };
        let db = self.ready("get")?;
        let row: Option<[Sql; 4]> =
            db.prepare_cached("select size, mtime, offset, items from files where path = ?")?.query_row([file], |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?])).optional()?;
        let size = st.len() as f64;
        let mtime = xfs::mtime_ms_of(&st).map_or(f64::NAN, f64::floor);
        if let Some([size_was, mtime_was, ..]) = &row {
            if sql_is(size_was, size) && sql_is(mtime_was, mtime) {
                return Ok(None);
            }
        }
        let zst = file.ends_with(".zst");
        let offset = row.as_ref().map_or(0.0, |r| sql_number(&r[2]));
        let whole = zst || row.is_none() || size < offset;
        // A Codex rollout's prompts are its completed user messages, or in
        // one with none at all its user_message events (see lines.rs). One
        // indexed so far by its events that has a completed user message now
        // is indexed again, whole, by those.
        let items = app == "codex" && !whole && row.as_ref().is_some_and(|r| sql_is(&r[3], 1.0));
        let known = row.is_some();
        Ok(Some(Plan { file: file.into(), app: app.into(), session_id, size, mtime, zst, whole, offset, items, known, checked: false, next_offset: 0.0 }))
    }

    /// Starts indexing what was read (all of it, or what was added since the
    /// last time): the lines to index, or that the file must be read whole
    /// first. Only complete lines are indexed; a half-written last line
    /// waits for next time.
    pub fn begin<'a>(&self, plan: &mut Plan, text: &'a str) -> Result<Begun<'a>, Error> {
        let body = |text: &'_ str| -> usize {
            if plan.zst {
                text.len()
            } else {
                text.rfind('\n').map_or(0, |i| i + 1)
            }
        };
        if plan.app == "codex" && !plan.items && !plan.checked {
            plan.checked = true;
            plan.items = holds_user_messages(&text[..body(text)]);
            if plan.items && !plan.whole {
                plan.whole = true;
                return Ok(Begun::ReadWhole);
            }
        }
        let offset = if plan.whole { 0.0 } else { plan.offset };
        let indexed = &text[..body(text)];
        let db = self.ready("exec")?;
        db.execute_batch("begin")?;
        // Taking a file's documents out reads every document, as FTS5 has
        // no index on the file, so it's done only when there are some:
        // across a first index of thousands of transcripts, it was most of
        // the time the index took.
        if plan.whole && plan.known {
            if let Err(e) = db.prepare_cached("delete from docs where file = ?").and_then(|mut s| s.execute([&plan.file])) {
                self.rollback()?;
                return Err(e.into());
            }
        }
        plan.next_offset = if plan.zst { plan.size } else { offset + indexed.len() as f64 };
        Ok(Begun::Lines(lines_of(indexed)))
    }

    /// The documents of a text of a session's file: `user_events` as in
    /// docs_from_line.
    pub fn run(file: &str, app: &str, session_id: &str, user_events: bool) -> Run {
        Run { file: file.into(), app: app.into(), session_id: session_id.into(), user_events, seen: HashSet::new(), n: 0.0 }
    }

    /// Indexes a record of the text being indexed: each document once, by
    /// how it starts.
    pub fn add(&self, run: &mut Run, raw: Value) -> Result<(), Error> {
        for d in docs_from_line(raw, &run.app, run.user_events)? {
            let body = jsstr::head(&d.text, MAX_TEXT);
            let key = jsstr::head(&body, 200);
            if !run.seen.insert(key) {
                continue;
            }
            self.ready("run")?.prepare_cached("insert into docs(text, session_id, app, role, ts, file) values (?, ?, ?, ?, ?, ?)")?.execute((
                body,
                &run.session_id,
                &run.app,
                d.role,
                d.ts,
                &run.file,
            ))?;
            run.n += 1.0;
        }
        Ok(())
    }

    /// Says where the file was read to, and commits.
    pub fn commit(&self, plan: &Plan) -> Result<(), Error> {
        let db = self.ready("run")?;
        db.prepare_cached("insert into files(path, size, mtime, offset, items) values (?, ?, ?, ?, ?) on conflict(path) do update set size = excluded.size, mtime = excluded.mtime, offset = excluded.offset, items = excluded.items")?
            .execute((&plan.file, plan.size, plan.mtime, plan.next_offset, if plan.items { 1.0 } else { 0.0 }))?;
        db.execute_batch("commit")?;
        Ok(())
    }

    pub fn rollback(&self) -> Result<(), Error> {
        self.db()?.execute_batch("rollback")?;
        Ok(())
    }

    /// How many files and documents the index has.
    pub fn counts(&self) -> Result<(f64, f64), Error> {
        let db = self.db()?;
        let n = |sql: &str| -> Result<f64, Error> { Ok(sql_number(&db.query_row(sql, [], |r| r.get::<_, Sql>(0))?)) };
        Ok((n("select count(*) n from files")?, n("select count(*) n from docs")?))
    }

    /// The sessions whose transcripts say every word of `q`, best first: each
    /// with its best score, how many of its documents match, and the first
    /// three snippets. At most `limit` (as `slice(0, limit)` takes them).
    pub fn search(&self, q: &str, limit: f64) -> Result<Vec<Out>, Error> {
        let db = self.db()?;
        let m = fts_query(q);
        if m.is_empty() {
            return Ok(Vec::new());
        }
        struct Hit {
            session_id: Sql,
            app: Sql,
            score: f64,
            hits: f64,
            snippets: Vec<Out>,
        }
        let mut stmt = db.prepare(
            "select session_id, app, role, ts, snippet(docs, 0, '⟦', '⟧', ' … ', 16) as snip, bm25(docs) as score from docs where docs match ? order by score limit 500",
        )?;
        let rows = stmt.query_map([&m], |r| Ok([r.get::<_, Sql>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?]))?;
        let mut by: Vec<Hit> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        for (i, row) in rows.enumerate() {
            let [session_id, app, role, ts, snip, score] = row?;
            let score = sql_number(&score);
            let k = sql_key(&session_id, i);
            let j = match at.get(&k) {
                Some(&j) => j,
                None => {
                    at.insert(k, by.len());
                    by.push(Hit { session_id, app, score, hits: 0.0, snippets: Vec::new() });
                    by.len() - 1
                }
            };
            let s = &mut by[j];
            s.hits += 1.0;
            s.score = if s.score.is_nan() || score.is_nan() { f64::NAN } else { s.score.min(score) };
            if s.snippets.len() < 3 {
                s.snippets.push(Out::obj(vec![("role", sql_out(&role)), ("ts", Out::Num(sql_number(&ts))), ("text", sql_out(&snip))]));
            }
        }
        xenonet_base::sort::sort_by(&mut by, |a, b| {
            let d = a.score - b.score;
            let r = if d != 0.0 && !d.is_nan() { d } else { b.hits - a.hits };
            r.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
        });
        let n = by.len();
        let limit = if limit.is_nan() { 0.0 } else { limit.trunc() };
        let end = if limit < 0.0 { (n as f64 + limit).max(0.0) as usize } else { (limit.min(n as f64)) as usize };
        Ok(by
            .into_iter()
            .take(end)
            .map(|s| {
                Out::obj(vec![
                    ("sessionId", sql_out(&s.session_id)),
                    ("app", sql_out(&s.app)),
                    ("score", Out::Num(s.score)),
                    ("hits", Out::Num(s.hits)),
                    ("snippets", Out::Arr(s.snippets)),
                ])
            })
            .collect())
    }

    // ---------- the handle a caller reads the index by ----------

    // The connection a handle was made for, while it's open.
    fn handle(&self, generation: u64) -> Result<&Connection, Error> {
        if generation != self.generation {
            return Err(Error::Sql(NOT_OPEN.into()));
        }
        self.db()
    }

    fn binds(params: &[Value]) -> Result<Vec<Sql>, Error> {
        params.iter().enumerate().map(|(i, v)| sql_param(v, i)).collect()
    }

    /// Checks a statement compiles, as a prepare does.
    pub fn check(&self, generation: u64, sql: &str) -> Result<(), Error> {
        self.handle(generation)?.prepare(sql)?;
        Ok(())
    }

    /// Closes the connection a handle was made for.
    pub fn close_handle(&mut self, generation: u64) -> Result<(), Error> {
        self.handle(generation)?;
        self.close();
        Ok(())
    }

    /// A statement's rows (`all`), or its first (`!all`), each as its
    /// columns and their values.
    pub fn query(&self, generation: u64, sql: &str, params: &[Value], all: bool) -> Result<Vec<Vec<(String, Out)>>, Error> {
        let db = self.handle(generation)?;
        let mut stmt = db.prepare(sql)?;
        let names: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
        let mut rows = stmt.query(params_from_iter(Self::binds(params)?))?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let mut row = Vec::with_capacity(names.len());
            for (i, name) in names.iter().enumerate() {
                row.push((name.clone(), sql_out(&r.get::<_, Sql>(i)?)));
            }
            out.push(row);
            if !all {
                break;
            }
        }
        Ok(out)
    }

    /// Runs a statement: { changes, lastInsertRowid }.
    pub fn run_sql(&self, generation: u64, sql: &str, params: &[Value]) -> Result<(f64, f64), Error> {
        let db = self.handle(generation)?;
        let mut stmt = db.prepare(sql)?;
        let mut rows = stmt.query(params_from_iter(Self::binds(params)?))?;
        while rows.next()?.is_some() {}
        Ok((db.changes() as f64, db.last_insert_rowid() as f64))
    }

    pub fn exec(&self, generation: u64, sql: &str) -> Result<(), Error> {
        self.handle(generation)?.execute_batch(sql)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn documents_from_lines() {
        let d = |raw: Value, app: &str, ev: bool| docs_from_line(raw, app, ev).unwrap().into_iter().map(|d| (d.role, d.text)).collect::<Vec<_>>();
        let ts = "2026-10-02T10:00:00Z";
        assert_eq!(
            d(json!({ "type": "user", "timestamp": ts, "message": { "content": "Should we use SendGrid?" } }), "claude", false),
            [("user", "Should we use SendGrid?".to_string())]
        );
        assert_eq!(d(json!({ "type": "user", "timestamp": ts, "isCompactSummary": true, "message": { "content": "continued" } }), "claude", false), []);
        assert_eq!(
            d(
                json!({ "type": "assistant", "timestamp": ts, "isSidechain": true, "message": { "content": [{ "type": "text", "text": "a" }, { "type": "text", "text": 5 }] } }),
                "claude",
                false
            ),
            [("subagent", "a\n5".to_string())]
        );
        assert!(docs_from_line(json!({ "type": "assistant", "timestamp": ts, "message": { "content": "text" } }), "claude", false).is_err(), "a string has no filter");
        assert_eq!(d(json!({ "type": "user", "timestamp": 0, "message": { "content": "a numeric timestamp" } }), "claude", false), []);
        let ev = json!({ "type": "event_msg", "timestamp": ts, "payload": { "type": "user_message", "message": "Audit it" } });
        assert_eq!((d(ev.clone(), "codex", false), d(ev, "codex", true)), (vec![], vec![("user", "Audit it".to_string())]));
        assert_eq!(
            d(
                json!({ "type": "event_msg", "timestamp": ts, "payload": { "type": "item_completed", "item": { "type": "AgentMessage", "content": [{ "text": "Done" }, "x", { "text": null }] } } }),
                "codex",
                false
            ),
            [("assistant", "Done\nx\n".to_string())]
        );
        assert_eq!(
            d(
                json!({ "type": "response_item", "timestamp": ts, "payload": { "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "# AGENTS.md here" }] } }),
                "codex",
                false
            ),
            []
        );
    }

    #[test]
    fn queries_and_sessions() {
        assert_eq!(fts_query("a-b c"), "\"a\"* AND \"b\"* AND \"c\"*");
        assert_eq!(fts_query("¿Qué? ñandú_2 ✓"), "\"Qué\"* AND \"ñandú_2\"*");
        assert_eq!(fts_query("  "), "");
        assert_eq!(session_of("/c/p/11111111-1111-1111-1111-111111111111/subagents/agent-a.jsonl", "claude").as_deref(), Some("claude:11111111-1111-1111-1111-111111111111"));
        assert_eq!(session_of(r"C:\c\p\AAAAAAAA-1111-1111-1111-111111111111.JSONL", "claude").as_deref(), Some("claude:AAAAAAAA-1111-1111-1111-111111111111"));
        assert_eq!(session_of("/x/rollout-2026-22222222-2222-2222-2222-222222222222.jsonl.zst", "codex").as_deref(), Some("codex:22222222-2222-2222-2222-222222222222"));
        assert_eq!(session_of("/x/22222222-2222-2222-2222-222222222222.jsonl", "codex"), None);
        // An app XenoNet doesn't tail has no session here, and no documents.
        assert_eq!(session_of("/x/rollout-2026-22222222-2222-2222-2222-222222222222.jsonl", "gemini-cli"), None);
        assert_eq!(session_of("/c/p/11111111-1111-1111-1111-111111111111.jsonl", "opencode"), None);
        let refused = docs_from_line(serde_json::json!({ "type": "user", "message": { "content": "hello" } }), "gemini-cli", false).map(|_| ());
        assert_eq!(refused, Err(TypeError("XenoNet doesn't read Gemini CLI's transcripts".into())));
        assert!(holds_user_messages("{\"x\":1}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"item\":{\"type\":\"UserMessage\",\"content\":[]}}}"));
        assert!(!holds_user_messages("{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"\\\"UserMessage\\\"\"}}\nnot json \"UserMessage\""));
    }

    #[test]
    fn indexes_appends_and_finds() {
        let d = tmp("index");
        let claude = d.join("claude");
        std::fs::create_dir_all(claude.join("proj")).unwrap();
        let file = claude.join("proj").join("11111111-1111-1111-1111-111111111111.jsonl");
        let line = |text: &str| format!("{}\n", json!({ "type": "user", "timestamp": "2026-10-02T10:00:00Z", "message": { "content": text } }));
        std::fs::write(&file, line("Should we use SendGrid for email?") + "{\"half").unwrap();
        let mut s = Search::new(Some(&d.join("cache").join("search.sqlite").to_string_lossy()), Some(&claude.to_string_lossy()), &[]);
        assert_eq!(s.roots.len(), 1);
        s.connect().unwrap();
        s.prepare().unwrap();
        let f = file.to_string_lossy().into_owned();
        let index = |s: &Search| -> f64 {
            let Some(mut plan) = s.plan(&f, "claude").unwrap() else { return 0.0 };
            let from = if plan.whole { 0 } else { plan.offset as usize };
            let text = String::from_utf8_lossy(&std::fs::read(&f).unwrap()[from..]).into_owned();
            let Begun::Lines(lines) = s.begin(&mut plan, &text).unwrap() else { panic!("read whole") };
            let mut run = Search::run(&f, "claude", &plan.session_id, false);
            for l in lines {
                s.add(&mut run, json::parse(l).unwrap()).unwrap();
            }
            s.commit(&plan).unwrap();
            run.n
        };
        assert_eq!(index(&s), 1.0);
        assert_eq!(index(&s), 0.0, "unchanged");
        let hit = s.search("sendgrid", 30.0).unwrap();
        assert_eq!(hit.len(), 1);
        std::fs::write(&file, line("Should we use SendGrid for email?") + &line("Mailgun is the alternative.")).unwrap();
        assert_eq!(index(&s), 1.0, "the line once it's whole");
        assert_eq!(s.counts().unwrap(), (1.0, 2.0));
        assert_eq!(s.search("mailgun", 30.0).unwrap().len(), 1);
        assert_eq!(s.search("mailgun", f64::NAN).unwrap().len(), 0);
        // Written again, shorter: read whole, and what it said before goes.
        std::fs::write(&file, line("Postmark, then.")).unwrap();
        assert_eq!(index(&s), 1.0);
        assert_eq!(s.counts().unwrap(), (1.0, 1.0));
        assert_eq!(s.search("mailgun", 30.0).unwrap().len(), 0);
        let rows = s.query(s.generation, "select typeof(ts) t, ts from docs", &[], true).unwrap();
        assert_eq!(rows[0][0].1, Out::Str("real".into()), "a JS number goes in as a double");
        s.close();
        assert_eq!(s.search("mailgun", 30.0).unwrap_err(), Error::Sql(NOT_OPEN.into()));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
