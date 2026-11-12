//! A space's chain (`xenolith-space/1`; docs/design/direction.md,
//! "Xenolith"): who may read, append to, write and curate one layer of
//! shared context, and which entity it is about. A space has one owner
//! (a person, or, standalone, a key) and exactly one curator at a time,
//! the context key of the Xenolith that files its notes into pages.
//!
//! **The id.** Entry 0, `found`, names what the space is about (an
//! `EntityRef`), its owner, its first curator, a name and a nonce. The
//! space's id is xenonet-model's `SpaceId::of_genesis` of that entry's
//! hash, so every holder derives it and nobody chooses it. Every later
//! entry names the space, so it can't be put on another space's chain.
//!
//! **Two kinds of entry.** A space is a person's, and a person has several
//! devices, so two of their acts can be made apart, in a partition. If the
//! owner extended the chain themselves, those two acts would fork it and
//! split the members for good. So:
//!
//! - **Sequenced entries** (`share`, `bind`, `move-owner`) are signed by
//!   the owner and appended by the current curator with its own signature.
//!   The curator puts concurrent owner acts in one order, so they never
//!   fork. Each names the curator epoch it was sequenced under.
//! - **Side entries** (`unshare`, `take`, `move-curator`) are the owner's
//!   alone and sit beside the chain, valid at any position. An `unshare`
//!   takes effect at each receiver as soon as it holds it, so narrowing
//!   never waits on the curator. `take` and `move-curator` hand the
//!   chain to a new curator from a point on it (`base`), at the next epoch.
//!
//! | entry | signed by | presence (person owner) |
//! |---|---|---|
//! | `found` | its curator, and a key owner too | none |
//! | `share` that widens (a new subject, a higher level, a new endpoint) | the owner and the curator | yes |
//! | `share` that narrows (a lower level for a subject already shared) | the owner and the curator | no |
//! | `bind` (a name to an entity the owner owns, once) | a person owner and the curator | yes |
//! | `move-owner` (a key-owned space to a person) | the key, a device of the person, the curator | yes, the person's |
//! | `unshare` | any owner the space has had, alone | no |
//! | `move-curator` | the owner and the curator at the base | yes |
//! | `take` | the owner alone | yes |
//!
//! A person owner signs with one of their devices, checked against their
//! person chain at the head the entry names (`personHead`), as the
//! network chain checks an admin's. A chain checks presence blocks for
//! what they sign, never their age; the curator, live, checks a block is
//! fresh (`PresenceCheck`) before it sequences the entry. A key owner (a standalone Xenolith's
//! owner key, a file) can't prove presence, so its widenings need none and
//! the view says the space is `weaker`, as decisions on Linux devices are.
//! A key-owned space can't be shared into a network or bound to an entity
//! until `move-owner` gives it to a person; ownership never moves key to
//! key, or from a person (sharing between persons waits for orgs).
//!
//! **Curators.** The chain's effective history is built from everything a
//! holder has, by one rule every member applies the same way: entries of
//! epoch 0 (the founding curator's) from entry 0; at each epoch, of the
//! `take` and `move-curator` entries for the next epoch whose base is on
//! that epoch's run, the one with the lowest id wins; the run is cut after
//! its base, and the winner's curator sequences from there. Two takes made
//! in a partition give one curator everywhere once the members have both.
//! What a superseded curator sequenced after the winning base is kept as
//! evidence and not applied. A curator that signs two different entries
//! at one place has forked its run: the run stops before the fork
//! (`frozen`), reads and notes go on, and only the owner's `take` moves the
//! chain on.
//!
//! **Unsharing.** An `unshare` names a subject. A subject's `share` is in
//! force only while every `unshare` of it the holder has is one a `share`
//! of it lists in `after`, that is, one the owner had seen when they
//! shared it again. So a share made in a partition, beside an unshare made
//! on another device, leaves the unshare in force, and sharing again is
//! one more `share` that lists it. An `unshare` can't remove the current
//! curator's key: that's a `take`.
//!
//! **What a space is about.** A space counts as an entity's (shown as
//! "about E", tied to E on the map) only when its owner owns E by the
//! chains the receiver holds and the owner has signed something in it
//! (`Space::is_about`): anyone can found a space naming any entity and any
//! owner, so a space named about a project by its founder alone is just
//! "about E, owned by X". A name binds to nobody.

use crate::chain::{self, ChainError, Codes, Format, Policy, Signature, MAX_SIGNERS, R};
use crate::network::is_network_id;
use crate::person::{is_key_id, Devices};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use xenonet_id::codec::{b64, is_b64, sha256, unb64, unb64_n};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};
use xenonet_model::ids::{is_actor_slug, is_signing_key_id, is_space_name, EntityRef, SpaceId};

/// An entry of a space's chain.
pub type Entry = chain::Entry<SpacePolicy>;

/// What a presence block approves for a space act that widens.
pub const WIDEN: &str = "widen";
/// The most subjects a space shares with.
pub const MAX_SUBJECTS: usize = 256;
/// The most unshares one share lists as seen.
pub const MAX_AFTER: usize = 8;
/// The most entries and side entries a space holds that aren't on its
/// effective chain (evidence, and what waits for what it follows).
pub const MAX_HELD: usize = 512;
/// The most side entries a space keeps that its owner signed.
pub const MAX_SIDES: usize = 1024;
/// The most takes and moves a space keeps that it can't judge yet, since
/// their base isn't on its chain: anyone may send those, so they're few,
/// and the oldest goes first.
pub const MAX_WAITING: usize = 64;

fn refused(message: impl Into<String>) -> ChainError {
    SpacePolicy::FORMAT.refused(message)
}

fn bad(what: &str) -> ChainError {
    SpacePolicy::FORMAT.bad(what)
}

/// A level of access to a space, lowest first: each allows what the ones
/// below it do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    /// Its pages.
    Read,
    /// And leaving notes for the curator.
    Append,
    /// And proposing changes to pages.
    Write,
    /// And what the owner may hand on: everything but owning it.
    Admin,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Read, Level::Append, Level::Write, Level::Admin];

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Read => "read",
            Level::Append => "append",
            Level::Write => "write",
            Level::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.as_str() == s)
    }
}

/// Whom a space is shared with.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subject {
    /// A person, by their `prs_`.
    Person(String),
    /// A machine, by its signing key's `xns1_`.
    Machine(String),
    /// A network, by its `net_`: its members' Xenoliths, as the receiver
    /// binds them to members.
    Network(String),
    /// An actor of a machine.
    Actor { slug: String, machine: String },
    /// A Xenolith's context key, by its `xns1_`: a standalone peer.
    Key(String),
}

impl Subject {
    pub fn parse(s: &str) -> Option<Subject> {
        let (kind, id) = s.split_once(':')?;
        match kind {
            "person" if xenonet_model::ids::is_person_id(id) => Some(Subject::Person(id.into())),
            "machine" if is_signing_key_id(id) => Some(Subject::Machine(id.into())),
            "network" if is_network_id(id) => Some(Subject::Network(id.into())),
            "key" if is_signing_key_id(id) => Some(Subject::Key(id.into())),
            "actor" => {
                let (slug, machine) = id.split_once('@')?;
                (is_actor_slug(slug) && is_signing_key_id(machine)).then(|| Subject::Actor { slug: slug.into(), machine: machine.into() })
            }
            _ => None,
        }
    }
}

impl fmt::Display for Subject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Subject::Person(id) => write!(f, "person:{id}"),
            Subject::Machine(id) => write!(f, "machine:{id}"),
            Subject::Network(id) => write!(f, "network:{id}"),
            Subject::Actor { slug, machine } => write!(f, "actor:{slug}@{machine}"),
            Subject::Key(id) => write!(f, "key:{id}"),
        }
    }
}

/// Whether `e` is an iroh endpoint id as a share names one: 64 lowercase
/// hex characters.
pub fn is_endpoint_id(e: &str) -> bool {
    e.len() == 64 && e.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Who owns a space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    /// A person, by their `prs_`: their devices sign, with presence.
    Person(String),
    /// A standalone Xenolith's owner key: a file, so weaker.
    Key(VerifyingKey),
}

impl Owner {
    fn to_json(&self) -> Value {
        match self {
            Owner::Person(p) => json!({ "person": p }),
            Owner::Key(k) => json!({ "key": k.to_json() }),
        }
    }

    fn parse(v: &Value) -> Option<Owner> {
        let o = v.as_object().filter(|o| o.len() == 1)?;
        if let Some(p) = o.get("person").and_then(Value::as_str) {
            return xenonet_model::ids::is_person_id(p).then(|| Owner::Person(p.into()));
        }
        o.get("key").and_then(VerifyingKey::parse).map(Owner::Key)
    }

    pub fn person(&self) -> Option<&str> {
        match self {
            Owner::Person(p) => Some(p),
            Owner::Key(_) => None,
        }
    }
}

/// What a sequenced entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Found {
        about: EntityRef,
        owner: Owner,
        curator: VerifyingKey,
        name: String,
        nonce: [u8; 16],
    },
    /// Shares the space with `subject` at `level`, in place of any share
    /// of it before. `endpoint` is a key subject's iroh endpoint id, the
    /// one own mode admits it from. `after` is the unshares of `subject`
    /// the owner had seen, by id, sorted.
    Share {
        subject: Subject,
        level: Level,
        endpoint: Option<String>,
        after: Vec<String>,
    },
    /// Ties a space made under a name to the entity it's about, once.
    Bind {
        about: EntityRef,
    },
    /// Gives a key-owned space to a person.
    MoveOwner {
        to: String,
    },
}

impl Op {
    pub fn as_str(&self) -> &'static str {
        match self {
            Op::Found { .. } => "found",
            Op::Share { .. } => "share",
            Op::Bind { .. } => "bind",
            Op::MoveOwner { .. } => "move-owner",
        }
    }
}

/// What a sequenced entry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// The space's id; empty for `found`, which makes it.
    pub space: String,
    /// The curator epoch it was sequenced under; 0 for `found`.
    pub epoch: u32,
    pub op: Op,
    /// The owner's person chain's head, for a person's device's signature.
    pub person_head: Option<String>,
    /// Milliseconds since 1970, by the signer's clock.
    pub at: i64,
}

/// A subject's share as the chain has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareRow {
    pub level: Level,
    pub endpoint: Option<String>,
    /// The entry that set it.
    pub n: u64,
}

/// A space as its effective chain leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// `spc_…`.
    pub id: String,
    /// Entry 0's hash.
    pub genesis: String,
    pub about: EntityRef,
    pub name: String,
    pub owner: Owner,
    /// Every owner the space has had, the current one last.
    pub owners: Vec<Owner>,
    /// Whether the owner has signed an entry of it, not only been named.
    pub owner_signed: bool,
    pub curator: VerifyingKey,
    pub epoch: u32,
    /// Each subject's last share, by its text.
    pub shares: BTreeMap<String, (Subject, ShareRow)>,
    /// The unshares each subject's shares list as seen, by the subject's
    /// text.
    pub seen: BTreeMap<String, BTreeSet<String>>,
    /// The index of the last entry.
    pub n: u64,
}

impl View {
    /// Whether the owner is a key, which proves no presence.
    pub fn weaker(&self) -> bool {
        matches!(self.owner, Owner::Key(_))
    }
}

/// Who owns an entity, by the chains the receiver holds: a project's
/// person, a machine's network's person, an actor's machine's person. A
/// person owns themselves without asking.
pub trait Owners: Send + Sync {
    fn owner_of(&self, entity: &EntityRef) -> Option<String>;
}

/// A space chain's rules, with the person chains its owners' devices are
/// checked against and, for `bind`, who owns what.
#[derive(Clone, Default)]
pub struct SpacePolicy {
    devices: Option<Arc<dyn Devices>>,
    owners: Option<Arc<dyn Owners>>,
}

impl fmt::Debug for SpacePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SpacePolicy({}, {})",
            if self.devices.is_some() { "with person chains" } else { "no person chain" },
            if self.owners.is_some() { "with owners" } else { "no owners" }
        )
    }
}

impl SpacePolicy {
    pub fn new(devices: Option<Arc<dyn Devices>>, owners: Option<Arc<dyn Owners>>) -> SpacePolicy {
        SpacePolicy { devices, owners }
    }

    /// `by`'s key as a device of `person`, active at `head`.
    fn device(&self, person: &str, head: Option<&str>, by: &str) -> Option<VerifyingKey> {
        let head = head?;
        self.devices.as_ref()?.person_device(person, head, by).map(|d| d.key)
    }

    /// The key `by` signs with as `owner` (a device of a person owner at
    /// `head`, or a key owner itself).
    fn owner_key(&self, owner: &Owner, head: Option<&str>, by: &str) -> Option<VerifyingKey> {
        match owner {
            Owner::Key(k) => (k.id() == by).then(|| k.clone()),
            Owner::Person(p) => self.device(p, head, by),
        }
    }
}

/// Whether `s` brings presence for `what` on the act `act` from the
/// device `key` it signed with.
fn present(s: &Signature, key: &VerifyingKey, act: &[u8; 32]) -> Result<(), String> {
    let Some(block) = &s.presence else { return Err(format!("{} signed without the person's presence", s.by)) };
    if block.device != s.by {
        return Err(format!("{}'s presence is another device's", s.by));
    }
    block.signed_for(key, WIDEN, act).map_err(|e| e.message)
}

fn presence_refused(why: String) -> ChainError {
    ChainError::new(403, "presence", format!("that widens the space, so it needs the person there: {why}"))
}

fn write_op(op: &Op, out: &mut Map<String, Value>) {
    match op {
        Op::Found { about, owner, curator, name, nonce } => {
            out.insert("about".into(), Value::String(about.to_string()));
            out.insert("owner".into(), owner.to_json());
            out.insert("curator".into(), curator.to_json());
            out.insert("name".into(), Value::String(name.clone()));
            out.insert("nonce".into(), Value::String(b64(nonce)));
        }
        Op::Share { subject, level, endpoint, after } => {
            out.insert("subject".into(), Value::String(subject.to_string()));
            out.insert("level".into(), Value::String(level.as_str().into()));
            out.insert("endpoint".into(), endpoint.as_ref().map_or(Value::Null, |e| Value::String(e.clone())));
            out.insert("after".into(), json!(after));
        }
        Op::Bind { about } => {
            out.insert("about".into(), Value::String(about.to_string()));
        }
        Op::MoveOwner { to } => {
            out.insert("to".into(), Value::String(to.clone()));
        }
    }
}

fn after_ok(after: &[String]) -> bool {
    after.len() <= MAX_AFTER && after.windows(2).all(|w| w[0] < w[1]) && after.iter().all(|a| a.len() == 43 && is_b64(a))
}

fn parse_op(v: &Value) -> R<Op> {
    let text = |k: &str| v.get(k).and_then(Value::as_str);
    Ok(match text("op") {
        Some("found") => Op::Found {
            about: text("about").and_then(|a| EntityRef::parse(a).ok()).ok_or_else(|| bad("about"))?,
            owner: v.get("owner").and_then(Owner::parse).ok_or_else(|| bad("owner"))?,
            curator: v.get("curator").and_then(VerifyingKey::parse).ok_or_else(|| bad("curator"))?,
            name: text("name").filter(|n| is_space_name(n)).ok_or_else(|| bad("name"))?.into(),
            nonce: text("nonce").and_then(unb64_n::<16>).ok_or_else(|| bad("nonce"))?,
        },
        Some("share") => {
            let endpoint = match v.get("endpoint") {
                None | Some(Value::Null) => None,
                Some(e) => Some(e.as_str().filter(|e| is_endpoint_id(e)).ok_or_else(|| bad("endpoint"))?.to_string()),
            };
            let after: Vec<String> = v
                .get("after")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("after"))?
                .iter()
                .map(|a| a.as_str().map(str::to_string))
                .collect::<Option<_>>()
                .filter(|a: &Vec<String>| after_ok(a))
                .ok_or_else(|| bad("after"))?;
            Op::Share {
                subject: text("subject").and_then(Subject::parse).ok_or_else(|| bad("subject"))?,
                level: text("level").and_then(Level::parse).ok_or_else(|| bad("level"))?,
                endpoint,
                after,
            }
        }
        Some("bind") => Op::Bind { about: text("about").and_then(|a| EntityRef::parse(a).ok()).ok_or_else(|| bad("about"))? },
        Some("move-owner") => Op::MoveOwner { to: text("to").filter(|p| xenonet_model::ids::is_person_id(p)).ok_or_else(|| bad("owner"))?.into() },
        _ => return Err(bad("op")),
    })
}

fn person_head_json(head: Option<&String>) -> Value {
    head.map_or(Value::Null, |h| Value::String(h.clone()))
}

fn parse_person_head(v: &Value) -> R<Option<String>> {
    match v.get("personHead") {
        None | Some(Value::Null) => Ok(None),
        Some(h) => h.as_str().filter(|h| !h.is_empty() && is_b64(h)).map(|h| Some(h.to_string())).ok_or_else(|| bad("person head")),
    }
}

impl Policy for SpacePolicy {
    type Body = Body;
    type State = View;

    const FORMAT: Format = Format {
        tag: Purpose::SPACE,
        noun: "space chain",
        signer: "key",
        signer_key: "key",
        presence: true,
        codes: Codes { gap: "space-gap", fork: "space-fork", refused: "space-refused" },
    };

    fn write_body(b: &Body, out: &mut Map<String, Value>) {
        if !matches!(b.op, Op::Found { .. }) {
            out.insert("space".into(), Value::String(b.space.clone()));
            out.insert("epoch".into(), Value::from(b.epoch));
        }
        out.insert("op".into(), Value::String(b.op.as_str().into()));
        write_op(&b.op, out);
        if !matches!(b.op, Op::Found { .. }) {
            out.insert("personHead".into(), person_head_json(b.person_head.as_ref()));
        }
        out.insert("at".into(), Value::from(b.at));
    }

    fn parse_body(v: &Value) -> R<Body> {
        let op = parse_op(v)?;
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        if matches!(op, Op::Found { .. }) {
            return Ok(Body { space: String::new(), epoch: 0, op, person_head: None, at });
        }
        let space = v.get("space").and_then(Value::as_str).filter(|s| SpaceId::parse(s).is_ok()).ok_or_else(|| bad("space"))?;
        let epoch = v.get("epoch").and_then(Value::as_u64).and_then(|e| u32::try_from(e).ok()).ok_or_else(|| bad("epoch"))?;
        Ok(Body { space: space.into(), epoch, op, person_head: parse_person_head(v)?, at })
    }

    fn body_well_formed(b: &Body) -> R<()> {
        match &b.op {
            Op::Found { name, .. } if !is_space_name(name) => return Err(bad("name")),
            Op::Found { .. } if !b.space.is_empty() || b.epoch != 0 || b.person_head.is_some() => return Err(bad("founding")),
            Op::Found { .. } => return Ok(()),
            Op::Share { endpoint, after, .. } if endpoint.as_deref().is_some_and(|e| !is_endpoint_id(e)) || !after_ok(after) => return Err(bad("share")),
            Op::MoveOwner { to } if !xenonet_model::ids::is_person_id(to) => return Err(bad("owner")),
            _ => {}
        }
        if SpaceId::parse(&b.space).is_err() {
            return Err(bad("space"));
        }
        if b.person_head.as_deref().is_some_and(|h| h.is_empty() || !is_b64(h)) {
            return Err(bad("person head"));
        }
        Ok(())
    }

    fn is_signer(id: &str) -> bool {
        is_key_id(id)
    }

    fn sig_ok(sig: &[u8]) -> bool {
        (64..=72).contains(&sig.len())
    }

    fn same(a: &Body, b: &Body) -> bool {
        a.space == b.space && a.epoch == b.epoch && a.op == b.op && a.person_head == b.person_head
    }

    fn scope(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(v) = view else {
            let Op::Found { owner, curator, .. } = &e.op else { return Err(refused("a space's chain starts with its founding")) };
            let mut want = vec![curator.id()];
            if let Owner::Key(k) = owner {
                if k.id() != curator.id() {
                    want.push(k.id());
                }
            }
            let mut got: Vec<String> = e.signers.iter().map(|s| s.by.clone()).collect();
            want.sort();
            got.sort();
            if want != got || e.signers.iter().any(|s| s.presence.is_some()) {
                return Err(refused("a space is founded by its curator, with a key owner's signature too, and nobody else"));
            }
            return Ok(());
        };
        if matches!(e.op, Op::Found { .. }) {
            return Err(refused("a space is founded once"));
        }
        if e.space != v.id {
            return Err(refused("that entry is for another space"));
        }
        if e.epoch != v.epoch {
            return Err(ChainError::new(409, "space-epoch", format!("that entry was sequenced under curator epoch {}, and the space is at {}", e.epoch, v.epoch)));
        }
        Ok(())
    }

    fn signer_key(&self, view: Option<&View>, e: &Entry, by: &str) -> R<VerifyingKey> {
        let Some(v) = view else {
            let Op::Found { owner, curator, .. } = &e.op else { return Err(refused("a space's chain starts with its founding")) };
            if by == curator.id() {
                return Ok(curator.clone());
            }
            return match owner {
                Owner::Key(k) if k.id() == by => Ok(k.clone()),
                _ => Err(refused(format!("{by} doesn't found this space"))),
            };
        };
        if by == v.curator.id() {
            return Ok(v.curator.clone());
        }
        if let Some(k) = self.owner_key(&v.owner, e.person_head.as_deref(), by) {
            return Ok(k);
        }
        if let Op::MoveOwner { to } = &e.op {
            if let Some(k) = self.device(to, e.person_head.as_deref(), by) {
                return Ok(k);
            }
        }
        Err(refused(format!("{by} isn't this space's curator, or a key or device of its owner at the person head the entry names")))
    }

    fn check(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(v) = view else { return Ok(()) };
        if !e.signers.iter().any(|s| s.by == v.curator.id()) {
            return Err(refused("the curator sequences a space's entries: it signs each"));
        }
        let owner_sigs: Vec<(&Signature, VerifyingKey)> = e.signers.iter().filter_map(|s| self.owner_key(&v.owner, e.person_head.as_deref(), &s.by).map(|k| (s, k))).collect();
        if owner_sigs.is_empty() {
            return Err(ChainError::needs_approval("that needs the space's owner"));
        }
        let act = e.digest();
        // What widens takes the person there: a presence block from one of
        // the owner's devices that signed. A key owner has none to bring.
        let widen = |sigs: &[(&Signature, VerifyingKey)]| -> R<()> {
            let mut why = None;
            for (s, key) in sigs {
                match present(s, key, &act) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        why.get_or_insert(e);
                    }
                }
            }
            Err(presence_refused(why.unwrap_or_default()))
        };
        match &e.op {
            Op::Found { .. } => Err(refused("a space is founded once")),
            Op::Share { subject, level, endpoint, after } => {
                if matches!(subject, Subject::Network(_)) && v.weaker() {
                    return Err(refused("a space its owner key owns is shared into a network only once a person owns it (move-owner)"));
                }
                if matches!(subject, Subject::Key(k) if *k == v.curator.id()) {
                    return Err(ChainError::no_change("the curator reads and writes its space already"));
                }
                if endpoint.is_some() && !matches!(subject, Subject::Key(_)) {
                    return Err(bad("endpoint"));
                }
                let key = subject.to_string();
                let old = v.shares.get(&key).map(|(_, r)| r);
                if old.is_none() && v.shares.len() >= MAX_SUBJECTS {
                    return Err(refused(format!("a space shares with {MAX_SUBJECTS} subjects at most")));
                }
                let seen = v.seen.get(&key);
                let new_seen = after.iter().any(|a| seen.is_none_or(|s| !s.contains(a)));
                if old.is_some_and(|o| o.level == *level && o.endpoint == *endpoint) && !new_seen {
                    return Err(ChainError::no_change(format!("the space is shared with {key} so already")));
                }
                let narrows = old.is_some_and(|o| *level <= o.level && o.endpoint == *endpoint) && !new_seen;
                match (&v.owner, narrows) {
                    (Owner::Person(_), false) => widen(&owner_sigs),
                    _ => Ok(()),
                }
            }
            Op::Bind { about } => {
                if v.about.is_bound() {
                    return Err(ChainError::taken(format!("the space is about {} already", v.about)));
                }
                if !about.is_bound() {
                    return Err(bad("about"));
                }
                let Owner::Person(p) = &v.owner else {
                    return Err(refused("a space its owner key owns binds to an entity only once a person owns it (move-owner)"));
                };
                let owns = match about {
                    EntityRef::Person(q) => q == p,
                    _ => self.owners.as_ref().and_then(|o| o.owner_of(about)).is_some_and(|q| q == *p),
                };
                if !owns {
                    return Err(refused(format!("{p} doesn't own {about} by the chains held here, so the space can't be tied to it")));
                }
                widen(&owner_sigs)
            }
            Op::MoveOwner { to } => {
                if !v.weaker() {
                    return Err(refused("a space moves from its owner key to a person, once; a person's space stays theirs"));
                }
                let devices: Vec<(&Signature, VerifyingKey)> = e.signers.iter().filter_map(|s| self.device(to, e.person_head.as_deref(), &s.by).map(|k| (s, k))).collect();
                if devices.is_empty() {
                    return Err(ChainError::needs_approval(format!("that needs a device of {to}'s")));
                }
                widen(&devices)
            }
        }
    }

    fn apply(&self, view: Option<View>, e: &Entry) -> View {
        let Some(mut v) = view else {
            let Op::Found { about, owner, curator, name, .. } = &e.op else { unreachable!("scope takes only `found` as entry 0") };
            let hash = e.hash();
            return View {
                id: space_id(&hash),
                genesis: hash,
                about: about.clone(),
                name: name.clone(),
                owner: owner.clone(),
                owners: vec![owner.clone()],
                owner_signed: matches!(owner, Owner::Key(_)),
                curator: curator.clone(),
                epoch: 0,
                shares: BTreeMap::new(),
                seen: BTreeMap::new(),
                n: 0,
            };
        };
        v.owner_signed = true;
        v.n = e.n;
        match &e.op {
            Op::Found { .. } => {}
            Op::Share { subject, level, endpoint, after } => {
                let key = subject.to_string();
                v.seen.entry(key.clone()).or_default().extend(after.iter().cloned());
                v.shares.insert(key, (subject.clone(), ShareRow { level: *level, endpoint: endpoint.clone(), n: e.n }));
            }
            Op::Bind { about } => v.about = about.clone(),
            Op::MoveOwner { to } => {
                v.owner = Owner::Person(to.clone());
                v.owners.push(v.owner.clone());
            }
        }
        v
    }
}

/// The space whose entry 0 has `genesis` as its hash (base64url).
pub fn space_id(genesis: &str) -> String {
    let bytes = unb64_n::<32>(genesis).unwrap_or_default();
    SpaceId::of_genesis(&bytes).to_string()
}

/// Entry 0 of a new space, signed by its curator and, for a key owner, by
/// that key too. `nonce` makes two spaces with the same names two ids.
pub fn found(about: EntityRef, owner: Owner, curator: &dyn Signer, owner_key: Option<&dyn Signer>, name: &str, nonce: [u8; 16], at: i64) -> xenonet_id::R<Entry> {
    let key = curator.public();
    let mut e = Entry::new(
        0,
        "",
        Body { space: String::new(), epoch: 0, op: Op::Found { about, owner: owner.clone(), curator: key.clone(), name: name.into(), nonce }, person_head: None, at },
    );
    e.sign(&key.id(), curator, None)?;
    if let (Owner::Key(k), Some(signer)) = (&owner, owner_key) {
        if k.id() != key.id() {
            e.sign(&k.id(), signer, None)?;
        }
    }
    Ok(e)
}

// ---- Side entries ----

/// Where a curator hands over: an entry of the effective chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Base {
    pub n: u64,
    pub hash: String,
}

/// What a side entry does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideOp {
    /// Stops sharing the space with a subject.
    Unshare { subject: Subject },
    /// The chain goes to `to` after `base`, at `epoch`: the curator at the
    /// base agrees (`move`) or not (`take`).
    Curator { take: bool, epoch: u32, to: VerifyingKey, base: Base },
}

/// A side entry: the owner's, beside the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    pub space: String,
    pub op: SideOp,
    pub person_head: Option<String>,
    pub at: i64,
    pub signers: Vec<Signature>,
}

impl Side {
    pub fn new(space: &str, op: SideOp, person_head: Option<String>, at: i64) -> Side {
        Side { space: space.into(), op, person_head, at, signers: Vec::new() }
    }

    fn unsigned(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("space".into(), Value::String(self.space.clone()));
        match &self.op {
            SideOp::Unshare { subject } => {
                m.insert("op".into(), Value::String("unshare".into()));
                m.insert("subject".into(), Value::String(subject.to_string()));
            }
            SideOp::Curator { take, epoch, to, base } => {
                m.insert("op".into(), Value::String(if *take { "take" } else { "move-curator" }.into()));
                m.insert("epoch".into(), Value::from(*epoch));
                m.insert("to".into(), to.to_json());
                m.insert("base".into(), json!({ "n": base.n, "hash": base.hash }));
            }
        }
        m.insert("personHead".into(), person_head_json(self.person_head.as_ref()));
        m.insert("at".into(), Value::from(self.at));
        m
    }

    /// What each signer signs: the space tag, a newline, the canonical
    /// JSON without the signers. It starts with `space`, where a chain
    /// entry starts with `n`, so neither's signature is the other's.
    pub fn signing_bytes(&self) -> Vec<u8> {
        Purpose::SPACE.bytes(Value::Object(self.unsigned()).to_string().as_bytes())
    }

    pub fn digest(&self) -> [u8; 32] {
        sha256(&self.signing_bytes())
    }

    /// Its id: what an unshare is listed by, and what competing takes are
    /// ranked by.
    pub fn id(&self) -> String {
        b64(&self.digest())
    }

    pub fn sign(&mut self, by: &str, key: &dyn Signer, presence: Option<PresenceBlock>) -> xenonet_id::R<()> {
        let sig = key.sign(Purpose::SPACE, Value::Object(self.unsigned()).to_string().as_bytes())?;
        self.signers.retain(|s| s.by != by);
        self.signers.push(Signature { by: by.into(), sig, presence });
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut m = self.unsigned();
        let signers = self.signers.iter().map(|s| {
            let mut o = Map::new();
            o.insert("key".into(), Value::String(s.by.clone()));
            o.insert("sig".into(), Value::String(b64(&s.sig)));
            if let Some(p) = &s.presence {
                o.insert("presence".into(), p.to_json());
            }
            Value::Object(o)
        });
        m.insert("signers".into(), Value::Array(signers.collect()));
        Value::Object(m)
    }

    pub fn parse(v: &Value) -> R<Side> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let space = text("space").filter(|s| SpaceId::parse(s).is_ok()).ok_or_else(|| bad("space"))?;
        let op = match text("op") {
            Some("unshare") => SideOp::Unshare { subject: text("subject").and_then(Subject::parse).ok_or_else(|| bad("subject"))? },
            Some(kind @ ("take" | "move-curator")) => {
                let base = v.get("base").ok_or_else(|| bad("base"))?;
                SideOp::Curator {
                    take: kind == "take",
                    epoch: v.get("epoch").and_then(Value::as_u64).and_then(|e| u32::try_from(e).ok()).filter(|e| *e > 0).ok_or_else(|| bad("epoch"))?,
                    to: v.get("to").and_then(VerifyingKey::parse).ok_or_else(|| bad("curator"))?,
                    base: Base {
                        n: base.get("n").and_then(Value::as_u64).ok_or_else(|| bad("base"))?,
                        hash: base.get("hash").and_then(Value::as_str).filter(|h| h.len() == 43 && is_b64(h)).ok_or_else(|| bad("base"))?.into(),
                    },
                }
            }
            _ => return Err(bad("op")),
        };
        let list = v.get("signers").and_then(Value::as_array).filter(|s| !s.is_empty() && s.len() <= MAX_SIGNERS).ok_or_else(|| bad("signers"))?;
        let mut signers = Vec::new();
        for s in list {
            let by = s.get("key").and_then(Value::as_str).filter(|k| is_key_id(k)).ok_or_else(|| bad("signer"))?;
            let sig = s.get("sig").and_then(Value::as_str).and_then(unb64).filter(|x| (64..=72).contains(&x.len())).ok_or_else(|| bad("signature"))?;
            let presence = match s.get("presence") {
                Some(p) if !p.is_null() => Some(PresenceBlock::parse(p).ok_or_else(|| bad("presence"))?),
                _ => None,
            };
            signers.push(Signature { by: by.into(), sig, presence });
        }
        Ok(Side { space: space.into(), op, person_head: parse_person_head(v)?, at: v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?, signers })
    }

    // Signers each once, one to four, each signature checking against the
    // key `key_of` gives (refused when it gives none).
    fn signatures(&self, key_of: impl Fn(&str) -> Option<VerifyingKey>) -> R<Vec<(&Signature, VerifyingKey)>> {
        if self.signers.is_empty() || self.signers.len() > MAX_SIGNERS {
            return Err(bad("signers"));
        }
        let msg = self.signing_bytes();
        let mut out: Vec<(&Signature, VerifyingKey)> = Vec::new();
        for s in &self.signers {
            if out.iter().any(|(o, _)| o.by == s.by) {
                return Err(refused("a side entry is signed by each key once"));
            }
            let key = key_of(&s.by).ok_or_else(|| refused(format!("{} can't sign this space's side entries", s.by)))?;
            if !key.verify_raw(&msg, &s.sig) {
                return Err(refused(format!("{}'s signature doesn't check", s.by)));
            }
            out.push((s, key));
        }
        Ok(out)
    }
}

/// What happened to an entry or a side entry offered to a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    /// On the effective chain, or in force.
    Applied,
    /// Held already.
    Duplicate,
    /// Kept, not applied: it follows what isn't held yet, its epoch's
    /// curator isn't known yet, or it's evidence of a branch that lost.
    Held,
}

/// What a holder knows of a member, for its level: its context key, its
/// machine, the person whose device it is, the networks it's admitted from
/// and the actor it is. The caller fills these from what it checked (a
/// forwarded connection's machine and networks, a grant's `key(id)`), never
/// from the member's word.
#[derive(Debug, Clone, Default)]
pub struct Member<'a> {
    pub key: Option<&'a str>,
    pub machine: Option<&'a str>,
    pub person: Option<&'a str>,
    pub networks: &'a [&'a str],
    pub actor: Option<(&'a str, &'a str)>,
}

/// A space as one holder has it: its effective chain, the side entries in
/// force, and what it holds beside them.
#[derive(Debug, Clone)]
pub struct Space {
    policy: SpacePolicy,
    path: Vec<Entry>,
    /// The view after each entry of `path`.
    states: Vec<View>,
    /// Entries off the effective chain.
    held: Vec<Entry>,
    /// Side entries the owner signed, in the order this holder stored
    /// them, with that order.
    sides: Vec<(Side, u64)>,
    /// Takes and moves whose base isn't on the chain yet.
    waiting: Vec<Side>,
    stored: u64,
    frozen: Option<u64>,
    /// The ids of the takes and moves in force, oldest first.
    winners: Vec<String>,
}

impl Space {
    /// A space from its entry 0.
    pub fn found(policy: SpacePolicy, genesis: Entry) -> R<Space> {
        if genesis.n != 0 || !genesis.prev.is_empty() {
            return Err(refused("a space's chain starts with its founding"));
        }
        let state = Self::take(&policy, None, &genesis)?;
        Ok(Space { policy, path: vec![genesis], states: vec![state], held: Vec::new(), sides: Vec::new(), waiting: Vec::new(), stored: 0, frozen: None, winners: Vec::new() })
    }

    // The checks Chain::would_take makes, against `state`, then the view
    // after `e`.
    fn take(policy: &SpacePolicy, state: Option<&View>, e: &Entry) -> R<View> {
        e.well_formed()?;
        let mut seen: Vec<&str> = Vec::new();
        for s in &e.signers {
            if seen.contains(&s.by.as_str()) {
                return Err(refused("an entry is signed by each key once"));
            }
            seen.push(&s.by);
        }
        policy.scope(state, e)?;
        let msg = e.signing_bytes();
        for s in &e.signers {
            let key = policy.signer_key(state, e, &s.by)?;
            if !key.verify_raw(&msg, &s.sig) {
                return Err(policy.bad_signature(state, &s.by));
            }
        }
        policy.check(state, e)?;
        Ok(policy.apply(state.cloned(), e))
    }

    /// The effective view.
    pub fn view(&self) -> &View {
        self.states.last().expect("a space has its founding")
    }

    pub fn id(&self) -> &str {
        &self.view().id
    }

    /// The effective chain, from entry 0.
    pub fn entries(&self) -> &[Entry] {
        &self.path
    }

    /// What's held off the effective chain.
    pub fn held(&self) -> &[Entry] {
        &self.held
    }

    /// The side entries held, in store order.
    pub fn sides(&self) -> impl Iterator<Item = &Side> {
        self.sides.iter().map(|(s, _)| s)
    }

    pub fn head(&self) -> String {
        self.path.last().map(Entry::hash).unwrap_or_default()
    }

    pub fn len(&self) -> u64 {
        self.path.len() as u64
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// Where the curator forked its run: the index of the entry it signed
    /// twice. Nothing past it is applied until the owner's take.
    pub fn frozen(&self) -> Option<u64> {
        self.frozen
    }

    /// The next entry for the current curator to sequence, unsigned.
    pub fn next(&self, op: Op, person_head: Option<String>, at: i64) -> Entry {
        let v = self.view();
        Entry::new(self.len(), self.head(), Body { space: v.id.clone(), epoch: v.epoch, op, person_head, at })
    }

    /// A take or a move to `to` from the head, unsigned.
    pub fn curator_side(&self, take: bool, to: VerifyingKey, person_head: Option<String>, at: i64) -> Side {
        let v = self.view();
        Side::new(&v.id, SideOp::Curator { take, epoch: v.epoch + 1, to, base: Base { n: self.len() - 1, hash: self.head() } }, person_head, at)
    }

    /// An unshare of `subject`, unsigned.
    pub fn unshare(&self, subject: Subject, person_head: Option<String>, at: i64) -> Side {
        Side::new(self.id(), SideOp::Unshare { subject }, person_head, at)
    }

    /// The unshares of `subject` held here, by id: what a share of it
    /// lists as seen.
    pub fn unshares_of(&self, subject: &Subject) -> Vec<String> {
        let mut ids: Vec<String> = self.sides().filter(|s| matches!(&s.op, SideOp::Unshare { subject: x } if x == subject)).map(Side::id).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// Whether `key` curated an epoch before the current one: a returning
    /// curator that sees this stops filing.
    pub fn superseded(&self, key: &str) -> bool {
        let founder = match &self.path[0].op {
            Op::Found { curator, .. } => curator.id() == key,
            _ => false,
        };
        let took = self.sides().any(|s| self.is_winner(&s.id()) && matches!(&s.op, SideOp::Curator { to, .. } if to.id() == key));
        self.view().curator.id() != key && (founder || took)
    }

    /// Takes an entry: on the effective chain when it extends it, else
    /// held. Refused when it's malformed, for another space, or extends
    /// the head and breaks the rules there.
    pub fn offer(&mut self, e: Entry) -> R<Placed> {
        e.well_formed()?;
        let hash = e.hash();
        if self.path.iter().chain(&self.held).any(|x| x.hash() == hash) {
            return Ok(Placed::Duplicate);
        }
        if matches!(e.op, Op::Found { .. }) || e.space != self.view().id {
            return Err(refused("that entry is for another space"));
        }
        // An entry that extends the head under the current epoch is
        // judged now; anything else waits beside the chain.
        if e.n == self.len() && e.prev == self.head() && e.epoch == self.view().epoch && self.frozen.is_none() {
            Self::take(&self.policy, Some(self.view()), &e)?;
        }
        if self.held.len() >= MAX_HELD {
            self.held.remove(0);
        }
        self.held.push(e);
        self.rebuild();
        Ok(if self.path.iter().any(|x| x.hash() == hash) { Placed::Applied } else { Placed::Held })
    }

    /// Takes a side entry: an unshare in force from now, here, or a take
    /// or move ranked with the others for its epoch. Refused when it's
    /// malformed or not the owner's.
    pub fn offer_side(&mut self, s: Side) -> R<Placed> {
        let id = s.id();
        if self.sides().chain(&self.waiting).any(|x| x.id() == id) {
            return Ok(Placed::Duplicate);
        }
        if s.space != self.view().id {
            return Err(refused("that side entry is for another space"));
        }
        if self.sides.len() >= MAX_SIDES {
            return Err(refused(format!("a space keeps {MAX_SIDES} side entries at most")));
        }
        let placed = match &s.op {
            SideOp::Unshare { subject } => {
                self.check_unshare(&s, subject)?;
                Placed::Applied
            }
            SideOp::Curator { base, epoch, .. } => {
                // Judged against the view at its base when that's on the
                // effective chain; a base not held yet waits.
                match self.state_at(base) {
                    Some(at) if at.epoch + 1 == *epoch => {
                        self.check_curator(at, &s)?;
                        Placed::Applied
                    }
                    _ => Placed::Held,
                }
            }
        };
        if placed == Placed::Held {
            if self.waiting.len() >= MAX_WAITING {
                self.waiting.remove(0);
            }
            self.waiting.push(s);
            self.rebuild();
            return Ok(if self.is_winner(&id) { Placed::Applied } else { Placed::Held });
        }
        self.sides.push((s, self.stored));
        self.stored += 1;
        self.rebuild();
        Ok(match placed {
            Placed::Applied if matches!(self.sides.last().map(|(s, _)| &s.op), Some(SideOp::Curator { .. })) && !self.is_winner(&id) => Placed::Held,
            p => p,
        })
    }

    fn is_winner(&self, id: &str) -> bool {
        self.winners.iter().any(|w| w == id)
    }

    /// The ids of the takes and moves in force, oldest first.
    pub fn curators_by(&self) -> &[String] {
        &self.winners
    }

    // The view at `base` on the effective chain, if it's there.
    fn state_at(&self, base: &Base) -> Option<&View> {
        let i = usize::try_from(base.n).ok()?;
        (self.path.get(i)?.hash() == base.hash).then(|| &self.states[i])
    }

    // An unshare is the owner's, any owner the space has had, alone: it
    // narrows, so it takes no presence and no curator. Never the current
    // curator's own key.
    fn check_unshare(&self, s: &Side, subject: &Subject) -> R<()> {
        let v = self.view();
        if matches!(subject, Subject::Key(k) if *k == v.curator.id()) {
            return Err(refused("an unshare can't remove the curator: take the space to another curator instead"));
        }
        let owners = v.owners.clone();
        let policy = &self.policy;
        s.signatures(|by| owners.iter().rev().find_map(|o| policy.owner_key(o, s.person_head.as_deref(), by)))?;
        Ok(())
    }

    // A take is the owner's alone, a move the owner's and the curator's
    // at the base; both widen, so a person owner's device brings presence.
    fn check_curator(&self, at: &View, s: &Side) -> R<()> {
        let SideOp::Curator { take, to, .. } = &s.op else { return Err(bad("op")) };
        if to.id() == at.curator.id() {
            return Err(ChainError::no_change("that's the curator already"));
        }
        let policy = &self.policy;
        let signed = s.signatures(|by| {
            if !*take && by == at.curator.id() {
                return Some(at.curator.clone());
            }
            policy.owner_key(&at.owner, s.person_head.as_deref(), by)
        })?;
        let owner = signed.iter().find(|(x, _)| x.by != at.curator.id() || matches!(&at.owner, Owner::Key(k) if k.id() == x.by));
        let Some((owner_sig, key)) = owner else { return Err(ChainError::needs_approval("that needs the space's owner")) };
        if !*take && !signed.iter().any(|(x, _)| x.by == at.curator.id()) {
            return Err(ChainError::needs_approval("a move needs the curator it moves from; without it, it's a take"));
        }
        if let Owner::Person(_) = &at.owner {
            present(owner_sig, key, &s.digest()).map_err(presence_refused)?;
        }
        Ok(())
    }

    // Builds the effective chain again from everything held: each epoch's
    // run from where it starts, cut at the base of the next epoch's
    // winning take or move (the lowest id among those whose base is on the
    // run), until no epoch follows. The same entries give the same chain on
    // every holder, whatever order they came in.
    fn rebuild(&mut self) {
        let mut pool: Vec<Entry> = self.path.drain(1..).chain(self.held.drain(..)).collect();
        self.states.truncate(1);
        self.frozen = None;
        self.winners.clear();
        let mut start = 0usize;
        loop {
            // The current epoch's run.
            loop {
                let (n, head, view) = (self.path.len() as u64, self.head(), self.view().clone());
                let fits: Vec<usize> = (0..pool.len()).filter(|&i| pool[i].n == n && pool[i].prev == head && pool[i].epoch == view.epoch).collect();
                let valid: Vec<(usize, View)> = fits.into_iter().filter_map(|i| Self::take(&self.policy, Some(&view), &pool[i]).ok().map(|s| (i, s))).collect();
                // Copies of one entry with other signers on are one entry,
                // the lowest hash; two that say different things are the
                // curator forking its run.
                if valid.iter().any(|(i, _)| pool[*i].digest() != pool[valid[0].0].digest()) {
                    self.frozen = Some(n);
                    break;
                }
                let Some((i, state)) = valid.into_iter().min_by_key(|(i, _)| pool[*i].hash()) else { break };
                self.path.push(pool.remove(i));
                self.states.push(state);
            }
            // The next epoch's winner, if any.
            let epoch = self.view().epoch;
            let mut best: Option<([u8; 32], Side, usize)> = None;
            for s in self.sides().chain(&self.waiting) {
                let SideOp::Curator { epoch: e, base, .. } = &s.op else { continue };
                let Ok(i) = usize::try_from(base.n) else { continue };
                if *e != epoch + 1 || i < start || i >= self.path.len() || self.path[i].hash() != base.hash {
                    continue;
                }
                if self.check_curator(&self.states[i], s).is_err() {
                    continue;
                }
                let digest = s.digest();
                if best.as_ref().is_none_or(|(b, _, _)| digest < *b) {
                    best = Some((digest, s.clone(), i));
                }
            }
            let Some((_, winner, i)) = best else { break };
            self.winners.push(winner.id());
            let SideOp::Curator { to, .. } = &winner.op else { break };
            pool.extend(self.path.drain(i + 1..));
            self.states.truncate(i + 1);
            let state = &mut self.states[i];
            state.epoch = epoch + 1;
            state.curator = to.clone();
            self.frozen = None;
            start = i;
        }
        self.held = pool;
        // A waiting take or move the chain now reaches was judged above:
        // the owner's, it's kept with the others.
        let waiting = std::mem::take(&mut self.waiting);
        for s in waiting {
            let SideOp::Curator { base, epoch, .. } = &s.op else { continue };
            let judged = self.state_at(base).filter(|at| at.epoch + 1 == *epoch).map(|at| self.check_curator(at, &s).is_ok());
            match judged {
                Some(true) => {
                    self.sides.push((s, self.stored));
                    self.stored += 1;
                }
                Some(false) => {}
                None => self.waiting.push(s),
            }
        }
    }

    /// `subject`'s level: its last share, unless an unshare of it held here
    /// is one no share of it lists as seen.
    pub fn level(&self, subject: &Subject) -> Option<Level> {
        let v = self.view();
        let key = subject.to_string();
        let (_, row) = v.shares.get(&key)?;
        let seen = v.seen.get(&key);
        let unshared = self.sides().any(|s| matches!(&s.op, SideOp::Unshare { subject: x } if x == subject) && seen.is_none_or(|seen| !seen.contains(&s.id())));
        (!unshared).then_some(row.level)
    }

    /// When this holder stored the unshare in force for `subject`, in its
    /// own order of side entries: a holder serves the subject nothing
    /// stored after it.
    pub fn unshared_at(&self, subject: &Subject) -> Option<u64> {
        let seen = self.view().seen.get(&subject.to_string());
        self.sides
            .iter()
            .filter(|(s, _)| matches!(&s.op, SideOp::Unshare { subject: x } if x == subject) && seen.is_none_or(|seen| !seen.contains(&s.id())))
            .map(|(_, at)| *at)
            .min()
    }

    /// A member's level: admin for the curator and the owner, else the
    /// highest of the shares that name what it is.
    pub fn level_for(&self, m: &Member<'_>) -> Option<Level> {
        let v = self.view();
        let owner = match &v.owner {
            Owner::Person(p) => m.person == Some(p.as_str()),
            Owner::Key(k) => m.key == Some(k.id().as_str()),
        };
        if owner || m.key == Some(v.curator.id().as_str()) {
            return Some(Level::Admin);
        }
        let mut subjects = Vec::new();
        subjects.extend(m.key.map(|k| Subject::Key(k.into())));
        subjects.extend(m.machine.map(|k| Subject::Machine(k.into())));
        subjects.extend(m.person.map(|p| Subject::Person(p.into())));
        subjects.extend(m.networks.iter().map(|n| Subject::Network((*n).into())));
        subjects.extend(m.actor.map(|(slug, machine)| Subject::Actor { slug: slug.into(), machine: machine.into() }));
        subjects.iter().filter_map(|s| self.level(s)).max()
    }

    /// Whether the space counts as `entity`'s: it's about it, the owner
    /// signed something in it, and the owner owns the entity by the chains
    /// `owners` reads (a person owns themselves). Otherwise it's only
    /// "about it, owned by" its owner.
    pub fn is_about(&self, entity: &EntityRef, owners: Option<&dyn Owners>) -> bool {
        let v = self.view();
        let Owner::Person(p) = &v.owner else { return false };
        if &v.about != entity || !entity.is_bound() || !v.owner_signed {
            return false;
        }
        match entity {
            EntityRef::Person(q) => q == p,
            _ => owners.and_then(|o| o.owner_of(entity)).is_some_and(|q| q == *p),
        }
    }

    /// Everything this holder has of the space: the effective chain, what
    /// it holds beside it, and the side entries, for the space's file and
    /// for a member that asks.
    pub fn to_json(&self) -> Value {
        json!({
            "entries": self.path.iter().chain(&self.held).map(Entry::to_json).collect::<Vec<_>>(),
            "sides": self.sides().chain(&self.waiting).map(Side::to_json).collect::<Vec<_>>(),
        })
    }

    /// A space from what `to_json` wrote: entry 0 first, the rest offered
    /// in any order, side entries in the order they were stored.
    pub fn from_json(policy: SpacePolicy, v: &Value) -> R<Space> {
        let entries = v.get("entries").and_then(Value::as_array).filter(|e| !e.is_empty()).ok_or_else(|| bad("entries"))?;
        let mut space = Space::found(policy, Entry::parse(&entries[0])?)?;
        for e in &entries[1..] {
            let e = Entry::parse(e)?;
            if space.offer(e.clone()).is_err() {
                // Kept as it was: refused only against today's head.
                space.held.push(e);
            }
        }
        for s in v.get("sides").and_then(Value::as_array).ok_or_else(|| bad("sides"))? {
            space.offer_side(Side::parse(s)?)?;
        }
        Ok(space)
    }
}

pub mod marker;

#[cfg(test)]
mod tests;
