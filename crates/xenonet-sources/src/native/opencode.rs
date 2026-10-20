//! core/native/opencode.js: OpenCode's database. Each OpenCode session in
//! opencode.db is a session on the map, "`opencode:<id>`", and the edit, write,
//! multiedit and apply_patch tools' files are its edits. The database is
//! opened read only for each read and closed after it, and SQLite takes the
//! edits out: a part's other fields never leave the database, apart from
//! apply_patch's text, whose file headers are kept and the rest dropped.

use super::files::Files;
use super::node_path;
use super::{edit_file, edits_out, js_max, js_min, push_edit, str_of, titled, Checked, Edits, NativeReader, Out, Threw, Vendor, DAY_MS, EDIT_KEEP_MS, SESSIONS_MAX};
use crate::session::{RunSession, Via};
use serde_json::{json, Map, Value};
use xenonet_base::json;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::patch;
use xenonet_base::paths::path_style;
use xenonet_base::text::{js_number, js_string, truthy};
use xenonet_schema::drift::clean_version;

pub const SOURCE: &str = "anomaly/opencode/db";
/// The tools whose calls are edits, as the app registry has them.
pub fn edit_tools() -> &'static [String] {
    xenonet_base::apps::edit_tools("opencode")
}
pub const PATCH_TOOLS: [&str; 2] = ["apply_patch", "patch"];
const SESSION_COLUMNS: [&str; 8] = ["id", "parent_id", "directory", "title", "version", "time_created", "time_updated", "time_archived"];
const NEEDED_SESSION: [&str; 4] = ["id", "directory", "time_created", "time_updated"];
const NEEDED_PART: [&str; 3] = ["session_id", "time_created", "data"];
const EDITS_MAX: usize = 5000;

/// OpenCode's ids (ses_…) as XenoNet's session ids take them.
pub fn local_id(id: &str) -> String {
    id.replace('_', "-")
}

pub fn session_id_of(id: &str) -> String {
    format!("opencode:{}", local_id(id))
}

/// The files an apply_patch text names, from its headers only: those it
/// adds, updates, deletes and moves to, less a header with no path.
pub fn patch_files(text: Option<&Value>) -> Vec<String> {
    match text {
        Some(Value::String(s)) => patch::paths(s, true).into_iter().filter(|p| !p.is_empty()).collect(),
        _ => Vec::new(),
    }
}

fn quoted(names: &[&str]) -> String {
    names.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ")
}

/// The edit tools' files since a time, newest first, so a busy day can't
/// crowd out the edits that matter: the last ones.
pub fn edits_sql() -> String {
    let all: Vec<&str> = edit_tools().iter().map(String::as_str).chain(PATCH_TOOLS.iter().copied()).collect();
    format!(
        "
  SELECT session_id, time_created,
    json_extract(data, '$.tool') AS tool,
    CASE WHEN json_type(data, '$.state.input.filePath') = 'text' THEN json_extract(data, '$.state.input.filePath') END AS path,
    CASE WHEN json_extract(data, '$.tool') IN ({}) AND json_type(data, '$.state.input.patchText') = 'text' THEN json_extract(data, '$.state.input.patchText') END AS patch
  FROM part
  WHERE time_created > ? AND json_valid(data) AND json_extract(data, '$.type') = 'tool'
    AND json_extract(data, '$.tool') IN ({})
    AND COALESCE(json_extract(data, '$.state.status'), '') <> 'error'
  ORDER BY time_created DESC LIMIT {EDITS_MAX}",
        quoted(&PATCH_TOOLS),
        quoted(&all)
    )
}

/// A database handle as node:sqlite's is used: a statement's rows, its
/// first row, and closing. Err carries what it threw.
pub trait Db {
    fn all(&mut self, sql: &str, param: Option<f64>) -> Result<Vec<Value>, String>;
    fn get(&mut self, sql: &str) -> Result<Option<Value>, String>;
    fn close(&mut self);
}

/// OpenCode's database opened read only, through rusqlite, with its values
/// as node:sqlite gives them.
pub struct Sqlite(Option<rusqlite::Connection>);

impl Sqlite {
    pub fn open(file: &str) -> Result<Sqlite, String> {
        use rusqlite::OpenFlags;
        rusqlite::Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).map(|c| Sqlite(Some(c))).map_err(|e| e.to_string())
    }
}

// A column's value as node:sqlite reads it: an integer past what a double
// holds exactly is an error, a blob is its bytes (by index, as a
// Uint8Array crosses), and a REAL that isn't finite is null (JSON's).
fn value_of(v: rusqlite::types::ValueRef<'_>) -> Result<Value, String> {
    use rusqlite::types::ValueRef;
    const MAX_SAFE: i64 = (1 << 53) - 1;
    Ok(match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) if !(-MAX_SAFE..=MAX_SAFE).contains(&i) => return Err(format!("Value is too large to be represented as a JavaScript number: {i}")),
        ValueRef::Integer(i) => Value::from(i),
        ValueRef::Real(f) => json::number(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Object(b.iter().enumerate().map(|(i, x)| (i.to_string(), Value::from(*x))).collect()),
    })
}

impl Sqlite {
    fn rows(&mut self, sql: &str, param: Option<f64>, first_only: bool) -> Result<Vec<Value>, String> {
        let conn = self.0.as_ref().ok_or("database is closed")?;
        let mut st = conn.prepare(sql).map_err(|e| e.to_string())?;
        let names: Vec<String> = st.column_names().iter().map(ToString::to_string).collect();
        let mut rows = match param {
            Some(p) => st.query([p]),
            None => st.query([]),
        }
        .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let mut m = Map::new();
            for (i, n) in names.iter().enumerate() {
                m.insert(n.clone(), value_of(row.get_ref(i).map_err(|e| e.to_string())?)?);
            }
            out.push(Value::Object(m));
            if first_only {
                break;
            }
        }
        Ok(out)
    }
}

impl Db for Sqlite {
    fn all(&mut self, sql: &str, param: Option<f64>) -> Result<Vec<Value>, String> {
        self.rows(sql, param, false)
    }
    fn get(&mut self, sql: &str) -> Result<Option<Value>, String> {
        Ok(self.rows(sql, None, true)?.into_iter().next())
    }
    fn close(&mut self) {
        self.0 = None;
    }
}

/// What's in the database since a time: its state (ok, missing, denied,
/// unreadable), the newest session's version, and the sessions and edits
/// as the vendor check gave them back.
#[derive(Clone, Debug, PartialEq)]
pub struct Read {
    pub state: &'static str,
    pub version: Option<String>,
    pub sessions: Value,
    pub edits: Value,
}

impl Read {
    fn empty(state: &'static str) -> Read {
        Read { state, version: None, sessions: json!([]), edits: json!([]) }
    }

    pub fn out(&self) -> Out {
        Out::Obj(vec![
            ("state", Out::str(self.state)),
            ("version", Out::opt_str(self.version.as_deref())),
            ("sessions", Out::Json(self.sessions.clone())),
            ("edits", Out::Json(self.edits.clone())),
        ])
    }
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

// A table's columns, as a Set of PRAGMA table_info's names.
struct Columns(Vec<Option<Value>>);

impl Columns {
    fn of(db: &mut dyn Db, table: &str) -> Result<Columns, String> {
        let mut names: Vec<Option<Value>> = Vec::new();
        for row in db.all(&format!("PRAGMA table_info({table})"), None)? {
            let n = field(&row, "name").cloned();
            if !names.contains(&n) {
                names.push(n);
            }
        }
        Ok(Columns(names))
    }
    fn has(&self, c: &str) -> bool {
        self.0.iter().any(|n| n.as_ref().and_then(Value::as_str) == Some(c))
    }
}

/// readDb: `exists` is what the twin's files said, `open` opens the
/// database (Err with what it threw). Err(Threw) when getting the vendor
/// reader threw, which readDb throws on.
pub fn read_db<'d>(exists: bool, open: &mut dyn FnMut() -> Result<Box<dyn Db + 'd>, String>, vendor: &mut dyn Vendor, since_ms: f64, edits_since_ms: f64) -> Result<Read, Threw> {
    if !exists {
        return Ok(Read::empty("missing"));
    }
    let mut db = match open() {
        Ok(db) => db,
        Err(message) => {
            let m = message.to_ascii_lowercase();
            return Ok(Read::empty(if m.contains("permission") || m.contains("eacces") || m.contains("eperm") { "denied" } else { "unreadable" }));
        }
    };
    if vendor.ready().is_err() {
        db.close();
        return Err(Threw);
    }
    let read = read_open(db.as_mut(), vendor, since_ms, edits_since_ms).unwrap_or_else(|()| Read::empty("unreadable"));
    db.close();
    Ok(read)
}

fn read_open(db: &mut dyn Db, vendor: &mut dyn Vendor, since_ms: f64, edits_since_ms: f64) -> Result<Read, ()> {
    let session = Columns::of(db, "session").map_err(drop)?;
    let part = Columns::of(db, "part").map_err(drop)?;
    // The newest session's version, by rowid when time_updated is gone, so
    // the drift below is still recorded with a version.
    let version = if session.has("version") {
        let sql = format!("SELECT version AS v FROM session ORDER BY {} DESC LIMIT 1", if session.has("time_updated") { "time_updated" } else { "rowid" });
        let row = db.get(&sql).map_err(drop)?;
        row.as_ref().and_then(|r| str_of(field(r, "v"))).and_then(clean_version).map(str::to_string)
    } else {
        None
    };
    for (t, cols, needed) in [("session", &session, &NEEDED_SESSION[..]), ("part", &part, &NEEDED_PART[..])] {
        if cols.0.is_empty() {
            vendor.missing(t, version.as_deref()).map_err(drop)?;
        } else {
            for c in needed.iter().filter(|c| !cols.has(c)) {
                vendor.missing(&format!("{t}[].{c}"), version.as_deref()).map_err(drop)?;
            }
        }
    }
    if !NEEDED_SESSION.iter().all(|c| session.has(c)) {
        return Ok(Read { version, ..Read::empty("unreadable") });
    }
    let cols: Vec<&str> = SESSION_COLUMNS.iter().copied().filter(|c| session.has(c)).collect();
    let sql = format!("SELECT {} FROM session WHERE time_updated > ? ORDER BY time_updated DESC LIMIT {SESSIONS_MAX}", cols.join(", "));
    let sessions = db.all(&sql, Some(since_ms)).map_err(drop)?;
    let mut raw = if NEEDED_PART.iter().all(|c| part.has(c)) { db.all(&edits_sql(), Some(edits_since_ms)).map_err(drop)? } else { Vec::new() };
    raw.reverse();
    let mut edits = Vec::new();
    for e in &raw {
        let edit = |path: Value| json!({ "session_id": field(e, "session_id").cloned().unwrap_or(Value::Null), "time_created": field(e, "time_created").cloned().unwrap_or(Value::Null), "tool": field(e, "tool").cloned().unwrap_or(Value::Null), "path": path });
        if truthy(field(e, "path")) {
            edits.push(edit(field(e, "path").cloned().unwrap_or(Value::Null)));
        }
        for p in patch_files(field(e, "patch")) {
            edits.push(edit(Value::String(p)));
        }
    }
    let record = json!({ "session": sessions, "edits": edits });
    let checked = match vendor.check(&record, version.as_deref()).map_err(drop)? {
        Checked::Same => record,
        Checked::Null => json!({}),
        Checked::Other(v) => v,
    };
    let or_empty = |k: &str| field(&checked, k).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!([]));
    Ok(Read { state: "ok", version, sessions: or_empty("session"), edits: or_empty("edits") })
}

/// Where OpenCode keeps its database: OPENCODE_DB (absolute, or relative to
/// its data folder), else $XDG_DATA_HOME/opencode/opencode.db, else
/// ~/.local/share/opencode/opencode.db, on every OS.
pub fn opencode_db_path(home: &str, xdg_data_home: Option<&str>, named: Option<&str>) -> String {
    let st = path_style(home);
    let data = match xdg_data_home.filter(|x| !x.is_empty()) {
        Some(x) => x.to_string(),
        None => node_path::join(st, &[home, ".local", "share"]),
    };
    let dir = node_path::join(st, &[&data, "opencode"]);
    if let Some(n) = named.filter(|n| !n.is_empty() && *n != ":memory:") {
        return if node_path::is_absolute(st, n) { n.to_string() } else { node_path::join(st, &[&dir, n]) };
    }
    node_path::join(st, &[&dir, "opencode.db"])
}

/// A session's time as the database had it (missing when the vendor check
/// took it out), or a number once an edit later than it moved it on.
#[derive(Clone, Debug, PartialEq)]
pub enum Stamp {
    Raw(Option<Value>),
    Num(f64),
}

impl Stamp {
    fn number(&self) -> f64 {
        match self {
            Stamp::Raw(v) => js_number(v.as_ref()),
            Stamp::Num(n) => *n,
        }
    }
    fn out(&self) -> Out {
        match self {
            Stamp::Raw(None) => Out::Undef,
            Stamp::Raw(Some(v)) => Out::Json(v.clone()),
            Stamp::Num(n) => Out::Num(*n),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub raw_id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub created_at: Stamp,
    pub updated_at: Stamp,
    pub archived: bool,
    pub edits: Edits,
}

// for…of over what the check gave back: an array's items, a string's
// characters, or a TypeError.
fn items(v: &Value, what: &str) -> Result<Vec<Value>, String> {
    match v {
        Value::Array(a) => Ok(a.clone()),
        Value::String(s) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        _ => Err(format!("got.{what} is not iterable")),
    }
}

/// The OpenCodeSessions block's reading.
pub struct OpenCode {
    pub runs: OrderedMap<Run>,
    pub file_state: &'static str,
    pub version: Option<String>,
}

impl Default for OpenCode {
    fn default() -> Self {
        OpenCode { runs: OrderedMap::new(), file_state: "unread", version: None }
    }
}

impl OpenCode {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a read of the database. A read that failed once (the database
    /// busy mid-write, say) keeps the last good one, so sessions don't
    /// flicker off the map; a database that's gone or can't be opened
    /// drops them. Err with a TypeError's message when the check gave back
    /// what can't be read.
    pub fn take(&mut self, got: &Read, now: f64) -> Result<(), String> {
        self.file_state = got.state;
        self.version.clone_from(&got.version);
        if got.state != "ok" {
            if got.state != "unreadable" {
                self.runs.clear();
            }
            return Ok(());
        }
        let mut runs: OrderedMap<Run> = OrderedMap::new();
        for s in items(&got.sessions, "sessions")? {
            let Some(raw) = str_of(field(&s, "id")) else { continue };
            let id = session_id_of(raw);
            let run = Run {
                id: id.clone(),
                raw_id: raw.to_string(),
                cwd: str_of(field(&s, "directory")).map(str::to_string),
                title: str_of(field(&s, "title")).filter(|t| !t.is_empty()).map(str::to_string),
                parent_id: str_of(field(&s, "parent_id")).map(session_id_of),
                created_at: Stamp::Raw(field(&s, "time_created").cloned()),
                updated_at: Stamp::Raw(field(&s, "time_updated").cloned()),
                archived: field(&s, "time_archived").is_some_and(|v| !v.is_null()),
                edits: Edits::new(),
            };
            runs.insert(&id, run);
        }
        for e in items(&got.edits, "edits")? {
            let sid = match field(&e, "session_id") {
                Some(v) => js_string(v),
                None => "undefined".into(),
            };
            let Some(run) = runs.get_mut(&session_id_of(&sid)) else { continue };
            let Some(path) = str_of(field(&e, "path")) else { continue };
            let at = js_number(field(&e, "time_created"));
            if now - at > EDIT_KEEP_MS {
                continue;
            }
            let Some(file) = edit_file(run.cwd.as_deref(), path) else { continue };
            push_edit(&mut run.edits, &file, js_min(at, now));
            run.updated_at = Stamp::Num(js_max(run.updated_at.number(), at));
        }
        self.runs = runs;
        Ok(())
    }
}

impl NativeReader for OpenCode {
    /// Reads the database at `at` (on the disk, read only; `files` says
    /// whether it's there): its sessions of the last day and their edits of
    /// the last half hour. A vendor check that threw is reported.
    fn read(&mut self, at: &str, now: f64, files: &mut dyn Files, vendor: &mut dyn Vendor) -> Result<(), String> {
        let exists = files.exists(at);
        let mut open = || Sqlite::open(at).map(|db| Box::new(db) as Box<dyn Db>);
        match read_db(exists, &mut open, vendor, now - DAY_MS, now - EDIT_KEEP_MS) {
            Ok(got) => self.take(&got, now),
            Err(Threw) => Err("the vendor check failed".into()),
        }
    }

    fn source_state(&self) -> &'static str {
        self.file_state
    }

    fn ids(&self) -> Vec<String> {
        self.runs.keys().map(str::to_string).collect()
    }

    fn sessions(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                RunSession {
                    id: &r.id,
                    app: "opencode",
                    raw_id: &r.raw_id,
                    title: r.title.clone().unwrap_or_else(|| titled("OpenCode", r.cwd.as_deref())),
                    cwd: r.cwd.as_deref(),
                    created_at: r.created_at.out(),
                    updated_at: r.updated_at.out(),
                    archived: r.archived,
                    model: None,
                    branch: None,
                    subagent_of: r.parent_id.as_deref(),
                    via: Via::Reader,
                }
                .record()
            })
            .collect()
    }

    fn runs_out(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("rawId", Out::str(&r.raw_id)),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("title", Out::opt_str(r.title.as_deref())),
                    ("parentId", Out::opt_str(r.parent_id.as_deref())),
                    ("createdAt", r.created_at.out()),
                    ("updatedAt", r.updated_at.out()),
                    ("archived", Out::bool(r.archived)),
                    ("edits", edits_out(&r.edits)),
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Plain(Vec<String>);
    impl Vendor for Plain {
        fn ready(&mut self) -> Result<(), Threw> {
            Ok(())
        }
        fn missing(&mut self, path: &str, _: Option<&str>) -> Result<(), Threw> {
            self.0.push(path.to_string());
            Ok(())
        }
        fn check(&mut self, _: &Value, _: Option<&str>) -> Result<Checked, Threw> {
            Ok(Checked::Same)
        }
    }

    fn db(sql: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("xenonet-opencode-rs-{}-{}", std::process::id(), sql.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("opencode.db");
        let _ = std::fs::remove_file(&file);
        rusqlite::Connection::open(&file).unwrap().execute_batch(sql).unwrap();
        (dir, file.to_string_lossy().into_owned())
    }

    #[test]
    fn sessions_and_edits_from_the_database() {
        let now = 1_800_000_000_000.0_f64;
        let tool = |name: &str, input: &str, status: &str| format!(r#"{{"type":"tool","tool":"{name}","state":{{"status":"{status}","input":{input},"output":"SECRET"}}}}"#);
        let sql = format!(
            "CREATE TABLE session (id text, parent_id text, directory text, title text, version text, time_created integer, time_updated integer, time_archived integer);
             CREATE TABLE part (id text, session_id text, time_created integer, data text);
             INSERT INTO session VALUES ('ses_a', NULL, '/w/atlas', 'Overwrite notes', '1.19.3', {c}, {u}, NULL);
             INSERT INTO part VALUES ('p1', 'ses_a', {t1}, '{w}'), ('p2', 'ses_a', {t2}, '{p}'), ('p3', 'ses_a', {t3}, '{f}'), ('p4', 'ses_a', {t3}, 'not json');",
            c = now - 600e3,
            u = now - 60e3,
            t1 = now - 120e3,
            t2 = now - 100e3,
            t3 = now - 90e3,
            w = tool("write", r#"{"filePath":"src/app.ts","content":"SECRET"}"#, "completed"),
            p = tool("apply_patch", r#"{"patchText":"*** Begin Patch\n*** Update File: src/lib.ts\n-SECRET\n*** End Patch"}"#, "completed"),
            f = tool("write", r#"{"filePath":"/w/atlas/failed.txt"}"#, "error"),
        );
        let (dir, file) = db(&sql);
        let mut v = Plain(Vec::new());
        let got = read_db(true, &mut || Sqlite::open(&file).map(|d| Box::new(d) as Box<dyn Db>), &mut v, now - 86400e3, now - EDIT_KEEP_MS).unwrap();
        assert_eq!((got.state, got.version.as_deref()), ("ok", Some("1.19.3")));
        assert!(!got.edits.to_string().contains("SECRET"));
        let mut o = OpenCode::new();
        o.take(&got, now).unwrap();
        let r = o.runs.get("opencode:ses-a").unwrap();
        assert_eq!(r.edits.keys().collect::<Vec<_>>(), ["/w/atlas/src/app.ts", "/w/atlas/src/lib.ts"]);
        assert_eq!(r.updated_at, Stamp::Num(now - 60e3));
        assert_eq!(v.0, Vec::<String>::new());
        std::fs::remove_dir_all(&dir).unwrap();
        let (dir, file) = db("CREATE TABLE session (id text, title text);");
        let got = read_db(true, &mut || Sqlite::open(&file).map(|d| Box::new(d) as Box<dyn Db>), &mut v, 0.0, 0.0).unwrap();
        assert_eq!(got.state, "unreadable");
        assert_eq!(v.0, ["session[].directory", "session[].time_created", "session[].time_updated", "part"]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(read_db(false, &mut || Err("never".into()), &mut v, 0.0, 0.0).unwrap().state, "missing");
        assert_eq!(read_db(true, &mut || Err("EACCES: permission denied".into()), &mut v, 0.0, 0.0).unwrap().state, "denied");
    }

    #[test]
    fn where_the_database_is() {
        assert_eq!(opencode_db_path("/home/ada", None, None), "/home/ada/.local/share/opencode/opencode.db");
        assert_eq!(opencode_db_path("/home/ada", Some("/data"), None), "/data/opencode/opencode.db");
        assert_eq!(opencode_db_path("/home/ada", None, Some("/x/o.db")), "/x/o.db");
        assert_eq!(opencode_db_path("/home/ada", None, Some("o.db")), "/home/ada/.local/share/opencode/o.db");
        assert_eq!(opencode_db_path("/home/ada", None, Some(":memory:")), "/home/ada/.local/share/opencode/opencode.db");
        assert_eq!(patch_files(Some(&Value::from("*** Begin Patch\n*** Add File: a.txt\n*** Move to: c.txt\n*** End Patch"))), ["a.txt", "c.txt"]);
        assert_eq!(patch_files(None), Vec::<String>::new());
    }
}
