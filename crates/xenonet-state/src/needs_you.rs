//! core/needs-you.js: what waits on you, as one queue: sessions waiting on
//! a question or a permission, actors waiting on you, programs asking to be
//! actors, and sessions' asks for a gateway token. The longest wait comes
//! first; what can't be dated goes after what can, and a wait on another
//! machine that isn't live goes last.

use serde_json::{Map, Value};
use std::cmp::Ordering;
use xenonet_base::jsstr::cmp_units;
use xenonet_base::text::{js_string, truthy};

pub const KINDS: [&str; 6] = ["question", "permission", "actor", "request", "gateway", "requests"];

fn s(v: &Value) -> Option<&str> {
    v.as_str()
}

/// What an item waits for, in a few words after its name.
pub fn waits_words(it: &Value) -> &'static str {
    match it.get("kind").and_then(s) {
        Some("question") => "asks",
        Some("permission") => "needs permission",
        Some("actor") => match it.get("waits").and_then(s) {
            Some("question") => "asks",
            Some("permission") => "needs permission",
            Some("review") => "is ready for review",
            _ => "waits for you",
        },
        Some("request") => "asks to join XenoNet",
        Some("gateway") => "is asked for a gateway token",
        Some("requests") => {
            if it.get("count").and_then(Value::as_f64) == Some(1.0) {
                "asks to join XenoNet"
            } else {
                "ask to join XenoNet"
            }
        }
        _ => "waits for you",
    }
}

// Number.isFinite(v) ? v : null
fn num(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Number(n)) => Value::Number(n.clone()),
        _ => Value::Null,
    }
}

// typeof v === "string" ? v : v == null ? null : String(v)
fn str_of(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) => Value::String(s.clone()),
        Some(v) => Value::String(js_string(v)),
    }
}

fn or_id(v: Value, id: &Value) -> Value {
    if v.is_null() {
        id.clone()
    } else {
        v
    }
}

fn strings(xs: &[&str]) -> Value {
    Value::Array(xs.iter().map(|x| Value::String((*x).into())).collect())
}

fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// The queue: `input` is { sessions, actors, requests, gatewayAsks,
/// canDecide, waiting }, as needsYouQueue takes it.
pub fn needs_you_queue(input: &Value) -> Vec<Value> {
    let get = |k: &str| input.get(k);
    let can_decide = truthy(get("canDecide"));
    let mut out: Vec<Value> = Vec::new();
    for s in items(get("sessions")) {
        let kind = match s.get("kind").and_then(Value::as_str) {
            Some(k @ ("question" | "permission")) if truthy(s.get("id")) => k,
            _ => continue,
        };
        let id = s.get("id").cloned().unwrap_or(Value::Null);
        let local = !truthy(s.get("host"));
        let mut m = Map::new();
        m.insert("key".into(), Value::String(format!("session:{}", js_string(&id))));
        m.insert("kind".into(), Value::String(kind.into()));
        m.insert("id".into(), id.clone());
        m.insert("title".into(), or_id(str_of(s.get("title")), &id));
        m.insert("app".into(), str_of(s.get("app")));
        m.insert("text".into(), str_of(s.get("text")));
        m.insert("since".into(), num(s.get("since")));
        if !local {
            m.insert("host".into(), s["host"].clone());
        }
        if truthy(s.get("stale")) {
            m.insert("stale".into(), Value::Bool(true));
        }
        m.insert("actions".into(), strings(if local { &["show", "open", "dismiss"] } else { &["show"] }));
        out.push(Value::Object(m));
    }
    for a in items(get("actors")) {
        let Some(need) = a.get("needsYou").filter(|n| truthy(Some(n))) else { continue };
        if a.get("kind").and_then(s) == Some("human") || a.get("id").and_then(s) == Some("actor:you") {
            continue;
        }
        let id = a.get("id").cloned().unwrap_or(Value::Null);
        let text = match str_of(need.get("text")) {
            Value::String(t) if !t.is_empty() => Value::String(t),
            _ => Value::String("Waiting for you".into()),
        };
        let mut m = Map::new();
        m.insert("key".into(), Value::String(format!("actor:{}", a.get("id").map_or_else(|| "undefined".into(), js_string))));
        m.insert("kind".into(), Value::String("actor".into()));
        m.insert("id".into(), id.clone());
        m.insert("title".into(), or_id(str_of(a.get("name")), &id));
        m.insert("waits".into(), str_of(need.get("kind")));
        m.insert("text".into(), text);
        m.insert("since".into(), num(need.get("since")));
        m.insert("actions".into(), strings(if can_decide { &["show", "message"] } else { &["show"] }));
        out.push(Value::Object(m));
    }
    if can_decide {
        for (list, kind) in [("requests", "request"), ("gatewayAsks", "gateway")] {
            for r in items(get(list)) {
                if !truthy(r.get("id")) {
                    continue;
                }
                let id = r["id"].clone();
                let mut m = Map::new();
                m.insert("key".into(), Value::String(format!("{kind}:{}", js_string(&id))));
                m.insert("kind".into(), Value::String(kind.into()));
                m.insert("id".into(), id.clone());
                m.insert("title".into(), or_id(str_of(r.get("name")), &id));
                m.insert("actor".into(), str_of(r.get("actor")));
                m.insert("since".into(), num(r.get("at")));
                m.insert("expiresAt".into(), num(r.get("expiresAt")));
                m.insert("actions".into(), strings(&["allow", "deny"]));
                out.push(Value::Object(m));
            }
        }
    } else if let Some(waiting) = get("waiting").and_then(Value::as_f64).filter(|w| w.fract() == 0.0 && *w > 0.0) {
        let mut m = Map::new();
        m.insert("key".into(), Value::String("requests".into()));
        m.insert("kind".into(), Value::String("requests".into()));
        m.insert("id".into(), Value::Null);
        m.insert("title".into(), Value::String(if waiting == 1.0 { "A program".into() } else { format!("{} programs", xenonet_base::json::number_string(waiting)) }));
        m.insert("count".into(), xenonet_base::json::number(waiting));
        m.insert("since".into(), Value::Null);
        m.insert("actions".into(), Value::Array(Vec::new()));
        out.push(Value::Object(m));
    }
    xenonet_base::sort::sort_by(&mut out, order);
    out
}

fn since(v: &Value) -> Option<f64> {
    v.get("since").and_then(Value::as_f64)
}

/// The longest wait first; undated after dated, a host's stale waits last;
/// then by key.
pub fn order(a: &Value, b: &Value) -> Ordering {
    let stale = |v: &Value| truthy(v.get("stale"));
    let undated = |v: &Value| v.get("since").is_none_or(Value::is_null);
    let key = |v: &Value| v.get("key").map_or_else(String::new, js_string);
    stale(a)
        .cmp(&stale(b))
        .then(undated(a).cmp(&undated(b)))
        .then_with(|| since(a).unwrap_or(0.0).partial_cmp(&since(b).unwrap_or(0.0)).unwrap_or(Ordering::Equal))
        .then_with(|| cmp_units(&key(a), &key(b)))
}

/// The queue from /api/summary: its sessions, the actors that wait and how
/// many programs ask.
pub fn queue_of_summary(summary: Option<&Value>) -> Vec<Value> {
    let field = |k: &str| summary.and_then(|s| s.get(k)).filter(|v| !v.is_null());
    let actors: Vec<Value> = items(field("actorsNeedYou"))
        .iter()
        .map(|a| {
            let mut need = Map::new();
            for (to, from) in [("kind", "kind"), ("text", "text"), ("since", "since")] {
                if let Some(v) = a.get(from) {
                    need.insert(to.into(), v.clone());
                }
            }
            let mut m = Map::new();
            for k in ["id", "name"] {
                if let Some(v) = a.get(k) {
                    m.insert(k.into(), v.clone());
                }
            }
            m.insert("needsYou".into(), Value::Object(need));
            Value::Object(m)
        })
        .collect();
    let mut input = Map::new();
    input.insert("sessions".into(), field("needsYou").cloned().unwrap_or(Value::Array(Vec::new())));
    input.insert("actors".into(), Value::Array(actors));
    input.insert("waiting".into(), field("requests").cloned().unwrap_or(Value::from(0)));
    needs_you_queue(&Value::Object(input))
}

/// How many wait on you: every program that asks counts, and a wait on a
/// host that isn't live doesn't.
pub fn count_of(items: &[Value]) -> f64 {
    items.iter().fold(0.0, |n, it| {
        n + if truthy(it.get("stale")) {
            0.0
        } else if it.get("kind").and_then(Value::as_str) == Some("requests") {
            xenonet_base::text::js_number(it.get("count"))
        } else {
            1.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_longest_wait_first() {
        let q = needs_you_queue(&json!({
            "sessions": [
                { "id": "claude:b", "kind": "question", "since": 20, "title": "B" },
                { "id": "claude:a", "kind": "permission", "since": 10, "host": "@h", "stale": true },
                { "id": "codex:c", "kind": "review", "since": 1 },
                { "id": "codex:d", "kind": "question" },
            ],
            "actors": [{ "id": "actor:x", "name": "X", "needsYou": { "kind": "review", "since": 5 } }],
            "waiting": 2,
        }));
        let keys: Vec<&str> = q.iter().map(|i| i["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["actor:actor:x", "session:claude:b", "requests", "session:codex:d", "session:claude:a"]);
        assert_eq!(q[0]["text"], "Waiting for you");
        assert_eq!(waits_words(&q[0]), "is ready for review");
        assert_eq!(count_of(&q), 5.0);
    }

    #[test]
    fn the_summarys_queue_has_programs_that_ask_last() {
        let summary = json!({
            "needsYou": [{ "id": "codex:x", "app": "codex", "title": "Rate-limit the API", "kind": "question", "since": 2000 }, { "id": "claude:y", "kind": "review" }],
            "actorsNeedYou": [{ "id": "actor:wren", "name": "Wren", "kind": "permission", "text": "May I?", "since": 1000 }],
            "requests": 2,
        });
        let q = queue_of_summary(Some(&summary));
        let kinds: Vec<_> = q.iter().map(|it| it["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["actor", "question", "requests"]);
        assert_eq!(
            q[0],
            json!({ "key": "actor:actor:wren", "kind": "actor", "id": "actor:wren", "title": "Wren", "waits": "permission", "text": "May I?", "since": 1000, "actions": ["show"] })
        );
        assert_eq!(waits_words(&q[0]), "needs permission");
        assert_eq!(q[2]["title"], "2 programs");
        assert_eq!(waits_words(&q[2]), "ask to join XenoNet");
        assert_eq!(waits_words(&json!({ "kind": "requests", "count": 1 })), "asks to join XenoNet");
        assert_eq!(queue_of_summary(Some(&json!({}))), Vec::<Value>::new());
    }
}
