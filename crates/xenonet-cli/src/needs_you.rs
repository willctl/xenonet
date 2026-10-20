//! What waits on you, as one queue (core/needs-you.js, which stays JS for
//! the page; this is the command's copy, as units.rs is): sessions waiting
//! on a question or a permission, actors waiting on you, programs asking
//! to be actors, and sessions' asks for an actor's gateway token. xenonet
//! status reads it from /api/summary.
//!
//! The queue is in the order things started waiting, the longest wait
//! first. What XenoNet can't date goes after what it can, and a wait on
//! another machine that isn't live, which may be over, goes last.

use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use xenonet_base::text::{js_string, truthy};

/// What each kind of item waits for, in a few words after its name.
pub fn waits_words(it: &Value) -> String {
    let kind = it.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "question" => "asks",
        "permission" => "needs permission",
        "actor" => match it.get("waits").and_then(Value::as_str) {
            Some("question") => "asks",
            Some("permission") => "needs permission",
            Some("review") => "is ready for review",
            _ => "waits for you",
        },
        "request" => "asks to join XenoNet",
        "gateway" => "is asked for a gateway token",
        "requests" => {
            if it.get("count").and_then(Value::as_f64) == Some(1.0) {
                "asks to join XenoNet"
            } else {
                "ask to join XenoNet"
            }
        }
        _ => "waits for you",
    }
    .to_string()
}

// Number.isFinite(v) ? v : null
fn num(v: Option<&Value>) -> Value {
    match v {
        Some(Value::Number(n)) if n.as_f64().is_some_and(f64::is_finite) => Value::Number(n.clone()),
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

/// needsYouQueue, for a window without the key, as the summary has it:
/// sessions, the actors that wait and how many programs ask.
pub fn queue(sessions: &[Value], actors: &[Value], waiting: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    for s in sessions {
        let kind = s.get("kind").and_then(Value::as_str);
        if !truthy(s.get("id")) || !matches!(kind, Some("question" | "permission")) {
            continue;
        }
        let id = s["id"].clone();
        let host = s.get("host").filter(|h| truthy(Some(h)));
        let mut it = Map::new();
        it.insert("key".into(), Value::String(format!("session:{}", js_string(&id))));
        it.insert("kind".into(), json!(kind));
        it.insert("id".into(), id.clone());
        it.insert("title".into(), or_id(str_of(s.get("title")), &id));
        it.insert("app".into(), str_of(s.get("app")));
        it.insert("text".into(), str_of(s.get("text")));
        it.insert("since".into(), num(s.get("since")));
        if let Some(h) = host {
            it.insert("host".into(), h.clone());
        }
        if truthy(s.get("stale")) {
            it.insert("stale".into(), Value::Bool(true));
        }
        it.insert("actions".into(), if host.is_none() { json!(["show", "open", "dismiss"]) } else { json!(["show"]) });
        items.push(Value::Object(it));
    }
    for a in actors {
        let Some(needs) = a.get("needsYou").filter(|n| truthy(Some(n))) else { continue };
        if a.get("kind").and_then(Value::as_str) == Some("human") || a.get("id").and_then(Value::as_str) == Some("actor:you") {
            continue;
        }
        let id = a.get("id").cloned().unwrap_or(Value::Null);
        let text = str_of(needs.get("text"));
        let mut it = Map::new();
        it.insert("key".into(), Value::String(format!("actor:{}", a.get("id").map_or("undefined".into(), js_string))));
        it.insert("kind".into(), json!("actor"));
        it.insert("id".into(), id.clone());
        it.insert("title".into(), or_id(str_of(a.get("name")), &id));
        it.insert("waits".into(), str_of(needs.get("kind")));
        it.insert("text".into(), if truthy(Some(&text)) { text } else { json!("Waiting for you") });
        it.insert("since".into(), num(needs.get("since")));
        it.insert("actions".into(), json!(["show"]));
        items.push(Value::Object(it));
    }
    if let Some(n) = waiting.as_f64().filter(|n| n.fract() == 0.0 && *n > 0.0) {
        let title = if n == 1.0 { "A program".to_string() } else { format!("{} programs", xenonet_base::json::number_string(n)) };
        items.push(json!({ "key": "requests", "kind": "requests", "id": null, "title": title, "count": waiting, "since": null, "actions": [] }));
    }
    items.sort_by(order);
    items
}

/// The longest wait first; undated after dated, a host's stale waits last;
/// then by key, so two that started together keep their places.
pub fn order(a: &Value, b: &Value) -> Ordering {
    let stale = |v: &Value| truthy(v.get("stale"));
    let since = |v: &Value| v.get("since").and_then(Value::as_f64);
    let key = |v: &Value| v.get("key").map(js_string).unwrap_or_default();
    stale(a)
        .cmp(&stale(b))
        .then(since(a).is_none().cmp(&since(b).is_none()))
        .then(since(a).unwrap_or(0.0).partial_cmp(&since(b).unwrap_or(0.0)).unwrap_or(Ordering::Equal))
        .then_with(|| key(a).cmp(&key(b)))
}

fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The queue from /api/summary: its sessions, the actors that wait and
/// how many programs ask.
pub fn queue_of_summary(summary: &Value) -> Vec<Value> {
    let actors: Vec<Value> = list(summary.get("actorsNeedYou"))
        .iter()
        .map(|a| {
            let at = |k: &str| a.get(k).cloned();
            let mut needs = Map::new();
            for (to, from) in [("kind", "kind"), ("text", "text"), ("since", "since")] {
                if let Some(v) = at(from) {
                    needs.insert(to.into(), v);
                }
            }
            let mut out = Map::new();
            for k in ["id", "name"] {
                if let Some(v) = at(k) {
                    out.insert(k.into(), v);
                }
            }
            out.insert("needsYou".into(), Value::Object(needs));
            Value::Object(out)
        })
        .collect();
    let waiting = summary.get("requests").filter(|r| !r.is_null()).cloned().unwrap_or(json!(0));
    queue(&list(summary.get("needsYou")), &actors, &waiting)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_wait_first_and_programs_that_ask_last() {
        let summary = json!({
            "needsYou": [{ "id": "codex:x", "app": "codex", "title": "Rate-limit the API", "kind": "question", "since": 2000 }, { "id": "claude:y", "kind": "review" }],
            "actorsNeedYou": [{ "id": "actor:wren", "name": "Wren", "kind": "permission", "text": "May I?", "since": 1000 }],
            "requests": 2,
        });
        let q = queue_of_summary(&summary);
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
        assert_eq!(queue_of_summary(&json!({})), Vec::<Value>::new());
    }
}
