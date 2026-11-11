//! Where this machine's keys live, and how it asks the person
//! (docs/design/direction.md, "Authorization" and "The machine key"). Only
//! the composition root uses this crate: everything else sees a
//! `xenonet_id::sign::Signer` and a `xenonet_id::presence::UserCheck`.
//!
//! - macOS (`macos`): a Mac with a Secure Enclave keeps the machine signing
//!   key there without biometry, and the person's device key there with
//!   Touch ID, which asks for every signature. The person's root seed is
//!   sealed to the device key (`macos::wrap`, `macos::unwrap`). Presence is
//!   biometric only, with no passphrase or PIN instead.
//! - Linux and the BSDs (`linux`): file keys, and a `UserCheck` that asks
//!   polkit, or a passphrase on the controlling terminal. Weaker, and said so.
//! - Windows (`windows`, `tpm`): the machine signing key in the TPM where
//!   there is one, non-exportable; the person's device key is a file key,
//!   with a `UserCheck` that asks Windows Hello. Hello always lets a PIN
//!   stand in for a face or a finger, which an agent that can type could
//!   enter, so it's weaker, like Linux, and said so.
//!
//! A machine signing key in a secure element is never replaced: one that
//! can't be opened is an error, and a file key stands in only where none
//! was made (`machine`).
//!
//! `System` picks these for the machine it runs on; `Fake` stands in for
//! demo and test instances, and never prompts or touches the hardware.
//!
//! Nothing here prompts on its own: making a key, loading one and asking
//! what's available never show anything. Only a biometric key's signature
//! or unwrap asks Touch ID, and only a `UserCheck`'s `confirm` asks polkit
//! or the terminal, so only a route where the person acts should call them.

mod ecdsa;
#[cfg(unix)]
pub mod linux;
pub mod machine;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(windows)]
pub mod tpm;
#[cfg(windows)]
pub mod windows;

use serde_json::Value;
use std::cell::RefCell;
use std::path::Path;
use xenonet_id::presence::{FixedCheck, UserCheck};
use xenonet_id::sign::{Purpose, SeedSigner, Signer, VerifyingKey};
use xenonet_id::store::MemoryKeyStore;
use xenonet_id::vault::{DeviceKey, Vault};
use xenonet_id::{IdError, R};

/// The machine signing key's file in a machine's key folder.
pub const MACHINE_FILE: &str = "machine-sign.key";
/// The person's device key's file.
pub const DEVICE_FILE: &str = "device-sign.key";
/// The passphrase's hash, where a passphrase is the check.
pub const PASSPHRASE_FILE: &str = "presence-passphrase.json";

/// How this device can show the person is there, as the doctor and the
/// person's chain (a device entry's kind) name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Presence {
    /// The key asks Touch ID for every signature.
    Biometric,
    /// A check before a file key signs: polkit, a passphrase or Windows
    /// Hello (which takes a PIN).
    Weaker,
    /// No way to ask the person here.
    None,
}

impl Presence {
    pub fn as_str(self) -> &'static str {
        match self {
            Presence::Biometric => "biometric",
            Presence::Weaker => "weaker",
            Presence::None => "none",
        }
    }
}

/// The person's key at this device, with the check it needs before it
/// signs (none for a key that asks by itself) and its kind.
pub struct Device {
    pub signer: Box<dyn Signer>,
    pub check: Option<Box<dyn UserCheck>>,
    pub kind: Presence,
}

/// The keys of the machine XenoNet runs on, or fakes of them.
pub trait Keys: Send + Sync {
    /// The machine signing key in `dir`, made there the first time.
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>>;
    /// The person's device key in `dir`, made the first time where this
    /// device can ask the person; None where it can't.
    fn device(&self, dir: &Path) -> R<Option<Device>>;
    /// The check a key that doesn't ask needs, where there is one.
    fn user_check(&self, dir: &Path) -> Option<Box<dyn UserCheck>>;
    /// What this device can do to show the person is there.
    fn available(&self, dir: &Path) -> Presence;
}

/// The machine's own keys.
pub struct System;

impl Keys for System {
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>> {
        machine_signer(dir)
    }
    fn device(&self, dir: &Path) -> R<Option<Device>> {
        device(dir)
    }
    fn user_check(&self, dir: &Path) -> Option<Box<dyn UserCheck>> {
        user_check(dir)
    }
    fn available(&self, dir: &Path) -> Presence {
        available(dir)
    }
}

/// Keys for a demo or test instance: file keys in its own folder, and a
/// check that answers `allow` without asking anyone. Nothing prompts.
pub struct Fake {
    pub allow: bool,
}

impl Fake {
    /// A key store in memory, where the instance keeps any other secret.
    pub fn key_store() -> MemoryKeyStore {
        MemoryKeyStore::new()
    }

    /// The doctor's row for a fake's machine signing key in `dir`, which
    /// is a file key: None before it's made. It never asks the hardware.
    pub fn machine_standing(dir: &Path) -> Option<Value> {
        machine::standing(dir, None)
    }
}

impl Keys for Fake {
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>> {
        Ok(Box::new(SeedSigner::load_or_create(&dir.join(MACHINE_FILE))?))
    }
    fn device(&self, dir: &Path) -> R<Option<Device>> {
        let signer = SeedSigner::load_or_create(&dir.join(DEVICE_FILE))?;
        Ok(Some(Device { signer: Box::new(signer), check: self.user_check(dir), kind: Presence::Weaker }))
    }
    fn user_check(&self, _dir: &Path) -> Option<Box<dyn UserCheck>> {
        Some(Box::new(FixedCheck(self.allow)))
    }
    fn available(&self, _dir: &Path) -> Presence {
        Presence::Weaker
    }
}

// Both kinds of keys are a `Vault` too, which is how the rest of XenoNet
// takes them: it sees the trait, never this crate.
impl Vault for System {
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>> {
        machine_signer(dir)
    }
    fn device(&self, dir: &Path) -> R<Option<DeviceKey>> {
        Ok(device(dir)?.map(Device::into_key))
    }
    fn seal(&self, to: &VerifyingKey, secret: &[u8]) -> R<Option<Vec<u8>>> {
        #[cfg(target_os = "macos")]
        if matches!(to, VerifyingKey::P256(_)) {
            return macos::wrap(to, secret).map(Some);
        }
        let _ = (to, secret);
        Ok(None)
    }
    fn open(&self, dir: &Path, sealed: &[u8], reason: &str) -> R<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            let device = macos::SecureEnclaveSigner::load(&dir.join(DEVICE_FILE))?;
            device.unwrap_with_reason(sealed, reason)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (dir, sealed, reason);
            Err(IdError::internal("only a Mac seals the person's key to its device key"))
        }
    }
    fn asking(&self, reason: &str, f: &mut dyn FnMut()) {
        with_reason(reason, f);
    }
}

impl Vault for Fake {
    fn machine_signer(&self, dir: &Path) -> R<Box<dyn Signer>> {
        Keys::machine_signer(self, dir)
    }
    fn device(&self, dir: &Path) -> R<Option<DeviceKey>> {
        Ok(Keys::device(self, dir)?.map(Device::into_key))
    }
    /// A fake seals nothing: the seed goes in an owner-only file, as on a
    /// machine with no secure element.
    fn seal(&self, _to: &VerifyingKey, _secret: &[u8]) -> R<Option<Vec<u8>>> {
        Ok(None)
    }
    fn open(&self, _dir: &Path, _sealed: &[u8], _reason: &str) -> R<Vec<u8>> {
        Err(IdError::internal("a fake's keys seal nothing"))
    }
    fn asking(&self, _reason: &str, f: &mut dyn FnMut()) {
        f();
    }
}

impl Device {
    fn into_key(self) -> DeviceKey {
        DeviceKey { signer: self.signer, check: self.check, biometric: self.kind == Presence::Biometric }
    }
}

/// What a key file in `file` holds, by its fields, or None when there's no
/// file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    /// A Secure Enclave key's blob.
    Enclave,
    /// A TPM key's name and public key (`machine`).
    Tpm,
    /// An Ed25519 seed (`SeedSigner`).
    Seed,
}

fn held(file: &Path) -> R<Option<Held>> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let v: Value = serde_json::from_str(&text).map_err(|_| IdError::internal(format!("{} isn't readable", file.display())))?;
    if v["kind"].as_str() == Some("secure-enclave") {
        Ok(Some(Held::Enclave))
    } else if v["kind"].as_str() == Some(machine::TPM) {
        Ok(Some(Held::Tpm))
    } else if v["seed"].is_string() {
        Ok(Some(Held::Seed))
    } else {
        Err(IdError::internal(format!("{} isn't a key XenoNet knows", file.display())))
    }
}

/// A Secure Enclave key's file, loaded where there's a Secure Enclave.
fn enclave(file: &Path) -> R<Box<dyn Signer>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(macos::SecureEnclaveSigner::load(file)?))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(IdError::internal(format!("{} is a key in a Mac's Secure Enclave, which only that Mac can use", file.display())))
    }
}

/// The machine signing key in `dir`: in the Secure Enclave on a Mac that
/// has one, in the TPM on Windows where there is one, else a file key, so
/// that its home (`KeyHome::Hardware` or `KeyHome::File`) says where it
/// really is. A key made before stays what it was: one the Secure Enclave
/// or the TPM can't open any more is an error, never silently replaced,
/// since the machine's identity rests on it (`machine`).
pub fn machine_signer(dir: &Path) -> R<Box<dyn Signer>> {
    #[cfg(target_os = "macos")]
    {
        let file = dir.join(MACHINE_FILE);
        if held(&file)?.is_none() && macos::has_secure_enclave() {
            return Ok(Box::new(macos::SecureEnclaveSigner::create(&file, false)?));
        }
    }
    machine::signer(dir, ELEMENT)
}

/// The doctor's row for the machine signing key in `dir`: where it is and,
/// for a key in hardware, that it opens and how long a signature takes;
/// None before the key is made. Nothing prompts.
pub fn machine_standing(dir: &Path) -> Option<Value> {
    machine::standing(dir, ELEMENT)
}

/// The secure element that keeps this machine's keys by name: the TPM on
/// Windows, none elsewhere.
#[cfg(windows)]
const ELEMENT: Option<&dyn machine::NamedKeys> = Some(&tpm::Tpm);
#[cfg(not(windows))]
const ELEMENT: Option<&dyn machine::NamedKeys> = None;

/// The person's device key in `dir`.
///
/// - macOS: a Secure Enclave key with Touch ID, made the first time on a
///   Mac with a Secure Enclave and Touch ID enrolled and usable (asked
///   without prompting); None on a Mac without them, since presence there
///   is biometric only. A key made before loads whatever Touch ID's state
///   now, and asks when it's used.
/// - Linux and the BSDs: a file key with `user_check`'s check, made the
///   first time there is a check. The check may be missing later (pkcheck
///   gone): then `presence::prove` refuses, as it should.
/// - Windows: a file key with Windows Hello's check, made the first time
///   Hello is set up here.
pub fn device(dir: &Path) -> R<Option<Device>> {
    let file = dir.join(DEVICE_FILE);
    #[cfg(target_os = "macos")]
    {
        let signer = match held(&file)? {
            Some(Held::Enclave) => macos::SecureEnclaveSigner::load(&file)?,
            Some(Held::Seed | Held::Tpm) => {
                return Err(IdError::internal(format!("{} isn't a Secure Enclave key, and a Mac's device key is in its Secure Enclave", file.display())))
            }
            None if macos::has_secure_enclave() && macos::has_touch_id() => macos::SecureEnclaveSigner::create(&file, true)?,
            None => return Ok(None),
        };
        if !signer.is_biometric() {
            return Err(IdError::internal(format!("{} doesn't ask Touch ID, and a Mac's device key must", file.display())));
        }
        Ok(Some(Device { signer: Box::new(signer), check: None, kind: Presence::Biometric }))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let check = user_check(dir);
        if check.is_none() && held(&file)?.is_none() {
            return Ok(None);
        }
        let signer = SeedSigner::load_or_create(&file)?;
        Ok(Some(Device { signer: Box::new(signer), check, kind: Presence::Weaker }))
    }
    #[cfg(windows)]
    {
        let check = user_check(dir);
        if check.is_none() && held(&file)?.is_none() {
            return Ok(None);
        }
        let signer = SeedSigner::load_or_create(&file)?;
        Ok(Some(Device { signer: Box::new(signer), check, kind: Presence::Weaker }))
    }
}

/// The check a key that doesn't ask needs before it signs. Linux and the
/// BSDs: polkit, asked about this process, with the passphrase in `dir` as
/// the fallback when polkit can't ask anyone; the passphrase alone without
/// pkcheck. A surface that acts for the person in another process (the
/// window) should ask about that process instead, since polkit asks
/// through its session: `linux::PolkitCheck::for_process`. Windows:
/// Windows Hello, where it's set up. macOS: None (its device key asks by
/// itself).
pub fn user_check(dir: &Path) -> Option<Box<dyn UserCheck>> {
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let file = dir.join(PASSPHRASE_FILE);
        let passphrase = || linux::PassphraseCheck::load(&file).ok().flatten();
        match linux::PolkitCheck::for_process(std::process::id(), passphrase()) {
            Ok(Some(check)) => Some(Box::new(check)),
            // No pkcheck, or no /proc to name this process by.
            Ok(None) | Err(_) => passphrase().map(|p| Box::new(p) as Box<dyn UserCheck>),
        }
    }
    #[cfg(windows)]
    {
        let _ = dir;
        windows::HelloCheck::available().map(|c| Box::new(c) as Box<dyn UserCheck>)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = dir;
        None
    }
}

/// What this device can do to show the person is there, asked without
/// prompting, for the doctor. On Linux, `Weaker` means a check exists, not
/// that polkit has an agent to ask with right now.
pub fn available(dir: &Path) -> Presence {
    #[cfg(target_os = "macos")]
    {
        let _ = dir;
        if macos::has_secure_enclave() && macos::has_touch_id() {
            Presence::Biometric
        } else {
            Presence::None
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if user_check(dir).is_some() {
            Presence::Weaker
        } else {
            Presence::None
        }
    }
}

thread_local! {
    static REASON: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Runs `f` with `reason` as what a key that asks the person shows, for
/// every signature and unwrap `f` makes on this thread. `reason` completes
/// "xenonet is trying to …", so it names the act: "answer Bash's request
/// in atlas". For a key that doesn't ask, `presence::prove` passes its own
/// reason to the check.
///
/// ```text
/// let block = with_reason("allow Bash in atlas", || presence::prove(device, None, "answer", &act, "allow Bash in atlas", now))?;
/// ```
pub fn with_reason<T>(reason: &str, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0.take();
            REASON.with(|r| *r.borrow_mut() = before);
        }
    }
    let _restore = Restore(REASON.with(|r| r.replace(Some(reason.to_owned()))));
    f()
}

/// The reason `with_reason` set on this thread.
#[cfg_attr(all(not(target_os = "macos"), not(test)), expect(dead_code, reason = "only macOS's keys ask the person themselves"))]
fn current_reason() -> Option<String> {
    REASON.with(|r| r.borrow().clone())
}

/// What a key that asks shows when nobody set a reason: the act, from what
/// is signed.
pub fn default_reason(purpose: Purpose, msg: &[u8]) -> String {
    let reason = if purpose == Purpose::PRESENCE {
        // A presence block's message: device, what, act, nonce, time.
        match std::str::from_utf8(msg).ok().and_then(|m| m.lines().nth(1)) {
            Some("answer") => "answer a permission request in XenoNet",
            Some("admit") => "add a machine to your XenoNet network",
            Some("widen") => "widen what a XenoNet network can reach",
            Some("publish") => "publish from XenoNet",
            Some("revoke") => "revoke a XenoNet key",
            Some("pair") => "pair a new device with XenoNet",
            _ => "confirm an act in XenoNet",
        }
    } else if purpose == Purpose::PERSON {
        "change your XenoNet identity"
    } else if purpose == Purpose::ACT {
        "act as you in XenoNet"
    } else {
        "sign as you in XenoNet"
    };
    reason.to_owned()
}

/// What unwrapping the person's root seed shows.
pub const UNWRAP_REASON: &str = "unlock your XenoNet identity";

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::codec::sha256;
    use xenonet_id::presence::{prove, PresenceCheck};
    use xenonet_id::sign::KeyHome;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-keys-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn fakes_are_file_keys_that_keep_and_answer_as_told() {
        let d = dir("fake");
        let keys = Fake { allow: true };
        let machine = Keys::machine_signer(&keys, &d).unwrap();
        assert_eq!(machine.home(), KeyHome::File);
        assert_eq!(Keys::machine_signer(&keys, &d).unwrap().public(), machine.public(), "made once");
        let device = Keys::device(&keys, &d).unwrap().unwrap();
        assert_eq!(device.kind, Presence::Weaker);
        assert!(!device.signer.asks_person());
        let act = sha256(b"admit laptop");
        let block = prove(device.signer.as_ref(), device.check.as_deref(), "admit", &act, "add laptop", 1_000).unwrap();
        PresenceCheck::new().accept(&block, &device.signer.public(), "admit", &act, 1_500).unwrap();
        let no = Keys::device(&Fake { allow: false }, &d).unwrap().unwrap();
        assert!(prove(no.signer.as_ref(), no.check.as_deref(), "admit", &act, "add laptop", 1_000).is_err());
        assert_eq!(keys.available(&d), Presence::Weaker);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_key_file_says_what_it_holds() {
        let d = dir("held");
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("k");
        assert_eq!(held(&f).unwrap(), None);
        std::fs::write(&f, r#"{"v":1,"kind":"secure-enclave","toid":"x"}"#).unwrap();
        assert_eq!(held(&f).unwrap(), Some(Held::Enclave));
        std::fs::write(&f, r#"{"v":1,"kind":"tpm","name":"x"}"#).unwrap();
        assert_eq!(held(&f).unwrap(), Some(Held::Tpm));
        std::fs::write(&f, r#"{"v":1,"seed":"x"}"#).unwrap();
        assert_eq!(held(&f).unwrap(), Some(Held::Seed));
        std::fs::write(&f, r#"{"v":1}"#).unwrap();
        assert!(held(&f).is_err());
        std::fs::write(&f, "not json").unwrap();
        assert!(held(&f).is_err());
        // A file key stays a file key wherever it's loaded.
        std::fs::remove_file(&f).unwrap();
        let file_key = SeedSigner::load_or_create(&d.join(MACHINE_FILE)).unwrap();
        assert_eq!(machine_signer(&d).unwrap().public(), file_key.public());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_reason_lasts_for_its_call_and_names_the_act_otherwise() {
        assert_eq!(current_reason(), None);
        let inner = with_reason("allow Bash in atlas", || {
            let nested = with_reason("pair a phone", current_reason);
            (current_reason(), nested)
        });
        assert_eq!(inner, (Some("allow Bash in atlas".into()), Some("pair a phone".into())));
        assert_eq!(current_reason(), None);
        assert_eq!(default_reason(Purpose::PRESENCE, b"xns1_x\nanswer\nact\nnonce\n5"), "answer a permission request in XenoNet");
        assert_eq!(default_reason(Purpose::PRESENCE, b"xns1_x\nadmit\n"), "add a machine to your XenoNet network");
        assert_eq!(default_reason(Purpose::PRESENCE, b"\xff"), "confirm an act in XenoNet");
        assert_eq!(default_reason(Purpose::PERSON, b""), "change your XenoNet identity");
        assert_eq!(Presence::Biometric.as_str(), "biometric");
    }
}
