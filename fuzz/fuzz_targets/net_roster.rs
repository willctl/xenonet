#![no_main]

// The roster (network/roster-entry/v2): entries parsed and offered to a
// Roster, one after another, from the fixed network's chain or from
// nothing. Each step is an entry as it came, an entry built from the
// fixed keys (any op, host, window choice and gateway, signed by any of
// the hosts, with their own key or another's, with or without their
// device's presence, at its index or not) and
// maybe changed in its JSON, two copies of one merged, or a whole chain.
//
// Nothing may panic. An entry is taken only when each of its signers
// signed it with the key the roster has for them, at the roster's head;
// the founder and the gateway stay in the network; a chain reads back
// from its own JSON as the same chain.

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use xenonet_fuzz::net::{self, Choices};
use xenonet_net::roster::{Entry, GatewayAt, Offered, Op, Roster, Settings, Start, Wake};

const OPS: [Op; 6] = [Op::Found, Op::Add, Op::Remove, Op::Window, Op::Gateway, Op::Settings];

/// Whether `e`'s signers signed it as `r` (before `e`) has them.
fn rightly_signed(r: &Roster, e: &Entry) -> bool {
    let bytes = e.signing_bytes();
    if e.signers.is_empty() {
        return false;
    }
    match r.view_ref() {
        None => e.signers.iter().all(|s| s.by == e.host.id && e.host.keys.verify(&bytes, &s.sig)),
        Some(v) => e.signers.iter().all(|s| v.member(&s.by).is_some_and(|m| m.keys.verify(&bytes, &s.sig))),
    }
}

fn holds(r: &Roster) {
    let Some(v) = r.view_ref() else {
        assert!(r.is_empty());
        return;
    };
    assert!(v.member(&v.founder).is_some(), "the founder left the network");
    assert!(v.is_window(&v.founder));
    assert!(v.member(&v.gateway).is_some(), "the gateway's host left the network");
    let back = Roster::from_json_with(net::policy(), &r.to_json()).expect("a roster reads back from its own JSON");
    assert_eq!(back.view(), r.view());
    assert_eq!(back.head(), r.head());
    for e in r.entries() {
        assert_eq!(Entry::parse(&e.to_json()).as_ref(), Ok(e), "an entry reads back as another");
    }
}

/// Offers `e` to `r`, checking what the roster did against what it may.
fn offer(r: &mut Roster, e: Entry) {
    let before = r.clone();
    let would = r.would_take(&e);
    match r.offer(e.clone()) {
        Ok(Offered::Appended) => {
            assert!(would.is_ok(), "offer took what would_take refused: {would:?}");
            assert!(rightly_signed(&before, &e), "the roster took an entry that isn't rightly signed: {e:?}");
            assert_eq!(e.n, before.len());
            assert_eq!(e.prev, before.head().unwrap_or_default());
            assert_eq!(r.len(), before.len() + 1);
        }
        Ok(Offered::Duplicate) => {
            assert_eq!(r.len(), before.len(), "a duplicate changed the roster");
            assert_eq!(r.head(), before.head());
        }
        Err(_) => {
            assert!(would.is_err() || usize::try_from(e.n).is_ok_and(|i| i < before.entries().len()), "would_take took what offer refused");
            assert_eq!(r.len(), before.len(), "a refused entry changed the roster");
            assert_eq!(r.head(), before.head());
        }
    }
    holds(r);
}

/// An entry of the fixed network, as a host would draft and sign it, with
/// the choices' changes.
fn built(r: &Roster, c: &mut Choices<'_>) -> Entry {
    let op = *c.pick(&OPS);
    let host = c.below(net::HOSTS.len());
    // Sometimes another host's keys under this one's id.
    let window = c.bool();
    let mut host_ref = net::host_ref(host, window);
    if c.rarely(24) {
        host_ref.keys = net::public(c.below(net::HOSTS.len()));
    }
    let gateway = (op == Op::Gateway || c.rarely(16)).then(|| {
        let address = String::from_utf8_lossy(c.chunk()).into_owned();
        let version = r.view().gateway_version.saturating_add(u64::from(c.u8() % 3)).wrapping_sub(u64::from(c.u8() % 2));
        GatewayAt { address: if address.is_empty() { "gw2.example.net".into() } else { address }, version }
    });
    let mut e = r.draft(op, host_ref, gateway, net::T0 + i64::from(c.i16()));
    e.person_head = net::person().head();
    if op == Op::Settings {
        e.settings = Some(Settings { start: *c.pick(&Start::ALL), wake: *c.pick(&Wake::ALL) });
    }
    let what = e.presence_for(&r.view());
    match c.below(8) {
        0 => e.n = e.n.wrapping_add(u64::from(c.u8() % 3)).wrapping_sub(1),
        1 => e.prev = String::from_utf8_lossy(c.chunk()).into_owned(),
        2 => e.network = format!("net_{:032x}", c.u16()),
        _ => {}
    }
    for _ in 0..=c.below(4) {
        let signer = c.below(net::HOSTS.len());
        let with = if c.rarely(32) { c.below(net::HOSTS.len()) } else { signer };
        let presence = what.filter(|_| c.bool()).map(|w| net::presence(with, w, e.digest(), net::T0));
        e.sign_with(net::HOSTS[with], net::key(with), presence);
        // Named for one host, signed with another's key.
        if let Some(s) = e.signers.last_mut() {
            s.by = net::HOSTS[signer].into();
        }
    }
    if c.rarely(16) {
        e.signers.truncate(c.below(e.signers.len() + 1));
    }
    e
}

/// An entry's JSON with one field changed, read again.
fn changed(e: &Entry, c: &mut Choices<'_>) -> Option<Entry> {
    let mut v = e.to_json();
    let v2 = c.value();
    let path: &[&str] = c.pick(&[
        &["n"][..],
        &["prev"],
        &["network"],
        &["op"],
        &["host", "id"],
        &["host", "sign"],
        &["host", "seal"],
        &["host", "keyId"],
        &["host", "window"],
        &["gateway"],
        &["at"],
        &["signers"],
        &["signers", "0"],
    ]);
    let mut at = &mut v;
    for (i, k) in path.iter().enumerate() {
        let last = i + 1 == path.len();
        at = match (at, k.parse::<usize>()) {
            (Value::Array(a), Ok(n)) if n < a.len() => &mut a[n],
            (Value::Object(o), _) if last || o.contains_key(*k) => o.entry(k.to_string()).or_insert(Value::Null),
            _ => return Entry::parse(&v2).ok(),
        };
    }
    *at = v2;
    Entry::parse(&v).ok()
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let start = c.u8();
    let mut r = if start & 1 == 0 { net::roster() } else { Roster::with_policy(net::policy()) };
    holds(&r);
    for _ in 0..8 {
        match c.below(6) {
            0 => {
                if let Ok(e) = Entry::parse(&c.value()) {
                    assert_eq!(Entry::parse(&e.to_json()).as_ref(), Ok(&e), "an entry reads back as another");
                    offer(&mut r, e);
                }
            }
            1 | 2 => {
                let e = built(&r, &mut c);
                offer(&mut r, e);
            }
            3 => {
                let e = built(&r, &mut c);
                if let Some(e) = changed(&e, &mut c) {
                    offer(&mut r, e);
                }
            }
            // Two approvals of one entry, made apart and merged, as the
            // gateway merges a proposal's signers.
            4 => {
                let mut a = built(&r, &mut c);
                let mut b = a.clone();
                b.signers.clear();
                let s = c.below(net::HOSTS.len());
                b.sign_by(net::HOSTS[s], net::key(s));
                a.merge_signers(&b);
                assert!(a.signers.len() <= xenonet_net::roster::MAX_SIGNERS);
                offer(&mut r, a);
            }
            _ => {
                // A whole chain: the fixed one with an entry changed, or
                // the input's.
                let v = if c.bool() {
                    let mut v = net::roster().to_json();
                    let i = c.below(5);
                    if let Some(e) = v.get(i).and_then(|e| Entry::parse(e).ok()) {
                        if let Some(e2) = changed(&e, &mut c) {
                            v[i] = e2.to_json();
                        }
                    }
                    v
                } else {
                    c.value()
                };
                if let Ok(chain) = Roster::from_json_with(net::policy(), &v) {
                    let mut again = Roster::with_policy(net::policy());
                    for e in chain.entries() {
                        assert!(rightly_signed(&again, e), "a chain was read with an entry that isn't rightly signed: {e:?}");
                        assert_eq!(again.offer(e.clone()).ok(), Some(Offered::Appended));
                    }
                    holds(&chain);
                }
            }
        }
        if c.is_empty() {
            break;
        }
    }
});
