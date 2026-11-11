//! Mailboxes (docs/design/network.md, the transport): member machines a
//! network lists that keep sealed envelopes for members that are away,
//! and hand them over when those members connect.
//!
//! **What a mailbox keeps.** An envelope its author hands it, on the
//! author's own connection, only when by the mailbox's chain the author
//! and the recipient are members of the envelope's network and that
//! network lists this machine as a mailbox (`admit`). It can't open one:
//! each is sealed for its recipient. Each (network, recipient) has its own
//! queue of at most 10,000 envelopes or 50 MB, kept a week at most
//! (`Caps`); past that the oldest go. A member of another network fills
//! nothing here, and a member of this one fills only its recipients'
//! queues. What goes is a gap the recipient sees: a kept entry missing
//! before the ones it is handed, which it fetches from a holder or the
//! author (`holder`). A loose entry older than retention by its time is
//! refused, as the recipient's feed would refuse it.
//!
//! **Held receipts.** For what it keeps, a mailbox sends the author a
//! receipt (`Held`): the network, the mailbox, the author, and each entry
//! by its id with its recipient and when it was kept, batched, signed by
//! the mailbox's machine signing key. The author takes one only from a
//! connection whose peer is a mailbox its own chain lists, and shows a
//! kept entry as "held, not delivered" (`Handed`) until the recipient acks
//! it (a message's delivery is its receipt's, which the engine keeps).
//! A mailbox that drops or delays envelopes can't hide it: what it took
//! and didn't deliver stays held, not delivered.
//!
//! **Hand-off.** When the recipient connects, the mailbox sends what it
//! keeps for it, oldest first, and drops each once the recipient says it
//! took it (`Control::Took`). A mailbox that dies mid-transfer sends the
//! rest, and again what wasn't said to be taken, when it's back; the
//! recipient's feed takes each entry once.
//!
//! **Not through a mailbox.** An act whose authority carries a presence
//! block is live only: the receiver refuses a presence block more than a
//! minute old, so one that waited at a mailbox would be refused on
//! delivery. It is refused at hand-off, "recipient offline" (`route`).

use crate::dial::PeerId;
use crate::envelope::{Class, Envelope, To};
use crate::persist::Persist;
use crate::roster::View;
use crate::{NetError, R};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use xenonet_id::codec::{b64, unb64};
use xenonet_id::sign::{Purpose, Signer};
use xenonet_log::feed::{Pos, RETENTION_MS};

/// What one (network, recipient) queue holds at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub entries: u64,
    pub bytes: u64,
    pub keep_ms: u64,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps { entries: 10_000, bytes: 50 << 20, keep_ms: RETENTION_MS }
    }
}

/// The most entries one receipt names.
pub const HELD_BATCH: usize = 256;
/// The most envelopes a hand-off sends at once; the rest after the
/// recipient says it took them.
pub const HAND: usize = 256;

/// An envelope a mailbox keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mail {
    pub network: String,
    /// The recipient's machine signing key id.
    pub recipient: String,
    pub author: String,
    /// The entry's id, as the head names it.
    pub id: String,
    /// When the mailbox kept it, ms.
    pub kept: u64,
    /// The envelope as it came.
    pub envelope: Vec<u8>,
}

impl Mail {
    pub fn size(&self) -> u64 {
        self.envelope.len() as u64
    }
}

fn refused(code: &'static str, why: impl Into<String>) -> NetError {
    NetError::new(403, code, why)
}

/// Whether this machine (`me`, its host id), a mailbox by `view`, keeps
/// `env`, which the peer `from` handed it, at `now`.
pub fn admit(view: &View, me: &str, from: PeerId, env: &Envelope, now: u64) -> R<()> {
    if view.network != env.network {
        return Err(refused("not-member", "that envelope is for another network"));
    }
    if !view.is_mailbox(me) {
        return Err(refused("not-a-mailbox", format!("{} doesn't list this machine as a mailbox", view.network)));
    }
    let author = view.by_machine(&env.author).ok_or_else(|| refused("not-member", "that envelope's author isn't in the network"))?;
    if author.keys.sign != from.0 {
        return Err(refused("not-author", "a mailbox keeps what an author hands it, on its own connection"));
    }
    let To::Machine(to) = &env.to else { return Err(NetError::bad("a live envelope isn't kept")) };
    let recipient = view.by_machine(to).ok_or_else(|| refused("not-member", "that envelope's recipient isn't in the network"))?;
    if recipient.id == me {
        return Err(NetError::bad("that envelope is for this machine"));
    }
    match env.class {
        Class::Live => Err(NetError::bad("a live envelope isn't kept")),
        Class::Loose if env.at.saturating_add(RETENTION_MS) < now => Err(NetError::new(410, "expired", "that message is older than the network keeps messages")),
        _ => Ok(()),
    }
}

/// The mail an admitted envelope makes.
pub fn mail(env: &Envelope, now: u64) -> Mail {
    Mail { network: env.network.clone(), recipient: env.to.as_str().to_string(), author: env.author.clone(), id: env.id.clone(), kept: now, envelope: env.encode() }
}

/// How many of a queue's oldest go to stay within `caps`, given the sizes
/// of what it holds, oldest first, with one of `new` bytes added at the
/// end.
pub fn evict(held: &[u64], new: u64, caps: &Caps) -> usize {
    let mut count = held.len() as u64 + 1;
    let mut bytes = held.iter().sum::<u64>() + new;
    let mut n = 0;
    while (count > caps.entries || bytes > caps.bytes) && n < held.len() {
        count -= 1;
        bytes -= held[n];
        n += 1;
    }
    n
}

/// Keeps `env`, which `from` handed this mailbox (`me`, its host id), once
/// `admit` lets it, letting go what its queue's caps say and what is older
/// than they keep: the receipt's item for it. A copy of one kept already
/// is kept once, and receipted again.
pub fn take(persist: &dyn Persist, view: &View, me: &str, from: PeerId, env: &Envelope, now: u64, caps: &Caps) -> R<HeldItem> {
    admit(view, me, from, env, now)?;
    let m = mail(env, now);
    persist.prune_mail(now.saturating_sub(caps.keep_ms))?;
    let sizes = persist.mail_sizes(&m.network, &m.recipient)?;
    persist.put_mail(&m, evict(&sizes, m.size(), caps))?;
    Ok(HeldItem { id: m.id, to: m.recipient, at: now })
}

/// What a mailbox hands over, and what's on the way.
#[derive(Debug)]
pub struct Handing {
    /// The most envelopes sent before the recipient says it took them.
    pub limit: usize,
    inflight: BTreeMap<(String, String), BTreeSet<String>>,
}

impl Default for Handing {
    fn default() -> Handing {
        Handing { limit: HAND, inflight: BTreeMap::new() }
    }
}

impl Handing {
    /// `recipient` connected, or another envelope came for it: what to send
    /// it now, oldest first. A connection that's new sends from the start
    /// of the queue again, since what was on the way may not have come.
    pub fn start(&mut self, persist: &dyn Persist, network: &str, recipient: &str, fresh: bool) -> R<Vec<Envelope>> {
        let key = (network.to_string(), recipient.to_string());
        if !fresh && self.inflight.get(&key).is_some_and(|f| !f.is_empty()) {
            return Ok(Vec::new());
        }
        let mail = persist.mail_for(network, recipient, self.limit)?;
        self.inflight.insert(key, mail.iter().map(|m| m.id.clone()).collect());
        Ok(mail.iter().filter_map(|m| Envelope::decode(&m.envelope).ok()).collect())
    }

    /// `recipient` took `ids`: they go, and once nothing it was sent is on
    /// the way, the next.
    pub fn took(&mut self, persist: &dyn Persist, network: &str, recipient: &str, ids: &[String]) -> R<Vec<Envelope>> {
        persist.mail_took(network, recipient, ids)?;
        let key = (network.to_string(), recipient.to_string());
        let Some(f) = self.inflight.get_mut(&key) else { return Ok(Vec::new()) };
        for id in ids {
            f.remove(id);
        }
        if !f.is_empty() {
            return Ok(Vec::new());
        }
        self.start(persist, network, recipient, true)
    }
}

/// A machine's side of mailboxes: the receipts it owes as a mailbox, what
/// it hands over, and what it handed mailboxes as an author.
#[derive(Debug, Default)]
pub struct Post {
    pub batcher: Batcher,
    pub handing: Handing,
    pub handed: Handed,
}

/// Whether an act's authority carries a presence block.
pub fn carries_presence(authority: Option<&Value>) -> bool {
    authority.is_some_and(|a| a.get("presence").is_some_and(|p| !p.is_null()))
}

/// The mailboxes an entry for `recipient` (a host id), which isn't
/// connected, is handed to: the ones `view` lists, but this machine and
/// the recipient, that `connected` says are. An act carrying a presence
/// block goes to none.
pub fn route(view: &View, me: &str, recipient: &str, connected: impl Fn(PeerId) -> bool, authority: Option<&Value>) -> Result<Vec<PeerId>, NetError> {
    if carries_presence(authority) {
        return Err(NetError::new(503, "recipient-offline", format!("{recipient} is offline: an act with your presence goes only to a machine that's connected")));
    }
    Ok(view.mailboxes.iter().filter(|m| *m != me && *m != recipient).filter_map(|m| view.member(m)).map(|m| PeerId(m.keys.sign)).filter(|p| connected(*p)).collect())
}

// ---------- Held receipts ----------

/// One entry a mailbox keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldItem {
    pub id: String,
    /// The recipient's machine signing key id.
    pub to: String,
    pub at: u64,
}

/// A mailbox's receipt to one author for what it keeps of the author's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub network: String,
    /// The mailbox's host id.
    pub mailbox: String,
    pub author: String,
    pub items: Vec<HeldItem>,
    pub at: u64,
    pub sig: Vec<u8>,
}

fn bad(what: &str) -> NetError {
    NetError::bad(format!("a Held receipt's {what} isn't right"))
}

impl Held {
    fn unsigned(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("network".into(), json!(self.network));
        m.insert("mailbox".into(), json!(self.mailbox));
        m.insert("author".into(), json!(self.author));
        m.insert("items".into(), Value::Array(self.items.iter().map(|i| json!({ "id": i.id, "to": i.to, "at": i.at })).collect()));
        m.insert("at".into(), json!(self.at));
        m
    }

    /// Signed by the mailbox's machine signing key.
    pub fn sign(&mut self, key: &dyn Signer) -> R<()> {
        self.sig = key.sign(Purpose::HELD, Value::Object(self.unsigned()).to_string().as_bytes()).map_err(|e| NetError::internal(e.message))?;
        Ok(())
    }

    pub fn to_json(&self) -> Value {
        let mut m = self.unsigned();
        m.insert("sig".into(), json!(b64(&self.sig)));
        Value::Object(m)
    }

    pub fn parse(v: &Value) -> R<Held> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad(k));
        let items = v.get("items").and_then(Value::as_array).filter(|i| !i.is_empty() && i.len() <= HELD_BATCH).ok_or_else(|| bad("items"))?;
        let item = |i: &Value| -> R<HeldItem> {
            let t = |k: &str| i.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(|| bad(k));
            Ok(HeldItem { id: t("id")?, to: t("to")?, at: i.get("at").and_then(Value::as_u64).ok_or_else(|| bad("at"))? })
        };
        Ok(Held {
            network: s("network")?,
            mailbox: s("mailbox")?,
            author: s("author")?,
            items: items.iter().map(item).collect::<R<_>>()?,
            at: v.get("at").and_then(Value::as_u64).ok_or_else(|| bad("at"))?,
            sig: unb64(&s("sig")?).ok_or_else(|| bad("signature"))?,
        })
    }

    /// Whether the author `me` (its machine signing key id) takes this
    /// receipt, which came on `from`'s connection: by its own chain `view`,
    /// the mailbox is one the network lists, the connection is the
    /// mailbox's, the receipt is for `me`, and the mailbox's machine
    /// signing key signed it.
    pub fn verify(&self, view: &View, from: PeerId, me: &str) -> R<()> {
        if self.network != view.network {
            return Err(refused("not-member", "that receipt is for another network"));
        }
        if !view.is_mailbox(&self.mailbox) {
            return Err(refused("not-a-mailbox", format!("{} isn't a mailbox of the network", self.mailbox)));
        }
        let m = view.member(&self.mailbox).ok_or_else(|| refused("not-member", format!("{} isn't in the network", self.mailbox)))?;
        if m.keys.sign != from.0 {
            return Err(refused("not-forwarded", "a receipt comes from its mailbox's own connection"));
        }
        if self.author != me {
            return Err(refused("wrong-recipient", "that receipt is for another author"));
        }
        if !m.machine.verify_raw(&Purpose::HELD.bytes(Value::Object(self.unsigned()).to_string().as_bytes()), &self.sig) {
            return Err(refused("refused", format!("{}'s signature on a receipt doesn't check", self.mailbox)));
        }
        Ok(())
    }
}

/// The receipts a mailbox owes, by (network, author), until they're sent.
#[derive(Debug, Default)]
pub struct Batcher {
    pending: BTreeMap<(String, String), Vec<HeldItem>>,
}

impl Batcher {
    /// One more entry kept: true when it starts a batch, so the caller
    /// sends the batch soon.
    pub fn add(&mut self, network: &str, author: &str, item: HeldItem) -> bool {
        let first = self.pending.is_empty();
        self.pending.entry((network.to_string(), author.to_string())).or_default().push(item);
        first
    }

    /// The receipts to sign and send now, each at most `HELD_BATCH` items.
    pub fn take(&mut self, mailbox: &str, at: u64) -> Vec<Held> {
        let mut out = Vec::new();
        for ((network, author), items) in std::mem::take(&mut self.pending) {
            for chunk in items.chunks(HELD_BATCH) {
                out.push(Held { network: network.clone(), mailbox: mailbox.to_string(), author: author.clone(), items: chunk.to_vec(), at, sig: Vec::new() });
            }
        }
        out
    }
}

// ---------- the author's side ----------

/// One kept entry this machine handed a mailbox for an away member. A
/// message's delivery is its receipt's, which the engine keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Handoff {
    pos: Pos,
    /// The mailboxes whose receipts name it.
    held_by: BTreeSet<String>,
}

/// What this machine handed mailboxes and hasn't seen delivered, by
/// network and recipient (machine signing key id).
#[derive(Debug, Default)]
pub struct Handed {
    by: BTreeMap<(String, String), BTreeMap<String, Handoff>>,
}

/// How this machine's entries for one member stand at its mailboxes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Standing {
    /// Held, not delivered: a mailbox's receipt names them.
    pub held: u64,
    /// Handed, with no receipt yet.
    pub unreceipted: u64,
}

impl Handed {
    /// A kept entry handed to mailboxes for `recipient`.
    pub fn handed(&mut self, network: &str, recipient: &str, id: &str, pos: Pos) {
        self.by.entry((network.to_string(), recipient.to_string())).or_default().entry(id.to_string()).or_insert(Handoff { pos, held_by: BTreeSet::new() });
    }

    /// A receipt that verified: the entries it names are held.
    pub fn held(&mut self, h: &Held) {
        for i in &h.items {
            if let Some(e) = self.by.get_mut(&(h.network.clone(), i.to.clone())).and_then(|q| q.get_mut(&i.id)) {
                e.held_by.insert(h.mailbox.clone());
            }
        }
    }

    /// `recipient` holds this machine's kept entries up to `at`: those are
    /// delivered.
    pub fn acked(&mut self, network: &str, recipient: &str, at: Pos) {
        if let Some(q) = self.by.get_mut(&(network.to_string(), recipient.to_string())) {
            q.retain(|_, e| e.pos > at);
        }
    }

    pub fn standing(&self, network: &str, recipient: &str) -> Standing {
        let Some(q) = self.by.get(&(network.to_string(), recipient.to_string())) else { return Standing::default() };
        let held = q.values().filter(|e| !e.held_by.is_empty()).count() as u64;
        Standing { held, unreceipted: q.len() as u64 - held }
    }

    /// The ids still held, not delivered, for `recipient`.
    pub fn held_ids(&self, network: &str, recipient: &str) -> Vec<String> {
        self.by.get(&(network.to_string(), recipient.to_string())).map(|q| q.iter().filter(|(_, e)| !e.held_by.is_empty()).map(|(id, _)| id.clone()).collect()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::HostKey;
    use crate::roster::{Member, Settings};
    use xenonet_id::sign::{KeyHome, SeedSigner};
    use xenonet_log::feed::{Kind, Writer};

    const N1: &str = "net_00000000000000000000000000000001";
    const N2: &str = "net_00000000000000000000000000000002";
    const T: u64 = 1_792_512_000_000;

    fn transport(n: u8) -> HostKey {
        HostKey::from_secrets(&[n; 32], [n + 1; 32]).unwrap()
    }

    fn signer(n: u8) -> SeedSigner {
        SeedSigner::from_seed(&[n + 2; 32], KeyHome::File).unwrap()
    }

    fn host(n: u8) -> String {
        format!("h{n}")
    }

    fn peer(n: u8) -> PeerId {
        PeerId(transport(n).public().sign)
    }

    fn member(n: u8) -> Member {
        Member { id: host(n), keys: *transport(n).public(), window: false, device: None, machine: signer(n).public(), settings: Settings::default(), settings_by: None, since: 1 }
    }

    fn view(network: &str, members: &[u8], mailboxes: &[u8]) -> View {
        View {
            network: network.into(),
            founder: host(members[0]),
            members: members.iter().map(|n| (host(*n), member(*n))).collect(),
            mailboxes: mailboxes.iter().map(|n| host(*n)).collect(),
            ..View::default()
        }
    }

    fn envelope(network: &str, from: u8, to: u8, kind: Kind, at: u64) -> Envelope {
        let mut w = Writer::start(signer(from).public().id(), network, T);
        let e = w.write(&signer(from), kind, at, json!({ "to": host(to), "text": "hi" })).unwrap();
        Envelope::seal_entry(&e, &signer(to).public().id(), transport(to).public(), None).unwrap()
    }

    #[test]
    fn a_mailbox_keeps_only_what_an_author_of_its_network_hands_it_for_a_member() {
        let v = view(N1, &[1, 2, 3], &[3]);
        let env = envelope(N1, 1, 2, Kind::Claim, T);
        assert!(admit(&v, "h3", peer(1), &env, T).is_ok());
        assert_eq!(admit(&view(N1, &[1, 2, 3], &[]), "h3", peer(1), &env, T).unwrap_err().code, "not-a-mailbox", "not listed");
        assert_eq!(admit(&v, "h3", peer(2), &env, T).unwrap_err().code, "not-author", "handed by another member");
        assert_eq!(admit(&view(N1, &[1, 3], &[3]), "h3", peer(1), &env, T).unwrap_err().code, "not-member", "the recipient isn't in it");
        assert_eq!(admit(&view(N1, &[2, 3], &[3]), "h3", peer(1), &env, T).unwrap_err().code, "not-member", "the author isn't in it");
        assert_eq!(admit(&view(N2, &[1, 2, 3], &[3]), "h3", peer(1), &env, T).unwrap_err().code, "not-member", "another network's");
        let old = envelope(N1, 1, 2, Kind::Message, T);
        assert_eq!(admit(&v, "h3", peer(1), &old, T + RETENTION_MS + 1).unwrap_err().code, "expired");
        assert!(admit(&v, "h3", peer(1), &envelope(N1, 1, 2, Kind::Claim, T), T + RETENTION_MS + 1).is_ok(), "a kept entry ages by its kind, at the receiver");
    }

    #[test]
    fn a_full_queue_lets_the_oldest_go() {
        let caps = Caps { entries: 3, bytes: 100, keep_ms: RETENTION_MS };
        assert_eq!(evict(&[10, 10, 10], 10, &caps), 1);
        assert_eq!(evict(&[10, 10], 10, &caps), 0);
        assert_eq!(evict(&[10, 10, 10], 85, &caps), 2, "by bytes too");
        // Kept in memory: each (network, recipient) a queue of its own.
        let caps = Caps { entries: 3, bytes: 1 << 20, keep_ms: RETENTION_MS };
        let p = crate::persist::Memory::default();
        let v = view(N1, &[1, 2, 3, 4], &[3]);
        for i in 0..5 {
            let env = envelope(N1, 1, 2, Kind::Claim, T + i);
            take(&p, &v, "h3", peer(1), &env, T, &caps).unwrap();
        }
        take(&p, &v, "h3", peer(1), &envelope(N1, 1, 4, Kind::Claim, T), T, &caps).unwrap();
        let to2 = signer(2).public().id();
        assert_eq!(p.mail_for(N1, &to2, 10).unwrap().len(), 3, "the oldest two went");
        assert_eq!(p.mail_for(N1, &signer(4).public().id(), 10).unwrap().len(), 1, "another recipient's queue is its own");
        // Handed over three at a time; the next once they're taken.
        let mut h = Handing { limit: 2, ..Handing::default() };
        let first = h.start(&p, N1, &to2, true).unwrap();
        assert_eq!(first.len(), 2);
        assert!(h.start(&p, N1, &to2, false).unwrap().is_empty(), "on the way still");
        assert!(h.took(&p, N1, &to2, &[first[0].id.clone()]).unwrap().is_empty(), "one still on the way");
        let next = h.took(&p, N1, &to2, &[first[1].id.clone()]).unwrap();
        assert_eq!(next.len(), 1, "the last");
        assert_eq!(h.start(&p, N1, &to2, true).unwrap(), next, "a new connection sends what wasn't taken again");
        // Past the week, it goes.
        assert_eq!(p.prune_mail(T + 1).unwrap(), 2);
    }

    #[test]
    fn a_receiver_remembers_a_message_longer_than_a_mailbox_keeps_it_or_a_token_in_it_holds() {
        use xenonet_log::feed::{LOOSE_DEDUPE_MS, TOKEN_TTL_MS};
        const DAY: u64 = 24 * 60 * 60 * 1000;
        // So a mailbox, or a holder, that hands a message over again finds
        // its dedupe record there, or the message refused as too old.
        assert!(Caps::default().keep_ms <= RETENTION_MS);
        assert!(LOOSE_DEDUPE_MS >= RETENTION_MS.max(TOKEN_TTL_MS) + DAY, "{LOOSE_DEDUPE_MS}");
    }

    #[test]
    fn an_act_with_a_presence_block_goes_through_no_mailbox() {
        let v = view(N1, &[1, 2, 3, 4], &[3, 4]);
        assert_eq!(route(&v, "h1", "h2", |_| true, None).unwrap(), [peer(3), peer(4)]);
        assert_eq!(route(&v, "h1", "h2", |p| p == peer(4), Some(&json!({}))).unwrap(), [peer(4)], "connected ones only");
        assert_eq!(route(&v, "h3", "h2", |_| true, None).unwrap(), [peer(4)], "never itself");
        let e = route(&v, "h1", "h2", |_| true, Some(&json!({ "presence": { "device": "d" } }))).unwrap_err();
        assert!(e.message.contains("offline"), "{}", e.message);
    }

    #[test]
    fn a_receipt_counts_only_from_a_listed_mailbox_on_its_own_connection() {
        let v = view(N1, &[1, 2, 3], &[3]);
        let me = signer(1).public().id();
        let mut b = Batcher::default();
        assert!(b.add(N1, &me, HeldItem { id: "e1".into(), to: signer(2).public().id(), at: T }));
        assert!(!b.add(N1, &me, HeldItem { id: "e2".into(), to: signer(2).public().id(), at: T }));
        let mut held = b.take("h3", T).pop().unwrap();
        assert_eq!(held.items.len(), 2, "batched");
        held.sign(&signer(3)).unwrap();
        let back = Held::parse(&held.to_json()).unwrap();
        assert!(back.verify(&v, peer(3), &me).is_ok());
        assert_eq!(back.verify(&view(N1, &[1, 2, 3], &[]), peer(3), &me).unwrap_err().code, "not-a-mailbox", "unlisted");
        assert_eq!(back.verify(&v, peer(2), &me).unwrap_err().code, "not-forwarded");
        assert_eq!(back.verify(&v, peer(3), &signer(2).public().id()).unwrap_err().code, "wrong-recipient");
        let mut forged = back.clone();
        forged.items[0].id = "e9".into();
        assert_eq!(forged.verify(&v, peer(3), &me).unwrap_err().code, "refused");
        // The author shows them held, not delivered, until the ack.
        let mut h = Handed::default();
        let to = signer(2).public().id();
        h.handed(N1, &to, "e1", Pos::new(T, 1));
        h.handed(N1, &to, "e2", Pos::new(T, 2));
        h.handed(N1, &to, "e3", Pos::new(T, 3));
        assert_eq!(h.standing(N1, &to), Standing { held: 0, unreceipted: 3 });
        h.held(&back);
        assert_eq!(h.standing(N1, &to), Standing { held: 2, unreceipted: 1 });
        h.acked(N1, &to, Pos::new(T, 1));
        assert_eq!(h.held_ids(N1, &to), ["e2"]);
        h.acked(N1, &to, Pos::new(T, 3));
        assert_eq!(h.standing(N1, &to), Standing::default());
    }
}
