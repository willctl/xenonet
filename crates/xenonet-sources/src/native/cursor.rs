//! core/native/cursor.js: Cursor's hook inputs, as XenoNet's Cursor hook
//! (scripts/cursor/hook.mjs) writes them. Each Cursor agent conversation is
//! a session on the map, "`cursor:<conversation id>`", with Cursor's
//! afterFileEdit files as its edits, ended at its sessionEnd.

use super::files::Files;
use super::node_path;
use super::{by_num, edits_out, fits_id, js_max, js_min, older, prune_edits, push_edit, str_of, titled, Check, Checked, Edits, Out, Threw, DAY_MS, SESSIONS_MAX};
use serde_json::Value;
use xenonet_base::fs::FsError;
use xenonet_base::json::{self, number_string};
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::{path_style, PathStyle};
use xenonet_base::text::{is_blank, js_string};
use xenonet_schema::drift::clean_version;

pub const SOURCE: &str = "anysphere/cursor/hook-input";
const READ_MAX: f64 = (4u64 << 20) as f64;
const CHECKS_MAX: usize = 32;

pub fn session_id_of(id: &str) -> Option<String> {
    fits_id(id, 128).then(|| format!("cursor:{id}"))
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

// Date.parse(v ?? ""): a string as it is, anything else as String() has it.
fn date_of(v: Option<&Value>) -> Option<f64> {
    match v {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => xenonet_base::time::parse_iso(s),
        Some(other) => xenonet_base::time::parse_iso(&js_string(other)),
    }
}

/// The file the hook writes: XENONET_CURSOR_EVENTS, else
/// cursor-hooks.ndjson in XenoNet's state folder.
pub fn cursor_events_file(named: Option<&str>, state_dir: &str) -> String {
    match named.filter(|n| !n.is_empty()) {
        Some(n) => node_path::resolve(PathStyle::Native, &[n]),
        None => node_path::join(PathStyle::Native, &[state_dir, "cursor-hooks.ndjson"]),
    }
}

/// The last READ_MAX of a file, whole lines only, with its size and mtime.
pub fn tail(file: &str, files: &mut dyn Files) -> Result<(Vec<String>, f64, f64), FsError> {
    let st = files.stat(file)?;
    let from = js_max(0.0, st.size - READ_MAX);
    let len = st.size - from;
    if !(len >= 0.0 && len.is_finite()) {
        return Err(FsError { code: Some("ERR_OUT_OF_RANGE".into()), message: format!("The value of \"size\" is out of range. Received {}", number_string(len)) });
    }
    let bytes = files.read_at(file, from, len as usize)?;
    let mut lines: Vec<String> = String::from_utf8_lossy(&bytes).split('\n').map(str::to_string).collect();
    if from > 0.0 {
        lines.remove(0);
    }
    Ok((lines, st.mtime_ms, st.size))
}

#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub raw_id: String,
    pub cwd: Option<String>,
    pub created_at: f64,
    pub updated_at: f64,
    pub ended_at: Option<f64>,
    pub edits: Edits,
}

/// The CursorSessions block's reading: its runs, the file's stamp when it
/// last parsed it, and whether it could read the file.
pub struct Cursor {
    pub runs: OrderedMap<Run>,
    pub file_state: &'static str,
    pub seen: Option<String>,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor { runs: OrderedMap::new(), file_state: "unread", seen: None }
    }
}

impl Cursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the file when it changed (by size and mtime), else drops what
    /// went stale. Threw when the vendor check did: the runs stay as they
    /// were, and the stamp is taken.
    pub fn read(&mut self, file: Option<&str>, now: f64, files: &mut dyn Files, check: &mut Check<'_>) -> Result<(), Threw> {
        let got = match file {
            Some(f) => tail(f, files),
            None => Err(FsError { code: Some("ERR_INVALID_ARG_TYPE".into()), message: "no file".into() }),
        };
        let (lines, mtime_ms, size) = match got {
            Ok(t) => t,
            Err(err) => {
                self.file_state = if err.is("ENOENT") { "missing" } else { "unreadable" };
                self.runs.clear();
                return Ok(());
            }
        };
        self.file_state = "ok";
        let stamp = format!("{}:{}", number_string(size), number_string(mtime_ms));
        if self.seen.as_deref() != Some(stamp.as_str()) {
            self.seen = Some(stamp);
            self.parse(&lines, now, check)?;
        } else {
            self.prune(now);
        }
        Ok(())
    }

    fn parse(&mut self, lines: &[String], now: f64, check: &mut Check<'_>) -> Result<(), Threw> {
        let mut runs: OrderedMap<Run> = OrderedMap::new();
        let mut checked = 0;
        for line in lines {
            if is_blank(line) {
                continue;
            }
            let Ok(mut e) = json::parse(line) else { continue };
            if !(e.is_object() || e.is_array()) {
                continue;
            }
            if checked < CHECKS_MAX {
                let version = str_of(field(&e, "cursor_version")).and_then(clean_version).map(str::to_string);
                if let Checked::Other(v) = check(&e, version.as_deref())? {
                    e = v;
                }
            }
            checked += 1;
            let Some(at) = date_of(field(&e, "at")) else { continue };
            if now - at > DAY_MS {
                continue;
            }
            let raw = field(&e, "conversation_id").filter(|v| !v.is_null()).or_else(|| field(&e, "session_id"));
            let Some(raw) = str_of(raw) else { continue };
            let Some(id) = session_id_of(raw) else { continue };
            if runs.get(&id).is_none() {
                runs.insert(&id, Run { id: id.clone(), raw_id: raw.to_string(), cwd: None, created_at: at, updated_at: at, ended_at: None, edits: Edits::new() });
            }
            let run = runs.get_mut(&id).expect("just added");
            run.created_at = js_min(run.created_at, at);
            run.updated_at = js_max(run.updated_at, at);
            let root = match field(&e, "workspace_roots") {
                Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).find(|x| node_path::is_absolute(path_style(x), x)),
                _ => None,
            };
            if let Some(root) = root {
                if run.cwd.as_deref().is_none_or(str::is_empty) {
                    run.cwd = Some(root.to_string());
                }
            }
            let hook = str_of(field(&e, "hook_event_name"));
            if hook == Some("sessionEnd") {
                run.ended_at = Some(at);
            } else if run.ended_at.is_some_and(|end| at > end) {
                run.ended_at = None;
            }
            if hook == Some("afterFileEdit") {
                if let Some(p) = str_of(field(&e, "file_path")).filter(|p| node_path::is_absolute(path_style(p), p)) {
                    let file = node_path::resolve(path_style(p), &[p]);
                    push_edit(&mut run.edits, &file, js_min(at, now));
                }
            }
        }
        if runs.len() > SESSIONS_MAX {
            let mut by_age: Vec<(f64, String)> = runs.iter().map(|(k, r)| (r.updated_at, k.to_string())).collect();
            by_num(&mut by_age, |x| x.0);
            for (_, k) in by_age.into_iter().take(runs.len() - SESSIONS_MAX) {
                runs.remove(&k);
            }
        }
        self.runs = runs;
        self.prune(now);
        Ok(())
    }

    fn prune(&mut self, now: f64) {
        self.runs.retain(|_, r| !older(now, r.updated_at, DAY_MS));
        for (_, r) in self.runs.iter_mut() {
            prune_edits(&mut r.edits, now);
        }
    }

    pub fn sessions(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("app", Out::str("cursor")),
                    ("rawId", Out::str(&r.raw_id)),
                    ("title", Out::str(&titled("Cursor", r.cwd.as_deref()))),
                    ("preview", Out::null()),
                    ("cwd", Out::opt_str(r.cwd.as_deref())),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("archived", Out::bool(false)),
                    ("model", Out::null()),
                    ("branch", Out::null()),
                    ("parentId", Out::null()),
                    ("parentKind", Out::null()),
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
    use crate::native::no_check;

    #[test]
    fn a_conversation_is_a_session_until_it_goes_stale() {
        let dir = std::env::temp_dir().join(format!("xenonet-cursor-rs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cursor-hooks.ndjson").to_string_lossy().into_owned();
        let at = "2026-10-16T12:00:00.000Z";
        let now = xenonet_base::time::parse_iso(at).unwrap();
        let line = |hook: &str, extra: &str| {
            format!("{{\"hook_event_name\":\"{hook}\",\"conversation_id\":\"6b0e5f1c-1111\",\"workspace_roots\":[\"/w/atlas\"],\"at\":\"{at}\"{extra}}}\n")
        };
        std::fs::write(&file, line("afterFileEdit", ",\"file_path\":\"/w/atlas/notes.txt\"")).unwrap();
        let mut c = Cursor::new();
        c.read(Some(&file), now, &mut StdFiles, &mut no_check).unwrap();
        let r = c.runs.get("cursor:6b0e5f1c-1111").unwrap();
        assert_eq!((r.cwd.as_deref(), r.ended_at), (Some("/w/atlas"), None));
        assert_eq!(r.edits.keys().collect::<Vec<_>>(), ["/w/atlas/notes.txt"]);
        std::fs::write(&file, line("afterFileEdit", "") + &line("sessionEnd", "")).unwrap();
        c.read(Some(&file), now, &mut StdFiles, &mut no_check).unwrap();
        assert_eq!(c.runs.get("cursor:6b0e5f1c-1111").unwrap().ended_at, Some(now));
        c.read(Some(&file), now + DAY_MS + 1.0, &mut StdFiles, &mut no_check).unwrap();
        assert_eq!(c.runs.len(), 0, "a day later, gone");
        c.read(Some(&format!("{file}.none")), now, &mut StdFiles, &mut no_check).unwrap();
        assert_eq!(c.file_state, "missing");
        std::fs::remove_dir_all(&dir).unwrap();
        // Spelled as on a unix host; Windows writes the same file with backslashes.
        if cfg!(unix) {
            assert_eq!(cursor_events_file(None, "/s"), "/s/cursor-hooks.ndjson");
        }
    }
}
