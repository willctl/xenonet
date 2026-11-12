// Known answers for the network's crypto (test/fixtures/net/known-answers.json):
// fixed keys, a fixed ephemeral key and fixed ids, so another
// implementation can check its key ids, heads, sealing, tickets and roster
// hashes against these, byte for byte. Ed25519 signs deterministically,
// and the presence blocks have fixed nonces, so all of it is reproducible.
// The roster is `xenonet-roster/2`, signed by each signer's machine signing
// key (seed 7 for studio), never its host key: entry 0 names its owner
// person, whose chain (a root key and studio's device) is here too, and
// laptop's admit carries the presence block from studio's device that the
// chain checks it by. The person's chain goes on past the head the roster
// names, with a delegation (the standard one, every device, a year) signed
// by that device with its presence, as a widening is. `sealedEnvelope`
// carries an entry of studio's feed, signed by studio's machine key as the
// roster lists it, in an envelope (`xenonet-net/2`) sealed for laptop with
// a fixed ephemeral key, and `envelope` is a live presence envelope from
// studio, unsealed.
//
// XENONET_WRITE_FIXTURES=1 cargo test -p xenonet-net --test known_answers
// writes the file again.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer};
use xenonet_log::person::{self, DeviceKind, NewDevice, PersonChain};
use xenonet_net::codec::{b64, unb64_n};
use xenonet_net::envelope::{Envelope, Me};
use xenonet_net::feed::{FeedEntry, Kind, Writer};
use xenonet_net::keys::HostKey;
use xenonet_net::roster::{found, HostRef, NetworkPolicy, Op, Roster, ADMIT};
use xenonet_net::ticket;

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../test/fixtures/net/known-answers.json");
const NET: &str = "net_00112233445566778899aabbccddeeff";

fn secret(byte: u8) -> [u8; 32] {
    let mut s = [byte; 32];
    s[0] = 0x40 | (byte & 0x0f);
    s
}

fn host(name: &str, sign: u8, seal: u8) -> (HostKey, Value) {
    let k = HostKey::from_secrets(&secret(sign), secret(seal)).unwrap();
    let v = json!({
        "id": name,
        "signSeed": b64(&secret(sign)),
        "sealSecret": b64(&secret(seal)),
        "sign": k.public().sign_b64(),
        "seal": k.public().seal_b64(),
        "keyId": k.key_id(),
    });
    (k, v)
}

/// A presence block from `device` for `what` and `act`, with a fixed nonce.
fn block(device: &SeedSigner, what: &str, act: [u8; 32], nonce: u8, at: i64) -> PresenceBlock {
    let mut b = PresenceBlock { device: device.public().id(), what: what.into(), act, nonce: [nonce; 16], at, sig: Vec::new() };
    b.sig = device.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
    b
}

/// The owner's chain: a root key, and studio's device paired under its own
/// presence.
fn person(root: &SeedSigner, device: &SeedSigner) -> PersonChain {
    let mut c = PersonChain::new();
    c.offer(person::found_salted(root, [0x5a; person::SALT_BYTES], 1_792_511_000_000).unwrap()).unwrap();
    let mut add = c.next(person::Body { op: person::Op::AddDevice(NewDevice { key: device.public(), kind: DeviceKind::Weaker, label: "studio".into() }), at: 1_792_511_000_001 });
    let b = block(device, person::PAIR, add.digest(), 0x11, 1_792_511_000_001);
    add.sign(&root.public().id(), root, Some(b)).unwrap();
    c.offer(add).unwrap();
    c
}

fn answers() -> Value {
    let (studio, studio_v) = host("studio", 1, 2);
    let (laptop, laptop_v) = host("laptop", 3, 4);
    let send = json!({
        "schema": "network/send/v1",
        "mid": "msg_00000000-0000-4000-8000-000000000001",
        "sender": "claude:aaaaaaaa-0001",
        "senderKind": "agent",
        "target": "claude:bbbbbbbb-0001",
        "body": "hello from studio",
        "replyTo": null,
        "scope": "direct",
        "kind": "message",
        "hops": 0,
        "at": 1_792_512_345_678_i64,
    });
    let studio_machine = SeedSigner::from_seed(&secret(7), KeyHome::File).unwrap();
    let entry = Writer::start(studio_machine.public().id(), NET, 1_792_512_000_000).write(&studio_machine, Kind::Message, 1_792_512_345_678, send).unwrap();
    let code = "KX7Q49MZT2";
    let root = SeedSigner::from_seed(&secret(5), KeyHome::File).unwrap();
    let device = SeedSigner::from_seed(&secret(6), KeyHome::File).unwrap();
    let machine_key = |n: u8| SeedSigner::from_seed(&secret(n), KeyHome::File).unwrap();
    let machine = |n: u8| machine_key(n).public();
    let owner = person(&root, &device);
    let mut r = Roster::with_policy(NetworkPolicy::with(std::sync::Arc::new(owner.clone())));
    let me = HostRef { id: "studio".into(), keys: *studio.public(), window: true, device: Some(device.public().id()), machine: machine(7) };
    r.offer(found(NET, me, &machine_key(7), owner.id().map(str::to_string), owner.head(), None, 1_792_512_000_000).unwrap()).unwrap();
    let mut add = r.draft(Op::Add, HostRef { id: "laptop".into(), keys: *laptop.public(), window: false, device: None, machine: machine(8) }, None, 1_792_512_100_000);
    add.person_head = owner.head();
    let admit = block(&device, ADMIT, add.digest(), 0x22, 1_792_512_100_000);
    add.sign_with("studio", &machine_key(7), Some(admit)).unwrap();
    r.offer(add).unwrap();
    let mut owner = owner;
    let mut dlg =
        owner.next(person::Body { op: person::Op::Delegate(person::Delegation::standard("dlg_00112233445566778899aabb".into(), 1_823_048_000_000)), at: 1_792_512_200_000 });
    let b = block(&device, person::DELEGATE, dlg.digest(), 0x33, 1_792_512_200_000);
    dlg.sign(&device.public().id(), &device, Some(b)).unwrap();
    owner.offer(dlg).unwrap();
    // The same entry in an envelope (xenonet-net/2), sealed for laptop's
    // transport key and named for its machine signing key; and a live
    // presence envelope from studio, unsealed.
    let sealed = Envelope::seal_entry(&entry, &machine(8).id(), laptop.public(), Some(secret(11))).unwrap();
    let presence = json!({ "op": "delta", "sessions": [{ "id": "claude:aaaaaaaa-0001", "state": "working" }] });
    let live = Envelope::live(NET, &machine(7).id(), "presence", 1_792_512_345_679, &presence).unwrap();
    json!({
        "about": "Known answers for one network across machines (docs/design/network.md, section 3): written by crates/xenonet-net/tests/known_answers.rs. Secrets here are test values, never a real host's.",
        "network": NET,
        "hosts": [studio_v, laptop_v],
        "ticket": {
            "code": code,
            "display": ticket::display(code),
            "hash": b64(&ticket::hash(code)),
        },
        "person": {
            "rootSeed": b64(&secret(5)),
            "deviceSeed": b64(&secret(6)),
            "id": owner.id(),
            "chain": owner.to_json(),
            "head": owner.head(),
        },
        "roster": r.to_json(),
        "rosterHead": r.head(),
        "envelope": { "body": presence, "head": live.head(), "envelope": live.to_json() },
        "sealedEnvelope": {
            "ephemeralSecret": b64(&secret(11)),
            "machineSeed": b64(&secret(7)),
            "entryId": entry.id(),
            "entry": entry.to_json(),
            "sealingHead": sealed.sealing_head(),
            "head": sealed.head(),
            "envelope": sealed.to_json(),
        },
    })
}

#[test]
fn the_crypto_gives_its_known_answers() {
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
    // What another implementation would do: read the file, rebuild the
    // keys from their secrets, and check each answer.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    let net = v["network"].as_str().unwrap();
    let key = |h: &Value| HostKey::from_secrets(&unb64_n::<32>(h["signSeed"].as_str().unwrap()).unwrap(), unb64_n::<32>(h["sealSecret"].as_str().unwrap()).unwrap()).unwrap();
    let studio = key(&v["hosts"][0]);
    let laptop = key(&v["hosts"][1]);
    assert_eq!(studio.key_id(), v["hosts"][0]["keyId"]);
    // The entry the sealed envelope carries checks against studio's
    // machine key, as the roster lists it, and has its id.
    let machine = SeedSigner::from_seed(&unb64_n::<32>(v["sealedEnvelope"]["machineSeed"].as_str().unwrap()).unwrap(), KeyHome::File).unwrap().public();
    let entry = FeedEntry::parse(&v["sealedEnvelope"]["entry"]).unwrap();
    assert_eq!(entry.id(), v["sealedEnvelope"]["entryId"]);
    let studio_in_roster = v["roster"][0]["host"]["machine"].clone();
    assert_eq!(studio_in_roster, machine.to_json(), "the roster's key for studio's machine");
    entry.verify(&machine).unwrap();
    let t = &v["ticket"];
    assert_eq!(b64(&ticket::hash(t["code"].as_str().unwrap())), t["hash"]);
    assert_eq!(ticket::normalize(t["display"].as_str().unwrap()).as_deref(), t["code"].as_str());
    let owner = PersonChain::from_json(&v["person"]["chain"]).unwrap();
    assert_eq!(owner.id(), v["person"]["id"].as_str());
    let device = SeedSigner::from_seed(&unb64_n::<32>(v["person"]["deviceSeed"].as_str().unwrap()).unwrap(), KeyHome::File).unwrap();
    assert!(owner.person().unwrap().active_device(&device.public().id()).is_some());
    assert_eq!(owner.person().unwrap().delegations_of(&device.public().id()).count(), 1);
    let r = Roster::from_json_with(NetworkPolicy::with(std::sync::Arc::new(owner)), &v["roster"]).unwrap();
    assert_eq!(r.head().as_deref(), v["rosterHead"].as_str());
    // Without the owner's chain, the admit's presence can't be checked.
    assert!(Roster::from_json(&v["roster"]).is_err());
    // The envelopes: the sealed one opens for laptop to the entry, checked
    // against studio's machine key in the roster; the live one is taken
    // from studio's own connection.
    let view = r.view();
    let sealed = Envelope::parse(&v["sealedEnvelope"]["envelope"]).unwrap();
    assert_eq!(sealed.sealing_head(), v["sealedEnvelope"]["sealingHead"].as_str().unwrap());
    assert_eq!(sealed.head(), v["sealedEnvelope"]["head"].as_str().unwrap());
    let laptop_machine = view.member("laptop").unwrap().machine.id();
    let opened = sealed.open_entry(&Me { machine: &laptop_machine, key: &laptop }, &view).unwrap();
    assert_eq!(opened.to_json(), v["sealedEnvelope"]["entry"]);
    let live = Envelope::parse(&v["envelope"]["envelope"]).unwrap();
    assert_eq!(live.head(), v["envelope"]["head"].as_str().unwrap());
    assert_eq!(live.open_live(&studio.public().sign, &view).unwrap(), v["envelope"]["body"]);
    assert_eq!(view.network, net, "the network the chain names");
}

#[test]
fn each_hosts_endpoint_id_is_its_host_key() {
    // The transport's endpoint id (iroh, on ed25519-dalek) from a host's
    // seed is the `sign` key ring made from the same seed, so a member's
    // chain entry names the endpoint to dial.
    let v: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    for h in v["hosts"].as_array().unwrap() {
        let seed = unb64_n::<32>(h["signSeed"].as_str().unwrap()).unwrap();
        let sign = unb64_n::<32>(h["sign"].as_str().unwrap()).unwrap();
        assert_eq!(iroh::SecretKey::from_bytes(&seed).public().as_bytes(), &sign, "{}", h["id"]);
        let key = HostKey::from_secrets(&seed, unb64_n::<32>(h["sealSecret"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(xenonet_net::endpoint::secret_key(&key).public().as_bytes(), &sign);
        assert_eq!(xenonet_net::endpoint::peer_id(key.public()).0, sign);
    }
}
