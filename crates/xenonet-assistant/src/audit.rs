//! core/assistant/audit.js: the log of what remote actors read, as
//! engine.sqlite keeps it (the store is the engine's): when, which
//! request, grant, revision, actor and connection, which operation, a
//! result code, how long it took, how many bytes went and how many
//! projects it could see. Never a title, a name, an argument, a
//! credential, a path or an account.

use serde_json::{json, Map, Value};
use xenonet_base::text::js_string;

const OPERATIONS: [&str; 8] = ["status", "projects", "sessions", "attention", "session", "actors", "hello", "unknown"];

fn code_ok(s: &str) -> bool {
    (1..=40).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'-')
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn integer(v: Option<&Value>) -> Value {
    finite(v).filter(|n| n.fract() == 0.0).map_or(Value::Null, xenonet_base::json::number)
}

/// The record store.addAudit gets for a read, or a refusal, through a
/// grant: `a` as audit.access() was given it.
pub fn access(a: &Value, now: f64) -> Value {
    let field = |k: &str| a.get(k).cloned().unwrap_or(Value::Null);
    let op = a.get("operation").and_then(Value::as_str).filter(|o| OPERATIONS.contains(o)).unwrap_or("unknown");
    let result = a.get("result").map_or_else(|| "undefined".to_string(), js_string);
    json!({
        "at": xenonet_base::json::number(now),
        "kind": "access",
        "request_id": field("requestId"),
        "grant_id": field("grant"),
        "grant_revision": integer(a.get("revision")),
        "actor": field("actor"),
        "connection_id": field("connection"),
        "operation": op,
        "result": if code_ok(&result) { result } else { "unknown".into() },
        "latency_ms": finite(a.get("latencyMs")).map_or(Value::Null, |l| xenonet_base::json::number(xenonet_base::units::js_round(l))),
        "bytes": finite(a.get("bytes")).map_or(Value::Null, xenonet_base::json::number),
        "projects": integer(a.get("projects")),
    })
}

/// How many records recent() asks for: 1 to 1000.
pub fn limit_of(limit: f64) -> f64 {
    limit.max(1.0).min(1000.0)
}

/// The store's records as recent() gives them: without the database's own
/// id, and `at` as ISO 8601.
pub fn recent(rows: &[Value]) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            let mut m: Map<String, Value> = r.as_object().cloned().unwrap_or_default();
            m.shift_remove("id");
            let at = xenonet_base::text::js_number(m.get("at"));
            m.insert("at".into(), json!(xenonet_base::time::iso(at)));
            Value::Object(m)
        })
        .collect()
}
