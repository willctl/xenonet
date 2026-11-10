//! A signed hash chain, generic over its `Policy`.
//!
//! An entry is its index `n`, `prev` (the base64url SHA-256 of the
//! previous entry's canonical JSON with its signers; empty for entry 0),
//! the policy's body, and its signers. Each signer signs the entry's
//! canonical JSON without the signers, after the policy's tag and a
//! newline (`xenonet_id::sign::Purpose`), so a signature made for one
//! chain kind can't be taken for another's. A signer may carry a presence
//! block (`xenonet_id::presence`) for the entry, which a policy can ask
//! for; the block approves the entry's `id`, which is outside what it
//! signs, so it can be made before the signature and travels beside it.
//!
//! What every chain holds to, whatever its policy:
//!
//! - an entry extends the head: the next index (else a gap, 409) and the
//!   head's hash as `prev` (else a fork, 409);
//! - one offered where the chain already has an entry that says the same
//!   (`Policy::same`) with the same `prev` is a duplicate, dropped quietly
//!   (two signers approving the same thing at once); anything else there
//!   is a fork;
//! - one to four signers, each once, each with a signature that checks
//!   against the key the policy resolves for it;
//! - whatever else the policy's `check` says, which may answer
//!   `needs-approval` for an entry signed rightly but by too few: a relay
//!   may keep it as a proposal for more signers (`merge_signers`).
//!
//! The canonical JSON is `n`, `prev`, the body's fields in the policy's
//! order, then `signers`, compact, keys in that order (serde_json keeps
//! insertion order in this workspace), so a policy can keep a format byte
//! for byte.

use serde_json::{Map, Value};
use std::fmt;
use xenonet_id::codec::{b64, hash_b64, is_b64, sha256, unb64};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};

/// The most signers an entry carries.
pub const MAX_SIGNERS: usize = 4;

/// Why a chain didn't take an entry, or couldn't read one: an HTTP status
/// for the routes that pass it on, a short code, and words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ChainError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> ChainError {
        ChainError { status, code, message: message.into() }
    }

    /// Something that isn't right to read.
    pub fn bad(message: impl Into<String>) -> ChainError {
        ChainError::new(400, "bad-request", message)
    }

    /// Signed rightly, but by too few: it may wait for more signers.
    pub fn needs_approval(message: impl Into<String>) -> ChainError {
        ChainError::new(403, "needs-approval", message)
    }

    /// What the entry adds is there already.
    pub fn taken(message: impl Into<String>) -> ChainError {
        ChainError::new(409, "taken", message)
    }

    /// What the entry changes is what the chain has already.
    pub fn no_change(message: impl Into<String>) -> ChainError {
        ChainError::new(409, "no-change", message)
    }

    /// A version that isn't past the chain's.
    pub fn stale(message: impl Into<String>) -> ChainError {
        ChainError::new(409, "stale", message)
    }
}

impl fmt::Display for ChainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ChainError {}

pub type R<T> = Result<T, ChainError>;

/// The codes a chain kind answers with for its own three refusals, so a
/// network's say `roster-gap` and a person's `person-gap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Codes {
    pub gap: &'static str,
    pub fork: &'static str,
    pub refused: &'static str,
}

/// How a chain kind writes its entries and words its refusals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// What its signatures cover first.
    pub tag: Purpose,
    /// What the chain is called in messages: `roster`.
    pub noun: &'static str,
    /// What a signer is called in messages: `host`.
    pub signer: &'static str,
    /// The key a signer's id is written under.
    pub signer_key: &'static str,
    /// Whether a signer may carry a presence block. A chain kind that
    /// doesn't take them reads past one, so its entries' bytes stay what
    /// they were.
    pub presence: bool,
    pub codes: Codes,
}

impl Format {
    /// Refused by the chain's rules (403).
    pub fn refused(&self, message: impl Into<String>) -> ChainError {
        ChainError::new(403, self.codes.refused, message)
    }

    /// A part of an entry that doesn't read.
    pub fn bad(&self, what: &str) -> ChainError {
        ChainError::bad(format!("a {} entry's {what} isn't right", self.noun))
    }

    fn gap(&self, next: u64, got: u64) -> ChainError {
        ChainError::new(409, self.codes.gap, format!("the {}'s next entry is {next}, not {got}", self.noun))
    }

    fn fork(&self, message: String) -> ChainError {
        ChainError::new(409, self.codes.fork, message)
    }
}

/// A chain kind: what its entries say, how they're written, and who may
/// add each. The format half is associated functions, since an entry is
/// read and written without a chain at hand; the rules half takes `&self`,
/// so a policy can hold what it consults.
pub trait Policy: Clone + fmt::Debug {
    /// What an entry says, besides its place and its signers.
    type Body: Clone + PartialEq + fmt::Debug;
    /// What the chain's entries add up to.
    type State: Clone + PartialEq + fmt::Debug;

    const FORMAT: Format;

    /// The body's fields, in canonical order, into an entry's JSON after
    /// `n` and `prev`.
    fn write_body(body: &Self::Body, out: &mut Map<String, Value>);
    /// The body from an entry's JSON (the whole entry object).
    fn parse_body(v: &Value) -> R<Self::Body>;
    /// What `parse_body` holds a body to, for one made here: an entry the
    /// chain takes must read back from its own JSON.
    fn body_well_formed(body: &Self::Body) -> R<()>;
    /// Whether `id` could be a signer's id.
    fn is_signer(id: &str) -> bool;
    /// Whether `sig` has a signature's shape.
    fn sig_ok(sig: &[u8]) -> bool;
    /// Whether two bodies say the same thing: the duplicate test.
    fn same(a: &Self::Body, b: &Self::Body) -> bool;

    /// Whether `e` belongs on this chain at all, before any signature is
    /// checked: entry 0's kind and signer, the group an entry names.
    /// `state` is None before entry 0.
    fn scope(&self, state: Option<&Self::State>, e: &Entry<Self>) -> R<()> {
        let _ = (state, e);
        Ok(())
    }
    /// The key `by` signs `e` with, as `state` has it; refused when `by`
    /// can't sign here.
    fn signer_key(&self, state: Option<&Self::State>, e: &Entry<Self>, by: &str) -> R<VerifyingKey>;
    /// What a signature that doesn't check is answered with.
    fn bad_signature(&self, state: Option<&Self::State>, by: &str) -> ChainError {
        let _ = state;
        Self::FORMAT.refused(format!("{by}'s signature doesn't check"))
    }
    /// The rules for `e`'s body, its signatures already checked.
    fn check(&self, state: Option<&Self::State>, e: &Entry<Self>) -> R<()>;
    /// `state` after `e`, which `check` let through.
    fn apply(&self, state: Option<Self::State>, e: &Entry<Self>) -> Self::State;
}

/// One signer of an entry: its id, its signature, and the presence block
/// it brought, if the chain kind takes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub by: String,
    pub sig: Vec<u8>,
    pub presence: Option<PresenceBlock>,
}

/// An entry of a chain of kind `P`. It reads as its body (`Deref`), so a
/// policy's fields are `e.op`, not `e.body.op`; a body's own fields are
/// therefore never named `n`, `prev`, `body` or `signers`.
pub struct Entry<P: Policy> {
    pub n: u64,
    pub prev: String,
    pub body: P::Body,
    pub signers: Vec<Signature>,
}

// By hand, so an entry is Clone, PartialEq and Debug without asking the
// same of the policy, which may hold things that aren't.
impl<P: Policy> Clone for Entry<P> {
    fn clone(&self) -> Self {
        Entry { n: self.n, prev: self.prev.clone(), body: self.body.clone(), signers: self.signers.clone() }
    }
}

impl<P: Policy> PartialEq for Entry<P> {
    fn eq(&self, other: &Self) -> bool {
        self.n == other.n && self.prev == other.prev && self.body == other.body && self.signers == other.signers
    }
}

impl<P: Policy> fmt::Debug for Entry<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Entry").field("n", &self.n).field("prev", &self.prev).field("body", &self.body).field("signers", &self.signers).finish()
    }
}

impl<P: Policy> std::ops::Deref for Entry<P> {
    type Target = P::Body;
    fn deref(&self) -> &P::Body {
        &self.body
    }
}

impl<P: Policy> std::ops::DerefMut for Entry<P> {
    fn deref_mut(&mut self) -> &mut P::Body {
        &mut self.body
    }
}

impl<P: Policy> Entry<P> {
    /// An unsigned entry.
    pub fn new(n: u64, prev: impl Into<String>, body: P::Body) -> Entry<P> {
        Entry { n, prev: prev.into(), body, signers: Vec::new() }
    }

    fn unsigned(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("n".into(), Value::from(self.n));
        m.insert("prev".into(), Value::String(self.prev.clone()));
        P::write_body(&self.body, &mut m);
        m
    }

    /// The entry as the chain keeps it: its canonical JSON.
    pub fn to_json(&self) -> Value {
        let f = P::FORMAT;
        let mut m = self.unsigned();
        let signers = self.signers.iter().map(|s| {
            let mut o = Map::new();
            o.insert(f.signer_key.into(), Value::String(s.by.clone()));
            o.insert("sig".into(), Value::String(b64(&s.sig)));
            if let Some(p) = s.presence.as_ref().filter(|_| f.presence) {
                o.insert("presence".into(), p.to_json());
            }
            Value::Object(o)
        });
        m.insert("signers".into(), Value::Array(signers.collect()));
        Value::Object(m)
    }

    /// What each signer signs: the tag, a newline, the canonical JSON
    /// without the signers.
    pub fn signing_bytes(&self) -> Vec<u8> {
        P::FORMAT.tag.bytes(Value::Object(self.unsigned()).to_string().as_bytes())
    }

    /// The SHA-256 of what's signed: the act a presence block approves.
    pub fn digest(&self) -> [u8; 32] {
        sha256(&self.signing_bytes())
    }

    /// What an entry says, apart from who signed it, by its hash, so an
    /// approval signs exactly the entry the person saw.
    pub fn id(&self) -> String {
        b64(&self.digest())
    }

    /// What the next entry's `prev` is.
    pub fn hash(&self) -> String {
        hash_b64(self.to_json().to_string().as_bytes())
    }

    /// Whether two entries say the same thing, their signers aside.
    pub fn same(&self, other: &Entry<P>) -> bool {
        P::same(&self.body, &other.body)
    }

    /// Takes another copy's signatures too: the same entry, approved by
    /// more signers. At most four.
    pub fn merge_signers(&mut self, other: &Entry<P>) {
        for s in &other.signers {
            if self.signers.len() < MAX_SIGNERS && !self.signers.iter().any(|x| x.by == s.by) {
                self.signers.push(s.clone());
            }
        }
    }

    /// Puts `by`'s signature on, in place of any it had.
    pub fn add_signature(&mut self, by: &str, sig: Vec<u8>, presence: Option<PresenceBlock>) {
        self.signers.retain(|s| s.by != by);
        self.signers.push(Signature { by: by.into(), sig, presence });
    }

    /// Has `key` sign the entry as `by`, with the presence block it
    /// brings.
    pub fn sign(&mut self, by: &str, key: &dyn Signer, presence: Option<PresenceBlock>) -> xenonet_id::R<()> {
        let sig = key.sign(P::FORMAT.tag, Value::Object(self.unsigned()).to_string().as_bytes())?;
        self.add_signature(by, sig, presence);
        Ok(())
    }

    /// What `parse` holds an entry to, for one made here: the chain takes
    /// only an entry it can read back from its own file, and that the
    /// others can read from its frame.
    pub fn well_formed(&self) -> R<()> {
        let f = P::FORMAT;
        if !is_b64(&self.prev) {
            return Err(f.bad("prev"));
        }
        P::body_well_formed(&self.body)?;
        if self.signers.is_empty() || self.signers.len() > MAX_SIGNERS {
            return Err(f.bad("signers"));
        }
        for s in &self.signers {
            if !P::is_signer(&s.by) {
                return Err(f.bad("signer"));
            }
            if !P::sig_ok(&s.sig) {
                return Err(f.bad("signature"));
            }
            if s.presence.is_some() && !f.presence {
                return Err(f.bad("presence"));
            }
        }
        Ok(())
    }

    pub fn parse(v: &Value) -> R<Entry<P>> {
        let f = P::FORMAT;
        let n = v.get("n").and_then(Value::as_u64).ok_or_else(|| f.bad("index"))?;
        let prev = v.get("prev").and_then(Value::as_str).filter(|p| is_b64(p)).ok_or_else(|| f.bad("prev"))?;
        let body = P::parse_body(v)?;
        let list = v.get("signers").and_then(Value::as_array).filter(|s| !s.is_empty() && s.len() <= MAX_SIGNERS).ok_or_else(|| f.bad("signers"))?;
        let mut signers = Vec::new();
        for s in list {
            let by = s.get(f.signer_key).and_then(Value::as_str).filter(|h| P::is_signer(h)).ok_or_else(|| f.bad("signer"))?;
            let sig = s.get("sig").and_then(Value::as_str).and_then(unb64).filter(|x| P::sig_ok(x)).ok_or_else(|| f.bad("signature"))?;
            let presence = match s.get("presence") {
                Some(p) if f.presence && !p.is_null() => Some(PresenceBlock::parse(p).ok_or_else(|| f.bad("presence"))?),
                _ => None,
            };
            signers.push(Signature { by: by.into(), sig, presence });
        }
        Ok(Entry { n, prev: prev.into(), body, signers })
    }
}

/// What happened to an entry offered to a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offered {
    Appended,
    /// The chain has it already.
    Duplicate,
}

/// A chain of kind `P`, checked whole from entry 0, and what it adds up
/// to.
#[derive(Debug, Clone)]
pub struct Chain<P: Policy> {
    policy: P,
    entries: Vec<Entry<P>>,
    /// Each entry's hash, so the head and a past head are found without
    /// hashing the chain again.
    hashes: Vec<String>,
    state: Option<P::State>,
}

impl<P: Policy + Default> Default for Chain<P> {
    fn default() -> Self {
        Chain::with_policy(P::default())
    }
}

impl<P: Policy + Default> Chain<P> {
    pub fn new() -> Chain<P> {
        Chain::default()
    }

    /// A chain, checked whole from entry 0.
    pub fn from_entries(entries: Vec<Entry<P>>) -> R<Chain<P>> {
        Chain::from_entries_with(P::default(), entries)
    }

    pub fn from_json(v: &Value) -> R<Chain<P>> {
        Chain::from_json_with(P::default(), v)
    }
}

impl<P: Policy> Chain<P> {
    /// An empty chain that `policy` decides.
    pub fn with_policy(policy: P) -> Chain<P> {
        Chain { policy, entries: Vec::new(), hashes: Vec::new(), state: None }
    }

    pub fn from_entries_with(policy: P, entries: Vec<Entry<P>>) -> R<Chain<P>> {
        let mut c = Chain::with_policy(policy);
        for e in entries {
            if c.offer(e)? == Offered::Duplicate {
                return Err(ChainError::bad(format!("a {} has each entry once", P::FORMAT.noun)));
            }
        }
        Ok(c)
    }

    pub fn from_json_with(policy: P, v: &Value) -> R<Chain<P>> {
        let list = v.as_array().ok_or_else(|| ChainError::bad(format!("a {} is a list of entries", P::FORMAT.noun)))?;
        Chain::from_entries_with(policy, list.iter().map(Entry::parse).collect::<R<Vec<_>>>()?)
    }

    pub fn to_json(&self) -> Value {
        Value::Array(self.entries.iter().map(Entry::to_json).collect())
    }

    pub fn policy(&self) -> &P {
        &self.policy
    }

    pub fn entries(&self) -> &[Entry<P>] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries; the next one's index.
    pub fn len(&self) -> u64 {
        self.entries.len() as u64
    }

    /// What the chain adds up to; the empty state before entry 0.
    pub fn view(&self) -> P::State
    where
        P::State: Default,
    {
        self.state.clone().unwrap_or_default()
    }

    /// What the chain adds up to; None before entry 0.
    pub fn view_ref(&self) -> Option<&P::State> {
        self.state.as_ref()
    }

    /// The hash of the last entry.
    pub fn head(&self) -> Option<String> {
        self.hashes.last().cloned()
    }

    /// The index of the entry whose hash is `head`: where the chain stood
    /// when that was its head.
    pub fn index_of(&self, head: &str) -> Option<u64> {
        self.hashes.iter().position(|h| h == head).map(|i| i as u64)
    }

    /// Takes an entry that extends the head, or drops one the chain has
    /// already; refuses a fork, a gap and anything the rules don't allow.
    pub fn offer(&mut self, e: Entry<P>) -> R<Offered> {
        if let Some(have) = usize::try_from(e.n).ok().and_then(|i| self.entries.get(i)) {
            return if have.same(&e) && have.prev == e.prev { Ok(Offered::Duplicate) } else { Err(P::FORMAT.fork(format!("the {} has another entry there", P::FORMAT.noun))) };
        }
        self.would_take(&e)?;
        self.state = Some(self.policy.apply(self.state.take(), &e));
        self.hashes.push(e.hash());
        self.entries.push(e);
        Ok(Offered::Appended)
    }

    /// Whether `e` would be taken now, without taking it.
    pub fn would_take(&self, e: &Entry<P>) -> R<()> {
        let f = P::FORMAT;
        let state = self.state.as_ref();
        if e.n != self.len() {
            return Err(f.gap(self.len(), e.n));
        }
        if e.prev != self.head().unwrap_or_default() {
            return Err(f.fork(format!("that entry doesn't follow the {}'s head", f.noun)));
        }
        // An entry made here goes to the chain's file and to the other
        // holders, which read it with `parse`: one it couldn't read back
        // would leave the file unreadable and the others a gap they can't
        // fill.
        e.well_formed()?;
        let mut seen: Vec<&str> = Vec::new();
        for s in &e.signers {
            if seen.contains(&s.by.as_str()) {
                return Err(f.refused(format!("an entry is signed by each {} once", f.signer)));
            }
            seen.push(s.by.as_str());
        }
        self.policy.scope(state, e)?;
        let msg = e.signing_bytes();
        for s in &e.signers {
            let key = self.policy.signer_key(state, e, &s.by)?;
            if !key.verify_raw(&msg, &s.sig) {
                return Err(self.policy.bad_signature(state, &s.by));
            }
        }
        self.policy.check(state, e)
    }

    /// The next entry, unsigned: its index and `prev` filled in.
    pub fn next(&self, body: P::Body) -> Entry<P> {
        Entry::new(self.len(), self.head().unwrap_or_default(), body)
    }

    /// The entries from index `from` on.
    pub fn since(&self, from: u64) -> &[Entry<P>] {
        let i = usize::try_from(from).unwrap_or(usize::MAX).min(self.entries.len());
        &self.entries[i..]
    }
}
