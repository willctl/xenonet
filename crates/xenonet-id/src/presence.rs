//! A person, proven present. A `PresenceBlock` is a device's signature
//! that the person was there, for one act. How strong it is depends on the
//! device key, which the person's chain records when the device is added:
//!
//! - a key that asks the person before each signature (a Secure Enclave
//!   key made with biometry, `Signer::asks_person`): the signature is the
//!   proof, since nothing signs with that key unless the person answers
//!   Touch ID. An agent that holds the key, or can type, can't pass it.
//! - any other key, which the service holds: it signs only after a
//!   `UserCheck` passed (polkit or a passphrase on Linux). That is weaker,
//!   since another program of the account can use the key without asking,
//!   and the chain says so.
//!
//! A block names the one act it approves, by the act's hash, with what it
//! is for (`answer`, `admit`, `widen`, `publish`, `revoke`, `pair`), a
//! nonce and the time, all under the device's signature. So it can't be
//! lifted off one act and put on another. A receiver takes it once
//! (`PresenceCheck`), and not when it's more than 60 seconds old by the
//! clock it was given. A chain keeps blocks with its entries: there they
//! are checked for what they sign, not for their age, since a machine
//! catching up on a chain reads old entries that were fresh when made.

use crate::codec::{b64, random, unb64, unb64_n};
use crate::sign::{Purpose, Signer, VerifyingKey};
use crate::{IdError, R};
use serde_json::{json, Value};
use std::collections::HashMap;

/// How long after it's made a receiver still takes a block, either way.
pub const FRESH_MS: i64 = 60_000;
/// How long a receiver remembers a block's nonce.
pub const SEEN_MS: i64 = 2 * FRESH_MS;

/// The OS asking the person, for a device key that doesn't ask by itself.
/// `confirm` shows the prompt with `reason` and returns once they
/// answered: refused when they declined or there's nobody to ask.
pub trait UserCheck: Send + Sync {
    fn confirm(&self, reason: &str) -> R<()>;
}

/// A check that answers as it was told, never prompting: for tests, and
/// for an agent's process, where no prompt may show.
pub struct FixedCheck(pub bool);

impl UserCheck for FixedCheck {
    fn confirm(&self, _reason: &str) -> R<()> {
        if self.0 {
            Ok(())
        } else {
            Err(IdError::refused("nobody here can be asked"))
        }
    }
}

/// A device's signed word that the person was there, for one act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceBlock {
    /// The device key's id (`xns1_…`).
    pub device: String,
    /// What it approves: `answer`, `admit`, `widen`, `publish`, `revoke`, `pair`.
    pub what: String,
    /// The SHA-256 of the act.
    pub act: [u8; 32],
    pub nonce: [u8; 16],
    /// Milliseconds since 1970, by the device's clock.
    pub at: i64,
    pub sig: Vec<u8>,
}

/// The longest `what` a block carries.
const WHAT_MAX: usize = 32;

impl PresenceBlock {
    /// What the device signs: device, what, act, nonce and time, one a
    /// line. Public so a known-answer file can be written with a fixed
    /// nonce; `prove` is how a block is made.
    pub fn signed_bytes(&self) -> Vec<u8> {
        format!("{}\n{}\n{}\n{}\n{}", self.device, self.what, b64(&self.act), b64(&self.nonce), self.at).into_bytes()
    }

    /// Whether `device` signed this block, for `what` and the act whose
    /// hash is `act`. Its age isn't looked at.
    pub fn signed_for(&self, device: &VerifyingKey, what: &str, act: &[u8; 32]) -> R<()> {
        if self.device != device.id() {
            return Err(IdError::refused("that presence is another device's"));
        }
        if self.what != what {
            return Err(IdError::refused(format!("that presence approves {}, not {what}", self.what)));
        }
        if &self.act != act {
            return Err(IdError::refused("that presence approves another act"));
        }
        if !device.verify(Purpose::PRESENCE, &self.signed_bytes(), &self.sig) {
            return Err(IdError::refused("that presence's signature doesn't check"));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "device": self.device,
            "what": self.what,
            "act": b64(&self.act),
            "nonce": b64(&self.nonce),
            "at": self.at,
            "sig": b64(&self.sig),
        })
    }

    pub fn parse(v: &Value) -> Option<PresenceBlock> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let device = text("device").filter(|d| d.starts_with("xns1_") && d.len() == 37)?;
        let what = text("what").filter(|w| !w.is_empty() && w.len() <= WHAT_MAX && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'))?;
        Some(PresenceBlock {
            device: device.into(),
            what: what.into(),
            act: unb64_n(text("act")?)?,
            nonce: unb64_n(text("nonce")?)?,
            at: v.get("at")?.as_i64()?,
            sig: unb64(text("sig")?).filter(|s| (64..=72).contains(&s.len()))?,
        })
    }
}

/// Has `device` sign a block for `what` and the act whose hash is `act`.
/// A key that asks the person by itself does so as it signs; any other
/// signs only after `check` passed, and with no check it doesn't sign.
pub fn prove(device: &dyn Signer, check: Option<&dyn UserCheck>, what: &str, act: &[u8; 32], reason: &str, now: i64) -> R<PresenceBlock> {
    if what.is_empty() || what.len() > WHAT_MAX || !what.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        return Err(IdError::internal(format!("{what} isn't something a presence approves")));
    }
    if !device.asks_person() {
        check.ok_or_else(|| IdError::refused("this device can't ask the person"))?.confirm(reason)?;
    }
    let mut block = PresenceBlock { device: device.public().id(), what: what.into(), act: *act, nonce: random::<16>()?, at: now, sig: Vec::new() };
    block.sig = device.sign(Purpose::PRESENCE, &block.signed_bytes())?;
    Ok(block)
}

/// A receiver's memory of the blocks it took, so each counts once. The
/// service keeps it across restarts (`seen`, `from_seen`), so a block
/// can't count again after one.
#[derive(Debug, Default)]
pub struct PresenceCheck {
    seen: HashMap<(String, [u8; 16]), i64>,
}

impl PresenceCheck {
    pub fn new() -> PresenceCheck {
        PresenceCheck::default()
    }

    /// The blocks taken in the last two minutes: device, nonce, when.
    pub fn seen(&self) -> Vec<(String, [u8; 16], i64)> {
        self.seen.iter().map(|((d, n), t)| (d.clone(), *n, *t)).collect()
    }

    /// One that remembers what `seen` said.
    pub fn from_seen(seen: impl IntoIterator<Item = (String, [u8; 16], i64)>) -> PresenceCheck {
        PresenceCheck { seen: seen.into_iter().map(|(d, n, t)| ((d, n), t)).collect() }
    }

    /// Takes `block` for a live act: `device` signed it for `what` and
    /// `act`, it's under a minute old either way by this machine's clock,
    /// and it wasn't taken before.
    pub fn accept(&mut self, block: &PresenceBlock, device: &VerifyingKey, what: &str, act: &[u8; 32], now: i64) -> R<()> {
        block.signed_for(device, what, act)?;
        if (now - block.at).abs() > FRESH_MS {
            return Err(IdError::refused("that presence is more than a minute old"));
        }
        self.seen.retain(|_, t| now - *t < SEEN_MS);
        if self.seen.insert((block.device.clone(), block.nonce), now).is_some() {
            return Err(IdError::refused("that presence was taken before"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::sha256;
    use crate::sign::{KeyHome, SeedSigner, VerifyingKey};

    fn device(n: u8) -> SeedSigner {
        SeedSigner::from_seed(&[n; 32], KeyHome::Keychain).unwrap()
    }

    /// A key that asks the person as it signs, as a Secure Enclave key with
    /// biometry does; here it signs whenever asked.
    struct Asking(SeedSigner);

    impl Signer for Asking {
        fn public(&self) -> VerifyingKey {
            self.0.public()
        }
        fn home(&self) -> KeyHome {
            KeyHome::Hardware
        }
        fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
            self.0.sign(purpose, msg)
        }
        fn asks_person(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_block_approves_the_one_act_it_names_once_and_for_a_minute() {
        let d = device(1);
        let act = sha256(b"answer ask_1 allow");
        let block = prove(&d, Some(&FixedCheck(true)), "answer", &act, "Allow Bash in atlas?", 1_000_000).unwrap();
        let mut check = PresenceCheck::new();
        // Lifted onto another act, or another purpose: refused.
        let other = sha256(b"answer ask_1 deny");
        assert!(check.accept(&block, &d.public(), "answer", &other, 1_000_500).is_err());
        assert!(check.accept(&block, &d.public(), "admit", &act, 1_000_500).is_err());
        // Another device's key doesn't take it.
        assert!(check.accept(&block, &device(2).public(), "answer", &act, 1_000_500).is_err());
        // Too old, or from too far ahead.
        assert!(check.accept(&block, &d.public(), "answer", &act, 1_000_000 + FRESH_MS + 1).is_err());
        assert!(check.accept(&block, &d.public(), "answer", &act, 1_000_000 - FRESH_MS - 1).is_err());
        // Once, then never again, even after a restart.
        check.accept(&block, &d.public(), "answer", &act, 1_000_500).unwrap();
        let again = check.accept(&block, &d.public(), "answer", &act, 1_000_600).unwrap_err();
        assert!(again.message.contains("before"), "{again}");
        let mut restarted = PresenceCheck::from_seen(check.seen());
        assert!(restarted.accept(&block, &d.public(), "answer", &act, 1_000_700).is_err());
    }

    #[test]
    fn a_changed_block_doesnt_check() {
        let d = device(1);
        let act = sha256(b"admit laptop");
        let block = prove(&d, Some(&FixedCheck(true)), "admit", &act, "Add laptop?", 5).unwrap();
        let mut later = block.clone();
        later.at += 30_000;
        assert!(later.signed_for(&d.public(), "admit", &act).is_err(), "a new time");
        let mut other = block.clone();
        other.nonce = [9; 16];
        assert!(other.signed_for(&d.public(), "admit", &act).is_err(), "a new nonce");
        assert_eq!(PresenceBlock::parse(&block.to_json()), Some(block.clone()));
        // A chain's replay checks what it signs, not its age.
        block.signed_for(&d.public(), "admit", &act).unwrap();
    }

    #[test]
    fn a_key_that_doesnt_ask_signs_only_after_the_check() {
        let d = device(1);
        let act = sha256(b"x");
        assert!(prove(&d, Some(&FixedCheck(false)), "widen", &act, "", 5).is_err(), "declined");
        assert!(prove(&d, None, "widen", &act, "", 5).is_err(), "nobody to ask");
        assert!(prove(&d, Some(&FixedCheck(true)), "Widen!", &act, "", 5).is_err(), "not a purpose");
        // A key that asks by itself needs no check of its own.
        let asking = Asking(device(3));
        let block = prove(&asking, None, "widen", &act, "", 5).unwrap();
        block.signed_for(&asking.public(), "widen", &act).unwrap();
    }
}
