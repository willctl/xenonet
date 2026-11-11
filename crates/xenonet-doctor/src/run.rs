//! The doctor's runner: which checks `only` keeps, the summary, what
//! counts as a new problem, and the last run kept on disk.

use crate::checks::STATUSES;
use serde_json::{json, Map, Value};
use xenonet_os::files;

/// Whether `only` keeps a check: its id, or one it starts with and a dot.
pub fn kept(id: &str, only: &[String]) -> bool {
    only.iter().any(|o| id == o || id.starts_with(&format!("{o}.")))
}

/// How many results have each status.
pub fn summarize(results: &[Value]) -> Value {
    Value::Object(STATUSES.iter().map(|st| ((*st).to_string(), json!(results.iter().filter(|r| r.get("status").and_then(Value::as_str) == Some(st)).count()))).collect())
}

fn rank(s: Option<&str>) -> Option<u8> {
    match s {
        Some("ok" | "info" | "skip") => Some(0),
        Some("warn") => Some(1),
        Some("fail") => Some(2),
        _ => None,
    }
}

/// The problems in `next` that weren't in `prev`: what's worth telling you.
pub fn new_problems(prev: &Value, next: &Value) -> Vec<Value> {
    let before: Vec<(Value, Value)> = prev
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|r| (r.get("id").cloned().unwrap_or(Value::Null), r.get("status").cloned().unwrap_or(Value::Null)))
        .collect();
    let was = |id: Option<&Value>| before.iter().rev().find(|(k, _)| xenonet_text::value::same(Some(k), id)).and_then(|(_, s)| rank(s.as_str())).unwrap_or(0);
    next.get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|r| rank(r.get("status").and_then(Value::as_str)).is_some_and(|n| n > 0 && n > was(r.get("id"))))
        .cloned()
        .collect()
}

/// The last run, from its file; None when there's none to read.
pub fn read_run(file: Option<&str>) -> Option<Value> {
    xenonet_text::json::parse(&files::read_text(file?).ok()?).ok()
}

/// Keeps a run, saying what it is (contracts, section 1); quietly, as the
/// JS did, when it can't.
pub fn write_run(file: Option<&str>, result: &Value) {
    let Some(file) = file else { return };
    let mut m = Map::new();
    m.insert("schema".into(), json!("doctor/run/v1"));
    for (k, v) in result.as_object().into_iter().flatten() {
        m.insert(k.clone(), v.clone());
    }
    let tmp = format!("{file}.tmp");
    let _ = files::mkdir_p(&xenonet_os::nodepath::NATIVE.dirname(file), None)
        .and_then(|()| files::write_file(&tmp, xenonet_text::json::stringify(&Value::Object(m), 0).as_bytes(), None))
        .and_then(|()| files::rename(&tmp, file));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_only_keeps_and_what_is_new() {
        assert!(kept("network.link", &["network".into()]) && !kept("networkx", &["network".into()]));
        let prev = json!({ "results": [{ "id": "a", "status": "warn" }] });
        let next = json!({ "results": [{ "id": "a", "status": "warn" }, { "id": "b", "status": "fail" }, { "id": "c", "status": "ok" }] });
        assert_eq!(new_problems(&prev, &next), vec![json!({ "id": "b", "status": "fail" })]);
    }
}
