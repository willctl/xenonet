//! What every building block has (core/blocks/lifecycle.js): a name and a
//! kind, start and stop, and its health, which /api/health lists and the
//! doctor's xenonet.blocks check reads.
//!
//! ```text
//!   state      idle       made, not started or used yet
//!              running    working, and the last thing it did went well
//!              degraded   working, but the last thing it did had a problem
//!              failed     not working: the last thing it did failed
//!              stopped    stopped
//!   since      when it came to be in that state
//!   lastOk     when it last did its work well
//!   lastError  { at, message } of the last problem; it stays after the
//!              block recovers, and `state` says whether it's current
//!   counters   numbers the kind of block keeps (runs, lines, errors, …)
//! ```
//!
//! A block used without being started (a source run on demand) counts as
//! running from its first success. A stopped block stays stopped whatever
//! it reports, until it starts again.
//!
//! Time comes in as an argument, so whoever runs the block keeps the clock.
//! Each change of state is kept until the runtime takes it (`take_changes`),
//! to tell whoever listens.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Running,
    Degraded,
    Failed,
    Stopped,
}

impl State {
    pub const ALL: [State; 5] = [State::Idle, State::Running, State::Degraded, State::Failed, State::Stopped];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Idle => "idle",
            State::Running => "running",
            State::Degraded => "degraded",
            State::Failed => "failed",
            State::Stopped => "stopped",
        }
    }

    pub fn parse(s: &str) -> Option<State> {
        State::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// A change of state: from, to, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub from: State,
    pub to: State,
    pub at: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ErrorAt {
    pub at: f64,
    pub message: String,
}

/// A block's counters, in the order they were first counted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counters(Vec<(String, f64)>);

impl Counters {
    pub fn of(pairs: &[(&str, f64)]) -> Self {
        Counters(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect())
    }
    pub fn get(&self, name: &str) -> Option<f64> {
        self.0.iter().find(|(k, _)| k == name).map(|(_, v)| *v)
    }
    pub fn set(&mut self, name: &str, v: f64) {
        match self.0.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = v,
            None => self.0.push((name.to_string(), v)),
        }
    }
    /// Adds `n`, counting a counter it hasn't seen from 0.
    pub fn add(&mut self, name: &str, n: f64) {
        let v = self.get(name).unwrap_or(0.0);
        self.set(name, v + n);
    }
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|(k, _)| k != name);
        self.0.len() != before
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, f64)> {
        self.0.iter().map(|(k, v)| (k.as_str(), *v))
    }
}

#[derive(Clone, Debug)]
pub struct Lifecycle {
    pub state: State,
    pub since: f64,
    pub last_ok: Option<f64>,
    pub last_error: Option<ErrorAt>,
    pub counters: Counters,
    pub started: bool,
    // Each start and stop moves this on, so a start can tell whether it
    // still stands (see current()).
    run: u64,
    changes: Vec<Change>,
}

impl Lifecycle {
    pub fn new(now: f64) -> Self {
        Lifecycle { state: State::Idle, since: now, last_ok: None, last_error: None, counters: Counters::of(&[("errors", 0.0)]), started: false, run: 0, changes: Vec::new() }
    }

    /// The changes of state since the runtime last took them, oldest first.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    pub fn set_state(&mut self, state: State, at: f64) {
        if state == self.state {
            return;
        }
        let from = self.state;
        self.state = state;
        self.since = at;
        self.changes.push(Change { from, to: state, at });
    }

    /// A start begins: the run that names it, or None when it's already
    /// started. A stopped block is idle again while it starts.
    pub fn begin_start(&mut self, at: f64) -> Option<u64> {
        if self.started {
            return None;
        }
        self.started = true;
        self.run += 1;
        if self.state == State::Stopped {
            self.set_state(State::Idle, at);
        }
        Some(self.run)
    }

    /// The kind's start threw: failed, if that start still stands.
    pub fn start_threw(&mut self, run: u64, message: &str, at: f64) {
        if self.current(run) {
            self.fail(message, at);
        }
    }

    /// The kind's start is done: running, if it still stands and nothing it
    /// reported says otherwise.
    pub fn start_done(&mut self, run: u64, at: f64) {
        if self.current(run) && self.state == State::Idle {
            self.set_state(State::Running, at);
        }
    }

    /// Whether the start that's `run` still stands: the block wasn't
    /// stopped, or stopped and started again, since.
    pub fn current(&self, run: u64) -> bool {
        self.started && run == self.run
    }

    /// A stop begins: no start stands any more. The kind's own stop runs
    /// next, in whoever runs the block, and then `set_state(Stopped)`.
    pub fn begin_stop(&mut self) {
        self.started = false;
        self.run += 1;
    }

    /// Stops it, for a kind with no stop of its own.
    pub fn stop(&mut self, at: f64) {
        self.begin_stop();
        self.set_state(State::Stopped, at);
    }

    /// The work went well: running again after a problem.
    pub fn ok(&mut self, at: f64) {
        self.last_ok = Some(at);
        if self.state != State::Running && self.state != State::Stopped {
            self.set_state(State::Running, at);
        }
    }

    /// A problem the block works around: degraded until the next success.
    pub fn problem(&mut self, message: &str, at: f64) {
        self.error(message, at);
        if self.state != State::Stopped {
            self.set_state(State::Degraded, at);
        }
    }

    /// A problem the block can't work around: failed until the next success.
    pub fn fail(&mut self, message: &str, at: f64) {
        self.error(message, at);
        if self.state != State::Stopped {
            self.set_state(State::Failed, at);
        }
    }

    /// A problem with one thing the block handled, which doesn't change how
    /// the block itself is: kept as the last error, and counted.
    pub fn error(&mut self, message: &str, at: f64) {
        self.last_error = Some(ErrorAt { at, message: message.to_string() });
        self.count("errors", 1.0);
    }

    pub fn count(&mut self, name: &str, n: f64) {
        self.counters.add(name, n);
    }
}

/// The blocks a part of XenoNet is made of, by name, for health reports:
/// the order they were added, each one's name unique, and the source of the
/// map a block reads for, so the doctor doesn't report a source's failure
/// twice. Whoever owns the blocks keeps them; this keeps their names.
#[derive(Clone, Debug, Default)]
pub struct Registry {
    names: Vec<String>,
    sources: Vec<(String, String)>,
}

impl Registry {
    /// Adds a block's name. `same` says whether the block already listed
    /// under the name is this one: adding it again only updates its source.
    pub fn add(&mut self, name: &str, same: bool, source: Option<&str>) -> Result<(), String> {
        let listed = self.names.iter().any(|n| n == name);
        if listed && !same {
            return Err(format!("there's already a block called {name}"));
        }
        if !listed {
            self.names.push(name.to_string());
        }
        if let Some(s) = source {
            self.sources.retain(|(n, _)| n != name);
            self.sources.push((name.to_string(), s.to_string()));
        }
        Ok(())
    }

    pub fn remove(&mut self, name: &str) {
        self.names.retain(|n| n != name);
        self.sources.retain(|(n, _)| n != name);
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn source(&self, name: &str) -> Option<&str> {
        self.sources.iter().find(|(n, _)| n == name).map(|(_, s)| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_their_changes() {
        let mut b = Lifecycle::new(0.0);
        let run = b.begin_start(1.0).unwrap();
        assert_eq!(b.begin_start(1.0), None, "starting twice starts once");
        b.start_done(run, 1.0);
        b.problem("a folder it couldn't read", 2.0);
        b.ok(3.0);
        b.fail("gone", 4.0);
        b.stop(5.0);
        b.ok(6.0);
        let seen: Vec<_> = b.take_changes().iter().map(|c| format!("{}→{}", c.from.as_str(), c.to.as_str())).collect();
        assert_eq!(seen, ["idle→running", "running→degraded", "degraded→running", "running→failed", "failed→stopped"]);
        assert_eq!((b.state, b.counters.get("errors")), (State::Stopped, Some(2.0)));
    }

    #[test]
    fn a_start_overtaken_by_a_stop_says_nothing() {
        let mut b = Lifecycle::new(0.0);
        let first = b.begin_start(0.0).unwrap();
        b.stop(1.0);
        let second = b.begin_start(2.0).unwrap();
        b.start_threw(first, "couldn't start", 3.0);
        assert_eq!((b.state, b.last_error.is_none()), (State::Idle, true));
        b.start_done(second, 4.0);
        assert_eq!(b.state, State::Running);
    }

    #[test]
    fn names_are_unique() {
        let mut r = Registry::default();
        r.add("a", false, None).unwrap();
        assert!(r.add("a", false, None).is_err());
        r.add("a", true, Some("claude")).unwrap();
        assert_eq!((r.names().len(), r.source("a")), (1, Some("claude")));
    }
}
