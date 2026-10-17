//! core/openrouter-usage.js: what your OpenRouter key has spent, for the
//! Token use panel, from OpenRouter's GET /api/v1/key, at most once a
//! minute. The key and the request stay with whoever runs this (the twin's
//! fetch); here is what an answer means.

use serde_json::{json, Map, Value};
use xenonet_base::json::number;
use xenonet_base::text::js_number;

pub const KEY_URL: &str = "https://openrouter.ai/api/v1/key";
/// How long a reading is kept before OpenRouter is asked again.
pub const EVERY_MS: f64 = 60e3;

// Number(v) when it's a finite number and v isn't null or "".
fn dollars(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if s.is_empty() => Value::Null,
        v => {
            let n = js_number(v);
            if n.is_finite() {
                number(n)
            } else {
                Value::Null
            }
        }
    }
}

/// OpenRouter's answer as the panel uses it.
pub fn usage_of(body: Option<&Value>) -> Value {
    let empty = Value::Object(Map::new());
    let d = body.and_then(|b| b.get("data")).filter(|d| !d.is_null()).unwrap_or(&empty);
    let f = |k: &str| d.get(k).filter(|_| d.is_object());
    json!({
        "today": dollars(f("usage_daily")),
        "week": dollars(f("usage_weekly")),
        "month": dollars(f("usage_monthly")),
        "total": dollars(f("usage")),
        "limit": dollars(f("limit")),
        "remaining": dollars(f("limit_remaining")),
        "reset": f("limit_reset").filter(|v| v.is_string()).cloned().unwrap_or(Value::Null),
        "freeTier": f("is_free_tier") == Some(&Value::Bool(true)),
    })
}

/// What a reading says: no key, a key OpenRouter refused or didn't answer
/// for, or the figures. `answer` is OpenRouter's status and whether it was
/// ok (res.ok), or None when it couldn't be reached.
pub fn reading(has_key: bool, answer: Option<(&Value, bool)>, body: Option<&Value>, at: f64) -> Value {
    if !has_key {
        return json!({ "key": "missing" });
    }
    let Some((status, ok)) = answer else { return json!({ "key": "stored", "error": "OpenRouter didn't answer" }) };
    if status.as_f64().is_some_and(|s| s == 401.0 || s == 403.0) {
        return json!({ "key": "rejected", "error": "OpenRouter didn't take the key" });
    }
    if !ok {
        return json!({ "key": "stored", "error": format!("OpenRouter answered {}", xenonet_base::text::js_string(status)) });
    }
    let mut out = Map::new();
    out.insert("key".into(), Value::String("stored".into()));
    out.insert("at".into(), number(at));
    for (k, v) in usage_of(body).as_object().unwrap() {
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}

/// Whether a reading taken at `at` still stands at `now`.
pub fn fresh(at: f64, now: f64) -> bool {
    now - at < EVERY_MS
}

/// The demo's key, made up.
pub fn demo(now: f64) -> Value {
    json!({ "key": "stored", "at": number(now), "today": 0.84, "week": 6.12, "month": 21.4, "total": 63.75, "limit": 50, "remaining": 28.6, "reset": "monthly", "freeTier": false })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        let body = json!({ "data": { "usage_daily": "0.5", "usage": 3, "limit": null, "limit_remaining": "", "is_free_tier": true } });
        let u = usage_of(Some(&body));
        assert_eq!(
            (u["today"].clone(), u["total"].clone(), u["limit"].clone(), u["remaining"].clone(), u["freeTier"].clone()),
            (json!(0.5), json!(3), Value::Null, Value::Null, json!(true))
        );
        assert_eq!(reading(true, Some((&json!(403), false)), None, 0.0)["key"], "rejected");
        assert_eq!(reading(true, Some((&json!(500), false)), None, 0.0)["error"], "OpenRouter answered 500");
        assert_eq!(reading(false, None, None, 0.0), json!({ "key": "missing" }));
        assert!(fresh(0.0, 59_999.0) && !fresh(0.0, 60_000.0));
    }
}
