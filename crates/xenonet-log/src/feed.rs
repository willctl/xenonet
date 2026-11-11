//! A machine's feed in a network (`xenonet-feed/1`): what the machine
//! tells the other members, as entries it signs with its machine signing
//! key, numbered so each receiver applies every entry once, in the
//! author's order, whatever way the entries reached it.
//!
//! An entry names its author (the machine signing key's `xns1_` id), the
//! network, the author's incarnation and a sequence number, its kind, when
//! it was made and a body (a JSON object the engine defines per kind).
//! The author signs the canonical JSON of all of that, after the tag and a
//! newline (`Purpose::FEED`). The entry's id is the SHA-256 of those same
//! bytes, without the signature: a P-256 key signs differently each time,
//! so a copy the author signed again is still the same entry.
//!
//! **Two classes of kind.** Claims, intents, handoffs, assigns and reports
//! are kept: they form one gapless sequence per (author, network), and a
//! receiver applies them strictly in order, holding any that arrive early
//! until the gap before them fills. Messages and receipts are loose: they
//! sit outside that sequence, each has its own number in its own count, and
//! a receiver takes each once by its id. A late message doesn't wait for a
//! claim, and a claim never waits for a message.
//!
//! **Incarnations.** Kept entries are numbered from 1 within an
//! incarnation. The incarnation changes only when the author's store was
//! restored or XenoNet was set up again with the same machine key: it is
//! `max(now, last + 1)` in milliseconds (`next_incarnation`), so it
//! increases. The first kept entry of an incarnation names `prev`, the
//! last kept entry its author still holds, as (incarnation, seq), or
//! (0, 0) for none. A receiver links the new incarnation when that `prev`
//! is at or below what it holds: at it is the plain case, below it is a
//! restore (the author lost entries the receiver has, which stay applied),
//! never a fork. A `prev` above what it holds is a gap.
//!
//! **What a receiver answers** (`Feed::offer`), for an entry whose
//! signature it checked (`Verified`):
//!
//! - `New`: the entry applied, with the held entries it released, in order;
//! - `Duplicate`: the receiver has an entry with the same id there;
//! - `Gap`: held until the entries before it arrive, with the `Want` to send
//!   the author (resend from that place on);
//! - `Fork`: another entry with the same place and a different id, which
//!   freezes the author's feed in this network;
//! - `Frozen`: the author's feed is frozen, and nothing from it is taken
//!   until the person unfreezes it (`Feed::unfreeze`);
//! - `Stale`: from an incarnation the receiver has moved past, at a place
//!   it never held: history the author gave up when it was restored.
//!
//! A receiver acknowledges only the contiguous high of the incarnation it
//! is in (`Ack`), and the author resends everything after it when it
//! reconnects. What a receiver holds and has applied lives in a
//! `FeedStore`, which the store crate implements on its tables, inside the
//! transaction that applies the entries, so each is applied exactly once;
//! `MemoryStore` keeps it in memory, for tests and tools.

use crate::network::is_network_id;
use crate::person::is_key_id;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::convert::Infallible;
use std::fmt;
use xenonet_id::codec::{b64, sha256, unb64};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};

/// The most bytes an entry's JSON takes: a frame's limit, so any entry
/// fits in one.
pub const MAX_ENTRY: usize = 64 * 1024;

/// Why an entry doesn't read, or wasn't signed by its author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedError {
    pub code: &'static str,
    pub message: String,
}

impl FeedError {
    fn bad(what: &str) -> FeedError {
        FeedError { code: "bad-entry", message: format!("a feed entry's {what} isn't right") }
    }

    fn refused(message: impl Into<String>) -> FeedError {
        FeedError { code: "refused", message: message.into() }
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for FeedError {}

pub type R<T> = Result<T, FeedError>;

/// What an entry is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Claim,
    Intent,
    Handoff,
    Assign,
    Report,
    Message,
    Receipt,
}

/// Whether a kind is in the gapless sequence or outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Applied strictly in order, with no gaps.
    Kept,
    /// Taken once by id, in any order.
    Loose,
}

impl Kind {
    pub const ALL: [Kind; 7] = [Kind::Claim, Kind::Intent, Kind::Handoff, Kind::Assign, Kind::Report, Kind::Message, Kind::Receipt];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Claim => "claim",
            Kind::Intent => "intent",
            Kind::Handoff => "handoff",
            Kind::Assign => "assign",
            Kind::Report => "report",
            Kind::Message => "message",
            Kind::Receipt => "receipt",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn class(self) -> Class {
        match self {
            Kind::Message | Kind::Receipt => Class::Loose,
            _ => Class::Kept,
        }
    }
}

/// A place in an author's feed: an incarnation and a sequence number in
/// it. Places order by incarnation, then by number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub inc: u64,
    pub seq: u64,
}

impl Pos {
    /// Before the author's first entry.
    pub const ORIGIN: Pos = Pos { inc: 0, seq: 0 };

    pub fn new(inc: u64, seq: u64) -> Pos {
        Pos { inc, seq }
    }
}

/// The next incarnation after `last`, at `now` (ms): `max(now, last + 1)`,
/// so it is past every one before even when the clock went back. `last`
/// is the highest incarnation the author knows of, its store's or one a
/// receiver's `Ack` named.
pub fn next_incarnation(last: u64, now: u64) -> u64 {
    now.max(last.saturating_add(1))
}

/// One entry of a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// The author's machine signing key id, `xns1_…`.
    pub author: String,
    pub network: String,
    /// The author's incarnation, from 1.
    pub inc: u64,
    /// From 1 in each incarnation, kept and loose kinds each counted on
    /// their own.
    pub seq: u64,
    /// On the first kept entry of an incarnation only: the last kept entry
    /// before it, or `Pos::ORIGIN`.
    pub prev: Option<Pos>,
    pub kind: Kind,
    /// When it was made, in ms.
    pub at: u64,
    /// What it says: a JSON object, whose shape the kind's owner defines.
    pub body: Value,
    /// Empty until signed.
    pub sig: Vec<u8>,
}

impl FeedEntry {
    pub fn pos(&self) -> Pos {
        Pos { inc: self.inc, seq: self.seq }
    }

    pub fn class(&self) -> Class {
        self.kind.class()
    }

    fn unsigned(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("author".into(), Value::String(self.author.clone()));
        m.insert("network".into(), Value::String(self.network.clone()));
        m.insert("inc".into(), Value::from(self.inc));
        m.insert("seq".into(), Value::from(self.seq));
        if let Some(p) = self.prev {
            let mut prev = Map::new();
            prev.insert("inc".into(), Value::from(p.inc));
            prev.insert("seq".into(), Value::from(p.seq));
            m.insert("prev".into(), Value::Object(prev));
        }
        m.insert("kind".into(), Value::String(self.kind.as_str().into()));
        m.insert("at".into(), Value::from(self.at));
        m.insert("body".into(), self.body.clone());
        m
    }

    /// What the author signs: the tag, a newline, the canonical JSON
    /// without the signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        Purpose::FEED.bytes(Value::Object(self.unsigned()).to_string().as_bytes())
    }

    /// The SHA-256 of what's signed: what tells a duplicate from a fork.
    pub fn hash(&self) -> [u8; 32] {
        sha256(&self.signing_bytes())
    }

    /// The hash in base64url: the entry's id, by which loose entries are
    /// taken once.
    pub fn id(&self) -> String {
        b64(&self.hash())
    }

    /// The entry as it's kept and sent: its canonical JSON, signature last.
    pub fn to_json(&self) -> Value {
        let mut m = self.unsigned();
        m.insert("sig".into(), Value::String(b64(&self.sig)));
        Value::Object(m)
    }

    pub fn encode(&self) -> Vec<u8> {
        self.to_json().to_string().into_bytes()
    }

    /// Signs the entry with its author's key.
    pub fn sign(&mut self, key: &dyn Signer) -> xenonet_id::R<()> {
        if key.public().id() != self.author {
            return Err(xenonet_id::IdError::refused("a feed entry is signed by its author's key"));
        }
        self.sig = key.sign(Purpose::FEED, Value::Object(self.unsigned()).to_string().as_bytes())?;
        Ok(())
    }

    /// What `parse` holds an entry to, for one made here: an entry goes
    /// out only if every receiver can read it back.
    pub fn well_formed(&self) -> R<()> {
        if !is_key_id(&self.author) {
            return Err(FeedError::bad("author"));
        }
        if !is_network_id(&self.network) {
            return Err(FeedError::bad("network"));
        }
        if self.inc == 0 {
            return Err(FeedError::bad("incarnation"));
        }
        if self.seq == 0 {
            return Err(FeedError::bad("seq"));
        }
        let first = self.class() == Class::Kept && self.seq == 1;
        match self.prev {
            None if first => return Err(FeedError::bad("prev")),
            Some(_) if !first => return Err(FeedError::bad("prev")),
            // Before this incarnation, and a real entry unless it's the
            // origin: (n, 0) names nothing.
            Some(p) if p.inc >= self.inc || (p.seq == 0) != (p.inc == 0) => return Err(FeedError::bad("prev")),
            _ => {}
        }
        if !self.body.is_object() {
            return Err(FeedError::bad("body"));
        }
        if !(64..=72).contains(&self.sig.len()) {
            return Err(FeedError::bad("signature"));
        }
        if self.to_json().to_string().len() > MAX_ENTRY {
            return Err(FeedError::bad("size"));
        }
        Ok(())
    }

    pub fn parse(v: &Value) -> R<FeedEntry> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| FeedError::bad(k));
        let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).ok_or_else(|| FeedError::bad(k));
        let prev = match v.get("prev") {
            None => None,
            Some(p) => Some(Pos { inc: n(p, "inc").map_err(|_| FeedError::bad("prev"))?, seq: n(p, "seq").map_err(|_| FeedError::bad("prev"))? }),
        };
        let e = FeedEntry {
            author: s("author")?.into(),
            network: s("network")?.into(),
            inc: n(v, "inc")?,
            seq: n(v, "seq")?,
            prev,
            kind: Kind::parse(s("kind")?).ok_or_else(|| FeedError::bad("kind"))?,
            at: n(v, "at")?,
            body: v.get("body").cloned().ok_or_else(|| FeedError::bad("body"))?,
            sig: unb64(s("sig")?).ok_or_else(|| FeedError::bad("signature"))?,
        };
        e.well_formed()?;
        Ok(e)
    }

    /// An entry from the bytes it came as.
    pub fn decode(bytes: &[u8]) -> R<FeedEntry> {
        if bytes.len() > MAX_ENTRY {
            return Err(FeedError::bad("size"));
        }
        let v: Value = serde_json::from_slice(bytes).map_err(|_| FeedError::bad("JSON"))?;
        FeedEntry::parse(&v)
    }

    /// The entry, once its signature checks against `key`, the author's
    /// key as the receiver's network chain has it.
    pub fn verify(self, key: &VerifyingKey) -> R<Verified> {
        self.well_formed()?;
        if key.id() != self.author {
            return Err(FeedError::refused("that key isn't the feed entry's author's"));
        }
        if !key.verify_raw(&self.signing_bytes(), &self.sig) {
            return Err(FeedError::refused(format!("{}'s signature on a feed entry doesn't check", self.author)));
        }
        Ok(Verified(self))
    }
}

/// An entry whose signature was checked against its author's key: the
/// only kind a `Feed` takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified(FeedEntry);

impl Verified {
    pub fn entry(&self) -> &FeedEntry {
        &self.0
    }

    pub fn into_entry(self) -> FeedEntry {
        self.0
    }
}

impl std::ops::Deref for Verified {
    type Target = FeedEntry;
    fn deref(&self) -> &FeedEntry {
        &self.0
    }
}

/// The author's side: numbers and signs its entries in one network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    author: String,
    network: String,
    inc: u64,
    kept: u64,
    loose: u64,
    /// What the incarnation's first kept entry names, until it's written.
    prev: Option<Pos>,
}

impl Writer {
    /// A feed's first incarnation, at `now` (ms).
    pub fn start(author: impl Into<String>, network: impl Into<String>, now: u64) -> Writer {
        Writer { author: author.into(), network: network.into(), inc: next_incarnation(0, now), kept: 0, loose: 0, prev: Some(Pos::ORIGIN) }
    }

    /// Where the author left off, as its store has it: the same
    /// incarnation after a restart. `prev` is the last kept entry of an
    /// earlier incarnation when this one has none of its own yet.
    pub fn resume(author: impl Into<String>, network: impl Into<String>, inc: u64, kept: u64, loose: u64, prev: Pos) -> Writer {
        Writer { author: author.into(), network: network.into(), inc, kept, loose, prev: (kept == 0).then_some(prev) }
    }

    /// After a restore or a new setup: a new incarnation past `last` (the
    /// highest the author knows of), whose first kept entry names `held`,
    /// the last kept entry the author still has (`Pos::ORIGIN` for none).
    pub fn restore(author: impl Into<String>, network: impl Into<String>, held: Pos, last: u64, now: u64) -> Writer {
        let inc = next_incarnation(last.max(held.inc), now);
        Writer { author: author.into(), network: network.into(), inc, kept: 0, loose: 0, prev: Some(held) }
    }

    pub fn inc(&self) -> u64 {
        self.inc
    }

    /// The last kept entry written, or the one before this incarnation.
    pub fn head(&self) -> Pos {
        if self.kept == 0 {
            self.prev.unwrap_or(Pos::ORIGIN)
        } else {
            Pos { inc: self.inc, seq: self.kept }
        }
    }

    /// The next entry, unsigned, numbered and counted.
    pub fn next(&mut self, kind: Kind, at: u64, body: Value) -> FeedEntry {
        let (seq, prev) = match kind.class() {
            Class::Kept => {
                self.kept += 1;
                (self.kept, if self.kept == 1 { self.prev.take() } else { None })
            }
            Class::Loose => {
                self.loose += 1;
                (self.loose, None)
            }
        };
        FeedEntry { author: self.author.clone(), network: self.network.clone(), inc: self.inc, seq, prev, kind, at, body, sig: Vec::new() }
    }

    /// The next entry, signed by `key`. Counted only once signed, so a key
    /// that fails leaves no hole in the sequence.
    pub fn write(&mut self, key: &dyn Signer, kind: Kind, at: u64, body: Value) -> xenonet_id::R<FeedEntry> {
        let before = self.clone();
        let mut e = self.next(kind, at, body);
        if let Err(err) = e.sign(key) {
            *self = before;
            return Err(err);
        }
        Ok(e)
    }
}

/// What a receiver asks an author for on a gap: every kept entry from
/// (`inc`, `from`) on. (0, 1) is from the author's first.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Want {
    pub author: String,
    pub network: String,
    pub inc: u64,
    pub from: u64,
}

impl Want {
    /// Whether the entry at `p` is one this asks for.
    pub fn wants(&self, p: Pos) -> bool {
        p >= Pos { inc: self.inc, seq: self.from }
    }
}

/// What a receiver acknowledges: it has every kept entry up to `high` of
/// incarnation `inc`, and so the author resends only what's after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ack {
    pub inc: u64,
    pub high: u64,
}

impl Ack {
    /// Whether the author should still send the kept entry at `p`.
    pub fn unacked(self, p: Pos) -> bool {
        p > Pos { inc: self.inc, seq: self.high }
    }
}

/// Two entries at one place: what froze a feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fork {
    pub at: Pos,
    /// The id of the entry the receiver had there.
    pub held: [u8; 32],
    /// The id of the other one.
    pub offered: [u8; 32],
}

/// Where a receiver stands with one author in one network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Head {
    /// The last kept entry applied, in the latest incarnation linked;
    /// `Pos::ORIGIN` before the first.
    pub latest: Pos,
    /// Set by a fork; cleared only by the person.
    pub frozen: Option<Fork>,
}

impl Default for Head {
    fn default() -> Head {
        Head { latest: Pos::ORIGIN, frozen: None }
    }
}

/// What a feed did with an offered entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// Applied: this entry, then any held entries it released, in order.
    New(Vec<FeedEntry>),
    Duplicate,
    /// Held; ask the author for what's missing.
    Gap(Want),
    /// The author signed two entries at one place: its feed is frozen.
    Fork(Fork),
    /// The author's feed was frozen by an earlier fork; dropped.
    Frozen(Fork),
    /// From history the author gave up in a restore; dropped.
    Stale,
}

/// What a receiver keeps of each author's feed in each network. The store
/// implements it on its tables, called inside the transaction that applies
/// what `Feed::offer` returns; entries come back as they were stored,
/// their signatures already checked.
pub trait FeedStore {
    type Error;

    /// Where the receiver stands; the default `Head` for an author it has
    /// nothing from.
    fn head(&self, author: &str, network: &str) -> Result<Head, Self::Error>;
    fn set_head(&mut self, author: &str, network: &str, head: Head) -> Result<(), Self::Error>;
    /// The id of the kept entry applied at `at`, if any.
    fn kept(&self, author: &str, network: &str, at: Pos) -> Result<Option<[u8; 32]>, Self::Error>;
    /// Records a kept entry as applied.
    fn keep(&mut self, e: &FeedEntry) -> Result<(), Self::Error>;
    /// Whether the loose entry with this id was taken.
    fn has_loose(&self, author: &str, network: &str, id: &[u8; 32]) -> Result<bool, Self::Error>;
    /// Records a loose entry as taken.
    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Self::Error>;
    /// The kept entry held (not yet applied) at `at`.
    fn held(&self, author: &str, network: &str, at: Pos) -> Result<Option<FeedEntry>, Self::Error>;
    /// Holds a kept entry until the entries before it arrive.
    fn hold(&mut self, e: FeedEntry) -> Result<(), Self::Error>;
    fn unhold(&mut self, author: &str, network: &str, at: Pos) -> Result<(), Self::Error>;
    /// The held first entries (seq 1) of incarnations after `inc`, lowest
    /// incarnation first.
    fn held_firsts(&self, author: &str, network: &str, after: u64) -> Result<Vec<FeedEntry>, Self::Error>;
    /// Drops the held entries of incarnations before `inc`, which can no
    /// longer apply.
    fn drop_held_before(&mut self, author: &str, network: &str, inc: u64) -> Result<(), Self::Error>;
}

impl<S: FeedStore + ?Sized> FeedStore for &mut S {
    type Error = S::Error;
    fn head(&self, author: &str, network: &str) -> Result<Head, Self::Error> {
        (**self).head(author, network)
    }
    fn set_head(&mut self, author: &str, network: &str, head: Head) -> Result<(), Self::Error> {
        (**self).set_head(author, network, head)
    }
    fn kept(&self, author: &str, network: &str, at: Pos) -> Result<Option<[u8; 32]>, Self::Error> {
        (**self).kept(author, network, at)
    }
    fn keep(&mut self, e: &FeedEntry) -> Result<(), Self::Error> {
        (**self).keep(e)
    }
    fn has_loose(&self, author: &str, network: &str, id: &[u8; 32]) -> Result<bool, Self::Error> {
        (**self).has_loose(author, network, id)
    }
    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Self::Error> {
        (**self).take_loose(e)
    }
    fn held(&self, author: &str, network: &str, at: Pos) -> Result<Option<FeedEntry>, Self::Error> {
        (**self).held(author, network, at)
    }
    fn hold(&mut self, e: FeedEntry) -> Result<(), Self::Error> {
        (**self).hold(e)
    }
    fn unhold(&mut self, author: &str, network: &str, at: Pos) -> Result<(), Self::Error> {
        (**self).unhold(author, network, at)
    }
    fn held_firsts(&self, author: &str, network: &str, after: u64) -> Result<Vec<FeedEntry>, Self::Error> {
        (**self).held_firsts(author, network, after)
    }
    fn drop_held_before(&mut self, author: &str, network: &str, inc: u64) -> Result<(), Self::Error> {
        (**self).drop_held_before(author, network, inc)
    }
}

/// A receiver's feeds: every author's in every network, over a store.
#[derive(Debug, Clone, Default)]
pub struct Feed<S> {
    store: S,
}

impl<S: FeedStore> Feed<S> {
    pub fn new(store: S) -> Feed<S> {
        Feed { store }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    /// Takes, holds or drops an entry, and says which.
    pub fn offer(&mut self, e: Verified) -> Result<Offer, S::Error> {
        let e = e.into_entry();
        let (author, network) = (e.author.clone(), e.network.clone());
        let mut head = self.store.head(&author, &network)?;
        if let Some(f) = head.frozen {
            return Ok(Offer::Frozen(f));
        }
        if e.class() == Class::Loose {
            let id = e.hash();
            if self.store.has_loose(&author, &network, &id)? {
                return Ok(Offer::Duplicate);
            }
            self.store.take_loose(&e)?;
            return Ok(Offer::New(vec![e]));
        }
        let at = e.pos();
        let id = e.hash();
        let had = match self.store.kept(&author, &network, at)? {
            Some(h) => Some(h),
            None => self.store.held(&author, &network, at)?.map(|h| h.hash()),
        };
        if let Some(held) = had {
            if held == id {
                return Ok(Offer::Duplicate);
            }
            let fork = Fork { at, held, offered: id };
            head.frozen = Some(fork);
            self.store.set_head(&author, &network, head)?;
            return Ok(Offer::Fork(fork));
        }
        let latest = head.latest;
        let applies = match at.inc.cmp(&latest.inc) {
            std::cmp::Ordering::Equal => at.seq == latest.seq + 1,
            // A later incarnation links at what the receiver holds, or
            // below it after a restore; above it, something's missing.
            std::cmp::Ordering::Greater => e.prev.is_some_and(|p| p <= latest),
            std::cmp::Ordering::Less => return Ok(Offer::Stale),
        };
        if !applies {
            // At or below the head with no entry there means the store
            // lost what it applied; nothing here can mend that.
            if at.inc == latest.inc && at.seq <= latest.seq {
                return Ok(Offer::Stale);
            }
            self.store.hold(e)?;
            return Ok(Offer::Gap(Want { author, network, inc: latest.inc, from: latest.seq + 1 }));
        }
        let mut out = Vec::new();
        self.apply(&mut head, e, &mut out)?;
        self.drain(&author, &network, &mut head, &mut out)?;
        self.store.set_head(&author, &network, head)?;
        Ok(Offer::New(out))
    }

    fn apply(&mut self, head: &mut Head, e: FeedEntry, out: &mut Vec<FeedEntry>) -> Result<(), S::Error> {
        if e.inc != head.latest.inc {
            // Linking a later incarnation closes the ones before it: what's
            // held of them is past where the author said they ended.
            self.store.drop_held_before(&e.author, &e.network, e.inc)?;
        }
        self.store.keep(&e)?;
        head.latest = e.pos();
        out.push(e);
        Ok(())
    }

    /// Applies what's held and now in order.
    fn drain(&mut self, author: &str, network: &str, head: &mut Head, out: &mut Vec<FeedEntry>) -> Result<(), S::Error> {
        loop {
            let next = Pos { inc: head.latest.inc, seq: head.latest.seq + 1 };
            if next.inc > 0 {
                if let Some(e) = self.store.held(author, network, next)? {
                    self.store.unhold(author, network, next)?;
                    self.apply(head, e, out)?;
                    continue;
                }
            }
            let firsts = self.store.held_firsts(author, network, head.latest.inc)?;
            match firsts.into_iter().find(|e| e.prev.is_some_and(|p| p <= head.latest)) {
                Some(e) => {
                    self.store.unhold(author, network, e.pos())?;
                    self.apply(head, e, out)?;
                }
                None => return Ok(()),
            }
        }
    }

    /// What the receiver acknowledges to `author`; None before its first
    /// kept entry.
    pub fn ack(&self, author: &str, network: &str) -> Result<Option<Ack>, S::Error> {
        let l = self.store.head(author, network)?.latest;
        Ok((l != Pos::ORIGIN).then_some(Ack { inc: l.inc, high: l.seq }))
    }

    /// The fork that froze `author`'s feed, if it's frozen.
    pub fn frozen(&self, author: &str, network: &str) -> Result<Option<Fork>, S::Error> {
        Ok(self.store.head(author, network)?.frozen)
    }

    /// Takes `author`'s feed again after a fork, keeping the entry the
    /// receiver had: only the person decides this, at the window or
    /// through the doctor, never an agent or another machine. Returns the
    /// held entries it released.
    pub fn unfreeze(&mut self, author: &str, network: &str) -> Result<Vec<FeedEntry>, S::Error> {
        let mut head = self.store.head(author, network)?;
        head.frozen = None;
        let mut out = Vec::new();
        self.drain(author, network, &mut head, &mut out)?;
        self.store.set_head(author, network, head)?;
        Ok(out)
    }
}

type Key = (String, String);

/// A `FeedStore` in memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    heads: HashMap<Key, Head>,
    kept: HashMap<Key, BTreeMap<Pos, [u8; 32]>>,
    loose: HashMap<Key, HashSet<[u8; 32]>>,
    held: HashMap<Key, BTreeMap<Pos, FeedEntry>>,
}

fn key(author: &str, network: &str) -> Key {
    (author.to_string(), network.to_string())
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// How many kept entries are held for `author`.
    pub fn held_count(&self, author: &str, network: &str) -> usize {
        self.held.get(&key(author, network)).map_or(0, BTreeMap::len)
    }
}

impl FeedStore for MemoryStore {
    type Error = Infallible;

    fn head(&self, author: &str, network: &str) -> Result<Head, Infallible> {
        Ok(self.heads.get(&key(author, network)).copied().unwrap_or_default())
    }

    fn set_head(&mut self, author: &str, network: &str, head: Head) -> Result<(), Infallible> {
        self.heads.insert(key(author, network), head);
        Ok(())
    }

    fn kept(&self, author: &str, network: &str, at: Pos) -> Result<Option<[u8; 32]>, Infallible> {
        Ok(self.kept.get(&key(author, network)).and_then(|m| m.get(&at)).copied())
    }

    fn keep(&mut self, e: &FeedEntry) -> Result<(), Infallible> {
        self.kept.entry(key(&e.author, &e.network)).or_default().insert(e.pos(), e.hash());
        Ok(())
    }

    fn has_loose(&self, author: &str, network: &str, id: &[u8; 32]) -> Result<bool, Infallible> {
        Ok(self.loose.get(&key(author, network)).is_some_and(|s| s.contains(id)))
    }

    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Infallible> {
        self.loose.entry(key(&e.author, &e.network)).or_default().insert(e.hash());
        Ok(())
    }

    fn held(&self, author: &str, network: &str, at: Pos) -> Result<Option<FeedEntry>, Infallible> {
        Ok(self.held.get(&key(author, network)).and_then(|m| m.get(&at)).cloned())
    }

    fn hold(&mut self, e: FeedEntry) -> Result<(), Infallible> {
        self.held.entry(key(&e.author, &e.network)).or_default().insert(e.pos(), e);
        Ok(())
    }

    fn unhold(&mut self, author: &str, network: &str, at: Pos) -> Result<(), Infallible> {
        if let Some(m) = self.held.get_mut(&key(author, network)) {
            m.remove(&at);
        }
        Ok(())
    }

    fn held_firsts(&self, author: &str, network: &str, after: u64) -> Result<Vec<FeedEntry>, Infallible> {
        let Some(m) = self.held.get(&key(author, network)) else { return Ok(Vec::new()) };
        Ok(m.range(Pos { inc: after.saturating_add(1), seq: 0 }..).filter(|(p, _)| p.seq == 1).map(|(_, e)| e.clone()).collect())
    }

    fn drop_held_before(&mut self, author: &str, network: &str, inc: u64) -> Result<(), Infallible> {
        if let Some(m) = self.held.get_mut(&key(author, network)) {
            m.retain(|p, _| p.inc >= inc);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
