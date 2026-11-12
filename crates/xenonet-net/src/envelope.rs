//! The envelope (`xenonet-net/2`, docs/design/network.md, section 3):
//! what carries one feed entry, or one live presence delta,
//! between two machines, directly or through a mailbox or a holder that
//! keeps it for a member that is away.
//!
//! The head is routing only, and nothing signs it:
//!
//! ```text
//! xenonet-net/2
//! <network id>
//! <id: the feed entry's id, or the live body's hash>
//! <kind>
//! <author: its machine signing key's id, xns1_…>
//! <to: the recipient's machine signing key's id, or * for live>
//! <pos: inc:seq, or - for live>
//! <at>
//! <class: live, kept or loose>
//! <sealed: 1 or 0>
//! <base64url SHA-256 of the body bytes as sent>
//! ```
//!
//! **Kept and loose** envelopes carry a feed entry (xenonet-log's `feed`),
//! signed by its author's machine signing key, and are sealed for one
//! recipient: the sealing of `frame` (an ephemeral X25519 key, HKDF-SHA256
//! with the recipient's transport key id and the sealing head as info,
//! ChaCha20-Poly1305 with the sealing head as associated data), with the
//! head's first ten lines as the sealing head. So a mailbox or holder can
//! read the head, and keep, drop or delay the envelope, but can't change a
//! line of the head without the envelope failing to open. The entry inside
//! is the authority: a receiver opens the envelope, reads the entry,
//! checks its signature against the author's machine key in its own copy
//! of the network's chain, and drops the envelope if the head says
//! anything the entry doesn't (network, id, kind, author, pos, at, class).
//! A holder that re-seals an entry for a requester makes a new envelope
//! around the same signed entry, which is why the head carries no
//! signature of its own.
//!
//! **An act's authority.** A message whose act rests on more than its
//! machine's word (the person's answer with their presence, a grant's
//! token, a delegation) carries what it rests on beside the entry, as
//! `network/message/v3`'s `authority`: sealed for the same recipient on
//! its own, with the sealing head and the line `authority` as associated
//! data, so it opens only with the envelope it came in, and a mailbox
//! that keeps the envelope can't read, swap or move it. Dropping it is
//! all a mailbox can do, which leaves the act refused at the receiver.
//! The receiver decides the act on its own facts, as it decides every
//! act (xenonet-authz). A holder serves kept entries, which carry none.
//!
//! **A space's entries** (a group that is a space, `spc_…`: Xenolith's
//! sync/2) go unsealed: a space has no mailboxes, and its entries travel
//! only over authenticated QUIC between members its chain admits. Such an
//! envelope may carry, beside the entry, the text the entry's body names by
//! hash (`attach`), so a holder can drop the text (a forget) and still
//! serve the entry. A network's envelopes are always sealed and never
//! carry one.
//!
//! **Live** envelopes carry a presence delta, unsealed. They're never
//! kept or forwarded: a receiver takes one only from a connection whose
//! peer is the author's transport key, which the connection (QUIC)
//! authenticated, and it is the only thing that does.
//!
//! **Replays.** A copy of an envelope already taken is refused (`Seen`),
//! within a window of recent ids; past it, a feed takes each entry once
//! by its place or id anyway.

use crate::codec::{b64, hash_b64, is_b64, sha256, unb64};
use crate::frame::{open, seal, seal_with};
use crate::keys::{HostKey, PublicKeys};
use crate::roster::View;
use crate::{NetError, R};
use serde_json::{json, Map, Value};
use std::collections::{HashSet, VecDeque};
use xenonet_id::sign::VerifyingKey;
use xenonet_log::feed::{self, FeedEntry, Pos, Verified};
// A machine signing key's id, `xns1_…`.
use xenonet_log::person::is_key_id;

pub const TAG: &str = "xenonet-net/2";
/// The most bytes an envelope's JSON takes: a link/2 record's limit.
pub const MAX_ENVELOPE: usize = 1 << 20;
/// What a live envelope may carry.
pub const LIVE_KINDS: [&str; 1] = ["presence"];
/// How many envelope ids `Seen` keeps by default.
pub const SEEN: usize = 65_536;

/// How an envelope travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Presence: unsealed, never kept or forwarded.
    Live,
    /// A kept feed entry, in the author's gapless sequence.
    Kept,
    /// A loose feed entry, a message or a receipt.
    Loose,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Live => "live",
            Class::Kept => "kept",
            Class::Loose => "loose",
        }
    }

    pub fn parse(s: &str) -> Option<Class> {
        [Class::Live, Class::Kept, Class::Loose].into_iter().find(|c| c.as_str() == s)
    }

    fn of(c: feed::Class) -> Class {
        match c {
            feed::Class::Kept => Class::Kept,
            feed::Class::Loose => Class::Loose,
        }
    }
}

/// Whom an envelope is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum To {
    /// Every member it's sent to: live only.
    All,
    /// One machine, by its machine signing key's id.
    Machine(String),
}

impl To {
    pub fn as_str(&self) -> &str {
        match self {
            To::All => "*",
            To::Machine(id) => id,
        }
    }

    fn parse(s: &str) -> Option<To> {
        match s {
            "*" => Some(To::All),
            id if is_key_id(id) => Some(To::Machine(id.into())),
            _ => None,
        }
    }
}

/// One envelope: its head's fields and its body as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub network: String,
    pub id: String,
    pub kind: String,
    pub author: String,
    pub to: To,
    /// The entry's place in its author's feed; None for live.
    pub pos: Option<Pos>,
    /// When it was made, in ms, as the entry says.
    pub at: u64,
    pub class: Class,
    pub sealed: bool,
    /// The sealed entry, a space's unsealed entry, or a live envelope's
    /// JSON body.
    pub body: Vec<u8>,
    /// The text a space's unsealed entry names by hash, carried beside it;
    /// None on everything else.
    pub attach: Option<Vec<u8>>,
    /// What a sealed message's act rests on, sealed for the same
    /// recipient (`authority_head`); None when it rests on nothing more.
    pub authority: Option<Vec<u8>>,
}

/// This machine as a recipient: its machine signing key's id and its
/// transport key, which envelopes are sealed to.
pub struct Me<'a> {
    pub machine: &'a str,
    pub key: &'a HostKey,
}

fn bad(what: &str) -> NetError {
    NetError::bad(format!("an envelope's {what} isn't right"))
}

/// The envelope disagrees with the entry it carries, or with the chain.
fn mismatch(what: &str) -> NetError {
    NetError::new(400, "envelope-mismatch", format!("the envelope's {what} isn't the entry's"))
}

fn is_id(id: &str) -> bool {
    id.len() == 43 && is_b64(id)
}

fn pos_line(pos: Option<Pos>) -> String {
    pos.map_or_else(|| "-".to_string(), |p| format!("{}:{}", p.inc, p.seq))
}

impl Envelope {
    /// The head's first ten lines: what sealing binds.
    pub fn sealing_head(&self) -> String {
        format!(
            "{TAG}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            self.network,
            self.id,
            self.kind,
            self.author,
            self.to.as_str(),
            pos_line(self.pos),
            self.at,
            self.class.as_str(),
            u8::from(self.sealed)
        )
    }

    /// What an act's authority is sealed under: the sealing head, then
    /// `authority`, so it opens only beside this envelope's entry.
    pub fn authority_head(&self) -> String {
        format!("{}\nauthority", self.sealing_head())
    }

    /// The whole head, with the body's hash: what a mailbox's receipt or
    /// a holder's index names it by.
    pub fn head(&self) -> String {
        format!("{}\n{}", self.sealing_head(), hash_b64(&self.body))
    }

    /// A feed entry, signed by its author, sealed for `to` (the
    /// recipient's machine signing key id) with its transport keys
    /// `to_keys`. `eph` fixes the ephemeral key, for known-answer tests.
    pub fn seal_entry(entry: &FeedEntry, to: &str, to_keys: &PublicKeys, eph: Option<[u8; 32]>) -> R<Envelope> {
        Envelope::seal_act(entry, to, to_keys, eph, None)
    }

    /// `seal_entry`, with what the act rests on (`authority`, a JSON
    /// object) sealed beside it for the same recipient. An empty object
    /// or None is no authority.
    pub fn seal_act(entry: &FeedEntry, to: &str, to_keys: &PublicKeys, eph: Option<[u8; 32]>, authority: Option<&Value>) -> R<Envelope> {
        let authority = authority.filter(|a| a.as_object().is_some_and(|o| !o.is_empty()));
        if authority.is_some() && entry.class() != feed::Class::Loose {
            return Err(bad("authority"));
        }
        entry.well_formed().map_err(|e| NetError::bad(e.message))?;
        let to = To::parse(to).filter(|t| *t != To::All).ok_or_else(|| bad("recipient"))?;
        let mut env = Envelope {
            network: entry.group.clone(),
            id: entry.id(),
            kind: entry.kind.as_str().into(),
            author: entry.author.clone(),
            to,
            pos: Some(entry.pos()),
            at: entry.at,
            class: Class::of(entry.class()),
            sealed: true,
            body: Vec::new(),
            attach: None,
            authority: None,
        };
        let head = env.sealing_head();
        let plain = entry.encode();
        env.body = match eph {
            Some(e) => seal_with(e, to_keys, &head, &plain)?,
            None => seal(to_keys, &head, &plain)?,
        };
        if let Some(a) = authority {
            env.authority = Some(seal(to_keys, &env.authority_head(), a.to_string().as_bytes())?);
        }
        env.well_formed()?;
        Ok(env)
    }

    /// A space's feed entry (its group is a `spc_`), signed by its author,
    /// unsealed, for `to` (the recipient's key id), with the text its body
    /// names beside it, if any.
    pub fn plain_entry(entry: &FeedEntry, to: &str, attach: Option<&[u8]>) -> R<Envelope> {
        entry.well_formed().map_err(|e| NetError::bad(e.message))?;
        if feed::Group::of(&entry.group) != Some(feed::Group::Space) {
            return Err(bad("seal"));
        }
        let to = To::parse(to).filter(|t| *t != To::All).ok_or_else(|| bad("recipient"))?;
        let env = Envelope {
            network: entry.group.clone(),
            id: entry.id(),
            kind: entry.kind.as_str().into(),
            author: entry.author.clone(),
            to,
            pos: Some(entry.pos()),
            at: entry.at,
            class: Class::of(entry.class()),
            sealed: false,
            body: entry.encode(),
            attach: attach.map(<[u8]>::to_vec),
            authority: None,
        };
        env.well_formed()?;
        Ok(env)
    }

    /// A live envelope from `author` (its machine signing key id): `body`,
    /// unsealed, for every member it's sent to.
    pub fn live(network: &str, author: &str, kind: &str, at: u64, body: &Value) -> R<Envelope> {
        let bytes = body.to_string().into_bytes();
        let env = Envelope {
            network: network.into(),
            id: hash_b64(&bytes),
            kind: kind.into(),
            author: author.into(),
            to: To::All,
            pos: None,
            at,
            class: Class::Live,
            sealed: false,
            body: bytes,
            attach: None,
            authority: None,
        };
        env.well_formed()?;
        Ok(env)
    }

    /// What `parse` holds an envelope to: the head's fields each in its
    /// shape, and the class, seal, recipient and place in agreement.
    pub fn well_formed(&self) -> R<()> {
        // A network's, or a space's: which kind a protocol takes is its own
        // to check (`link2::Protocol`).
        if feed::Group::of(&self.network).is_none() {
            return Err(bad("network"));
        }
        if !is_id(&self.id) {
            return Err(bad("id"));
        }
        if !is_key_id(&self.author) {
            return Err(bad("author"));
        }
        match self.class {
            Class::Live => {
                if !LIVE_KINDS.contains(&self.kind.as_str()) {
                    return Err(bad("kind"));
                }
                if self.sealed || self.pos.is_some() || self.attach.is_some() || self.authority.is_some() {
                    return Err(bad("class"));
                }
                // Not forwarded, so the id is the body's, checked here.
                if self.id != hash_b64(&self.body) {
                    return Err(bad("id"));
                }
            }
            Class::Kept | Class::Loose => {
                let kind = feed::Kind::parse(&self.kind).ok_or_else(|| bad("kind"))?;
                if Class::of(kind.class()) != self.class {
                    return Err(bad("class"));
                }
                // Sealed, or a space's (which go unsealed); for one machine.
                let space = feed::Group::of(&self.network) == Some(feed::Group::Space);
                if (!self.sealed && !space) || self.to == To::All {
                    return Err(bad("seal"));
                }
                if self.sealed && self.attach.is_some() {
                    return Err(bad("attach"));
                }
                // Only a sealed message carries an act's authority.
                if self.authority.is_some() && (!self.sealed || self.class != Class::Loose) {
                    return Err(bad("authority"));
                }
                if self.pos.is_none_or(|p| p.inc == 0 || p.seq == 0) {
                    return Err(bad("pos"));
                }
            }
        }
        if self.to_json().to_string().len() > MAX_ENVELOPE {
            return Err(bad("size"));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("v".into(), json!(2));
        m.insert("network".into(), json!(self.network));
        m.insert("id".into(), json!(self.id));
        m.insert("kind".into(), json!(self.kind));
        m.insert("author".into(), json!(self.author));
        m.insert("to".into(), json!(self.to.as_str()));
        m.insert("pos".into(), self.pos.map_or(Value::Null, |p| json!({ "inc": p.inc, "seq": p.seq })));
        m.insert("at".into(), json!(self.at));
        m.insert("class".into(), json!(self.class.as_str()));
        m.insert("sealed".into(), json!(self.sealed));
        m.insert("body".into(), json!(b64(&self.body)));
        if let Some(a) = &self.attach {
            m.insert("attach".into(), json!(b64(a)));
        }
        if let Some(a) = &self.authority {
            m.insert("authority".into(), json!(b64(a)));
        }
        Value::Object(m)
    }

    pub fn encode(&self) -> Vec<u8> {
        self.to_json().to_string().into_bytes()
    }

    /// An envelope from its JSON, each field checked for its shape. What
    /// it carries isn't checked here: that needs the recipient's key and
    /// the network's chain.
    pub fn parse(v: &Value) -> R<Envelope> {
        if v.get("v") != Some(&json!(2)) {
            return Err(bad("version"));
        }
        let text = |k: &str| v.get(k).and_then(Value::as_str).ok_or_else(|| bad(k));
        let pos = match v.get("pos") {
            None | Some(Value::Null) => None,
            Some(p) => {
                let n = |k: &str| p.get(k).and_then(Value::as_u64).ok_or_else(|| bad("pos"));
                Some(Pos { inc: n("inc")?, seq: n("seq")? })
            }
        };
        let env = Envelope {
            network: text("network")?.into(),
            id: text("id")?.into(),
            kind: text("kind")?.into(),
            author: text("author")?.into(),
            to: To::parse(text("to")?).ok_or_else(|| bad("recipient"))?,
            pos,
            at: v.get("at").and_then(Value::as_u64).ok_or_else(|| bad("time"))?,
            class: Class::parse(text("class")?).ok_or_else(|| bad("class"))?,
            sealed: v.get("sealed").and_then(Value::as_bool).ok_or_else(|| bad("sealed"))?,
            body: unb64(text("body")?).ok_or_else(|| bad("body"))?,
            attach: match v.get("attach") {
                None => None,
                Some(a) => Some(a.as_str().and_then(unb64).ok_or_else(|| bad("attach"))?),
            },
            authority: match v.get("authority") {
                None => None,
                Some(a) => Some(a.as_str().and_then(unb64).ok_or_else(|| bad("authority"))?),
            },
        };
        env.well_formed()?;
        Ok(env)
    }

    /// An envelope from the bytes it came as.
    pub fn decode(bytes: &[u8]) -> R<Envelope> {
        if bytes.len() > MAX_ENVELOPE {
            return Err(bad("size"));
        }
        let v: Value = serde_json::from_slice(bytes).map_err(|_| NetError::bad("an envelope isn't JSON"))?;
        Envelope::parse(&v)
    }

    /// The feed entry a kept or loose envelope carries, for `me`, once it
    /// opens with `me`'s key, says what its head says, and is signed by
    /// its author's machine signing key as `view` (this machine's copy of
    /// the network's chain) has it.
    pub fn open_entry(&self, me: &Me<'_>, view: &View) -> R<Verified> {
        if self.class == Class::Live {
            return Err(NetError::bad("a live envelope carries no feed entry"));
        }
        if self.network != view.network {
            return Err(NetError::new(403, "not-member", "that envelope is for another network"));
        }
        if self.to != To::Machine(me.machine.into()) {
            return Err(NetError::new(403, "wrong-recipient", "that envelope is for another machine"));
        }
        let author = view.by_machine(&self.author).ok_or_else(|| NetError::new(403, "not-member", "that envelope's author isn't in the network"))?;
        self.open_entry_by(me, &author.machine)
    }

    /// `open_entry` for a group whose chain isn't a network's (another
    /// program's: a space): `author` is the key the caller's own copy of
    /// the group's chain lists for the envelope's author, which the caller
    /// found a member of the group the envelope names.
    pub fn open_entry_by(&self, me: &Me<'_>, author: &VerifyingKey) -> R<Verified> {
        if self.class == Class::Live {
            return Err(NetError::bad("a live envelope carries no feed entry"));
        }
        if self.to != To::Machine(me.machine.into()) {
            return Err(NetError::new(403, "wrong-recipient", "that envelope is for another machine"));
        }
        let plain = open(me.key, &self.sealing_head(), &self.body).map_err(|_| NetError::new(400, "unsealed", "that envelope wasn't sealed for this machine, or was changed"))?;
        let entry = FeedEntry::decode(&plain).map_err(|e| NetError::bad(e.message))?;
        self.matches(&entry)?;
        entry.verify(author).map_err(|e| NetError::new(403, "refused", e.message))
    }

    /// What the act this envelope carries rests on, for `me`: the
    /// authority sealed beside its entry, or an empty object when it
    /// carries none. Call it once the entry opened: it is read only with
    /// the entry it came with, and decided at this machine as every act is.
    pub fn open_authority(&self, me: &Me<'_>) -> R<Value> {
        let Some(sealed) = &self.authority else { return Ok(json!({})) };
        if self.to != To::Machine(me.machine.into()) {
            return Err(NetError::new(403, "wrong-recipient", "that envelope is for another machine"));
        }
        let plain =
            open(me.key, &self.authority_head(), sealed).map_err(|_| NetError::new(400, "unsealed", "that act's authority wasn't sealed for this machine, or was changed"))?;
        let v: Value = serde_json::from_slice(&plain).map_err(|_| bad("authority"))?;
        if !v.is_object() {
            return Err(bad("authority"));
        }
        Ok(v)
    }

    /// The entry a space's unsealed envelope carries, for `me` (the
    /// recipient's key id), once it says what its head says and is signed
    /// by `author`, the key the caller's own copy of the space's chain lists
    /// for the envelope's author; and the text beside it, if any, which the
    /// caller checks against the hash the entry's body names.
    pub fn open_plain_by(&self, me: &str, author: &VerifyingKey) -> R<(Verified, Option<Vec<u8>>)> {
        if self.class == Class::Live || self.sealed {
            return Err(NetError::bad("only a space's unsealed envelope carries a plain entry"));
        }
        if self.to != To::Machine(me.into()) {
            return Err(NetError::new(403, "wrong-recipient", "that envelope is for another machine"));
        }
        let entry = FeedEntry::decode(&self.body).map_err(|e| NetError::bad(e.message))?;
        self.matches(&entry)?;
        let v = entry.verify(author).map_err(|e| NetError::new(403, "refused", e.message))?;
        Ok((v, self.attach.clone()))
    }

    // What the head says, against the entry it carries.
    fn matches(&self, entry: &FeedEntry) -> R<()> {
        let checks: [(&str, bool); 7] = [
            ("network", entry.group == self.network),
            ("id", entry.id() == self.id),
            ("kind", entry.kind.as_str() == self.kind),
            ("author", entry.author == self.author),
            ("pos", Some(entry.pos()) == self.pos),
            ("time", entry.at == self.at),
            ("class", Class::of(entry.class()) == self.class),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((what, _)) => Err(mismatch(what)),
            None => Ok(()),
        }
    }

    /// A live envelope's body, taken only from a connection whose peer
    /// `from` (the endpoint id the connection authenticated) is the
    /// author's transport key now, as `view` has it.
    pub fn open_live(&self, from: &[u8; 32], view: &View) -> R<Value> {
        if self.class != Class::Live {
            return Err(NetError::bad("only a live envelope is read from its connection"));
        }
        if self.network != view.network {
            return Err(NetError::new(403, "not-member", "that envelope is for another network"));
        }
        let author = view.by_machine(&self.author).ok_or_else(|| NetError::new(403, "not-member", "that envelope's author isn't in the network"))?;
        self.open_live_by(from, &author.keys.sign)
    }

    /// `open_live` for a group whose chain isn't a network's: `author` is
    /// the endpoint id the caller's own copy of the group's chain lists for
    /// the envelope's author.
    pub fn open_live_by(&self, from: &[u8; 32], author: &[u8; 32]) -> R<Value> {
        if self.class != Class::Live {
            return Err(NetError::bad("only a live envelope is read from its connection"));
        }
        if author != from {
            return Err(NetError::new(403, "not-forwarded", "a live envelope comes from its author's own connection"));
        }
        let body: Value = serde_json::from_slice(&self.body).map_err(|_| bad("body"))?;
        if !body.is_object() {
            return Err(bad("body"));
        }
        Ok(body)
    }
}

/// The envelopes taken lately, by network, author and id, so a copy is
/// refused: at most `cap`, the oldest forgotten first.
#[derive(Debug)]
pub struct Seen {
    cap: usize,
    order: VecDeque<[u8; 32]>,
    ids: HashSet<[u8; 32]>,
}

impl Default for Seen {
    fn default() -> Self {
        Seen::new(SEEN)
    }
}

impl Seen {
    pub fn new(cap: usize) -> Seen {
        Seen { cap: cap.max(1), order: VecDeque::new(), ids: HashSet::new() }
    }

    fn key(env: &Envelope) -> [u8; 32] {
        sha256(format!("{}\n{}\n{}\n{}", env.network, env.author, env.class.as_str(), env.id).as_bytes())
    }

    /// Takes `env`'s id, or refuses a copy of one taken already. Call it
    /// once the envelope opened, so what doesn't open fills nothing.
    pub fn take(&mut self, env: &Envelope) -> R<()> {
        let k = Seen::key(env);
        if !self.ids.insert(k) {
            return Err(NetError::new(409, "replayed", "that envelope was taken already"));
        }
        self.order.push_back(k);
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.ids.remove(&old);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
