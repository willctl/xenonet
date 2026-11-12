//! Presence on link/2 (docs/design/network.md, the transport): which
//! agents a machine has, and where the person is, pushed to the members it
//! is connected to, only when it changes.
//!
//! - **A snapshot on connect.** When a link/2 connection comes up, each
//!   side sends the other its whole presence (`Own::snapshot`), shared down
//!   to what it shares into the networks both are in.
//! - **Versioned deltas, only on change.** The engine says what this
//!   machine's presence is (`network/presence/v1`, a full set or a change);
//!   `Own::take` keeps the whole of it and says what changed since, as a
//!   delta at a new version, or nothing. Nothing goes while nothing
//!   changes: no tick, no renew. A peer that has the version before gets
//!   the delta; any other gets the snapshot (`Sent`).
//! - **Last writer wins.** Versions only go up (`max(last + 1, now)`, as
//!   the engine's counter), across restarts too, and a receiver takes a
//!   body only past the version it holds, as the engine's `presence_in`
//!   does.
//! - **Valid while connected.** A machine's presence holds while its
//!   link/2 connection is up (QUIC's keep-alives say it is), and a peer
//!   drops it 30 s after the connection goes (`Theirs`), unless it comes
//!   back first. Presence is never stored and never relayed: a live
//!   envelope is taken only from its author's own connection.
//! - **Connections kept.** A machine keeps a connection to every member of
//!   the networks it is in, up to 50 machines (`keep`); past that, the
//!   window hosts and mailboxes first.

use crate::dial::PeerId;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

/// How long a peer's presence holds after its connection went.
pub const GONE_MS: u64 = 30_000;
/// The machines a machine keeps connections between, itself included.
pub const KEEP_UP_TO: usize = 50;
pub const SCHEMA: &str = "network/presence/v1";

/// The keys of a presence body that aren't fields of the presence itself.
const FRAME: [&str; 5] = ["schema", "counter", "full", "agents", "removed"];

fn id_of(a: &Value) -> Option<String> {
    a.get("id").and_then(Value::as_str).map(str::to_string)
}

/// This machine's presence, as the engine said it last: every agent by
/// id, the fields beside them (where the person is, the roster's head,
/// who may start sessions here, the address record), and its version.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Own {
    version: u64,
    agents: BTreeMap<String, Value>,
    fields: Map<String, Value>,
}

impl Own {
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Takes a body the engine sent: a full set replaces what there was, a
    /// change goes on top of it. What changed, as a change body at the new
    /// version; None when nothing did.
    pub fn take(&mut self, body: &Value) -> Option<Value> {
        let full = body.get("full").and_then(Value::as_bool).unwrap_or(false);
        let listed = body.get("agents").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut agents = if full { BTreeMap::new() } else { self.agents.clone() };
        for a in listed {
            if let Some(id) = id_of(&a) {
                agents.insert(id, a);
            }
        }
        if !full {
            for r in body.get("removed").and_then(Value::as_array).into_iter().flatten() {
                if let Some(id) = r.as_str() {
                    agents.remove(id);
                }
            }
        }
        let fields: Map<String, Value> =
            body.as_object().map(|o| o.iter().filter(|(k, _)| !FRAME.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
        let changed: Vec<Value> = agents.iter().filter(|(id, a)| self.agents.get(*id) != Some(*a)).map(|(_, a)| a.clone()).collect();
        let gone: Vec<Value> = self.agents.keys().filter(|id| !agents.contains_key(*id)).map(|id| json!(id)).collect();
        if changed.is_empty() && gone.is_empty() && fields == self.fields && self.version > 0 {
            return None;
        }
        let counter = body.get("counter").and_then(Value::as_f64).map_or(0, |c| c.max(0.0) as u64);
        self.version = (self.version + 1).max(counter);
        self.agents = agents;
        self.fields = fields;
        Some(self.body(false, changed, gone))
    }

    fn body(&self, full: bool, agents: Vec<Value>, removed: Vec<Value>) -> Value {
        let mut m = self.fields.clone();
        m.insert("schema".into(), json!(SCHEMA));
        m.insert("counter".into(), json!(self.version));
        m.insert("full".into(), json!(full));
        m.insert("agents".into(), Value::Array(agents));
        m.insert("removed".into(), Value::Array(removed));
        Value::Object(m)
    }

    /// The whole presence, at its version.
    pub fn snapshot(&self) -> Value {
        self.body(true, self.agents.values().cloned().collect(), Vec::new())
    }

    /// Whether the engine has said anything yet.
    pub fn is_empty(&self) -> bool {
        self.version == 0
    }
}

/// Which version of this machine's presence each connected peer has.
#[derive(Debug, Default)]
pub struct Sent {
    by: HashMap<PeerId, u64>,
}

impl Sent {
    /// A connection to `peer` came up: it gets the snapshot.
    pub fn connected(&mut self, peer: PeerId, own: &Own) -> Value {
        self.by.insert(peer, own.version());
        own.snapshot()
    }

    /// The presence moved from `before` to `own`'s version with `delta`:
    /// what goes to `peer`, the delta when it had `before`, else the
    /// snapshot.
    pub fn next(&mut self, peer: PeerId, own: &Own, before: u64, delta: &Value) -> Value {
        let had = self.by.insert(peer, own.version());
        if had == Some(before) {
            delta.clone()
        } else {
            own.snapshot()
        }
    }

    pub fn gone(&mut self, peer: PeerId) {
        self.by.remove(&peer);
    }
}

#[derive(Debug, Clone)]
struct Their {
    host: String,
    network: String,
    counter: u64,
    down_since: Option<u64>,
}

/// The peers' presence as this machine took it: the last version, and
/// when the peer's connection went, so it is dropped 30 s later.
#[derive(Debug, Default)]
pub struct Theirs {
    by: HashMap<PeerId, Their>,
}

/// A peer's presence that is no longer valid: the body that ends it.
#[derive(Debug, Clone, PartialEq)]
pub struct Ended {
    pub host: String,
    pub network: String,
    pub body: Value,
}

impl Theirs {
    /// A presence `peer` (`host` in `network`) sent, at `counter`.
    pub fn took(&mut self, peer: PeerId, host: &str, network: &str, counter: u64) {
        let t = self.by.entry(peer).or_insert_with(|| Their { host: host.into(), network: network.into(), counter, down_since: None });
        t.counter = t.counter.max(counter);
        t.down_since = None;
    }

    pub fn up(&mut self, peer: PeerId) {
        if let Some(t) = self.by.get_mut(&peer) {
            t.down_since = None;
        }
    }

    pub fn down(&mut self, peer: PeerId, now: u64) {
        if let Some(t) = self.by.get_mut(&peer) {
            t.down_since.get_or_insert(now);
        }
    }

    /// The peers whose connection has been gone `GONE_MS`: each with the
    /// body that ends its presence here, an empty full set past the last
    /// version taken.
    pub fn expired(&mut self, now: u64) -> Vec<Ended> {
        let gone: Vec<PeerId> = self.by.iter().filter(|(_, t)| t.down_since.is_some_and(|d| now >= d.saturating_add(GONE_MS))).map(|(p, _)| *p).collect();
        gone.into_iter()
            .filter_map(|p| self.by.remove(&p))
            .map(|t| Ended {
                body: json!({ "schema": SCHEMA, "counter": t.counter + 1, "full": true, "agents": [], "removed": [], "you": false, "gone": true }),
                host: t.host,
                network: t.network,
            })
            .collect()
    }

    /// When `expired` next has something to say, if ever.
    pub fn next_due(&self) -> Option<u64> {
        self.by.values().filter_map(|t| t.down_since.map(|d| d.saturating_add(GONE_MS))).min()
    }
}

/// A machine's presence on link/2: its own, what each peer has of it, and
/// the peers'.
#[derive(Debug, Default)]
pub struct Live {
    pub own: Own,
    pub sent: Sent,
    pub theirs: Theirs,
}

impl Live {
    /// The engine said what this machine's presence is: what goes to each
    /// of `peers` now, if anything changed.
    pub fn changed(&mut self, body: &Value, peers: &[PeerId]) -> Vec<(PeerId, Value)> {
        let before = self.own.version();
        let Some(delta) = self.own.take(body) else { return Vec::new() };
        peers.iter().map(|p| (*p, self.sent.next(*p, &self.own, before, &delta))).collect()
    }

    /// What `peers` may be shown changed (what's shared into a network):
    /// the snapshot again, at a new version, so each takes it as newer than
    /// what it holds. Nothing before the engine has said anything.
    pub fn again(&mut self, peers: &[PeerId]) -> Vec<(PeerId, Value)> {
        if self.own.is_empty() {
            return Vec::new();
        }
        self.own.version += 1;
        peers.iter().map(|p| (*p, self.sent.connected(*p, &self.own))).collect()
    }

    /// A connection to `peer` came up: the snapshot it gets, once the
    /// engine has said anything.
    pub fn connected(&mut self, peer: PeerId) -> Option<Value> {
        self.theirs.up(peer);
        (!self.own.is_empty()).then(|| self.sent.connected(peer, &self.own))
    }

    /// The connection to `peer` went at `now`.
    pub fn disconnected(&mut self, peer: PeerId, now: u64) {
        self.sent.gone(peer);
        self.theirs.down(peer, now);
    }
}

/// The peers to keep connections to, of `members` (each with whether it
/// goes first: a window host or a mailbox), this machine aside: all of
/// them up to `KEEP_UP_TO` machines; past that, the ones that go first,
/// then the rest in order.
pub fn keep(members: &[(PeerId, bool)], me: PeerId) -> Vec<PeerId> {
    let mut out: Vec<PeerId> = members.iter().filter(|(p, first)| *first && *p != me).map(|(p, _)| *p).collect();
    out.extend(members.iter().filter(|(p, first)| !*first && *p != me).map(|(p, _)| *p));
    out.dedup();
    out.truncate(KEEP_UP_TO - 1);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(i: usize, status: &str) -> Value {
        // An agent as the engine lists it in presence: about this size.
        json!({
            "id": format!("claude:{i:08x}-6b1e-4c2a-9f3d-2a7c5e9b{i:04x}"),
            "platform": "claude-code",
            "title": format!("Refactor the session index for project {i}"),
            "status": status,
            "projectKey": format!("prj_{i:032x}"),
            "cwd": format!("~/Projects/atlas-{i}"),
            "branch": "main",
            "model": "opus",
            "lastAt": 1_792_512_000_000u64 + i as u64,
            "waitsOnYou": false,
            "editing": [format!("src/module_{i}.rs")]
        })
    }

    fn engine(counter: u64, full: bool, agents: Vec<Value>, removed: Vec<&str>) -> Value {
        json!({ "schema": SCHEMA, "counter": counter, "full": full, "agents": agents, "removed": removed, "you": true, "roster": "head", "reach": {}, "can": ["start"] })
    }

    #[test]
    fn only_a_change_goes_and_a_peer_that_missed_one_gets_the_snapshot() {
        let mut own = Own::default();
        let d = own.take(&engine(10, true, vec![agent(1, "working"), agent(2, "idle")], vec![])).unwrap();
        assert_eq!(own.version(), 10);
        assert_eq!(d["agents"].as_array().unwrap().len(), 2);
        // The same again, full or as a change, is no change.
        assert_eq!(own.take(&engine(11, true, vec![agent(1, "working"), agent(2, "idle")], vec![])), None);
        assert_eq!(own.take(&engine(12, false, vec![agent(1, "working")], vec![])), None);
        let mut sent = Sent::default();
        let (p, q) = (PeerId([1; 32]), PeerId([2; 32]));
        assert_eq!(sent.connected(p, &own)["full"], true);
        // One agent changes, one goes.
        let before = own.version();
        let d = own.take(&engine(13, false, vec![agent(1, "waiting")], vec![&id_of(&agent(2, "")).unwrap()])).unwrap();
        assert_eq!((d["full"].clone(), d["agents"].as_array().unwrap().len(), d["removed"].as_array().unwrap().len()), (json!(false), 1, 1));
        assert_eq!(own.version(), 13);
        assert_eq!(sent.next(p, &own, before, &d), d, "p had the version before: the delta");
        assert_eq!(sent.next(q, &own, before, &d)["full"], true, "q never had it: the snapshot");
        // Versions only go up, whatever the counter says.
        own.take(&engine(5, false, vec![agent(3, "idle")], vec![])).unwrap();
        assert_eq!(own.version(), 14);
    }

    #[test]
    fn shown_again_it_comes_at_a_newer_version_than_the_peer_holds() {
        let mut live = Live::default();
        let p = PeerId([1; 32]);
        assert!(live.again(&[p]).is_empty(), "nothing before the engine said anything");
        live.changed(&engine(10, true, vec![agent(1, "working")], vec![]), &[p]);
        let had = live.sent.by[&p];
        let again = live.again(&[p]);
        assert_eq!(again.len(), 1);
        assert_eq!((again[0].1["full"].clone(), again[0].1["counter"].clone()), (json!(true), json!(had + 1)), "a receiver takes only what's past its version");
        assert_eq!(live.sent.by[&p], had + 1);
    }

    #[test]
    fn a_peers_presence_holds_while_connected_and_goes_thirty_seconds_after() {
        let mut t = Theirs::default();
        let p = PeerId([1; 32]);
        t.took(p, "desk", "net_x", 7);
        t.down(p, 1000);
        assert_eq!(t.next_due(), Some(1000 + GONE_MS));
        assert!(t.expired(1000 + GONE_MS - 1).is_empty(), "not yet");
        t.up(p);
        assert_eq!(t.next_due(), None, "back in time: nothing goes");
        t.down(p, 5000);
        let ended = t.expired(5000 + GONE_MS);
        assert_eq!(ended.len(), 1);
        assert_eq!((ended[0].host.as_str(), ended[0].body["counter"].clone(), ended[0].body["agents"].clone()), ("desk", json!(8), json!([])));
        assert!(t.expired(u64::MAX).is_empty(), "once");
    }

    #[test]
    fn connections_go_to_every_member_up_to_fifty_window_hosts_and_mailboxes_first() {
        let me = PeerId([0; 32]);
        let members: Vec<(PeerId, bool)> = (0..80u8).map(|i| (PeerId([i; 32]), i % 20 == 7)).collect();
        let k = keep(&members, me);
        assert_eq!(k.len(), 49);
        assert!(!k.contains(&me));
        assert_eq!(&k[..4], [PeerId([7; 32]), PeerId([27; 32]), PeerId([47; 32]), PeerId([67; 32])]);
        assert_eq!(keep(&members[..10], me).len(), 9, "all of them, below fifty");
    }

    /// The bench (phase 5's commit 12): 50 machines, 1,000 agents on this
    /// one, one idle minute and one change-heavy minute, in the bytes of
    /// the live envelopes this machine sends its 49 peers. The engine
    /// says what its presence is every 2 s; QUIC's keep-alives (a PING
    /// each 5 s each way when idle, `iroh`'s HEARTBEAT_INTERVAL) are
    /// counted apart, at 60 bytes on the wire a packet.
    #[test]
    fn bench_fifty_peers_a_thousand_agents_under_a_mebibyte_a_minute_idle() {
        const PEERS: usize = 49;
        const AGENTS: usize = 1000;
        let net = "net_00112233445566778899aabbccddeeff";
        let me = &xenonet_id::sign::Signer::public(&xenonet_id::sign::SeedSigner::from_seed(&[9; 32], xenonet_id::sign::KeyHome::File).unwrap()).id();
        let wire = |body: &Value| crate::envelope::Envelope::live(net, me, "presence", 1, body).unwrap().encode().len() + 4;
        let mut own = Own::default();
        let mut sent = Sent::default();
        let peers: Vec<PeerId> = (0..PEERS).map(|i| PeerId([i as u8; 32])).collect();
        let all = |status: &str| (0..AGENTS).map(|i| agent(i, status)).collect::<Vec<_>>();
        own.take(&engine(1, true, all("idle"), vec![])).unwrap();
        let connect: usize = peers.iter().map(|p| wire(&sent.connected(*p, &own))).sum();
        // An idle minute: the engine says the same thirty times.
        let mut idle = 0;
        for t in 0..30 {
            let before = own.version();
            if let Some(d) = own.take(&engine(2 + t, true, all("idle"), vec![])) {
                idle += peers.iter().map(|p| wire(&sent.next(*p, &own, before, &d))).sum::<usize>();
            }
        }
        let keepalive = PEERS * 2 * (60 / 5) * 60;
        // A change-heavy minute: every 2 s, 50 agents change status, so
        // each of the 1,000 changes once and half of them twice.
        let mut busy = 0;
        for t in 0..30u64 {
            let before = own.version();
            let changed: Vec<Value> = (0..50).map(|j| agent((t as usize * 50 + j) % AGENTS, if t % 2 == 0 { "working" } else { "waiting" })).collect();
            if let Some(d) = own.take(&engine(100 + t, false, changed, vec![])) {
                busy += peers.iter().map(|p| wire(&sent.next(*p, &own, before, &d))).sum::<usize>();
            }
        }
        let mib = |b: usize| b as f64 / f64::from(1 << 20);
        println!(
            "presence bench, {PEERS} peers, {AGENTS} agents: connect {:.2} MiB once; idle minute {} B of presence + {:.3} MiB of keep-alives = {:.3} MiB; change-heavy minute {:.2} MiB",
            mib(connect),
            idle,
            mib(keepalive),
            mib(idle + keepalive),
            mib(busy + keepalive)
        );
        assert_eq!(idle, 0, "nothing goes while nothing changes");
        assert!(idle + keepalive < 1 << 20);
    }
}
