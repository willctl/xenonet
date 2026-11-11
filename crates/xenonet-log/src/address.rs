//! A machine's address record (`xenonet-address/1`): where its endpoint
//! can be reached now, as it tells the members of one network. Peers keep
//! the records they check in their address lookup until they expire.
//!
//! A record names the network it was sent into, the member (its host id),
//! its endpoint id (the transport key's Ed25519 public half, which iroh
//! dials by), at most one home relay, the direct addresses it listens on,
//! when it was made and how many seconds it holds. The machine signs the
//! canonical JSON of all that with its machine signing key, after the tag
//! and a newline, so a copied `host.key` can't say where the machine is.
//!
//! A receiver takes a record only when, by its own copy of the network's
//! chain, the host is a member, the endpoint id is that member's transport
//! key now, the signature is by that member's machine signing key, and the
//! relay, if any, is one the network lists: a record never sends a peer to
//! a relay the network didn't choose. A record past its time is dropped,
//! and so is one made more than five minutes ahead of the receiver's
//! clock, since it would hold longer than its TTL says.

use crate::chain::{ChainError, R};
use crate::network::{is_host_id, is_network_id, is_relay_url, View};
use serde_json::{Map, Value};
use std::net::SocketAddr;
use xenonet_id::codec::{b64, unb64, unb64_n};
use xenonet_id::sign::{Purpose, Signer};

/// The most direct addresses a record names.
pub const MAX_ADDRS: usize = 16;
/// The longest a record holds, in seconds: a day.
pub const MAX_TTL: u64 = 24 * 60 * 60;
/// How far ahead of the receiver's clock a record may be made, in ms.
pub const MAX_AHEAD: u64 = 5 * 60 * 1000;
/// The most bytes a record's JSON takes.
pub const MAX_RECORD: usize = 4096;

/// Where a member's endpoint is reached, signed by its machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressRecord {
    pub network: String,
    pub host: String,
    /// The endpoint id: the transport key's Ed25519 public key.
    pub endpoint: [u8; 32],
    /// The home relay, only when the network lists it.
    pub relay: Option<String>,
    /// Direct addresses, `ip:port`.
    pub addrs: Vec<String>,
    /// When it was made, in ms.
    pub at: u64,
    /// How long it holds from `at`, in seconds.
    pub ttl: u64,
    /// Empty until signed.
    pub sig: Vec<u8>,
}

fn bad(what: &str) -> ChainError {
    ChainError::bad(format!("an address record's {what} isn't right"))
}

impl AddressRecord {
    fn unsigned(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("network".into(), Value::String(self.network.clone()));
        m.insert("host".into(), Value::String(self.host.clone()));
        m.insert("endpoint".into(), Value::String(b64(&self.endpoint)));
        m.insert("relay".into(), self.relay.clone().map_or(Value::Null, Value::String));
        m.insert("addrs".into(), Value::from(self.addrs.clone()));
        m.insert("at".into(), Value::from(self.at));
        m.insert("ttl".into(), Value::from(self.ttl));
        m
    }

    /// What the machine signs: the tag, a newline, the canonical JSON
    /// without the signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        Purpose::ADDRESS.bytes(Value::Object(self.unsigned()).to_string().as_bytes())
    }

    /// When it stops holding, in ms.
    pub fn until(&self) -> u64 {
        self.at.saturating_add(self.ttl.saturating_mul(1000))
    }

    /// Signs it with the machine signing key.
    pub fn sign(&mut self, key: &dyn Signer) -> xenonet_id::R<()> {
        self.sig = key.sign(Purpose::ADDRESS, Value::Object(self.unsigned()).to_string().as_bytes())?;
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut m = self.unsigned();
        m.insert("sig".into(), Value::String(b64(&self.sig)));
        Value::Object(m)
    }

    pub fn encode(&self) -> Vec<u8> {
        self.to_json().to_string().into_bytes()
    }

    /// What `parse` holds a record to, for one made here.
    pub fn well_formed(&self) -> R<()> {
        if !is_network_id(&self.network) {
            return Err(bad("network"));
        }
        if !is_host_id(&self.host) {
            return Err(bad("host"));
        }
        if self.relay.as_deref().is_some_and(|r| !is_relay_url(r)) {
            return Err(bad("relay"));
        }
        let addr_ok = |a: &String| a.parse::<SocketAddr>().is_ok_and(|s| s.to_string() == *a && s.port() != 0);
        if self.addrs.len() > MAX_ADDRS || !self.addrs.iter().all(addr_ok) || self.addrs.iter().enumerate().any(|(i, a)| self.addrs[..i].contains(a)) {
            return Err(bad("addresses"));
        }
        if self.ttl == 0 || self.ttl > MAX_TTL {
            return Err(bad("ttl"));
        }
        if !(64..=72).contains(&self.sig.len()) {
            return Err(bad("signature"));
        }
        if self.encode().len() > MAX_RECORD {
            return Err(bad("size"));
        }
        Ok(())
    }

    pub fn parse(v: &Value) -> R<AddressRecord> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| bad(k));
        let n = |k: &str| v.get(k).and_then(Value::as_u64).ok_or_else(|| bad(k));
        let relay = match v.get("relay") {
            None | Some(Value::Null) => None,
            Some(r) => Some(r.as_str().ok_or_else(|| bad("relay"))?.to_string()),
        };
        let addrs = v.get("addrs").and_then(Value::as_array).ok_or_else(|| bad("addresses"))?;
        let r = AddressRecord {
            network: s("network")?.into(),
            host: s("host")?.into(),
            endpoint: unb64_n::<32>(s("endpoint")?).ok_or_else(|| bad("endpoint"))?,
            relay,
            addrs: addrs.iter().map(|a| a.as_str().map(str::to_string).ok_or_else(|| bad("addresses"))).collect::<R<Vec<_>>>()?,
            at: n("at")?,
            ttl: n("ttl")?,
            sig: unb64(s("sig")?).ok_or_else(|| bad("signature"))?,
        };
        r.well_formed()?;
        Ok(r)
    }

    /// A record from the bytes it came as.
    pub fn decode(bytes: &[u8]) -> R<AddressRecord> {
        if bytes.len() > MAX_RECORD {
            return Err(bad("size"));
        }
        let v: Value = serde_json::from_slice(bytes).map_err(|_| bad("JSON"))?;
        AddressRecord::parse(&v)
    }

    /// Whether `view`, the receiver's copy of the network's chain, says
    /// this record is the member's own and holds at `now` (ms).
    pub fn verify(&self, view: &View, now: u64) -> R<()> {
        self.well_formed()?;
        let refused = |m: String| ChainError::new(403, "address-refused", m);
        if self.network != view.network {
            return Err(refused("that address record is for another network".into()));
        }
        let Some(m) = view.member(&self.host) else { return Err(refused(format!("{} isn't in the network", self.host))) };
        if m.keys.sign != self.endpoint {
            return Err(refused(format!("that isn't {}'s endpoint now", self.host)));
        }
        if !m.machine.verify_raw(&self.signing_bytes(), &self.sig) {
            return Err(refused(format!("{}'s signature on an address record doesn't check", self.host)));
        }
        if let Some(relay) = self.relay.as_deref().filter(|r| !view.relays.iter().any(|x| x == r)) {
            return Err(refused(format!("the network lists no relay {relay}")));
        }
        if self.at > now.saturating_add(MAX_AHEAD) {
            return Err(ChainError::new(409, "clock-skew", "that address record was made ahead of this machine's clock"));
        }
        if self.until() <= now {
            return Err(ChainError::new(410, "expired", "that address record has expired"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
