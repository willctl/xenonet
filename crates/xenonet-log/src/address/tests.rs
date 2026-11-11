//! Address records: signed by the machine signing key, checked against
//! the chain's member, its endpoint now and the network's relays.

use super::*;
use crate::network::{Member, Settings};
use serde_json::json;
use xenonet_id::keys::HostKey;
use xenonet_id::sign::{KeyHome, SeedSigner};

const NET: &str = "net_0123456789abcdef0123456789abcdef";
const T: u64 = 1_792_512_000_000;
const RELAY: &str = "https://relay.example.net";

struct Laptop {
    transport: HostKey,
    machine: SeedSigner,
}

fn laptop() -> Laptop {
    Laptop { transport: HostKey::from_secrets(&[3; 32], [4; 32]).unwrap(), machine: SeedSigner::from_seed(&[5; 32], KeyHome::File).unwrap() }
}

fn view(l: &Laptop) -> View {
    let m = Member {
        id: "laptop".into(),
        keys: *l.transport.public(),
        window: false,
        device: None,
        machine: l.machine.public(),
        settings: Settings::default(),
        settings_by: None,
        since: 1,
    };
    View { network: NET.into(), founder: "studio".into(), members: [("laptop".to_string(), m)].into(), relays: vec![RELAY.into()], ..View::default() }
}

fn record(l: &Laptop) -> AddressRecord {
    let mut r = AddressRecord {
        network: NET.into(),
        host: "laptop".into(),
        endpoint: l.transport.public().sign,
        relay: Some(RELAY.into()),
        addrs: vec!["192.168.1.20:41641".into(), "[fd7a:115c:a1e0::1]:41641".into()],
        at: T,
        ttl: 600,
        sig: Vec::new(),
    };
    r.sign(&l.machine).unwrap();
    r
}

#[test]
fn a_record_checks_against_the_members_machine_key_and_reads_back() {
    let l = laptop();
    let r = record(&l);
    r.verify(&view(&l), T + 1).unwrap();
    assert!(r.signing_bytes().starts_with(b"xenonet-address/1\n"));
    let back = AddressRecord::decode(&r.encode()).unwrap();
    assert_eq!(back, r);
    // Without a relay, and with no direct address: still a record.
    let mut bare = AddressRecord { relay: None, addrs: Vec::new(), ..r.clone() };
    bare.sign(&l.machine).unwrap();
    bare.verify(&view(&l), T).unwrap();
}

#[test]
fn a_record_its_machine_didnt_sign_is_refused() {
    let l = laptop();
    let v = view(&l);
    // Signed by the transport key: a copied host.key can't say where the
    // machine is.
    let mut r = record(&l);
    r.sig = l.transport.sign(&r.signing_bytes());
    assert_eq!(r.verify(&v, T).unwrap_err().code, "address-refused");
    // Changed after signing.
    for change in 0..5 {
        let mut r = record(&l);
        match change {
            0 => r.addrs.push("10.0.0.9:1".into()),
            1 => r.relay = None,
            2 => r.ttl += 1,
            3 => r.at += 1,
            _ => r.host = "desk".into(),
        }
        assert!(r.verify(&v, T).is_err(), "change {change}");
    }
}

#[test]
fn a_record_names_the_endpoint_now_and_only_a_relay_the_network_lists() {
    let l = laptop();
    let mut v = view(&l);
    let r = record(&l);
    // After a rekey, the old endpoint's record holds no more.
    v.members.get_mut("laptop").unwrap().keys = *HostKey::generate().unwrap().public();
    assert_eq!(r.verify(&v, T).unwrap_err().code, "address-refused");
    let mut v = view(&l);
    v.relays.clear();
    assert_eq!(r.verify(&v, T).unwrap_err().code, "address-refused", "a relay the network doesn't list");
    let mut v = view(&l);
    v.network = "net_ffffffffffffffffffffffffffffffff".into();
    assert_eq!(r.verify(&v, T).unwrap_err().code, "address-refused", "another network");
    let mut v = view(&l);
    v.members.clear();
    assert_eq!(r.verify(&v, T).unwrap_err().code, "address-refused", "not a member");
}

#[test]
fn a_record_holds_for_its_ttl_and_not_from_far_ahead() {
    let l = laptop();
    let v = view(&l);
    let r = record(&l);
    r.verify(&v, T + 599_999).unwrap();
    assert_eq!(r.verify(&v, T + 600_000).unwrap_err().code, "expired");
    r.verify(&v, T - MAX_AHEAD).unwrap();
    assert_eq!(r.verify(&v, T - MAX_AHEAD - 1).unwrap_err().code, "clock-skew");
}

#[test]
fn records_are_checked_for_their_shape() {
    let l = laptop();
    let good = record(&l).to_json();
    assert!(AddressRecord::parse(&good).is_ok());
    for (k, v) in [
        ("network", json!("net_x")),
        ("host", json!("Laptop")),
        ("endpoint", json!("AAAA")),
        ("relay", json!("https://u:token@relay.example.net")),
        ("relay", json!("https://relay.example.net/?token=x")),
        ("addrs", json!(["laptop.local:41641"])),
        ("addrs", json!(["192.168.1.20:0"])),
        ("addrs", json!(["192.168.1.20:41641", "192.168.1.20:41641"])),
        ("addrs", json!((0..=MAX_ADDRS).map(|i| format!("10.0.0.{i}:1")).collect::<Vec<_>>())),
        ("ttl", json!(0)),
        ("ttl", json!(MAX_TTL + 1)),
        ("at", json!(-1)),
        ("sig", json!("AA")),
    ] {
        let mut bad = good.clone();
        bad[k] = v.clone();
        assert!(AddressRecord::parse(&bad).is_err(), "{k}: {v}");
    }
    assert!(AddressRecord::decode(&vec![b' '; MAX_RECORD + 1]).is_err());
}
