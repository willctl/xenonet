//! Grok Build's usage ledger: a session's usage.json, which its folder
//! (`~/.grok/sessions/<encoded cwd>/<session id>/`) keeps beside its
//! summary.json, read as the turns and model calls a session's stats
//! count (`Tailed::usage_turns`) and the live tailer adds up; and the times
//! its files write.

use serde_json::{Map, Value};
pub use xenonet_adapter::reader::UsageTurn;
use xenonet_text::time;

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

fn text(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn nonempty(v: Option<&Value>) -> Option<&str> {
    text(v).filter(|s| !s.is_empty())
}

fn object(v: Option<&Value>) -> Option<&Map<String, Value>> {
    v.and_then(Value::as_object)
}

// A figure that's a positive number, else none.
fn count(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0).unwrap_or(0.0)
}

/// Whether a session is a subagent's, whose turns its parent's ledger
/// counts too: Grok adds a subagent's usage into the turn of its parent's
/// that ran it.
pub fn usage_subagent(summary: Option<&Value>) -> bool {
    let s = object(summary);
    let kind = s.and_then(|m| m.get("session_kind")).and_then(Value::as_str);
    kind.is_some_and(|k| k.starts_with("subagent")) && nonempty(s.and_then(|m| m.get("parent_session_id"))).is_some()
}

/// A session's turns from its usage.json (core/grok-usage.js's grokUsageTurns),
/// as calls, one for each model a turn used: its input apart from what was
/// read from the cache or written to it, since Grok's inputTokens is the
/// whole prompt; its output, reasoning included; and its cost in dollars
/// (costUsdTicks / 1e10), None when no call said or only some did. A turn
/// with no time it ended is left out, and so, in a session that came from
/// another, are the turns that ended before it was made (`forked_at`, else
/// `created_at`), which are the other session's.
pub fn usage_turns(usage: Option<&Value>, summary: Option<&Value>) -> Vec<UsageTurn> {
    let s = object(summary);
    let at = |k: &str| s.and_then(|m| m.get(k));
    let since = nonempty(at("parent_session_id")).and_then(|_| grok_time(at("forked_at")).or_else(|| grok_time(at("created_at"))));
    let fallback = nonempty(at("current_model_id")).unwrap_or("grok").to_string();
    let turns = object(usage).and_then(|m| m.get("turns")).and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let mut out = Vec::new();
    for turn in turns {
        let r = object(Some(turn));
        let field = |k: &str| r.and_then(|m| m.get(k));
        let Some(t) = grok_time(field("endedAt")) else { continue };
        if since.is_some_and(|since| t <= since) {
            continue;
        }
        let n = match field("turnNumber") {
            Some(Value::Number(x)) => xenonet_text::json::number_string(x.as_f64().unwrap_or(f64::NAN)),
            Some(Value::String(x)) => x.clone(),
            _ => String::new(),
        };
        let by_model: Vec<(String, Option<&Value>)> = object(field("modelUsage")).map(|m| m.iter().map(|(k, v)| (k.clone(), Some(v))).collect()).unwrap_or_default();
        let rows = if by_model.is_empty() { vec![(nonempty(field("primaryModelId")).map_or_else(|| fallback.clone(), str::to_string), Some(turn))] } else { by_model };
        for (model, u) in rows {
            let row = object(u);
            let get = |k: &str| row.and_then(|m| m.get(k));
            let calls = get("modelCalls").and_then(Value::as_f64).filter(|c| c.is_finite() && *c >= 0.0).unwrap_or(1.0);
            let (cache_read, cache_write) = (count(get("cachedReadTokens")), count(get("cacheCreationTokens")));
            let cost = get("costUsdTicks").and_then(Value::as_f64).filter(|c| c.is_finite() && get("costIsPartial") != Some(&Value::Bool(true))).map(|c| c / 1e10);
            out.push(UsageTurn {
                t,
                key: format!("{n}|{model}"),
                model,
                calls,
                input: (count(get("inputTokens")) - cache_read - cache_write).max(0.0),
                cache_read,
                cache_write,
                output: count(get("outputTokens")),
                cost,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn times_as_grok_writes_them() {
        assert_eq!(grok_time(Some(&json!("2026-10-09T15:00:00Z"))), Some(1791558000000.0));
        assert_eq!(grok_time(Some(&json!(1791558000))), Some(1791558000000.0));
        assert_eq!(grok_time(Some(&json!("soon"))), None);
    }

    #[test]
    fn a_sessions_turns_from_its_usage() {
        let usage = json!({ "turns": [
            { "turnNumber": 1, "endedAt": "2026-10-09T14:50:00Z", "inputTokens": 100, "outputTokens": 10 },
            { "turnNumber": 2, "endedAt": "2026-10-09T15:05:00Z", "modelUsage": {
                "grok-build": { "inputTokens": 5000, "cachedReadTokens": 4000, "cacheCreationTokens": 200, "outputTokens": 300, "modelCalls": 3, "costUsdTicks": 120000000 },
                "grok-build-mini": { "inputTokens": 900, "outputTokens": 20, "costUsdTicks": 1, "costIsPartial": true }
            } },
            { "turnNumber": 3, "inputTokens": 5 },
        ] });
        let fork = json!({ "parent_session_id": "a", "created_at": "2026-10-09T15:00:00Z", "current_model_id": "grok-build" });
        let turns = usage_turns(Some(&usage), Some(&fork));
        assert_eq!(
            turns.iter().map(|c| (c.key.as_str(), c.calls, c.input, c.cache_read, c.cache_write, c.output, c.cost)).collect::<Vec<_>>(),
            [("2|grok-build", 3.0, 800.0, 4000.0, 200.0, 300.0, Some(0.012)), ("2|grok-build-mini", 1.0, 900.0, 0.0, 0.0, 20.0, None)],
            "a fork's own turns, its cost when it's whole"
        );
        let all = usage_turns(Some(&usage), Some(&json!({ "current_model_id": "grok-build" })));
        assert_eq!((all.len(), all[0].key.as_str(), all[0].t), (3, "1|grok-build", 1791557400000.0));
        assert!(usage_subagent(Some(&json!({ "session_kind": "subagent", "parent_session_id": "a" }))) && !usage_subagent(Some(&fork)) && !usage_subagent(None));
        assert_eq!(usage_turns(None, None), Vec::new());
    }
}
