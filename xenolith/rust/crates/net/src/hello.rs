//! sync/2's hello: a Xenolith's context key speaking for its endpoint, on one connection.
//!
//! The endpoint's transport key authenticates the connection (QUIC), and a share names both the
//! member's context key and its endpoint id (review revision 15), so each side checks that the
//! other's context key is the one its chains bind to that endpoint before anything of the
//! connection's spaces is read. The context key signs, after the tag `xenolith-sync-hello/1`:
//! both endpoint ids, a nonce, the time, the spaces it tells the peer of and the connection's own
//! keying material (`Link::exporter`, RFC 5705), which isn't sent: the receiver derives the same
//! bytes from its end of the connection. A hello caught on one connection fails on any other, and
//! a nonce seen in the last ten minutes or a time more than five minutes off is refused too, for a
//! transport that has no keying material.
//!
//! The spaces it lists are the ones the sender holds whose chain admits the receiver (revision
//! 6): each with its genesis hash and the digest of what the sender holds of it, so a member that
//! holds nothing of a space shared with it yet hears of it, and a space that doesn't admit the
//! peer is never named to it.

use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use xenonet_id::codec::{b64, random, unb64, unb64_n};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};

/// The label the connection's keying material is derived under.
pub const EXPORTER: &[u8] = b"xenolith/sync/2 hello";
/// How far a hello's time may be from this machine's.
pub const SKEW_MS: i64 = 5 * 60 * 1000;
/// How long a nonce is remembered.
const NONCE_MS: i64 = 10 * 60 * 1000;
const MAX_SPACES: usize = 256;

/// A space as a hello tells of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Told {
    pub space: String,
    pub genesis: String,
    pub digest: String,
}

/// A hello that checked: the peer's context key and the spaces it told of.
#[derive(Clone, Debug)]
pub struct Checked {
    pub key: VerifyingKey,
    pub spaces: Vec<Told>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn message(fields: &Map<String, Value>, exporter: Option<&[u8; 32]>) -> Vec<u8> {
    let mut m = Value::Object(fields.clone()).to_string().into_bytes();
    m.push(b'\n');
    m.extend_from_slice(exporter.map(|e| b64(e)).unwrap_or_default().as_bytes());
    m
}

/// This Xenolith's hello to `to` from `from` (endpoint ids), at `now` (ms), on a connection whose
/// keying material is `exporter`.
pub fn make(key: &dyn Signer, from: &[u8; 32], to: &[u8; 32], spaces: &[Told], exporter: Option<&[u8; 32]>, now: i64) -> Option<Value> {
    let mut f = Map::new();
    f.insert("v".into(), json!(1));
    f.insert("key".into(), key.public().to_json());
    f.insert("from".into(), json!(hex(from)));
    f.insert("to".into(), json!(hex(to)));
    f.insert("nonce".into(), json!(b64(&random::<16>().ok()?)));
    f.insert("at".into(), json!(now));
    f.insert("bound".into(), json!(exporter.is_some()));
    f.insert("spaces".into(), json!(spaces.iter().map(|t| json!({ "space": t.space, "genesis": t.genesis, "digest": t.digest })).collect::<Vec<_>>()));
    let sig = key.sign(Purpose::SYNC_HELLO, &message(&f, exporter)).ok()?;
    f.insert("sig".into(), json!(b64(&sig)));
    Some(Value::Object(f))
}

/// The nonces of the hellos taken lately.
#[derive(Default)]
pub struct Nonces(Mutex<VecDeque<(i64, String)>>);

impl Nonces {
    fn take(&self, nonce: &str, now: i64) -> bool {
        let mut q = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while q.front().is_some_and(|(at, _)| now - *at > NONCE_MS) {
            q.pop_front();
        }
        if q.iter().any(|(_, n)| n == nonce) {
            return false;
        }
        q.push_back((now, nonce.to_string()));
        true
    }
}

fn refused(why: &str) -> String {
    format!("a sync/2 hello was refused: {why}")
}

/// Checks a peer's hello on a connection from `from` to `me` (endpoint ids) whose keying material
/// is `exporter`: signed by the key it names, for this connection, fresh and not seen before.
/// Who that key may be is the caller's to decide, by its chains.
pub fn check(v: Option<&Value>, me: &[u8; 32], from: &[u8; 32], exporter: Option<&[u8; 32]>, nonces: &Nonces, now: i64) -> Result<Checked, String> {
    let v = v.and_then(Value::as_object).ok_or_else(|| refused("it has no signed part"))?;
    if v.get("v") != Some(&json!(1)) {
        return Err(refused("its version"));
    }
    let key = v.get("key").and_then(VerifyingKey::parse).ok_or_else(|| refused("its key"))?;
    if v.get("from").and_then(Value::as_str) != Some(hex(from).as_str()) || v.get("to").and_then(Value::as_str) != Some(hex(me).as_str()) {
        return Err(refused("it's for another connection"));
    }
    if v.get("bound").and_then(Value::as_bool) != Some(exporter.is_some()) {
        return Err(refused("it's for another connection"));
    }
    let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| refused("its time"))?;
    if (at - now).abs() > SKEW_MS {
        return Err(refused("its time is more than five minutes from this machine's"));
    }
    let nonce = v.get("nonce").and_then(Value::as_str).filter(|n| unb64_n::<16>(n).is_some()).ok_or_else(|| refused("its nonce"))?;
    let sig = v.get("sig").and_then(Value::as_str).and_then(unb64).ok_or_else(|| refused("its signature"))?;
    let mut fields = v.clone();
    fields.remove("sig");
    if !key.verify(Purpose::SYNC_HELLO, &message(&fields, exporter), &sig) {
        return Err(refused("its signature doesn't check, or it was made for another connection"));
    }
    let list = v.get("spaces").and_then(Value::as_array).filter(|s| s.len() <= MAX_SPACES).ok_or_else(|| refused("its spaces"))?;
    let mut spaces = Vec::new();
    for t in list {
        let s = |k: &str| t.get(k).and_then(Value::as_str).map(String::from);
        match (s("space").filter(|x| xenonet_model::ids::SpaceId::parse(x).is_ok()), s("genesis"), s("digest")) {
            (Some(space), Some(genesis), Some(digest)) => spaces.push(Told { space, genesis, digest }),
            _ => return Err(refused("its spaces")),
        }
    }
    // Last, so a hello refused for anything else doesn't use up its nonce.
    if !nonces.take(nonce, now) {
        return Err(refused("it was taken before (a replay)"));
    }
    Ok(Checked { key, spaces })
}

/// Which keys each endpoint may speak as, and in which spaces, by this Xenolith's chains and
/// follow records.
pub type Bindings = BTreeMap<String, BTreeMap<String, std::collections::BTreeSet<String>>>;

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    const A: [u8; 32] = [1; 32];
    const B: [u8; 32] = [2; 32];
    const T: i64 = 1_792_512_000_000;

    fn key(n: u8) -> SeedSigner {
        SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
    }

    fn told() -> Vec<Told> {
        vec![Told { space: "spc_abcdefghijklmnopqrstuvwxyz234567".into(), genesis: "g".into(), digest: "d".into() }]
    }

    #[test]
    fn a_hello_checks_on_its_own_connection_once() {
        let k = key(7);
        let x1 = [9u8; 32];
        let h = make(&k, &A, &B, &told(), Some(&x1), T).unwrap();
        let nonces = Nonces::default();
        let c = check(Some(&h), &B, &A, Some(&x1), &nonces, T + 1000).unwrap();
        assert_eq!((c.key.id(), c.spaces), (k.public().id(), told()));
        // The same bytes again, on the same connection or another: refused.
        assert!(check(Some(&h), &B, &A, Some(&x1), &nonces, T + 2000).unwrap_err().contains("replay"));
        let fresh = Nonces::default();
        assert!(check(Some(&h), &B, &A, Some(&[8u8; 32]), &fresh, T).unwrap_err().contains("another connection"), "replayed on another connection");
        assert!(check(Some(&h), &B, &A, None, &fresh, T).unwrap_err().contains("another connection"), "on a transport without keying material");
    }

    #[test]
    fn a_hello_is_refused_for_another_pair_a_stale_time_a_changed_field_or_none() {
        let k = key(7);
        let h = make(&k, &A, &B, &told(), None, T).unwrap();
        let n = Nonces::default();
        assert!(check(Some(&h), &A, &B, None, &n, T).is_err(), "from and to swapped");
        assert!(check(Some(&h), &B, &[3; 32], None, &n, T).is_err(), "from another endpoint");
        assert!(check(Some(&h), &B, &A, None, &n, T + SKEW_MS + 1).unwrap_err().contains("five minutes"));
        let mut lied = h.clone();
        lied["spaces"] = json!([]);
        assert!(check(Some(&lied), &B, &A, None, &n, T).unwrap_err().contains("signature"));
        let mut other = h.clone();
        other["key"] = key(8).public().to_json();
        assert!(check(Some(&other), &B, &A, None, &n, T).unwrap_err().contains("signature"), "another key's name on it");
        assert!(check(None, &B, &A, None, &n, T).is_err());
        assert!(check(Some(&h), &B, &A, None, &n, T).is_ok(), "the refusals used up no nonce");
    }
}
