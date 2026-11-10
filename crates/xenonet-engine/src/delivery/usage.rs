//! How much of its plan a provider has used, from 0 to 1, for the wake
//! budget's rule that a provider at 90% takes no wakes (policy):
//! core/delivery/usage.js. Read from the live limit readings: the fuller of
//! the provider's windows that hasn't reset yet. None when there's no fresh
//! reading, and for a provider XenoNet reads no limits for.

use serde_json::Value;

/// The plan limits an app has, as live/snapshot/v1 names them (the
/// registry's limits): Claude Code's or Codex's.
fn limits_of(app: &str) -> Option<&'static str> {
    xenonet_adapter::apps::app(app)?.limits.as_deref()
}

/// `limits` is the live snapshot's `limits`; `app` the session's app. The
/// shape of the app's own reading is its adapter's (`plan_used`).
pub fn plan_usage(limits: &Value, app: &str, now: f64) -> Option<f64> {
    let name = limits_of(app)?;
    let read = super::apps::of(app)?.plan_used?;
    let pcts = read(limits.get(name), now)?;
    if pcts.is_empty() {
        return None;
    }
    Some(pcts.iter().copied().fold(f64::NEG_INFINITY, f64::max) / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_fuller_window_that_hasnt_reset() {
        xenonet_platforms::install();
        let limits = json!({
            "claude": { "fiveHour": 40, "sevenDay": 95, "resets": { "sevenDay": 10 } },
            "codex": { "observedAt": 100, "primary": { "pct": 30 }, "secondary": { "pct": 80, "resetsAt": 200 } },
        });
        assert_eq!(plan_usage(&limits, "claude", 100.0), Some(0.4), "the seven-day window reset");
        assert_eq!(plan_usage(&limits, "codex", 150.0), Some(0.8));
        assert_eq!(plan_usage(&limits, "codex", 100.0 + 7.0 * 3600e3), None, "a reading six hours old");
        assert_eq!(plan_usage(&json!({ "claude": { "fiveHour": 50, "stale": true } }), "claude", 1.0), None);
        assert_eq!(plan_usage(&limits, "grok", 100.0), None, "no limits read");
    }
}
