//! Retention and compaction: what a compact entry keeps and drops, how a
//! receiver behind it catches up from the author's `Compacted` answer, and
//! the phase 5 gates (revision 15): the live set is kept under random
//! deliveries, cancel-then-compact stops a run, a held claim behind a gap
//! doesn't come back, and a fork in a compacted range is still detected.
//! The same rules are modelled in formal/delivery.qnt.

use super::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use xenonet_id::sign::{KeyHome, SeedSigner};

const NET: &str = "net_00112233445566778899aabbccddeeff";
const T: u64 = 1_792_512_000_000;
const HOUR: u64 = 60 * 60 * 1000;

fn key() -> SeedSigner {
    SeedSigner::from_seed(&[3; 32], KeyHome::File).unwrap()
}

/// An author with what it still holds of its feed, as the store keeps it:
/// the kept entries it can resend, and the header of every kept entry it
/// wrote. A compact entry drops what it doesn't keep, keeping its header.
struct Author {
    k: SeedSigner,
    w: Writer,
    held: Vec<FeedEntry>,
    headers: Vec<Header>,
    /// Every kept entry it ever signed: what a replaying holder can serve.
    signed: Vec<FeedEntry>,
}

impl Author {
    fn new() -> Author {
        let k = key();
        let w = Writer::start(k.public().id(), NET, T);
        Author { k, w, held: Vec::new(), headers: Vec::new(), signed: Vec::new() }
    }

    fn id(&self) -> String {
        self.k.public().id()
    }

    fn keep(&mut self, e: &FeedEntry) {
        self.held.push(e.clone());
        self.headers.push(e.header());
        self.signed.push(e.clone());
    }

    fn write(&mut self, kind: Kind, at: u64, body: Value) -> FeedEntry {
        let e = self.w.write(&self.k, kind, at, body).unwrap();
        self.keep(&e);
        e
    }

    /// A compact entry at `now`, keeping what `open` says is open.
    fn compact(&mut self, now: u64, open: impl Fn(&FeedEntry) -> bool) -> FeedEntry {
        let live = live_set(&self.held, self.w.inc(), now, open).unwrap();
        let e = self.w.compact(&self.k, now, &live).unwrap();
        self.held.retain(|x| x.inc != e.inc || live.binary_search(&x.seq).is_ok());
        self.keep(&e);
        e
    }

    /// The answer to a Want from `after` on, when it's below the floor.
    fn answer(&self, after: Pos) -> Option<Compacted> {
        let compact = self.held.iter().rev().find(|e| e.kind == Kind::Compact)?.clone();
        let c = compact.compact()?;
        if after.inc > c.inc || after >= Pos::new(c.inc, c.upto) {
            return None;
        }
        let lo = if after.inc == c.inc { after.seq + 1 } else { 1 };
        let range = |inc: u64, seq: u64| inc == c.inc && (lo..=c.upto).contains(&seq);
        Some(Compacted {
            compact,
            live: self.held.iter().filter(|e| range(e.inc, e.seq)).cloned().collect(),
            headers: self.headers.iter().filter(|h| range(h.inc, h.seq)).cloned().collect(),
        })
    }

    fn verified(&self, e: &FeedEntry) -> Verified {
        e.clone().verify(&self.k.public()).unwrap()
    }
}

fn feed() -> Feed<MemoryStore> {
    Feed::at(MemoryStore::new(), T + HOUR)
}

fn offer(f: &mut Feed<MemoryStore>, a: &Author, e: &FeedEntry) -> Offer {
    f.offer(a.verified(e)).unwrap()
}

fn offer_compacted(f: &mut Feed<MemoryStore>, a: &Author, c: Compacted) -> Offer {
    f.offer_compacted(c.verify(&a.k.public()).unwrap()).unwrap()
}

fn applied(o: &Offer) -> &Applied {
    match o {
        Offer::New(a) => a,
        other => panic!("not taken: {other:?}"),
    }
}

fn seqs(o: &Offer) -> Vec<u64> {
    applied(o).iter().map(|e| e.seq).collect()
}

fn after_ack(f: &Feed<MemoryStore>, a: &Author) -> Pos {
    f.ack(&a.id(), NET).unwrap().map_or(Pos::ORIGIN, |x| Pos::new(x.inc, x.high))
}

/// The author's answer to the receiver's Want, taken.
fn catch_up(f: &mut Feed<MemoryStore>, a: &Author) -> Offer {
    let answer = a.answer(after_ack(f, a)).unwrap();
    offer_compacted(f, a, answer)
}

#[test]
fn each_kind_has_its_retention() {
    assert_eq!(LOOSE_DEDUPE_MS, 9 * 24 * HOUR, "past 7 days' retention and 8 days' tokens, with a day's margin");
    let mut a = Author::new();
    let claim = a.write(Kind::Claim, T, json!({ "path": "a" }));
    let released = a.write(Kind::Claim, T, json!({ "path": "b" }));
    let report = a.write(Kind::Report, T, json!({ "text": "old" }));
    let intent = a.write(Kind::Intent, T, json!({ "op": "make", "until": T + HOUR }));
    let forever = a.write(Kind::Intent, T, json!({ "op": "make" }));
    let handoff = a.write(Kind::Handoff, T, json!({ "op": "offer" }));
    let open = |e: &FeedEntry| e.seq != released.seq;
    let at = |now: u64| live_set(&a.held, a.w.inc(), now, open).unwrap();
    assert_eq!(at(T + 1), [claim.seq, report.seq, intent.seq, forever.seq, handoff.seq]);
    assert_eq!(at(T + 2 * HOUR), [claim.seq, report.seq, forever.seq, handoff.seq], "an intent goes once its until passes");
    assert_eq!(at(T + RETENTION_MS + 1), [claim.seq, forever.seq, handoff.seq], "a report ages out");
    let c = a.compact(T + 2, open);
    assert!(!live_set(&a.held, a.w.inc(), T + 3, |_| true).unwrap().contains(&c.seq), "a compact entry is superseded");
    // An open entry of an earlier incarnation is written again first.
    let mut r = Writer::restore(a.id(), NET, a.w.head(), a.w.inc(), T + 5);
    let later = r.write(&a.k, Kind::Claim, T + 6, json!({})).unwrap();
    assert!(live_set(&[claim.clone(), later.clone()], r.inc(), T + 7, |_| true).is_err());
    assert_eq!(live_set(&[claim, later], r.inc(), T + 7, |e| e.inc == r.inc()).unwrap(), [1]);
}

#[test]
fn a_compact_entry_holds_to_its_shape() {
    let mut a = Author::new();
    assert!(a.w.compact(&a.k, T, &[]).is_err(), "nothing to compact yet");
    let one = a.write(Kind::Claim, T, json!({}));
    a.write(Kind::Claim, T, json!({}));
    assert!(a.w.clone().compact(&a.k, T, &[3]).is_err(), "it keeps only places it covers");
    assert!(a.w.clone().write(&a.k, Kind::Compact, T, json!({})).is_err(), "written only as one");
    let c = a.w.clone().compact(&a.k, T + 1, &[2, 1, 2]).unwrap();
    let body = c.compact().unwrap();
    assert_eq!((body.inc, body.upto, body.live.clone(), body.base), (a.w.inc(), 2, vec![1, 2], Link::ORIGIN));
    assert_eq!(Some(body.at_id), a.w.head().id);
    assert_eq!(FeedEntry::decode(&c.encode()).unwrap(), c);
    let bad = |f: &dyn Fn(&mut Value)| {
        let mut v = c.to_json();
        f(&mut v["body"]);
        FeedEntry::parse(&v).is_err()
    };
    assert!(bad(&|b| b["live"] = json!([2, 1])), "in order");
    assert!(bad(&|b| b["live"] = json!([0])), "from 1");
    assert!(bad(&|b| b["live"] = json!([3])), "up to upto");
    assert!(bad(&|b| b["upto"] = json!(1)), "everything before it");
    assert!(bad(&|b| b["inc"] = json!(7)), "its own incarnation");
    assert!(bad(&|b| b["atId"] = json!(one.id())), "the id it names is the entry before it");
    assert!(bad(&|b| b["base"] = json!({ "inc": T + 10, "seq": 1, "id": one.id() })), "the link is before it");
    assert!(bad(&|b| b["base"] = json!({ "inc": 0, "seq": 0, "id": one.id() })), "the origin has no id");
    assert!(bad(&|b| *b = json!({ "upto": 2 })), "a compact body");
}

#[test]
fn a_compact_entry_ends_what_its_live_set_leaves_out() {
    let mut a = Author::new();
    let kept = a.write(Kind::Claim, T, json!({ "path": "kept" }));
    let released = a.write(Kind::Claim, T, json!({ "path": "released" }));
    let soon = a.write(Kind::Intent, T, json!({ "op": "make", "until": T + 10 }));
    let later = a.write(Kind::Intent, T, json!({ "op": "make", "until": T + 2 * HOUR }));
    let mut f = feed();
    for e in [&kept, &released, &soon, &later] {
        offer(&mut f, &a, e);
    }
    // A wrong author leaves the long intent out before its until.
    let live = [kept.seq];
    let c = a.w.compact(&a.k, T + HOUR, &live).unwrap();
    let o = offer(&mut f, &a, &c);
    assert_eq!(applied(&o).ended, [released.pos(), soon.pos()], "the released claim, and the intent past its until; not the one still holding");
    let again = a.w.compact(&a.k, T + HOUR + 1, &live).unwrap();
    let o = offer(&mut f, &a, &again);
    assert!(applied(&o).ended.is_empty(), "what's ended stays ended");
    assert_eq!(f.store().open_upto(&a.id(), NET, Pos::new(u64::MAX, 0)).unwrap().iter().map(|o| o.at).collect::<Vec<_>>(), [kept.pos(), later.pos(), Pos::new(a.w.inc(), 6)]);
}

/// A receiver standing in for the engine: the runs it started from
/// intents, by the place of the intent, and what it applied.
#[derive(Default)]
struct Runs {
    running: BTreeSet<Pos>,
}

impl Runs {
    fn take(&mut self, o: &Offer) {
        let Offer::New(a) = o else { return };
        for e in a {
            match (e.kind, e.body["op"].as_str()) {
                (Kind::Intent, Some("make")) => {
                    self.running.insert(e.pos());
                }
                (Kind::Intent, Some("cancel")) => {
                    self.running.remove(&Pos::new(e.inc, e.body["of"].as_u64().unwrap()));
                }
                _ => {}
            }
        }
        for p in &a.ended {
            self.running.remove(p);
        }
    }
}

#[test]
fn cancel_then_compact_stops_a_run() {
    // B saw the intent and started; A gets the person's cancel and
    // compacts; B, back later, gets only the compacted answer.
    for (when, why) in [(T + HOUR / 2, "before until: the cancel stays live"), (T + 2 * HOUR, "after until: absence ends it")] {
        let mut a = Author::new();
        let make = a.write(Kind::Intent, T, json!({ "op": "make", "until": T + HOUR }));
        let mut b = feed();
        let mut runs = Runs::default();
        runs.take(&offer(&mut b, &a, &make));
        assert_eq!(runs.running.len(), 1);
        let cancel = a.write(Kind::Intent, T + 1, json!({ "op": "cancel", "of": make.seq, "until": T + HOUR }));
        let c = a.compact(when, |_| false);
        let live = c.compact().unwrap().live;
        assert_eq!(live.is_empty(), when > T + HOUR, "{why}");
        assert!(matches!(offer(&mut b, &a, &c), Offer::Gap(_)), "alone, it waits for its chain");
        let o = catch_up(&mut b, &a);
        runs.take(&o);
        assert!(runs.running.is_empty(), "{why}");
        assert_eq!(seqs(&o), if live.is_empty() { vec![c.seq] } else { vec![cancel.seq, c.seq] }, "{why}");
    }
}

#[test]
fn a_held_claim_behind_a_gap_doesnt_come_back() {
    let mut a = Author::new();
    let one = a.write(Kind::Claim, T, json!({ "path": "one" }));
    let two = a.write(Kind::Claim, T, json!({ "path": "two" }));
    let released = a.write(Kind::Claim, T, json!({ "op": "release", "path": "two" }));
    let c = a.compact(T + 1, |e| e.seq == one.seq);
    let mut f = feed();
    seqs(&offer(&mut f, &a, &one));
    assert!(matches!(offer(&mut f, &a, &released), Offer::Gap(_)), "held behind two");
    let o = catch_up(&mut f, &a);
    assert_eq!(seqs(&o), [c.seq], "nothing live it lacked, then the compact entry");
    assert_eq!(f.store().held_count(&a.id(), NET), 0, "the held entry is dropped");
    assert_eq!(offer(&mut f, &a, &two), Offer::Duplicate, "on the chain, and covered");
    assert_eq!(offer(&mut f, &a, &released), Offer::Duplicate, "never applied");
    assert_eq!(f.store().open_upto(&a.id(), NET, c.pos()).unwrap().iter().map(|o| o.at).collect::<Vec<_>>(), [one.pos(), c.pos()]);
    assert_eq!(f.ack(&a.id(), NET).unwrap(), Some(Ack { inc: c.inc, high: c.seq }));
}

#[test]
fn a_receiver_behind_takes_the_live_set_it_lacks() {
    let mut a = Author::new();
    let one = a.write(Kind::Claim, T, json!({ "path": "one" }));
    a.write(Kind::Claim, T, json!({ "path": "gone" }));
    let three = a.write(Kind::Claim, T, json!({ "path": "three" }));
    let c = a.compact(T + 1, |e| e.seq != 2);
    let after = a.write(Kind::Claim, T + 2, json!({ "path": "after" }));
    // A new receiver: the whole range's chain, both live entries.
    let mut f = feed();
    let answer = a.answer(Pos::ORIGIN).unwrap();
    assert_eq!(answer.headers.len(), 3);
    assert!(matches!(offer(&mut f, &a, &after), Offer::Gap(_)));
    let o = offer_compacted(&mut f, &a, answer.clone());
    assert_eq!(seqs(&o), [one.seq, three.seq, c.seq, after.seq], "and what was held after it");
    assert_eq!(offer_compacted(&mut f, &a, answer), Offer::Duplicate);
    // One that has the first: the chain from the second on.
    let mut g = feed();
    seqs(&offer(&mut g, &a, &one));
    let answer = a.answer(after_ack(&g, &a)).unwrap();
    assert_eq!((answer.headers.len(), answer.live.len()), (2, 1));
    assert_eq!(seqs(&offer_compacted(&mut g, &a, answer)), [three.seq, c.seq]);
    // A short answer or a broken chain is asked for again, not a fork.
    let mut h = feed();
    let mut short = a.answer(Pos::ORIGIN).unwrap();
    short.live.pop();
    assert!(matches!(offer_compacted(&mut h, &a, short), Offer::Gap(_)));
    let mut broken = a.answer(Pos::ORIGIN).unwrap();
    broken.headers.remove(1);
    assert!(matches!(offer_compacted(&mut h, &a, broken), Offer::Gap(_)));
    let mut wrong = a.answer(Pos::ORIGIN).unwrap();
    wrong.headers[1].at += 1;
    assert!(matches!(offer_compacted(&mut h, &a, wrong), Offer::Gap(_)));
    assert_eq!(h.frozen(&a.id(), NET).unwrap(), None);
    assert_eq!(seqs(&offer_compacted(&mut h, &a, a.answer(Pos::ORIGIN).unwrap())), [one.seq, three.seq, c.seq]);
}

/// An author that signed a second entry at `at` and went on from it:
/// the honest side `first`, the other side and what followed it.
fn forked(at: u64) -> (Author, Vec<FeedEntry>) {
    let mut a = Author::new();
    for n in 1..at {
        a.write(Kind::Claim, T, json!({ "n": n }));
    }
    let mut other = a.w.clone();
    let first = a.write(Kind::Claim, T, json!({ "side": "first" }));
    let second = other.write(&a.k, Kind::Claim, T, json!({ "side": "second" })).unwrap();
    // The author goes on from the second side: its store has that one.
    a.w = other;
    a.held.retain(|e| e.pos() != first.pos());
    a.headers.retain(|h| h.pos() != first.pos());
    a.keep(&second);
    a.write(Kind::Claim, T, json!({ "n": "after" }));
    (a, vec![first, second])
}

#[test]
fn a_fork_in_a_compacted_range_is_still_detected() {
    // The receiver applied the first side; the chain runs through the
    // second.
    let (mut a, sides) = forked(2);
    let mut f = feed();
    seqs(&offer(&mut f, &a, &a.signed[0].clone()));
    seqs(&offer(&mut f, &a, &sides[0]));
    a.compact(T + 1, |_| false);
    let o = catch_up(&mut f, &a);
    assert!(matches!(o, Offer::Fork(Fork { at, held, .. }) if at == sides[0].pos() && held == sides[0].hash()), "{o:?}");
    assert!(f.frozen(&a.id(), NET).unwrap().is_some());

    // The receiver held the first side behind a gap.
    let (mut a, sides) = forked(3);
    let mut f = feed();
    seqs(&offer(&mut f, &a, &a.signed[0].clone()));
    assert!(matches!(offer(&mut f, &a, &sides[0]), Offer::Gap(_)));
    a.compact(T + 1, |_| false);
    assert!(matches!(catch_up(&mut f, &a), Offer::Fork(_)));

    // The first side comes after the receiver skipped its place: the
    // chain's id there tells it from a copy of the second.
    let (mut a, sides) = forked(2);
    a.compact(T + 1, |_| false);
    let mut f = feed();
    seqs(&offer_compacted(&mut f, &a, a.answer(Pos::ORIGIN).unwrap()));
    assert_eq!(offer(&mut f, &a, &sides[1]), Offer::Duplicate, "the side on the chain");
    assert!(matches!(offer(&mut f, &a, &sides[0]), Offer::Fork(_)), "the other side");

    // A live entry off the chain.
    let (mut a, sides) = forked(2);
    a.compact(T + 1, |e| e.seq == 2);
    let mut answer = a.answer(Pos::ORIGIN).unwrap();
    answer.live = vec![sides[0].clone()];
    assert!(matches!(offer_compacted(&mut feed(), &a, answer), Offer::Fork(_)));
}

#[test]
fn loose_entries_expire_and_are_remembered_past_it() {
    let mut a = Author::new();
    let msg = a.w.write(&a.k, Kind::Message, T, json!({ "text": "hi" })).unwrap();
    let mut f = Feed::at(MemoryStore::new(), T + RETENTION_MS + 1);
    assert_eq!(offer(&mut f, &a, &msg), Offer::Expired, "older than retention by its signed time");
    let mut f = Feed::at(MemoryStore::new(), T + RETENTION_MS);
    seqs(&offer(&mut f, &a, &msg));
    let store = f.into_store();
    // A replay a day later is refused for its age; the record is still
    // there until the dedupe period ends.
    let mut f = Feed::at(store, T + LOOSE_DEDUPE_MS - 1);
    assert_eq!(offer(&mut f, &a, &msg), Offer::Expired);
    let fresh = a.w.write(&a.k, Kind::Message, T + LOOSE_DEDUPE_MS - 2, json!({ "text": "later" })).unwrap();
    seqs(&offer(&mut f, &a, &fresh));
    assert_eq!(f.store().loose_count(), 2);
    let mut f = Feed::at(f.into_store(), T + LOOSE_DEDUPE_MS + 1);
    assert_eq!(offer(&mut f, &a, &fresh), Offer::Duplicate);
    assert_eq!(f.store().loose_count(), 1, "the first one's record is pruned");
}

/// What a receiver holds open, as an engine would: each item's claim, by
/// the place of the entry that opened it.
fn open_items(o: &Offer, items: &mut HashMap<u64, Pos>) {
    let Offer::New(a) = o else { return };
    for e in a {
        let item = e.body["item"].as_u64().unwrap_or(0);
        match e.body["op"].as_str() {
            Some("open") => {
                items.insert(item, e.pos());
            }
            Some("close") => {
                items.remove(&item);
            }
            _ => {}
        }
    }
    items.retain(|_, p| !a.ended.contains(p));
}

#[test]
fn compaction_keeps_the_live_set() {
    let mut runner = TestRunner::new(Config { cases: 256, ..Config::default() });
    let schedule = proptest::collection::vec(any::<prop::sample::Index>(), 0..40);
    runner
        .run(&(proptest::collection::vec(0u8..12, 1..40), schedule), |(ops, schedule)| {
            let mut a = Author::new();
            let mut open: HashMap<u64, Pos> = HashMap::new();
            let mut now = T;
            for op in ops {
                now += 1;
                let item = u64::from(op / 4);
                match op % 4 {
                    0 | 1 => {
                        let e = a.write(Kind::Claim, now, json!({ "op": "open", "item": item }));
                        open.insert(item, e.pos());
                    }
                    2 => {
                        a.write(Kind::Claim, now, json!({ "op": "close", "item": item }));
                        open.remove(&item);
                    }
                    _ if a.w.kept() > 0 => {
                        let places: Vec<Pos> = open.values().copied().collect();
                        a.compact(now, |e| places.contains(&e.pos()));
                    }
                    _ => {}
                }
            }
            // Some of everything the author ever signed, in any order, as
            // holders replay it; then the author's answer to the
            // receiver's ack, and what it holds past that.
            let mut f = feed();
            let mut items = HashMap::new();
            for i in schedule.iter().filter(|_| !a.signed.is_empty()) {
                let e = a.signed[i.index(a.signed.len())].clone();
                let o = offer(&mut f, &a, &e);
                prop_assert!(!matches!(o, Offer::Fork(_) | Offer::Frozen(_)), "an honest author's entry was {:?}", o);
                open_items(&o, &mut items);
            }
            if let Some(answer) = a.answer(after_ack(&f, &a)) {
                let o = offer_compacted(&mut f, &a, answer);
                prop_assert!(matches!(o, Offer::New(_) | Offer::Duplicate), "the answer was {:?}", o);
                open_items(&o, &mut items);
            }
            let ack = after_ack(&f, &a);
            for e in a.held.clone().iter().filter(|e| e.pos() > ack) {
                open_items(&offer(&mut f, &a, e), &mut items);
            }
            prop_assert_eq!(f.store().held_count(&a.id(), NET), 0);
            prop_assert_eq!(after_ack(&f, &a), a.w.head().at, "caught up");
            let mut want: Vec<(u64, Pos)> = open.into_iter().collect();
            let mut got: Vec<(u64, Pos)> = items.into_iter().collect();
            want.sort();
            got.sort();
            prop_assert_eq!(got, want, "the receiver holds open what the author does");
            for e in &a.held {
                prop_assert_eq!(f.store().kept(&a.id(), NET, e.pos()).unwrap(), Some(e.hash()), "every live entry is applied");
            }
            Ok(())
        })
        .unwrap();
}
