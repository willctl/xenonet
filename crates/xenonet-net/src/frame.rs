//! A frame on the wire (`network/frame/v1`, docs/design/network.md,
//! section 3 and "The wire"): one JSON object a line, signed by the host
//! that sent it and, between hosts, sealed for the host it's for.
//!
//! The head is one string, what is signed:
//!
//! ```text
//! xenonet-net/1
//! <network id>
//! <frame id>
//! <kind>
//! <from>
//! <to>
//! <at>
//! <sealed: 1 or 0>
//! <base64url SHA-256 of the body bytes as sent>
//! ```
//!
//! Sealing binds the head too, as HKDF's info and as the associated data,
//! but the head's last line is the hash of the sealed bytes, which don't
//! exist until the sealing is done. So sealing binds the head's first
//! eight lines (`sealing_head`), and the signature, made after, covers
//! all nine, and with them the ciphertext.
//!
//! Sealing: an ephemeral X25519 key agrees a secret with the recipient's;
//! HKDF-SHA256 (no salt) makes a key from it, with the ephemeral public
//! key, the recipient's key id and the sealing head as its info; the body
//! is ChaCha20-Poly1305 with a zero nonce (each key is used once) and the
//! sealing head as associated data, with the ephemeral public key in front
//! of the ciphertext.

use crate::codec::{b64, hash_b64, is_b64, is_frame_id, unb64};
use crate::keys::{HostKey, PublicKeys};
use crate::{NetError, R};
use ring::{aead, hkdf};
use serde_json::{json, Map, Value};
use x25519_dalek::{PublicKey as SealPublic, StaticSecret};
use xenonet_base::ids::is_host_id;

/// Every kind a frame can be.
pub const FRAME_KINDS: [&str; 12] = ["hello", "ping", "gap", "roster", "removed", "presence", "claims", "message", "receipt", "handoff", "relay", "operator"];
/// The gateway's own frames: unsealed, signed by the gateway's host key.
pub const GATEWAY_KINDS: [&str; 5] = ["hello", "ping", "gap", "roster", "removed"];
/// Frames between hosts: sealed, and signed by the host that sent them.
pub const HOST_KINDS: [&str; 7] = ["presence", "claims", "message", "receipt", "handoff", "relay", "operator"];
/// What the gateway keeps for a host that's away (core/schema/network.js's
/// QUEUED_KINDS); the rest are dropped for it, since a full set goes when
/// it's back.
pub const QUEUED_KINDS: [&str; 5] = ["message", "receipt", "handoff", "operator", "roster"];

pub const HEAD_TAG: &str = "xenonet-net/1";
const NONCE: [u8; 12] = [0u8; 12];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub id: String,
    /// Set by the gateway, per recipient, within its epoch.
    pub seq: Option<u64>,
    pub kind: String,
    pub from: String,
    pub to: String,
    /// When the sender made it, in ms, by its clock as the gateway keeps it.
    pub at: i64,
    pub sealed: bool,
    /// The body's bytes as sent: the sealed bytes, or the JSON of an
    /// unsealed frame.
    pub body: Vec<u8>,
    pub sig: Vec<u8>,
}

/// The head's first eight lines: what sealing binds.
pub fn sealing_head(network: &str, id: &str, kind: &str, from: &str, to: &str, at: i64, sealed: bool) -> String {
    format!("{HEAD_TAG}\n{network}\n{id}\n{kind}\n{from}\n{to}\n{at}\n{}", u8::from(sealed))
}

impl Frame {
    pub fn sealing_head(&self, network: &str) -> String {
        sealing_head(network, &self.id, &self.kind, &self.from, &self.to, self.at, self.sealed)
    }

    /// The whole head: what the signature covers.
    pub fn head(&self, network: &str) -> String {
        format!("{}\n{}", self.sealing_head(network), hash_b64(&self.body))
    }

    /// Whether `keys` signed this frame's head in `network`.
    pub fn verify(&self, network: &str, keys: &PublicKeys) -> bool {
        keys.verify(self.head(network).as_bytes(), &self.sig)
    }

    pub fn is_gateway_kind(&self) -> bool {
        GATEWAY_KINDS.contains(&self.kind.as_str())
    }

    pub fn queued(&self) -> bool {
        QUEUED_KINDS.contains(&self.kind.as_str())
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("v".into(), json!(1));
        m.insert("id".into(), json!(self.id));
        if let Some(s) = self.seq {
            m.insert("seq".into(), json!(s));
        }
        m.insert("kind".into(), json!(self.kind));
        m.insert("from".into(), json!(self.from));
        m.insert("to".into(), json!(self.to));
        m.insert("at".into(), json!(self.at));
        m.insert("sealed".into(), json!(self.sealed));
        m.insert("body".into(), json!(b64(&self.body)));
        m.insert("sig".into(), json!(b64(&self.sig)));
        Value::Object(m)
    }

    /// One NDJSON line, with its newline.
    pub fn line(&self) -> String {
        format!("{}\n", self.to_json())
    }

    /// A frame from its JSON, with every field checked for its shape. The
    /// signature isn't checked here: that needs the roster.
    pub fn parse(v: &Value) -> R<Frame> {
        let bad = |what: &str| NetError::bad(format!("a frame's {what} isn't right"));
        if v.get("v") != Some(&json!(1)) {
            return Err(bad("version"));
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let id = text("id").filter(|i| is_frame_id(i)).ok_or_else(|| bad("id"))?;
        let seq = match v.get("seq") {
            None | Some(Value::Null) => None,
            Some(s) => Some(s.as_u64().ok_or_else(|| bad("seq"))?),
        };
        let kind = text("kind").filter(|k| FRAME_KINDS.contains(k)).ok_or_else(|| bad("kind"))?;
        let from = text("from").filter(|h| is_host_id(h)).ok_or_else(|| bad("from"))?;
        let to = text("to").filter(|h| *h == "*" || is_host_id(h)).ok_or_else(|| bad("to"))?;
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        let sealed = v.get("sealed").and_then(Value::as_bool).ok_or_else(|| bad("sealed"))?;
        let body = text("body").filter(|b| is_b64(b)).and_then(unb64).ok_or_else(|| bad("body"))?;
        let sig = text("sig").and_then(unb64).filter(|s| s.len() == 64).ok_or_else(|| bad("signature"))?;
        Ok(Frame { id: id.into(), seq, kind: kind.into(), from: from.into(), to: to.into(), at, sealed, body, sig })
    }

    pub fn parse_line(line: &str) -> R<Frame> {
        let v: Value = serde_json::from_str(line).map_err(|_| NetError::bad("a frame isn't JSON"))?;
        Frame::parse(&v)
    }

    /// An unsealed frame's body, as JSON.
    pub fn body_json(&self) -> R<Value> {
        if self.sealed {
            return Err(NetError::bad("that frame's body is sealed"));
        }
        serde_json::from_slice(&self.body).map_err(|_| NetError::bad("a frame's body isn't JSON"))
    }
}

fn sealing_key(shared: &[u8; 32], eph_pub: &[u8; 32], key_id: &str, head: &str) -> R<aead::LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]).extract(shared);
    let info: [&[u8]; 3] = [eph_pub, key_id.as_bytes(), head.as_bytes()];
    let okm = prk.expand(&info, &aead::CHACHA20_POLY1305).map_err(|_| NetError::internal("HKDF refused"))?;
    Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
}

/// Seals `plain` for `to` with the ephemeral key `eph`: the ephemeral
/// public key, then the ciphertext and its tag. A fixed `eph` is for
/// known-answer tests only; `seal` draws a new one each time.
pub fn seal_with(eph: [u8; 32], to: &PublicKeys, head: &str, plain: &[u8]) -> R<Vec<u8>> {
    let secret = StaticSecret::from(eph);
    let eph_pub = SealPublic::from(&secret).to_bytes();
    let shared = secret.diffie_hellman(&SealPublic::from(to.seal));
    if !shared.was_contributory() {
        return Err(NetError::bad("that host's sealing key can't be sealed to"));
    }
    let key = sealing_key(shared.as_bytes(), &eph_pub, &to.key_id(), head)?;
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(NONCE), aead::Aad::from(head.as_bytes()), &mut buf).map_err(|_| NetError::internal("sealing failed"))?;
    let mut out = Vec::with_capacity(32 + buf.len());
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&buf);
    Ok(out)
}

/// Seals `plain` for `to`, binding `head`.
pub fn seal(to: &PublicKeys, head: &str, plain: &[u8]) -> R<Vec<u8>> {
    seal_with(crate::codec::random::<32>()?, to, head, plain)
}

/// Opens what was sealed for this host's key, binding `head`.
pub fn open(key: &HostKey, head: &str, sealed: &[u8]) -> R<Vec<u8>> {
    let refused = || NetError::new(400, "unsealed", "that frame wasn't sealed for this host, or was changed");
    if sealed.len() < 32 + 16 {
        return Err(refused());
    }
    let mut eph_pub = [0u8; 32];
    eph_pub.copy_from_slice(&sealed[..32]);
    let shared = key.agree(&eph_pub).ok_or_else(refused)?;
    let k = sealing_key(&shared, &eph_pub, &key.key_id(), head)?;
    let mut buf = sealed[32..].to_vec();
    let plain = k.open_in_place(aead::Nonce::assume_unique_for_key(NONCE), aead::Aad::from(head.as_bytes()), &mut buf).map_err(|_| refused())?;
    Ok(plain.to_vec())
}

/// What a host frame says, before it's sealed and signed.
pub struct Draft<'a> {
    pub network: &'a str,
    pub id: String,
    pub kind: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub at: i64,
}

/// A host frame: `body` sealed for `to_keys`, the head signed with `key`.
/// `eph` fixes the ephemeral key, for known-answer tests.
pub fn host_frame(d: &Draft<'_>, key: &HostKey, to_keys: &PublicKeys, body: &Value, eph: Option<[u8; 32]>) -> R<Frame> {
    let head = sealing_head(d.network, &d.id, d.kind, d.from, d.to, d.at, true);
    let plain = body.to_string();
    let sealed = match eph {
        Some(e) => seal_with(e, to_keys, &head, plain.as_bytes())?,
        None => seal(to_keys, &head, plain.as_bytes())?,
    };
    let mut f = Frame { id: d.id.clone(), seq: None, kind: d.kind.into(), from: d.from.into(), to: d.to.into(), at: d.at, sealed: true, body: sealed, sig: Vec::new() };
    f.sig = key.sign(f.head(d.network).as_bytes());
    Ok(f)
}

/// A gateway frame: `body` as JSON, unsealed, the head signed with the
/// gateway's host key.
pub fn gateway_frame(d: &Draft<'_>, key: &HostKey, body: &Value) -> Frame {
    let mut f = Frame {
        id: d.id.clone(),
        seq: None,
        kind: d.kind.into(),
        from: d.from.into(),
        to: d.to.into(),
        at: d.at,
        sealed: false,
        body: body.to_string().into_bytes(),
        sig: Vec::new(),
    };
    f.sig = key.sign(f.head(d.network).as_bytes());
    f
}

/// A host frame's body, opened with this host's key: call it only after
/// the signature was checked against the roster.
pub fn open_frame(network: &str, key: &HostKey, f: &Frame) -> R<Value> {
    if !f.sealed {
        return Err(NetError::bad("a frame between hosts must be sealed"));
    }
    let plain = open(key, &f.sealing_head(network), &f.body)?;
    serde_json::from_slice(&plain).map_err(|_| NetError::bad("a frame's body isn't JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NET: &str = "net_0123456789abcdef0123456789abcdef";

    fn draft(id: &str) -> Draft<'static> {
        Draft { network: NET, id: id.into(), kind: "message", from: "studio", to: "laptop", at: 1_792_512_345_678 }
    }

    #[test]
    fn a_frame_seals_signs_and_opens() {
        let studio = HostKey::generate().unwrap();
        let laptop = HostKey::generate().unwrap();
        let id = crate::codec::frame_id().unwrap();
        let body = json!({ "schema": "network/message/v1", "body": "hi" });
        let f = host_frame(&draft(&id), &studio, laptop.public(), &body, None).unwrap();
        assert!(f.verify(NET, studio.public()));
        assert!(!f.verify(NET, laptop.public()), "only the sender's key");
        assert!(!f.verify("net_ffffffffffffffffffffffffffffffff", studio.public()), "another network");
        assert_eq!(open_frame(NET, &laptop, &f).unwrap(), body);
        // Through its JSON and back.
        let back = Frame::parse_line(&f.line()).unwrap();
        assert_eq!(back, f);
        assert_eq!(open_frame(NET, &laptop, &back).unwrap(), body);
    }

    #[test]
    fn a_wrong_recipient_a_changed_head_or_body_fails() {
        let studio = HostKey::generate().unwrap();
        let laptop = HostKey::generate().unwrap();
        let other = HostKey::generate().unwrap();
        let id = crate::codec::frame_id().unwrap();
        let f = host_frame(&draft(&id), &studio, laptop.public(), &json!({ "a": 1 }), None).unwrap();
        assert_eq!(open_frame(NET, &other, &f).unwrap_err().code, "unsealed", "a wrong recipient");
        for change in 0..5 {
            let mut g = f.clone();
            match change {
                0 => g.to = "desk".into(),
                1 => g.at += 1,
                2 => g.kind = "receipt".into(),
                3 => g.id = crate::codec::frame_id().unwrap(),
                _ => {
                    let last = g.body.len() - 1;
                    g.body[last] ^= 1;
                }
            }
            assert!(!g.verify(NET, studio.public()), "change {change}: the signature");
            assert!(open_frame(NET, &laptop, &g).is_err(), "change {change}: the sealing");
        }
        let mut h = f.clone();
        h.sig[0] ^= 1;
        assert!(!h.verify(NET, studio.public()));
    }

    #[test]
    fn gateway_frames_are_signed_and_open() {
        let gw = HostKey::generate().unwrap();
        let d = Draft { kind: "ping", from: "studio", to: "laptop", ..draft(&crate::codec::frame_id().unwrap()) };
        let f = gateway_frame(&d, &gw, &json!({ "now": 5 }));
        assert!(f.verify(NET, gw.public()) && !f.sealed);
        assert_eq!(f.body_json().unwrap(), json!({ "now": 5 }));
        assert!(open_frame(NET, &gw, &f).is_err());
    }

    #[test]
    fn frames_are_checked_for_their_shape() {
        let good = json!({ "v": 1, "id": "frm_00000000-0000-4000-8000-000000000000", "kind": "ping", "from": "studio", "to": "laptop", "at": 1, "sealed": false, "body": "", "sig": b64(&[0u8; 64]) });
        assert!(Frame::parse(&good).is_ok());
        for (k, v) in [
            ("v", json!(2)),
            ("id", json!("frm_x")),
            ("kind", json!("hack")),
            ("from", json!("Studio")),
            ("at", json!(1.5)),
            ("body", json!("a+b")),
            ("sig", json!("AA")),
            ("seq", json!(-1)),
        ] {
            let mut bad = good.clone();
            bad[k] = v;
            assert!(Frame::parse(&bad).is_err(), "{k}");
        }
    }
}
