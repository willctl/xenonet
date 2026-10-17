//! core/reported-runs.js: runs an actor reports through the actor gateway
//! (its report_run tool), such as a harness running in a sandbox. Each run
//! is a session on the map, "<app>:<id>" with app its harness, marked
//! observed through the gateway and reported by its actor. Kept in memory:
//! a run goes when it hasn't reported for an hour, or ten minutes after it
//! ended, and with the actor's gateway token.

use crate::native::{older, Out};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashSet, VecDeque};
use xenonet_base::ids::{is_session_app, SESSION_APPS};
use xenonet_base::jsstr;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::text::js_trim;

const STALE_MS: f64 = 3600e3;
const ENDED_KEEP_MS: f64 = 10.0 * 60e3;
const PER_ACTOR: usize = 32;
const WORKING_MS: f64 = 120e3;

/// A session id from the actor and its own name for the run, so one actor
/// can't name another's runs.
pub fn id_of(actor: &str, app: &str, run: &str) -> String {
    let hex = crate::native::hex(&Sha256::digest(format!("{actor}\n{run}").as_bytes()));
    format!("{app}:r{}", &hex[..31])
}

// /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/
fn fits_run(run: &str) -> bool {
    let b = run.as_bytes();
    (1..=64).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// Why a report isn't one: its run's name, its app or its presence.
pub fn refusal(run: Option<&str>, app: &str, presence: Option<&str>) -> Option<String> {
    if !run.is_some_and(fits_run) {
        return Some("run is 1 to 64 letters, digits, dots, dashes or underscores".into());
    }
    if !is_session_app(app) {
        let apps: Vec<&str> = SESSION_APPS.iter().map(|(a, _)| *a).collect();
        return Some(format!("app is the harness that runs, one of {}", apps.join(", ")));
    }
    if !presence.is_some_and(|p| ["working", "idle", "ended"].contains(&p)) {
        return Some("presence is working, idle or ended".into());
    }
    None
}

#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub id: String,
    pub actor: String,
    pub app: String,
    pub run: String,
    pub created_at: f64,
    pub updated_at: f64,
    pub ended_at: Option<f64>,
    pub title: Option<String>,
    /// Which run under this id it is: a run that went and was reported
    /// again is another.
    pub serial: u64,
}

impl Run {
    /// working while it reports, idle once it's quiet or ended.
    pub fn status(ended_at: Option<f64>, updated_at: f64, now: f64) -> &'static str {
        if ended_at.is_none() && now - updated_at < WORKING_MS {
            "working"
        } else {
            "idle"
        }
    }
}

// How many runs that went are remembered, for the status of a live view
// taken before they went.
const GONE_KEPT: usize = 256;

#[derive(Default)]
pub struct ReportedRuns {
    pub runs: OrderedMap<Run>,
    serials: u64,
    // Runs that went, newest last: (serial, endedAt, updatedAt).
    gone: VecDeque<(u64, Option<f64>, f64)>,
}

impl ReportedRuns {
    pub fn new() -> Self {
        Self::default()
    }

    /// The run's session id, or why it isn't one. `now` is asked only once
    /// the report is good.
    pub fn report(&mut self, actor: &str, run: Option<&str>, app: &str, title: Option<&str>, presence: Option<&str>, now: &mut dyn FnMut() -> f64) -> Result<String, String> {
        if let Some(why) = refusal(run, app, presence) {
            return Err(why);
        }
        let (run, presence) = (run.unwrap_or_default(), presence.unwrap_or_default());
        let now = now();
        self.prune(now);
        let id = id_of(actor, app, run);
        if self.runs.get(&id).is_none() {
            if self.runs.iter().filter(|(_, x)| x.actor == actor).count() >= PER_ACTOR {
                return Err(format!("an actor reports at most {PER_ACTOR} runs at once"));
            }
            self.serials += 1;
            let r =
                Run { id: id.clone(), actor: actor.into(), app: app.into(), run: run.into(), created_at: now, updated_at: now, ended_at: None, title: None, serial: self.serials };
            self.runs.insert(&id, r);
        }
        let r = self.runs.get_mut(&id).expect("just added");
        r.updated_at = now;
        if let Some(t) = title.map(js_trim).filter(|t| !t.is_empty()) {
            r.title = Some(jsstr::head(&jsstr::collapse_space(t), 90));
        }
        r.ended_at = (presence == "ended").then_some(now);
        Ok(id)
    }

    /// Every run of an actor goes, its gateway token with it.
    pub fn forget(&mut self, actor: &str) {
        self.drop_where(|r| r.actor == actor);
    }

    fn prune(&mut self, now: f64) {
        self.drop_where(|r| older(now, r.updated_at, STALE_MS) || r.ended_at.is_some_and(|e| older(now, e, ENDED_KEEP_MS)));
    }

    fn drop_where(&mut self, go: impl Fn(&Run) -> bool) {
        let ids: Vec<String> = self.runs.iter().filter(|(_, r)| go(r)).map(|(k, _)| k.to_string()).collect();
        for id in ids {
            if let Some(r) = self.runs.remove(&id) {
                if self.gone.len() == GONE_KEPT {
                    self.gone.pop_front();
                }
                self.gone.push_back((r.serial, r.ended_at, r.updated_at));
            }
        }
    }

    pub fn sessions(&mut self, now: f64) -> Vec<Out> {
        self.prune(now);
        self.runs
            .iter()
            .map(|(_, r)| {
                let name = SESSION_APPS.iter().find(|(a, _)| *a == r.app).map_or("", |(_, n)| *n);
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("app", Out::str(&r.app)),
                    ("rawId", Out::str(&r.run)),
                    ("title", Out::str(&r.title.clone().unwrap_or_else(|| format!("{name}, {}", r.run)))),
                    ("preview", Out::null()),
                    ("cwd", Out::null()),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("archived", Out::bool(false)),
                    ("model", Out::null()),
                    ("branch", Out::null()),
                    ("parentId", Out::null()),
                    ("parentKind", Out::null()),
                    ("observed", Out::str("gateway")),
                    ("reportedBy", Out::str(&r.actor)),
                ])
            })
            .collect()
    }

    /// The run's status now: from the run while it's the same one, else
    /// from what it was when it went, else from what it was when the
    /// caller looked (`then`: endedAt, updatedAt).
    pub fn status_of(&self, id: &str, serial: u64, then: (Option<f64>, f64), now: f64) -> &'static str {
        if let Some(r) = self.runs.get(id).filter(|r| r.serial == serial) {
            return Run::status(r.ended_at, r.updated_at, now);
        }
        match self.gone.iter().rev().find(|g| g.0 == serial) {
            Some(&(_, ended_at, updated_at)) => Run::status(ended_at, updated_at, now),
            None => Run::status(then.0, then.1, now),
        }
    }

    pub fn runs_out(&self) -> Vec<Out> {
        self.runs
            .iter()
            .map(|(_, r)| {
                Out::Obj(vec![
                    ("id", Out::str(&r.id)),
                    ("actor", Out::str(&r.actor)),
                    ("app", Out::str(&r.app)),
                    ("run", Out::str(&r.run)),
                    ("createdAt", Out::Num(r.created_at)),
                    ("updatedAt", Out::Num(r.updated_at)),
                    ("endedAt", Out::opt_num(r.ended_at)),
                    ("title", Out::opt_str(r.title.as_deref())),
                ])
            })
            .collect()
    }
}

/// observedSessions' "one session per id, the first source to list it
/// wins": which of the ids, in order, are the first of their kind (as a
/// Set has them: an object or array is never one already seen).
pub fn firsts(ids: &[Option<Value>]) -> Vec<bool> {
    let mut seen = HashSet::new();
    ids.iter()
        .map(|id| {
            let key = match id {
                None => "u".to_string(),
                Some(Value::Null) => "n".to_string(),
                Some(Value::Bool(b)) => format!("b{b}"),
                Some(Value::Number(n)) => {
                    let f = n.as_f64().unwrap_or(f64::NAN);
                    format!("d{}", if f == 0.0 { 0.0 } else { f })
                }
                Some(Value::String(s)) => format!("s{s}"),
                Some(_) => return true,
            };
            seen.insert(key)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_are_reported_kept_and_let_go() {
        let mut r = ReportedRuns::new();
        let t = std::cell::Cell::new(1_000_000.0);
        let mut now = || t.get();
        let a = r.report("actor:space-runner", Some("sbx_one"), "claude", Some("  Running   the\ttests "), Some("working"), &mut now).unwrap();
        assert!(a.starts_with("claude:r") && a.len() == "claude:r".len() + 31);
        assert_eq!(a, id_of("actor:space-runner", "claude", "sbx_one"));
        assert_eq!(r.runs.get(&a).unwrap().title.as_deref(), Some("Running the tests"));
        assert_eq!(r.report("actor:x", Some("../x"), "claude", None, Some("working"), &mut now).unwrap_err(), "run is 1 to 64 letters, digits, dots, dashes or underscores");
        assert!(r.report("actor:x", Some("sbx"), "toaster", None, Some("working"), &mut now).unwrap_err().starts_with("app is the harness that runs, one of claude, codex, "));
        assert_eq!(r.report("actor:x", Some("sbx"), "claude", None, Some("gone"), &mut now).unwrap_err(), "presence is working, idle or ended");
        r.report("actor:space-runner", Some("sbx_one"), "claude", None, Some("ended"), &mut now).unwrap();
        let run = r.runs.get(&a).unwrap().clone();
        assert_eq!(r.status_of(&a, run.serial, (run.ended_at, run.updated_at), t.get()), "idle");
        t.set(t.get() + ENDED_KEEP_MS + 1.0);
        assert!(r.sessions(t.get()).is_empty(), "ended ten minutes ago: gone");
        for i in 0..PER_ACTOR {
            r.report("actor:busy", Some(&format!("run{i}")), "codex", None, Some("idle"), &mut now).unwrap();
        }
        assert!(r.report("actor:busy", Some("one-more"), "codex", None, Some("idle"), &mut now).unwrap_err().contains("at most 32"));
        let busy = r.runs.get(&id_of("actor:busy", "codex", "run1")).unwrap().clone();
        r.report("actor:busy", Some("run1"), "codex", None, Some("ended"), &mut now).unwrap();
        r.forget("actor:busy");
        assert!(r.runs.is_empty());
        assert_eq!(r.status_of(&busy.id, busy.serial, (None, busy.updated_at), t.get()), "idle", "as it was when it went");
        assert_eq!(firsts(&[Some(Value::from("a")), Some(Value::from("a")), None, None, Some(serde_json::json!({}))]), [true, false, true, false, true]);
    }
}
