//! core/blocks/gatherer.js's runtime: a snapshot from a source, when it's
//! due, with what it served and its last run kept here; xenonet-blocks'
//! gatherer keeps the schedule and decides. The gather itself is the
//! caller's (it awaits a listing, git, a fetch): `begin` says whether to
//! gather, and `done` or `failed` says how it went.

use crate::blocks::{health_of, Block};
use crate::util::{lock, now_ms, num};
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_blocks::gatherer::{Backoff, Gatherer};

/// The last gather: { value, at, ms, ok, error, stamp }.
#[derive(Clone, Debug)]
pub struct Last {
    pub value: Value,
    pub at: f64,
    pub ms: f64,
    pub ok: bool,
    pub error: Option<String>,
    pub stamp: Option<String>,
}

pub struct State {
    pub core: Gatherer,
    pub value: Value,
    pub last: Option<Last>,
    fallback: Value,
    retry: bool,
}

impl State {
    pub fn good(&self) -> bool {
        self.core.good
    }

    /// Whether a run at `now` would gather, with the source's stamp now.
    pub fn due(&self, now: f64, stamp: Option<&str>) -> bool {
        let same = self.last.as_ref().is_some_and(|l| self.core.stamped && l.stamp.as_deref() == stamp);
        self.core.due(now, same)
    }

    pub fn done(&mut self, value: Value, at: f64, ms: f64, stamp: Option<String>, wait: Option<f64>) -> Value {
        self.value = value.clone();
        self.last = Some(Last { value: value.clone(), at, ms, ok: true, error: None, stamp });
        self.core.done(at, wait);
        value
    }

    pub fn failed(&mut self, error: &str, at: f64, ms: f64, stamp: Option<String>) -> Value {
        let kept = self.core.failed(at, error);
        if !kept {
            self.value = self.fallback.clone();
        }
        let stamp = if self.retry { self.last.as_ref().and_then(|l| l.stamp.clone()) } else { stamp };
        self.last = Some(Last { value: self.value.clone(), at, ms, ok: false, error: Some(error.into()), stamp });
        self.value.clone()
    }

    pub fn forget(&mut self) {
        self.last = None;
        self.value = self.fallback.clone();
        self.core.forget();
    }

    pub fn runs(&self) -> f64 {
        self.core.life.counters.get("runs").unwrap_or(0.0)
    }
}

/// How a gatherer is made: core/blocks/gatherer.js's options.
#[derive(Default)]
pub struct Options {
    pub every_ms: Option<f64>,
    pub next_in: bool,
    pub backoff: Option<Backoff>,
    pub stamped: bool,
    pub retry: bool,
    pub keep: bool,
    pub fallback: Value,
}

pub struct Gather {
    pub name: String,
    kind: &'static str,
    pub state: Mutex<State>,
}

impl Gather {
    pub fn new(name: &str, o: Options) -> Gather {
        let core = Gatherer::new(name, o.every_ms, o.next_in, o.backoff, o.stamped, o.keep, now_ms());
        Gather { name: name.into(), kind: "gatherer", state: Mutex::new(State { core, value: o.fallback.clone(), last: None, fallback: o.fallback, retry: o.retry }) }
    }

    /// Whether to gather now (with `force`, always); a run that isn't due
    /// is counted, and serves what it has.
    pub fn begin(&self, force: bool, now: f64, stamp: Option<&str>) -> Result<(), Value> {
        let mut s = lock(&self.state);
        if !force && !s.due(now, stamp) {
            s.core.skipped();
            return Err(s.value.clone());
        }
        Ok(())
    }

    pub fn done(&self, value: Value, at: f64, ms: f64, stamp: Option<String>, wait: Option<f64>) -> Value {
        lock(&self.state).done(value, at, ms, stamp, wait)
    }

    pub fn failed(&self, error: &str, at: f64, ms: f64, stamp: Option<String>) -> Value {
        lock(&self.state).failed(error, at, ms, stamp)
    }

    /// A run of a source that gives `Ok(value)` or `Err(message)`, timed.
    pub async fn run<F, Fut>(&self, force: bool, stamp: Option<String>, gather: F) -> Value
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Value, String>>,
    {
        let at = now_ms();
        if let Err(v) = self.begin(force, at, stamp.as_deref()) {
            return v;
        }
        let t0 = now_ms();
        match gather().await {
            Ok(v) => self.done(v, at, now_ms() - t0, stamp, None),
            Err(e) => self.failed(&e, at, now_ms() - t0, stamp),
        }
    }

    /// The same for a source that answers at once.
    pub fn get(&self, force: bool, stamp: Option<String>, gather: impl FnOnce() -> Result<Value, String>) -> Value {
        let at = now_ms();
        if let Err(v) = self.begin(force, at, stamp.as_deref()) {
            return v;
        }
        let t0 = now_ms();
        match gather() {
            Ok(v) => self.done(v, at, now_ms() - t0, stamp, None),
            Err(e) => self.failed(&e, at, now_ms() - t0, stamp),
        }
    }

    pub fn value(&self) -> Value {
        lock(&self.state).value.clone()
    }

    pub fn last(&self) -> Option<Last> {
        lock(&self.state).last.clone()
    }

    /// `{ ok, ms, error }` of the last run, as a source of the map reads it.
    pub fn last_json(&self) -> Value {
        match self.last() {
            Some(l) => json!({ "ok": l.ok, "ms": num(l.ms), "error": l.error }),
            None => json!({ "ok": false, "ms": null, "error": null }),
        }
    }
}

impl Block for Gather {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn health(&self) -> Value {
        health_of(&self.name, self.kind, &lock(&self.state).core.life, &[], Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_it_had_when_asked_to() {
        let g = Gather::new("t", Options { keep: true, fallback: json!([]), ..Default::default() });
        assert_eq!(g.get(false, None, || Ok(json!([1]))), json!([1]));
        assert_eq!(g.get(false, None, || Err("down".into())), json!([1]), "kept");
        assert_eq!(g.health()["state"], "degraded");
        let h = Gather::new("u", Options { fallback: json!([]), ..Default::default() });
        h.get(false, None, || Ok(json!([1])));
        assert_eq!(h.get(false, None, || Err("down".into())), json!([]), "fell back");
        assert_eq!(h.health()["state"], "failed");
        let s = Gather::new("s", Options { stamped: true, ..Default::default() });
        s.get(false, Some("1".into()), || Ok(json!("a")));
        assert_eq!(s.get(false, Some("1".into()), || Ok(json!("b"))), json!("a"), "same stamp: not due");
        assert_eq!(s.get(false, Some("2".into()), || Ok(json!("b"))), json!("b"));
    }
}
