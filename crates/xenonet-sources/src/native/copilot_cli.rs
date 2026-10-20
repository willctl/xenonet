//! core/native/copilot-cli.js: GitHub Copilot CLI's session events. Each
//! session's event log (`~/.copilot/session-state/<id>/events.jsonl`, or
//! `<id>.jsonl` before 1.0.11) is a session on the map, "`copilot:<id>`", with
//! its edit tools' files and its shutdown's changed files as its edits.

use super::files::Files;
use super::node_path;
use super::{by_num, edit_file, edits_out, fits_id, js_max, js_min, ms_of, push_edit, str_of, titled, Check, Checked, Edits, Out, Threw, DAY_MS, EDIT_KEEP_MS, SESSIONS_MAX};
use crate::session::{RunSession, Via};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_base::json::{self, number_string};
use xenonet_base::jsstr::is_array_index;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::patch;
use xenonet_base::paths::PathStyle;
use xenonet_base::text::{is_blank, truthy};

pub const SOURCE: &str = "github/copilot-cli/session-event";
/// Copilot CLI's built-in file tools; str_replace_editor's view isn't an edit.
pub const EDIT_TOOLS: [&str; 5] = ["str_replace_editor", "edit", "create", "write", "apply_patch"];
const FILE_MAX: f64 = (32u64 << 20) as f64;
const CHECKS_PER_FILE: usize = 16;

pub fn session_id_of(id: &str) -> Option<String> {
    fits_id(id, 128).then(|| format!("copilot:{id}"))
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

// Object.values: an object's values in JS's key order, array indexes first.
fn js_values(m: &Map<String, Value>) -> Vec<&Value> {
    let mut indexes: Vec<(u64, &Value)> = Vec::new();
    let mut rest = Vec::new();
    for (k, v) in m {
        if is_array_index(k) {
            indexes.push((k.parse().unwrap_or(0), v));
        } else {
            rest.push(v);
        }
    }
    indexes.sort_by_key(|x| x.0);
    indexes.into_iter().map(|x| x.1).chain(rest).collect()
}

/// The files a tool call names, if it's an edit.
pub fn edited_by(tool: Option<&Value>, args: Option<&Value>) -> Vec<String> {
    let Some(tool) = str_of(tool).filter(|t| EDIT_TOOLS.contains(t)) else { return Vec::new() };
    let args = match args {
        Some(Value::String(s)) => return if tool == "apply_patch" { patch::paths(s, true) } else { Vec::new() },
        Some(a @ (Value::Object(_) | Value::Array(_))) => a,
        _ => return Vec::new(),
    };
    if tool == "str_replace_editor" && field(args, "command").and_then(Value::as_str) == Some("view") {
        return Vec::new();
    }
    let mut out: Vec<String> = ["path", "file_path", "filePath"].iter().filter_map(|k| str_of(field(args, k)).filter(|p| !p.is_empty()).map(str::to_string)).collect();
    if tool == "apply_patch" {
        let values: Vec<&Value> = match args {
            Value::Object(m) => js_values(m),
            Value::Array(a) => a.iter().collect(),
            _ => Vec::new(),
        };
        for v in values {
            if let Value::String(s) = v {
                out.extend(patch::paths(s, true));
            }
        }
    }
    out
}

/// An edit as the log has it: the file as written, when, and the folder
/// the session was in then.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEdit {
    pub path: String,
    pub at: Option<f64>,
    pub cwd: Option<String>,
}

/// One session's event log as XenoNet reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    pub id: String,
    /// session.start's version, as it's written (None: there's none).
    pub version: Option<Value>,
    pub started_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub title: Option<String>,
    pub edits: Vec<LogEdit>,
}

/// A session's event log, or None when it has no session.start. `check`
/// sees the first few session events and tool starts, and what it gives
/// back is read on (Threw when it threw, which ends the read).
pub fn read_events(text: &str, check: &mut dyn FnMut(&Value) -> Result<Checked, Threw>) -> Result<Option<Log>, Threw> {
    let mut start: Option<(String, Option<f64>, Option<Value>)> = None;
    let (mut cwd, mut branch, mut title): (Option<String>, Option<String>, Option<String>) = (None, None, None);
    let mut ended_at = None;
    let mut last: Option<f64> = None;
    let mut edits = Vec::new();
    let mut checked = 0;
    for line in text.split('\n') {
        if is_blank(line) {
            continue;
        }
        let Ok(mut e) = json::parse(line) else { continue };
        let Some(ty) = field(&e, "type").and_then(Value::as_str) else { continue };
        if checked < CHECKS_PER_FILE && (ty.starts_with("session.") || ty == "tool.execution_start") {
            if let Checked::Other(v) = check(&e)? {
                e = v;
            }
            checked += 1;
        }
        let at = ms_of(field(&e, "timestamp"));
        if let Some(a) = at {
            last = Some(js_max(last.unwrap_or(0.0), a));
        }
        let d = field(&e, "data").filter(|d| d.is_object() || d.is_array());
        let get = |k: &str| d.and_then(|d| field(d, k));
        let context = |k: &str| str_of(get("context").and_then(|c| field(c, k))).map(str::to_string);
        match field(&e, "type").and_then(Value::as_str) {
            Some("session.start") if str_of(get("sessionId")).is_some() => {
                start = Some((str_of(get("sessionId")).unwrap_or_default().to_string(), ms_of(get("startTime")).or(at), get("version").cloned()));
                if let Some(c) = context("cwd") {
                    cwd = Some(c);
                }
                if let Some(b) = context("branch") {
                    branch = Some(b);
                }
            }
            Some("session.resume") => {
                ended_at = None;
                if let Some(c) = context("cwd") {
                    cwd = Some(c);
                }
            }
            Some("session.context_changed") => {
                if let Some(c) = str_of(get("cwd")) {
                    cwd = Some(c.to_string());
                }
                if let Some(b) = str_of(get("branch")) {
                    branch = Some(b.to_string());
                }
            }
            Some("session.title_changed") if str_of(get("title")).is_some() => title = str_of(get("title")).map(str::to_string),
            Some("session.shutdown") => {
                ended_at = at;
                if let Some(Value::Array(files)) = get("codeChanges").and_then(|c| field(c, "filesModified")) {
                    for f in files.iter().filter_map(Value::as_str) {
                        edits.push(LogEdit { path: f.to_string(), at, cwd: cwd.clone() });
                    }
                }
            }
            Some("tool.execution_start") if !truthy(field(&e, "ephemeral")) => {
                for f in edited_by(get("toolName"), get("arguments")) {
                    edits.push(LogEdit { path: f, at, cwd: cwd.clone() });
                }
            }
            _ => {}
        }
    }
    let Some((id, start_at, version)) = start else { return Ok(None) };
    Ok(Some(Log { id, version, started_at: start_at.or(last), updated_at: last.or(start_at), ended_at, cwd, branch, title, edits }))
}

/// ~/.copilot/session-state, as path.join makes it.
pub fn copilot_root(home: &str) -> String {
    node_path::join(PathStyle::Native, &[home, ".copilot", "session-state"])
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub raw_id: String,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub title: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub ended_at: Option<f64>,
    pub edits: Edits,
}

struct Cached {
    mtime_ms: f64,
    size: f64,
    log: Option<Log>,
}

struct Found {
    file: String,
    mtime_ms: f64,
    size: f64,
}

/// The CopilotCliSessions block's reading: its runs, the logs it read
/// (until they change), and whether it could read the folder.
pub struct CopilotCli {
    pub runs: OrderedMap<Run>,
    cache: HashMap<String, Cached>,
    pub root_state: &'static str,
}

impl Default for CopilotCli {
    fn default() -> Self {
        CopilotCli { runs: OrderedMap::new(), cache: HashMap::new(), root_state: "unread" }
    }
}

impl CopilotCli {
    pub fn new() -> Self {
        Self::default()
    }

    // The logs changed in the last day, newest first.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
    fn found(&mut self, root: Option<&str>, now: f64, files: &mut dyn Files) -> Vec<Found> {
        let Some(root) = root else {
            self.root_state = "unreadable";
            return Vec::new();
        };
        let entries = match files.read_dir(root) {
            Ok(e) => e,
            Err(err) => {
                self.root_state = if err.is("ENOENT") { "missing" } else { "unreadable" };
                return Vec::new();
            }
        };
        self.root_state = "ok";
        let mut out = Vec::new();
        for e in entries {
            let file = if e.is_dir {
                node_path::join(PathStyle::Native, &[root, &e.name, "events.jsonl"])
            } else if e.is_file && e.name.ends_with(".jsonl") {
                node_path::join(PathStyle::Native, &[root, &e.name])
            } else {
                continue;
            };
            if let Ok(st) = files.stat(&file) {
                if now - st.mtime_ms <= DAY_MS && st.size <= FILE_MAX {
                    out.push(Found { file, mtime_ms: st.mtime_ms, size: st.size });
                }
            }
        }
        by_num(&mut out, |f| -f.mtime_ms);
        out.truncate(SESSIONS_MAX);
        out
    }

    /// Reads what changed. `check` is the vendor reader's, given the
    /// version session.start says.
    pub fn read(&mut self, root: Option<&str>, now: f64, files: &mut dyn Files, check: &mut Check<'_>) {
        let found = self.found(root, now, files);
        let mut runs = OrderedMap::new();
        let mut seen = HashSet::new();
        for f in found {
            seen.insert(f.file.clone());
            let fresh = self.cache.get(&f.file).is_some_and(|h| h.mtime_ms == f.mtime_ms && h.size == f.size);
            if !fresh {
                let log = files.read_text(&f.file).ok().and_then(|text| {
                    let mut version: Option<String> = None;
                    let mut checking = |rec: &Value| {
                        if field(rec, "type").and_then(Value::as_str) == Some("session.start") {
                            if let Some(n) = field(rec, "data").and_then(|d| field(d, "version")).and_then(Value::as_f64) {
                                version = Some(number_string(n));
                            }
                        }
                        check(rec, version.as_deref())
                    };
                    read_events(&text, &mut checking).ok().flatten()
                });
                self.cache.insert(f.file.clone(), Cached { mtime_ms: f.mtime_ms, size: f.size, log });
            }
            let Some(l) = self.cache.get(&f.file).and_then(|h| h.log.as_ref()) else { continue };
            let Some(id) = session_id_of(&l.id) else { continue };
            let mut run = Run {
                id: id.clone(),
                raw_id: l.id.clone(),
                cwd: l.cwd.clone(),
                branch: l.branch.clone(),
                title: l.title.clone(),
                created_at: l.started_at.unwrap_or(f.mtime_ms),
                updated_at: l.updated_at.unwrap_or(f.mtime_ms),
                ended_at: l.ended_at,
                edits: Edits::new(),
            };
            for e in &l.edits {
                let at = js_min(e.at.unwrap_or(run.updated_at), now);
                if now - at > EDIT_KEEP_MS {
                    continue;
                }
                let base = e.cwd.as_deref().or(run.cwd.as_deref());
                if let Some(file) = edit_file(base, &e.path) {
                    push_edit(&mut run.edits, &file, at);
                }
            }
            runs.insert(&id, run);
        }
        self.cache.retain(|k, _| seen.contains(k));
        self.runs = runs;
    }

    pub fn sessions(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                RunSession {
                    id: &r.id,
                    app: "copilot",
                    raw_id: &r.raw_id,
                    title: r.title.clone().unwrap_or_else(|| titled("Copilot CLI", r.cwd.as_deref())),
                    cwd: r.cwd.as_deref(),
                    created_at: Out::Num(r.created_at),
                    updated_at: Out::Num(r.updated_at),
                    archived: false,
                    model: None,
                    branch: r.branch.as_deref(),
                    subagent_of: None,
                    via: Via::Reader,
                }
                .record()
            })
            .collect()
    }

    /// The runs as the block's `runs` Map holds them, edits as pairs.
    pub fn runs_out(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("rawId", Out::str(&r.raw_id)),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("branch", Out::opt_str(r.branch.as_deref())),
                    ("title", Out::opt_str(r.title.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("endedAt", Out::opt_num(r.ended_at)),
                    ("edits", edits_out(&r.edits)),
                ])
            })
            .collect()
    }
}

/// A log as readEvents gives it.
pub fn log_out(l: &Log) -> Out {
    Out::Obj(vec![
        ("id", Out::str(&l.id)),
        ("version", l.version.clone().map_or(Out::Undef, Out::Json)),
        ("startedAt", Out::opt_num(l.started_at)),
        ("updatedAt", Out::opt_num(l.updated_at)),
        ("endedAt", Out::opt_num(l.ended_at)),
        ("cwd", Out::opt_str(l.cwd.as_deref())),
        ("branch", Out::opt_str(l.branch.as_deref())),
        ("title", Out::opt_str(l.title.as_deref())),
        ("edits", Out::Arr(l.edits.iter().map(|e| Out::Obj(vec![("path", Out::str(&e.path)), ("at", Out::opt_num(e.at)), ("cwd", Out::opt_str(e.cwd.as_deref()))])).collect())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::files::StdFiles;
    use crate::native::no_check;

    fn lines(rs: &[Value]) -> String {
        rs.iter().map(ToString::to_string).collect::<Vec<_>>().join("\n") + "\n"
    }

    #[test]
    fn edits_are_edit_tools_files() {
        let j = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(edited_by(Some(&j("\"create\"")), Some(&j(r#"{"path":"/a"}"#))), ["/a"]);
        assert_eq!(edited_by(Some(&j("\"apply_patch\"")), Some(&j("\"*** Begin Patch\\n*** Update File: b.ts\\n*** End Patch\""))), ["b.ts"]);
        assert_eq!(edited_by(Some(&j("\"view\"")), Some(&j(r#"{"path":"/a"}"#))), Vec::<String>::new());
        assert_eq!(edited_by(Some(&j("\"str_replace_editor\"")), Some(&j(r#"{"command":"view","path":"/a"}"#))), Vec::<String>::new());
        assert_eq!(edited_by(Some(&j("\"apply_patch\"")), Some(&j(r#"{"b":"*** Add File: x","1":"*** Add File: y","path":"/p"}"#))), ["/p", "y", "x"]);
    }

    #[test]
    fn a_log_is_its_start_title_and_edits() {
        let text = lines(&[
            serde_json::json!({ "type": "session.start", "timestamp": "2026-10-16T11:50:00Z", "data": { "sessionId": "0f3c2b1a-1111", "version": 1, "startTime": "2026-10-16T11:50:00Z", "context": { "cwd": "/w/atlas", "branch": "main" } } }),
            serde_json::json!({ "type": "session.title_changed", "timestamp": "2026-10-16T11:51:00Z", "data": { "title": "Partial refunds" } }),
            serde_json::json!({ "type": "tool.execution_start", "timestamp": "2026-10-16T11:58:00Z", "data": { "toolName": "edit", "arguments": { "path": "src/app.ts" } } }),
            serde_json::json!({ "type": "tool.execution_start", "ephemeral": true, "timestamp": "2026-10-16T11:58:00Z", "data": { "toolName": "edit", "arguments": { "path": "eph.ts" } } }),
            serde_json::json!({ "type": "session.shutdown", "timestamp": "2026-10-16T11:59:00Z", "data": { "codeChanges": { "filesModified": ["/w/atlas/notes.txt"] } } }),
        ]);
        let l = read_events(&text, &mut |_| Ok(Checked::Same)).unwrap().unwrap();
        assert_eq!(l.id, "0f3c2b1a-1111");
        assert_eq!(l.version, Some(serde_json::json!(1)));
        assert_eq!((l.title.as_deref(), l.cwd.as_deref(), l.branch.as_deref()), (Some("Partial refunds"), Some("/w/atlas"), Some("main")));
        assert_eq!(l.edits.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["src/app.ts", "/w/atlas/notes.txt"]);
        assert_eq!(l.ended_at, xenonet_base::time::parse_iso("2026-10-16T11:59:00Z"));
        assert_eq!(read_events("{\"type\":\"user.message\"}\n", &mut |_| Ok(Checked::Same)).unwrap(), None);
        assert_eq!(read_events(&text, &mut |_| Err(Threw)), Err(Threw));
    }

    #[test]
    fn the_folder_is_read() {
        let home = std::env::temp_dir().join(format!("xenonet-copilot-rs-{}", std::process::id()));
        let root = copilot_root(&home.to_string_lossy());
        std::fs::create_dir_all(format!("{root}/0f3c2b1a-1111")).unwrap();
        std::fs::write(
            format!("{root}/0f3c2b1a-1111/events.jsonl"),
            lines(&[serde_json::json!({ "type": "session.start", "data": { "sessionId": "0f3c2b1a-1111", "context": { "cwd": "/w/atlas" } } })]),
        )
        .unwrap();
        let now = xenonet_base::fs::mtime_ms_of(&std::fs::metadata(format!("{root}/0f3c2b1a-1111/events.jsonl")).unwrap()).unwrap();
        let mut c = CopilotCli::new();
        c.read(Some(&root), now, &mut StdFiles, &mut no_check);
        assert_eq!(c.root_state, "ok");
        assert_eq!(c.runs.keys().collect::<Vec<_>>(), ["copilot:0f3c2b1a-1111"]);
        let mut gone = CopilotCli::new();
        gone.read(Some(&format!("{root}/none")), now, &mut StdFiles, &mut no_check);
        assert_eq!(gone.root_state, "missing");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
