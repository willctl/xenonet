//! What a remote actor's reads say of it (docs/design/cloud-assistants.md,
//! section 3), put to the actors' registry the way its other sources are,
//! without taking their place: core/actors/remote-presence.js.
//!
//!   - a read that went out says the actor was at work then: its
//!     connection's own instance (remote-<connection>) says working, for a
//!     read's lease (REMOTE_LEASE_MS), then it's out of sight (unknown),
//!     never offline. An observer that sees it at work, or a registration
//!     that reports it, outranks a lapsed lease, as the most present
//!     instance always wins;
//!   - a read of one project, or one session, ties the actor to it as
//!     watches, which the map draws as a beam;
//!   - that's all: no event of its own, and nothing it did elsewhere.
//!
//! Polling shouldn't flood the feed or the database: an instance's word is
//! taken at most every 15 seconds, and a watches target touched at most
//! once a minute. The map shows the reads themselves too: "onRead" hears
//! of them, at most one an actor a second, and keeps nothing.

use crate::registry::REMOTE_LEASE_MS;
use std::collections::HashMap;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::js_text;

const PRESENCE_MS: f64 = 15e3;
const TARGET_MS: f64 = 60e3;
const SHOWN_MS: f64 = 1000.0;
const REACH_MAX: usize = 8;

/// A connection's own instance of its actor.
pub fn remote_instance(connection_id: &Arg) -> String {
    let id = js_text(connection_id);
    let id = id.strip_prefix("cn_").unwrap_or(&id);
    format!("remote-{}", id.chars().take(12).collect::<String>())
}

/// Values as a Set keeps them: the first of each, in order.
fn unique(items: &[Arg]) -> Vec<Arg> {
    let mut out: Vec<Arg> = Vec::new();
    for v in items {
        let same = |a: &Arg| match (a, v) {
            (Arg::Num(x), Arg::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
            (Arg::Obj(_) | Arg::List(_), _) => false,
            (a, v) => a == v,
        };
        if !out.iter().any(same) {
            out.push(v.clone());
        }
    }
    out
}

#[derive(Default)]
pub struct RemotePresence {
    said: HashMap<String, f64>,
    watched: HashMap<String, f64>,
    shown: HashMap<String, f64>,
}

impl RemotePresence {
    /// A read through `connection_id` went out for `actor_id`, about
    /// `targets` (node ids on the map, or none), having read `reach` (the
    /// places it covered, for the map's light only). "actors" is the
    /// registry, and "onRead" (when `on_read`) hears of the read.
    pub fn read(&mut self, c: &mut dyn Calls, actor_id: &Arg, connection_id: &Arg, targets: &[Arg], reach: &[Arg], at: f64, on_read: bool) -> Result<()> {
        if !c.call("actors", "has", vec![actor_id.clone()])?.truthy() {
            return Ok(());
        }
        let actor = js_text(actor_id);
        if on_read && at - self.shown.get(&actor).copied().unwrap_or(f64::NEG_INFINITY) >= SHOWN_MS {
            self.shown.insert(actor.clone(), at);
            let mut reach = unique(reach);
            reach.truncate(REACH_MAX);
            let read =
                Arg::Obj(vec![("actor".into(), actor_id.clone()), ("connection".into(), connection_id.clone()), ("reach".into(), Arg::List(reach)), ("at".into(), Arg::num(at))]);
            let _ = c.call("", "onRead", vec![read]);
        }
        let key = format!("{actor}|{}", js_text(connection_id));
        if at - self.said.get(&key).copied().unwrap_or(f64::NEG_INFINITY) >= PRESENCE_MS {
            self.said.insert(key, at);
            let report = Arg::Obj(vec![
                ("instance".into(), Arg::text(remote_instance(connection_id))),
                ("presence".into(), Arg::text("working")),
                ("heartbeatMs".into(), Arg::Null),
                ("leaseMs".into(), Arg::num(REMOTE_LEASE_MS)),
                ("via".into(), Arg::text("remote")),
                ("at".into(), Arg::num(at)),
            ]);
            c.call("actors", "report", vec![actor_id.clone(), report])?;
        }
        for target in targets.iter().take(4) {
            let k = format!("{actor}|{}", js_text(target));
            if at - self.watched.get(&k).copied().unwrap_or(f64::NEG_INFINITY) < TARGET_MS {
                continue;
            }
            self.watched.insert(k, at);
            let _ = c.call("actors", "target", vec![actor_id.clone(), target.clone(), Arg::text("watches"), Arg::num(at)]);
        }
        if self.said.len() > 1024 {
            self.said.clear();
        }
        if self.shown.len() > 1024 {
            self.shown.clear();
        }
        if self.watched.len() > 4096 {
            self.watched.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connections_instance_is_named_from_its_id() {
        assert_eq!(remote_instance(&Arg::text("cn_0123456789abcdef")), "remote-0123456789ab");
        assert_eq!(remote_instance(&Arg::text("x")), "remote-x");
    }
}
