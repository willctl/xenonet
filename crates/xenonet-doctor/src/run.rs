//! core/doctor/index.js's runner and core/doctor/service.js's file: a
//! check for each host, which checks `only` keeps, the summary, what
//! counts as a new problem, and the last run kept on disk.

use crate::checks::STATUSES;
use serde_json::{json, Map, Value};
use xenonet_app::fsx;
use xenonet_app::jsv::truthy;
use xenonet_base::text::js_string;

const SKEW_MS: f64 = 120e3;

fn span(ms: f64) -> String {
    let s = ms.abs() / 1000.0;
    let r = xenonet_base::units::js_round;
    let n = xenonet_base::json::number_string;
    if s < 90.0 {
        format!("{} s", n(r(s)))
    } else if s < 5400.0 {
        format!("{} min", n(r(s / 60.0)))
    } else {
        format!("{} h", n(r(s / 3600.0)))
    }
}

fn verdict(status: &str, detail: String, fix: Option<String>) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), json!(status));
    m.insert("detail".into(), json!(detail));
    if let Some(f) = fix.filter(|f| !f.is_empty()) {
        m.insert("fix".into(), json!(f));
    }
    Value::Object(m)
}

/// A host's check: reachable, speaking a protocol this XenoNet speaks, and
/// its clock within two minutes of this one's. `h` is how the hub lists it.
pub fn host_check(h: &Value, now: f64) -> Value {
    let s = |v: Option<&Value>| v.filter(|x| !x.is_null()).map(js_string);
    let id = s(h.get("id")).unwrap_or_else(|| "undefined".into());
    let error = h.get("error").filter(|e| truthy(Some(e)));
    let fix = error.and_then(|e| s(e.get("fix")));
    let message = error.and_then(|e| s(e.get("message")));
    let last = h.get("lastHeard").and_then(Value::as_f64).filter(|t| *t != 0.0);
    match h.get("state").and_then(Value::as_str) {
        Some("error") => return verdict("fail", message.unwrap_or_else(|| format!("{id} needs you")), fix),
        Some("offline") => {
            let heard = last.map_or(String::new(), |t| format!(", last heard {} ago", span(now - t)));
            let cached = if truthy(h.get("cached")) { "; the map shows its last snapshot" } else { "" };
            return verdict(
                "warn",
                format!("offline{heard}: {}{cached}", message.unwrap_or_else(|| "the connection ended".into())),
                Some(fix.unwrap_or_else(|| "XenoNet tries again by itself.".into())),
            );
        }
        Some("stale") => {
            return verdict(
                "warn",
                format!("no word from it for {}", span(now - h.get("lastHeard").and_then(Value::as_f64).unwrap_or(now))),
                Some("XenoNet waits a little longer, then connects again.".into()),
            )
        }
        Some("connecting") => return verdict("info", format!("connecting to {}", s(h.get("target")).unwrap_or_else(|| id.clone())), None),
        _ => {}
    }
    let skew = h.get("offsetMs").and_then(Value::as_f64).filter(|x| x.is_finite());
    let off = skew.filter(|x| x.abs() >= 2000.0).map(|x| format!("its clock {} {} this one's", span(x), if x > 0.0 { "ahead of" } else { "behind" }));
    if skew.is_some_and(|x| x.abs() > SKEW_MS) {
        return verdict(
            "warn",
            format!("live, but {}", off.unwrap_or_default()),
            Some(format!("Have {id} set its clock by itself: on Linux, timedatectl set-ntp true; on a Mac, System Settings › General › Date & Time.")),
        );
    }
    let version = s(h.get("version")).unwrap_or_else(|| "of an unknown version".into());
    let platform = s(h.get("platform")).unwrap_or_else(|| "an unknown system".into());
    let demo = if truthy(h.get("demo")) { ", with made-up data" } else { "" };
    verdict("ok", format!("live: XenoNet {version} on {platform}{demo}{}", off.map_or(String::new(), |o| format!(", {o}"))), None)
}

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
    let was = |id: Option<&Value>| before.iter().rev().find(|(k, _)| xenonet_app::jsv::strict_eq(Some(k), id)).and_then(|(_, s)| rank(s.as_str())).unwrap_or(0);
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
    xenonet_base::json::parse(&fsx::read_text(file?).ok()?).ok()
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
    let _ = fsx::mkdir_p(&xenonet_app::nodepath::NATIVE.dirname(file), None)
        .and_then(|()| fsx::write_file(&tmp, xenonet_base::json::stringify(&Value::Object(m), 0).as_bytes(), None))
        .and_then(|()| fsx::rename(&tmp, file));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_and_its_clock() {
        let h = json!({ "id": "vm1", "state": "live", "offsetMs": 180e3, "version": "0.4.0", "platform": "linux" });
        assert_eq!(host_check(&h, 0.0)["detail"], json!("live, but its clock 3 min ahead of this one's"));
        let h = json!({ "id": "vm1", "state": "offline", "lastHeard": 1000, "cached": true });
        assert_eq!(host_check(&h, 61000.0)["detail"], json!("offline, last heard 60 s ago: the connection ended; the map shows its last snapshot"));
        assert!(kept("hosts.vm1", &["hosts".into()]) && !kept("hostsx", &["hosts".into()]));
        let prev = json!({ "results": [{ "id": "a", "status": "warn" }] });
        let next = json!({ "results": [{ "id": "a", "status": "warn" }, { "id": "b", "status": "fail" }, { "id": "c", "status": "ok" }] });
        assert_eq!(new_problems(&prev, &next), vec![json!({ "id": "b", "status": "fail" })]);
    }
}
