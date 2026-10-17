//! How much of its plan a provider has used, from 0 to 1, for the wake
//! budget's rule that a provider at 90% takes no wakes (policy):
//! core/delivery/usage.js. Read from the live limit readings: the fuller of
//! the provider's windows that hasn't reset yet. None when there's no fresh
//! reading, and for a provider XenoNet reads no limits for.

use crate::sql::Arg;

const FRESH_MS: f64 = 6.0 * 3600e3;

fn num(a: &Arg) -> f64 {
    match a {
        Arg::Num(n) => *n,
        Arg::Null => 0.0,
        Arg::Bool(b) => f64::from(u8::from(*b)),
        Arg::Text(s) => xenonet_base::text::js_number(Some(&serde_json::Value::String(s.clone()))),
        _ => f64::NAN,
    }
}

/// `x && x <= now`: a reset that's passed.
fn passed(x: &Arg, now: f64) -> bool {
    x.truthy() && num(x) <= now
}

fn finite(a: &Arg) -> Option<f64> {
    match a {
        Arg::Num(n) if n.is_finite() => Some(*n),
        _ => None,
    }
}

pub fn plan_usage(limits: &Arg, app: &Arg, now: f64) -> Option<f64> {
    let mut pcts = Vec::new();
    match app.as_str() {
        Some("codex") => {
            let c = limits.field("codex");
            if !c.truthy() || now - num(&c.field("observedAt").or(Arg::num(0.0))) > FRESH_MS {
                return None;
            }
            for w in [c.field("primary"), c.field("secondary")] {
                if let Some(p) = finite(&w.field("pct")) {
                    if !passed(&w.field("resetsAt"), now) {
                        pcts.push(p);
                    }
                }
            }
        }
        Some("claude") => {
            let c = limits.field("claude");
            if !c.truthy() || c.field("stale").truthy() {
                return None;
            }
            for k in ["fiveHour", "sevenDay"] {
                if let Some(p) = finite(&c.field(k)) {
                    if !passed(&c.field("resets").field(k), now) {
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
