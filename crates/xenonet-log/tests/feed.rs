// Known answers for feeds (test/fixtures/log/feed.json): a fixed machine
// signing key and fixed times, so another implementation can check its
// canonical bytes, entry ids and signatures against these, byte for byte.
// Ed25519 signs deterministically, so all of it is reproducible. The
// entries are one author's: a claim that starts its feed, an intent, a
// message (loose, counted on its own), and the first entry of the
// incarnation after a restore, naming the last kept entry it still had.
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-log --test feed
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use xenonet_id::codec::{b64, unb64_n};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::feed::{Feed, FeedEntry, Kind, MemoryStore, Offer, Pos, Writer};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/log/feed.json");
const NET: &str = "net_00112233445566778899aabbccddeeff";
const T: u64 = 1_792_512_345_678;

fn seed() -> [u8; 32] {
    let mut s = [0x0c; 32];
    s[0] = 0x4c;
    s
}

fn entries(k: &SeedSigner) -> Vec<(&'static str, FeedEntry)> {
    let mut w = Writer::start(k.public().id(), NET, T);
    let claim = w.write(k, Kind::Claim, T + 1, json!({ "project": "prj_atlas", "path": "src/api.rs", "note": "the route" })).unwrap();
    let intent = w.write(k, Kind::Intent, T + 2, json!({ "op": "make", "goal": "fix the build" })).unwrap();
    let message = w.write(k, Kind::Message, T + 3, json!({ "to": "laptop", "text": "héllo \"there\"" })).unwrap();
    let mut r = Writer::restore(k.public().id(), NET, Pos::new(T, 1), w.inc(), T + 10);
    let restored = r.write(k, Kind::Report, T + 11, json!({ "op": "note", "text": "back" })).unwrap();
    vec![("the feed's first entry", claim), ("a second kept entry", intent), ("a loose entry", message), ("the first after a restore", restored)]
}

fn answers() -> Value {
    let k = SeedSigner::from_seed(&seed(), KeyHome::File).unwrap();
    json!({
        "about": "Known answers for a machine's feed (crates/xenonet-log/src/feed.rs): written by crates/xenonet-log/tests/feed.rs. The seed is a test value, never a machine's.",
        "network": NET,
        "author": { "seed": b64(&seed()), "keyId": k.public().id() },
        "entries": entries(&k).into_iter().map(|(what, e)| json!({
            "what": what,
            "signingBytes": String::from_utf8(e.signing_bytes()).unwrap(),
            "id": e.id(),
            "entry": e.to_json(),
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn a_feed_gives_its_known_answers() {
    let got = answers();
    if std::env::var("XENONET_WRITE_FIXTURES").as_deref() == Ok("1") {
        std::fs::create_dir_all(std::path::Path::new(FILE).parent().unwrap()).unwrap();
        std::fs::write(FILE, format!("{}\n", serde_json::to_string_pretty(&got).unwrap())).unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want, "the known answers changed: if that's meant, write them again (see the top of this file)");
}

#[test]
fn the_fixtures_check_from_their_own_values() {
    // What another implementation would do: rebuild the key from its
    // seed, read each entry from its bytes, and check it.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let k = SeedSigner::from_seed(&unb64_n::<32>(v["author"]["seed"].as_str().unwrap()).unwrap(), KeyHome::File).unwrap();
    assert_eq!(k.public().id(), v["author"]["keyId"]);
    let mut f = Feed::new(MemoryStore::new());
    for x in v["entries"].as_array().unwrap() {
        let e = FeedEntry::decode(x["entry"].to_string().as_bytes()).unwrap();
        assert_eq!(e.signing_bytes(), x["signingBytes"].as_str().unwrap().as_bytes());
        assert_eq!(e.id(), x["id"]);
        assert_eq!(e.to_json(), x["entry"]);
        let offered = f.offer(e.clone().verify(&k.public()).unwrap()).unwrap();
        assert_eq!(offered, Offer::New(vec![e]), "{}", x["what"]);
    }
}
