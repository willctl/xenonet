//! A machine's feed in a group (`xenonet-feed/1`): what the machine tells
//! the group's other members, as entries it signs with its machine signing
//! key, numbered so each receiver applies every entry once, in the
//! author's order, whatever way the entries reached it.
//!
//! **Groups.** A feed is scoped to a group: a network (`net_…`), whose
//! members its roster names, or a space (`spc_…`), whose members its space
//! chain names. The id's prefix says which (`Group::of`, a space's id as
//! xenonet-model's `SpaceId` reads it). An author has one feed in each
//! group it's in, and a receiver keeps each (author, group) apart.
//!
//! An entry names its author (the machine signing key's `xns1_` id), the
//! group, the author's incarnation and a sequence number, its kind, when
//! it was made and a body (a JSON object the engine defines per kind).
//! The author signs the entry's header: the canonical JSON of all of that
//! with the body as its SHA-256 (`bodyHash`), after the tag and a newline
//! (`Purpose::FEED`). The entry's id is the SHA-256 of those same bytes,
//! without the signature: a P-256 key signs differently each time, so a
//! copy the author signed again is still the same entry, and a header
//! (`Header`) shows an entry's id and the one before it without its body.
//!
//! **Two classes of kind.** Claims, intents, handoffs, assigns, reports and
//! compact entries are kept: they form one gapless sequence per (author,
//! group), and a receiver applies them strictly in order, holding any that
//! arrive early until the gap before them fills. Each names `prev`, the
//! place and id of the kept entry before it, so the kept entries are a
//! hash chain: a receiver checks each against the entry it applied there,
//! and two sides of a fork can't both pass, whichever holders served them.
//! Messages and receipts are loose: they sit outside that sequence, each
//! has its own number in its own count, and a receiver takes each once by
//! its id. A late message doesn't wait for a claim, and a claim never waits
//! for a message.
//!
//! **Incarnations.** Kept entries are numbered from 1 within an
//! incarnation. The incarnation changes only when the author's store was
//! restored or XenoNet was set up again with the same machine key: it is
//! `max(now, last + 1)` in milliseconds (`next_incarnation`), so it
//! increases. The first kept entry of an incarnation names as `prev` the
//! last kept entry its author still holds, or the origin (0, 0) for none:
//! the incarnation's link. A receiver links the new incarnation when the
//! link is at or below what it holds: at it is the plain case, below it is
//! a restore (the author lost entries the receiver has, which stay
//! applied), never a fork. A link above what it holds is a gap.
//!
//! **Retention and compaction.** Each kind has a retention
//! (`Kind::retention`): claims and handoffs are kept while they're open,
//! reports, messages and receipts for `RETENTION_MS` by their signed time,
//! intents and assigns until their `until` passes, a compact entry until
//! the next, and Xenolith's kinds in a space (`ctx.*`: notes, proposals,
//! pages, removals, log days, forgets and snapshots) for good, since a
//! space's history is its curator's record (a forget drops texts, which
//! those entries name by hash, never the entries). An author compacts with a signed compact entry
//! (`Compact`): it names its incarnation, `upto` (every kept entry before
//! it), the id of the entry at `upto` and the incarnation's link, and
//! lists in `live` the entries of that range still open (`live_set`).
//! The author then drops the rest of the range, keeping their headers.
//! The compact entry is authoritative about what is open: a receiver ends
//! everything it applied from that author before it that `live` leaves
//! out (`Applied::ended`), but absence ends only what the author's machine
//! may end alone (`Kind::ends_alone`): an intent or assign stays open,
//! and in `live` with its cancel, until its `until` passes. What a
//! compact entry may drop is that closed table of kinds. Nothing that
//! revokes goes in a feed: revocations live in chains, kept forever.
//!
//! A receiver behind a compact entry can't fill the gap entry by entry,
//! since the author dropped them. A Want below the author's floor gets a
//! `Compacted` answer instead: the compact entry, the live entries the
//! receiver wants, and the header chain of the range it wants. The
//! receiver checks that chain from the signed `atId` down to what it
//! applied and holds there, so a fork inside the compacted range is still
//! seen, drops what it held there that isn't live (never applied later),
//! applies the live entries and the compact entry, and keeps the chain's
//! ids for the places it skipped, so a later copy of one is a duplicate
//! and another entry there is a fork (`Feed::offer_compacted`).
//!
//! **What a receiver answers** (`Feed::offer`), for an entry whose
//! signature it checked (`Verified`):
//!
//! - `New`: the entry applied, with the held entries it released, in
//!   order, and what a compact entry among them ended;
//! - `Duplicate`: the receiver has an entry with the same id there;
//! - `Gap`: held until the entries before it arrive, with the `Want` to send
//!   the author (resend from that place on);
//! - `Fork`: another entry with the same place and a different id, or one
//!   naming another entry before it than the one applied there, which
//!   freezes the author's feed in this group;
//! - `Frozen`: the author's feed is frozen, and nothing from it is taken
//!   until the person unfreezes it (`Feed::unfreeze`);
//! - `Stale`: from an incarnation the receiver has moved past, at a place
//!   it never held: history the author gave up when it was restored, or
//!   that a compact entry covered;
//! - `Expired`: a loose entry older than `RETENTION_MS` by its signed time.
//!   A receiver remembers each loose entry it took for `LOOSE_DEDUPE_MS`,
//!   longer than that and than any token a message carries lives, so a
//!   replay is either remembered or refused.
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
use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::fmt;
use xenonet_id::codec::{b64, sha256, unb64, unb64_n};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};
use xenonet_model::ids::SpaceId;

/// The most bytes an entry's JSON takes: a frame's limit, so any entry
/// fits in one.
pub const MAX_ENTRY: usize = 64 * 1024;

const DAY: u64 = 24 * 60 * 60 * 1000;

/// How long an entry that ages out is kept, by its signed time: loose
/// entries, and reports.
pub const RETENTION_MS: u64 = 7 * DAY;

/// The longest a per-act token lives: a message replayed within it could
/// still act.
pub const TOKEN_TTL_MS: u64 = 8 * DAY;

/// How long a receiver remembers a loose entry it took: past retention,
/// after which the entry is refused anyway, and past any token it could
/// carry, with a day's margin.
pub const LOOSE_DEDUPE_MS: u64 = (if RETENTION_MS > TOKEN_TTL_MS { RETENTION_MS } else { TOKEN_TTL_MS }) + DAY;

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

/// What an entry is about. There is no kind that revokes: a revocation
/// lives in a chain, which is never compacted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Claim,
    Intent,
    Handoff,
    Assign,
    Report,
    Message,
    Receipt,
    /// What is still open of the range before it (`Compact`).
    Compact,
    /// Xenolith's, in a space: a note for the space's curator to file.
    CtxNote,
    /// Xenolith's: a member's replace or removal of a page, against the
    /// version it had (its base), for the curator to apply or hold.
    CtxPropose,
    /// Xenolith's, by the curator: a page's version, its text named by
    /// hash and carried apart, so it can be forgotten.
    CtxPage,
    /// Xenolith's, by the curator: a page removed.
    CtxRemove,
    /// Xenolith's, by the curator: a day of the space's log.
    CtxLog,
    /// Xenolith's, by the curator with the owner's approval: texts every
    /// holder drops, by hash, keeping the entries that named them.
    CtxForget,
    /// Xenolith's, by the curator: the live pages at an epoch, so a new
    /// member gets the live set rather than the history.
    CtxSnapshot,
}

/// Whether a kind is in the gapless sequence or outside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Applied strictly in order, with no gaps.
    Kept,
    /// Taken once by id, in any order.
    Loose,
}

/// How long an author keeps an entry of a kind before a compact entry
/// may drop it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Retention {
    /// While the author holds it open (the engine says which are).
    WhileOpen,
    /// For this many ms after its signed time.
    Aged(u64),
    /// Until its body's `until` (ms) passes; for good without one.
    Until,
    /// Until the next compact entry.
    Superseded,
    /// For good: a compact entry never drops it (a space's context).
    Forever,
}

impl Kind {
    pub const ALL: [Kind; 15] = [
        Kind::Claim,
        Kind::Intent,
        Kind::Handoff,
        Kind::Assign,
        Kind::Report,
        Kind::Message,
        Kind::Receipt,
        Kind::Compact,
        Kind::CtxNote,
        Kind::CtxPropose,
        Kind::CtxPage,
        Kind::CtxRemove,
        Kind::CtxLog,
        Kind::CtxForget,
        Kind::CtxSnapshot,
    ];
    /// Xenolith's kinds, which only a space's feeds carry.
    pub const CTX: [Kind; 7] = [Kind::CtxNote, Kind::CtxPropose, Kind::CtxPage, Kind::CtxRemove, Kind::CtxLog, Kind::CtxForget, Kind::CtxSnapshot];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Claim => "claim",
            Kind::Intent => "intent",
            Kind::Handoff => "handoff",
            Kind::Assign => "assign",
            Kind::Report => "report",
            Kind::Message => "message",
            Kind::Receipt => "receipt",
            Kind::Compact => "compact",
            Kind::CtxNote => "ctx.note",
            Kind::CtxPropose => "ctx.propose",
            Kind::CtxPage => "ctx.page",
            Kind::CtxRemove => "ctx.remove",
            Kind::CtxLog => "ctx.log",
            Kind::CtxForget => "ctx.forget",
            Kind::CtxSnapshot => "ctx.snapshot",
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

    /// The closed table of what a compact entry may drop, and when.
    pub fn retention(self) -> Retention {
        match self {
            Kind::Claim | Kind::Handoff => Retention::WhileOpen,
            Kind::Report | Kind::Message | Kind::Receipt => Retention::Aged(RETENTION_MS),
            Kind::Intent | Kind::Assign => Retention::Until,
            Kind::Compact => Retention::Superseded,
            // Direction's retention table: a space's notes and pages are
            // kept (forgetting drops texts, never entries).
            Kind::CtxNote | Kind::CtxPropose | Kind::CtxPage | Kind::CtxRemove | Kind::CtxLog | Kind::CtxForget | Kind::CtxSnapshot => Retention::Forever,
        }
    }

    /// Whether the author's machine may end an entry of this kind alone,
    /// so that a compact entry leaving it out ends it at every receiver.
    /// An intent or an assign carries the person's word, so only its
    /// cancel or its `until` ends it.
    pub fn ends_alone(self) -> bool {
        match self {
            Kind::Claim | Kind::Handoff | Kind::Report | Kind::Compact => true,
            Kind::Intent | Kind::Assign | Kind::Message | Kind::Receipt => false,
            // Never compacted, so never ended by absence.
            Kind::CtxNote | Kind::CtxPropose | Kind::CtxPage | Kind::CtxRemove | Kind::CtxLog | Kind::CtxForget | Kind::CtxSnapshot => false,
        }
    }

    /// Whether this is one of Xenolith's kinds (`ctx.*`), which belong in
    /// a space's feed. Which kinds a group takes is its engine's to check.
    pub fn is_ctx(self) -> bool {
        Kind::CTX.contains(&self)
    }
}

/// Whether a compact entry made at `at` that leaves out an applied entry
/// of `kind`, open until `until`, ends it.
fn ends_by_absence(kind: Kind, until: Option<u64>, at: u64) -> bool {
    kind.ends_alone() || until.is_some_and(|u| u < at)
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

/// The kept entry before another: its place and its id, none at the
/// origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Link {
    pub at: Pos,
    pub id: Option<[u8; 32]>,
}

impl Link {
    pub const ORIGIN: Link = Link { at: Pos::ORIGIN, id: None };

    pub fn to(at: Pos, id: [u8; 32]) -> Link {
        Link { at, id: Some(id) }
    }

    /// An id for a real place and none for the origin, and no place 0 of
    /// an incarnation, which names nothing.
    fn well_formed(self) -> bool {
        (self.at.seq == 0) == (self.at.inc == 0) && self.id.is_some() == (self.at != Pos::ORIGIN)
    }

    fn to_json(self) -> Value {
        let mut m = Map::new();
        m.insert("inc".into(), Value::from(self.at.inc));
        m.insert("seq".into(), Value::from(self.at.seq));
        if let Some(id) = self.id {
            m.insert("id".into(), Value::String(b64(&id)));
        }
        Value::Object(m)
    }

    fn parse(v: &Value) -> Option<Link> {
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        let id = match v.get("id") {
            None => None,
            Some(i) => Some(unb64_n::<32>(i.as_str()?)?),
        };
        Some(Link { at: Pos { inc: n("inc")?, seq: n("seq")? }, id })
    }
}

/// The next incarnation after `last`, at `now` (ms): `max(now, last + 1)`,
/// so it is past every one before even when the clock went back. `last`
/// is the highest incarnation the author knows of, its store's or one a
/// receiver's `Ack` named.
pub fn next_incarnation(last: u64, now: u64) -> u64 {
    now.max(last.saturating_add(1))
}

/// What a feed is scoped to, by its id's prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// A network: `net_` and 32 hex digits.
    Network,
    /// A space: `spc_` and 32 base32 characters.
    Space,
}

impl Group {
    /// Which group `id` names, or None when it names neither.
    pub fn of(id: &str) -> Option<Group> {
        if is_network_id(id) {
            Some(Group::Network)
        } else if SpaceId::parse(id).is_ok() {
            Some(Group::Space)
        } else {
            None
        }
    }
}

/// What a compact entry says (the body of a `Kind::Compact` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compact {
    /// The incarnation it compacts, its own.
    pub inc: u64,
    /// The last place it covers: every kept entry before it in `inc`.
    pub upto: u64,
    /// The places in `inc` up to `upto` still open, in order.
    pub live: Vec<u64>,
    /// The id of the entry at `upto`, where the header chain starts.
    pub at_id: [u8; 32],
    /// The incarnation's link: what its first kept entry names.
    pub base: Link,
}

impl Compact {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("inc".into(), Value::from(self.inc));
        m.insert("upto".into(), Value::from(self.upto));
        m.insert("live".into(), Value::Array(self.live.iter().map(|s| Value::from(*s)).collect()));
        m.insert("atId".into(), Value::String(b64(&self.at_id)));
        m.insert("base".into(), self.base.to_json());
        Value::Object(m)
    }

    pub fn parse(v: &Value) -> Option<Compact> {
        let live = v.get("live")?.as_array()?.iter().map(Value::as_u64).collect::<Option<Vec<_>>>()?;
        Some(Compact { inc: v.get("inc")?.as_u64()?, upto: v.get("upto")?.as_u64()?, live, at_id: unb64_n::<32>(v.get("atId")?.as_str()?)?, base: Link::parse(v.get("base")?)? })
    }

    /// Whether it lists the place `p` as open.
    pub fn keeps(&self, p: Pos) -> bool {
        p.inc == self.inc && self.live.binary_search(&p.seq).is_ok()
    }

    /// What it holds to, in the entry `e` that carries it.
    fn fits(&self, e: &FeedEntry) -> bool {
        let increasing = self.live.windows(2).all(|w| w[0] < w[1]);
        let in_range = self.live.first().is_none_or(|s| *s >= 1) && self.live.last().is_none_or(|s| *s <= self.upto);
        self.inc == e.inc
            && self.upto >= 1
            && self.upto.checked_add(1) == Some(e.seq)
            && increasing
            && in_range
            && e.prev.and_then(|p| p.id) == Some(self.at_id)
            && self.base.well_formed()
            && self.base.at.inc < self.inc
    }
}

/// The places a compact entry made at `now` keeps open: the author's kept
/// entries (`held`, the ones it still has) that their kind's retention
/// still keeps, `open` saying which the engine holds open. All of them
/// must be in incarnation `inc`, the compact entry's: an open entry of an
/// earlier one is written again before compacting.
pub fn live_set(held: &[FeedEntry], inc: u64, now: u64, open: impl Fn(&FeedEntry) -> bool) -> R<Vec<u64>> {
    let mut live = Vec::new();
    for e in held.iter().filter(|e| e.class() == Class::Kept) {
        let keep = match e.kind.retention() {
            Retention::WhileOpen => open(e),
            Retention::Aged(ms) => e.at.saturating_add(ms) >= now,
            Retention::Until => e.until().is_none_or(|u| u >= now),
            Retention::Superseded => false,
            Retention::Forever => true,
        };
        if !keep {
            continue;
        }
        if e.inc != inc {
            return Err(FeedError::refused(format!("the {} at {}.{} is still open: it's written again before its feed is compacted", e.kind.as_str(), e.inc, e.seq)));
        }
        live.push(e.seq);
    }
    live.sort_unstable();
    live.dedup();
    Ok(live)
}

/// An entry without its body: what its author signed, the body as its
/// hash. Enough to check an entry's id and the one before it, which is
/// what a compacted range's header chain is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub author: String,
    pub group: String,
    pub inc: u64,
    pub seq: u64,
    pub prev: Option<Link>,
    pub kind: Kind,
    pub at: u64,
    pub body_hash: [u8; 32],
}

impl Header {
    pub fn pos(&self) -> Pos {
        Pos { inc: self.inc, seq: self.seq }
    }

    fn signed(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("author".into(), Value::String(self.author.clone()));
        m.insert("group".into(), Value::String(self.group.clone()));
        m.insert("inc".into(), Value::from(self.inc));
        m.insert("seq".into(), Value::from(self.seq));
        if let Some(p) = self.prev {
            m.insert("prev".into(), p.to_json());
        }
        m.insert("kind".into(), Value::String(self.kind.as_str().into()));
        m.insert("at".into(), Value::from(self.at));
        m.insert("bodyHash".into(), Value::String(b64(&self.body_hash)));
        m
    }

    /// What the author signs: the tag, a newline, the header's canonical
    /// JSON.
    pub fn signing_bytes(&self) -> Vec<u8> {
        Purpose::FEED.bytes(Value::Object(self.signed()).to_string().as_bytes())
    }

    /// The entry's id, as bytes.
    pub fn hash(&self) -> [u8; 32] {
        sha256(&self.signing_bytes())
    }

    pub fn id(&self) -> String {
        b64(&self.hash())
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.signed())
    }

    pub fn parse(v: &Value) -> R<Header> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| FeedError::bad(k));
        let n = |k: &str| v.get(k).and_then(Value::as_u64).ok_or_else(|| FeedError::bad(k));
        let prev = match v.get("prev") {
            None => None,
            Some(p) => Some(Link::parse(p).filter(|l| l.well_formed()).ok_or_else(|| FeedError::bad("prev"))?),
        };
        let h = Header {
            author: s("author")?.into(),
            group: s("group")?.into(),
            inc: n("inc")?,
            seq: n("seq")?,
            prev,
            kind: Kind::parse(s("kind")?).ok_or_else(|| FeedError::bad("kind"))?,
            at: n("at")?,
            body_hash: unb64_n::<32>(s("bodyHash")?).ok_or_else(|| FeedError::bad("bodyHash"))?,
        };
        if !is_key_id(&h.author) || Group::of(&h.group).is_none() || h.inc == 0 || h.seq == 0 {
            return Err(FeedError::bad("header"));
        }
        Ok(h)
    }
}

/// One entry of a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// The author's machine signing key id, `xns1_…`.
    pub author: String,
    /// The group's id: a network's `net_…` or a space's `spc_…`.
    pub group: String,
    /// The author's incarnation, from 1.
    pub inc: u64,
    /// From 1 in each incarnation, kept and loose kinds each counted on
    /// their own.
    pub seq: u64,
    /// On kept entries only: the kept entry before it, the incarnation's
    /// link for its first.
    pub prev: Option<Link>,
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

    /// When an intent or an assign stops holding, from its body's `until`
    /// (ms).
    pub fn until(&self) -> Option<u64> {
        self.body.get("until").and_then(Value::as_u64)
    }

    /// What a compact entry says.
    pub fn compact(&self) -> Option<Compact> {
        (self.kind == Kind::Compact).then(|| Compact::parse(&self.body)).flatten()
    }

    /// The entry as it's signed, its body as its hash.
    pub fn header(&self) -> Header {
        Header {
            author: self.author.clone(),
            group: self.group.clone(),
            inc: self.inc,
            seq: self.seq,
            prev: self.prev,
            kind: self.kind,
            at: self.at,
            body_hash: sha256(self.body.to_string().as_bytes()),
        }
    }

    /// What the author signs: the header's signing bytes.
    pub fn signing_bytes(&self) -> Vec<u8> {
        self.header().signing_bytes()
    }

    /// The SHA-256 of what's signed: what tells a duplicate from a fork.
    pub fn hash(&self) -> [u8; 32] {
        self.header().hash()
    }

    /// The hash in base64url: the entry's id, by which loose entries are
    /// taken once.
    pub fn id(&self) -> String {
        b64(&self.hash())
    }

    /// The entry as it's kept and sent: its canonical JSON with its body,
    /// signature last.
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("author".into(), Value::String(self.author.clone()));
        m.insert("group".into(), Value::String(self.group.clone()));
        m.insert("inc".into(), Value::from(self.inc));
        m.insert("seq".into(), Value::from(self.seq));
        if let Some(p) = self.prev {
            m.insert("prev".into(), p.to_json());
        }
        m.insert("kind".into(), Value::String(self.kind.as_str().into()));
        m.insert("at".into(), Value::from(self.at));
        m.insert("body".into(), self.body.clone());
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
        self.sig = key.sign(Purpose::FEED, Value::Object(self.header().signed()).to_string().as_bytes())?;
        Ok(())
    }

    /// What `parse` holds an entry to, for one made here: an entry goes
    /// out only if every receiver can read it back.
    pub fn well_formed(&self) -> R<()> {
        if !is_key_id(&self.author) {
            return Err(FeedError::bad("author"));
        }
        if Group::of(&self.group).is_none() {
            return Err(FeedError::bad("group"));
        }
        if self.inc == 0 {
            return Err(FeedError::bad("incarnation"));
        }
        if self.seq == 0 {
            return Err(FeedError::bad("seq"));
        }
        let prev_fits = match (self.class(), self.prev) {
            (Class::Loose, None) => true,
            (Class::Loose, Some(_)) | (Class::Kept, None) => false,
            // The first names a place before its incarnation; every other
            // the place just before it.
            (Class::Kept, Some(p)) if self.seq == 1 => p.well_formed() && p.at.inc < self.inc,
            (Class::Kept, Some(p)) => p.id.is_some() && p.at == Pos { inc: self.inc, seq: self.seq - 1 },
        };
        if !prev_fits {
            return Err(FeedError::bad("prev"));
        }
        if !self.body.is_object() {
            return Err(FeedError::bad("body"));
        }
        if self.kind == Kind::Compact && !self.compact().is_some_and(|c| c.fits(self)) {
            return Err(FeedError::bad("compact body"));
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
            Some(p) => Some(Link::parse(p).ok_or_else(|| FeedError::bad("prev"))?),
        };
        let e = FeedEntry {
            author: s("author")?.into(),
            group: s("group")?.into(),
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
    /// key as the receiver's chain of the group has it.
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

/// The author's answer to a Want below its floor: its latest compact
/// entry, the live entries of the range from the place wanted on, and the
/// headers of the range from that place to `upto`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compacted {
    pub compact: FeedEntry,
    pub live: Vec<FeedEntry>,
    pub headers: Vec<Header>,
}

impl Compacted {
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("compact".into(), self.compact.to_json());
        m.insert("live".into(), Value::Array(self.live.iter().map(FeedEntry::to_json).collect()));
        m.insert("headers".into(), Value::Array(self.headers.iter().map(Header::to_json).collect()));
        Value::Object(m)
    }

    pub fn parse(v: &Value) -> R<Compacted> {
        let list = |k: &str| v.get(k).and_then(Value::as_array).ok_or_else(|| FeedError::bad(k));
        Ok(Compacted {
            compact: FeedEntry::parse(v.get("compact").ok_or_else(|| FeedError::bad("compact"))?)?,
            live: list("live")?.iter().map(FeedEntry::parse).collect::<R<_>>()?,
            headers: list("headers")?.iter().map(Header::parse).collect::<R<_>>()?,
        })
    }

    /// The answer, once the compact entry and every live entry check
    /// against `key` and all of it is one author's feed in one group.
    pub fn verify(self, key: &VerifyingKey) -> R<VerifiedCompacted> {
        if self.compact.kind != Kind::Compact {
            return Err(FeedError::bad("compact entry"));
        }
        let compact = self.compact.verify(key)?;
        let same = |author: &str, group: &str| author == compact.author && group == compact.group;
        if !self.live.iter().all(|e| same(&e.author, &e.group) && e.class() == Class::Kept) || !self.headers.iter().all(|h| same(&h.author, &h.group)) {
            return Err(FeedError::refused("a compacted range mixes feeds"));
        }
        let live = self.live.into_iter().map(|e| e.verify(key)).collect::<R<Vec<_>>>()?;
        Ok(VerifiedCompacted { compact, live, headers: self.headers })
    }
}

/// A `Compacted` whose entries' signatures were checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedCompacted {
    compact: Verified,
    live: Vec<Verified>,
    headers: Vec<Header>,
}

impl VerifiedCompacted {
    /// The compact entry it carries: its author, group and place.
    pub fn compact(&self) -> &Verified {
        &self.compact
    }
}

/// The author's side: numbers and signs its entries in one group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writer {
    author: String,
    group: String,
    inc: u64,
    kept: u64,
    loose: u64,
    /// What the incarnation's first kept entry names.
    link: Link,
    /// The last kept entry written, the link until the first.
    head: Link,
}

impl Writer {
    /// A feed's first incarnation, at `now` (ms).
    pub fn start(author: impl Into<String>, group: impl Into<String>, now: u64) -> Writer {
        Writer { author: author.into(), group: group.into(), inc: next_incarnation(0, now), kept: 0, loose: 0, link: Link::ORIGIN, head: Link::ORIGIN }
    }

    /// Where the author left off, as its store has it: the same
    /// incarnation after a restart, its link and the last kept entry it
    /// wrote (the link while it has written none).
    pub fn resume(author: impl Into<String>, group: impl Into<String>, inc: u64, kept: u64, loose: u64, link: Link, head: Link) -> Writer {
        Writer { author: author.into(), group: group.into(), inc, kept, loose, link, head }
    }

    /// After a restore or a new setup: a new incarnation past `last` (the
    /// highest the author knows of), whose first kept entry names `held`,
    /// the last kept entry the author still has (`Link::ORIGIN` for none).
    pub fn restore(author: impl Into<String>, group: impl Into<String>, held: Link, last: u64, now: u64) -> Writer {
        let inc = next_incarnation(last.max(held.at.inc), now);
        Writer { author: author.into(), group: group.into(), inc, kept: 0, loose: 0, link: held, head: held }
    }

    pub fn inc(&self) -> u64 {
        self.inc
    }

    /// The kept entries written in this incarnation.
    pub fn kept(&self) -> u64 {
        self.kept
    }

    /// The loose entries written in this incarnation.
    pub fn loose(&self) -> u64 {
        self.loose
    }

    /// What the incarnation's first kept entry names: with `inc`, `kept`,
    /// `loose` and `head`, what `resume` takes back.
    pub fn link(&self) -> Link {
        self.link
    }

    /// The last kept entry written, or the one before this incarnation.
    pub fn head(&self) -> Link {
        self.head
    }

    /// The next entry, unsigned, numbered and counted.
    pub fn next(&mut self, kind: Kind, at: u64, body: Value) -> FeedEntry {
        let (seq, prev) = match kind.class() {
            Class::Kept => (self.kept + 1, Some(self.head)),
            Class::Loose => (self.loose + 1, None),
        };
        let e = FeedEntry { author: self.author.clone(), group: self.group.clone(), inc: self.inc, seq, prev, kind, at, body, sig: Vec::new() };
        match kind.class() {
            Class::Kept => {
                self.kept = seq;
                self.head = Link::to(e.pos(), e.hash());
            }
            Class::Loose => self.loose = seq,
        }
        e
    }

    /// The next entry, signed by `key`. Counted only once signed, so a key
    /// that fails leaves no hole in the sequence. A compact entry is
    /// written by `compact`.
    pub fn write(&mut self, key: &dyn Signer, kind: Kind, at: u64, body: Value) -> xenonet_id::R<FeedEntry> {
        if kind == Kind::Compact {
            return Err(xenonet_id::IdError::refused("a compact entry is written as one (Writer::compact)"));
        }
        self.signed(key, kind, at, body)
    }

    /// A compact entry over everything this incarnation has written,
    /// keeping `live` open (`live_set` says which).
    pub fn compact(&mut self, key: &dyn Signer, at: u64, live: &[u64]) -> xenonet_id::R<FeedEntry> {
        let (Some(at_id), true) = (self.head.id, self.kept >= 1 && self.head.at.inc == self.inc) else {
            return Err(xenonet_id::IdError::refused("this incarnation has nothing to compact"));
        };
        let mut live = live.to_vec();
        live.sort_unstable();
        live.dedup();
        if live.iter().any(|s| *s == 0 || *s > self.kept) {
            return Err(xenonet_id::IdError::refused("a compact entry keeps only places it covers"));
        }
        let body = Compact { inc: self.inc, upto: self.kept, live, at_id, base: self.link }.to_json();
        self.signed(key, Kind::Compact, at, body)
    }

    fn signed(&mut self, key: &dyn Signer, kind: Kind, at: u64, body: Value) -> xenonet_id::R<FeedEntry> {
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
    pub group: String,
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
    /// The id of the other one: the entry offered there, or the one an
    /// offered entry or header chain names there.
    pub offered: [u8; 32],
}

/// Where a receiver stands with one author in one group.
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

/// What an offer applied: the entries, in order, and the places of the
/// entries applied before that a compact entry among them ended.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applied {
    pub entries: Vec<FeedEntry>,
    pub ended: Vec<Pos>,
}

impl From<Vec<FeedEntry>> for Applied {
    fn from(entries: Vec<FeedEntry>) -> Applied {
        Applied { entries, ended: Vec::new() }
    }
}

impl std::ops::Deref for Applied {
    type Target = [FeedEntry];
    fn deref(&self) -> &[FeedEntry] {
        &self.entries
    }
}

impl IntoIterator for Applied {
    type Item = FeedEntry;
    type IntoIter = std::vec::IntoIter<FeedEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a Applied {
    type Item = &'a FeedEntry;
    type IntoIter = std::slice::Iter<'a, FeedEntry>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

/// What a feed did with an offered entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// Applied: this entry, then any held entries it released, in order.
    New(Applied),
    Duplicate,
    /// Held; ask the author for what's missing.
    Gap(Want),
    /// The author signed two entries at one place: its feed is frozen.
    Fork(Fork),
    /// The author's feed was frozen by an earlier fork; dropped.
    Frozen(Fork),
    /// From history the author gave up in a restore, or a compact entry
    /// covered; dropped.
    Stale,
    /// A loose entry past retention by its signed time; refused.
    Expired,
}

/// An applied kept entry that is still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Open {
    pub at: Pos,
    pub kind: Kind,
    pub until: Option<u64>,
}

/// What a receiver keeps of each author's feed in each group. The store
/// implements it on its tables, called inside the transaction that applies
/// what `Feed::offer` returns; entries come back as they were stored,
/// their signatures already checked.
pub trait FeedStore {
    type Error;

    /// Where the receiver stands; the default `Head` for an author it has
    /// nothing from.
    fn head(&self, author: &str, group: &str) -> Result<Head, Self::Error>;
    fn set_head(&mut self, author: &str, group: &str, head: Head) -> Result<(), Self::Error>;
    /// The id of the kept entry applied at `at`, or that a compacted
    /// range's header chain put there, if any.
    fn kept(&self, author: &str, group: &str, at: Pos) -> Result<Option<[u8; 32]>, Self::Error>;
    /// Records a kept entry as applied, and open.
    fn keep(&mut self, e: &FeedEntry) -> Result<(), Self::Error>;
    /// Records the id a header chain puts at a place the receiver skipped.
    fn know(&mut self, h: &Header) -> Result<(), Self::Error>;
    /// The applied entries still open at or before `upto`, in order.
    fn open_upto(&self, author: &str, group: &str, upto: Pos) -> Result<Vec<Open>, Self::Error>;
    /// The entry applied at `at` is no longer open.
    fn close(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Self::Error>;
    /// Whether the loose entry with this id was taken.
    fn has_loose(&self, author: &str, group: &str, id: &[u8; 32]) -> Result<bool, Self::Error>;
    /// Records a loose entry as taken.
    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Self::Error>;
    /// Forgets the loose entries taken whose signed time is before
    /// `before`, every author's.
    fn prune_loose(&mut self, before: u64) -> Result<(), Self::Error>;
    /// The kept entry held (not yet applied) at `at`.
    fn held(&self, author: &str, group: &str, at: Pos) -> Result<Option<FeedEntry>, Self::Error>;
    /// The kept entries held in incarnation `inc` at or before `upto`, in
    /// order.
    fn held_in(&self, author: &str, group: &str, inc: u64, upto: u64) -> Result<Vec<FeedEntry>, Self::Error>;
    /// Holds a kept entry until the entries before it arrive.
    fn hold(&mut self, e: FeedEntry) -> Result<(), Self::Error>;
    fn unhold(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Self::Error>;
    /// The held first entries (seq 1) of incarnations after `inc`, lowest
    /// incarnation first.
    fn held_firsts(&self, author: &str, group: &str, after: u64) -> Result<Vec<FeedEntry>, Self::Error>;
    /// Drops the held entries of incarnations before `inc`, which can no
    /// longer apply.
    fn drop_held_before(&mut self, author: &str, group: &str, inc: u64) -> Result<(), Self::Error>;
}

impl<S: FeedStore + ?Sized> FeedStore for &mut S {
    type Error = S::Error;
    fn head(&self, author: &str, group: &str) -> Result<Head, Self::Error> {
        (**self).head(author, group)
    }
    fn set_head(&mut self, author: &str, group: &str, head: Head) -> Result<(), Self::Error> {
        (**self).set_head(author, group, head)
    }
    fn kept(&self, author: &str, group: &str, at: Pos) -> Result<Option<[u8; 32]>, Self::Error> {
        (**self).kept(author, group, at)
    }
    fn keep(&mut self, e: &FeedEntry) -> Result<(), Self::Error> {
        (**self).keep(e)
    }
    fn know(&mut self, h: &Header) -> Result<(), Self::Error> {
        (**self).know(h)
    }
    fn open_upto(&self, author: &str, group: &str, upto: Pos) -> Result<Vec<Open>, Self::Error> {
        (**self).open_upto(author, group, upto)
    }
    fn close(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Self::Error> {
        (**self).close(author, group, at)
    }
    fn has_loose(&self, author: &str, group: &str, id: &[u8; 32]) -> Result<bool, Self::Error> {
        (**self).has_loose(author, group, id)
    }
    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Self::Error> {
        (**self).take_loose(e)
    }
    fn prune_loose(&mut self, before: u64) -> Result<(), Self::Error> {
        (**self).prune_loose(before)
    }
    fn held(&self, author: &str, group: &str, at: Pos) -> Result<Option<FeedEntry>, Self::Error> {
        (**self).held(author, group, at)
    }
    fn held_in(&self, author: &str, group: &str, inc: u64, upto: u64) -> Result<Vec<FeedEntry>, Self::Error> {
        (**self).held_in(author, group, inc, upto)
    }
    fn hold(&mut self, e: FeedEntry) -> Result<(), Self::Error> {
        (**self).hold(e)
    }
    fn unhold(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Self::Error> {
        (**self).unhold(author, group, at)
    }
    fn held_firsts(&self, author: &str, group: &str, after: u64) -> Result<Vec<FeedEntry>, Self::Error> {
        (**self).held_firsts(author, group, after)
    }
    fn drop_held_before(&mut self, author: &str, group: &str, inc: u64) -> Result<(), Self::Error> {
        (**self).drop_held_before(author, group, inc)
    }
}

/// What a drain does with a held entry that turns out to name another
/// entry before it than the one applied there.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OnFork {
    Freeze,
    /// After the person unfroze the feed, keeping what the receiver had.
    Drop,
}

/// A receiver's feeds: every author's in every group, over a store.
#[derive(Debug, Clone, Default)]
pub struct Feed<S> {
    store: S,
    /// Now, in ms: what loose entries' age is measured against.
    now: u64,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl<S: FeedStore> Feed<S> {
    /// A receiver's feeds, as of the system clock.
    pub fn new(store: S) -> Feed<S> {
        Feed::at(store, now_ms())
    }

    /// A receiver's feeds, as of `now` (ms).
    pub fn at(store: S, now: u64) -> Feed<S> {
        Feed { store, now }
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
        let (author, group) = (e.author.clone(), e.group.clone());
        let mut head = self.store.head(&author, &group)?;
        if let Some(f) = head.frozen {
            return Ok(Offer::Frozen(f));
        }
        if e.class() == Class::Loose {
            if e.at.saturating_add(RETENTION_MS) < self.now {
                return Ok(Offer::Expired);
            }
            self.store.prune_loose(self.now.saturating_sub(LOOSE_DEDUPE_MS))?;
            if self.store.has_loose(&author, &group, &e.hash())? {
                return Ok(Offer::Duplicate);
            }
            self.store.take_loose(&e)?;
            return Ok(Offer::New(vec![e].into()));
        }
        if let Some(o) = self.check(&author, &group, &mut head, &e)? {
            return Ok(o);
        }
        let at = e.pos();
        let latest = head.latest;
        let applies = match at.inc.cmp(&latest.inc) {
            std::cmp::Ordering::Equal => at.seq == latest.seq + 1,
            // A later incarnation links at what the receiver holds, or
            // below it after a restore; above it, something's missing.
            std::cmp::Ordering::Greater => at.seq == 1 && e.prev.is_some_and(|p| p.at <= latest),
            std::cmp::Ordering::Less => return Ok(Offer::Stale),
        };
        if !applies {
            // At or below the head with no entry there means the store
            // lost what it applied; nothing here can mend that.
            if at.inc == latest.inc && at.seq <= latest.seq {
                return Ok(Offer::Stale);
            }
            // A compact entry ahead of the receiver is held too: only its
            // header chain (`offer_compacted`) or the entries before it
            // let it apply.
            self.store.hold(e)?;
            return Ok(Offer::Gap(want(author, group, latest)));
        }
        let mut out = Applied::default();
        self.apply(&mut head, e, &mut out)?;
        self.drain(&author, &group, &mut head, &mut out, OnFork::Freeze)?;
        self.store.set_head(&author, &group, head)?;
        Ok(Offer::New(out))
    }

    /// Takes the author's answer to a Want below its floor: checks the
    /// header chain from the compact entry's `atId` down to what the
    /// receiver applied and holds in the range, drops what it held there
    /// that isn't live, applies the live entries it lacks and the compact
    /// entry, and keeps the chain's ids for the places it skipped.
    pub fn offer_compacted(&mut self, c: VerifiedCompacted) -> Result<Offer, S::Error> {
        let VerifiedCompacted { compact, live, headers } = c;
        let e = compact.into_entry();
        let Some(body) = e.compact() else { return Ok(Offer::Stale) };
        let (author, group) = (e.author.clone(), e.group.clone());
        let mut head = self.store.head(&author, &group)?;
        if let Some(f) = head.frozen {
            return Ok(Offer::Frozen(f));
        }
        // Held already, when it came alone before its chain did: it's
        // taken from the hold here, and held again if it still can't apply.
        if self.store.held(&author, &group, e.pos())?.is_some_and(|h| h.hash() == e.hash()) {
            self.store.unhold(&author, &group, e.pos())?;
        }
        if let Some(o) = self.check(&author, &group, &mut head, &e)? {
            return Ok(o);
        }
        let latest = head.latest;
        if e.inc < latest.inc || (e.inc == latest.inc && e.seq <= latest.seq) {
            return Ok(Offer::Stale);
        }
        if e.inc == latest.inc && latest.seq == body.upto {
            // Nothing between: it applies as any next entry does.
            return self.offer(Verified(e));
        }
        if e.inc > latest.inc && body.base.at > latest {
            // Something before the incarnation's link is missing.
            self.store.hold(e)?;
            return Ok(Offer::Gap(want(author, group, latest)));
        }
        let lo = if e.inc == latest.inc { latest.seq + 1 } else { 1 };
        // The header chain, from `atId` down to `lo`: each names the one
        // below it, and the lowest names the receiver's latest, or the
        // incarnation's link. One that doesn't is a bad answer, not the
        // author's word: hold the compact entry and ask again.
        let chain: BTreeMap<u64, Header> = headers.into_iter().filter(|h| h.inc == e.inc && (lo..=body.upto).contains(&h.seq)).map(|h| (h.seq, h)).collect();
        let mut expect = Some(body.at_id);
        let mut whole = true;
        for s in (lo..=body.upto).rev() {
            let Some(h) = chain.get(&s) else {
                whole = false;
                break;
            };
            let below = if s == 1 { Some(body.base) } else { h.prev.filter(|p| p.at == Pos::new(e.inc, s - 1)) };
            if Some(h.hash()) != expect || below.is_none() || (s == 1 && h.prev != Some(body.base)) {
                whole = false;
                break;
            }
            expect = below.and_then(|b| b.id);
        }
        if !whole {
            self.store.hold(e)?;
            return Ok(Offer::Gap(want(author, group, latest)));
        }
        let id_at = |s: u64| chain.get(&s).map(Header::hash);
        // The chain passes through the receiver's latest entry, or it's
        // another side of a fork.
        if e.inc == latest.inc {
            let named = chain.get(&lo).and_then(|h| h.prev).and_then(|p| p.id);
            if let (Some(held), Some(named)) = (self.store.kept(&author, &group, latest)?, named) {
                if held != named {
                    return self.freeze(&author, &group, &mut head, Fork { at: latest, held, offered: named });
                }
            }
        }
        // What the receiver holds in the range, and the live entries that
        // came, are on the chain, or the author forked there.
        let held = self.store.held_in(&author, &group, e.inc, body.upto)?;
        let mut ready: BTreeMap<u64, FeedEntry> = BTreeMap::new();
        for x in held.into_iter().chain(live.into_iter().map(Verified::into_entry)) {
            if x.inc != e.inc || x.seq < lo || x.seq > body.upto {
                continue;
            }
            let on = id_at(x.seq).unwrap_or_default();
            if x.hash() != on {
                return self.freeze(&author, &group, &mut head, Fork { at: x.pos(), held: on, offered: x.hash() });
            }
            ready.insert(x.seq, x);
        }
        let wanted: Vec<u64> = body.live.iter().copied().filter(|s| *s >= lo).collect();
        if !wanted.iter().all(|s| ready.contains_key(s)) {
            // A live entry is missing: the answer is short.
            self.store.hold(e)?;
            return Ok(Offer::Gap(want(author, group, latest)));
        }
        // Held entries that aren't live are dropped, never applied.
        for s in ready.keys().copied().collect::<Vec<_>>() {
            self.store.unhold(&author, &group, Pos::new(e.inc, s))?;
        }
        let mut out = Applied::default();
        for s in lo..=body.upto {
            match ready.remove(&s) {
                Some(x) if body.keeps(x.pos()) => self.apply(&mut head, x, &mut out)?,
                _ => {
                    if let Some(h) = chain.get(&s) {
                        self.store.know(h)?;
                    }
                }
            }
        }
        self.store.unhold(&author, &group, e.pos())?;
        self.apply(&mut head, e, &mut out)?;
        self.drain(&author, &group, &mut head, &mut out, OnFork::Freeze)?;
        self.store.set_head(&author, &group, head)?;
        Ok(Offer::New(out))
    }

    /// The checks by place before anything else: the same entry again, or
    /// another at its place, or one naming another entry before it than
    /// the one applied there.
    fn check(&mut self, author: &str, group: &str, head: &mut Head, e: &FeedEntry) -> Result<Option<Offer>, S::Error> {
        let at = e.pos();
        let id = e.hash();
        let had = match self.store.kept(author, group, at)? {
            Some(h) => Some(h),
            None => self.store.held(author, group, at)?.map(|h| h.hash()),
        };
        if let Some(held) = had {
            if held == id {
                return Ok(Some(Offer::Duplicate));
            }
            return self.freeze(author, group, head, Fork { at, held, offered: id }).map(Some);
        }
        if let Some(f) = self.link_fork(author, group, e)? {
            return self.freeze(author, group, head, f).map(Some);
        }
        Ok(None)
    }

    /// The fork an entry shows by naming, as the entry before it or as its
    /// incarnation's link, another id than the one applied there.
    fn link_fork(&self, author: &str, group: &str, e: &FeedEntry) -> Result<Option<Fork>, S::Error> {
        for link in [e.prev, e.compact().map(|c| c.base)].into_iter().flatten() {
            if let (Some(named), Some(held)) = (link.id, self.store.kept(author, group, link.at)?) {
                if named != held {
                    return Ok(Some(Fork { at: link.at, held, offered: named }));
                }
            }
        }
        Ok(None)
    }

    fn freeze(&mut self, author: &str, group: &str, head: &mut Head, fork: Fork) -> Result<Offer, S::Error> {
        head.frozen = Some(fork);
        self.store.set_head(author, group, *head)?;
        Ok(Offer::Fork(fork))
    }

    fn apply(&mut self, head: &mut Head, e: FeedEntry, out: &mut Applied) -> Result<(), S::Error> {
        if e.inc != head.latest.inc {
            // Linking a later incarnation closes the ones before it: what's
            // held of them is past where the author said they ended.
            self.store.drop_held_before(&e.author, &e.group, e.inc)?;
        }
        self.store.keep(&e)?;
        head.latest = e.pos();
        if let Some(c) = e.compact() {
            // Authoritative about what is open: what it leaves out ends,
            // as far as the author may end it alone.
            for o in self.store.open_upto(&e.author, &e.group, Pos::new(c.inc, c.upto))? {
                if c.keeps(o.at) {
                    continue;
                }
                if o.kind == Kind::Compact {
                    self.store.close(&e.author, &e.group, o.at)?;
                } else if ends_by_absence(o.kind, o.until, e.at) {
                    self.store.close(&e.author, &e.group, o.at)?;
                    out.ended.push(o.at);
                }
            }
        }
        out.entries.push(e);
        Ok(())
    }

    /// Applies what's held and now in order.
    fn drain(&mut self, author: &str, group: &str, head: &mut Head, out: &mut Applied, on_fork: OnFork) -> Result<(), S::Error> {
        loop {
            let next = Pos { inc: head.latest.inc, seq: head.latest.seq + 1 };
            let mut e = if next.inc > 0 { self.store.held(author, group, next)? } else { None };
            if e.is_none() {
                let firsts = self.store.held_firsts(author, group, head.latest.inc)?;
                e = firsts.into_iter().find(|e| e.prev.is_some_and(|p| p.at <= head.latest));
            }
            let Some(e) = e else { return Ok(()) };
            if let Some(f) = self.link_fork(author, group, &e)? {
                if on_fork == OnFork::Freeze {
                    head.frozen = Some(f);
                    return Ok(());
                }
                self.store.unhold(author, group, e.pos())?;
                continue;
            }
            self.store.unhold(author, group, e.pos())?;
            self.apply(head, e, out)?;
        }
    }

    /// What the receiver acknowledges to `author`; None before its first
    /// kept entry.
    pub fn ack(&self, author: &str, group: &str) -> Result<Option<Ack>, S::Error> {
        let l = self.store.head(author, group)?.latest;
        Ok((l != Pos::ORIGIN).then_some(Ack { inc: l.inc, high: l.seq }))
    }

    /// The fork that froze `author`'s feed, if it's frozen.
    pub fn frozen(&self, author: &str, group: &str) -> Result<Option<Fork>, S::Error> {
        Ok(self.store.head(author, group)?.frozen)
    }

    /// Takes `author`'s feed again after a fork, keeping the entry the
    /// receiver had: only the person decides this, at the window or
    /// through the doctor, never an agent or another machine. Returns the
    /// held entries it released; held entries off the receiver's chain
    /// are dropped.
    pub fn unfreeze(&mut self, author: &str, group: &str) -> Result<Applied, S::Error> {
        let mut head = self.store.head(author, group)?;
        head.frozen = None;
        let mut out = Applied::default();
        self.drain(author, group, &mut head, &mut out, OnFork::Drop)?;
        self.store.set_head(author, group, head)?;
        Ok(out)
    }
}

fn want(author: String, group: String, latest: Pos) -> Want {
    Want { author, group, inc: latest.inc, from: latest.seq + 1 }
}

type Key = (String, String);

/// What a `MemoryStore` keeps of an applied place.
#[derive(Debug, Clone, Copy)]
struct Place {
    id: [u8; 32],
    kind: Kind,
    until: Option<u64>,
    open: bool,
}

/// A `FeedStore` in memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    heads: HashMap<Key, Head>,
    kept: HashMap<Key, BTreeMap<Pos, Place>>,
    /// Each loose entry taken, with its signed time.
    loose: HashMap<Key, HashMap<[u8; 32], u64>>,
    held: HashMap<Key, BTreeMap<Pos, FeedEntry>>,
}

fn key(author: &str, group: &str) -> Key {
    (author.to_string(), group.to_string())
}

impl MemoryStore {
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    /// How many kept entries are held for `author`.
    pub fn held_count(&self, author: &str, group: &str) -> usize {
        self.held.get(&key(author, group)).map_or(0, BTreeMap::len)
    }

    /// How many loose entries it remembers taking.
    pub fn loose_count(&self) -> usize {
        self.loose.values().map(HashMap::len).sum()
    }
}

impl FeedStore for MemoryStore {
    type Error = Infallible;

    fn head(&self, author: &str, group: &str) -> Result<Head, Infallible> {
        Ok(self.heads.get(&key(author, group)).copied().unwrap_or_default())
    }

    fn set_head(&mut self, author: &str, group: &str, head: Head) -> Result<(), Infallible> {
        self.heads.insert(key(author, group), head);
        Ok(())
    }

    fn kept(&self, author: &str, group: &str, at: Pos) -> Result<Option<[u8; 32]>, Infallible> {
        Ok(self.kept.get(&key(author, group)).and_then(|m| m.get(&at)).map(|p| p.id))
    }

    fn keep(&mut self, e: &FeedEntry) -> Result<(), Infallible> {
        let p = Place { id: e.hash(), kind: e.kind, until: e.until(), open: true };
        self.kept.entry(key(&e.author, &e.group)).or_default().insert(e.pos(), p);
        Ok(())
    }

    fn know(&mut self, h: &Header) -> Result<(), Infallible> {
        let p = Place { id: h.hash(), kind: h.kind, until: None, open: false };
        self.kept.entry(key(&h.author, &h.group)).or_default().entry(h.pos()).or_insert(p);
        Ok(())
    }

    fn open_upto(&self, author: &str, group: &str, upto: Pos) -> Result<Vec<Open>, Infallible> {
        let Some(m) = self.kept.get(&key(author, group)) else { return Ok(Vec::new()) };
        Ok(m.range(..=upto).filter(|(_, p)| p.open).map(|(at, p)| Open { at: *at, kind: p.kind, until: p.until }).collect())
    }

    fn close(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Infallible> {
        if let Some(p) = self.kept.get_mut(&key(author, group)).and_then(|m| m.get_mut(&at)) {
            p.open = false;
        }
        Ok(())
    }

    fn has_loose(&self, author: &str, group: &str, id: &[u8; 32]) -> Result<bool, Infallible> {
        Ok(self.loose.get(&key(author, group)).is_some_and(|s| s.contains_key(id)))
    }

    fn take_loose(&mut self, e: &FeedEntry) -> Result<(), Infallible> {
        self.loose.entry(key(&e.author, &e.group)).or_default().insert(e.hash(), e.at);
        Ok(())
    }

    fn prune_loose(&mut self, before: u64) -> Result<(), Infallible> {
        for m in self.loose.values_mut() {
            m.retain(|_, at| *at >= before);
        }
        Ok(())
    }

    fn held(&self, author: &str, group: &str, at: Pos) -> Result<Option<FeedEntry>, Infallible> {
        Ok(self.held.get(&key(author, group)).and_then(|m| m.get(&at)).cloned())
    }

    fn held_in(&self, author: &str, group: &str, inc: u64, upto: u64) -> Result<Vec<FeedEntry>, Infallible> {
        let Some(m) = self.held.get(&key(author, group)) else { return Ok(Vec::new()) };
        Ok(m.range(Pos::new(inc, 0)..=Pos::new(inc, upto)).map(|(_, e)| e.clone()).collect())
    }

    fn hold(&mut self, e: FeedEntry) -> Result<(), Infallible> {
        self.held.entry(key(&e.author, &e.group)).or_default().insert(e.pos(), e);
        Ok(())
    }

    fn unhold(&mut self, author: &str, group: &str, at: Pos) -> Result<(), Infallible> {
        if let Some(m) = self.held.get_mut(&key(author, group)) {
            m.remove(&at);
        }
        Ok(())
    }

    fn held_firsts(&self, author: &str, group: &str, after: u64) -> Result<Vec<FeedEntry>, Infallible> {
        let Some(m) = self.held.get(&key(author, group)) else { return Ok(Vec::new()) };
        Ok(m.range(Pos { inc: after.saturating_add(1), seq: 0 }..).filter(|(p, _)| p.seq == 1).map(|(_, e)| e.clone()).collect())
    }

    fn drop_held_before(&mut self, author: &str, group: &str, inc: u64) -> Result<(), Infallible> {
        if let Some(m) = self.held.get_mut(&key(author, group)) {
            m.retain(|p, _| p.inc >= inc);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod compaction_tests;
