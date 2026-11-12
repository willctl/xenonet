// Known answers for Xenolith's kinds in a space's feed
// (test/fixtures/log/ctx.json): a fixed context key, a fixed space and fixed
// times, one entry of each `ctx.*` kind in the shapes Xenolith's sync/2
// writes them (xenolith/rust/crates/core/src/syncfeed.rs). A text a body
// names goes beside the entry, by its SHA-256, so the entry's id doesn't
// change when a holder drops the text (a forget).
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-log --test ctx
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use xenonet_id::codec::{b64, sha256, unb64_n};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::feed::{live_set, Feed, FeedEntry, Group, Header, Kind, MemoryStore, Offer, Retention, Writer};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/log/ctx.json");
const SPACE: &str = "spc_abcdefghijklmnopqrstuvwxyz234567";
const T: u64 = 1_792_512_345_678;

fn seed() -> [u8; 32] {
    let mut s = [0x0d; 32];
    s[0] = 0x5c;
    s
}

fn text(t: &str) -> Value {
    json!({ "sha256": b64(&sha256(t.as_bytes())), "bytes": t.len() })
}

fn entries(k: &SeedSigner) -> Vec<(&'static str, FeedEntry)> {
    let mut w = Writer::start(k.public().id(), SPACE, T);
    let page = "---\ntitle: Atlas database\n---\n\n# Atlas database\n\nPostgres 16.\n";
    let day = "# 2026-11-11\n\n- 09:00 · curator · Filed [[db]]\n";
    let mut e = |kind, n: u64, body| w.write(k, kind, T + n, body).unwrap();
    vec![
        ("a note for the curator", e(Kind::CtxNote, 1, json!({ "id": "2026-11-11_09-00-00-000-abc123", "kind": "log", "app": "codex", "source": "agent", "title": "Atlas moves to Postgres 17", "tags": [], "pages": ["db"], "text": text("Logical replication.") }))),
        ("a replace against the version the member had", e(Kind::CtxPropose, 2, json!({ "slug": "db", "mode": "replace", "base": "0123456789abcdef", "title": "Atlas database", "text": text(page) }))),
        ("a page's version", e(Kind::CtxPage, 3, json!({ "slug": "db", "epoch": 0, "n": 1, "base": "none", "version": "0123456789abcdef", "text": text(page) }))),
        ("a page removed", e(Kind::CtxRemove, 4, json!({ "slug": "old", "epoch": 0, "base": "fedcba9876543210" }))),
        ("a day of the log", e(Kind::CtxLog, 5, json!({ "date": "2026-11-11", "epoch": 0, "version": "00112233aabbccdd", "text": text(day) }))),
        ("texts dropped", e(Kind::CtxForget, 6, json!({ "epoch": 0, "hashes": [b64(&sha256(page.as_bytes()))] }))),
        ("the live pages", e(Kind::CtxSnapshot, 7, json!({ "epoch": 0, "pages": [{ "slug": "db", "version": "0123456789abcdef", "sha256": b64(&sha256(page.as_bytes())) }] }))),
    ]
}

fn answers() -> Value {
    let k = SeedSigner::from_seed(&seed(), KeyHome::File).unwrap();
    json!({
        "about": "Known answers for Xenolith's kinds in a space's feed (crates/xenonet-log/src/feed.rs): written by crates/xenonet-log/tests/ctx.rs. The seed is a test value, never a Xenolith's.",
        "group": SPACE,
        "author": { "seed": b64(&seed()), "keyId": k.public().id() },
        "entries": entries(&k).into_iter().map(|(what, e)| json!({
            "what": what,
            "signingBytes": String::from_utf8(e.signing_bytes()).unwrap(),
            "id": e.id(),
            "header": e.header().to_json(),
            "entry": e.to_json(),
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn xenoliths_kinds_give_their_known_answers() {
    let got = answers();
    if std::env::var("XENONET_WRITE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(FILE, format!("{}\n", serde_json::to_string_pretty(&got).unwrap())).unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want, "the known answers changed: if that's meant, write them again (see the top of this file)");
}

#[test]
fn the_ctx_fixtures_check_from_their_own_values_and_stay_for_good() {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let k = SeedSigner::from_seed(&unb64_n::<32>(v["author"]["seed"].as_str().unwrap()).unwrap(), KeyHome::File).unwrap();
    assert_eq!(k.public().id(), v["author"]["keyId"]);
    assert_eq!(Group::of(v["group"].as_str().unwrap()), Some(Group::Space));
    let mut f = Feed::at(MemoryStore::new(), T + 10);
    let mut held = Vec::new();
    for x in v["entries"].as_array().unwrap() {
        let e = FeedEntry::decode(x["entry"].to_string().as_bytes()).unwrap();
        assert_eq!(e.signing_bytes(), x["signingBytes"].as_str().unwrap().as_bytes());
        assert_eq!(e.id(), x["id"]);
        let h = Header::parse(&x["header"]).unwrap();
        assert_eq!(h.id(), e.id());
        assert!(e.kind.is_ctx() && Kind::parse(e.kind.as_str()) == Some(e.kind), "{}", x["what"]);
        assert_eq!(e.kind.retention(), Retention::Forever, "{}", x["what"]);
        assert!(!e.kind.ends_alone(), "never ended by a compact entry's absence");
        let Offer::New(applied) = f.offer(e.clone().verify(&k.public()).unwrap()).unwrap() else { panic!("{} wasn't taken", x["what"]) };
        assert_eq!(applied.entries, std::slice::from_ref(&e));
        held.push(e);
    }
    // In order, one after another, and every one stays live: nothing of a
    // space's feed is compacted away, however old.
    let inc = held[0].inc;
    assert_eq!(live_set(&held, inc, u64::MAX, |_| false).unwrap(), (1..=7).collect::<Vec<u64>>());
    // Every kind there is reads back, and XenoNet's are none of these.
    for kind in Kind::ALL {
        assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        assert_eq!(kind.is_ctx(), kind.as_str().starts_with("ctx."));
    }
}
