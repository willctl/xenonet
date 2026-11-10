//! What proves the person: their device's signature over one act, and for
//! the acts that need them there, a presence block for the same act. Both
//! are checked here against the device key the receiver's own copy of the
//! person's chain gives, and each keeps the hash of the act it was checked
//! for, so a decision can tell that it's this request's. They can't be
//! built any other way, which is what makes `Principal::Person` mean the
//! person.

use crate::vocab::{act_hash, Act, DeviceId, PersonId, Principal, Request, Resource};
use crate::{AuthzError, DeviceKind, Ground, R};
use xenonet_id::presence::{PresenceBlock, PresenceCheck};
use xenonet_id::sign::{Purpose, Signer};

/// The person, proven by their device's signature over one act.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonSigned {
    person: PersonId,
    device: DeviceId,
    act: [u8; 32],
}

impl PersonSigned {
    /// Checks `sig`, `device`'s signature over the act done to `resource`,
    /// against the key the person's chain holds for that device.
    pub fn verify(ground: &dyn Ground, person: &PersonId, device: &DeviceId, resource: &Resource, act: &Act, sig: &[u8]) -> R<PersonSigned> {
        let key = ground.device(person, device).ok_or_else(|| AuthzError::new("person", format!("{device} isn't one of {person}'s devices, or it was revoked")))?.key;
        let hash = act_hash(resource, act);
        if key.id() != device.as_str() || !key.verify(Purpose::ACT, &hash, sig) {
            return Err(AuthzError::new("person", format!("that isn't {device}'s signature over this act")));
        }
        Ok(PersonSigned { person: person.clone(), device: device.clone(), act: hash })
    }

    /// The device's side: its signature over the act done to `resource`.
    pub fn sign(device: &dyn Signer, resource: &Resource, act: &Act) -> R<Vec<u8>> {
        Ok(device.sign(Purpose::ACT, &act_hash(resource, act))?)
    }

    pub fn person(&self) -> &PersonId {
        &self.person
    }

    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    pub(crate) fn act(&self) -> &[u8; 32] {
        &self.act
    }
}

/// The person was at their device for this one act: a presence block
/// taken through `PresenceCheck::accept`, so once, within a minute, for the
/// act's hash, from the device that signed the act.
#[derive(Debug)]
pub struct VerifiedPresence {
    person: PersonId,
    device: DeviceId,
    kind: DeviceKind,
    act: [u8; 32],
}

impl VerifiedPresence {
    /// Takes `block` for `request`, whose principal must be the person.
    /// `seen` is the receiver's memory of blocks already taken.
    pub fn check(seen: &mut PresenceCheck, block: &PresenceBlock, ground: &dyn Ground, request: &Request) -> R<VerifiedPresence> {
        let Principal::Person(signed) = request.principal() else {
            return Err(AuthzError::new("presence", "only the person is proven present"));
        };
        let what = request.act().presence_what().ok_or_else(|| AuthzError::new("presence", format!("{} doesn't take a presence", request.action().as_str())))?;
        if block.device != signed.device().as_str() {
            return Err(AuthzError::new("presence", "that presence is from another device than the one that signed the act"));
        }
        let device = ground
            .device(signed.person(), signed.device())
            .ok_or_else(|| AuthzError::new("presence", format!("{} isn't one of the person's devices, or it was revoked", signed.device())))?;
        let hash = request.act_hash();
        seen.accept(block, &device.key, what, &hash, request.at())?;
        Ok(VerifiedPresence { person: signed.person().clone(), device: signed.device().clone(), kind: device.kind, act: hash })
    }

    pub fn person(&self) -> &PersonId {
        &self.person
    }

    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    pub fn kind(&self) -> DeviceKind {
        self.kind
    }

    pub(crate) fn act(&self) -> &[u8; 32] {
        &self.act
    }
}
