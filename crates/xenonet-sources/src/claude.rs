//! core/claude.js: Claude Code's sessions and transcripts, with Claude
//! desktop's records of them.
//!
//! Claude desktop's session files (read from disk here), merged into the
//! Agent SDK's listing; a transcript's messages as the console shows them,
//! a page at a time; and the checks and the edit when a session is renamed
//! in the desktop's record. The Agent SDK's calls, and the lenient checks
//! of what it and the desktop's files say (vendor.js's readers), stay with
//! the caller.

use crate::codex::{excerpt, excerpt_json, js_max0, nn, slice_index, tpl, Obj, F};
use crate::lines::{opt_prop, prop, Js, TypeError};
use serde_json::{Map, Value};
use std::path::Path;
use xenonet_base::text::{js_number, js_string, truthy};
use xenonet_base::{json, jsstr, paths, time};

/// One of Claude desktop's session files: where it is, the folders it sits
/// in below claude-code-sessions/, and what it says (None when it isn't
/// JSON, or can't be read).
#[derive(Clone, Debug, PartialEq)]
pub struct DesktopFile {
    pub file: String,
    pub parts: Vec<String>,
    pub data: Option<Value>,
}

// /^local_.+\.json$/
fn is_session_file(name: &str) -> bool {
    match name.strip_prefix("local_").and_then(|r| r.strip_suffix(".json")) {
        Some(mid) => !mid.is_empty() && !mid.contains(['\n', '\r', '\u{2028}', '\u{2029}']),
        None => false,
    }
}

/// Claude desktop's session files under `root` (its claude-code-sessions
/// folder), in the order a walk finds them: sessions sit in
/// <account>/<organization>/, so the folders say which Claude account
/// each ran as. A folder that can't be read ends the walk, as the JS's did.
pub fn desktop_files(root: &str) -> Vec<DesktopFile> {
    fn walk(dir: &str, parts: &[String], out: &mut Vec<DesktopFile>) -> std::io::Result<()> {
        for ent in paths::read_dir_sorted(Path::new(dir))? {
            let name = ent.file_name().to_string_lossy().into_owned();
            let full = paths::join(dir, &name);
            let Ok(t) = ent.file_type() else { continue };
            if t.is_dir() && parts.len() < 3 && !name.starts_with("deleted_") {
                let mut deeper = parts.to_vec();
                deeper.push(name);
                walk(&full, &deeper, out)?;
            } else if t.is_file() && is_session_file(&name) {
                let data = std::fs::read(&full).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)).ok());
                out.push(DesktopFile { file: full, parts: parts.to_vec(), data });
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if Path::new(root).exists() {
        let _ = walk(root, &[], &mut out);
    }
    out
}

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

// A value as a Map's key: by type and value, and an object as no other.
fn key_of(v: Js<'_>, n: usize) -> String {
    match v {
        None => "u".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => format!("b:{b}"),
        Some(Value::Number(x)) => format!("n:{}", json::number_string(x.as_f64().unwrap_or(f64::NAN))),
        Some(Value::String(s)) => format!("s:{s}"),
        Some(_) => format!("o:{n}"),
    }
}

/// The desktop's records as the listing keeps them, from the files' records
/// (after the lenient reader): (each record, with the account its folders
/// name; last of each CLI session, in the order first found) and (desktop
/// id → file, for renaming).
pub fn desktop_index(records: Vec<(String, Vec<String>, Value)>) -> (Vec<Value>, Vec<(String, String)>) {
    let mut by_cli: Vec<(String, Value)> = Vec::new();
    let mut files = Vec::new();
    for (n, (file, parts, mut d)) in records.into_iter().enumerate() {
        if !truthy(Some(&d)) {
            continue;
        }
        if parts.len() == 2 && parts.iter().all(|p| is_uuid(p)) {
            // d.account = …, which throws (and skips the record) on a
            // value that isn't an object.
            match &mut d {
                Value::Object(m) => {
                    m.insert("account".into(), serde_json::json!({ "account": parts[0], "org": parts[1] }));
                }
                Value::Array(_) => {}
                _ => continue,
            }
        }
        let cli = opt_prop(Some(&d), "cliSessionId");
        if truthy(cli) {
            let k = key_of(cli, n);
            match by_cli.iter_mut().find(|(x, _)| *x == k) {
                Some((_, at)) => *at = d.clone(),
                None => by_cli.push((k, d.clone())),
            }
        }
        if let Some(Value::String(id)) = opt_prop(Some(&d), "sessionId") {
            files.push((id.clone(), file));
        }
    }
    (by_cli.into_iter().map(|(_, d)| d).collect(), files)
}

/// The Agent SDK's sessions (after the lenient reader), with what Claude
/// desktop's records say of them: its title first, the repo a worktree
/// session branched from, starred and archived, its model, the session
/// that spawned it, and the account its folders name.
pub fn sessions(listed: &[Value], desktop: &[Value]) -> Result<Vec<Obj>, TypeError> {
    let by_cli: Vec<(String, &Value)> = desktop.iter().enumerate().map(|(i, d)| (key_of(opt_prop(Some(d), "cliSessionId"), i), d)).collect();
    // desktop id → its CLI session id; the last of each desktop id wins.
    let mut desktop_to_cli: Vec<(String, Js<'_>)> = Vec::new();
    for (i, d) in desktop.iter().enumerate() {
        let k = key_of(opt_prop(Some(d), "sessionId"), i + 1_000_000);
        let v = opt_prop(Some(d), "cliSessionId");
        match desktop_to_cli.iter_mut().find(|(x, _)| *x == k) {
            Some((_, at)) => *at = v,
            None => desktop_to_cli.push((k, v)),
        }
    }
    let mut out = Vec::with_capacity(listed.len());
    for (n, s) in listed.iter().enumerate() {
        let s = Some(s);
        let get = |k: &str| prop(s, k);
        let sid = get("sessionId")?;
        let want = key_of(sid, usize::MAX - n);
        let d = by_cli.iter().find(|(k, _)| *k == want).map(|(_, d)| *d);
        let dp = |k: &str| opt_prop(d, k);
        let spawned = opt_prop(dp("spawnedFrom"), "sessionId");
        let parent_cli = if truthy(spawned) {
            let k = key_of(spawned, usize::MAX);
            desktop_to_cli.iter().find(|(x, _)| *x == k).and_then(|(_, v)| *v)
        } else {
            None
        };
        let parent_cli = parent_cli.filter(|p| truthy(Some(p)));
        let first = get("firstPrompt")?;
        let title = [dp("title"), get("customTitle")?, get("summary")?].into_iter().flatten().find(|v| truthy(Some(v))).cloned();
        let title = match title {
            Some(t) => t,
            None => {
                let cut = match nn(first) {
                    None => None,
                    Some(Value::String(p)) => Some(Value::String(jsstr::head(p, 80))),
                    Some(Value::Array(a)) => Some(Value::Array(a.iter().take(80).cloned().collect())),
                    Some(_) => return Err(TypeError("s.firstPrompt?.slice is not a function".into())),
                };
                cut.filter(|c| truthy(Some(c))).unwrap_or_else(|| Value::String("(untitled session)".into()))
            }
        };
        let cwd = [dp("originCwd"), get("cwd")?].into_iter().flatten().find(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null);
        let updated = {
            let a = js_number(Some(nn(get("lastModified")?).unwrap_or(&Value::from(0))));
            let b = js_number(Some(nn(dp("lastActivityAt")).unwrap_or(&Value::from(0))));
            let m = if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) };
            if m == 0.0 || m.is_nan() {
                F::null()
            } else {
                F::V(json::number(m))
            }
        };
        let created = nn(get("createdAt")?).or(nn(dp("createdAt")));
        out.push(vec![
            ("id", F::s(format!("claude:{}", tpl(sid)))),
            ("app", F::s("claude")),
            ("rawId", F::of(sid)),
            ("desktopId", F::or_null(dp("sessionId"))),
            ("inDesktop", F::V(Value::Bool(d.is_some()))),
            ("title", F::V(title)),
            ("preview", F::or_null(first)),
            ("cwd", F::V(cwd)),
            ("worktree", F::or_null(dp("worktreePath"))),
            ("createdAt", F::or_null(created)),
            ("updatedAt", updated),
            ("archived", F::V(Value::Bool(truthy(dp("isArchived"))))),
            ("starred", F::V(Value::Bool(truthy(dp("isStarred"))))),
            ("model", F::or_null(dp("model"))),
            ("effort", F::or_null(dp("effort"))),
            ("permissionMode", F::or_null(dp("permissionMode"))),
            ("branch", F::or_null(nn(dp("branch")).or(nn(get("gitBranch")?)))),
            ("tag", F::or_null(get("tag")?)),
            ("sizeBytes", F::or_null(get("fileSize")?)),
            ("parentId", parent_cli.map_or(F::null(), |p| F::s(format!("claude:{}", js_string(p))))),
            ("parentKind", if parent_cli.is_some() { F::s("spawn") } else { F::null() }),
            ("claudeAccount", F::or_null(dp("account"))),
        ]);
    }
    Ok(out)
}

// ---------- a transcript ----------

/// The page of `len` messages to show, newest first: (start, end, cursor
/// further back). `cursor` is the count of messages already shown from
/// the end.
pub fn page(len: usize, cursor: Js<'_>, limit: f64) -> (usize, usize, Option<String>) {
    let shown = js_number(Some(nn(cursor).unwrap_or(&Value::from(0))));
    let end = len as f64 - shown;
    let start = js_max0(end - limit);
    let (a, b) = (slice_index(start, len), slice_index(end, len));
    let next = (start > 0.0).then(|| json::number_string(len as f64 - start));
    (a, b.max(a), next)
}

// What a tool result's content says: its text, an image as "[image]".
fn block_text(content: Js<'_>) -> Result<String, TypeError> {
    match content {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for c in parts {
                let ty = prop(Some(c), "type")?;
                out.push(match ty.and_then(Value::as_str) {
                    Some("text") => nn(prop(Some(c), "text")?).map_or_else(String::new, js_string),
                    Some("image") => "[image]".into(),
                    _ => String::new(),
                });
            }
            Ok(out.join("\n"))
        }
        _ => Ok(String::new()),
    }
}

// A message's entries, as the console shows them.
fn message_entries(m: &Value, out: &mut Vec<Obj>) -> Result<(), TypeError> {
    let m = Some(m);
    let stamp = prop(m, "timestamp")?;
    let ts = if truthy(stamp) {
        match time::parse_iso(&js_string(stamp.unwrap())) {
            Some(t) => F::V(json::number(t)),
            None => F::Num(f64::NAN),
        }
    } else {
        F::null()
    };
    let kind = prop(m, "type")?.and_then(Value::as_str).map(str::to_string);
    let uuid = prop(m, "uuid")?;
    let content = opt_prop(prop(m, "message")?, "content");
    if let Some(Value::String(text)) = content {
        let role = match kind.as_deref() {
            Some("assistant") => "assistant",
            Some("user") => "user",
            _ => "system",
        };
        out.push(vec![("id", F::of(uuid)), ("ts", ts), ("role", F::s(role)), ("text", F::s(text.clone()))]);
        return Ok(());
    }
    let blocks: &[Value] = match nn(content) {
        None => &[],
        Some(Value::Array(a)) => a,
        Some(_) => return Err(TypeError("(content ?? []).entries is not a function".into())),
    };
    for (i, b) in blocks.iter().enumerate() {
        let b = Some(b);
        let id = F::s(format!("{}:{i}", tpl(uuid)));
        match prop(b, "type")?.and_then(Value::as_str) {
            Some("text") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s(if kind.as_deref() == Some("assistant") { "assistant" } else { "user" })),
                ("text", F::of(prop(b, "text")?)),
            ]),
            Some("thinking") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s("reasoning")),
                ("text", F::V(nn(prop(b, "thinking")?).cloned().unwrap_or(Value::String(String::new())))),
            ]),
            Some("tool_use") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s("tool")),
                ("kind", F::s("tool_use")),
                ("title", F::of(prop(b, "name")?)),
                ("text", excerpt_json(prop(b, "input")?, 2500)),
            ]),
            Some("tool_result") => {
                let status = if truthy(prop(b, "is_error")?) { "failed" } else { "completed" };
                let text = block_text(prop(b, "content")?)?;
                out.push(vec![
                    ("id", id),
                    ("ts", ts.clone()),
                    ("role", F::s("tool")),
                    ("kind", F::s("tool_result")),
                    ("title", F::s("result")),
                    ("status", F::s(status)),
                    ("text", excerpt(Some(&Value::String(text)), 4000)),
                ]);
            }
            _ => {}
        }
    }
    Ok(())
}

fn shows(e: &Obj) -> bool {
    let field = |k: &str| e.iter().find(|(n, _)| *n == k).map(|(_, f)| f);
    let truthy_f = |f: Option<&F>| match f {
        Some(F::V(v)) => truthy(Some(v)),
        Some(F::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(F::Arr(_)) => true,
        _ => false,
    };
    truthy_f(field("text")) || truthy_f(field("title"))
}

/// A page of a transcript's messages (after the lenient reader) as the
/// console shows them: each message's text, thinking, tool calls and
/// results, the long ones cut, and only those with something to show.
pub fn entries(messages: &[Value]) -> Result<Vec<Obj>, TypeError> {
    let mut out = Vec::new();
    for m in messages {
        message_entries(m, &mut out)?;
    }
    out.retain(shows);
    Ok(out)
}

// ---------- renaming ----------

pub const NO_RECORD: &str = "XenoNet hasn't found Claude desktop's record of this session";
pub const OTHER_SESSION: &str = "Claude desktop's record is of another session now";

// a === b, for values from JSON: objects are never the same one.
fn strict_eq(a: Js<'_>, b: Js<'_>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// The desktop's record with only its title changed and marked as yours,
/// as the text to write, so the desktop's own naming doesn't replace it.
/// None when the record is of another session now.
pub fn renamed_record(record: Value, desktop_id: Js<'_>, cli_id: Js<'_>, title: Js<'_>) -> Option<String> {
    let Value::Object(mut m) = record else { return None };
    if !strict_eq(m.get("sessionId"), desktop_id) || !strict_eq(m.get("cliSessionId"), cli_id) {
        return None;
    }
    match title {
        Some(t) => {
            m.insert("title".into(), t.clone());
        }
        // record.title = undefined, which JSON.stringify leaves out.
        None => {
            m.shift_remove("title");
        }
    }
    m.insert("titleSource".into(), Value::String("user".into()));
    Some(json::stringify(&js_order(Value::Object(m)), 0))
}

// An object's keys as a JS object lists them, all the way down.
fn js_order(v: Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(jsstr::js_key_order(m.into_iter().map(|(k, v)| (k, js_order(v))).collect::<Map<_, _>>())),
        Value::Array(a) => Value::Array(a.into_iter().map(js_order).collect()),
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get<'a>(o: &'a Obj, k: &str) -> Option<&'a F> {
        o.iter().find(|(n, _)| *n == k).map(|(_, f)| f)
    }

    #[test]
    fn merges_the_desktops_records() {
        let a = "0190d3e0-0000-7000-a000-000000000001".to_string();
        let o = "0190d3e0-0000-7000-b000-000000000001".to_string();
        let recs = vec![
            ("/d/local_1.json".to_string(), vec![a.clone(), o.clone()], json!({ "sessionId": "local_1", "cliSessionId": "c1", "title": "Refunds", "lastActivityAt": 50 })),
            (
                "/d/local_2.json".to_string(),
                vec![a.clone(), o.clone()],
                json!({ "sessionId": "local_2", "cliSessionId": "c2", "title": "", "spawnedFrom": { "sessionId": "local_1" } }),
            ),
            ("/d/x/local_3.json".to_string(), vec!["x".into()], json!({ "sessionId": "local_3", "cliSessionId": "c3" })),
        ];
        let (desktop, files) = desktop_index(recs);
        assert_eq!(files.len(), 3);
        assert_eq!(desktop[0]["account"], json!({ "account": a, "org": o }));
        let listed =
            [json!({ "sessionId": "c1", "lastModified": 10, "firstPrompt": "Add refunds" }), json!({ "sessionId": "c2", "firstPrompt": "Review" }), json!({ "sessionId": "c9" })];
        let s = sessions(&listed, &desktop).unwrap();
        assert_eq!(get(&s[0], "title"), Some(&F::s("Refunds")));
        assert_eq!(get(&s[0], "updatedAt"), Some(&F::V(json!(50))));
        assert_eq!(get(&s[1], "title"), Some(&F::s("Review")));
        assert_eq!(get(&s[1], "parentId"), Some(&F::s("claude:c1")));
        assert_eq!(get(&s[2], "inDesktop"), Some(&F::V(json!(false))));
        assert_eq!(get(&s[2], "title"), Some(&F::s("(untitled session)")));
    }

    #[test]
    fn pages_and_entries() {
        assert_eq!(page(4, None, 120.0), (0, 4, None));
        assert_eq!(page(4, None, 2.0), (2, 4, Some("2".into())));
        assert_eq!(page(4, Some(&json!("2")), 2.0), (0, 2, None));
        assert_eq!(page(5, Some(&json!(7)), 120.0), (0, 3, None));
        let ms = [
            json!({ "type": "user", "uuid": "u2", "timestamp": "2026-10-03T12:02:00Z", "message": { "content": [{ "type": "tool_result", "is_error": true, "content": [{ "type": "text", "text": "1 failed" }, { "type": "image" }] }, { "type": "text" }] } }),
        ];
        let es = entries(&ms).unwrap();
        assert_eq!(es.len(), 1);
        assert_eq!(get(&es[0], "text"), Some(&F::s("1 failed\n[image]")));
        assert_eq!(get(&es[0], "id"), Some(&F::s("u2:0")));
        assert!(entries(&[json!({ "message": { "content": {} } })]).is_err());
    }

    #[test]
    fn renames_only_the_same_session() {
        let r = json!({ "sessionId": "local_4", "cliSessionId": "c4", "title": "Old", "2": 1, "titleSource": "auto" });
        let out = renamed_record(r.clone(), Some(&json!("local_4")), Some(&json!("c4")), Some(&json!("New"))).unwrap();
        assert_eq!(out, r#"{"2":1,"sessionId":"local_4","cliSessionId":"c4","title":"New","titleSource":"user"}"#);
        assert_eq!(renamed_record(r, Some(&json!("local_4")), Some(&json!("c1")), Some(&json!("New"))), None);
        assert!(is_session_file("local_x.json") && !is_session_file("local_.json") && !is_session_file("notes.txt"));
    }
}
