//! What a machine keeps of its network and its person between starts: the
//! network (this machine's id in it, the gateway it pinned), the roster's
//! entries, how the link last stood, the services published to it, the
//! people's chains, the presence blocks taken, and the token roots'
//! certificates. The keys stay files in the network's folder (`files`).
//!
//! The network crate doesn't say where these go: the runtime keeps them
//! in the store (xenonet-store) through this trait, and a test, or a node
//! with nothing to keep them in, uses `Memory`.

use crate::files::NetFile;
use crate::{roster, R};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use xenonet_log::person;
use xenonet_os::sync::Locked;

/// A presence block taken: the device that signed it, its nonce, and when.
pub type Seen = (String, [u8; 16], i64);

/// A person whose chain this machine holds: their id, whether they're this
/// machine's own person, and the chain's entries from entry 0.
pub type Person = (String, bool, Vec<person::Entry>);

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
}
