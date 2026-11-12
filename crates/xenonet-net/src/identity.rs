//! The person on this machine (docs/design/direction.md, "Authorization"):
//! their chain, the other people's chains this machine holds to check a
//! network's chain, and the presence blocks it has taken, so a block
//! counts once across restarts, all kept through `Persist`; and in
//! `<state>/identity/`, the device key the machine's `Vault` keeps there
//! and their root seed sealed to the device key (`root.json`; on a machine
//! that can't seal, an owner-only file, which is weaker and says so).
//!
//! One person per machine, in this first cut. The person is founded from
//! XenoNet's window, when the first network is made or the first act as
//! them needs a device signature; a machine that joins pairs its device
//! into the inviter's person. A machine whose own person was never paired
//! with another device (a "local" person) takes the inviter's when it
//! joins; one that already holds a different person with other devices
//! joins as a plain member, holds the owner's chain only to check the
//! network's, and does no acts as the person there.
//!
//! What prompts: signing with a biometric device key, and opening the
//! sealed root. Founding asks once (the root pairs the device under the
//! device's presence), pairing another device twice (the root's opening
//! and the device's presence), issuing or widening a delegation twice (the
//! device's presence and its signature on the entry), narrowing or ending
//! one once (the signature), and every method that may ask takes
//! `at_window`, refusing without asking when the act didn't come from
//! XenoNet's window.

use crate::persist::Persist;
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use xenonet_id::codec::{b64, random, unb64, unb64_n};
use xenonet_id::presence::{prove, PresenceBlock, PresenceCheck};
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer, VerifyingKey};
use xenonet_id::vault::{DeviceKey, Vault};
use xenonet_log::chain::Offered;
pub use xenonet_log::person;
use xenonet_log::person::{Device, DeviceKind, Devices, NewDevice, PersonChain};
use xenonet_os::sync::{Locked, ReadWrite};

pub const ROOT: &str = "root.json";
/// The most entries a person's chain sends in one go.
pub const CHAIN_MAX: usize = 64;

/// Refused because the act didn't come from XenoNet's window, where the
/// person can be asked.
pub fn not_at_window(what: &str) -> NetError {
    NetError::new(403, "window", format!("{what} asks you, so it's done from XenoNet's window"))
}

fn no_device() -> NetError {
    NetError::new(409, "no-device", "this machine has no way to ask you (no Touch ID, polkit or passphrase here), so it does nothing as you")
}

fn no_person() -> NetError {
    NetError::new(409, "no-person", "this machine has no identity of yours yet: make or join a network from XenoNet's window")
}

pub struct Identity {
    dir: PathBuf,
    persist: Arc<dyn Persist>,
    vault: Option<Arc<dyn Vault>>,
    own: RwLock<PersonChain>,
    others: RwLock<BTreeMap<String, PersonChain>>,
    device: Mutex<Option<Arc<DeviceKey>>>,
    machine: Mutex<Option<Arc<dyn Signer>>>,
    seen: Mutex<PresenceCheck>,
    /// Hears each entry appended to the own chain here, to push it to the
    /// network's other machines.
    on_append: Mutex<Option<OnAppend>>,
    /// The roots this machine mints tokens with, and their certificates.
    roots: crate::roots::TokenRoots,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Identity({:?})", self.person_id())
    }
}

/// What's told of each entry appended to this person's chain.
type OnAppend = Box<dyn Fn(&person::Entry) + Send + Sync>;

fn chain_from(entries: Vec<person::Entry>) -> R<PersonChain> {
    Ok(PersonChain::from_entries(entries)?)
}

// root.json, a key file: None when it isn't there.
fn read_root(file: &Path) -> R<Option<Value>> {
    match std::fs::read_to_string(file) {
        Ok(t) => serde_json::from_str(&t).map(Some).map_err(|_| NetError::internal("root.json isn't readable")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl Identity {
    /// The identity as `persist` keeps it, with its keys in `dir`. Nothing
    /// is made and nothing asks: keys come when they're first needed.
    pub fn open(dir: impl Into<PathBuf>, persist: Arc<dyn Persist>, vault: Option<Arc<dyn Vault>>) -> R<Arc<Identity>> {
        let dir = dir.into();
        let mut own = PersonChain::new();
        let mut others = BTreeMap::new();
        for (id, is_own, entries) in persist.persons()? {
            if is_own {
                own = chain_from(entries)?;
                continue;
            }
            // A chain that doesn't read is dropped: it's sent again with the
            // network's next roster.
            if let Ok(c) = chain_from(entries) {
                if c.id() == Some(id.as_str()) {
                    others.insert(id, c);
                }
            }
        }
        let seen = persist.presence_seen()?;
        Ok(Arc::new(Identity {
            roots: crate::roots::TokenRoots::open(persist.clone()),
            persist,
            vault,
            own: RwLock::new(own),
            others: RwLock::new(others),
            device: Mutex::new(None),
            machine: Mutex::new(None),
            seen: Mutex::new(PresenceCheck::from_seen(seen)),
            on_append: Mutex::new(None),
            dir,
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Calls `f` with each entry appended to this person's chain here.
    pub fn on_append(&self, f: Box<dyn Fn(&person::Entry) + Send + Sync>) {
        *self.on_append.locked() = Some(f);
    }

    fn vault(&self) -> R<&Arc<dyn Vault>> {
        self.vault.as_ref().ok_or_else(no_device)
    }

    /// The machine signing key, made the first time.
    pub fn machine_key(&self) -> R<Arc<dyn Signer>> {
        let mut m = self.machine.locked();
        if let Some(k) = m.as_ref() {
            return Ok(k.clone());
        }
        let key: Arc<dyn Signer> = match &self.vault {
            Some(v) => Arc::from(v.machine_signer(&self.dir)?),
            // With no vault (a test's node), a file key in the folder.
            None => Arc::new(SeedSigner::load_or_create(&self.dir.join("machine-sign.key"))?),
        };
        *m = Some(key.clone());
        Ok(key)
    }

    /// The root to mint this machine's tokens with as `machine`, now,
    /// certified by the machine signing key. Signing it never asks.
    pub fn token_root(&self, machine: &xenonet_authz::MachineId, now: i64) -> R<Arc<xenonet_authz::TokenRoot>> {
        let key = self.machine_key()?;
        self.roots.root(machine, key.as_ref(), now)
    }

    /// Every token root certificate of this machine's kept.
    pub fn all_token_roots(&self) -> Vec<xenonet_authz::RootCertificate> {
        self.roots.all()
    }

    /// This machine's token root certificates whose key id is `key_id`.
    pub fn token_roots(&self, key_id: u32) -> Vec<xenonet_authz::RootCertificate> {
        self.roots.certificates(key_id)
    }

    /// The person's device key here, made the first time where this device
    /// can ask the person. Making it never asks.
    pub fn device_key(&self) -> R<Option<Arc<DeviceKey>>> {
        let mut d = self.device.locked();
        if let Some(k) = d.as_ref() {
            return Ok(Some(k.clone()));
        }
        let Some(vault) = &self.vault else { return Ok(None) };
        let key = vault.device(&self.dir)?.map(Arc::new);
        d.clone_from(&key);
        Ok(key)
    }

    /// The device key's id, without making one.
    pub fn device_id(&self) -> Option<String> {
        self.device_key().ok().flatten().map(|d| d.signer.public().id())
    }

    pub fn device_kind(&self) -> Option<DeviceKind> {
        self.device_key().ok().flatten().map(|d| if d.biometric { DeviceKind::Biometric } else { DeviceKind::Weaker })
    }

    pub fn person_id(&self) -> Option<String> {
        self.own.reading().id().map(str::to_string)
    }

    pub fn head(&self) -> Option<String> {
        self.own.reading().head()
    }

    /// The own chain.
    pub fn chain(&self) -> PersonChain {
        self.own.reading().clone()
    }

    /// The chain of `person`, own or held.
    pub fn chain_of(&self, person: &str) -> Option<PersonChain> {
        let own = self.own.reading();
        if own.id() == Some(person) {
            return Some(own.clone());
        }
        self.others.reading().get(person).cloned()
    }

    /// Whether this machine's device is an active device of its own person:
    /// it can act as them.
    pub fn can_act(&self) -> bool {
        let Some(device) = self.device_id() else { return false };
        self.own.reading().person().is_some_and(|p| p.active_device(&device).is_some())
    }

    fn write_own(&self, c: &PersonChain) -> R<()> {
        match c.id() {
            Some(id) => self.persist.put_person(id, true, c.entries()),
            None => Ok(()),
        }
    }

    fn write_other(&self, id: &str, c: &PersonChain) -> R<()> {
        self.persist.put_person(id, false, c.entries())
    }

    fn told(&self, entries: &[person::Entry]) {
        if let Some(f) = self.on_append.locked().as_ref() {
            for e in entries {
                f(e);
            }
        }
    }

    /// A presence block from this device for `what` and the act whose hash
    /// is `act`. Asks the person: only for an act from the window.
    pub fn prove(&self, at_window: bool, what: &str, act: &[u8; 32], reason: &str, now: i64) -> R<PresenceBlock> {
        if !at_window {
            return Err(not_at_window("that"));
        }
        let device = self.device_key()?.ok_or_else(no_device)?;
        let mut out = None;
        self.vault()?.asking(reason, &mut || out = Some(prove(device.signer.as_ref(), device.check.as_deref(), what, act, reason, now)));
        out.unwrap_or_else(|| Err(xenonet_id::IdError::internal("the vault didn't ask"))).map_err(refused)
    }

    /// This device's signature over the act whose hash is `act`, as the
    /// person (`Purpose::ACT`): its device id and the signature. A
    /// biometric key asks: only for an act from the window.
    pub fn sign_act(&self, at_window: bool, act: &[u8; 32], reason: &str) -> R<(String, Vec<u8>)> {
        if !at_window {
            return Err(not_at_window("acting as you"));
        }
        if !self.can_act() {
            return Err(no_person());
        }
        let device = self.device_key()?.ok_or_else(no_device)?;
        let mut out = None;
        self.vault()?.asking(reason, &mut || out = Some(device.signer.sign(Purpose::ACT, act)));
        let sig = out.unwrap_or_else(|| Err(xenonet_id::IdError::internal("the vault didn't ask"))).map_err(refused)?;
        Ok((device.signer.public().id(), sig))
    }

    /// This device's signature over `msg` for `purpose`, as the person: its
    /// device id and the signature. For what the person's device certifies
    /// once and a key of its own then signs (`Purpose::WINDOW`). A
    /// biometric key asks: only from the window.
    pub fn sign_as_person(&self, at_window: bool, purpose: Purpose, msg: &[u8], reason: &str) -> R<(String, Vec<u8>)> {
        if !at_window {
            return Err(not_at_window("acting as you"));
        }
        if !self.can_act() {
            return Err(no_person());
        }
        let device = self.device_key()?.ok_or_else(no_device)?;
        let mut out = None;
        self.vault()?.asking(reason, &mut || out = Some(device.signer.sign(purpose, msg)));
        let sig = out.unwrap_or_else(|| Err(xenonet_id::IdError::internal("the vault didn't ask"))).map_err(refused)?;
        Ok((device.signer.public().id(), sig))
    }

    /// Founds the person here, when there's none: a root key, sealed to
    /// this device, pairs this device under its own presence. Asks once.
    pub fn found(&self, at_window: bool, label: &str, now: i64) -> R<()> {
        if self.person_id().is_some() {
            return Ok(());
        }
        if !at_window {
            return Err(not_at_window("setting up your identity"));
        }
        let device = self.device_key()?.ok_or_else(no_device)?;
        let seed = random::<32>()?;
        let root = SeedSigner::from_seed(&seed, KeyHome::File)?;
        let mut chain = PersonChain::new();
        chain.offer(person::found(&root, now)?)?;
        let kind = if device.biometric { DeviceKind::Biometric } else { DeviceKind::Weaker };
        let mut add = chain.next(person::Body { op: person::Op::AddDevice(NewDevice { key: device.signer.public(), kind, label: clip(label) }), at: now });
        let block = self.prove(true, person::PAIR, &add.digest(), "set up your XenoNet identity on this machine", now)?;
        add.sign(&root.public().id(), &root, Some(block)).map_err(refused)?;
        chain.offer(add)?;
        self.keep_root(&device, &seed)?;
        self.write_own(&chain)?;
        *self.own.writing() = chain;
        Ok(())
    }

    fn keep_root(&self, device: &DeviceKey, seed: &[u8; 32]) -> R<()> {
        let v = match self.vault()?.seal(&device.signer.public(), seed)? {
            Some(sealed) => json!({ "v": 1, "kind": "sealed", "to": device.signer.public().id(), "sealed": b64(&sealed) }),
            None => json!({ "v": 1, "kind": "file", "seed": b64(seed), "note": "kept in this owner-only file, since this machine can't seal it to its device key: weaker" }),
        };
        let text = format!("{}\n", serde_json::to_string_pretty(&v).unwrap_or_default());
        xenonet_id::keys::write_private(&self.dir.join(ROOT), text.as_bytes())?;
        Ok(())
    }

    /// Whether the root seed is sealed to the device key (else it's in an
    /// owner-only file), for the doctor.
    pub fn root_sealed(&self) -> Option<bool> {
        let v = read_root(&self.dir.join(ROOT)).ok().flatten()?;
        Some(v["kind"] == "sealed")
    }

    /// The root key, opened: asks the person where it's sealed.
    fn root(&self, reason: &str) -> R<SeedSigner> {
        let v = read_root(&self.dir.join(ROOT))?
            .ok_or_else(|| NetError::new(409, "no-root", "this machine doesn't hold your identity's root key: pair from the machine that made it"))?;
        let seed: [u8; 32] = match v["kind"].as_str() {
            Some("sealed") => {
                let sealed = v["sealed"].as_str().and_then(unb64).ok_or_else(|| NetError::internal("root.json isn't readable"))?;
                let opened = self.vault()?.open(&self.dir, &sealed, reason).map_err(refused)?;
                opened.try_into().map_err(|_| NetError::internal("the root key didn't open whole"))?
            }
            Some("file") => v["seed"].as_str().and_then(unb64_n).ok_or_else(|| NetError::internal("root.json isn't readable"))?,
            _ => return Err(NetError::internal("root.json isn't readable")),
        };
        Ok(SeedSigner::from_seed(&seed, KeyHome::File)?)
    }

    /// Pairs `new` into this person: the root signs its `add-device`, with
    /// this device's presence. Asks twice where the root is sealed (its
    /// opening and the presence), once where it isn't. The entries it
    /// appended.
    pub fn pair(&self, at_window: bool, new: NewDevice, now: i64) -> R<Vec<person::Entry>> {
        if !at_window {
            return Err(not_at_window("pairing a machine"));
        }
        if !self.can_act() {
            return Err(no_person());
        }
        let id = new.key.id();
        if self.own.reading().person().is_some_and(|p| p.active_device(&id).is_some()) {
            return Ok(Vec::new());
        }
        let label = clip(&new.label);
        let mut e = self.own.reading().next(person::Body { op: person::Op::AddDevice(NewDevice { label, ..new }), at: now });
        let block = self.prove(true, person::PAIR, &e.digest(), "pair a machine with your XenoNet identity", now)?;
        let root = self.root(&xenonet_id_reason("pair a machine with your XenoNet identity"))?;
        e.sign(&root.public().id(), &root, Some(block)).map_err(refused)?;
        {
            let mut own = self.own.writing();
            own.offer(e.clone())?;
            self.write_own(&own)?;
        }
        let made = vec![e];
        self.told(&made);
        Ok(made)
    }

    /// Issues a delegation in this person's chain, or changes the one with
    /// its id, signed by this device; one that widens carries the device's
    /// presence too. Signing asks the person where the device key asks
    /// (Touch ID on a Mac), so only from the window. The entries appended.
    pub fn delegate(&self, at_window: bool, d: person::Delegation, now: i64) -> R<Vec<person::Entry>> {
        let widens = self.own.reading().person().is_some_and(|p| p.delegations.get(&d.id).is_none_or(|(old, _)| !d.within(old)));
        let reason = "let your devices run and answer the work you delegate";
        self.append(at_window, person::Op::Delegate(d), widens, reason, now)
    }

    /// Ends the delegation `id`: narrowing, so no presence, but still this
    /// device's signature.
    pub fn undelegate(&self, at_window: bool, id: &str, now: i64) -> R<Vec<person::Entry>> {
        self.append(at_window, person::Op::Undelegate { id: id.to_string() }, false, "end a delegation of yours", now)
    }

    /// Revokes another of this person's devices, as for a lost machine:
    /// signed by this device with its presence (it asks once, or twice
    /// where the device key asks for its signature too). A revoked device
    /// signs nothing that counts again, here or at any machine that holds
    /// the entry. Never this machine's own device, which would leave it
    /// unable to act as the person; revoke it from another machine.
    pub fn revoke_device(&self, at_window: bool, device: &str, now: i64) -> R<Vec<person::Entry>> {
        if self.device_id().as_deref() == Some(device) {
            return Err(NetError::new(409, "own-device", "that's this machine's own device: revoke it from another machine of yours"));
        }
        let known = self.own.reading().person().and_then(|p| p.device(device).map(Device::active));
        match known {
            None => return Err(NetError::new(404, "no-device", format!("{device} isn't one of your devices"))),
            Some(false) => return Err(NetError::new(409, "no-change", format!("{device} can't sign already"))),
            Some(true) => {}
        }
        self.append(at_window, person::Op::Revoke(person::Revocation::Device(device.to_string())), true, "revoke a device of yours", now)
    }

    // An entry of this person's own, signed by this device, with its
    // presence when `present`.
    fn append(&self, at_window: bool, op: person::Op, present: bool, reason: &str, now: i64) -> R<Vec<person::Entry>> {
        if !at_window {
            return Err(not_at_window("that"));
        }
        if !self.can_act() {
            return Err(no_person());
        }
        let device = self.device_key()?.ok_or_else(no_device)?;
        let mut e = self.own.reading().next(person::Body { op, at: now });
        let block = if present { Some(self.prove(true, e.op.presence_for(), &e.digest(), reason, now)?) } else { None };
        let id = device.signer.public().id();
        let mut signed = None;
        self.vault()?.asking(reason, &mut || signed = Some(e.sign(&id, device.signer.as_ref(), block.clone())));
        signed.unwrap_or_else(|| Err(xenonet_id::IdError::internal("the vault didn't ask"))).map_err(refused)?;
        {
            let mut own = self.own.writing();
            own.offer(e.clone())?;
            self.write_own(&own)?;
        }
        let made = vec![e];
        self.told(&made);
        Ok(made)
    }

    /// Takes `person`'s chain as this machine's own, when it holds this
    /// device and this machine's own person is none or local (paired with
    /// no other device). True when it was taken.
    pub fn adopt(&self, chain: &PersonChain) -> bool {
        let Some(device) = self.device_id() else { return false };
        if chain.person().is_none_or(|p| p.active_device(&device).is_none()) {
            return false;
        }
        let mut own = self.own.writing();
        let local = own.person().is_none_or(|p| p.active_devices().all(|d| d.id == device));
        if !local && own.id() != chain.id() {
            return false;
        }
        if own.id() == chain.id() && own.len() >= chain.len() {
            return true;
        }
        if self.write_own(chain).is_err() {
            return false;
        }
        *own = chain.clone();
        true
    }

    /// Entries of a person's chain from another machine (from entry 0, or
    /// from where this machine's copy stands), offered to the chain they
    /// extend: this person's own, or one held. True when one was appended.
    pub fn offer(&self, person: &str, entries: &[Value]) -> R<bool> {
        let mut parsed = Vec::new();
        for e in entries.iter().take(CHAIN_MAX * 4) {
            parsed.push(person::Entry::parse(e)?);
        }
        let mut appended = false;
        {
            let mut own = self.own.writing();
            if own.id() == Some(person) {
                for e in parsed {
                    appended |= own.offer(e)? == Offered::Appended;
                }
                if appended {
                    self.write_own(&own)?;
                }
                return Ok(appended);
            }
        }
        let mut others = self.others.writing();
        let mut chain = others.get(person).cloned().unwrap_or_default();
        for e in parsed {
            appended |= chain.offer(e)? == Offered::Appended;
        }
        if chain.id() != Some(person) {
            return Err(NetError::bad("that isn't the chain of the person it names"));
        }
        if appended {
            self.write_other(person, &chain)?;
            others.insert(person.to_string(), chain);
        }
        Ok(appended)
    }

    /// The chain of `person` from index `from`, as entries travel.
    pub fn since(&self, person: &str, from: u64) -> Vec<Value> {
        self.chain_of(person).map(|c| c.since(from).iter().take(CHAIN_MAX).map(person::Entry::to_json).collect()).unwrap_or_default()
    }

    /// Takes `block` for a live act, once (`PresenceCheck::accept`), and
    /// keeps what it took.
    pub fn accept(&self, block: &PresenceBlock, device: &VerifyingKey, what: &str, act: &[u8; 32], now: i64) -> R<()> {
        self.with_seen(|seen| seen.accept(block, device, what, act, now).map_err(refused))
    }

    /// Runs `f` on the blocks taken so far, then keeps them.
    pub fn with_seen<T>(&self, f: impl FnOnce(&mut PresenceCheck) -> R<T>) -> R<T> {
        let mut seen = self.seen.locked();
        let out = f(&mut seen)?;
        self.persist.put_presence_seen(&seen.seen())?;
        Ok(out)
    }

    /// `device`, as `person`'s chain has it now, active.
    pub fn active_device(&self, person: &str, device: &str) -> Option<Device> {
        let c = self.chain_of(person)?;
        c.person()?.active_device(device).cloned()
    }

    /// The head index of `person`'s chain here, and the index of `head` in
    /// it (None when it isn't one of its heads).
    pub fn where_is(&self, person: &str, head: &str) -> (Option<u64>, Option<u64>) {
        let Some(c) = self.chain_of(person) else { return (None, None) };
        (c.len().checked_sub(1), c.index_of(head))
    }

    /// Forgets what's held for the network a machine left; its own person
    /// stays.
    pub fn forget_others(&self) {
        let mut others = self.others.writing();
        others.clear();
        let _ = self.persist.forget_others();
    }
}

fn xenonet_id_reason(s: &str) -> String {
    s.to_string()
}

fn clip(label: &str) -> String {
    let l: String = label.chars().filter(|c| !c.is_control()).take(48).collect();
    if l.is_empty() {
        "this machine".into()
    } else {
        l
    }
}

fn refused(e: xenonet_id::IdError) -> NetError {
    NetError::new(403, "refused", e.message)
}

impl Devices for Identity {
    fn person_device(&self, person: &str, head: &str, device: &str) -> Option<Device> {
        let own = self.own.reading();
        if own.id() == Some(person) {
            return own.device_at(device, head).cloned();
        }
        self.others.reading().get(person).and_then(|c| c.device_at(device, head).cloned())
    }
}

/// A device as a join request carries it: its key, kind and label.
pub fn device_json(key: &VerifyingKey, kind: DeviceKind, label: &str) -> Value {
    json!({ "key": key.to_json(), "kind": kind.as_str(), "label": clip(label) })
}

pub fn parse_device(v: &Value) -> Option<NewDevice> {
    let key = v.get("key").and_then(VerifyingKey::parse)?;
    let kind = v.get("kind").and_then(Value::as_str).and_then(DeviceKind::parse)?;
    let label = v.get("label").and_then(Value::as_str).map(clip)?;
    Some(NewDevice { key, kind, label })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::presence::FixedCheck;

    /// File keys in the folder, a check that answers as told.
    struct Fake;

    impl Vault for Fake {
        fn machine_signer(&self, dir: &Path) -> xenonet_id::R<Box<dyn Signer>> {
            Ok(Box::new(SeedSigner::load_or_create(&dir.join("machine.key"))?))
        }
        fn device(&self, dir: &Path) -> xenonet_id::R<Option<DeviceKey>> {
            let signer = SeedSigner::load_or_create(&dir.join("device.key"))?;
            Ok(Some(DeviceKey { signer: Box::new(signer), check: Some(Box::new(FixedCheck(true))), biometric: false }))
        }
        fn seal(&self, _to: &VerifyingKey, _secret: &[u8]) -> xenonet_id::R<Option<Vec<u8>>> {
            Ok(None)
        }
        fn open(&self, _dir: &Path, _sealed: &[u8], _reason: &str) -> xenonet_id::R<Vec<u8>> {
            Err(xenonet_id::IdError::internal("none"))
        }
        fn asking(&self, _reason: &str, f: &mut dyn FnMut()) {
            f();
        }
    }

    // A machine: its keys' folder, and what it keeps, which outlives a
    // restart as the store does.
    fn dir(name: &str) -> (PathBuf, Arc<dyn Persist>) {
        let d = std::env::temp_dir().join(format!("xenonet-identity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        (d, Arc::new(crate::persist::Memory::default()))
    }

    fn open(m: &(PathBuf, Arc<dyn Persist>)) -> Arc<Identity> {
        Identity::open(&m.0, m.1.clone(), Some(Arc::new(Fake))).unwrap()
    }

    #[test]
    fn a_person_is_founded_at_the_window_and_pairs_another_device() {
        let (a, b) = (dir("a"), dir("b"));
        let studio = open(&a);
        assert!(studio.found(false, "studio", 1).is_err(), "only from the window");
        studio.found(true, "studio", 1).unwrap();
        assert!(studio.can_act());
        let person = studio.person_id().unwrap();
        // laptop's device, paired by studio; laptop takes the chain.
        let laptop = open(&b);
        let key = laptop.device_key().unwrap().unwrap().signer.public();
        assert!(studio.pair(false, NewDevice { key: key.clone(), kind: DeviceKind::Weaker, label: "laptop".into() }, 2).is_err());
        let made = studio.pair(true, NewDevice { key, kind: DeviceKind::Weaker, label: "laptop".into() }, 2).unwrap();
        assert_eq!(made.len(), 1);
        assert!(laptop.adopt(&studio.chain()));
        assert_eq!(laptop.person_id().as_deref(), Some(person.as_str()));
        assert!(laptop.can_act());
        // Kept across a restart, the presence it took too.
        let again = open(&b);
        assert_eq!(again.head(), studio.head());
        let act = xenonet_id::codec::sha256(b"answer");
        let block = studio.prove(true, "answer", &act, "", 10).unwrap();
        let device = studio.device_key().unwrap().unwrap().signer.public();
        laptop.accept(&block, &device, "answer", &act, 20).unwrap();
        let later = open(&b);
        assert!(later.accept(&block, &device, "answer", &act, 30).is_err(), "seen before a restart");
        let _ = std::fs::remove_dir_all(a.0);
        let _ = std::fs::remove_dir_all(b.0);
    }

    #[test]
    fn another_device_is_revoked_from_the_window_and_never_this_one() {
        let (a, b) = (dir("revoke-a"), dir("revoke-b"));
        let studio = open(&a);
        studio.found(true, "studio", 1).unwrap();
        let laptop = open(&b);
        let key = laptop.device_key().unwrap().unwrap().signer.public();
        studio.pair(true, NewDevice { key: key.clone(), kind: DeviceKind::Weaker, label: "laptop".into() }, 2).unwrap();
        assert!(laptop.adopt(&studio.chain()) && laptop.can_act());
        let lost = key.id();
        // Away from the window, nothing is asked or signed.
        assert_eq!(studio.revoke_device(false, &lost, 3).unwrap_err().code, "window");
        // This machine's own device, or one the person doesn't have: refused.
        assert_eq!(studio.revoke_device(true, &studio.device_id().unwrap(), 3).unwrap_err().code, "own-device");
        assert_eq!(studio.revoke_device(true, "xns1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 3).unwrap_err().code, "no-device");
        // laptop's device, revoked with studio's presence: it can't sign as
        // the person any more, and laptop, taking the entry, can't act.
        let made = studio.revoke_device(true, &lost, 3).unwrap();
        assert_eq!(made.len(), 1);
        assert!(made[0].signers[0].presence.is_some(), "a revocation carries the device's presence");
        assert!(studio.chain().person().unwrap().active_device(&lost).is_none());
        assert_eq!(studio.revoke_device(true, &lost, 4).unwrap_err().code, "no-change");
        let person = studio.person_id().unwrap();
        assert!(laptop.offer(&person, &[made[0].to_json()]).unwrap());
        assert!(!laptop.can_act());
        let _ = std::fs::remove_dir_all(a.0);
        let _ = std::fs::remove_dir_all(b.0);
    }
}
