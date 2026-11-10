//! A network's chain, the roster (docs/design/network.md, section 3): the
//! signed chain of a network's hosts and their keys
//! (`network/roster-entry/v1`). Every host keeps the chain and checks it
//! whole, and accepts only entries that extend its own head, so a gateway
//! can withhold an entry but never add or change one.
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

use crate::chain::{self, Chain, ChainError, Codes, Format, Policy, R};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use xenonet_id::keys::{HostKey, PublicKeys};
use xenonet_id::sign::{Purpose, VerifyingKey};

pub const ROSTER_TAG: &str = Purpose::ROSTER.tag();
/// The longest gateway address an entry carries, in bytes.
pub const ADDRESS_MAX: usize = 300;

/// A network's chain.
pub type Roster = Chain<NetworkPolicy>;
/// An entry of a network's chain.
pub type Entry = chain::Entry<NetworkPolicy>;

fn address_ok(address: &str) -> bool {
    !address.is_empty() && address.len() <= ADDRESS_MAX
}

/// `net_<32 hex>`, as the founder makes it.
pub fn is_network_id(id: &str) -> bool {
    id.strip_prefix("net_").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// A host's id: 2 to 32 lowercase letters, digits and dashes, starting
/// with a letter. The same rule as `xenonet_model::ids::is_host_id`, which
/// this crate doesn't depend on (a test in the network crate holds the two
/// together).
pub fn is_host_id(host: &str) -> bool {
    let b = host.as_bytes();
    (2..=32).contains(&b.len()) && b[0].is_ascii_lowercase() && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

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

/// What a roster entry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub network: String,
    pub op: Op,
    pub host: HostRef,
    pub gateway: Option<GatewayAt>,
    pub at: i64,
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

/// Signed rightly, but by too few: the entry may wait for the founder or a
/// second window host to sign it too.
pub fn needs_approval() -> ChainError {
    ChainError::needs_approval("that needs the founder, or two window hosts")
}

fn no_host(id: &str) -> ChainError {
    ChainError::new(404, "no-host", format!("{id} isn't in the network with those keys"))
}

fn ed25519(keys: &PublicKeys) -> VerifyingKey {
    VerifyingKey::Ed25519(keys.sign)
}

/// Today's roster rules. It holds nothing yet; the rules that check a
/// signer's device against its person's chain will hold that chain here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkPolicy;

impl Policy for NetworkPolicy {
    type Body = Body;
    type State = View;

    const FORMAT: Format = Format {
        tag: Purpose::ROSTER,
        noun: "roster",
        signer: "host",
        signer_key: "host",
        presence: false,
        codes: Codes { gap: "roster-gap", fork: "roster-fork", refused: "roster-refused" },
    };

    fn write_body(b: &Body, out: &mut Map<String, Value>) {
        out.insert("network".into(), Value::String(b.network.clone()));
        out.insert("op".into(), Value::String(b.op.as_str().into()));
        out.insert(
            "host".into(),
            json!({
                "id": b.host.id,
                "sign": b.host.keys.sign_b64(),
                "seal": b.host.keys.seal_b64(),
                "keyId": b.host.keys.key_id(),
                "window": b.host.window,
            }),
        );
        out.insert("gateway".into(), b.gateway.as_ref().map_or(Value::Null, |g| json!({ "address": g.address, "version": g.version })));
        out.insert("at".into(), Value::from(b.at));
    }

    fn parse_body(v: &Value) -> R<Body> {
        let bad = |what: &str| Self::FORMAT.bad(what);
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
                address: g.get("address").and_then(Value::as_str).filter(|a| address_ok(a)).ok_or_else(|| bad("gateway address"))?.to_string(),
                version: g.get("version").and_then(Value::as_u64).ok_or_else(|| bad("gateway version"))?,
            }),
        };
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        Ok(Body { network: network.into(), op, host: HostRef { id: id.into(), keys, window }, gateway, at })
    }

    fn body_well_formed(b: &Body) -> R<()> {
        let bad = |what: &str| Self::FORMAT.bad(what);
        if !is_network_id(&b.network) {
            return Err(bad("network"));
        }
        if !is_host_id(&b.host.id) {
            return Err(bad("host id"));
        }
        if b.gateway.as_ref().is_some_and(|g| !address_ok(&g.address)) {
            return Err(bad("gateway address"));
        }
        Ok(())
    }

    fn is_signer(id: &str) -> bool {
        is_host_id(id)
    }

    fn sig_ok(sig: &[u8]) -> bool {
        sig.len() == 64
    }

    fn same(a: &Body, b: &Body) -> bool {
        a.op == b.op && a.host == b.host && a.gateway == b.gateway && a.network == b.network
    }

    fn scope(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(view) = view else {
            // Entry 0: the founder's, signed by itself.
            if e.op != Op::Found {
                return Err(Self::FORMAT.refused("a roster starts with its founder"));
            }
            if e.signers.len() != 1 || e.signers[0].by != e.host.id {
                return Err(founder_alone());
            }
            return Ok(());
        };
        if e.network != view.network {
            return Err(Self::FORMAT.refused("that entry is for another network"));
        }
        Ok(())
    }

    fn signer_key(&self, view: Option<&View>, e: &Entry, by: &str) -> R<VerifyingKey> {
        match view {
            None => Ok(ed25519(&e.host.keys)),
            Some(view) => view.member(by).map(|m| ed25519(&m.keys)).ok_or_else(|| Self::FORMAT.refused(format!("{by} isn't in the network"))),
        }
    }

    fn bad_signature(&self, view: Option<&View>, by: &str) -> ChainError {
        match view {
            None => founder_alone(),
            Some(_) => Self::FORMAT.refused(format!("{by}'s signature doesn't check")),
        }
    }

    fn check(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(view) = view else { return Ok(()) };
        let refused = |m: String| Self::FORMAT.refused(m);
        let seen: Vec<&str> = e.signers.iter().map(|s| s.by.as_str()).collect();
        let founder = seen.contains(&view.founder.as_str());
        let windows = seen.iter().filter(|h| view.is_window(h)).count();
        let strong = founder || windows >= 2;
        let any_window = founder || windows >= 1;
        let current = view.member(&e.host.id);
        let same_keys = |m: &Member| m.keys == e.host.keys;
        match e.op {
            Op::Found => Err(refused("a network has one founder".into())),
            Op::Add => {
                if current.is_some() {
                    return Err(ChainError::taken(format!("{} is in the network already", e.host.id)));
                }
                if view.members.values().any(|m| m.keys == e.host.keys) {
                    return Err(ChainError::taken("those keys are another host's"));
                }
                if e.gateway.is_some() {
                    return Err(ChainError::bad("only a gateway entry names a gateway"));
                }
                match (e.host.window, strong, any_window) {
                    (true, false, _) => Err(needs_approval()),
                    (false, _, false) => Err(refused("adding a host needs a window host".into())),
                    _ => Ok(()),
                }
            }
            Op::Remove => {
                let Some(m) = current.filter(|m| same_keys(m)) else { return Err(no_host(&e.host.id)) };
                if m.id == view.founder {
                    return Err(refused("the founder can't be removed: a network whose founder is lost is made again".into()));
                }
                // Removed, the gateway's host would be cut off from a network
                // still pinned to it; and every host would lose its gateway.
                if m.id == view.gateway {
                    return Err(refused(format!("{} is the network's gateway: move the gateway first", m.id)));
                }
                if e.gateway.is_some() {
                    return Err(ChainError::bad("only a gateway entry names a gateway"));
                }
                if e.signers.len() == 1 && e.signers[0].by == m.id {
                    return Ok(());
                }
                match (view.is_window(&m.id), strong, any_window) {
                    (true, false, _) => Err(needs_approval()),
                    (false, _, false) => Err(refused("removing a host needs a window host".into())),
                    _ => Ok(()),
                }
            }
            Op::Window => {
                let Some(m) = current.filter(|m| same_keys(m)) else { return Err(no_host(&e.host.id)) };
                if m.window == e.host.window {
                    return Err(ChainError::no_change("that's the window choice it has"));
                }
                if e.gateway.is_some() {
                    return Err(ChainError::bad("only a gateway entry names a gateway"));
                }
                if strong {
                    Ok(())
                } else {
                    Err(needs_approval())
                }
            }
            Op::Gateway => {
                if current.is_none_or(|m| !same_keys(m)) {
                    return Err(no_host(&e.host.id));
                }
                let Some(g) = &e.gateway else { return Err(ChainError::bad("a gateway entry names where the gateway is")) };
                if g.version <= view.gateway_version {
                    return Err(ChainError::stale("a gateway entry needs a higher version than the last"));
                }
                if strong {
                    Ok(())
                } else {
                    Err(needs_approval())
                }
            }
        }
    }

    fn apply(&self, view: Option<View>, e: &Entry) -> View {
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
                v.removed.insert(e.host.id.clone(), Removal { n: e.n, by: e.signers.iter().map(|s| s.by.clone()).collect(), at: e.at, keys: e.host.keys });
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
}

fn founder_alone() -> ChainError {
    NetworkPolicy::FORMAT.refused("the founder signs the first entry, alone")
}

impl Entry {
    /// Adds `host`'s signature.
    pub fn sign_by(&mut self, host: &str, key: &HostKey) {
        let sig = key.sign(&self.signing_bytes());
        self.add_signature(host, sig, None);
    }
}

impl Roster {
    /// The next entry, unsigned: its index, `prev` and network filled in.
    pub fn draft(&self, op: Op, host: HostRef, gateway: Option<GatewayAt>, at: i64) -> Entry {
        self.next(Body { network: self.view().network, op, host, gateway, at })
    }
}

/// Entry 0 of a new network, signed by its founder.
pub fn found(network: &str, founder: &str, key: &HostKey, gateway: Option<GatewayAt>, at: i64) -> Entry {
    let mut e = Entry::new(0, "", Body { network: network.into(), op: Op::Found, host: HostRef { id: founder.into(), keys: *key.public(), window: true }, gateway, at });
    e.sign_by(founder, key);
    e
}

#[cfg(test)]
mod tests;
