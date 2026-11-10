//! How much of its plan a provider has used, from 0 to 1, for the wake
//! budget's rule that a provider at 90% takes no wakes (policy):
//! core/delivery/usage.js. Read from the live limit readings: the fuller of
//! the provider's windows that hasn't reset yet. None when there's no fresh
//! reading, and for a provider XenoNet reads no limits for.

use super::truthy;
use serde_json::Value;

const FRESH_MS: f64 = 6.0 * 3600e3;

/// Number(x) of a reading's field; NaN for one that isn't there.
fn num(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(v @ Value::String(_)) => xenonet_text::text::to_number(Some(v)),
        Some(_) => f64::NAN,
    }
}

/// `x && x <= now`: a reset that's passed.
fn passed(x: Option<&Value>, now: f64) -> bool {
    truthy(x) && num(x) <= now
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// The plan limits an app has, as live/snapshot/v1 names them (the
/// registry's limits): Claude Code's or Codex's.
fn limits_of(app: &str) -> Option<&'static str> {
    xenonet_adapter::apps::app(app)?.limits.as_deref()
}

/// `limits` is the live snapshot's `limits`; `app` the session's app.
pub fn plan_usage(limits: &Value, app: &str, now: f64) -> Option<f64> {
    let mut pcts = Vec::new();
    match limits_of(app) {
        Some("codex") => {
            let c = limits.get("codex");
            if !truthy(c) || now - c.and_then(|c| c.get("observedAt")).filter(|v| !v.is_null()).map_or(0.0, |v| num(Some(v))) > FRESH_MS {
                return None;
            }
            for w in ["primary", "secondary"] {
                let w = c.and_then(|c| c.get(w));
                if let Some(p) = finite(w.and_then(|w| w.get("pct"))) {
                    if !passed(w.and_then(|w| w.get("resetsAt")), now) {
                        pcts.push(p);
                    }
                }
            }
        }
        Some("claude") => {
            let c = limits.get("claude");
            if !truthy(c) || truthy(c.and_then(|c| c.get("stale"))) {
                return None;
            }
            for k in ["fiveHour", "sevenDay"] {
                if let Some(p) = finite(c.and_then(|c| c.get(k))) {
                    if !passed(c.and_then(|c| c.get("resets")).and_then(|r| r.get(k)), now) {
                        pcts.push(p);
                    }
                }
            }
        }
        _ => {}
    }
    if pcts.is_empty() {
        return None;
    }
    Some(pcts.iter().copied().fold(f64::NEG_INFINITY, f64::max) / 100.0)
}
