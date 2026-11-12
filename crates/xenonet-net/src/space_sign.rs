//! The person's device signing a space entry a curator drafted (phase 6,
//! revisions 7 and 21): Xenolith's `spaces draft`, signed at XenoNet's
//! window. The window asks for one act (`Asked`); the curator's draft
//! comes back from another program, so it's untrusted: it's read back as
//! an entry, and signed only when it does exactly that act to that space,
//! and its act hash is the one it says. What widens (moving a space to the
//! person, sharing it into a network) takes the person's presence for that
//! act hash; an unshare narrows, and takes their device's signature alone.

use crate::identity::Identity;
use crate::{NetError, R};
use serde_json::{json, Value};
use xenonet_id::codec::{b64, unb64_n};
use xenonet_id::sign::Purpose;
use xenonet_log::chain::Signature;
pub use xenonet_log::space::Level;
use xenonet_log::space::{Entry, Op, Side, SideOp, Subject, WIDEN};

/// What the person asked for at the window, of one space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Asked {
    /// The space goes from the standalone owner key to the person here.
    MoveOwner,
    /// The space is shared into the network at the level.
    Share { network: String, level: Level },
    /// The space stops being shared into the network.
    Unshare { network: String },
}

impl Asked {
    /// Whether it widens what anyone may do: it then takes presence.
    pub fn widens(&self) -> bool {
        !matches!(self, Asked::Unshare { .. })
    }

    /// What the window says it does.
    pub fn words(&self, space: &str) -> String {
        match self {
            Asked::MoveOwner => format!("make the space {space} yours"),
            Asked::Share { network, level } => format!("share the space {space} into {network} ({})", level.as_str()),
            Asked::Unshare { network } => format!("stop sharing the space {space} into {network}"),
        }
    }
}

fn refused(words: impl Into<String>) -> NetError {
    NetError::new(409, "not-that-draft", words)
}

// The placeholder signature a draft is read back with before the device
// signs it: reading checks a signature's shape, never its value.
fn placeholder(by: &str) -> Value {
    json!({ "key": by, "sig": b64(&[0; 64]) })
}

fn with_placeholder(draft: &Value, by: &str) -> R<Value> {
    let mut d = draft.as_object().cloned().ok_or_else(|| refused("a draft is an entry's JSON"))?;
    let mut signers = d.remove("signers").and_then(|s| s.as_array().cloned()).unwrap_or_default();
    signers.retain(|s| s.get("key").and_then(Value::as_str) != Some(by));
    signers.push(placeholder(by));
    d.insert("signers".into(), Value::Array(signers));
    Ok(Value::Object(d))
}

// The JSON signed, without the purpose's tag.
fn unsigned_of(signing_bytes: &[u8]) -> R<Vec<u8>> {
    let tag = Purpose::SPACE.bytes(b"");
    signing_bytes.strip_prefix(tag.as_slice()).map(<[u8]>::to_vec).ok_or_else(|| NetError::internal("a space entry's bytes start with its tag"))
}

/// Signs `draft` (`{entry, act}` as the curator drafted it, or
/// `{side, act}` for an unshare) as the person, for `asked` of `space`:
/// refused unless it does exactly that, with the person here the one a
/// move names. Only from the window (`at_window`), where the person is
/// asked: their presence for what widens. The signed entry or side, for
/// the curator to sequence.
pub fn sign(identity: &Identity, at_window: bool, space: &str, asked: &Asked, draft: &Value, now: i64) -> R<Value> {
    if !at_window {
        return Err(crate::identity::not_at_window("signing a space's entry as you"));
    }
    let person = identity.person_id().ok_or_else(|| NetError::new(409, "no-person", "this machine has no identity of yours yet"))?;
    let device = identity.device_id().ok_or_else(|| NetError::new(409, "no-device", "this machine has no device key of yours"))?;
    let act: [u8; 32] = draft.get("act").and_then(Value::as_str).and_then(unb64_n).ok_or_else(|| refused("the draft names no act"))?;
    let reason = asked.words(space);
    match asked {
        Asked::Unshare { network } => {
            let raw = draft.get("side").ok_or_else(|| refused("an unshare is a side entry"))?;
            let mut s = Side::parse(&with_placeholder(raw, &device)?).map_err(|e| refused(e.message))?;
            let want = Subject::Network(network.clone());
            if s.space != space || !matches!(&s.op, SideOp::Unshare { subject } if *subject == want) {
                return Err(refused(format!("that draft isn't one to {reason}")));
            }
            if s.digest() != act {
                return Err(refused("the draft's act isn't its entry's"));
            }
            let (by, sig) = identity.sign_as_person(at_window, Purpose::SPACE, &unsigned_of(&s.signing_bytes())?, &reason)?;
            s.signers.retain(|x| x.by != by);
            s.signers.push(Signature { by, sig, presence: None });
            Ok(s.to_json())
        }
        _ => {
            let raw = draft.get("entry").ok_or_else(|| refused("the draft has no entry"))?;
            let mut e = Entry::parse(&with_placeholder(raw, &device)?).map_err(|e| refused(e.message))?;
            let matches = match (asked, &e.op) {
                (Asked::MoveOwner, Op::MoveOwner { to }) => *to == person,
                (Asked::Share { network, level }, Op::Share { subject, level: l, endpoint: None, .. }) => *subject == Subject::Network(network.clone()) && l == level,
                _ => false,
            };
            if e.space != space || !matches {
                return Err(refused(format!("that draft isn't one to {reason}")));
            }
            if e.digest() != act {
                return Err(refused("the draft's act isn't its entry's"));
            }
            let presence = Some(identity.prove(at_window, WIDEN, &act, &reason, now)?);
            let (by, sig) = identity.sign_as_person(at_window, Purpose::SPACE, &unsigned_of(&e.signing_bytes())?, &reason)?;
            e.add_signature(&by, sig, presence);
            Ok(e.to_json())
        }
    }
}

#[cfg(test)]
#[path = "space_sign_tests.rs"]
mod tests;
