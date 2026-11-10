//! The roster's rules: each row of the table, duplicates, forks and gaps, and a chain read back from its JSON.

use super::*;
use crate::chain::Offered;
use serde_json::json;

const NET: &str = "net_0123456789abcdef0123456789abcdef";

struct Hosts {
    keys: BTreeMap<&'static str, HostKey>,
}

impl Hosts {
    fn new(names: &[&'static str]) -> Hosts {
        Hosts { keys: names.iter().map(|n| (*n, HostKey::generate().unwrap())).collect() }
    }
    fn r(&self, id: &'static str, window: bool) -> HostRef {
        HostRef { id: id.into(), keys: *self.keys[id].public(), window }
    }
    fn entry(&self, r: &Roster, op: Op, host: HostRef, signers: &[&'static str]) -> Entry {
        let gateway = (op == Op::Gateway).then(|| GatewayAt { address: format!("{}.example.net", host.id), version: r.view().gateway_version + 1 });
        let mut e = r.draft(op, host, gateway, 1000 + r.len() as i64);
        for s in signers {
            e.sign_by(s, &self.keys[s]);
        }
        e
    }
}

/// studio founds; desk and nook are window hosts (added by studio);
/// laptop isn't.
fn network(h: &Hosts) -> Roster {
    let mut r = Roster::new();
    r.offer(found(NET, "studio", &h.keys["studio"], None, 1)).unwrap();
    for (id, window) in [("desk", true), ("nook", true), ("laptop", false)] {
        let e = h.entry(&r, Op::Add, h.r(id, window), &["studio"]);
        assert_eq!(r.offer(e).unwrap(), Offered::Appended, "{id}");
    }
    r
}

fn hosts() -> Hosts {
    Hosts::new(&["studio", "desk", "nook", "laptop", "spare", "tablet"])
}

#[test]
fn found_is_entry_0_signed_by_the_founder_alone() {
    let h = hosts();
    let mut r = Roster::new();
    let mut e = found(NET, "studio", &h.keys["studio"], None, 1);
    e.sign_by("desk", &h.keys["desk"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "roster-refused", "alone");
    let mut e = found(NET, "studio", &h.keys["studio"], None, 1);
    e.signers[0].sig = h.keys["desk"].sign(&e.signing_bytes());
    assert!(r.clone().offer(e).is_err(), "by its own key");
    let e = h.entry(&r, Op::Add, h.r("desk", false), &["desk"]);
    assert!(r.offer(e).is_err(), "a roster starts with found");
    let r = network(&h);
    let again = found(NET, "spare", &h.keys["spare"], None, 9);
    let again = Entry { n: r.len(), prev: r.head().unwrap(), ..again };
    assert!(r.clone().offer(again).is_err(), "once");
    assert_eq!(r.view().founder, "studio");
    assert_eq!(r.view().gateway, "studio");
}

#[test]
fn an_ordinary_host_is_added_or_removed_by_any_window_host() {
    let h = hosts();
    let mut r = network(&h);
    // laptop isn't a window host: it can't add.
    let e = h.entry(&r, Op::Add, h.r("spare", false), &["laptop"]);
    assert_eq!(r.offer(e).unwrap_err().code, "roster-refused");
    let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
    r.offer(e).unwrap();
    let e = h.entry(&r, Op::Remove, h.r("spare", false), &["laptop"]);
    assert!(r.offer(e).is_err(), "not by an ordinary host");
    let e = h.entry(&r, Op::Remove, h.r("spare", false), &["nook"]);
    r.offer(e).unwrap();
    assert!(r.view().removal("spare").is_some());
    assert!(r.view().member("spare").is_none());
    // Removed, it can be added again (a reinstalled machine, Replace).
    let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
    r.offer(e).unwrap();
    assert!(r.view().removal("spare").is_none());
}

#[test]
fn window_hosts_gateway_moves_and_their_removal_need_the_founder_or_two() {
    let h = hosts();
    let r = network(&h);
    let strong = [
        h.entry(&r, Op::Add, h.r("tablet", true), &[]),
        h.entry(&r, Op::Window, h.r("laptop", true), &[]),
        h.entry(&r, Op::Remove, h.r("desk", true), &[]),
        h.entry(&r, Op::Gateway, h.r("desk", true), &[]),
    ];
    for e in strong {
        let what = format!("{} {}", e.op.as_str(), e.host.id);
        let with = |signers: &[&'static str]| {
            let mut e = e.clone();
            for s in signers {
                e.sign_by(s, &h.keys[s]);
            }
            r.clone().offer(e)
        };
        let one = if e.host.id == "desk" { "nook" } else { "desk" };
        assert_eq!(with(&[one]).unwrap_err().code, "needs-approval", "{what}: one window host");
        assert_eq!(with(&[one, "laptop"]).unwrap_err().code, "needs-approval", "{what}: an ordinary host doesn't count");
        assert_eq!(with(&["studio"]).unwrap(), Offered::Appended, "{what}: the founder");
        assert_eq!(with(&["desk", "nook"]).unwrap(), Offered::Appended, "{what}: two window hosts");
    }
    // A gateway entry moves the gateway, with a higher version each time.
    let mut r = r;
    let e = h.entry(&r, Op::Gateway, h.r("desk", true), &["studio"]);
    r.offer(e).unwrap();
    assert_eq!((r.view().gateway.as_str(), r.view().gateway_version), ("desk", 1));
    let mut stale = r.draft(Op::Gateway, h.r("nook", true), Some(GatewayAt { address: "x.example".into(), version: 1 }), 5);
    stale.sign_by("studio", &h.keys["studio"]);
    assert_eq!(r.offer(stale).unwrap_err().code, "stale");
}

#[test]
fn two_window_hosts_approving_apart_make_one_entry_that_counts() {
    let h = hosts();
    let mut r = network(&h);
    let desk = h.entry(&r, Op::Gateway, h.r("desk", true), &["desk"]);
    assert_eq!(r.clone().offer(desk.clone()).unwrap_err().code, "needs-approval");
    let mut nook = desk.clone();
    nook.signers.clear();
    nook.sign_by("nook", &h.keys["nook"]);
    let mut both = desk.clone();
    both.merge_signers(&nook);
    both.merge_signers(&desk);
    assert_eq!(both.signers.len(), 2);
    assert_eq!(r.offer(both).unwrap(), Offered::Appended);
    assert_eq!(r.view().gateway, "desk");
}

#[test]
fn the_founder_is_unremovable_and_a_host_may_remove_itself() {
    let h = hosts();
    let mut r = network(&h);
    for signers in [&["studio"][..], &["desk", "nook"], &["studio", "desk", "nook"]] {
        let e = h.entry(&r, Op::Remove, h.r("studio", true), signers);
        assert!(r.clone().offer(e).unwrap_err().message.contains("founder"), "{signers:?}");
    }
    // Leaving: laptop takes itself out, and so may a window host.
    let e = h.entry(&r, Op::Remove, h.r("laptop", false), &["laptop"]);
    r.offer(e).unwrap();
    let e = h.entry(&r, Op::Remove, h.r("nook", true), &["nook"]);
    r.offer(e).unwrap();
    // But not another host.
    let e = h.entry(&r, Op::Remove, h.r("desk", true), &["desk", "studio"]);
    assert!(r.clone().offer(e).is_ok());
    let mut e = h.entry(&r, Op::Remove, h.r("desk", true), &[]);
    e.sign_by("desk", &h.keys["desk"]);
    e.signers[0].by = "studio".into();
    assert!(r.offer(e).is_err(), "a signature checks against its signer's key");
}

#[test]
fn the_gateways_host_is_removed_only_once_the_gateway_moved() {
    let h = hosts();
    let mut r = network(&h);
    let e = h.entry(&r, Op::Gateway, h.r("desk", true), &["studio"]);
    r.offer(e).unwrap();
    assert_eq!(r.view().gateway, "desk");
    // Not by the founder, nor by itself leaving.
    for signers in [&["studio"][..], &["desk"], &["studio", "nook"]] {
        let e = h.entry(&r, Op::Remove, h.r("desk", true), signers);
        let err = r.clone().offer(e).unwrap_err();
        assert!(err.message.contains("move the gateway first"), "{signers:?}: {}", err.message);
    }
    // Moved back, it may go.
    let e = h.entry(&r, Op::Gateway, h.r("studio", true), &["studio"]);
    r.offer(e).unwrap();
    let e = h.entry(&r, Op::Remove, h.r("desk", true), &["studio"]);
    assert_eq!(r.offer(e).unwrap(), Offered::Appended);
}

#[test]
fn duplicates_are_dropped_and_forks_refused() {
    let h = hosts();
    let mut r = network(&h);
    let base = r.clone();
    // Two window hosts approve the same thing at once.
    let a = h.entry(&base, Op::Add, h.r("spare", false), &["desk"]);
    let b = h.entry(&base, Op::Add, h.r("spare", false), &["nook"]);
    assert_eq!(r.offer(a).unwrap(), Offered::Appended);
    assert_eq!(r.offer(b).unwrap(), Offered::Duplicate);
    assert_eq!(r.len(), base.len() + 1);
    // A different choice at the same index loses to the chain's order.
    let c = h.entry(&base, Op::Add, h.r("spare", true), &["studio"]);
    assert_eq!(r.offer(c).unwrap_err().code, "roster-fork");
    let d = h.entry(&base, Op::Add, h.r("tablet", false), &["desk"]);
    assert_eq!(r.offer(d).unwrap_err().code, "roster-fork");
    // An entry past the head is a gap, not taken.
    let mut e = h.entry(&r, Op::Add, h.r("tablet", false), &[]);
    e.n += 1;
    e.sign_by("desk", &h.keys["desk"]);
    assert_eq!(r.offer(e).unwrap_err().code, "roster-gap");
    // The same host again is taken; its keys under another id too.
    let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
    assert_eq!(r.offer(e).unwrap_err().code, "taken");
    let e = h.entry(&r, Op::Add, HostRef { id: "other".into(), ..h.r("spare", false) }, &["desk"]);
    assert_eq!(r.offer(e).unwrap_err().code, "taken");
}

#[test]
fn an_entry_the_roster_couldnt_read_back_is_refused() {
    let h = hosts();
    let mut r = network(&h);
    // A gateway entry made here with an address past 300 bytes was
    // taken, and then roster.json didn't read.
    let long = GatewayAt { address: "\u{fffd}".repeat(101), version: 1 };
    let mut e = r.draft(Op::Gateway, h.r("desk", true), Some(long), 5);
    e.sign_by("studio", &h.keys["studio"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    let mut e = r.draft(Op::Gateway, h.r("desk", true), Some(GatewayAt { address: String::new(), version: 1 }), 5);
    e.sign_by("studio", &h.keys["studio"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request", "nor an empty one");
    // A host id that isn't one.
    let mut e = r.draft(Op::Add, HostRef { id: "Spare Host".into(), ..h.r("spare", false) }, None, 5);
    e.sign_by("studio", &h.keys["studio"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    // Five signers, where an entry carries four at most.
    let e = h.entry(&r, Op::Add, h.r("tablet", false), &["studio"]);
    r.offer(e).unwrap();
    let e = h.entry(&r, Op::Add, h.r("spare", true), &["studio", "desk", "nook", "laptop", "tablet"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    let back = Roster::from_json(&r.to_json()).unwrap();
    assert_eq!(back.view(), r.view());
}

#[test]
fn the_whole_chain_is_checked_and_survives_its_json() {
    let h = hosts();
    let r = network(&h);
    let back = Roster::from_json(&r.to_json()).unwrap();
    assert_eq!(back.view(), r.view());
    assert_eq!(back.head(), r.head());
    // A changed entry breaks its own signature, or the next one's prev.
    let mut v = r.to_json();
    v[2]["host"]["window"] = json!(false);
    assert!(Roster::from_json(&v).is_err());
    let mut v = r.to_json();
    v[1]["at"] = json!(77);
    let err = Roster::from_json(&v).unwrap_err();
    assert_eq!(err.code, "roster-refused", "{err}");
    // An entry dropped from the middle leaves a gap.
    let mut v = r.to_json();
    v.as_array_mut().unwrap().remove(1);
    assert!(Roster::from_json(&v).is_err());
    // A key id that doesn't match the keys.
    let mut v = r.to_json();
    v[1]["host"]["keyId"] = json!("xnk1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(Roster::from_json(&v).is_err());
    assert_eq!(r.since(2).len(), 2);
    assert_eq!(r.since(99).len(), 0);
}
