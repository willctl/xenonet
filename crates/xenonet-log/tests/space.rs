// Known answers for a space's chain and its side entries
// (test/fixtures/log/space.json): fixed keys, nonces and times, so another
// implementation (Xenolith's tests, a second reader of the format) can check
// its canonical bytes, ids and signatures against these, byte for byte, and
// the view they add up to. A person with two devices owns a space about
// themselves, founded by curator c1; they share it with a peer's key at
// append and with a network at read; from the second device they unshare
// the network; then they take the space to curator c2, which sequences a
// share again that lists the unshare as seen.
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-log --test space
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_id::codec::b64;
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer};
use xenonet_log::person::{self, DeviceKind, NewDevice, PersonChain};
use xenonet_log::space::{found, Entry, Level, Op, Owner, Side, Space, SpacePolicy, Subject, WIDEN};
use xenonet_model::ids::{EntityRef, SpaceId};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/log/space.json");
const T: i64 = 1_792_512_000_000;
const NET: &str = "net_00112233445566778899aabbccddeeff";

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn signer(byte: u8) -> SeedSigner {
    SeedSigner::from_seed(&secret(byte), KeyHome::File).unwrap()
}

fn block(device: &SeedSigner, what: &str, act: [u8; 32], nonce: u8, at: i64) -> PresenceBlock {
    let mut b = PresenceBlock { device: device.public().id(), what: what.into(), act, nonce: [nonce; 16], at, sig: Vec::new() };
    b.sig = device.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
    b
}

fn person(root: &SeedSigner, devices: &[&SeedSigner]) -> PersonChain {
    let mut c = PersonChain::new();
    c.offer(person::found_salted(root, [0x5a; person::SALT_BYTES], T - 1000).unwrap()).unwrap();
    for (i, d) in devices.iter().enumerate() {
        let at = T - 900 + i as i64;
        let mut add = c.next(person::Body { op: person::Op::AddDevice(NewDevice { key: d.public(), kind: DeviceKind::Weaker, label: format!("device {i}") }), at });
        let b = block(d, person::PAIR, add.digest(), 0x11 + i as u8, at);
        add.sign(&root.public().id(), root, Some(b)).unwrap();
        c.offer(add).unwrap();
    }
    c
}

fn entry_json(what: &str, e: &Entry) -> Value {
    json!({ "what": what, "signingBytes": String::from_utf8(e.signing_bytes()).unwrap(), "id": e.id(), "hash": e.hash(), "entry": e.to_json() })
}

fn side_json(what: &str, s: &Side) -> Value {
    json!({ "what": what, "signingBytes": String::from_utf8(s.signing_bytes()).unwrap(), "id": s.id(), "side": s.to_json() })
}

fn answers() -> Value {
    let (d1, d2) = (signer(2), signer(3));
    let owner = person(&signer(1), &[&d1, &d2]);
    let pid = owner.id().unwrap().to_string();
    let head = owner.head();
    let (c1, c2, peer) = (signer(10), signer(11), signer(12));
    let policy = SpacePolicy::new(Some(Arc::new(owner.clone())), None);
    let genesis = found(EntityRef::parse(&pid).unwrap(), Owner::Person(pid.clone()), &c1, None, "will", [0x77; 16], T).unwrap();
    let mut sp = Space::found(policy, genesis.clone()).unwrap();
    let mut entries = vec![entry_json("found", &genesis)];
    let mut sides = Vec::new();
    // The owner's device signs with its presence, the curator sequences.
    let mut sequence = |sp: &mut Space, what: &str, op: Op, curator: &SeedSigner, nonce: u8| {
        let mut e = sp.next(op, head.clone(), T + sp.len() as i64);
        let b = block(&d1, WIDEN, e.digest(), nonce, e.at);
        e.sign(&d1.public().id(), &d1, Some(b)).unwrap();
        e.sign(&curator.public().id(), curator, None).unwrap();
        sp.offer(e.clone()).unwrap();
        entries.push(entry_json(what, &e));
    };
    let key = Subject::Key(peer.public().id());
    let net = Subject::Network(NET.into());
    sequence(&mut sp, "share with a peer's key at append", Op::Share { subject: key.clone(), level: Level::Append, endpoint: Some("ab".repeat(32)), after: vec![] }, &c1, 0x21);
    sequence(&mut sp, "share with a network at read", Op::Share { subject: net.clone(), level: Level::Read, endpoint: None, after: vec![] }, &c1, 0x22);
    // From the second device, alone, no presence: an unshare narrows.
    let mut un = sp.unshare(net.clone(), head.clone(), T + 10);
    un.sign(&d2.public().id(), &d2, None).unwrap();
    sp.offer_side(un.clone()).unwrap();
    sides.push(side_json("unshare the network", &un));
    // The owner takes the space to c2, with presence.
    let mut take = sp.curator_side(true, c2.public(), head.clone(), T + 20);
    let b = block(&d1, WIDEN, take.digest(), 0x23, take.at);
    take.sign(&d1.public().id(), &d1, Some(b)).unwrap();
    sp.offer_side(take.clone()).unwrap();
    sides.push(side_json("take to c2", &take));
    sequence(&mut sp, "c2 shares with the network again", Op::Share { subject: net.clone(), level: Level::Read, endpoint: None, after: vec![un.id()] }, &c2, 0x24);
    let v = sp.view();
    json!({
        "about": "Known answers for a space's chain and side entries (crates/xenonet-log/src/space.rs): written by crates/xenonet-log/tests/space.rs. Seeds are test values, never a machine's.",
        "person": owner.to_json(),
        "seeds": { "devices": [b64(&secret(2)), b64(&secret(3))], "curators": [b64(&secret(10)), b64(&secret(11))], "peer": b64(&secret(12)) },
        "space": sp.id(),
        "entries": entries,
        "sides": sides,
        "view": {
            "about": v.about.to_string(),
            "owner": pid,
            "curator": v.curator.id(),
            "epoch": v.epoch,
            "head": sp.head(),
            "levels": { key.to_string(): sp.level(&key).map(Level::as_str), net.to_string(): sp.level(&net).map(Level::as_str) },
        },
    })
}

#[test]
fn the_space_chain_gives_its_known_answers() {
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
    // What another implementation would do: read the person's chain, the
    // entries and the side entries from their JSON, in another order, and
    // reach the same view.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let owner = PersonChain::from_json(&v["person"]).unwrap();
    let entries: Vec<Entry> = v["entries"].as_array().unwrap().iter().map(|x| Entry::parse(&x["entry"]).unwrap()).collect();
    for (x, e) in v["entries"].as_array().unwrap().iter().zip(&entries) {
        assert_eq!(e.signing_bytes(), x["signingBytes"].as_str().unwrap().as_bytes(), "{}", x["what"]);
        assert_eq!((e.id(), e.hash()), (x["id"].as_str().unwrap().to_string(), x["hash"].as_str().unwrap().to_string()));
    }
    let hash: [u8; 32] = xenonet_id::codec::unb64_n(&entries[0].hash()).unwrap();
    assert_eq!(v["space"].as_str(), Some(SpaceId::of_genesis(&hash).as_str()));
    let mut sp = Space::found(SpacePolicy::new(Some(Arc::new(owner)), None), entries[0].clone()).unwrap();
    // The side entries first, then the entries last to first: the take
    // waits for its base, and the rest for what they follow.
    for x in v["sides"].as_array().unwrap() {
        let s = Side::parse(&x["side"]).unwrap();
        assert_eq!(s.signing_bytes(), x["signingBytes"].as_str().unwrap().as_bytes(), "{}", x["what"]);
        assert_eq!(s.id(), x["id"].as_str().unwrap());
        sp.offer_side(s).unwrap();
    }
    for e in entries[1..].iter().rev() {
        sp.offer(e.clone()).unwrap();
    }
    let view = &v["view"];
    assert_eq!(sp.head().as_str(), view["head"].as_str().unwrap());
    assert_eq!(sp.view().curator.id(), view["curator"].as_str().unwrap());
    assert_eq!(u64::from(sp.view().epoch), view["epoch"].as_u64().unwrap());
    for (subject, level) in view["levels"].as_object().unwrap() {
        assert_eq!(sp.level(&Subject::parse(subject).unwrap()).map(Level::as_str), level.as_str(), "{subject}");
    }
}
