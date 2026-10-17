//! A gatherer takes a snapshot from a source: a list of sessions from an
//! app, a file XenoNet reads, a reading from a web API, a command's output
//! (core/blocks/gatherer.js). It gathers when asked and when it's due, one
//! gather at a time, times each one, and after a failure either keeps
//! serving what it had or falls back.
//!
//! When it's due:
//!   - with a stamp, when the stamp (a file's mtime, say) differs from the
//!     one at the last gather;
//!   - otherwise once `every_ms` has passed since the last gather (always,
//!     without it), or the time the source asked for (`next_in`), or, after
//!     failures, the backoff's delay, which doubles with each one.
//!
//! A failure is a gather that throws. With `keep`, the gatherer serves the
//! last good value and is degraded; without it, or with nothing to keep, it
//! serves its fallback and is failed. Either way the next success makes it
//! running again.
//!
//! This keeps the schedule and decides; the values, the stamps and the
//! gather itself are whoever runs it.

use crate::lifecycle::Lifecycle;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backoff {
    pub base_ms: f64,
    pub factor: f64,
    pub max_ms: f64,
}

impl Backoff {
    /// A backoff as given: `factor` 2 and `max_ms` 32 times the base unless
    /// it says otherwise.
    pub fn of(base_ms: f64, factor: Option<f64>, max_ms: Option<f64>) -> Self {
        Backoff { base_ms, factor: factor.unwrap_or(2.0), max_ms: max_ms.unwrap_or(base_ms * 32.0) }
    }
}

pub struct Gatherer {
    pub name: String,
    pub every_ms: Option<f64>,
    pub next_in: bool,
    pub backoff: Option<Backoff>,
    pub stamped: bool,
    pub keep: bool,
    /// Whether it has gathered since it was made or forgot.
    pub gathered: bool,
    /// Whether what it serves came from a gather that worked.
    pub good: bool,
    pub next_at: f64,
    pub failures: u32,
    pub life: Lifecycle,
}

impl Gatherer {
    pub fn new(name: &str, every_ms: Option<f64>, next_in: bool, backoff: Option<Backoff>, stamped: bool, keep: bool, now: f64) -> Self {
        Gatherer { name: name.into(), every_ms, next_in, backoff, stamped, keep, gathered: false, good: false, next_at: 0.0, failures: 0, life: Lifecycle::new(now) }
    }

    fn cadenced(&self) -> bool {
        self.every_ms.is_some_and(|e| e != 0.0 && !e.is_nan()) || self.next_in || self.backoff.is_some()
    }

    /// Whether a run now would gather. `same_stamp`: whether the stamp now
    /// is the one at the last gather (for a stamped source). Without a
    /// cadence, a backoff or a source that says when, always: even if the
    /// system clock steps back.
    pub fn due(&self, now: f64, same_stamp: bool) -> bool {
        if !self.gathered {
            return true;
        }
        if self.stamped {
            return !same_stamp;
        }
        if !self.cadenced() {
            return true;
        }
        now >= self.next_at
    }

    /// A run that wasn't due: counted.
    pub fn skipped(&mut self) {
        self.life.count("skipped", 1.0);
    }

    /// A gather worked, at `at`; `wait` is when the source asked to be
    /// gathered again, if it did.
    pub fn done(&mut self, at: f64, wait: Option<f64>) {
        self.life.count("runs", 1.0);
        self.failures = 0;
        self.good = true;
        self.gathered = true;
        let wait = wait.filter(|w| w.is_finite() && *w > 0.0);
        self.next_at = at + wait.unwrap_or(self.every_ms.unwrap_or(0.0));
        self.life.ok(at);
    }

    /// A gather failed: whether the gatherer keeps serving the last good
    /// value (degraded), or falls back (failed).
    pub fn failed(&mut self, at: f64, message: &str) -> bool {
        self.life.count("runs", 1.0);
        self.failures += 1;
        self.gathered = true;
        let kept = self.keep && self.good;
        if !kept {
            self.good = false;
        }
        self.next_at = at
            + match self.backoff {
                Some(b) => b.max_ms.min(b.base_ms * b.factor.powf(f64::from(self.failures) - 1.0)),
                None => self.every_ms.unwrap_or(0.0),
            };
        if kept {
            self.life.problem(message, at);
        } else {
            self.life.fail(message, at);
        }
        kept
    }

    /// Forgets what it gathered, so the next run gathers again.
    pub fn forget(&mut self) {
        self.gathered = false;
        self.good = false;
        self.next_at = 0.0;
        self.failures = 0;
    }

    /// How long the cadence waits for the next run.
    pub fn next_wait(&self, now: f64) -> f64 {
        if self.gathered {
            (self.next_at - now).max(0.0)
        } else {
            self.every_ms.unwrap_or(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_and_keeps() {
        let mut g = Gatherer::new("t", Some(60e3), false, Some(Backoff::of(1000.0, None, None)), false, true, 0.0);
        assert!(g.due(0.0, false));
        g.done(0.0, None);
        assert!(!g.due(59e3, false) && g.due(60e3, false));
        assert!(g.failed(60e3, "down"), "kept: it had a good value");
        assert_eq!(g.next_at, 61e3);
        g.failed(61e3, "down");
        assert_eq!(g.next_at, 63e3, "doubled");
        for _ in 0..10 {
            g.failed(0.0, "down");
        }
        assert_eq!(g.next_at, 32e3, "at most 32 times the base");
        g.done(0.0, Some(5e3));
        assert_eq!(g.next_at, 5e3, "when the source asked");
    }
}
