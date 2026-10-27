//! The person's actions from another host (`network/operator/v1`;
//! docs/design/network.md, section 8): what a host checks before it acts
//! on one, and the rules for `settings`, which changes who may start and
//! wake sessions on it.
//!
//! Every action comes from a window host's console, as you, checked
//! strongly there, under a minute old and never seen before. Then:
//!
//! - `settings` narrows (`off`, `you`, `consent`) from any window host.
//!   Widening to `network` takes the founder, or a second window host:
//!   one from another window host waits (`pending`) until a different
//!   window host approves it, for ten minutes at most.
//! - `answer` is your Allow or Deny for a run's permission prompt, which
//!   the engine takes.

use serde_json::{json, Value};
use std::collections::HashMap;
use xenonet_base::network::{STARTS, WAKES};

/// What another host may ask of this one.
pub const ACTIONS: [&str; 6] = ["message", "ticket", "remove", "replace", "settings", "answer"];
/// An action older than this, either way, isn't acted on.
pub const FRESH_MS: f64 = 60e3;
/// A widening waits this long for its second window host.
pub const WIDEN_MS: f64 = 10.0 * 60e3;

/// Whether the person's action from `from` may be acted on: its host is a
/// window host, the action is one another machine may do, it's under a
/// minute old, it's new, and it was checked strongly. `seen` keeps the
/// nonces of the last two minutes.
pub fn check(from: &str, window: bool, body: &Value, now: f64, seen: &mut HashMap<String, f64>) -> Result<(), String> {
    if !window {
        return Err(format!("{from} isn't a window host"));
    }
    let action = body.get("action").and_then(Value::as_str).unwrap_or("");
    if !ACTIONS.contains(&action) {
        return Err(format!("{action} isn't done from another machine"));
    }
    let at = body.get("at").and_then(Value::as_f64).unwrap_or(0.0);
    if (now - at).abs() > FRESH_MS {
        return Err("it's more than a minute old".into());
    }
    let nonce = body.get("nonce").and_then(Value::as_str).unwrap_or("");
    seen.retain(|_, t| now - *t < 2.0 * FRESH_MS);
    if nonce.is_empty() || seen.insert(format!("{from}|{nonce}"), now).is_some() {
        return Err("it was seen before".into());
    }
    let checked = body.get("checked").and_then(Value::as_str);
    if !matches!(checked, Some("window" | "person")) || body.get("strength").and_then(Value::as_str) != Some("strong") {
        return Err(format!("it was checked {} on {from}, which isn't strong enough to act on", checked.unwrap_or("not at all")));
    }
    Ok(())
}

/// A change to `network` from a window host that isn't the founder,
/// waiting for a second one.
#[derive(Debug, Clone, PartialEq)]
pub struct Widen {
    pub id: String,
    /// `start` or `wake`.
    pub setting: String,
    pub by: String,
    pub at: f64,
}

impl Widen {
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "setting": self.setting, "value": "network", "by": self.by, "at": self.at.round() as i64 })
    }
}

/// What a `settings` action came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The settings as they are now.
    Set { start: String, wake: String },
    /// A widening waits for a second window host, with the settings that
    /// changed meanwhile.
    Waits { start: String, wake: String, widen: Widen },
}

/// A `settings` action's args (`{ start?, wake? }`, or `{ approve }` with
/// a waiting widening's id) from window host `from`, against this host's
/// settings now. `founder` says whether `from` is the network's founder;
/// `pending` holds the widenings that wait, which this keeps current;
/// `new_id` names a new one.
pub fn settings(cur: (&str, &str), args: &Value, from: &str, founder: bool, pending: &mut Vec<Widen>, now: f64, new_id: &mut dyn FnMut() -> String) -> Result<Outcome, String> {
    pending.retain(|w| now - w.at < WIDEN_MS);
    let (mut start, mut wake) = (cur.0.to_string(), cur.1.to_string());
    if let Some(id) = args.get("approve").and_then(Value::as_str) {
        let Some(i) = pending.iter().position(|w| w.id == id) else { return Err("that change isn't waiting here: it was approved, or it lapsed".into()) };
        if pending[i].by == from {
            return Err(format!("{from} asked for that change, so another window host approves it"));
        }
        let w = pending.remove(i);
        if w.setting == "start" {
            start = "network".into();
        } else {
            wake = "network".into();
        }
        return Ok(Outcome::Set { start, wake });
    }
    let wanted = |k: &str, allowed: &[&str]| -> Result<Option<String>, String> {
        match args.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(v)) if allowed.contains(&v.as_str()) => Ok(Some(v.clone())),
            Some(v) => Err(format!("{k} is one of {}, not {v}", allowed.join(", "))),
        }
    };
    let asked = [("start", wanted("start", &STARTS)?), ("wake", wanted("wake", &WAKES)?)];
    if asked.iter().all(|(_, v)| v.is_none()) {
        return Err("say what to set: start or wake".into());
    }
    let mut widen = None;
    for (k, v) in asked {
        let Some(v) = v else { continue };
        let had = if k == "start" { &mut start } else { &mut wake };
        if v != "network" || founder {
            // Narrowing, or the founder's: done. It takes back any widening
            // of the same setting that waits.
            *had = v;
            pending.retain(|w| w.setting != k);
        } else if *had != "network" {
            let w = Widen { id: new_id(), setting: k.into(), by: from.into(), at: now };
            pending.retain(|x| x.setting != k);
            pending.push(w.clone());
            widen = Some(w);
        }
    }
    Ok(match widen {
        Some(widen) => Outcome::Waits { start, wake, widen },
        None => Outcome::Set { start, wake },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_792_512_345_678.0;

    fn body(action: &str, at: f64, nonce: &str, checked: &str, strength: &str) -> Value {
        json!({ "schema": "network/operator/v1", "action": action, "args": {}, "checked": checked, "strength": strength, "nonce": nonce, "at": at })
    }

    #[test]
    fn an_action_comes_fresh_once_strong_from_a_window_host() {
        let mut seen = HashMap::new();
        assert_eq!(check("laptop", true, &body("settings", NOW, "n1", "window", "strong"), NOW, &mut seen), Ok(()));
        assert_eq!(check("laptop", true, &body("settings", NOW, "n1", "window", "strong"), NOW, &mut seen), Err("it was seen before".into()));
        assert_eq!(check("desk", true, &body("settings", NOW, "n1", "window", "strong"), NOW, &mut seen), Ok(()), "a nonce is per host");
        assert_eq!(check("desk", false, &body("answer", NOW, "n2", "window", "strong"), NOW, &mut seen), Err("desk isn't a window host".into()));
        assert_eq!(check("laptop", true, &body("update", NOW, "n3", "window", "strong"), NOW, &mut seen), Err("update isn't done from another machine".into()));
        assert_eq!(check("laptop", true, &body("answer", NOW - 61e3, "n4", "window", "strong"), NOW, &mut seen), Err("it's more than a minute old".into()));
        assert!(check("laptop", true, &body("answer", NOW + 61e3, "n5", "window", "strong"), NOW, &mut seen).is_err(), "nor dated ahead");
        assert_eq!(check("laptop", true, &body("answer", NOW, "", "window", "strong"), NOW, &mut seen), Err("it was seen before".into()), "no nonce");
        assert_eq!(
            check("laptop", true, &body("answer", NOW, "n6", "person", "weak"), NOW, &mut seen),
            Err("it was checked person on laptop, which isn't strong enough to act on".into())
        );
        assert!(check("laptop", true, &body("answer", NOW, "n7", "token", "strong"), NOW, &mut seen).is_err(), "only the window or the person");
        for a in ACTIONS {
            assert_eq!(check("laptop", true, &body(a, NOW, &format!("a-{a}"), "person", "strong"), NOW, &mut seen), Ok(()), "{a}");
        }
        // Two minutes on, a nonce may come again: it's long stale by then.
        assert_eq!(check("laptop", true, &body("settings", NOW + 121e3, "n1", "window", "strong"), NOW + 121e3, &mut seen), Ok(()));
    }

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("wid_{n}")
        }
    }

    fn set(start: &str, wake: &str) -> Outcome {
        Outcome::Set { start: start.into(), wake: wake.into() }
    }

    #[test]
    fn narrowing_from_any_window_host_and_widening_takes_the_founder_or_two() {
        let mut p = Vec::new();
        let mut id = ids();
        // Narrowing: any window host.
        assert_eq!(settings(("network", "network"), &json!({ "start": "off", "wake": "consent" }), "laptop", false, &mut p, NOW, &mut id), Ok(set("off", "consent")));
        assert_eq!(settings(("off", "consent"), &json!({ "start": "you" }), "laptop", false, &mut p, NOW, &mut id), Ok(set("you", "consent")), "you is no widening to the network");
        // The founder widens at once.
        assert_eq!(settings(("you", "consent"), &json!({ "start": "network", "wake": "network" }), "studio", true, &mut p, NOW, &mut id), Ok(set("network", "network")));
        assert_eq!(p.len(), 0);
        // Another window host's widening waits, with what else it set.
        let w = settings(("you", "consent"), &json!({ "start": "network", "wake": "consent" }), "laptop", false, &mut p, NOW, &mut id).unwrap();
        let Outcome::Waits { start, wake, widen } = w else { panic!("{w:?}") };
        assert_eq!((start.as_str(), wake.as_str(), widen.setting.as_str(), widen.by.as_str()), ("you", "consent", "start", "laptop"));
        assert_eq!(widen.to_json()["value"], "network");
        // Its own host can't approve it; another window host can, once.
        assert_eq!(
            settings(("you", "consent"), &json!({ "approve": "wid_1" }), "laptop", false, &mut p, NOW, &mut id),
            Err("laptop asked for that change, so another window host approves it".into())
        );
        assert_eq!(settings(("you", "consent"), &json!({ "approve": "wid_1" }), "desk", false, &mut p, NOW + 1000.0, &mut id), Ok(set("network", "consent")));
        assert!(settings(("network", "consent"), &json!({ "approve": "wid_1" }), "desk", false, &mut p, NOW, &mut id).is_err(), "once");
        // A widening lapses after ten minutes, and a narrowing takes one back.
        let _ = settings(("you", "consent"), &json!({ "wake": "network" }), "laptop", false, &mut p, NOW, &mut id).unwrap();
        assert_eq!(p.len(), 1);
        assert!(settings(("you", "consent"), &json!({ "approve": p[0].id.clone() }), "desk", false, &mut p, NOW + WIDEN_MS, &mut id).is_err());
        let _ = settings(("you", "consent"), &json!({ "wake": "network" }), "laptop", false, &mut p, NOW, &mut id).unwrap();
        assert_eq!(settings(("you", "consent"), &json!({ "wake": "consent" }), "desk", false, &mut p, NOW, &mut id), Ok(set("you", "consent")));
        assert_eq!(p.len(), 0, "the narrowing took the waiting widening back");
        // What isn't a setting's value is refused, and so is nothing at all.
        assert!(settings(("you", "consent"), &json!({ "start": "everyone" }), "laptop", false, &mut p, NOW, &mut id).is_err());
        assert_eq!(settings(("you", "consent"), &json!({}), "laptop", false, &mut p, NOW, &mut id), Err("say what to set: start or wake".into()));
        // Already the network's: nothing waits.
        assert_eq!(settings(("network", "consent"), &json!({ "start": "network" }), "laptop", false, &mut p, NOW, &mut id), Ok(set("network", "consent")));
    }
}
