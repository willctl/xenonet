//! core/numbat.js: numbat's event records. Each run numbat records, one per
//! numbat session id, is a session on the map, "<app>:<id>", with app the
//! agent that ran it, marked observed through numbat; its edits count in
//! the warnings other sessions get, marked observed. The file numbat writes
//! is tailed: new whole lines are read once, and a file rotated or cut short
//! is read again from the start.
//!
//! From each record only the agent, the session (and its parent), the event
//! type, the file edited, the project, branch, model and time are kept;
//! never the endpoint, commands, prompts, previews, tool input or results.

use crate::native::files::Files;
use crate::native::node_path;
use crate::native::{by_num, edits_out, fits_id, js_max, js_min, older, prune_edits, push_edit, titled, Edits, Out, DAY_MS, SESSIONS_MAX};
use serde_json::Value;
use sha2::{Digest, Sha256};
use xenonet_base::fs::FsError;
use xenonet_base::ids::{is_session_app, SESSION_APPS};
use xenonet_base::json::{self, number_string};
use xenonet_base::jsstr;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::path_style;
use xenonet_base::text::{is_blank, js_string};

pub const SCHEMA_VERSIONS: [&str; 4] = ["0.2.0", "0.3.0", "0.4.0", "0.5.0"];
/// Apps XenoNet reads itself, and numbat's names that aren't a run's app.
pub const LEFT_OUT: [&str; 5] = ["claude-code", "codex", "hermes", "grok", "unknown"];
const EDITS: [&str; 2] = ["file.write", "file.delete"];
// A first read takes at most the last MiB, and each read at most 4 MiB.
const START_BACK: f64 = (1u64 << 20) as f64;
const READ_MAX: f64 = (4u64 << 20) as f64;
const LINE_MAX: usize = 1 << 20;

/// A run's id as XenoNet's session ids take it: numbat's own when it fits,
/// else a stable digest of it.
pub fn run_id(sid: &str) -> String {
    if fits_id(sid, 128) {
        return sid.to_string();
    }
    let hex = crate::native::hex(&Sha256::digest(sid.as_bytes()));
    format!("n{}", &hex[..31])
}

pub fn session_id_of(app: &str, sid: &str) -> String {
    format!("{app}:{}", run_id(sid))
}

/// One event, as XenoNet keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub app: String,
    pub kind: String,
    pub session: String,
    pub parent: Option<String>,
    pub at: Option<f64>,
    pub project: Option<String>,
    pub file: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
}

fn nonempty(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

fn absolute(p: &str) -> bool {
    !p.is_empty() && node_path::is_absolute(path_style(p), p)
}

/// One NDJSON line: the event, or why it's passed over (malformed,
/// not-an-event, unsupported-version, left-out, no-session).
pub fn read_record(line: &str) -> Result<Event, &'static str> {
    let Ok(Value::Object(r)) = json::parse(line) else { return Err("malformed") };
    if r.get("record_type").and_then(Value::as_str) != Some("event") {
        return Err("not-an-event");
    }
    if !r.get("schema_version").and_then(Value::as_str).is_some_and(|v| SCHEMA_VERSIONS.contains(&v)) {
        return Err("unsupported-version");
    }
    let (Some(app), Some(kind)) = (nonempty(r.get("source_agent")), nonempty(r.get("event_type"))) else { return Err("malformed") };
    if LEFT_OUT.contains(&app.as_str()) || !is_session_app(&app) {
        return Err("left-out");
    }
    let Some(session) = nonempty(r.get("session_id")) else { return Err("no-session") };
    let at = match r.get("timestamp") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => xenonet_base::time::parse_iso(s),
        Some(other) => xenonet_base::time::parse_iso(&js_string(other)),
    };
    let project = nonempty(r.get("project_path"));
    let mut file = if EDITS.contains(&kind.as_str()) { nonempty(r.get("file_path")) } else { None };
    if let Some(f) = file.clone().filter(|f| !absolute(f)) {
        file = project.as_deref().filter(|p| absolute(p)).map(|p| node_path::join(path_style(p), &[p, &f]));
    }
    let file = file.map(|f| node_path::resolve(path_style(&f), &[&f]));
    Ok(Event {
        app,
        kind,
        session,
        parent: nonempty(r.get("parent_session_id")),
        at,
        project: project.filter(|p| absolute(p)),
        file,
        branch: nonempty(r.get("git_branch")),
        model: nonempty(r.get("model")),
    })
}

/// An event as readRecord gives it.
pub fn event_out(e: &Event) -> Out {
    Out::Obj(vec![
        ("app", Out::str(&e.app)),
        ("type", Out::str(&e.kind)),
        ("session", Out::str(&e.session)),
        ("parent", Out::opt_str(e.parent.as_deref())),
        ("at", Out::opt_num(e.at)),
        ("project", Out::opt_str(e.project.as_deref())),
        ("file", Out::opt_str(e.file.as_deref())),
        ("branch", Out::opt_str(e.branch.as_deref())),
        ("model", Out::opt_str(e.model.as_deref())),
    ])
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub app: String,
    pub raw_id: String,
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub parent_id: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub ended_at: Option<f64>,
    pub edits: Edits,
}

/// The NumbatSessions block's reading: where it is in the file, the line
/// it has half of, its runs, and what it passed over.
pub struct Numbat {
    pub offset: Option<f64>,
    pub ino: Option<f64>,
    pub partial: String,
    pub runs: OrderedMap<Run>,
    pub file_state: &'static str,
    pub skipped: f64,
}

impl Default for Numbat {
    fn default() -> Self {
        Numbat { offset: None, ino: None, partial: String::new(), runs: OrderedMap::new(), file_state: "unread", skipped: 0.0 }
    }
}

impl Numbat {
    pub fn new() -> Self {
        Self::default()
    }

    // The new whole lines since the last read. Err when the read failed
    // (what it changed before then stays changed).
    fn lines(&mut self, file: Option<&str>, files: &mut dyn Files) -> Result<Vec<String>, FsError> {
        let Some(Ok(st)) = file.map(|f| files.stat(f)) else {
            self.file_state = "missing";
            return Ok(Vec::new());
        };
        let file = file.unwrap_or_default();
        let mut skip_first = false;
        let rotated = match (self.ino, st.ino) {
            (Some(was), now) => now != Some(was) || now.is_some_and(f64::is_nan),
            (None, _) => false,
        };
        if self.offset.is_none_or(|o| st.size < o) || rotated {
            let from = if self.offset.is_none() { js_max(0.0, st.size - START_BACK) } else { 0.0 };
            skip_first = from > 0.0;
            self.offset = Some(from);
            self.partial.clear();
        }
        self.ino = st.ino;
        self.file_state = "ok";
        let offset = self.offset.unwrap_or(0.0);
        let len = js_min(st.size - offset, READ_MAX);
        if len <= 0.0 {
            return Ok(Vec::new());
        }
        if !len.is_finite() {
            return Err(FsError { code: Some("ERR_OUT_OF_RANGE".into()), message: format!("The value of \"size\" is out of range. Received {}", number_string(len)) });
        }
        let buf = files.read_at(file, offset, len as usize)?;
        self.offset = Some(offset + len);
        let text = format!("{}{}", self.partial, String::from_utf8_lossy(&buf));
        let mut lines: Vec<&str> = text.split('\n').collect();
        self.partial = lines.pop().unwrap_or_default().to_string();
        if jsstr::len(&self.partial) > LINE_MAX {
            self.partial.clear();
        }
        if skip_first && !lines.is_empty() {
            lines.remove(0);
        }
        Ok(lines.into_iter().filter(|l| !is_blank(l) && jsstr::len(l) <= LINE_MAX).map(str::to_string).collect())
    }

    /// Reads what's new. Err with the message of a read that failed; a
    /// missing file is said in `file_state`.
    pub fn read(&mut self, file: Option<&str>, now: f64, files: &mut dyn Files) -> Result<(), String> {
        for line in self.lines(file, files).map_err(|e| e.message)? {
            match read_record(&line) {
                Ok(e) => self.take(e, now),
                Err(_) => self.skipped += 1.0,
            }
        }
        self.prune(now);
        Ok(())
    }

    fn take(&mut self, e: Event, now: f64) {
        let when = js_min(e.at.unwrap_or(now), now);
        if now - when > DAY_MS {
            return;
        }
        let id = session_id_of(&e.app, &e.session);
        if self.runs.get(&id).is_none() {
            let run = Run {
                id: id.clone(),
                app: e.app.clone(),
                raw_id: e.session.clone(),
                cwd: None,
                branch: None,
                model: None,
                parent_id: None,
                created_at: when,
                updated_at: when,
                ended_at: None,
                edits: Edits::new(),
            };
            self.runs.insert(&id, run);
        }
        let run = self.runs.get_mut(&id).expect("just added");
        run.created_at = js_min(run.created_at, when);
        run.updated_at = js_max(run.updated_at, when);
        if let Some(p) = e.project {
            if run.cwd.as_deref().is_none_or(str::is_empty) {
                run.cwd = Some(p);
            }
        }
        if e.branch.is_some() {
            run.branch = e.branch;
        }
        if e.model.is_some() {
            run.model = e.model;
        }
        if let Some(p) = e.parent.filter(|p| *p != e.session) {
            run.parent_id = Some(session_id_of(&e.app, &p));
        }
        if e.kind == "session.end" {
            run.ended_at = Some(when);
        } else if run.ended_at.is_some_and(|end| when > end) {
            run.ended_at = None;
        }
        if let Some(f) = e.file {
            push_edit(&mut run.edits, &f, when);
        }
    }

    fn prune(&mut self, now: f64) {
        self.runs.retain(|_, r| !older(now, r.updated_at, DAY_MS));
        for (_, r) in self.runs.iter_mut() {
            prune_edits(&mut r.edits, now);
        }
        if self.runs.len() > SESSIONS_MAX {
            let mut by_age: Vec<(f64, String)> = self.runs.iter().map(|(k, r)| (r.updated_at, k.to_string())).collect();
            by_num(&mut by_age, |x| x.0);
            let extra = self.runs.len() - SESSIONS_MAX;
            for (_, k) in by_age.into_iter().take(extra) {
                self.runs.remove(&k);
            }
        }
    }

    /// The runs as session/v1, for the map.
    pub fn sessions(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                let name = SESSION_APPS.iter().find(|(a, _)| *a == r.app).map_or("", |(_, n)| *n);
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("app", Out::str(&r.app)),
                    ("rawId", Out::str(&r.raw_id)),
                    ("title", Out::str(&titled(name, r.cwd.as_deref()))),
                    ("preview", Out::null()),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("archived", Out::bool(false)),
                    ("model", Out::opt_str(r.model.as_deref())),
                    ("branch", Out::opt_str(r.branch.as_deref())),
                    ("parentId", Out::opt_str(r.parent_id.as_deref())),
                    ("parentKind", if r.parent_id.is_some() { Out::str("subagent") } else { Out::null() }),
                    ("observed", Out::str("numbat")),
                ])
            })
            .collect()
    }

    pub fn runs_out(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("app", Out::str(&r.app)),
                    ("rawId", Out::str(&r.raw_id)),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("branch", Out::opt_str(r.branch.as_deref())),
                    ("model", Out::opt_str(r.model.as_deref())),
                    ("parentId", Out::opt_str(r.parent_id.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("endedAt", Out::opt_num(r.ended_at)),
                    ("edits", edits_out(&r.edits)),
                ])
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::files::StdFiles;
    use serde_json::json;

    fn event(fields: Value) -> String {
        let mut e =
            json!({ "schema_version": "0.5.0", "record_type": "event", "endpoint": { "hostname": "made-up-host" }, "source_agent": "gemini-cli", "event_type": "tool.call" });
        for (k, v) in fields.as_object().unwrap() {
            e[k] = v.clone();
        }
        e.to_string()
    }

    #[test]
    fn records() {
        let ok = read_record(&event(json!({ "event_type": "file.write", "project_path": "/w/atlas", "file_path": "src/a.js", "session_id": "s-0000001" }))).unwrap();
        assert_eq!(ok.file.as_deref(), Some("/w/atlas/src/a.js"));
        assert_eq!(read_record("{not json"), Err("malformed"));
        assert_eq!(read_record(&json!({ "record_type": "finding", "schema_version": "0.5.0" }).to_string()), Err("not-an-event"));
        assert_eq!(read_record(&event(json!({ "schema_version": "1.0.0", "session_id": "s" }))), Err("unsupported-version"));
        assert_eq!(read_record(&event(json!({}))), Err("no-session"));
        for agent in ["claude-code", "codex", "hermes", "unknown", "toaster"] {
            assert_eq!(read_record(&event(json!({ "source_agent": agent, "session_id": "s" }))), Err("left-out"), "{agent}");
        }
        assert_eq!(read_record(&event(json!({ "event_type": "file.read", "file_path": "/x", "session_id": "s" }))).unwrap().file, None);
        let win = read_record(&event(json!({ "event_type": "file.write", "project_path": "C:\\w\\atlas", "file_path": "src\\a.js", "session_id": "s" }))).unwrap();
        assert_eq!(win.file.as_deref(), Some("C:\\w\\atlas\\src\\a.js"));
        let id = session_id_of("cursor", "has spaces/and slashes");
        assert!(id.starts_with("cursor:n") && id.len() == "cursor:n".len() + 31, "{id}");
        assert_eq!(run_id("g-0000001"), "g-0000001");
    }

    #[test]
    fn the_file_is_tailed() {
        let dir = std::env::temp_dir().join(format!("xenonet-numbat-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("records.ndjson").to_string_lossy().into_owned();
        let now = xenonet_base::time::parse_iso("2026-10-16T12:00:00Z").unwrap();
        let line = |f: &str| {
            event(
                json!({ "event_type": "file.write", "session_id": "g-0000002", "project_path": "/w/atlas", "file_path": format!("/w/atlas/{f}"), "timestamp": "2026-10-16T12:00:00.000Z" }),
            )
        };
        std::fs::write(&file, "").unwrap();
        let mut n = Numbat::new();
        n.read(Some(&file), now, &mut StdFiles).unwrap();
        assert_eq!(n.file_state, "ok");
        let a = line("a.js");
        std::fs::write(&file, &a[..40]).unwrap();
        n.read(Some(&file), now, &mut StdFiles).unwrap();
        assert_eq!(n.runs.len(), 0, "half a line waits");
        std::fs::write(&file, format!("{a}\n")).unwrap();
        n.read(Some(&file), now, &mut StdFiles).unwrap();
        assert_eq!(n.runs.get("gemini-cli:g-0000002").unwrap().edits.keys().collect::<Vec<_>>(), ["/w/atlas/a.js"]);
        std::fs::rename(&file, format!("{file}.1")).unwrap();
        std::fs::write(&file, format!("{}\n", line("b.js"))).unwrap();
        n.read(Some(&file), now, &mut StdFiles).unwrap();
        assert_eq!(n.runs.get("gemini-cli:g-0000002").unwrap().edits.len(), 2);
        n.read(Some(&file), now + 25.0 * 3600e3, &mut StdFiles).unwrap();
        assert_eq!(n.runs.len(), 0, "a day later, gone");
        n.read(Some(&format!("{file}.none")), now, &mut StdFiles).unwrap();
        assert_eq!(n.file_state, "missing");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
