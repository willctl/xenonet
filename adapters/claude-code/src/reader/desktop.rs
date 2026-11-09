//! Claude desktop's records of Claude Code's sessions: its session files
//! under `claude-code-sessions/<account>/<organization>/`, read from disk,
//! indexed by the CLI session each is of, and the checks and the edit
//! when a session is renamed in the desktop's record.

use serde_json::{Map, Value};
use std::path::Path;
use xenonet_base::text::truthy;
use xenonet_base::{json, jsstr, paths};
use xenonet_sources::lines::{opt_prop, Js};

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

// The most of a session's record that is read; the record is small, and a
// longer file is treated as one that can't be read.
const SESSION_FILE_MAX: u64 = 16 * 1024 * 1024;

/// Claude desktop's session files under `root` (its claude-code-sessions
/// folder), in the order a walk finds them: sessions sit in
/// `<account>/<organization>/`, so the folders say which Claude account
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
                let data = xenonet_base::fs::read_capped(Path::new(&full), SESSION_FILE_MAX).ok().and_then(|b| json::parse(&String::from_utf8_lossy(&b)).ok());
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

/// /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i: a
/// session's id.
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

// A value as a Map's key: by type and value, and an object as no other.
pub(super) fn key_of(v: Js<'_>, n: usize) -> String {
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

    #[test]
    fn renames_only_the_same_session() {
        let r = json!({ "sessionId": "local_4", "cliSessionId": "c4", "title": "Old", "2": 1, "titleSource": "auto" });
        let out = renamed_record(r.clone(), Some(&json!("local_4")), Some(&json!("c4")), Some(&json!("New"))).unwrap();
        assert_eq!(out, r#"{"2":1,"sessionId":"local_4","cliSessionId":"c4","title":"New","titleSource":"user"}"#);
        assert_eq!(renamed_record(r, Some(&json!("local_4")), Some(&json!("c1")), Some(&json!("New"))), None);
        assert!(is_session_file("local_x.json") && !is_session_file("local_.json") && !is_session_file("notes.txt"));
    }
}
