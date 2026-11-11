//! What a machine keeps of its network and its person between starts: the
//! network (this machine's id in it, the gateway it pinned), the roster's
//! entries, how the link last stood and where it stands in the gateway's
//! stream, the services published to it, its own feed (where it writes,
//! the kept entries it wrote and how far each member acked them) and how
//! it stands with the other members' feeds, the people's chains, the
//! presence blocks taken, and the token roots' certificates. The keys stay
//! files in the network's folder (`files`).
//!
//! The network crate doesn't say where these go: the composition root
//! keeps them in the store (xenonet-service's `compose::net`, over
//! xenonet-store) through this trait, and a test, or a node
//! with nothing to keep them in, uses `Memory`.

use crate::files::NetFile;
use crate::share::Sharing;
use crate::{roster, R};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use xenonet_log::feed::{FeedEntry, Fork, Link, Pos};
use xenonet_log::person;
use xenonet_os::sync::Locked;

/// A presence block taken: the device that signed it, its nonce, and when.
pub type Seen = (String, [u8; 16], i64);

/// A person whose chain this machine holds: their id, whether they're this
/// machine's own person, and the chain's entries from entry 0.
pub type Person = (String, bool, Vec<person::Entry>);

/// Where this machine writes its feed in a network: its incarnation, the
/// kept and loose entries written in it, the incarnation's link and the
/// last kept entry it wrote (`Writer::resume`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriterAt {
    pub inc: u64,
    pub kept: u64,
    pub loose: u64,
    /// What the incarnation's first kept entry names.
    pub link: Link,
    /// The last kept entry written, the link until the first.
    pub head: Link,
}

pub trait Persist: Send + Sync {
    /// The network this machine is in, if it's in one.
    fn network(&self) -> R<Option<NetFile>>;
    /// The network, made or changed.
    fn put_network(&self, n: &NetFile) -> R<()>;
    /// A network's roster: its entries from entry 0.
    fn roster(&self, network: &str) -> R<Vec<roster::Entry>>;
    /// Keeps the roster's entries not kept yet.
    fn put_roster(&self, network: &str, entries: &[roster::Entry]) -> R<()>;
    /// How the network's link last stood.
    fn link(&self, network: &str) -> R<Option<Value>>;
    fn put_link(&self, network: &str, status: &Value) -> R<()>;
    /// The services this machine published to the network, in order.
    fn services(&self, network: &str) -> R<Vec<Value>>;
    fn put_services(&self, network: &str, services: &[Value]) -> R<()>;
    /// A network left: everything kept of it goes.
    fn forget(&self, network: &str) -> R<()>;
    /// The people whose chains this machine holds.
    fn persons(&self) -> R<Vec<Person>>;
    /// Keeps a person's chain; `own` makes them this machine's own person,
    /// in place of the one before.
    fn put_person(&self, id: &str, own: bool, entries: &[person::Entry]) -> R<()>;
    /// The other people's chains go; the machine's own person stays.
    fn forget_others(&self) -> R<()>;
    fn presence_seen(&self) -> R<Vec<Seen>>;
    /// The presence blocks taken, in place of what was.
    fn put_presence_seen(&self, seen: &[Seen]) -> R<()>;
    /// The token roots' certificates, in the order they were made.
    fn token_roots(&self) -> R<Vec<Value>>;
    fn put_token_roots(&self, certificates: &[Value]) -> R<()>;
    /// The gateway's epoch this machine last saw in the network, and the
    /// last seq it read in it.
    fn cursor(&self, network: &str) -> R<Option<(String, u64)>>;
    fn put_cursor(&self, network: &str, epoch: &str, seq: u64) -> R<()>;
    /// Where this machine writes its feed in the network, once it has begun.
    fn feed_writer(&self, network: &str) -> R<Option<WriterAt>>;
    /// Keeps where it writes, and the kept entry it just wrote: both or
    /// neither, on disk before it returns, so an incarnation and a number
    /// are never given twice.
    fn put_feed_writer(&self, network: &str, author: &str, at: WriterAt, kept: Option<&FeedEntry>) -> R<()>;
    /// The kept entries it wrote after `after`, in order, at most `limit`.
    fn feed_out_after(&self, network: &str, after: Pos, limit: usize) -> R<Vec<FeedEntry>>;
    /// The kept entries of this machine's feed in `network` from `from` on,
    /// in order, at most `limit`: what answers a Want, and what a peer's
    /// hello says it hasn't got.
    fn range(&self, network: &str, from: Pos, limit: usize) -> R<Vec<FeedEntry>> {
        self.feed_out_after(network, Pos::new(from.inc, from.seq.saturating_sub(1)), limit)
    }
    /// How far `member` said it holds this machine's feed.
    fn feed_ack(&self, network: &str, member: &str) -> R<Option<Pos>>;
    /// `member` holds it up to `at`; kept only when that's further.
    fn put_feed_ack(&self, network: &str, member: &str, at: Pos) -> R<()>;
    /// Where this machine stands with each author's feed: the last kept
    /// entry it applied.
    fn feed_heads(&self, network: &str) -> R<Vec<(String, Pos)>>;
    /// The authors whose feeds a fork froze here, with the fork.
    fn frozen_feeds(&self, network: &str) -> R<Vec<(String, Fork)>>;
    /// What the person shares into a network (`share`), once they said:
    /// None until then, when the network's default holds.
    fn sharing(&self, _network: &str) -> R<Option<Sharing>> {
        Ok(None)
    }
    /// Keeps what the person shares into a network. A store that keeps
    /// nothing of it refuses, so a choice is never lost silently.
    fn put_sharing(&self, _network: &str, _sharing: &Sharing) -> R<()> {
        Err(crate::NetError::new(501, "not-kept", "this machine doesn't keep what is shared into each network yet"))
    }
}

/// Everything kept in memory, gone with it: for tests, and the doctor's
/// checks of a folder with nothing kept.
#[derive(Default)]
pub struct Memory(Mutex<Kept>);

#[derive(Default, Clone)]
struct Kept {
    network: Option<NetFile>,
    rosters: BTreeMap<String, Vec<roster::Entry>>,
    links: BTreeMap<String, Value>,
    services: BTreeMap<String, Vec<Value>>,
    persons: BTreeMap<String, (bool, Vec<person::Entry>)>,
    seen: Vec<Seen>,
    roots: Vec<Value>,
    cursors: BTreeMap<String, (String, u64)>,
    writers: BTreeMap<String, WriterAt>,
    out: BTreeMap<String, Vec<FeedEntry>>,
    acks: BTreeMap<(String, String), Pos>,
    sharing: BTreeMap<String, Sharing>,
}

// The entries of `had` and those of `more` past them.
fn extend<E: Clone>(had: &mut Vec<E>, more: &[E]) {
    if more.len() > had.len() {
        had.extend_from_slice(&more[had.len()..]);
    }
}

impl Persist for Memory {
    fn network(&self) -> R<Option<NetFile>> {
        Ok(self.0.locked().network.clone())
    }
    fn put_network(&self, n: &NetFile) -> R<()> {
        self.0.locked().network = Some(n.clone());
        Ok(())
    }
    fn roster(&self, network: &str) -> R<Vec<roster::Entry>> {
        Ok(self.0.locked().rosters.get(network).cloned().unwrap_or_default())
    }
    fn put_roster(&self, network: &str, entries: &[roster::Entry]) -> R<()> {
        extend(self.0.locked().rosters.entry(network.to_string()).or_default(), entries);
        Ok(())
    }
    fn link(&self, network: &str) -> R<Option<Value>> {
        Ok(self.0.locked().links.get(network).cloned())
    }
    fn put_link(&self, network: &str, status: &Value) -> R<()> {
        let mut k = self.0.locked();
        if k.network.as_ref().is_some_and(|n| n.id == network) {
            k.links.insert(network.to_string(), status.clone());
        }
        Ok(())
    }
    fn services(&self, network: &str) -> R<Vec<Value>> {
        Ok(self.0.locked().services.get(network).cloned().unwrap_or_default())
    }
    fn put_services(&self, network: &str, services: &[Value]) -> R<()> {
        self.0.locked().services.insert(network.to_string(), services.to_vec());
        Ok(())
    }
    fn forget(&self, network: &str) -> R<()> {
        let mut k = self.0.locked();
        if k.network.as_ref().is_some_and(|n| n.id == network) {
            k.network = None;
        }
        k.rosters.remove(network);
        k.links.remove(network);
        k.services.remove(network);
        k.cursors.remove(network);
        k.writers.remove(network);
        k.out.remove(network);
        k.acks.retain(|(n, _), _| n != network);
        k.sharing.remove(network);
        Ok(())
    }
    fn persons(&self) -> R<Vec<Person>> {
        Ok(self.0.locked().persons.iter().map(|(id, (own, entries))| (id.clone(), *own, entries.clone())).collect())
    }
    fn put_person(&self, id: &str, own: bool, entries: &[person::Entry]) -> R<()> {
        let mut k = self.0.locked();
        if own {
            k.persons.retain(|other, (was_own, _)| !*was_own || other == id);
        }
        let held = k.persons.entry(id.to_string()).or_insert((own, Vec::new()));
        held.0 |= own;
        extend(&mut held.1, entries);
        Ok(())
    }
    fn forget_others(&self) -> R<()> {
        self.0.locked().persons.retain(|_, (own, _)| *own);
        Ok(())
    }
    fn presence_seen(&self) -> R<Vec<Seen>> {
        Ok(self.0.locked().seen.clone())
    }
    fn put_presence_seen(&self, seen: &[Seen]) -> R<()> {
        self.0.locked().seen = seen.to_vec();
        Ok(())
    }
    fn token_roots(&self) -> R<Vec<Value>> {
        Ok(self.0.locked().roots.clone())
    }
    fn put_token_roots(&self, certificates: &[Value]) -> R<()> {
        self.0.locked().roots = certificates.to_vec();
        Ok(())
    }
    fn cursor(&self, network: &str) -> R<Option<(String, u64)>> {
        Ok(self.0.locked().cursors.get(network).cloned())
    }
    fn put_cursor(&self, network: &str, epoch: &str, seq: u64) -> R<()> {
        self.0.locked().cursors.insert(network.to_string(), (epoch.to_string(), seq));
        Ok(())
    }
    fn feed_writer(&self, network: &str) -> R<Option<WriterAt>> {
        Ok(self.0.locked().writers.get(network).copied())
    }
    fn put_feed_writer(&self, network: &str, _author: &str, at: WriterAt, kept: Option<&FeedEntry>) -> R<()> {
        let mut k = self.0.locked();
        k.writers.insert(network.to_string(), at);
        if let Some(e) = kept {
            k.out.entry(network.to_string()).or_default().push(e.clone());
        }
        Ok(())
    }
    fn feed_out_after(&self, network: &str, after: Pos, limit: usize) -> R<Vec<FeedEntry>> {
        Ok(self.0.locked().out.get(network).map(|es| es.iter().filter(|e| e.pos() > after).take(limit).cloned().collect()).unwrap_or_default())
    }
    fn feed_ack(&self, network: &str, member: &str) -> R<Option<Pos>> {
        Ok(self.0.locked().acks.get(&(network.to_string(), member.to_string())).copied())
    }
    fn put_feed_ack(&self, network: &str, member: &str, at: Pos) -> R<()> {
        let mut k = self.0.locked();
        let had = k.acks.entry((network.to_string(), member.to_string())).or_insert(at);
        *had = (*had).max(at);
        Ok(())
    }
    // A machine with nothing kept applies nothing of others' feeds through
    // here: the engine's store does.
    fn feed_heads(&self, _network: &str) -> R<Vec<(String, Pos)>> {
        Ok(Vec::new())
    }
    fn frozen_feeds(&self, _network: &str) -> R<Vec<(String, Fork)>> {
        Ok(Vec::new())
    }
    fn sharing(&self, network: &str) -> R<Option<Sharing>> {
        Ok(self.0.locked().sharing.get(network).cloned())
    }
    fn put_sharing(&self, network: &str, sharing: &Sharing) -> R<()> {
        self.0.locked().sharing.insert(network.to_string(), sharing.clone());
        Ok(())
    }
}
