//! A feed's rules: entries read back and check, gaps hold and fill, forks
//! freeze until the person unfreezes, incarnations and restores, and,
//! under random deliveries, every receiver applying the same sequence
//! exactly once.

use super::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use serde_json::json;
use xenonet_id::sign::{KeyHome, SeedSigner};

const NET: &str = "net_00112233445566778899aabbccddeeff";
const T: u64 = 1_792_512_000_000;

fn key(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

fn writer(k: &SeedSigner) -> Writer {
    Writer::start(k.public().id(), NET, T)
}

fn write(w: &mut Writer, k: &SeedSigner, kind: Kind, n: u64) -> FeedEntry {
    w.write(k, kind, T + n, json!({ "n": n })).unwrap()
}

fn offer(f: &mut Feed<MemoryStore>, k: &SeedSigner, e: &FeedEntry) -> Offer {
    f.offer(e.clone().verify(&k.public()).unwrap()).unwrap()
}

fn seqs(o: &Offer) -> Vec<u64> {
    match o {
        Offer::New(v) => v.iter().map(|e| e.seq).collect(),
        other => panic!("not taken: {other:?}"),
    }
}

/// A P-256 key, standing in for a Secure Enclave's: it signs differently
/// each time.
struct P256(ring::signature::EcdsaKeyPair);

impl P256 {
    fn new() -> P256 {
        let rng = ring::rand::SystemRandom::new();
        let alg = &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
        let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        P256(ring::signature::EcdsaKeyPair::from_pkcs8(alg, doc.as_ref(), &rng).unwrap())
    }
}

impl Signer for P256 {
    fn public(&self) -> VerifyingKey {
        use ring::signature::KeyPair as _;
        VerifyingKey::P256(self.0.public_key().as_ref().to_vec())
    }
    fn home(&self) -> KeyHome {
        KeyHome::Hardware
    }
    fn sign(&self, purpose: Purpose, msg: &[u8]) -> xenonet_id::R<Vec<u8>> {
        Ok(self.0.sign(&ring::rand::SystemRandom::new(), &purpose.bytes(msg)).unwrap().as_ref().to_vec())
    }
}

#[test]
fn an_entry_reads_back_and_checks_only_against_its_author() {
    let k = key(1);
    let mut w = writer(&k);
    let e = write(&mut w, &k, Kind::Claim, 1);
    assert_eq!((e.inc, e.seq, e.prev), (T, 1, Some(Pos::ORIGIN)));
    assert_eq!(FeedEntry::decode(&e.encode()).unwrap(), e);
    assert!(e.signing_bytes().starts_with(b"xenonet-feed/1\n"));
    assert!(e.clone().verify(&k.public()).is_ok());
    assert_eq!(e.clone().verify(&key(2).public()).unwrap_err().code, "refused", "another key");
    let mut changed = e.clone();
    changed.body = json!({ "n": 2 });
    assert_eq!(changed.verify(&k.public()).unwrap_err().code, "refused", "a changed body");
    let mut other = e.clone();
    assert!(other.sign(&key(2)).is_err(), "signed by a key that isn't the author's");
    // A key that fails leaves no hole in the sequence.
    let mut bad = Writer::start(key(2).public().id(), NET, T);
    assert!(bad.write(&k, Kind::Claim, T, json!({})).is_err());
    assert_eq!(bad.write(&key(2), Kind::Claim, T, json!({})).unwrap().seq, 1);
}

#[test]
fn an_entry_holds_to_its_shape() {
    let k = key(1);
    let mut w = writer(&k);
    let first = write(&mut w, &k, Kind::Claim, 1);
    let second = write(&mut w, &k, Kind::Claim, 2);
    let loose = write(&mut w, &k, Kind::Message, 3);
    assert_eq!((loose.seq, loose.prev), (1, None), "a loose kind counts on its own");
    let bad = |e: &FeedEntry| FeedEntry::parse(&e.to_json()).is_err() && e.clone().verify(&k.public()).is_err();
    let with = |e: &FeedEntry, f: &dyn Fn(&mut FeedEntry)| {
        let mut e = e.clone();
        f(&mut e);
        e
    };
    assert!(bad(&with(&first, &|e| e.prev = None)), "a first kept entry names prev");
    assert!(bad(&with(&second, &|e| e.prev = Some(Pos::ORIGIN))), "only the first does");
    assert!(bad(&with(&loose, &|e| e.prev = Some(Pos::ORIGIN))), "a loose one never does");
    assert!(bad(&with(&first, &|e| e.prev = Some(Pos::new(T, 4)))), "prev is before its incarnation");
    assert!(bad(&with(&first, &|e| e.prev = Some(Pos::new(5, 0)))), "an incarnation's place 0 names nothing");
    assert!(bad(&with(&first, &|e| e.prev = Some(Pos::new(0, 3)))), "nor does the origin's");
    assert!(bad(&with(&first, &|e| e.inc = 0)));
    assert!(bad(&with(&second, &|e| e.seq = 0)));
    assert!(bad(&with(&first, &|e| e.body = json!([1]))), "a body is an object");
    assert!(bad(&with(&first, &|e| e.author = "studio".into())));
    assert!(bad(&with(&first, &|e| e.network = "net_x".into())));
    assert!(bad(&with(&first, &|e| e.sig.truncate(10))));
    assert!(bad(&with(&first, &|e| e.body = json!({ "text": "x".repeat(MAX_ENTRY) }))), "an entry fits in a frame");
    assert!(FeedEntry::decode(&vec![b' '; MAX_ENTRY + 1]).is_err());
    let mut v = first.to_json();
    v["kind"] = json!("gossip");
    assert!(FeedEntry::parse(&v).is_err(), "an unknown kind");
}

#[test]
fn a_gap_holds_entries_until_it_fills() {
    let k = key(1);
    let mut w = writer(&k);
    let e: Vec<_> = (1..=4).map(|n| write(&mut w, &k, Kind::Intent, n)).collect();
    let mut f = Feed::new(MemoryStore::new());
    let author = k.public().id();
    assert_eq!(f.ack(&author, NET).unwrap(), None);
    assert_eq!(offer(&mut f, &k, &e[2]), Offer::Gap(Want { author: author.clone(), network: NET.into(), inc: 0, from: 1 }), "nothing yet: from the first");
    assert_eq!(seqs(&offer(&mut f, &k, &e[0])), [1]);
    let want = Want { author: author.clone(), network: NET.into(), inc: T, from: 2 };
    assert_eq!(offer(&mut f, &k, &e[3]), Offer::Gap(want.clone()));
    assert!(want.wants(e[1].pos()) && !want.wants(e[0].pos()));
    assert_eq!(offer(&mut f, &k, &e[2]), Offer::Duplicate, "held already");
    assert_eq!(f.store().held_count(&author, NET), 2);
    assert_eq!(f.ack(&author, NET).unwrap(), Some(Ack { inc: T, high: 1 }), "acks are contiguous");
    assert_eq!(seqs(&offer(&mut f, &k, &e[1])), [2, 3, 4]);
    assert_eq!(f.store().held_count(&author, NET), 0);
    assert_eq!(f.ack(&author, NET).unwrap(), Some(Ack { inc: T, high: 4 }));
    assert_eq!(offer(&mut f, &k, &e[1]), Offer::Duplicate);
    assert!(!Ack { inc: T, high: 4 }.unacked(e[3].pos()));
}

#[test]
fn loose_entries_are_taken_once_and_wait_for_nothing() {
    let k = key(1);
    let mut w = writer(&k);
    let _lost = write(&mut w, &k, Kind::Claim, 1);
    let claim = write(&mut w, &k, Kind::Claim, 2);
    let msg = write(&mut w, &k, Kind::Message, 3);
    let receipt = write(&mut w, &k, Kind::Receipt, 4);
    let mut f = Feed::new(MemoryStore::new());
    assert!(matches!(offer(&mut f, &k, &claim), Offer::Gap(_)));
    assert_eq!(seqs(&offer(&mut f, &k, &receipt)), [2], "not behind the claim's gap");
    assert_eq!(seqs(&offer(&mut f, &k, &msg)), [1]);
    assert_eq!(offer(&mut f, &k, &msg), Offer::Duplicate);
}

#[test]
fn a_copy_signed_again_is_a_duplicate() {
    let k = P256::new();
    let mut w = Writer::start(k.public().id(), NET, T);
    let e = w.write(&k, Kind::Handoff, T, json!({ "to": "laptop" })).unwrap();
    let mut again = e.clone();
    again.sign(&k).unwrap();
    assert_ne!(again.sig, e.sig, "P-256 signs differently each time");
    assert_eq!(again.id(), e.id());
    let mut f = Feed::new(MemoryStore::new());
    assert!(matches!(f.offer(e.verify(&k.public()).unwrap()).unwrap(), Offer::New(_)));
    assert_eq!(f.offer(again.verify(&k.public()).unwrap()).unwrap(), Offer::Duplicate);
}

#[test]
fn a_fork_freezes_the_feed_until_the_person_unfreezes_it() {
    let k = key(1);
    let author = k.public().id();
    let mut w = writer(&k);
    let one = write(&mut w, &k, Kind::Claim, 1);
    let mut w2 = w.clone();
    let two = write(&mut w, &k, Kind::Claim, 2);
    let other = w2.write(&k, Kind::Claim, T, json!({ "n": "something else" })).unwrap();
    let three = write(&mut w, &k, Kind::Claim, 3);
    let four = write(&mut w, &k, Kind::Claim, 4);
    let mut f = Feed::new(MemoryStore::new());
    seqs(&offer(&mut f, &k, &one));
    seqs(&offer(&mut f, &k, &two));
    assert!(matches!(offer(&mut f, &k, &four), Offer::Gap(_)));
    let fork = Fork { at: two.pos(), held: two.hash(), offered: other.hash() };
    assert_eq!(offer(&mut f, &k, &other), Offer::Fork(fork));
    assert_eq!(f.frozen(&author, NET).unwrap(), Some(fork));
    assert_eq!(offer(&mut f, &k, &three), Offer::Frozen(fork), "nothing is taken while frozen");
    let msg = write(&mut w, &k, Kind::Message, 5);
    assert_eq!(offer(&mut f, &k, &msg), Offer::Frozen(fork), "loose entries neither");
    assert!(f.unfreeze(&author, NET).unwrap().is_empty(), "three is still missing");
    assert_eq!(f.frozen(&author, NET).unwrap(), None);
    assert_eq!(seqs(&offer(&mut f, &k, &three)), [3, 4], "the receiver kept its own two; four was held throughout");
    assert_eq!(offer(&mut f, &k, &two), Offer::Duplicate);
}

#[test]
fn two_entries_held_at_one_place_are_a_fork() {
    let k = key(1);
    let mut w = writer(&k);
    let _one = write(&mut w, &k, Kind::Claim, 1);
    let mut w2 = w.clone();
    let a = write(&mut w, &k, Kind::Claim, 2);
    let b = w2.write(&k, Kind::Claim, T, json!({ "other": true })).unwrap();
    let mut f = Feed::new(MemoryStore::new());
    assert!(matches!(offer(&mut f, &k, &a), Offer::Gap(_)));
    assert!(matches!(offer(&mut f, &k, &b), Offer::Fork(Fork { held, .. }) if held == a.hash()));
}

#[test]
fn an_incarnation_increases() {
    assert_eq!(next_incarnation(0, T), T);
    assert_eq!(next_incarnation(T, T), T + 1, "the clock didn't move");
    assert_eq!(next_incarnation(T + 50, T), T + 51, "the clock went back");
    assert_eq!(next_incarnation(T, T + 9), T + 9);
    let k = key(1);
    let mut w = writer(&k);
    write(&mut w, &k, Kind::Claim, 1);
    write(&mut w, &k, Kind::Message, 2);
    assert_eq!(w.head(), Pos::new(T, 1), "the head is the last kept entry");
    let mut r = Writer::restore(k.public().id(), NET, w.head(), w.inc(), T - 1000);
    assert_eq!(r.inc(), T + 1);
    assert_eq!(r.head(), Pos::new(T, 1));
    let first = write(&mut r, &k, Kind::Assign, 3);
    assert_eq!((first.inc, first.seq, first.prev), (T + 1, 1, Some(Pos::new(T, 1))));
    assert_eq!(write(&mut r, &k, Kind::Message, 4).seq, 1);
    assert_eq!(write(&mut r, &k, Kind::Report, 5).prev, None);
    // After a restart, the same incarnation goes on.
    let mut again = Writer::resume(k.public().id(), NET, r.inc(), 2, 1, Pos::ORIGIN);
    assert_eq!(write(&mut again, &k, Kind::Report, 6).pos(), Pos::new(T + 1, 3));
    let mut fresh = Writer::resume(k.public().id(), NET, T + 7, 0, 0, Pos::new(T, 1));
    assert_eq!(write(&mut fresh, &k, Kind::Report, 7).prev, Some(Pos::new(T, 1)));
}

#[test]
fn a_restore_links_below_what_the_receiver_holds() {
    let k = key(1);
    let author = k.public().id();
    let mut w = writer(&k);
    let old: Vec<_> = (1..=6).map(|n| write(&mut w, &k, Kind::Claim, n)).collect();
    let mut f = Feed::new(MemoryStore::new());
    for e in &old[..5] {
        seqs(&offer(&mut f, &k, e));
    }
    // The author's store came back from a backup taken after its third,
    // so it lost four to six; the receiver has four and five.
    let mut r = Writer::restore(author.clone(), NET, Pos::new(T, 3), T, T + 10);
    let first = write(&mut r, &k, Kind::Claim, 7);
    let second = write(&mut r, &k, Kind::Claim, 8);
    assert!(matches!(offer(&mut f, &k, &second), Offer::Gap(Want { inc: T, from: 6, .. })));
    assert_eq!(seqs(&offer(&mut f, &k, &first)), [1, 2], "a restore, not a fork");
    assert_eq!(f.frozen(&author, NET).unwrap(), None);
    assert_eq!(offer(&mut f, &k, &old[5]), Offer::Stale, "six is history the author gave up");
    assert_eq!(offer(&mut f, &k, &old[3]), Offer::Duplicate, "what the receiver applied stays");
    assert_eq!(f.ack(&author, NET).unwrap(), Some(Ack { inc: T + 10, high: 2 }));
}

#[test]
fn a_prev_above_what_the_receiver_holds_is_a_gap() {
    let k = key(1);
    let author = k.public().id();
    let mut w = writer(&k);
    let old: Vec<_> = (1..=4).map(|n| write(&mut w, &k, Kind::Claim, n)).collect();
    let mut r = Writer::restore(author.clone(), NET, w.head(), w.inc(), T + 10);
    let next = write(&mut r, &k, Kind::Claim, 5);
    let mut f = Feed::new(MemoryStore::new());
    seqs(&offer(&mut f, &k, &old[0]));
    seqs(&offer(&mut f, &k, &old[1]));
    assert!(matches!(offer(&mut f, &k, &old[3]), Offer::Gap(_)));
    assert_eq!(offer(&mut f, &k, &next), Offer::Gap(Want { author: author.clone(), network: NET.into(), inc: T, from: 3 }));
    assert_eq!(seqs(&offer(&mut f, &k, &old[2])), [3, 4, 1], "three, four, then the next incarnation");
    assert_eq!(f.store().held_count(&author, NET), 0);
}

#[test]
fn linking_a_later_incarnation_closes_the_ones_before() {
    let k = key(1);
    let author = k.public().id();
    let mut w = writer(&k);
    let old: Vec<_> = (1..=4).map(|n| write(&mut w, &k, Kind::Claim, n)).collect();
    let mut f = Feed::new(MemoryStore::new());
    seqs(&offer(&mut f, &k, &old[0]));
    assert!(matches!(offer(&mut f, &k, &old[3]), Offer::Gap(_)));
    // Restored from right after the first: two to four are gone.
    let mut r = Writer::restore(author.clone(), NET, Pos::new(T, 1), T, T + 10);
    let first = write(&mut r, &k, Kind::Claim, 5);
    assert_eq!(seqs(&offer(&mut f, &k, &first)), [1]);
    assert_eq!(f.store().held_count(&author, NET), 0, "four can't apply any more");
    assert_eq!(offer(&mut f, &k, &old[1]), Offer::Stale);
    // An incarnation below the receiver's that it never linked.
    let mut between = Writer::restore(author.clone(), NET, Pos::new(T, 1), T, T + 5);
    assert_eq!(offer(&mut f, &k, &write(&mut between, &k, Kind::Claim, 6)), Offer::Stale);
}

#[test]
fn feeds_are_apart_per_author_and_network() {
    let (a, b) = (key(1), key(2));
    let mut wa = writer(&a);
    let mut wb = writer(&b);
    let mut wn = Writer::start(a.public().id(), "net_ffffffffffffffffffffffffffffffff", T);
    let mut f = Feed::new(MemoryStore::new());
    for n in 1..=2 {
        seqs(&offer(&mut f, &a, &write(&mut wa, &a, Kind::Claim, n)));
        seqs(&offer(&mut f, &b, &write(&mut wb, &b, Kind::Claim, n)));
        seqs(&offer(&mut f, &a, &write(&mut wn, &a, Kind::Claim, n)));
    }
    assert_eq!(f.ack(&a.public().id(), NET).unwrap(), Some(Ack { inc: T, high: 2 }));
}

/// An author's script: kept kinds, loose kinds, and incarnations bumped
/// with nothing lost (a backup restored right after it was taken).
fn script(ops: &[u8]) -> (SeedSigner, Vec<FeedEntry>) {
    let k = key(9);
    let mut w = writer(&k);
    let mut out = Vec::new();
    let mut now = T;
    for (n, op) in ops.iter().enumerate() {
        now += 1;
        match op % 8 {
            7 => w = Writer::restore(k.public().id(), NET, w.head(), w.inc(), now),
            i => out.push(w.write(&k, Kind::ALL[usize::from(i)], now, json!({ "n": n })).unwrap()),
        }
    }
    (k, out)
}

/// Delivers `schedule`'s picks from `entries` to a new receiver, then has
/// the author reconnect: it resends every kept entry past the receiver's
/// ack, in order, and the gateway's queue gives every loose one again.
/// Returns the kept entries applied, in order, and the loose ones.
fn deliver(k: &SeedSigner, entries: &[FeedEntry], schedule: &[prop::sample::Index]) -> Result<(Vec<String>, Vec<String>), TestCaseError> {
    let kept_ids: Vec<String> = entries.iter().filter(|e| e.class() == Class::Kept).map(FeedEntry::id).collect();
    let mut f = Feed::new(MemoryStore::new());
    let (mut kept, mut loose) = (Vec::new(), Vec::new());
    let mut take = |f: &mut Feed<MemoryStore>, e: &FeedEntry| -> Result<(), TestCaseError> {
        match f.offer(e.clone().verify(&k.public()).unwrap()).unwrap() {
            Offer::New(v) => {
                for x in v {
                    match x.class() {
                        Class::Kept => kept.push(x.id()),
                        Class::Loose => loose.push(x.id()),
                    }
                }
                prop_assert!(kept_ids.starts_with(&kept), "applied out of the author's order");
            }
            Offer::Duplicate | Offer::Gap(_) => {}
            other => prop_assert!(false, "an honest author's entry was {other:?}"),
        }
        Ok(())
    };
    if !entries.is_empty() {
        for i in schedule {
            take(&mut f, &entries[i.index(entries.len())])?;
        }
    }
    let author = k.public().id();
    let ack = f.ack(&author, NET).unwrap();
    for e in entries.iter().filter(|e| e.class() == Class::Kept && ack.is_none_or(|a| a.unacked(e.pos()))) {
        take(&mut f, e)?;
    }
    for e in entries.iter().filter(|e| e.class() == Class::Loose) {
        take(&mut f, e)?;
    }
    prop_assert_eq!(f.store().held_count(&author, NET), 0);
    Ok((kept, loose))
}

#[test]
fn every_receiver_applies_the_same_sequence_exactly_once() {
    let mut runner = TestRunner::new(Config { cases: 256, ..Config::default() });
    let schedule = || proptest::collection::vec(any::<prop::sample::Index>(), 0..80);
    runner
        .run(&(proptest::collection::vec(0u8..8, 0..40), schedule(), schedule()), |(ops, one, two)| {
            let (k, entries) = script(&ops);
            let kept: Vec<String> = entries.iter().filter(|e| e.class() == Class::Kept).map(FeedEntry::id).collect();
            let mut loose: Vec<String> = entries.iter().filter(|e| e.class() == Class::Loose).map(FeedEntry::id).collect();
            loose.sort();
            for schedule in [&one, &two] {
                let (got_kept, mut got_loose) = deliver(&k, &entries, schedule)?;
                prop_assert_eq!(&got_kept, &kept, "the author's kept entries, in order, each once");
                got_loose.sort();
                prop_assert_eq!(&got_loose, &loose, "each loose entry once");
            }
            Ok(())
        })
        .unwrap();
}
