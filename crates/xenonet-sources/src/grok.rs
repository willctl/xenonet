//! core/grok.js: Grok Build's sessions, from its folders. Grok Build keeps
//! each session in a folder of its own, ~/.grok/sessions/<encoded
//! cwd>/<session id>/ (GROK_HOME moves ~/.grok): summary.json is its index
//! entry, and updates.jsonl its conversation, which the line readers read.
//! A subagent's session is one like any other, whose summary names its
//! parent. Reading summary.json leniently (vendor.js's readers) is the
//! caller's: `sample` hands back the record to read, or None to pass over
//! it.

use crate::line_events::{self, Kind, LineFile};
use crate::lines::GrokLine;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use xenonet_base::text::{js_string, js_trim};
use xenonet_base::{jsstr, time, units};

/// A time as summary.json writes it (RFC 3339), or in Unix seconds or
/// milliseconds; None when it isn't one.
pub fn grok_time(t: Option<&Value>) -> Option<f64> {
    let ms = match t {
        Some(Value::String(s)) => time::parse_iso(s).unwrap_or(f64::NAN),
        Some(Value::Number(n)) => n.as_f64().filter(|t| *t > 0.0).map_or(f64::NAN, |t| if t < 1e12 { t * 1000.0 } else { t }),
        _ => f64::NAN,
    };
    ms.is_finite().then_some(ms)
}

// Text's first line, trimmed, at most n characters (UTF-16 units, as JS
// counts them).
fn first_line(s: Option<&Value>, n: usize) -> String {
    let text = s.and_then(Value::as_str).unwrap_or_default();
    let line = js_trim(text).split('\n').next().unwrap_or_default();
    let t = js_trim(line.strip_suffix('\r').unwrap_or(line)).to_string();
    if jsstr::len(&t) > n {
        format!("{}…", jsstr::slice(&t, 0, Some(n as i64 - 1)))
    } else {
        t
    }
}

// The folders in a folder, sorted as JS sorts names, without the hidden
// ones.
fn dirs_in(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<String> =
        read.filter_map(Result::ok).filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).filter_map(|e| e.file_name().into_string().ok()).filter(|n| !n.starts_with('.')).collect();
    out.sort_by(|a, b| jsstr::cmp_units(a, b));
    out
}

fn absolute(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with("\\\\") || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
}

// decodeURIComponent: None where JS throws (a % without two hex digits
// after it, or bytes that aren't UTF-8).
fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The folder a group of sessions works in, from the group's name: Grok
/// URL-encodes the working directory, and when that's over 255 bytes uses
/// a slug and a hash instead and writes the folder to a .cwd file in the
/// group. None when neither says (each session's summary may).
pub fn group_cwd(group_dir: &Path, name: &str) -> Option<String> {
    if let Ok(written) = std::fs::read_to_string(group_dir.join(".cwd")) {
        let w = js_trim(&written);
        if !w.is_empty() {
            return Some(w.to_string());
        }
    }
    if !name.contains('%') {
        return None;
    }
    decode_uri_component(name).filter(|d| absolute(d))
}

fn text(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn nonempty(v: Option<&Value>) -> Option<&str> {
    text(v).filter(|s| !s.is_empty())
}

fn or_null(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::String)
}

fn number(v: Option<f64>) -> Value {
    v.map_or(Value::Null, xenonet_base::json::number)
}

// A session as the map lists it (session/v1), from its folder's summary
// (`s`, {} when it has none yet), the group's folder, and its updates
// file's time and size.
fn session_of(id: &str, s: &Value, cwd: Option<&str>, updates: Option<(Option<f64>, f64)>) -> Value {
    let info = s.get("info").filter(|i| i.is_object());
    let raw_id = nonempty(info.and_then(|i| i.get("id"))).unwrap_or(id).to_string();
    let here = nonempty(info.and_then(|i| i.get("cwd"))).or(cwd).map(str::to_string);
    // A worktree session works in one of Grok's worktrees, and belongs to
    // the folder its work came from.
    let source = nonempty(s.get("source_workspace_dir")).map(str::to_string);
    let kind = text(s.get("session_kind"));
    let parent = nonempty(s.get("parent_session_id"));
    let subagent = kind.is_some_and(|k| k.starts_with("subagent"));
    let title =
        [first_line(s.get("generated_title"), 80), first_line(s.get("session_summary"), 80)].into_iter().find(|t| !t.is_empty()).unwrap_or_else(|| "(untitled session)".into());
    let preview = first_line(s.get("session_summary"), 200);
    let updated = grok_time(s.get("last_active_at")).or_else(|| grok_time(s.get("updated_at"))).or_else(|| grok_time(s.get("created_at"))).or(updates.and_then(|u| u.0));
    let mut m = Map::new();
    m.insert("id".into(), Value::String(format!("grok:{raw_id}")));
    m.insert("app".into(), "grok".into());
    m.insert("rawId".into(), Value::String(raw_id));
    m.insert("title".into(), Value::String(title));
    m.insert("preview".into(), or_null(Some(preview).filter(|p| !p.is_empty())));
    m.insert("cwd".into(), or_null(source.clone().or_else(|| here.clone())));
    m.insert("worktree".into(), if source.is_some() { or_null(here) } else { Value::Null });
    m.insert("createdAt".into(), number(grok_time(s.get("created_at"))));
    m.insert("updatedAt".into(), number(updated));
    m.insert("archived".into(), Value::Bool(false));
    m.insert("model".into(), or_null(text(s.get("current_model_id")).map(str::to_string)));
    m.insert("branch".into(), or_null(text(s.get("head_branch")).map(str::to_string)));
    m.insert("status".into(), Value::Null);
    m.insert("source".into(), or_null(kind.map(str::to_string)));
    m.insert("parentId".into(), or_null(parent.map(|p| format!("grok:{p}"))));
    m.insert("parentKind".into(), or_null(parent.map(|_| if subagent { "subagent" } else { "fork" }.to_string())));
    // Background work: a run Grok marks hidden, or one run headless (grok
    // -p), as `codex exec` runs are for Codex. A subagent is its parent's.
    m.insert("background".into(), Value::Bool(s.get("hidden") == Some(&Value::Bool(true)) || kind == Some("headless")));
    m.insert("sizeBytes".into(), number(updates.map(|u| u.1)));
    Value::Object(m)
}

/// Grok Build's sessions under `home` (GROK_HOME), as XenoNet lists
/// sessions (session/v1).
pub fn list_sessions(home: &str, sample: &mut dyn FnMut(Value) -> Option<Value>) -> Vec<Value> {
    let root = Path::new(home).join("sessions");
    let mut out = Vec::new();
    for group in dirs_in(&root) {
        let group_dir = root.join(&group);
        let cwd = group_cwd(&group_dir, &group);
        for id in dirs_in(&group_dir) {
            let dir = group_dir.join(&id);
            let updates = std::fs::metadata(dir.join("updates.jsonl")).ok().map(|m| (xenonet_base::fs::mtime_ms_of(&m).ok(), m.len() as f64));
            let raw = std::fs::read_to_string(dir.join("summary.json")).ok().and_then(|t| xenonet_base::json::parse(&t).ok());
            // A folder without a summary yet is a session being made, once
            // its conversation is there.
            let s = match raw {
                None if updates.is_none() => continue,
                None => Value::Object(Map::new()),
                Some(raw) => match sample(raw) {
                    Some(s @ Value::Object(_)) => s,
                    _ => continue,
                },
            };
            out.push(session_of(&id, &s, cwd.as_deref(), updates));
        }
    }
    out
}

/// A session's folder under `home`, by its id.
pub fn folder(home: &str, raw_id: &str) -> Option<PathBuf> {
    let root = Path::new(home).join("sessions");
    dirs_in(&root).into_iter().map(|g| root.join(g).join(raw_id)).find(|d| d.is_dir())
}

/// A session's lines as the console shows them, oldest first, from their
/// events (line_events.rs): what you asked, what the agent said, and each
/// tool call with how it went.
pub fn entries(lines: &[GrokLine]) -> Vec<Value> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    let file = LineFile::default();
    let at = |o: &GrokLine| o.ts().ok().filter(|t| t.is_finite()).map_or(Value::Null, xenonet_base::json::number);
    for (n, o) in lines.iter().enumerate() {
        let ts = at(o);
        let entry = |role: &str, text: &str| -> Map<String, Value> {
            let mut m = Map::new();
            m.insert("id".into(), Value::String(format!("update-{n}")));
            m.insert("ts".into(), ts.clone());
            m.insert("role".into(), role.into());
            m.insert("text".into(), Value::String(units::excerpt(text, 2500)));
            m
        };
        for e in line_events::grok(o, &file) {
            match &e.kind {
                Kind::Prompt(p) => {
                    let asked = p.claude.as_ref().is_none_or(|c| c.meta != Some(&Value::Bool(true)));
                    if asked && !js_trim(&p.text).is_empty() {
                        out.push(entry("user", &p.text));
                    }
                }
                Kind::Said { parts: Some(Value::Array(blocks)), .. } => {
                    let said: String =
                        blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("text")).filter_map(|b| b.get("text").and_then(Value::as_str)).collect();
                    if !js_trim(&said).is_empty() {
                        out.push(entry("assistant", &said));
                    }
                }
                Kind::Tool { id, name, .. } => {
                    let title = nonempty(o.update().get("title")).map_or_else(|| name.map(js_string).unwrap_or_default(), str::to_string);
                    let mut m = Map::new();
                    m.insert("id".into(), Value::String(id.filter(|v| !v.is_null()).map_or_else(|| format!("call-{n}"), js_string)));
                    m.insert("ts".into(), ts.clone());
                    m.insert("role".into(), "tool".into());
                    m.insert("kind".into(), "tool_use".into());
                    m.insert("title".into(), Value::String(title));
                    m.insert("text".into(), Value::String(String::new()));
                    m.insert("status".into(), Value::Null);
                    out.push(m);
                }
                Kind::Result { id, error, .. } => {
                    let id = id.map_or_else(|| "undefined".into(), js_string);
                    if let Some(m) = out.iter_mut().rev().find(|m| m.get("kind").is_some() && m.get("id").and_then(Value::as_str) == Some(&id)) {
                        m.insert("status".into(), if *error == Some(true) { "failed" } else { "completed" }.into());
                    }
                }
                _ => {}
            }
        }
    }
    out.into_iter().map(Value::Object).collect()
}

/// A page of a session's entries, newest first: `cursor` is the count of
/// entries already shown from the end. { entries, cursor, total }.
pub fn page(all: Vec<Value>, cursor: Option<&str>, limit: usize) -> Value {
    let total = all.len();
    let shown = cursor.and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(0);
    let end = total.saturating_sub(shown);
    let start = end.saturating_sub(limit);
    let entries: Vec<Value> = all[start..end].to_vec();
    serde_json::json!({ "entries": entries, "cursor": if start > 0 { Value::String((total - start).to_string()) } else { Value::Null }, "total": total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn times_lines_and_folders() {
        assert_eq!(grok_time(Some(&json!("2026-10-09T15:00:00Z"))), Some(1791558000000.0));
        assert_eq!(grok_time(Some(&json!(1791558000))), Some(1791558000000.0));
        assert_eq!(grok_time(Some(&json!("soon"))), None);
        assert_eq!(first_line(Some(&json!("  Reconcile the ledger\r\nand more ")), 80), "Reconcile the ledger");
        assert_eq!(first_line(Some(&json!("abcdef")), 4), "abc…");
        assert_eq!(decode_uri_component("%2FUsers%2Fada"), Some("/Users/ada".into()));
        assert_eq!(decode_uri_component("%E0%A4%A"), None);
        assert!(absolute("/a") && absolute("C:\\a") && absolute("\\\\host\\x") && !absolute("a/b"));
    }

    #[test]
    #[cfg(unix)]
    fn lists_the_sessions_in_their_folders() {
        let home = std::env::temp_dir().join(format!("xenonet-grok-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let atlas = home.join("sessions").join("%2FUsers%2Fada%2Fatlas");
        let summary = |id: &str, more: Value| {
            let mut s = json!({ "info": { "id": id, "cwd": "/Users/ada/atlas" }, "session_summary": "", "created_at": "2026-10-09T14:00:00Z", "updated_at": "2026-10-09T15:00:00Z", "current_model_id": "grok-build" });
            if let (Value::Object(m), Value::Object(more)) = (&mut s, more) {
                m.extend(more);
            }
            s.to_string()
        };
        write(&atlas.join("a").join("summary.json"), &summary("a", json!({ "generated_title": "Add partial refunds", "head_branch": "feat/refunds" })));
        write(&atlas.join("a").join("updates.jsonl"), "{}\n");
        write(&atlas.join("b").join("summary.json"), &summary("b", json!({ "parent_session_id": "a", "session_kind": "subagent" })));
        write(&atlas.join("c").join("summary.json"), &summary("c", json!({ "parent_session_id": "a", "session_kind": "fork", "session_summary": "Try a flag\nmore" })));
        write(&atlas.join("d").join("summary.json"), &summary("d", json!({ "session_kind": "headless" })));
        write(
            &atlas.join("e").join("summary.json"),
            &summary("e", json!({ "session_kind": "worktree", "source_workspace_dir": "/Users/ada/atlas", "info": { "id": "e", "cwd": "/Users/ada/.grok/worktrees/atlas/wt" } })),
        );
        write(&atlas.join("f").join("updates.jsonl"), "{}\n");
        std::fs::create_dir_all(atlas.join("g")).unwrap();
        let long = home.join("sessions").join("ledger-3f9a");
        write(&long.join(".cwd"), "/Users/ada/ledger\n");
        write(&long.join("h").join("summary.json"), &json!({ "info": { "id": "h" }, "session_summary": "Reconcile" }).to_string());
        let mut sampled = 0;
        let list = list_sessions(&home.to_string_lossy(), &mut |raw| {
            sampled += 1;
            Some(raw)
        });
        let ids: Vec<&str> = list.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["grok:a", "grok:b", "grok:c", "grok:d", "grok:e", "grok:f", "grok:h"]);
        assert_eq!(sampled, 6, "every summary is read leniently");
        let a = &list[0];
        assert_eq!(
            (a["title"].as_str(), a["cwd"].as_str(), a["branch"].as_str(), a["model"].as_str(), a["updatedAt"].as_f64()),
            (Some("Add partial refunds"), Some("/Users/ada/atlas"), Some("feat/refunds"), Some("grok-build"), Some(1791558000000.0))
        );
        assert_eq!((list[1]["parentId"].as_str(), list[1]["parentKind"].as_str()), (Some("grok:a"), Some("subagent")));
        assert_eq!((list[2]["parentKind"].as_str(), list[2]["title"].as_str(), list[2]["preview"].as_str()), (Some("fork"), Some("Try a flag"), Some("Try a flag")));
        assert_eq!((list[3]["background"].as_bool(), list[1]["background"].as_bool()), (Some(true), Some(false)));
        assert_eq!((list[4]["cwd"].as_str(), list[4]["worktree"].as_str()), (Some("/Users/ada/atlas"), Some("/Users/ada/.grok/worktrees/atlas/wt")));
        assert_eq!((list[5]["title"].as_str(), list[5]["cwd"].as_str()), (Some("(untitled session)"), Some("/Users/ada/atlas")));
        assert_eq!(list[6]["cwd"].as_str(), Some("/Users/ada/ledger"));
        assert_eq!(folder(&home.to_string_lossy(), "c"), Some(atlas.join("c")));
        // A summary the lenient read drops is passed over.
        assert_eq!(list_sessions(&home.to_string_lossy(), &mut |_| None).len(), 1);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_sessions_entries_and_pages() {
        let up = |t: f64, update: Value| GrokLine::new(json!({ "timestamp": t, "method": "session/update", "params": { "update": update } }));
        let lines = [
            up(1.0, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "Run the tests" } })),
            up(2.0, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Execute `npm test`", "kind": "execute", "status": "in_progress" })),
            up(3.0, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed" })),
            up(4.0, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "One failed." } })),
        ];
        let all = entries(&lines);
        assert_eq!(all.iter().map(|e| e["role"].as_str().unwrap()).collect::<Vec<_>>(), ["user", "tool", "assistant"]);
        assert_eq!((all[1]["title"].as_str(), all[1]["status"].as_str(), all[1]["ts"].as_f64()), (Some("Execute `npm test`"), Some("failed"), Some(2000.0)));
        let p = page(all.clone(), None, 2);
        assert_eq!((p["entries"].as_array().map(Vec::len), p["cursor"].as_str(), p["total"].as_u64()), (Some(2), Some("2"), Some(3)));
        let p = page(all, Some("2"), 2);
        assert_eq!((p["entries"][0]["role"].as_str(), p["cursor"].is_null()), (Some("user"), true));
    }
}
