//! What a remote actor's reads say of it (docs/design/cloud-assistants.md,
//! section 3), put to the actors' registry the way its other sources are,
//! without taking their place.
//!
//!   - a read that went out says the actor was at work then: its
//!     connection's own instance (`remote-<connection>`) says working, for a
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
//! once a minute. The map shows the reads themselves too: `Host::on_read`
//! hears of them, at most one an actor a second, and keeps nothing.

use crate::registry::REMOTE_LEASE_MS;
use crate::store;
use serde_json::{json, Value};
use std::collections::HashMap;
use xenonet_base::json::number;

const PRESENCE_MS: f64 = 15e3;
const TARGET_MS: f64 = 60e3;
const SHOWN_MS: f64 = 1000.0;
const REACH_MAX: usize = 8;

/// What a read asks of the world: the actors' registry, and whoever shows
/// the reads.
pub trait Host {
    fn has(&mut self, id: &str) -> bool;
    /// One of an actor's instances says it's there (the registry's report).
    fn report(&mut self, id: &str, report: &Value) -> store::Result<()>;
    /// Ties an actor to a node of the map (the registry's target).
    fn target(&mut self, id: &str, node: &str, relation: &str, at: f64) -> store::Result<()>;
    /// A read went out: { actor, connection, reach, at }.
    fn on_read(&mut self, read: Value);
}

/// A connection's own instance of its actor.
pub fn remote_instance(connection_id: &str) -> String {
    let id = connection_id.strip_prefix("cn_").unwrap_or(connection_id);
    format!("remote-{}", id.chars().take(12).collect::<String>())
}

/// Values as a Set keeps them: the first of each, in order (numbers by
/// value; an object or a list is never another's equal).
fn unique(items: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for v in items {
        let same = |a: &Value| match (a, v) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Object(_) | Value::Array(_), _) => false,
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
    /// places it covered, for the map's light only). With `on_read`, the
    /// host hears of the read.
    #[expect(clippy::too_many_arguments, reason = "a read and what it was told, one argument each")]
    pub fn read(&mut self, host: &mut dyn Host, actor_id: &str, connection_id: &str, targets: &[Value], reach: &[Value], at: f64, on_read: bool) -> store::Result<()> {
        if !host.has(actor_id) {
            return Ok(());
        }
        let actor = actor_id.to_string();
        if on_read && at - self.shown.get(&actor).copied().unwrap_or(f64::NEG_INFINITY) >= SHOWN_MS {
            self.shown.insert(actor.clone(), at);
            let mut reach = unique(reach);
            reach.truncate(REACH_MAX);
            host.on_read(json!({ "actor": actor_id, "connection": connection_id, "reach": reach, "at": number(at) }));
        }
        let key = format!("{actor}|{connection_id}");
        if at - self.said.get(&key).copied().unwrap_or(f64::NEG_INFINITY) >= PRESENCE_MS {
            self.said.insert(key, at);
            let report = json!({
                "instance": remote_instance(connection_id),
                "presence": "working",
                "heartbeatMs": null,
                "leaseMs": number(REMOTE_LEASE_MS),
                "via": "remote",
                "at": number(at),
            });
            host.report(actor_id, &report)?;
        }
        for target in targets.iter().take(4) {
            let node = xenonet_base::text::js_string(target);
            let k = format!("{actor}|{node}");
            if at - self.watched.get(&k).copied().unwrap_or(f64::NEG_INFINITY) < TARGET_MS {
                continue;
            }
            self.watched.insert(k, at);
            let _ = host.target(actor_id, &node, "watches", at);
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
        assert_eq!(remote_instance("cn_0123456789abcdef"), "remote-0123456789ab");
        assert_eq!(remote_instance("x"), "remote-x");
    }
}
