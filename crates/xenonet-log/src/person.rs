//! A person's chain (docs/design/direction.md, "Domain model"): their root
//! key, the device keys it or their devices pair, and what they revoke
//! (`xenonet-person/1`). A receiver maps a device's signature to a person
//! through its own copy of this chain, so a device that was removed or
//! revoked stops counting the moment the receiver holds that entry.
//!
//! | op | signed by |
//! |---|---|
//! | `found` | the root key, alone, as entry 0 |
//! | `add-device`, `remove-device` | the root key with a device's presence block, or two devices each with its own |
//! | `revoke` a token or a device | one device with its presence block (or the root key with a device's) |
//! | `delegate` that issues or widens a delegation | one device with its presence block (or the root key with a device's) |
//! | `delegate` that narrows one, `undelegate` | one device, or the root key; no presence |
//!
//! **Delegation.** A `delegate` entry is a standing grant from the person to
//! a set of their devices (named, or all of them, those paired later too):
//! any device in the set may run provider sessions, and answer their
//! permission prompts within the grant's limits, for work the person
//! started on any device in the set. `work` says which (`start`,
//! `answer`), `prompts` which kinds of prompt an Allow under it may cover,
//! `limits` the rules a covered prompt must pass (today one set,
//! `routine-in-folder`, the delegation judge's), and `until` when it ends.
//! A delegation has an id; a `delegate` with an id the person already has
//! replaces that one, and it widens unless every field is within the old
//! one's. Widening asks for the person there; narrowing and ending
//! (`undelegate`) can't let anyone do more, so they don't. The set is
//! named as devices, not machines, so the intents phase 4 adds (made on
//! one device, worked on the others) take the same grant.
//!
//! Signers are keys, by their `xns1_` id: the root's, or a device's. A
//! device that was removed or revoked can't sign. Every signature but the
//! founding one comes with a presence block (`xenonet_id::presence`) for
//! the entry, `pair` to add a device and `revoke` to remove or revoke,
//! approving the entry's id: a device's block is its own, and the root's
//! is from a device the person holds, or, for `add-device`, from the
//! device being added, which is how the first device is paired. A chain
//! checks the blocks for what they sign, never for their age, since a
//! machine catching up reads entries that were fresh when they were made;
//! a live receiver checks freshness itself (`PresenceCheck`).
//!
//! A person's id is `prs_` and the base32 of the first 20 bytes of the
//! SHA-256 of entry 0's canonical JSON with its signer.
//!
//! **The project salt.** Entry 0 carries 32 random bytes, `salt`, made at
//! founding: a project's id (`prj_`, xenonet-model's `ProjectId::of_repo`)
//! is an HMAC of its git remote under it, so one repo is one project on
//! every machine of the person's, and the id names no repo to anyone who
//! doesn't hold the chain.
//!
//! A device key that was removed or revoked isn't added again: a machine
//! that comes back makes a new key, and so a device's history is one span,
//! which `device_at` reads for any past head.

use crate::chain::{self, Chain, ChainError, Codes, Format, Policy, Signature, R};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use xenonet_id::codec::{b64, base32, sha256, unb64_n};
use xenonet_id::sign::{Purpose, VerifyingKey};

/// A person's chain.
pub type PersonChain = Chain<PersonPolicy>;
/// An entry of a person's chain.
pub type Entry = chain::Entry<PersonPolicy>;

/// The longest label a device carries, in bytes.
pub const LABEL_MAX: usize = 64;
/// What a presence block approves to add a device.
pub const PAIR: &str = "pair";
/// What a presence block approves to remove or revoke.
pub const REVOKE: &str = "revoke";
/// What a presence block approves to issue or widen a delegation.
pub const DELEGATE: &str = "delegate";
/// The kinds of work a delegation covers.
pub const WORK: [&str; 2] = ["start", "answer"];
/// The kinds of permission prompt an Allow under a delegation may cover:
/// xenonet-authz's prompt kinds, by their words.
pub const PROMPTS: [&str; 5] = ["read", "edit", "command", "web", "tool"];
/// The rule sets a delegation's covered prompts must pass: today's one is
/// the delegation judge's (routine work inside the folder the run started
/// in; `xenonet_engine::delivery::delegation`).
pub const LIMITS: [&str; 1] = ["routine-in-folder"];
/// The most delegations a person holds at once, and devices one names.
pub const DELEGATIONS_MAX: usize = 16;

/// `dlg_` and 16 to 32 lowercase hex characters.
pub fn is_delegation_id(id: &str) -> bool {
    id.strip_prefix("dlg_").is_some_and(|h| (16..=32).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// `xns1_` and 32 base32 characters: a signing key's id.
pub fn is_key_id(id: &str) -> bool {
    id.strip_prefix("xns1_").is_some_and(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)))
}

/// `prs_` and 32 base32 characters.
pub fn is_person_id(id: &str) -> bool {
    id.strip_prefix("prs_").is_some_and(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)))
}

/// A token's revocation id: lowercase hex, 16 to 128 bytes of it.
pub fn is_token_id(id: &str) -> bool {
    id.len().is_multiple_of(2) && (32..=256).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn label_ok(label: &str) -> bool {
    !label.is_empty() && label.len() <= LABEL_MAX && !label.chars().any(char::is_control)
}

/// How a device proves the person was there, as the chain recorded it
/// when the device was added: policy reads strength here, never from a
/// block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// Its key asks for the person's biometric before each signature.
    Biometric,
    /// It signs after a weaker check (polkit, a passphrase).
    Weaker,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceKind::Biometric => "biometric",
            DeviceKind::Weaker => "weaker",
        }
    }

    pub fn parse(s: &str) -> Option<DeviceKind> {
        [DeviceKind::Biometric, DeviceKind::Weaker].into_iter().find(|k| k.as_str() == s)
    }
}

/// A device an `add-device` entry pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDevice {
    pub key: VerifyingKey,
    pub kind: DeviceKind,
    /// What the person calls it.
    pub label: String,
}

/// What a `revoke` entry revokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revocation {
    /// A token, by its revocation id.
    Token(String),
    /// A device, by its key's id.
    Device(String),
}

/// The devices a delegation names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSet {
    /// Every device of the person's, those paired later too.
    All,
    /// These, by their keys' ids.
    Only(Vec<String>),
}

impl DeviceSet {
    pub fn has(&self, device: &str) -> bool {
        match self {
            DeviceSet::All => true,
            DeviceSet::Only(list) => list.iter().any(|d| d == device),
        }
    }

    fn within(&self, other: &DeviceSet) -> bool {
        match (self, other) {
            (_, DeviceSet::All) => true,
            (DeviceSet::All, DeviceSet::Only(_)) => false,
            (DeviceSet::Only(a), DeviceSet::Only(b)) => a.iter().all(|d| b.contains(d)),
        }
    }
}

/// A standing grant from the person to a set of their devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    pub id: String,
    /// The devices that may work under it, and that work under it may have
    /// been started from.
    pub devices: DeviceSet,
    /// Of `WORK`, sorted, at least one.
    pub work: Vec<String>,
    /// Of `PROMPTS`, sorted: what an Allow under it may cover.
    pub prompts: Vec<String>,
    /// One of `LIMITS`.
    pub limits: String,
    /// Milliseconds since 1970 when it ends.
    pub until: i64,
}

impl Delegation {
    /// The delegation a machine set to take delegated work gets when none
    /// names it: every device, starts and answers, for the prompts the
    /// delegation judge may let through (reading, editing, routine commands,
    /// the tools that report back), within its limits, until `until`.
    pub fn standard(id: String, until: i64) -> Delegation {
        let words = |l: &[&str]| l.iter().map(ToString::to_string).collect();
        Delegation { id, devices: DeviceSet::All, work: words(&["answer", "start"]), prompts: words(&["command", "edit", "read", "tool"]), limits: LIMITS[0].into(), until }
    }

    /// Whether it lets no one do more than `old`.
    pub fn within(&self, old: &Delegation) -> bool {
        self.devices.within(&old.devices)
            && self.work.iter().all(|w| old.work.contains(w))
            && self.prompts.iter().all(|p| old.prompts.contains(p))
            && self.limits == old.limits
            && self.until <= old.until
    }

    pub fn covers(&self, work: &str) -> bool {
        self.work.iter().any(|w| w == work)
    }

    fn to_json(&self) -> Value {
        let devices = match &self.devices {
            DeviceSet::All => Value::String("all".into()),
            DeviceSet::Only(list) => json!(list),
        };
        json!({ "id": self.id, "devices": devices, "work": self.work, "prompts": self.prompts, "limits": self.limits, "until": self.until })
    }

    fn parse(v: &Value) -> Option<Delegation> {
        let words = |k: &str, of: &[&str]| -> Option<Vec<String>> {
            let list: Vec<String> = v.get(k)?.as_array()?.iter().map(|w| w.as_str().filter(|w| of.contains(w)).map(str::to_string)).collect::<Option<_>>()?;
            Some(list)
        };
        let devices = match v.get("devices")? {
            Value::String(s) if s == "all" => DeviceSet::All,
            Value::Array(a) => DeviceSet::Only(a.iter().map(|d| d.as_str().filter(|d| is_key_id(d)).map(str::to_string)).collect::<Option<_>>()?),
            _ => return None,
        };
        Some(Delegation {
            id: v.get("id")?.as_str().filter(|i| is_delegation_id(i))?.to_string(),
            devices,
            work: words("work", &WORK)?,
            prompts: words("prompts", &PROMPTS)?,
            limits: v.get("limits")?.as_str().filter(|l| LIMITS.contains(l))?.to_string(),
            until: v.get("until")?.as_i64()?,
        })
    }

    // Lists sorted and without repeats, so one grant has one form, and
    // within the bounds.
    fn well_formed(&self) -> bool {
        let sorted = |l: &[String]| l.windows(2).all(|w| w[0] < w[1]);
        let devices = match &self.devices {
            DeviceSet::All => true,
            DeviceSet::Only(list) => !list.is_empty() && list.len() <= DELEGATIONS_MAX && sorted(list) && list.iter().all(|d| is_key_id(d)),
        };
        is_delegation_id(&self.id)
            && devices
            && !self.work.is_empty()
            && sorted(&self.work)
            && self.work.iter().all(|w| WORK.contains(&w.as_str()))
            && sorted(&self.prompts)
            && self.prompts.iter().all(|p| PROMPTS.contains(&p.as_str()))
            && LIMITS.contains(&self.limits.as_str())
    }
}

/// The bytes of a person's project salt.
pub const SALT_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Found {
        root: VerifyingKey,
        /// What their projects' ids are made under.
        salt: [u8; SALT_BYTES],
    },
    AddDevice(NewDevice),
    RemoveDevice {
        device: String,
    },
    Revoke(Revocation),
    /// Issues a delegation, or changes the one with its id.
    Delegate(Delegation),
    /// Ends the delegation with this id.
    Undelegate {
        id: String,
    },
}

impl Op {
    pub fn as_str(&self) -> &'static str {
        match self {
            Op::Found { .. } => "found",
            Op::AddDevice(_) => "add-device",
            Op::RemoveDevice { .. } => "remove-device",
            Op::Revoke(_) => "revoke",
            Op::Delegate(_) => "delegate",
            Op::Undelegate { .. } => "undelegate",
        }
    }

    /// What a presence block for this op approves.
    pub fn presence_for(&self) -> &'static str {
        match self {
            Op::AddDevice(_) => PAIR,
            Op::Delegate(_) | Op::Undelegate { .. } => DELEGATE,
            _ => REVOKE,
        }
    }
}

/// What a person's chain entry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub op: Op,
    /// Milliseconds since 1970, by the signer's clock.
    pub at: i64,
}

/// One of a person's devices, with when it came and went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub key: VerifyingKey,
    pub kind: DeviceKind,
    pub label: String,
    /// The entry that added it.
    pub added: u64,
    pub removed: Option<u64>,
    pub revoked: Option<u64>,
}

impl Device {
    /// Whether it signs now.
    pub fn active(&self) -> bool {
        self.removed.is_none() && self.revoked.is_none()
    }

    /// Whether it signed as of entry `n`: added by then, and neither
    /// removed nor revoked by then.
    pub fn active_at(&self, n: u64) -> bool {
        self.added <= n && self.removed.is_none_or(|r| r > n) && self.revoked.is_none_or(|r| r > n)
    }
}

/// A person as their chain leaves them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    pub root: VerifyingKey,
    /// What their projects' ids are made under (the module's header).
    pub salt: [u8; SALT_BYTES],
    /// Every device ever paired, by key id, active or not.
    pub devices: BTreeMap<String, Device>,
    /// The revoked tokens' ids, with the entry that revoked each.
    pub revoked_tokens: BTreeMap<String, u64>,
    /// The delegations unended, by id, with the entry that last set each.
    pub delegations: BTreeMap<String, (Delegation, u64)>,
}

impl Person {
    pub fn device(&self, id: &str) -> Option<&Device> {
        self.devices.get(id)
    }

    /// A device that signs now.
    pub fn active_device(&self, id: &str) -> Option<&Device> {
        self.devices.get(id).filter(|d| d.active())
    }

    pub fn active_devices(&self) -> impl Iterator<Item = &Device> {
        self.devices.values().filter(|d| d.active())
    }

    pub fn is_revoked(&self, token: &str) -> bool {
        self.revoked_tokens.contains_key(token)
    }

    /// The delegations that name `device`, an active device of the
    /// person's: whether one covers a piece of work is the caller's.
    pub fn delegations_of<'a>(&'a self, device: &'a str) -> impl Iterator<Item = &'a Delegation> {
        let active = self.active_device(device).is_some();
        self.delegations.values().map(|(d, _)| d).filter(move |d| active && d.devices.has(device))
    }
}

/// What another chain's policy consults to check a signer's device: the
/// device as `person`'s active device at their chain's head `head`. One
/// person's chain answers for that person; a store of them answers for
/// each.
pub trait Devices: Send + Sync {
    fn person_device(&self, person: &str, head: &str, device: &str) -> Option<Device>;
}

/// The person chain's rules. It holds nothing: everything they need is in
/// the chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PersonPolicy;

fn refused(message: impl Into<String>) -> ChainError {
    PersonPolicy::FORMAT.refused(message)
}

fn no_device(id: &str) -> ChainError {
    ChainError::new(404, "no-device", format!("{id} isn't one of this person's devices"))
}

impl Policy for PersonPolicy {
    type Body = Body;
    type State = Person;

    const FORMAT: Format = Format {
        tag: Purpose::PERSON,
        noun: "person chain",
        signer: "key",
        signer_key: "key",
        presence: true,
        codes: Codes { gap: "person-gap", fork: "person-fork", refused: "person-refused" },
    };

    fn write_body(b: &Body, out: &mut Map<String, Value>) {
        out.insert("op".into(), Value::String(b.op.as_str().into()));
        match &b.op {
            Op::Found { root, salt } => {
                out.insert("root".into(), root.to_json());
                out.insert("salt".into(), Value::String(b64(salt)));
            }
            Op::AddDevice(d) => {
                out.insert("device".into(), json!({ "key": d.key.to_json(), "id": d.key.id(), "kind": d.kind.as_str(), "label": d.label }));
            }
            Op::RemoveDevice { device } => {
                out.insert("device".into(), Value::String(device.clone()));
            }
            Op::Revoke(r) => {
                let what = match r {
                    Revocation::Token(t) => json!({ "token": t }),
                    Revocation::Device(d) => json!({ "device": d }),
                };
                out.insert("revoke".into(), what);
            }
            Op::Delegate(d) => {
                out.insert("delegation".into(), d.to_json());
            }
            Op::Undelegate { id } => {
                out.insert("delegation".into(), Value::String(id.clone()));
            }
        }
        out.insert("at".into(), Value::from(b.at));
    }

    fn parse_body(v: &Value) -> R<Body> {
        let bad = |what: &str| Self::FORMAT.bad(what);
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let op = match v.get("op").and_then(Value::as_str) {
            Some("found") => Op::Found {
                root: v.get("root").and_then(VerifyingKey::parse).ok_or_else(|| bad("root key"))?,
                salt: v.get("salt").and_then(Value::as_str).and_then(unb64_n::<SALT_BYTES>).ok_or_else(|| bad("salt"))?,
            },
            Some("add-device") => {
                let d = v.get("device").filter(|d| d.is_object()).ok_or_else(|| bad("device"))?;
                let key = d.get("key").and_then(VerifyingKey::parse).ok_or_else(|| bad("device key"))?;
                if text(d, "id").as_deref() != Some(key.id().as_str()) {
                    return Err(bad("device id"));
                }
                let kind = d.get("kind").and_then(Value::as_str).and_then(DeviceKind::parse).ok_or_else(|| bad("device kind"))?;
                let label = text(d, "label").filter(|l| label_ok(l)).ok_or_else(|| bad("device label"))?;
                Op::AddDevice(NewDevice { key, kind, label })
            }
            Some("remove-device") => Op::RemoveDevice { device: text(v, "device").filter(|d| is_key_id(d)).ok_or_else(|| bad("device"))? },
            Some("revoke") => {
                let r = v.get("revoke").and_then(Value::as_object).ok_or_else(|| bad("revocation"))?;
                match (r.get("token").and_then(Value::as_str), r.get("device").and_then(Value::as_str), r.len()) {
                    (Some(t), None, 1) if is_token_id(t) => Op::Revoke(Revocation::Token(t.into())),
                    (None, Some(d), 1) if is_key_id(d) => Op::Revoke(Revocation::Device(d.into())),
                    _ => return Err(bad("revocation")),
                }
            }
            Some("delegate") => Op::Delegate(v.get("delegation").and_then(Delegation::parse).ok_or_else(|| bad("delegation"))?),
            Some("undelegate") => Op::Undelegate { id: text(v, "delegation").filter(|i| is_delegation_id(i)).ok_or_else(|| bad("delegation"))? },
            _ => return Err(bad("op")),
        };
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        Ok(Body { op, at })
    }

    fn body_well_formed(b: &Body) -> R<()> {
        let bad = |what: &str| Self::FORMAT.bad(what);
        match &b.op {
            Op::Found { .. } => Ok(()),
            Op::AddDevice(d) if !label_ok(&d.label) => Err(bad("device label")),
            Op::AddDevice(_) => Ok(()),
            Op::RemoveDevice { device } | Op::Revoke(Revocation::Device(device)) if !is_key_id(device) => Err(bad("device")),
            Op::Revoke(Revocation::Token(t)) if !is_token_id(t) => Err(bad("revocation")),
            Op::RemoveDevice { .. } | Op::Revoke(_) => Ok(()),
            Op::Delegate(d) if !d.well_formed() => Err(bad("delegation")),
            Op::Undelegate { id } if !is_delegation_id(id) => Err(bad("delegation")),
            Op::Delegate(_) | Op::Undelegate { .. } => Ok(()),
        }
    }

    fn is_signer(id: &str) -> bool {
        is_key_id(id)
    }

    /// Ed25519's 64 bytes, or P-256's DER, which a secure element signs
    /// with: the same bounds a presence block's signature has.
    fn sig_ok(sig: &[u8]) -> bool {
        (64..=72).contains(&sig.len())
    }

    fn same(a: &Body, b: &Body) -> bool {
        a.op == b.op
    }

    fn scope(&self, person: Option<&Person>, e: &Entry) -> R<()> {
        match (person, &e.op) {
            (None, Op::Found { root, .. }) => {
                if e.signers.len() != 1 || e.signers[0].by != root.id() {
                    return Err(root_alone());
                }
                if e.signers[0].presence.is_some() {
                    return Err(refused("the first entry is the root key's alone, with no presence"));
                }
                Ok(())
            }
            (None, _) => Err(refused("a person's chain starts with their root key")),
            (Some(_), Op::Found { .. }) => Err(refused("a person has one root key")),
            (Some(_), _) => Ok(()),
        }
    }

    fn signer_key(&self, person: Option<&Person>, e: &Entry, by: &str) -> R<VerifyingKey> {
        let Some(p) = person else {
            return match &e.op {
                Op::Found { root, .. } => Ok(root.clone()),
                _ => Err(root_alone()),
            };
        };
        if by == p.root.id() {
            return Ok(p.root.clone());
        }
        match p.device(by) {
            Some(d) if d.revoked.is_some() => Err(refused(format!("{by} was revoked: it can't sign"))),
            Some(d) if d.removed.is_some() => Err(refused(format!("{by} was removed: it can't sign"))),
            Some(d) => Ok(d.key.clone()),
            None => Err(refused(format!("{by} isn't one of this person's keys"))),
        }
    }

    fn bad_signature(&self, person: Option<&Person>, by: &str) -> ChainError {
        match person {
            None => root_alone(),
            Some(_) => refused(format!("{by}'s signature doesn't check")),
        }
    }

    fn check(&self, person: Option<&Person>, e: &Entry) -> R<()> {
        let Some(p) = person else { return Ok(()) };
        // What the entry changes must be there to change, before whose
        // say-so it is: as the roster answers `taken` before `needs-approval`.
        match &e.op {
            Op::Found { .. } => return Err(refused("a person has one root key")),
            Op::AddDevice(d) => {
                let id = d.key.id();
                if id == p.root.id() {
                    return Err(ChainError::taken("that's the root key, not a device's"));
                }
                if p.devices.contains_key(&id) {
                    return Err(ChainError::taken(format!("{id} was paired before: a device that comes back makes a new key")));
                }
            }
            Op::RemoveDevice { device } => {
                if p.device(device).is_none_or(|d| d.removed.is_some()) {
                    return Err(no_device(device));
                }
            }
            Op::Revoke(Revocation::Device(device)) => match p.device(device) {
                None => return Err(no_device(device)),
                Some(d) if !d.active() => return Err(ChainError::no_change(format!("{device} can't sign already"))),
                Some(_) => {}
            },
            Op::Revoke(Revocation::Token(t)) => {
                if p.is_revoked(t) {
                    return Err(ChainError::no_change("that token is revoked already"));
                }
            }
            Op::Delegate(d) => {
                if let DeviceSet::Only(list) = &d.devices {
                    if let Some(id) = list.iter().find(|id| p.active_device(id).is_none()) {
                        return Err(no_device(id));
                    }
                }
                let old = p.delegations.get(&d.id).map(|(o, _)| o);
                if old == Some(d) {
                    return Err(ChainError::no_change("that delegation is so already"));
                }
                if old.is_none() && p.delegations.len() >= DELEGATIONS_MAX {
                    return Err(refused(format!("a person holds {DELEGATIONS_MAX} delegations at most: end one first")));
                }
                return delegation_signers(p, e, old.is_none_or(|o| !d.within(o)));
            }
            Op::Undelegate { id } => {
                if !p.delegations.contains_key(id) {
                    return Err(ChainError::new(404, "no-delegation", format!("{id} isn't one of this person's delegations")));
                }
                return delegation_signers(p, e, false);
            }
        }
        let what = e.op.presence_for();
        let act = e.digest();
        let root = p.root.id();
        let mut by_root = false;
        let mut devices = 0;
        for s in &e.signers {
            if s.by == root {
                presence_from_device(p, e, s, what, &act)?;
                by_root = true;
            } else {
                let block = s.presence.as_ref().ok_or_else(|| refused(format!("{} signs with a presence block", s.by)))?;
                // signer_key let only an active device through.
                let key = p.active_device(&s.by).map(|d| &d.key).ok_or_else(|| no_device(&s.by))?;
                if block.device != s.by {
                    return Err(refused(format!("{}'s presence is another device's", s.by)));
                }
                block.signed_for(key, what, &act).map_err(|err| refused(err.message))?;
                devices += 1;
            }
        }
        match &e.op {
            Op::Revoke(_) => Ok(()),
            _ if by_root || devices >= 2 => Ok(()),
            _ => Err(ChainError::needs_approval("that needs the root key, or two devices")),
        }
    }

    fn apply(&self, person: Option<Person>, e: &Entry) -> Person {
        let Some(mut p) = person else {
            let Op::Found { root, salt } = &e.op else { unreachable!("scope takes only `found` as a person's entry 0") };
            return Person { id: person_id(e), root: root.clone(), salt: *salt, devices: BTreeMap::new(), revoked_tokens: BTreeMap::new(), delegations: BTreeMap::new() };
        };
        match &e.op {
            Op::Found { .. } => {}
            Op::AddDevice(d) => {
                let id = d.key.id();
                p.devices.insert(id.clone(), Device { id, key: d.key.clone(), kind: d.kind, label: d.label.clone(), added: e.n, removed: None, revoked: None });
            }
            Op::RemoveDevice { device } => {
                if let Some(d) = p.devices.get_mut(device) {
                    d.removed = Some(e.n);
                }
            }
            Op::Revoke(Revocation::Device(device)) => {
                if let Some(d) = p.devices.get_mut(device) {
                    d.revoked = Some(e.n);
                }
            }
            Op::Revoke(Revocation::Token(t)) => {
                p.revoked_tokens.insert(t.clone(), e.n);
            }
            Op::Delegate(d) => {
                p.delegations.insert(d.id.clone(), (d.clone(), e.n));
            }
            Op::Undelegate { id } => {
                p.delegations.remove(id);
            }
        }
        p
    }
}

/// The root key's signer brings a block from a device the person holds,
/// or, pairing one, from the device being paired.
fn presence_from_device(p: &Person, e: &Entry, s: &Signature, what: &str, act: &[u8; 32]) -> R<()> {
    let Some(block) = &s.presence else { return Err(refused("the root key's entries carry a device's presence")) };
    let key = match &e.op {
        Op::AddDevice(d) if block.device == d.key.id() => &d.key,
        _ => &p.active_device(&block.device).ok_or_else(|| refused("the root key's presence is from a device this person doesn't hold"))?.key,
    };
    block.signed_for(key, what, act).map_err(|err| refused(err.message))
}

/// The signers of a delegation's entry: one is enough, the root key or a
/// device (signer_key let only those through). One that widens carries the
/// person's presence: a device's own block, or the root's from a device the
/// person holds. A block that's there is checked either way.
fn delegation_signers(p: &Person, e: &Entry, widens: bool) -> R<()> {
    let act = e.digest();
    let mut present = false;
    for s in &e.signers {
        let Some(block) = &s.presence else { continue };
        if s.by == p.root.id() {
            presence_from_device(p, e, s, DELEGATE, &act)?;
        } else {
            let key = p.active_device(&s.by).map(|d| &d.key).ok_or_else(|| no_device(&s.by))?;
            if block.device != s.by {
                return Err(refused(format!("{}'s presence is another device's", s.by)));
            }
            block.signed_for(key, DELEGATE, &act).map_err(|err| refused(err.message))?;
        }
        present = true;
    }
    if widens && !present {
        return Err(refused("issuing or widening a delegation signs with the person's presence"));
    }
    Ok(())
}

fn root_alone() -> ChainError {
    refused("the root key signs the first entry, alone")
}

/// A person's id, from their chain's entry 0.
pub fn person_id(found: &Entry) -> String {
    format!("prs_{}", base32(&sha256(found.to_json().to_string().as_bytes())[..20]))
}

/// Entry 0 of a new person's chain, signed by their root key, with a
/// project salt of the system's random numbers.
pub fn found(root: &dyn xenonet_id::sign::Signer, at: i64) -> xenonet_id::R<Entry> {
    found_salted(root, xenonet_id::codec::random::<SALT_BYTES>()?, at)
}

/// Entry 0 with the salt given: for known answers and made-up worlds,
/// whose ids are the same each run.
pub fn found_salted(root: &dyn xenonet_id::sign::Signer, salt: [u8; SALT_BYTES], at: i64) -> xenonet_id::R<Entry> {
    let key = root.public();
    let mut e = Entry::new(0, "", Body { op: Op::Found { root: key.clone(), salt }, at });
    e.sign(&key.id(), root, None)?;
    Ok(e)
}

impl PersonChain {
    /// The person, once entry 0 is in.
    pub fn person(&self) -> Option<&Person> {
        self.view_ref()
    }

    pub fn id(&self) -> Option<&str> {
        self.view_ref().map(|p| p.id.as_str())
    }

    /// `device` as an active device of this person when `head` was the
    /// chain's head; None when it wasn't one then, or `head` isn't one of
    /// this chain's.
    pub fn device_at(&self, device: &str, head: &str) -> Option<&Device> {
        let n = self.index_of(head)?;
        self.view_ref()?.device(device).filter(|d| d.active_at(n))
    }

    /// Whether `token` was revoked by the time `head` was the head.
    pub fn revoked_at(&self, token: &str, head: &str) -> bool {
        let Some(n) = self.index_of(head) else { return false };
        self.view_ref().and_then(|p| p.revoked_tokens.get(token)).is_some_and(|r| *r <= n)
    }
}

impl Devices for PersonChain {
    fn person_device(&self, person: &str, head: &str, device: &str) -> Option<Device> {
        if self.id() != Some(person) {
            return None;
        }
        self.device_at(device, head).cloned()
    }
}

#[cfg(test)]
mod tests;
