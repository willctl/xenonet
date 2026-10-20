//! Claude Code channels on XenoNet's side: core/delivery/channel.js. The
//! xenonet MCP server in a Claude Code session started with XenoNet as a
//! channel holds a long poll here; a message for that session answers the
//! poll, and the server pushes it into the live session. What's kept (when
//! each session last polled, what waits for a poll) and every decision are
//! here; the polls themselves, promises with their timers, are the twin's.

use crate::sql::Arg;
use std::collections::HashMap;

/// A poll that ended this recently still counts: the next one is on its way.
pub const LIVE_MS: f64 = 5000.0;

#[derive(Default)]
pub struct Channels {
    last_poll: HashMap<String, f64>,
    queue: Vec<(String, Vec<Arg>)>,
}

/// Where a pushed item went.
#[derive(Debug, PartialEq)]
pub enum Push {
    /// No channel is live.
    NotLive,
    /// To the poll that waits.
    Waiter,
    /// Kept for the next poll.
    Queued,
}

impl Channels {
    /// Whether the session's channel is live: a poll waits, or one ended
    /// within LIVE_MS. `now` is read only when no poll waits.
    pub fn live(&self, agent: &str, waiting: bool, now: &mut dyn FnMut() -> f64) -> bool {
        waiting || now() - self.last_poll.get(agent).copied().unwrap_or(f64::NEG_INFINITY) < LIVE_MS
    }

    /// A poll begins: what waited for it, taken, or None to wait.
    pub fn poll(&mut self, agent: &str, now: f64) -> Option<Vec<Arg>> {
        self.last_poll.insert(agent.to_string(), now);
        let i = self.queue.iter().position(|(k, items)| k == agent && !items.is_empty())?;
        Some(self.queue.remove(i).1)
    }

    /// A poll ended.
    pub fn polled(&mut self, agent: &str, now: f64) {
        self.last_poll.insert(agent.to_string(), now);
    }

    /// How long a poll may wait: 0 to a minute.
    pub fn wait_ms(wait: f64) -> f64 {
        if wait.is_nan() {
            return f64::NAN;
        }
        wait.clamp(0.0, 60e3)
    }

    /// Hands an item to the session's poll, or keeps it for the next one.
    pub fn push(&mut self, agent: &str, item: Arg, waiting: bool, now: &mut dyn FnMut() -> f64) -> Push {
        if !self.live(agent, waiting, now) {
            return Push::NotLive;
        }
        if waiting {
            return Push::Waiter;
        }
        match self.queue.iter_mut().find(|(k, _)| k == agent) {
            Some((_, items)) => items.push(item),
            None => self.queue.push((agent.to_string(), vec![item])),
        }
        Push::Queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_push_waits_for_the_next_poll_while_the_channel_is_live() {
        let mut c = Channels::default();
        assert_eq!(c.push("claude:a", Arg::num(1.0), false, &mut || 0.0), Push::NotLive);
        assert_eq!(c.poll("claude:a", 0.0), None);
        c.polled("claude:a", 100.0);
        assert_eq!(c.push("claude:a", Arg::num(1.0), false, &mut || 200.0), Push::Queued);
        assert_eq!(c.poll("claude:a", 300.0), Some(vec![Arg::num(1.0)]));
        assert_eq!(c.push("claude:a", Arg::num(2.0), false, &mut || 9000.0), Push::NotLive);
    }
}
