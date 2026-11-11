// Known answers for the network chain's relays, mailboxes and rekey
// entries, and for an address record (test/fixtures/log/network.json):
// fixed keys, fixed nonces and fixed times, so another implementation can
// check its canonical bytes, entry hashes and signatures against these,
// byte for byte. Ed25519 signs deterministically, so all of it is
// reproducible. studio founds the network, owned by a person whose
// devices are studio's and desk's; desk joins as an admin; studio lists a
// relay; desk names itself a mailbox; desk rekeys its transport keys; and
// desk's machine signs where its endpoint is.
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-log --test network
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_id::codec::b64;
use xenonet_id::keys::HostKey;
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer};
use xenonet_log::address::AddressRecord;
use xenonet_log::network::{found, HostRef, NetworkPolicy, Op, Roster};
use xenonet_log::person::{self, DeviceKind, NewDevice, PersonChain};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/log/network.json");
const NET: &str = "net_00112233445566778899aabbccddeeff";
const T: i64 = 1_792_512_000_000;
const RELAY: &str = "https://relay.example.net";

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn signer(byte: u8) -> SeedSigner {
    SeedSigner::from_seed(&secret(byte), KeyHome::File).unwrap()
}

fn transport(sign: u8, seal: u8) -> HostKey {
    HostKey::from_secrets(&secret(sign), secret(seal)).unwrap()
}

/// A presence block from `device` for `what` and `act`, with a fixed nonce.
fn block(device: &SeedSigner, what: &str, act: [u8; 32], nonce: u8, at: i64) -> PresenceBlock {
    let mut b = PresenceBlock { device: device.public().id(), what: what.into(), act, nonce: [nonce; 16], at, sig: Vec::new() };
    b.sig = device.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
    b
}

fn person(root: &SeedSigner, devices: &[(&str, &SeedSigner)]) -> PersonChain {
    let mut c = PersonChain::new();
    c.offer(person::found_salted(root, [0x5a; person::SALT_BYTES], T - 1000).unwrap()).unwrap();
    for (i, (label, d)) in devices.iter().enumerate() {
        let at = T - 900 + i as i64;
        let mut add = c.next(person::Body { op: person::Op::AddDevice(NewDevice { key: d.public(), kind: DeviceKind::Weaker, label: (*label).into() }), at });
        let b = block(d, person::PAIR, add.digest(), 0x11 + i as u8, at);
        add.sign(&root.public().id(), root, Some(b)).unwrap();
        c.offer(add).unwrap();
    }
    c
}

fn answers() -> Value {
    let (studio_t, desk_t, desk_t2) = (transport(1, 2), transport(3, 4), transport(5, 6));
    let (studio_m, desk_m) = (signer(7), signer(8));
    let (studio_d, desk_d) = (signer(9), signer(10));
    let owner = person(&signer(11), &[("studio", &studio_d), ("desk", &desk_d)]);
    let head = owner.head();
    let mut r = Roster::with_policy(NetworkPolicy::with(Arc::new(owner.clone())));
    let studio = HostRef { id: "studio".into(), keys: *studio_t.public(), window: true, device: Some(studio_d.public().id()), machine: studio_m.public() };
    let desk = HostRef { id: "desk".into(), keys: *desk_t.public(), window: true, device: Some(desk_d.public().id()), machine: desk_m.public() };
    r.offer(found(NET, studio.clone(), &studio_m, owner.id().map(str::to_string), head.clone(), None, T).unwrap()).unwrap();
    let mut entries = Vec::new();
    let mut step = |r: &mut Roster, what: &'static str, op: Op, host: HostRef, by: (&str, &SeedSigner, &SeedSigner), list: Option<Vec<String>>, nonce: u8| {
        let mut e = r.draft(op, host, None, T + r.len() as i64);
        e.person_head.clone_from(&head);
        match op {
            Op::Relays => e.relays = list,
            Op::Mailboxes => e.mailboxes = list,
            _ => {}
        }
        let presence = e.presence_for(&r.view()).map(|w| block(by.2, w, e.digest(), nonce, e.at));
        e.sign_with(by.0, by.1, presence).unwrap();
        r.offer(e.clone()).unwrap();
        if what != "add" {
            entries.push(json!({
                "what": what,
                "signingBytes": String::from_utf8(e.signing_bytes()).unwrap(),
                "id": e.id(),
                "hash": e.hash(),
                "entry": e.to_json(),
            }));
        }
    };
    step(&mut r, "add", Op::Add, desk.clone(), ("studio", &studio_m, &studio_d), None, 0x21);
    step(&mut r, "the founder lists a relay", Op::Relays, studio.clone(), ("studio", &studio_m, &studio_d), Some(vec![RELAY.into()]), 0x22);
    step(&mut r, "an admin names itself a mailbox", Op::Mailboxes, desk.clone(), ("desk", &desk_m, &desk_d), Some(vec!["desk".into()]), 0x23);
    step(&mut r, "desk rekeys its transport keys", Op::Rekey, HostRef { keys: *desk_t2.public(), ..desk }, ("desk", &desk_m, &desk_d), None, 0x24);
    let mut a = AddressRecord {
        network: NET.into(),
        host: "desk".into(),
        endpoint: desk_t2.public().sign,
        relay: Some(RELAY.into()),
        addrs: vec!["192.168.1.20:41641".into()],
        at: T as u64 + 10,
        ttl: 600,
        sig: Vec::new(),
    };
    a.sign(&desk_m).unwrap();
    a.verify(&r.view(), T as u64 + 11).unwrap();
    json!({
        "about": "Known answers for the network chain's relays, mailboxes and rekey entries, and an address record (crates/xenonet-log/src/network.rs, address.rs): written by crates/xenonet-log/tests/network.rs. Seeds are test values, never a machine's.",
        "network": NET,
        "person": owner.to_json(),
        "seeds": {
            "studio": { "transport": [b64(&secret(1)), b64(&secret(2))], "machine": b64(&secret(7)), "device": b64(&secret(9)) },
            "desk": { "transport": [b64(&secret(3)), b64(&secret(4))], "rekeyedTo": [b64(&secret(5)), b64(&secret(6))], "machine": b64(&secret(8)), "device": b64(&secret(10)) },
        },
        "chain": r.to_json(),
        "head": r.head(),
        "entries": entries,
        "addressRecord": {
            "signingBytes": String::from_utf8(a.signing_bytes()).unwrap(),
            "record": a.to_json(),
        },
    })
}

#[test]
fn the_network_chain_gives_its_known_answers() {
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
    // What another implementation would do: read the person's chain and
    // the network's from their JSON, check them whole, and check the
    // address record against the chain they leave.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let owner = PersonChain::from_json(&v["person"]).unwrap();
    let r = Roster::from_json_with(NetworkPolicy::with(Arc::new(owner)), &v["chain"]).unwrap();
    assert_eq!(r.head().as_deref(), v["head"].as_str());
    for x in v["entries"].as_array().unwrap() {
        let n = x["entry"]["n"].as_u64().unwrap() as usize;
        let e = &r.entries()[n];
        assert_eq!(e.signing_bytes(), x["signingBytes"].as_str().unwrap().as_bytes(), "{}", x["what"]);
        assert_eq!((e.id(), e.hash()), (x["id"].as_str().unwrap().to_string(), x["hash"].as_str().unwrap().to_string()));
    }
    let view = r.view();
    assert_eq!(view.relays, vec![RELAY.to_string()]);
    assert_eq!(view.mailboxes, vec!["desk".to_string()]);
    let a = AddressRecord::parse(&v["addressRecord"]["record"]).unwrap();
    assert_eq!(a.signing_bytes(), v["addressRecord"]["signingBytes"].as_str().unwrap().as_bytes());
    a.verify(&view, a.at).unwrap();
}
