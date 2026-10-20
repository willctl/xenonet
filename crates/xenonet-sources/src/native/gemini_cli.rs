//! core/native/gemini-cli.js: Gemini CLI's chat records. Each session in
//! `<gemini home>/.gemini/tmp/<project>/chats/session-*.jsonl` is a session on
//! the map, "`gemini-cli:<sessionId>`", with a subagent's run
//! (`chats/<parent>/<id>.jsonl`) under the session that started it, and its
//! write_file and replace calls' files as its edits.

use super::files::Files;
use super::node_path;
use super::{by_num, edit_file, edits_out, fits_id, js_max, js_min, ms_of, push_edit, str_of, titled, Check, Checked, Edits, Out, Threw, DAY_MS, EDIT_KEEP_MS, SESSIONS_MAX};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_base::json;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::PathStyle;
use xenonet_base::text::{is_blank, js_trim, js_trim_start, truthy};

pub const SOURCE: &str = "google/gemini-cli/chat-record";
/// The tools whose calls are edits, as the app registry has them.
pub fn edit_tools() -> &'static [String] {
    xenonet_base::apps::edit_tools("gemini-cli")
}

fn is_edit(tool: &str) -> bool {
    edit_tools().iter().any(|t| t == tool)
}
const FAILED: [&str; 2] = ["error", "cancelled"];
const FILE_MAX: f64 = (16u64 << 20) as f64;
const CHECKS_PER_FILE: usize = 12;

pub fn session_id_of(id: &str) -> Option<String> {
    fits_id(id, 128).then(|| format!("gemini-cli:{id}"))
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

/// An edit as a chat has it: the file as written, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatEdit {
    pub path: String,
    pub at: Option<f64>,
}

/// One chat record file as XenoNet reads it, its messages after rewinds.
#[derive(Clone, Debug, PartialEq)]
pub struct Chat {
    pub id: String,
    pub kind: Option<String>,
    pub summary: Option<String>,
    pub started_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub edits: Vec<ChatEdit>,
}

// { ...meta, ...more }: more's keys after meta's, a key already there kept
// in its place. An array's are its indexes.
fn merged(meta: Option<Map<String, Value>>, more: &Value) -> Map<String, Value> {
    let mut m = meta.unwrap_or_default();
    match more {
        Value::Object(o) => {
            for (k, v) in o {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                m.insert(i.to_string(), v.clone());
            }
        }
        _ => {}
    }
    m
}

/// A chat record file, or None when it isn't one. `check` sees the first
/// records until the metadata, and the first with tool calls.
pub fn read_chat(text: &str, check: &mut dyn FnMut(&Value) -> Result<Checked, Threw>) -> Result<Option<Chat>, Threw> {
    let t = js_trim_start(text);
    let records: Vec<Value> = if t.starts_with('{') && !t.contains("\n{") {
        // The older layout: one JSON object with its messages inside.
        let Ok(Value::Object(whole)) = json::parse(t) else { return Ok(None) };
        let messages = match whole.get("messages") {
            Some(Value::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        let meta: Map<String, Value> = whole.into_iter().filter(|(k, _)| k != "messages").collect();
        std::iter::once(Value::Object(meta)).chain(messages).collect()
    } else {
        text.split('\n').filter(|l| !is_blank(l)).filter_map(|l| json::parse(l).ok()).collect()
    };
    let mut meta: Option<Map<String, Value>> = None;
    let mut messages: OrderedMap<Value> = OrderedMap::new();
    let mut checked = 0;
    for mut r in records {
        if !r.is_object() {
            continue;
        }
        if checked < CHECKS_PER_FILE && (meta.is_none() || truthy(field(&r, "toolCalls"))) {
            if let Checked::Other(v) = check(&r)? {
                r = v;
            }
            checked += 1;
        }
        // What a check gave back that isn't an object matches nothing below.
        let Some(o) = r.as_object() else { continue };
        if str_of(o.get("sessionId")).is_some() && str_of(o.get("projectHash")).is_some() && !o.contains_key("id") {
            meta = Some(merged(meta.take(), &r));
        } else if let Some(to) = str_of(o.get("$rewindTo")) {
            let ids: Vec<String> = messages.keys().map(str::to_string).collect();
            let mut found = false;
            for id in ids {
                found = found || id == to;
                if found {
                    messages.remove(&id);
                }
            }
            if !found {
                messages.clear();
            }
        } else if let Some(set) = o.get("$set").filter(|s| s.is_object() || s.is_array()) {
            meta = Some(merged(meta.take(), set));
        } else if let Some(id) = str_of(o.get("id")).filter(|_| !o.contains_key("$patch")) {
            let id = id.to_string();
            messages.insert(&id, r);
        }
    }
    let Some(meta) = meta else { return Ok(None) };
    let Some(id) = str_of(meta.get("sessionId")) else { return Ok(None) };
    let mut edits = Vec::new();
    for (_, m) in messages.iter() {
        let Some(Value::Array(calls)) = field(m, "toolCalls") else { continue };
        for c in calls {
            if !truthy(Some(c)) || !str_of(field(c, "name")).is_some_and(is_edit) || str_of(field(c, "status")).is_some_and(|s| FAILED.contains(&s)) {
                continue;
            }
            if let Some(file) = str_of(field(c, "args").and_then(|a| field(a, "file_path"))).filter(|f| !f.is_empty()) {
                edits.push(ChatEdit { path: file.to_string(), at: ms_of(field(c, "timestamp")).or_else(|| ms_of(field(m, "timestamp"))) });
            }
        }
    }
    let times: Vec<f64> = messages.iter().filter_map(|(_, m)| ms_of(field(m, "timestamp"))).collect();
    let started_at = ms_of(meta.get("startTime")).or_else(|| times.iter().copied().reduce(js_min));
    let latest = times.iter().copied().fold(js_max(ms_of(meta.get("lastUpdated")).unwrap_or(0.0), 0.0), js_max);
    Ok(Some(Chat {
        id: id.to_string(),
        kind: str_of(meta.get("kind")).map(str::to_string),
        summary: str_of(meta.get("summary")).filter(|s| !s.is_empty()).map(str::to_string),
        started_at,
        updated_at: (latest != 0.0 && !latest.is_nan()).then_some(latest),
        edits,
    }))
}

/// The Gemini CLI's home: GEMINI_CLI_HOME (when it's set), else yours.
pub fn gemini_root(cli_home: Option<&str>, home: &str) -> String {
    node_path::join(PathStyle::Native, &[cli_home.filter(|h| !h.is_empty()).unwrap_or(home), ".gemini", "tmp"])
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub raw_id: String,
    pub cwd: Option<String>,
    pub summary: Option<String>,
    pub parent_id: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub edits: Edits,
}

struct Cached {
    mtime_ms: f64,
    size: f64,
    chat: Option<Chat>,
}

struct Found {
    file: String,
    parent: Option<String>,
    cwd: Option<String>,
    mtime_ms: f64,
    size: f64,
}

#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
fn chat_file(name: &str) -> bool {
    name.ends_with(".json") || name.ends_with(".jsonl")
}

/// The GeminiCliSessions block's reading.
pub struct GeminiCli {
    pub runs: OrderedMap<Run>,
    cache: HashMap<String, Cached>,
    pub root_state: &'static str,
}

impl Default for GeminiCli {
    fn default() -> Self {
        GeminiCli { runs: OrderedMap::new(), cache: HashMap::new(), root_state: "unread" }
    }
}

impl GeminiCli {
    pub fn new() -> Self {
        Self::default()
    }

    // The chat files changed in the last day, newest first.
    fn found(&mut self, root: Option<&str>, now: f64, files: &mut dyn Files) -> Vec<Found> {
        let Some(root) = root else {
            self.root_state = "unreadable";
            return Vec::new();
        };
        let projects = match files.read_dir(root) {
            Ok(p) => p,
            Err(err) => {
                self.root_state = if err.is("ENOENT") { "missing" } else { "unreadable" };
                return Vec::new();
            }
        };
        self.root_state = "ok";
        let join = |parts: &[&str]| node_path::join(PathStyle::Native, parts);
        let mut out = Vec::new();
        for p in projects.into_iter().filter(|p| p.is_dir) {
            let dir = join(&[root, &p.name]);
            let cwd = files.read_text(&join(&[&dir, ".project_root"])).ok().map(|t| js_trim(&t).to_string()).filter(|c| !c.is_empty());
            let chats = join(&[&dir, "chats"]);
            let mut take = |file: String, parent: Option<String>, files: &mut dyn Files| {
                if let Ok(st) = files.stat(&file) {
                    if now - st.mtime_ms <= DAY_MS && st.size <= FILE_MAX {
                        out.push(Found { file, parent, cwd: cwd.clone(), mtime_ms: st.mtime_ms, size: st.size });
                    }
                }
            };
            for e in files.read_dir(&chats).unwrap_or_default() {
                if e.is_file && e.name.starts_with("session-") && chat_file(&e.name) {
                    take(join(&[&chats, &e.name]), None, files);
                } else if e.is_dir {
                    for s in files.read_dir(&join(&[&chats, &e.name])).unwrap_or_default() {
                        if s.is_file && chat_file(&s.name) {
                            take(join(&[&chats, &e.name, &s.name]), Some(e.name.clone()), files);
                        }
                    }
                }
            }
        }
        by_num(&mut out, |f| -f.mtime_ms);
        out.truncate(SESSIONS_MAX);
        out
    }

    pub fn read(&mut self, root: Option<&str>, now: f64, files: &mut dyn Files, check: &mut Check<'_>) {
        let found = self.found(root, now, files);
        let mut runs: OrderedMap<Run> = OrderedMap::new();
        let mut seen = HashSet::new();
        for f in found {
            seen.insert(f.file.clone());
            let fresh = self.cache.get(&f.file).is_some_and(|h| h.mtime_ms == f.mtime_ms && h.size == f.size);
            if !fresh {
                let chat = files.read_text(&f.file).ok().and_then(|text| read_chat(&text, &mut |rec| check(rec, None)).ok().flatten());
                self.cache.insert(f.file.clone(), Cached { mtime_ms: f.mtime_ms, size: f.size, chat });
            }
            let Some(c) = self.cache.get(&f.file).and_then(|h| h.chat.as_ref()) else { continue };
            let Some(id) = session_id_of(&c.id) else { continue };
            let updated = js_max(c.updated_at.unwrap_or(0.0), 0.0);
            let mut run = Run {
                id: id.clone(),
                raw_id: c.id.clone(),
                cwd: f.cwd.clone(),
                summary: c.summary.clone(),
                parent_id: f.parent.as_deref().filter(|p| !p.is_empty()).and_then(session_id_of),
                created_at: c.started_at.unwrap_or(f.mtime_ms),
                updated_at: if updated == 0.0 || updated.is_nan() { f.mtime_ms } else { updated },
                edits: Edits::new(),
            };
            for e in &c.edits {
                let at = js_min(e.at.unwrap_or(run.updated_at), now);
                if now - at > EDIT_KEEP_MS {
                    continue;
                }
                if let Some(file) = edit_file(run.cwd.as_deref(), &e.path) {
                    push_edit(&mut run.edits, &file, at);
                }
            }
            // A run read twice (a copy, say) is its newest.
            if runs.get(&id).is_none_or(|had| had.updated_at < run.updated_at) {
                runs.insert(&id, run);
            }
        }
        self.cache.retain(|k, _| seen.contains(k));
        self.runs = runs;
    }

    pub fn sessions(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("app", Out::str("gemini-cli")),
                    ("rawId", Out::str(&r.raw_id)),
                    ("title", Out::str(&r.summary.clone().unwrap_or_else(|| titled("Gemini CLI", r.cwd.as_deref())))),
                    ("preview", Out::null()),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("archived", Out::bool(false)),
                    ("model", Out::null()),
                    ("branch", Out::null()),
                    ("parentId", Out::opt_str(r.parent_id.as_deref())),
                    ("parentKind", if r.parent_id.is_some() { Out::str("subagent") } else { Out::null() }),
                    ("reader", Out::str("experimental")),
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
                    ("rawId", Out::str(&r.raw_id)),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("summary", Out::opt_str(r.summary.as_deref())),
                    ("parentId", Out::opt_str(r.parent_id.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("edits", edits_out(&r.edits)),
                ])
            })
            .collect()
    }
}

/// A chat as readChat gives it.
pub fn chat_out(c: &Chat) -> Out {
    Out::Obj(vec![
        ("id", Out::str(&c.id)),
        ("kind", Out::opt_str(c.kind.as_deref())),
        ("summary", Out::opt_str(c.summary.as_deref())),
        ("startedAt", Out::opt_num(c.started_at)),
        ("updatedAt", Out::opt_num(c.updated_at)),
        ("edits", Out::Arr(c.edits.iter().map(|e| Out::Obj(vec![("path", Out::str(&e.path)), ("at", Out::opt_num(e.at))])).collect())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[expect(clippy::unnecessary_wraps, reason = "it stands in for a schema check, so it has a check's type")]
    fn same(_: &Value) -> Result<Checked, Threw> {
        Ok(Checked::Same)
    }

    #[test]
    fn rewinds_and_failed_calls() {
        let call = |name: &str, file: &str, status: &str| json!({ "id": "c", "name": name, "args": { "file_path": file }, "status": status, "timestamp": "2026-10-16T11:59:00Z" });
        let text = [
            json!({ "sessionId": "8f1d2c3b-1111", "projectHash": "ab12", "startTime": "2026-10-16T11:50:00Z", "lastUpdated": "2026-10-16T11:50:00Z" }),
            json!({ "id": "g1", "timestamp": "2026-10-16T11:58:00Z", "toolCalls": [call("write_file", "/w/a.txt", "success"), call("write_file", "/w/failed.txt", "error"), call("read_file", "/w/r.txt", "success")] }),
            json!({ "id": "g2", "timestamp": "2026-10-16T11:58:30Z", "toolCalls": [call("replace", "/w/rewound.txt", "success")] }),
            json!({ "$rewindTo": "g2" }),
            json!({ "$set": { "summary": "Partial refunds", "lastUpdated": "2026-10-16T11:59:30Z" } }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        let c = read_chat(&text, &mut same).unwrap().unwrap();
        assert_eq!(c.id, "8f1d2c3b-1111");
        assert_eq!(c.summary.as_deref(), Some("Partial refunds"));
        assert_eq!(c.edits.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["/w/a.txt"]);
        assert_eq!(c.updated_at, xenonet_base::time::parse_iso("2026-10-16T11:59:30Z"));
        let old = json!({ "sessionId": "8f1d2c3b-1111", "projectHash": "x", "messages": [{ "id": "g", "timestamp": "2026-10-16T11:58:00Z", "toolCalls": [call("write_file", "/a/b", "success")] }] }).to_string();
        assert_eq!(read_chat(&old, &mut same).unwrap().unwrap().edits[0].path, "/a/b");
        assert_eq!(read_chat("not json", &mut same).unwrap(), None);
        assert_eq!(read_chat("{\"id\":\"m\"}\n", &mut same).unwrap(), None);
    }
}
