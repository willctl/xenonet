//! Where a machine's own keys come from, as the composition root gives
//! them (`xenonet-keys` implements it for the machine XenoNet runs on, and
//! fakes of it): the machine signing key, the person's device key with the
//! check it needs, and sealing a secret (the person's root seed) to the
//! device key. Everything above sees this trait, never a platform.
//!
//! Nothing here prompts on its own: making or loading a key never shows
//! anything. A biometric device key asks for each signature and each
//! `open`, and a `UserCheck` asks in `confirm`, so only an act the person
//! started at XenoNet's window should call them.

use crate::presence::UserCheck;
use crate::sign::{Signer, VerifyingKey};
use crate::R;
use std::path::Path;

/// The person's key on this device, with the check it needs before it
/// signs (none for a key that asks by itself).
pub struct DeviceKey {
    pub signer: Box<dyn Signer>,
    pub check: Option<Box<dyn UserCheck>>,
    /// Whether the key asks for a biometric before each signature; else
    /// it signs after a weaker check (polkit, a passphrase).
    pub biometric: bool,
}

pub trait Vault: Send + Sync {
    /// The machine signing key in `dir`, made there the first time.
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>>;
    /// The person's device key in `dir`, made the first time where this
    /// device can ask the person; None where it can't.
    fn device(&self, dir: &Path) -> R<Option<DeviceKey>>;
    /// `secret` sealed to the device key `to`, where this platform can seal
    /// to it so that opening it asks the person; None where it can't, and
    /// the secret is kept in an owner-only file instead, which is weaker.
    fn seal(&self, to: &VerifyingKey, secret: &[u8]) -> R<Option<Vec<u8>>>;
    /// What `seal` sealed to the device key in `dir`; asks the person.
    fn open(&self, dir: &Path, sealed: &[u8], reason: &str) -> R<Vec<u8>>;
    /// Runs `f` with `reason` as what a key that asks the person shows for
    /// the signatures `f` makes (it completes "xenonet is trying to …").
    fn asking(&self, reason: &str, f: &mut dyn FnMut());
}
