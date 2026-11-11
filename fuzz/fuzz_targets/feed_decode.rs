#![no_main]

// Feeds (xenonet-feed/1): the input's bytes read as an entry, then a fixed
// author's entries (kept and loose kinds, compact entries keeping nothing
// live, incarnations bumped by restores that lost nothing) offered to a
// receiver in the order the input picks, repeated, dropped, or with a
// field changed.
//
// Nothing may panic. An entry that decodes reads back as itself from its
// own bytes. A changed entry is taken only if its signature still checks,
// and then it says what one of the author's entries says. However the
// author's entries arrive, the receiver applies its kept entries in the
// author's order, each once, never sees a fork, and takes each loose one
// once.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_fuzz::net::{Choices, KINDS};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::feed::{Class, Feed, FeedEntry, MemoryStore, Offer, Writer};

const NET: &str = "net_0123456789abcdef0123456789abcdef";
const T: u64 = 1_792_512_000_000;
const FIELDS: [&str; 9] = ["author", "group", "inc", "seq", "prev", "kind", "at", "body", "sig"];

fn key() -> &'static SeedSigner {
    static KEY: OnceLock<SeedSigner> = OnceLock::new();
    KEY.get_or_init(|| SeedSigner::from_seed(&[0x46; 32], KeyHome::File).expect("a fixed seed is a key"))
}

/// The author's entries, as the input's choices script them.
fn script(c: &mut Choices<'_>) -> Vec<FeedEntry> {
    let k = key();
    let mut w = Writer::start(k.public().id(), NET, T);
    let mut now = T;
    let mut out = Vec::new();
    for n in 0..c.below(24) {
        now += 1 + u64::from(c.u8());
        match c.below(10) {
            8 => w = Writer::restore(k.public().id(), NET, w.head(), w.inc(), now),
            9 if w.kept() > 0 => out.push(w.compact(k, now, &[]).expect("the author's key signs")),
            i => out.push(w.write(k, KINDS[i % KINDS.len()], now, json!({ "n": n })).expect("the author's key signs")),
        }
    }
    out
}

/// `e` with one field changed or gone, read again.
fn changed(e: &FeedEntry, c: &mut Choices<'_>) -> Option<FeedEntry> {
    let mut v = e.to_json();
    let field = *c.pick(&FIELDS);
    let o = v.as_object_mut()?;
    match c.value() {
        Value::Null => {
            o.remove(field);
        }
        x => {
            o.insert(field.into(), x);
        }
    }
    FeedEntry::parse(&v).ok()
}

fuzz_target!(|data: &[u8]| {
    if let Ok(e) = FeedEntry::decode(data) {
        let again = FeedEntry::decode(&e.encode()).expect("an entry reads back from its own bytes");
        assert_eq!(again, e, "an entry reads back as another");
        assert_eq!(again.id(), e.id());
        assert!(e.well_formed().is_ok());
    }

    let mut c = Choices::new(data);
    let entries = script(&mut c);
    if entries.is_empty() {
        return;
    }
    let ids: Vec<String> = entries.iter().map(FeedEntry::id).collect();
    let kept: Vec<String> = entries.iter().filter(|e| e.class() == Class::Kept).map(FeedEntry::id).collect();
    let mut f = Feed::at(MemoryStore::new(), T);
    let (mut got_kept, mut got_loose) = (Vec::new(), Vec::new());
    for _ in 0..48 {
        if c.is_empty() {
            break;
        }
        let e = &entries[c.below(entries.len())];
        let e = if c.rarely(32) { changed(e, &mut c) } else { Some(e.clone()) };
        let Some(e) = e else { continue };
        let Ok(v) = e.verify(&key().public()) else { continue };
        assert!(ids.contains(&v.id()), "a changed entry checked: {:?}", v.entry());
        match f.offer(v).expect("memory doesn't fail") {
            Offer::New(list) => {
                for x in list {
                    match x.class() {
                        Class::Kept => got_kept.push(x.id()),
                        Class::Loose => {
                            assert!(!got_loose.contains(&x.id()), "a loose entry taken twice");
                            got_loose.push(x.id());
                        }
                    }
                }
                assert!(kept.starts_with(&got_kept), "applied out of the author's order");
            }
            Offer::Duplicate | Offer::Gap(_) => {}
            other => panic!("an honest author's entry was {other:?}"),
        }
    }
});
