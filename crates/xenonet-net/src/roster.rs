//! The roster (docs/design/network.md, section 3): the signed chain of a
//! network's hosts and their keys (`network/roster-entry/v1`). Every host
//! keeps the chain and checks it whole, and accepts only entries that
//! extend its own head, so a gateway can withhold an entry but never add or
//! change one.
//!
//! What each entry needs:
//!
//! | op | signed by |
//! |---|---|
//! | `found` | the founder, once, as entry 0 |
//! | `add` a host that isn't a window host | any window host |
//! | `add` a window host, `window`, `remove` a window host, `gateway` | the founder, or two different window hosts |
//! | `remove` a host that isn't a window host | any window host |
//! | `remove` the founder | nobody |
//!
//! And one more, for leaving: a host may sign its own `remove` (any host
//! but the founder), since taking yourself out harms no one.
//!
//! The founder counts as a window host. An entry's signers sign its
//! canonical JSON without the signers, after `xenonet-roster/1` and a
//! newline; `prev` is the base64url SHA-256 of the previous entry's
//! canonical JSON with its signers.
//!
//! Duplicates: an entry's identity is its op, host, keys, window choice and
//! gateway. One offered where the chain already has an identical entry at
//! its index is dropped quietly (two window hosts approving the same thing
//! at once); a different one there is a fork, and refused.
//!
//! An entry signed rightly but by too few is refused as `needs-approval`:
//! the gateway may keep it as a proposal, for the founder or a second
//! window host to sign too.

use crate::codec::{b64, hash_b64, is_network_id, unb64};
use crate::keys::{HostKey, PublicKeys};
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use xenonet_base::ids::is_host_id;

pub const ROSTER_TAG: &str = "xenonet-roster/1";
pub const MAX_SIGNERS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Found,
    Add,
    Remove,
    Window,
    Gateway,
}

impl Op {
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Found => "found",
            Op::Add => "add",
            Op::Remove => "remove",
            Op::Window => "window",
            Op::Gateway => "gateway",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        [Op::Found, Op::Add, Op::Remove, Op::Window, Op::Gateway].into_iter().find(|o| o.as_str() == s)
    }
}

/// The host an entry is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRef {
    pub id: String,
    pub keys: PublicKeys,
    pub window: bool,
}

/// For a `gateway` entry: where the network's gateway is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAt {
    pub address: String,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    pub host: String,
    pub sig: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub n: u64,
    pub prev: String,
    pub network: String,
    pub op: Op,
    pub host: HostRef,
    pub gateway: Option<GatewayAt>,
    pub at: i64,
    pub signers: Vec<Signer>,
}

impl Entry {
    fn unsigned(&self) -> Value {
        json!({
            "n": self.n,
            "prev": self.prev,
            "network": self.network,
            "op": self.op.as_str(),
            "host": {
                "id": self.host.id,
                "sign": self.host.keys.sign_b64(),
                "seal": self.host.keys.seal_b64(),
                "keyId": self.host.keys.key_id(),
                "window": self.host.window,
            },
            "gateway": self.gateway.as_ref().map(|g| json!({ "address": g.address, "version": g.version })),
            "at": self.at,
        })
    }

    /// The entry as the roster keeps it: its canonical JSON, keys in this
    /// order.
    pub fn to_json(&self) -> Value {
        let mut v = self.unsigned();
        v["signers"] = Value::Array(self.signers.iter().map(|s| json!({ "host": s.host, "sig": b64(&s.sig) })).collect());
        v
    }

    /// What each signer signs.
    pub fn signing_bytes(&self) -> Vec<u8> {
        format!("{ROSTER_TAG}\n{}", self.unsigned()).into_bytes()
    }

    /// What an entry says, apart from who signed it: what's approved, by
    /// its hash, so an approval signs exactly the entry the person saw.
    pub fn id(&self) -> String {
        hash_b64(&self.signing_bytes())
    }

    /// What the next entry's `prev` is.
    pub fn hash(&self) -> String {
        hash_b64(self.to_json().to_string().as_bytes())
    }

    /// Takes another copy's signatures too: the same entry, approved by more
    /// hosts. At most four.
    pub fn merge_signers(&mut self, other: &Entry) {
        for s in &other.signers {
            if self.signers.len() < MAX_SIGNERS && !self.signers.iter().any(|x| x.host == s.host) {
                self.signers.push(s.clone());
            }
        }
    }

    /// Adds `host`'s signature.
    pub fn sign_by(&mut self, host: &str, key: &HostKey) {
        let sig = key.sign(&self.signing_bytes());
        self.signers.retain(|s| s.host != host);
        self.signers.push(Signer { host: host.into(), sig });
    }

    /// Whether two entries say the same thing (their signers aside).
    pub fn same(&self, other: &Entry) -> bool {
        self.op == other.op && self.host == other.host && self.gateway == other.gateway && self.network == other.network
    }

    pub fn parse(v: &Value) -> R<Entry> {
        let bad = |what: &str| NetError::bad(format!("a roster entry's {what} isn't right"));
        let n = v.get("n").and_then(Value::as_u64).ok_or_else(|| bad("index"))?;
        let prev = v.get("prev").and_then(Value::as_str).filter(|p| crate::codec::is_b64(p)).ok_or_else(|| bad("prev"))?;
        let network = v.get("network").and_then(Value::as_str).filter(|n| is_network_id(n)).ok_or_else(|| bad("network"))?;
        let op = v.get("op").and_then(Value::as_str).and_then(Op::parse).ok_or_else(|| bad("op"))?;
        let h = v.get("host").ok_or_else(|| bad("host"))?;
        let id = h.get("id").and_then(Value::as_str).filter(|i| is_host_id(i)).ok_or_else(|| bad("host id"))?;
        let keys = PublicKeys::from_b64(h.get("sign").and_then(Value::as_str).unwrap_or(""), h.get("seal").and_then(Value::as_str).unwrap_or("")).ok_or_else(|| bad("keys"))?;
        if h.get("keyId").and_then(Value::as_str) != Some(keys.key_id().as_str()) {
            return Err(bad("key id"));
        }
        let window = h.get("window").and_then(Value::as_bool).ok_or_else(|| bad("window"))?;
        let gateway = match v.get("gateway") {
            None | Some(Value::Null) => None,
            Some(g) => Some(GatewayAt {
                address: g.get("address").and_then(Value::as_str).filter(|a| !a.is_empty() && a.len() <= 300).ok_or_else(|| bad("gateway address"))?.to_string(),
                version: g.get("version").and_then(Value::as_u64).ok_or_else(|| bad("gateway version"))?,
            }),
        };
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        let list = v.get("signers").and_then(Value::as_array).filter(|s| !s.is_empty() && s.len() <= MAX_SIGNERS).ok_or_else(|| bad("signers"))?;
        let mut signers = Vec::new();
        for s in list {
            let host = s.get("host").and_then(Value::as_str).filter(|h| is_host_id(h)).ok_or_else(|| bad("signer"))?;
            let sig = s.get("sig").and_then(Value::as_str).and_then(unb64).filter(|x| x.len() == 64).ok_or_else(|| bad("signature"))?;
            signers.push(Signer { host: host.into(), sig });
        }
        Ok(Entry { n, prev: prev.into(), network: network.into(), op, host: HostRef { id: id.into(), keys, window }, gateway, at, signers })
    }
}

/// A host in the network now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub keys: PublicKeys,
    pub window: bool,
    /// The entry that added it.
    pub since: u64,
}

/// A host that was removed: by which entry, whom, when, and the keys it
/// had, which still sign its requests until it hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub n: u64,
    pub by: Vec<String>,
    pub at: i64,
    pub keys: PublicKeys,
}

/// The network as the chain leaves it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub network: String,
    pub founder: String,
    pub members: BTreeMap<String, Member>,
    pub removed: BTreeMap<String, Removal>,
    /// The host whose service answers the network's routes.
    pub gateway: String,
    pub gateway_address: Option<String>,
    pub gateway_version: u64,
}

impl View {
    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.get(id)
    }

    /// The founder counts as a window host.
    pub fn is_window(&self, id: &str) -> bool {
        id == self.founder || self.members.get(id).is_some_and(|m| m.window)
    }

    pub fn gateway_member(&self) -> Option<&Member> {
        self.members.get(&self.gateway)
    }

    /// A removed host's last removal, unless it was added again since.
    pub fn removal(&self, id: &str) -> Option<&Removal> {
        if self.members.contains_key(id) {
            return None;
        }
        self.removed.get(id)
    }
}

fn refused(message: impl Into<String>) -> NetError {
    NetError::new(403, "roster-refused", message)
}

/// Signed rightly, but by too few: the entry may wait for the founder or a
/// second window host to sign it too.
pub fn needs_approval() -> NetError {
    NetError::new(403, "needs-approval", "that needs the founder, or two window hosts")
}

/// Whether `e` may follow `view` (None before entry 0): its signers, and
/// the rules for its op.
fn check(view: Option<&View>, prev: &str, n: u64, e: &Entry) -> R<()> {
    if e.n != n {
        return Err(NetError::new(409, "roster-gap", format!("the roster's next entry is {n}, not {}", e.n)));
    }
    if e.prev != prev {
        return Err(NetError::new(409, "roster-fork", "that entry doesn't follow the roster's head"));
    }
    let mut seen = Vec::new();
    for s in &e.signers {
        if seen.contains(&s.host.as_str()) {
            return Err(refused("an entry is signed by each host once"));
        }
        seen.push(s.host.as_str());
    }
    let msg = e.signing_bytes();
    let Some(view) = view else {
        // Entry 0: the founder's, signed by itself.
        if e.op != Op::Found {
            return Err(refused("a roster starts with its founder"));
        }
        if e.signers.len() != 1 || e.signers[0].host != e.host.id || !e.host.keys.verify(&msg, &e.signers[0].sig) {
            return Err(refused("the founder signs the first entry, alone"));
        }
        if !is_network_id(&e.network) {
            return Err(NetError::bad("that isn't a network id"));
        }
        return Ok(());
    };
    if e.network != view.network {
        return Err(refused("that entry is for another network"));
    }
    for s in &e.signers {
        let Some(m) = view.member(&s.host) else { return Err(refused(format!("{} isn't in the network", s.host))) };
        if !m.keys.verify(&msg, &s.sig) {
            return Err(refused(format!("{}'s signature doesn't check", s.host)));
        }
    }
    let founder = seen.contains(&view.founder.as_str());
    let windows = seen.iter().filter(|h| view.is_window(h)).count();
    let strong = founder || windows >= 2;
    let any_window = founder || windows >= 1;
    let current = view.member(&e.host.id);
    let same_keys = |m: &Member| m.keys == e.host.keys;
    match e.op {
        Op::Found => Err(refused("a network has one founder")),
        Op::Add => {
            if current.is_some() {
                return Err(NetError::new(409, "taken", format!("{} is in the network already", e.host.id)));
            }
            if view.members.values().any(|m| m.keys == e.host.keys) {
                return Err(NetError::new(409, "taken", "those keys are another host's"));
            }
            if e.gateway.is_some() {
                return Err(NetError::bad("only a gateway entry names a gateway"));
            }
            match (e.host.window, strong, any_window) {
                (true, false, _) => Err(needs_approval()),
                (false, _, false) => Err(refused("adding a host needs a window host")),
                _ => Ok(()),
            }
        }
        Op::Remove => {
            let Some(m) = current.filter(|m| same_keys(m)) else { return Err(NetError::new(404, "no-host", format!("{} isn't in the network with those keys", e.host.id))) };
            if m.id == view.founder {
                return Err(refused("the founder can't be removed: a network whose founder is lost is made again"));
            }
            if e.gateway.is_some() {
                return Err(NetError::bad("only a gateway entry names a gateway"));
            }
            if e.signers.len() == 1 && e.signers[0].host == m.id {
                return Ok(());
            }
            match (view.is_window(&m.id), strong, any_window) {
                (true, false, _) => Err(needs_approval()),
                (false, _, false) => Err(refused("removing a host needs a window host")),
                _ => Ok(()),
            }
        }
        Op::Window => {
            let Some(m) = current.filter(|m| same_keys(m)) else { return Err(NetError::new(404, "no-host", format!("{} isn't in the network with those keys", e.host.id))) };
            if m.window == e.host.window {
                return Err(NetError::new(409, "no-change", "that's the window choice it has"));
            }
            if e.gateway.is_some() {
                return Err(NetError::bad("only a gateway entry names a gateway"));
            }
            if strong {
                Ok(())
            } else {
                Err(needs_approval())
            }
        }
        Op::Gateway => {
            if current.is_none_or(|m| !same_keys(m)) {
                return Err(NetError::new(404, "no-host", format!("{} isn't in the network with those keys", e.host.id)));
            }
            let Some(g) = &e.gateway else { return Err(NetError::bad("a gateway entry names where the gateway is")) };
            if g.version <= view.gateway_version {
                return Err(NetError::new(409, "stale", "a gateway entry needs a higher version than the last"));
            }
            if strong {
                Ok(())
            } else {
                Err(needs_approval())
            }
        }
    }
}

/// `view` after `e`, which `check` let through.
fn apply(view: Option<View>, e: &Entry) -> View {
    let mut v = view.unwrap_or_default();
    match e.op {
        Op::Found => {
            v.network.clone_from(&e.network);
            v.founder.clone_from(&e.host.id);
            v.gateway.clone_from(&e.host.id);
            v.gateway_address = e.gateway.as_ref().map(|g| g.address.clone());
            v.gateway_version = e.gateway.as_ref().map_or(0, |g| g.version);
            v.members.insert(e.host.id.clone(), Member { id: e.host.id.clone(), keys: e.host.keys, window: true, since: e.n });
        }
        Op::Add => {
            v.removed.remove(&e.host.id);
            v.members.insert(e.host.id.clone(), Member { id: e.host.id.clone(), keys: e.host.keys, window: e.host.window, since: e.n });
        }
        Op::Remove => {
            v.members.remove(&e.host.id);
            v.removed.insert(e.host.id.clone(), Removal { n: e.n, by: e.signers.iter().map(|s| s.host.clone()).collect(), at: e.at, keys: e.host.keys });
        }
        Op::Window => {
            if let Some(m) = v.members.get_mut(&e.host.id) {
                m.window = e.host.window;
            }
        }
        Op::Gateway => {
            v.gateway.clone_from(&e.host.id);
            if let Some(g) = &e.gateway {
                v.gateway_address = Some(g.address.clone());
                v.gateway_version = g.version;
            }
        }
    }
    v
}

/// What happened to an entry offered to the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offered {
    Appended,
    /// The chain has it already.
    Duplicate,
}

#[derive(Debug, Clone, Default)]
pub struct Roster {
    entries: Vec<Entry>,
    view: Option<View>,
}

impl Roster {
    pub fn new() -> Roster {
        Roster::default()
    }

    /// A chain, checked whole from entry 0.
    pub fn from_entries(entries: Vec<Entry>) -> R<Roster> {
        let mut r = Roster::new();
        for e in entries {
            if r.offer(e)? == Offered::Duplicate {
                return Err(NetError::bad("a roster has each entry once"));
            }
        }
        Ok(r)
    }

    pub fn from_json(v: &Value) -> R<Roster> {
        let list = v.as_array().ok_or_else(|| NetError::bad("a roster is a list of entries"))?;
        Roster::from_entries(list.iter().map(Entry::parse).collect::<R<Vec<_>>>()?)
    }

    pub fn to_json(&self) -> Value {
        Value::Array(self.entries.iter().map(Entry::to_json).collect())
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries; the next one's index.
    pub fn len(&self) -> u64 {
        self.entries.len() as u64
    }

    /// The network as the chain leaves it; empty before entry 0.
    pub fn view(&self) -> View {
        self.view.clone().unwrap_or_default()
    }

    pub fn view_ref(&self) -> Option<&View> {
        self.view.as_ref()
    }

    /// The hash of the last entry, which presence carries.
    pub fn head(&self) -> Option<String> {
        self.entries.last().map(Entry::hash)
    }

    /// Takes an entry that extends the head, or drops one the chain has
    /// already; refuses a fork, a gap and anything the rules don't allow.
    pub fn offer(&mut self, e: Entry) -> R<Offered> {
        if let Some(have) = usize::try_from(e.n).ok().and_then(|i| self.entries.get(i)) {
            return if have.same(&e) && have.prev == e.prev { Ok(Offered::Duplicate) } else { Err(NetError::new(409, "roster-fork", "the roster has another entry there")) };
        }
        let prev = self.head().unwrap_or_default();
        check(self.view.as_ref(), &prev, self.len(), &e)?;
        self.view = Some(apply(self.view.take(), &e));
        self.entries.push(e);
        Ok(Offered::Appended)
    }

    /// Whether `e` would be taken now, without taking it.
    pub fn would_take(&self, e: &Entry) -> R<()> {
        check(self.view.as_ref(), &self.head().unwrap_or_default(), self.len(), e)
    }

    /// The next entry, unsigned: its index, `prev` and network filled in.
    pub fn draft(&self, op: Op, host: HostRef, gateway: Option<GatewayAt>, at: i64) -> Entry {
        Entry { n: self.len(), prev: self.head().unwrap_or_default(), network: self.view().network, op, host, gateway, at, signers: Vec::new() }
    }

    /// The entries from index `from` on.
    pub fn since(&self, from: u64) -> &[Entry] {
        let i = usize::try_from(from).unwrap_or(usize::MAX).min(self.entries.len());
        &self.entries[i..]
    }
}

/// Entry 0 of a new network, signed by its founder.
pub fn found(network: &str, founder: &str, key: &HostKey, gateway: Option<GatewayAt>, at: i64) -> Entry {
    let mut e = Entry {
        n: 0,
        prev: String::new(),
        network: network.into(),
        op: Op::Found,
        host: HostRef { id: founder.into(), keys: *key.public(), window: true },
        gateway,
        at,
        signers: Vec::new(),
    };
    e.sign_by(founder, key);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "net_0123456789abcdef0123456789abcdef";

    struct Hosts {
        keys: BTreeMap<&'static str, HostKey>,
    }

    impl Hosts {
        fn new(names: &[&'static str]) -> Hosts {
            Hosts { keys: names.iter().map(|n| (*n, HostKey::generate().unwrap())).collect() }
        }
        fn r(&self, id: &'static str, window: bool) -> HostRef {
            HostRef { id: id.into(), keys: *self.keys[id].public(), window }
        }
        fn entry(&self, r: &Roster, op: Op, host: HostRef, signers: &[&'static str]) -> Entry {
            let gateway = (op == Op::Gateway).then(|| GatewayAt { address: format!("{}.example.net", host.id), version: r.view().gateway_version + 1 });
            let mut e = r.draft(op, host, gateway, 1000 + r.len() as i64);
            for s in signers {
                e.sign_by(s, &self.keys[s]);
            }
            e
        }
    }

    /// studio founds; desk and nook are window hosts (added by studio);
    /// laptop isn't.
    fn network(h: &Hosts) -> Roster {
        let mut r = Roster::new();
        r.offer(found(NET, "studio", &h.keys["studio"], None, 1)).unwrap();
        for (id, window) in [("desk", true), ("nook", true), ("laptop", false)] {
            let e = h.entry(&r, Op::Add, h.r(id, window), &["studio"]);
            assert_eq!(r.offer(e).unwrap(), Offered::Appended, "{id}");
        }
        r
    }

    fn hosts() -> Hosts {
        Hosts::new(&["studio", "desk", "nook", "laptop", "spare", "tablet"])
    }

    #[test]
    fn found_is_entry_0_signed_by_the_founder_alone() {
        let h = hosts();
        let mut r = Roster::new();
        let mut e = found(NET, "studio", &h.keys["studio"], None, 1);
        e.sign_by("desk", &h.keys["desk"]);
        assert_eq!(r.clone().offer(e).unwrap_err().code, "roster-refused", "alone");
        let mut e = found(NET, "studio", &h.keys["studio"], None, 1);
        e.signers[0].sig = h.keys["desk"].sign(&e.signing_bytes());
        assert!(r.clone().offer(e).is_err(), "by its own key");
        let e = h.entry(&r, Op::Add, h.r("desk", false), &["desk"]);
        assert!(r.offer(e).is_err(), "a roster starts with found");
        let r = network(&h);
        let again = found(NET, "spare", &h.keys["spare"], None, 9);
        let again = Entry { n: r.len(), prev: r.head().unwrap(), ..again };
        assert!(r.clone().offer(again).is_err(), "once");
        assert_eq!(r.view().founder, "studio");
        assert_eq!(r.view().gateway, "studio");
    }

    #[test]
    fn an_ordinary_host_is_added_or_removed_by_any_window_host() {
        let h = hosts();
        let mut r = network(&h);
        // laptop isn't a window host: it can't add.
        let e = h.entry(&r, Op::Add, h.r("spare", false), &["laptop"]);
        assert_eq!(r.offer(e).unwrap_err().code, "roster-refused");
        let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
        r.offer(e).unwrap();
        let e = h.entry(&r, Op::Remove, h.r("spare", false), &["laptop"]);
        assert!(r.offer(e).is_err(), "not by an ordinary host");
        let e = h.entry(&r, Op::Remove, h.r("spare", false), &["nook"]);
        r.offer(e).unwrap();
        assert!(r.view().removal("spare").is_some());
        assert!(r.view().member("spare").is_none());
        // Removed, it can be added again (a reinstalled machine, Replace).
        let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
        r.offer(e).unwrap();
        assert!(r.view().removal("spare").is_none());
    }

    #[test]
    fn window_hosts_gateway_moves_and_their_removal_need_the_founder_or_two() {
        let h = hosts();
        let r = network(&h);
        let strong = [
            h.entry(&r, Op::Add, h.r("tablet", true), &[]),
            h.entry(&r, Op::Window, h.r("laptop", true), &[]),
            h.entry(&r, Op::Remove, h.r("desk", true), &[]),
            h.entry(&r, Op::Gateway, h.r("desk", true), &[]),
        ];
        for e in strong {
            let what = format!("{} {}", e.op.as_str(), e.host.id);
            let with = |signers: &[&'static str]| {
                let mut e = e.clone();
                for s in signers {
                    e.sign_by(s, &h.keys[s]);
                }
                r.clone().offer(e)
            };
            let one = if e.host.id == "desk" { "nook" } else { "desk" };
            assert_eq!(with(&[one]).unwrap_err().code, "needs-approval", "{what}: one window host");
            assert_eq!(with(&[one, "laptop"]).unwrap_err().code, "needs-approval", "{what}: an ordinary host doesn't count");
            assert_eq!(with(&["studio"]).unwrap(), Offered::Appended, "{what}: the founder");
            assert_eq!(with(&["desk", "nook"]).unwrap(), Offered::Appended, "{what}: two window hosts");
        }
        // A gateway entry moves the gateway, with a higher version each time.
        let mut r = r;
        let e = h.entry(&r, Op::Gateway, h.r("desk", true), &["studio"]);
        r.offer(e.clone()).unwrap();
        assert_eq!((r.view().gateway.as_str(), r.view().gateway_version), ("desk", 1));
        let mut stale = r.draft(Op::Gateway, h.r("nook", true), Some(GatewayAt { address: "x.example".into(), version: 1 }), 5);
        stale.sign_by("studio", &h.keys["studio"]);
        assert_eq!(r.offer(stale).unwrap_err().code, "stale");
    }

    #[test]
    fn two_window_hosts_approving_apart_make_one_entry_that_counts() {
        let h = hosts();
        let mut r = network(&h);
        let desk = h.entry(&r, Op::Gateway, h.r("desk", true), &["desk"]);
        assert_eq!(r.clone().offer(desk.clone()).unwrap_err().code, "needs-approval");
        let mut nook = desk.clone();
        nook.signers.clear();
        nook.sign_by("nook", &h.keys["nook"]);
        let mut both = desk.clone();
        both.merge_signers(&nook);
        both.merge_signers(&desk);
        assert_eq!(both.signers.len(), 2);
        assert_eq!(r.offer(both).unwrap(), Offered::Appended);
        assert_eq!(r.view().gateway, "desk");
    }

    #[test]
    fn the_founder_is_unremovable_and_a_host_may_remove_itself() {
        let h = hosts();
        let mut r = network(&h);
        for signers in [&["studio"][..], &["desk", "nook"], &["studio", "desk", "nook"]] {
            let e = h.entry(&r, Op::Remove, h.r("studio", true), signers);
            assert!(r.clone().offer(e).unwrap_err().message.contains("founder"), "{signers:?}");
        }
        // Leaving: laptop takes itself out, and so may a window host.
        let e = h.entry(&r, Op::Remove, h.r("laptop", false), &["laptop"]);
        r.offer(e).unwrap();
        let e = h.entry(&r, Op::Remove, h.r("nook", true), &["nook"]);
        r.offer(e).unwrap();
        // But not another host.
        let e = h.entry(&r, Op::Remove, h.r("desk", true), &["desk", "studio"]);
        assert!(r.clone().offer(e).is_ok());
        let mut e = h.entry(&r, Op::Remove, h.r("desk", true), &[]);
        e.sign_by("desk", &h.keys["desk"]);
        e.signers[0].host = "studio".into();
        assert!(r.offer(e).is_err(), "a signature checks against its signer's key");
    }

    #[test]
    fn duplicates_are_dropped_and_forks_refused() {
        let h = hosts();
        let mut r = network(&h);
        let base = r.clone();
        // Two window hosts approve the same thing at once.
        let a = h.entry(&base, Op::Add, h.r("spare", false), &["desk"]);
        let b = h.entry(&base, Op::Add, h.r("spare", false), &["nook"]);
        assert_eq!(r.offer(a).unwrap(), Offered::Appended);
        assert_eq!(r.offer(b).unwrap(), Offered::Duplicate);
        assert_eq!(r.len(), base.len() + 1);
        // A different choice at the same index loses to the chain's order.
        let c = h.entry(&base, Op::Add, h.r("spare", true), &["studio"]);
        assert_eq!(r.offer(c).unwrap_err().code, "roster-fork");
        let d = h.entry(&base, Op::Add, h.r("tablet", false), &["desk"]);
        assert_eq!(r.offer(d).unwrap_err().code, "roster-fork");
        // An entry past the head is a gap, not taken.
        let mut e = h.entry(&r, Op::Add, h.r("tablet", false), &[]);
        e.n += 1;
        e.sign_by("desk", &h.keys["desk"]);
        assert_eq!(r.offer(e).unwrap_err().code, "roster-gap");
        // The same host again is taken; its keys under another id too.
        let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
        assert_eq!(r.offer(e).unwrap_err().code, "taken");
        let e = h.entry(&r, Op::Add, HostRef { id: "other".into(), ..h.r("spare", false) }, &["desk"]);
        assert_eq!(r.offer(e).unwrap_err().code, "taken");
    }

    #[test]
    fn the_whole_chain_is_checked_and_survives_its_json() {
        let h = hosts();
        let r = network(&h);
        let back = Roster::from_json(&r.to_json()).unwrap();
        assert_eq!(back.view(), r.view());
        assert_eq!(back.head(), r.head());
        // A changed entry breaks its own signature, or the next one's prev.
        let mut v = r.to_json();
        v[2]["host"]["window"] = json!(false);
        assert!(Roster::from_json(&v).is_err());
        let mut v = r.to_json();
        v[1]["at"] = json!(77);
        let err = Roster::from_json(&v).unwrap_err();
        assert_eq!(err.code, "roster-refused", "{err}");
        // An entry dropped from the middle leaves a gap.
        let mut v = r.to_json();
        v.as_array_mut().unwrap().remove(1);
        assert!(Roster::from_json(&v).is_err());
        // A key id that doesn't match the keys.
        let mut v = r.to_json();
        v[1]["host"]["keyId"] = json!("xnk1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(Roster::from_json(&v).is_err());
        assert_eq!(r.since(2).len(), 2);
        assert_eq!(r.since(99).len(), 0);
    }
}
