//! Fetching a feed's missing entries from any member that holds them
//! (docs/design/network.md, the transport), on link/2.
//!
//! Every member applies the kept entries of the others' feeds, so every
//! member holds them, and a receiver with a gap in one author's feed can
//! fill it from whoever is reachable, not only from the author. A holder
//! can't forge an entry (each is signed by its author's machine signing
//! key, and the receiver checks it against its own copy of the chain, and
//! its `prev` against the entry it applied before), but it can withhold,
//! serve a stale incarnation, or serve a subset that hides a fork. So:
//!
//! - **One holder at a time** (`Fetches`). A Want goes to one member; the
//!   author first when it's connected, then the members that said in their
//!   hello they hold more, then the rest. Each has `ASK_MS` to move the
//!   receiver's head; "don't have" (`Control::DontHave`) is an answer, and
//!   so is an entry that doesn't open or check, and the Want goes to the
//!   next. When every one was asked, the fetch waits `RETRY_MS` and starts
//!   over. A fork split across two holders is caught by the feed itself:
//!   the entry the second holder serves names, as `prev`, another entry
//!   than the one the first served there.
//! - **Members only** (`answer`). A holder serves N's entries only to a
//!   current member of N by its own chain, sealed again for that member;
//!   the head is routing only, so a new envelope around the same signed
//!   entry is as good as the author's. Entries signed before the
//!   requester was admitted go only as the live set: entries still open,
//!   in the author's own compaction (`Compacted`), never the history a new
//!   member wasn't there for. A holder that can't serve that way says it
//!   doesn't have them, and the receiver asks the author.
//! - **Rate limits** per requester (`Limits`): 600 records and 60 messages
//!   a minute, the gateway's limits per link.

use crate::dial::PeerId;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use xenonet_log::feed::{Class, Compacted, FeedEntry, Kind, Pos};

/// How long a holder has to answer a Want before the next is asked.
pub const ASK_MS: u64 = 10_000;
/// How long a fetch that asked every holder waits to start over.
pub const RETRY_MS: u64 = 30_000;
/// The records (envelopes, control records) a requester may be sent in a
/// minute, and the loose entries (messages) among them.
pub const RECORDS_A_MINUTE: u64 = 600;
pub const MESSAGES_A_MINUTE: u64 = 60;
/// The most entries one answer carries; the receiver wants the rest next.
pub const SERVE: usize = 256;

/// A kept entry this machine applied of another member's feed, and whether
/// it is still open here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub entry: FeedEntry,
    pub open: bool,
}

/// Why a holder sends none of what was wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lack {
    /// It holds nothing from there on.
    None,
    /// What's there was signed before the requester was admitted and isn't
    /// open, and it holds no compaction of it: the author answers that.
    History,
    /// The requester asked more than its rate limit lets it be sent.
    Busy,
    /// The requester isn't a member of the network by this machine's chain.
    Refused,
}

impl Lack {
    pub fn as_str(self) -> &'static str {
        match self {
            Lack::None => "none",
            Lack::History => "history",
            Lack::Busy => "busy",
            Lack::Refused => "refused",
        }
    }

    pub fn parse(s: &str) -> Option<Lack> {
        [Lack::None, Lack::History, Lack::Busy, Lack::Refused].into_iter().find(|l| l.as_str() == s)
    }
}

/// What a holder answers a Want with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The entries, in order.
    Entries(Vec<FeedEntry>),
    /// The author's compaction of the range wanted, then the entries after
    /// the compact entry.
    Compacted(Box<Compacted>, Vec<FeedEntry>),
    DontHave(Lack),
}

/// Signed before the requester was admitted and not open: history it
/// wasn't there for, which goes only inside a compaction.
fn hidden(h: &Held, admitted_at: u64) -> bool {
    h.entry.at < admitted_at && !h.open && h.entry.kind != Kind::Compact
}

/// What a holder serves of one author's feed to a requester admitted at
/// `admitted_at` (ms, the time of the chain entry that added it), who wants
/// it from `from` on. `held` is what the holder applied of that feed, in
/// order, from `from` on or earlier.
pub fn answer(from: Pos, held: &[Held], admitted_at: u64, limit: usize) -> Answer {
    let wanted: Vec<&Held> = held.iter().filter(|h| h.entry.pos() >= from && h.entry.class() == Class::Kept).collect();
    if wanted.is_empty() {
        return Answer::DontHave(Lack::None);
    }
    let limit = limit.max(1);
    let Some(last_hidden) = wanted.iter().rposition(|h| hidden(h, admitted_at)) else {
        return Answer::Entries(wanted.iter().take(limit).map(|h| h.entry.clone()).collect());
    };
    // The first compact entry past the history it can't show, covering
    // everything wanted before it.
    let Some(c) = wanted.iter().skip(last_hidden + 1).position(|h| h.entry.kind == Kind::Compact).map(|i| i + last_hidden + 1) else {
        return Answer::DontHave(Lack::History);
    };
    let compact = &wanted[c].entry;
    let Some(body) = compact.compact() else { return Answer::DontHave(Lack::History) };
    let range = &wanted[..c];
    let covered = range.iter().all(|h| h.entry.inc == body.inc && h.entry.seq <= body.upto);
    // The header chain runs gapless from the first place wanted to `upto`.
    let lo = if from.inc == body.inc { from.seq.max(1) } else { 1 };
    let gapless = range.iter().map(|h| h.entry.seq).eq(lo..=body.upto);
    if !covered || !gapless {
        return Answer::DontHave(Lack::History);
    }
    let live: Vec<FeedEntry> = range.iter().filter(|h| body.live.contains(&h.entry.seq)).map(|h| h.entry.clone()).collect();
    let headers = range.iter().map(|h| h.entry.header()).collect();
    // After the compact entry, what's shown in full, up to the next
    // history it can't show: the receiver wants the rest again.
    let after: Vec<FeedEntry> = wanted[c + 1..].iter().take_while(|h| !hidden(h, admitted_at)).take(limit).map(|h| h.entry.clone()).collect();
    Answer::Compacted(Box::new(Compacted { compact: compact.clone(), live, headers }), after)
}

/// The time the chain entry that added a member was made, in ms.
pub fn admitted_at(roster: &crate::roster::Roster, member: &crate::roster::Member) -> u64 {
    roster.entries().get(usize::try_from(member.since).unwrap_or(usize::MAX)).map_or(u64::MAX, |e| u64::try_from(e.body.at).unwrap_or(0))
}

// ---------- the receiver's side ----------

/// One author's feed in one network, as a fetch names it.
pub type FeedKey = (String, String);

#[derive(Debug, Clone)]
struct Fetch {
    /// The first place missing.
    from: Pos,
    /// The holders asked since the fetch began, or began again.
    tried: BTreeSet<PeerId>,
    /// The holder asked now, and when.
    asking: Option<(PeerId, u64)>,
    /// When every holder was asked, and none filled it.
    parked: Option<u64>,
}

/// A Want to send: to whom, for which feed, from where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub to: PeerId,
    pub network: String,
    pub author: String,
    pub from: Pos,
}

/// The receiver's fetches: for each feed with a gap, the holder asked now
/// and the ones asked before.
#[derive(Debug, Default)]
pub struct Fetches {
    open: BTreeMap<FeedKey, Fetch>,
}

impl Fetches {
    /// The feeds with a gap now.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// The feeds with a gap.
    pub fn keys(&self) -> impl Iterator<Item = &FeedKey> {
        self.open.keys()
    }

    /// Who is asked for `author`'s feed in `network` now, if anyone.
    pub fn asking(&self, network: &str, author: &str) -> Option<PeerId> {
        self.open.get(&(network.to_string(), author.to_string()))?.asking.map(|(p, _)| p)
    }

    /// A gap at `from`, with `holders` the members that may hold it, best
    /// first. The Want to send, if one is to go now: none while a holder
    /// asked for the same place still has time.
    pub fn want(&mut self, network: &str, author: &str, from: Pos, holders: &[PeerId], now: u64) -> Option<Ask> {
        let key = (network.to_string(), author.to_string());
        let f = self.open.entry(key).or_insert_with(|| Fetch { from, tried: BTreeSet::new(), asking: None, parked: None });
        if from > f.from {
            // The head moved: the gap is a new one, and every holder may
            // have it.
            *f = Fetch { from, tried: BTreeSet::new(), asking: f.asking, parked: None };
            // The holder that moved it is asked again first.
            if let Some((p, _)) = f.asking {
                if holders.contains(&p) {
                    f.asking = Some((p, now));
                    return Some(Ask { to: p, network: network.into(), author: author.into(), from });
                }
            }
            f.asking = None;
        }
        let busy = f.asking.is_some_and(|(_, at)| now < at.saturating_add(ASK_MS));
        // Parked, every holder asked: a holder it hasn't asked yet (one
        // that just connected) is asked now; the rest wait their turn.
        let spent = f.parked.is_some() && holders.iter().all(|p| f.tried.contains(p));
        if busy || spent {
            return None;
        }
        Self::next(f, network, author, holders, now)
    }

    fn next(f: &mut Fetch, network: &str, author: &str, holders: &[PeerId], now: u64) -> Option<Ask> {
        if let Some((p, _)) = f.asking.take() {
            f.tried.insert(p);
        }
        match holders.iter().find(|p| !f.tried.contains(p)) {
            Some(p) => {
                f.asking = Some((*p, now));
                f.parked = None;
                Some(Ask { to: *p, network: network.into(), author: author.into(), from: f.from })
            }
            None => {
                f.parked = Some(now);
                None
            }
        }
    }

    /// `peer` answered that it doesn't have it, or served something that
    /// didn't check: the next holder is asked.
    pub fn lacks(&mut self, network: &str, author: &str, peer: PeerId, holders: &[PeerId], now: u64) -> Option<Ask> {
        let f = self.open.get_mut(&(network.to_string(), author.to_string()))?;
        if f.asking.is_some_and(|(p, _)| p == peer) {
            return Self::next(f, network, author, holders, now);
        }
        f.tried.insert(peer);
        None
    }

    /// This machine now holds `author`'s feed up to `head`: a fetch whose
    /// gap that fills is done.
    pub fn filled(&mut self, network: &str, author: &str, head: Pos) {
        let key = (network.to_string(), author.to_string());
        if self.open.get(&key).is_some_and(|f| head >= f.from) {
            self.open.remove(&key);
        }
    }

    /// The fetches whose holder ran out of time, and the parked ones whose
    /// wait is over: each goes to the next holder `holders` names for it.
    pub fn due(&mut self, now: u64, holders: impl Fn(&str, &str) -> Vec<PeerId>) -> Vec<Ask> {
        let mut out = Vec::new();
        for ((network, author), f) in &mut self.open {
            let go = match (f.asking, f.parked) {
                (Some((_, at)), _) => now >= at.saturating_add(ASK_MS),
                (None, Some(at)) if now >= at.saturating_add(RETRY_MS) => {
                    f.tried.clear();
                    f.parked = None;
                    true
                }
                (None, Some(_)) => false,
                (None, None) => true,
            };
            if !go {
                continue;
            }
            if let Some(a) = Self::next(f, network, author, &holders(network, author), now) {
                out.push(a);
            }
        }
        out
    }

    /// When `due` next has something to do, if ever.
    pub fn next_due(&self) -> Option<u64> {
        self.open
            .values()
            .map(|f| match (f.asking, f.parked) {
                (Some((_, at)), _) => at.saturating_add(ASK_MS),
                (None, Some(at)) => at.saturating_add(RETRY_MS),
                (None, None) => 0,
            })
            .min()
    }

    /// A feed's fetch is given up: its author left the network.
    pub fn forget(&mut self, network: &str, author: &str) {
        self.open.remove(&(network.to_string(), author.to_string()));
    }
}

/// What a machine keeps for fetching and serving: its fetches, the peers
/// whose hello said they hold more of a feed than it does, and each
/// requester's rate.
#[derive(Debug, Default)]
pub struct Fetching {
    pub fetches: Fetches,
    pub ahead: BTreeMap<FeedKey, Vec<PeerId>>,
    pub limits: Limits,
}

impl Fetching {
    /// `peer`'s hello says it holds `author`'s feed up to `at`, past
    /// `mine`: the place to fetch from, if it is ahead.
    pub fn ahead(&mut self, network: &str, author: &str, peer: PeerId, at: Pos, mine: Pos) -> Option<Pos> {
        if at <= mine {
            return None;
        }
        let peers = self.ahead.entry((network.to_string(), author.to_string())).or_default();
        if !peers.contains(&peer) {
            peers.push(peer);
        }
        Some(Pos::new(mine.inc, mine.seq + 1))
    }

    /// The members ahead of this machine on a feed.
    pub fn ahead_of(&self, network: &str, author: &str) -> &[PeerId] {
        self.ahead.get(&(network.to_string(), author.to_string())).map_or(&[], Vec::as_slice)
    }
}

/// The holders to ask for `author`'s feed, best first: the author, then
/// the members whose hello said they hold more of it, then the others,
/// each connected now.
pub fn order(author: Option<PeerId>, ahead: &[PeerId], connected: &[PeerId]) -> Vec<PeerId> {
    let mut out: Vec<PeerId> = Vec::new();
    for p in author.into_iter().chain(ahead.iter().copied()).chain(connected.iter().copied()) {
        if connected.contains(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

// ---------- rate limits ----------

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: u64,
    at: u64,
}

impl Bucket {
    fn full(per_minute: u64, now: u64) -> Bucket {
        Bucket { tokens: per_minute, at: now }
    }

    /// Up to `n` tokens, as many as there are.
    fn take(&mut self, n: u64, per_minute: u64, now: u64) -> u64 {
        let refill = now.saturating_sub(self.at).saturating_mul(per_minute) / 60_000;
        if refill > 0 {
            self.tokens = (self.tokens + refill).min(per_minute);
            self.at = now;
        }
        let got = n.min(self.tokens);
        self.tokens -= got;
        got
    }
}

/// What each requester may still be sent this minute.
#[derive(Debug, Default)]
pub struct Limits {
    per: HashMap<PeerId, (Bucket, Bucket)>,
}

impl Limits {
    fn of(&mut self, peer: PeerId, now: u64) -> &mut (Bucket, Bucket) {
        self.per.entry(peer).or_insert_with(|| (Bucket::full(RECORDS_A_MINUTE, now), Bucket::full(MESSAGES_A_MINUTE, now)))
    }

    /// How many of `n` records may go to `peer` now.
    pub fn records(&mut self, peer: PeerId, n: usize, now: u64) -> usize {
        usize::try_from(self.of(peer, now).0.take(n as u64, RECORDS_A_MINUTE, now)).unwrap_or(0)
    }

    /// Whether a message (a loose entry) may go to `peer` now; it counts
    /// as a record too.
    pub fn message(&mut self, peer: PeerId, now: u64) -> bool {
        let b = self.of(peer, now);
        if b.1.take(1, MESSAGES_A_MINUTE, now) == 0 {
            return false;
        }
        b.0.take(1, RECORDS_A_MINUTE, now) == 1
    }

    /// Requesters not heard from in a while are forgotten.
    pub fn prune(&mut self, now: u64) {
        self.per.retain(|_, (r, m)| now.saturating_sub(r.at.max(m.at)) < 10 * 60_000);
    }
}

/// An answer as the records that carry it: `envelope` seals each entry for
/// the requester. What the rate limit doesn't let go is cut, and a holder
/// that has nothing it may send says so.
pub fn records(answer: Answer, network: &str, author: &str, from: Pos, room: usize, envelope: impl Fn(&FeedEntry) -> Option<crate::envelope::Envelope>) -> Vec<crate::link2::Out> {
    use crate::link2::{Control, Out};
    let lack = |l: Lack| vec![Out::Control(Control::DontHave { network: network.into(), author: author.into(), inc: from.inc, from: from.seq, why: l })];
    if room == 0 {
        return lack(Lack::Busy);
    }
    match answer {
        Answer::DontHave(l) => lack(l),
        Answer::Entries(es) => es.iter().take(room).filter_map(&envelope).map(Out::Envelope).collect(),
        Answer::Compacted(c, after) => {
            let mut out = vec![Out::Control(Control::Compacted { network: network.into(), author: author.into(), body: c.to_json() })];
            out.extend(after.iter().take(room.saturating_sub(1)).filter_map(&envelope).map(Out::Envelope));
            out
        }
    }
}

/// A Want for `author`'s feed in `want.0` from `want.1` on, from the peer
/// `requester`, answered as a holder, by this machine's chain `view`: to a
/// current member of the network alone, sealed again for it, with only
/// the live set of what was signed before it was admitted
/// (`admitted_at`), within its rate limit. `held` reads what this machine
/// applied of the feed from a place on.
pub fn serve(
    view: &crate::roster::View,
    admitted_at: impl Fn(&crate::roster::Member) -> u64,
    requester: PeerId,
    want: (&str, &str, Pos),
    held: impl FnOnce(Pos) -> Vec<Held>,
    limits: &mut Limits,
    now: u64,
) -> Vec<crate::link2::Out> {
    serve_in(&Network { view, admitted_at }, requester, want, held, limits, now)
}

/// A member asking a holder for entries, as the holder's own copy of the
/// group's chain has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requester {
    /// Its own feed's author id: a holder never serves a member its own.
    pub author: String,
    /// When it was admitted (ms): what was signed before goes only as the
    /// live set.
    pub admitted_at: u64,
}

/// A group as a holder decides on it: this machine's own copy of the
/// group's chain. A network's roster is one (`serve`); another program
/// brings its own (Xenolith: a space's chain, which admits a requester only
/// at read or above, rev 14).
pub trait Holds {
    /// The group's id.
    fn group(&self) -> &str;
    /// The current member at `peer`'s endpoint id that may be served the
    /// group's entries, or None.
    fn requester(&self, peer: PeerId) -> Option<Requester>;
    /// Whether `author` writes a feed in the group.
    fn writes(&self, author: &str) -> bool;
    /// `entry` in an envelope for the requester at `peer` (sealed for it).
    fn envelope(&self, entry: &FeedEntry, peer: PeerId, to: &Requester) -> Option<crate::envelope::Envelope>;
}

/// A Want for `author`'s feed in `want.0` from `want.1` on, from the peer
/// `requester`, answered as a holder of `group`: to a current member of it
/// alone, in an envelope for it, with only the live set of what was signed
/// before it was admitted, within its rate limit. `held` reads what this
/// machine applied of the feed from a place on.
pub fn serve_in(group: &impl Holds, requester: PeerId, want: (&str, &str, Pos), held: impl FnOnce(Pos) -> Vec<Held>, limits: &mut Limits, now: u64) -> Vec<crate::link2::Out> {
    let (network, author, from) = want;
    let no = |l: Lack| records(Answer::DontHave(l), network, author, from, 1, |_| None);
    let Some(member) = (group.group() == network).then(|| group.requester(requester)).flatten() else {
        return no(Lack::Refused);
    };
    if !group.writes(author) || member.author == author {
        return no(Lack::None);
    }
    let a = answer(from, &held(from), member.admitted_at, SERVE);
    let n = match &a {
        Answer::Entries(es) => es.len(),
        Answer::Compacted(_, after) => after.len() + 1,
        Answer::DontHave(_) => 1,
    };
    let room = limits.records(requester, n, now);
    records(a, network, author, from, room, |e| group.envelope(e, requester, &member))
}

// A network's roster as a holder decides on it.
struct Network<'a, F> {
    view: &'a crate::roster::View,
    admitted_at: F,
}

impl<'a, F: Fn(&crate::roster::Member) -> u64> Network<'a, F> {
    fn member(&self, peer: PeerId) -> Option<&'a crate::roster::Member> {
        self.view.members.values().find(|m| m.keys.sign == peer.0)
    }
}

impl<F: Fn(&crate::roster::Member) -> u64> Holds for Network<'_, F> {
    fn group(&self) -> &str {
        &self.view.network
    }

    fn requester(&self, peer: PeerId) -> Option<Requester> {
        self.member(peer).map(|m| Requester { author: m.machine.id(), admitted_at: (self.admitted_at)(m) })
    }

    fn writes(&self, author: &str) -> bool {
        self.view.by_machine(author).is_some()
    }

    fn envelope(&self, entry: &FeedEntry, peer: PeerId, to: &Requester) -> Option<crate::envelope::Envelope> {
        let m = self.member(peer)?;
        crate::envelope::Envelope::seal_entry(entry, &to.author, &m.keys, None).ok()
    }
}

/// A `Compacted` body from a holder, read back.
pub fn compacted_of(body: &Value) -> Option<Compacted> {
    Compacted::parse(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
    use xenonet_log::feed::Writer;

    const N: &str = "net_00000000000000000000000000000001";
    const T: u64 = 1_792_512_000_000;

    fn p(n: u8) -> PeerId {
        PeerId([n; 32])
    }

    fn feed(n: usize) -> (SeedSigner, Writer, Vec<FeedEntry>) {
        let key = SeedSigner::from_seed(&[7; 32], KeyHome::File).unwrap();
        let mut w = Writer::start(key.public().id(), N, T);
        let es = (0..n).map(|i| w.write(&key, Kind::Claim, T + i as u64 * 1000, json!({ "project": "prj_atlas", "path": format!("{i}.rs") })).unwrap()).collect();
        (key, w, es)
    }

    fn held(es: &[FeedEntry], open: impl Fn(u64) -> bool) -> Vec<Held> {
        es.iter().map(|e| Held { entry: e.clone(), open: open(e.seq) }).collect()
    }

    #[test]
    fn a_member_there_from_the_start_gets_everything_from_where_it_wants() {
        let (_, _, es) = feed(5);
        let h = held(&es, |_| false);
        assert_eq!(answer(Pos::new(es[0].inc, 3), &h, 0, SERVE), Answer::Entries(es[2..].to_vec()));
        assert_eq!(answer(Pos::new(0, 1), &h, 0, 2), Answer::Entries(es[..2].to_vec()), "at most the limit");
        assert_eq!(answer(Pos::new(es[0].inc, 6), &h, 0, SERVE), Answer::DontHave(Lack::None));
    }

    #[test]
    fn a_new_member_gets_no_history_it_wasnt_there_for_only_the_authors_compaction_of_it() {
        let (key, mut w, mut es) = feed(5);
        // Admitted after the third entry was signed; 2 and 3 are closed.
        let admitted = es[2].at + 1;
        let open = |s: u64| s != 2 && s != 3;
        assert_eq!(answer(Pos::new(0, 1), &held(&es, open), admitted, SERVE), Answer::DontHave(Lack::History), "no compaction to show it through");
        // All still open: nothing is hidden.
        assert_eq!(answer(Pos::new(0, 1), &held(&es, |_| true), admitted, SERVE), Answer::Entries(es.clone()));
        // The author compacts, keeping 1, 4 and 5; then writes a sixth.
        let c = w.compact(&key, T + 10_000, &[1, 4, 5]).unwrap();
        let six = w.write(&key, Kind::Claim, T + 11_000, json!({ "project": "prj_atlas", "path": "6.rs" })).unwrap();
        es.extend([c.clone(), six.clone()]);
        let Answer::Compacted(got, after) = answer(Pos::new(0, 1), &held(&es, open), admitted, SERVE) else { panic!("a compaction") };
        assert_eq!(got.compact, c);
        assert_eq!(got.live.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 4, 5], "the open entries, never 2 or 3");
        assert_eq!(got.headers.len(), 5, "the header chain of the range");
        assert_eq!(after, [six]);
        // And what the author signed is what checks: the receiver's feed
        // takes it.
        assert!(got.verify(&key.public()).is_ok());
    }

    #[test]
    fn a_holder_that_lost_part_of_the_range_says_so_rather_than_send_a_chain_with_holes() {
        let (key, mut w, mut es) = feed(4);
        let c = w.compact(&key, T + 10_000, &[4]).unwrap();
        es.push(c);
        es.remove(1);
        assert_eq!(answer(Pos::new(0, 1), &held(&es, |s| s == 4), es[3].at + 1, SERVE), Answer::DontHave(Lack::History));
    }

    #[test]
    fn one_holder_is_asked_at_a_time_and_the_next_after_a_dont_have_or_the_timeout() {
        let mut f = Fetches::default();
        let from = Pos::new(T, 3);
        let hs = [p(1), p(2), p(3)];
        let ask = f.want(N, "a", from, &hs, 0).unwrap();
        assert_eq!(ask.to, p(1));
        assert_eq!(f.want(N, "a", from, &hs, 100), None, "the one asked has time");
        assert_eq!(f.lacks(N, "a", p(1), &hs, 200).unwrap().to, p(2), "don't have is an answer");
        assert_eq!(f.due(200 + ASK_MS - 1, |_, _| hs.to_vec()), [], "the one asked still has time");
        assert_eq!(f.due(200 + ASK_MS, |_, _| hs.to_vec())[0].to, p(3), "silence is skipped after ten seconds");
        assert!(f.due(200 + 2 * ASK_MS, |_, _| hs.to_vec()).is_empty(), "every one asked: it waits");
        let t = 200 + 2 * ASK_MS + RETRY_MS;
        assert_eq!(f.due(t, |_, _| hs.to_vec())[0].to, p(1), "and starts over");
        f.filled(N, "a", Pos::new(T, 2));
        assert_eq!(f.len(), 1, "not filled yet");
        f.filled(N, "a", from);
        assert!(f.is_empty(), "done");
    }

    #[test]
    fn a_holder_that_moved_the_head_is_asked_first_for_what_is_still_missing() {
        let mut f = Fetches::default();
        let hs = [p(1), p(2)];
        assert_eq!(f.want(N, "a", Pos::new(T, 1), &hs, 0).unwrap().to, p(1));
        let next = f.want(N, "a", Pos::new(T, 257), &hs, 50).unwrap();
        assert_eq!((next.to, next.from), (p(1), Pos::new(T, 257)));
    }

    #[test]
    fn the_author_goes_first_then_the_members_ahead_then_the_rest_connected_only() {
        assert_eq!(order(Some(p(9)), &[p(3)], &[p(1), p(3)]), [p(3), p(1)], "the author isn't connected");
        assert_eq!(order(Some(p(1)), &[p(3)], &[p(2), p(3), p(1)]), [p(1), p(3), p(2)]);
    }

    #[test]
    fn each_requester_gets_600_records_and_60_messages_a_minute() {
        let mut l = Limits::default();
        assert_eq!(l.records(p(1), 500, 0), 500);
        assert_eq!(l.records(p(1), 500, 0), 100);
        assert_eq!(l.records(p(1), 1, 0), 0);
        assert_eq!(l.records(p(2), 1, 0), 1, "per requester");
        assert_eq!(l.records(p(1), 100, 10_000), 100, "a sixth of a minute refills a sixth");
        let sent = (0..100).filter(|_| l.message(p(3), 0)).count();
        assert_eq!(sent, 60);
    }
}
