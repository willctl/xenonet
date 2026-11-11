//! The roster's rules: each row of the table, presence on admits and
//! widenings, duplicates, forks and gaps, and a chain read back from its
//! JSON.

use super::*;
use crate::chain::Offered;
use crate::person::{self, DeviceKind, NewDevice, PersonChain};
use serde_json::json;
use xenonet_id::presence::{prove, FixedCheck, PresenceBlock};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

const NET: &str = "net_0123456789abcdef0123456789abcdef";
const T: i64 = 1_792_512_000_000;

/// The hosts' transport keys, machine keys and device keys, and the
/// person whose devices they are.
struct Hosts {
    keys: BTreeMap<&'static str, HostKey>,
    devices: BTreeMap<&'static str, SeedSigner>,
    machines: BTreeMap<&'static str, SeedSigner>,
    person: Arc<std::sync::RwLock<PersonChain>>,
}

/// The person chain, shared as the network crate shares it.
struct Shared(Arc<std::sync::RwLock<PersonChain>>);

impl Devices for Shared {
    fn person_device(&self, person: &str, head: &str, device: &str) -> Option<person::Device> {
        self.0.read().unwrap().person_device(person, head, device)
    }
}

fn seed(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

impl Hosts {
    fn new(names: &[&'static str]) -> Hosts {
        let root = seed(200);
        let mut chain = PersonChain::new();
        chain.offer(person::found(&root, T).unwrap()).unwrap();
        let mut devices = BTreeMap::new();
        let mut machines = BTreeMap::new();
        for (i, n) in names.iter().enumerate() {
            let d = seed(100 + i as u8);
            let mut e = chain.next(person::Body { op: person::Op::AddDevice(NewDevice { key: d.public(), kind: DeviceKind::Weaker, label: (*n).into() }), at: T + i as i64 });
            let block = prove(&d, Some(&FixedCheck(true)), person::PAIR, &e.digest(), "", T).unwrap();
            e.sign(&root.public().id(), &root, Some(block)).unwrap();
            chain.offer(e).unwrap();
            devices.insert(*n, d);
            machines.insert(*n, seed(150 + i as u8));
        }
        Hosts { keys: names.iter().map(|n| (*n, HostKey::generate().unwrap())).collect(), devices, machines, person: Arc::new(std::sync::RwLock::new(chain)) }
    }

    fn policy(&self) -> NetworkPolicy {
        NetworkPolicy::with(Arc::new(Shared(self.person.clone())))
    }

    fn roster(&self) -> Roster {
        Roster::with_policy(self.policy())
    }

    fn owner(&self) -> String {
        self.person.read().unwrap().id().unwrap().to_string()
    }

    fn head(&self) -> String {
        self.person.read().unwrap().head().unwrap()
    }

    fn r(&self, id: &'static str, window: bool) -> HostRef {
        HostRef { id: id.into(), keys: *self.keys[id].public(), window, device: Some(self.devices[id].public().id()), machine: self.machines[id].public() }
    }

    fn found(&self) -> Entry {
        found(NET, self.r("studio", true), &self.keys["studio"], Some(self.owner()), Some(self.head()), None, 1)
    }

    fn draft(&self, r: &Roster, op: Op, host: HostRef) -> Entry {
        let gateway = (op == Op::Gateway).then(|| GatewayAt { address: format!("{}.example.net", host.id), version: r.view().gateway_version + 1 });
        let mut e = r.draft(op, host, gateway, 1000 + r.len() as i64);
        e.person_head = Some(self.head());
        e
    }

    /// `s` signs `e`, with its device's presence for what `e` needs.
    fn sign(&self, r: &Roster, e: &mut Entry, s: &'static str) {
        let presence = e.presence_for(&r.view()).map(|what| self.block(s, what, e));
        e.sign_with(s, &self.keys[s], presence);
    }

    fn block(&self, s: &'static str, what: &str, e: &Entry) -> PresenceBlock {
        prove(&self.devices[s], Some(&FixedCheck(true)), what, &e.digest(), "", T).unwrap()
    }

    fn entry(&self, r: &Roster, op: Op, host: HostRef, signers: &[&'static str]) -> Entry {
        let mut e = self.draft(r, op, host);
        for s in signers {
            self.sign(r, &mut e, s);
        }
        e
    }

    fn settings(&self, r: &Roster, host: &'static str, s: Settings, signers: &[&'static str]) -> Entry {
        let mut e = self.draft(r, Op::Settings, r.host_ref(host).unwrap());
        e.settings = Some(s);
        for by in signers {
            self.sign(r, &mut e, by);
        }
        e
    }
}

/// studio founds; desk and nook are window hosts (added by studio);
/// laptop isn't.
fn network(h: &Hosts) -> Roster {
    let mut r = h.roster();
    r.offer(h.found()).unwrap();
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
fn found_is_entry_0_signed_by_the_founder_alone_and_names_its_owner() {
    let h = hosts();
    let mut r = h.roster();
    let mut e = h.found();
    e.sign_by("desk", &h.keys["desk"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "roster-refused", "alone");
    let mut e = h.found();
    e.signers[0].sig = h.keys["desk"].sign(&e.signing_bytes());
    assert!(r.clone().offer(e).is_err(), "by its own key");
    let e = h.entry(&r, Op::Add, h.r("desk", false), &["desk"]);
    assert!(r.offer(e).is_err(), "a roster starts with found");
    let r = network(&h);
    let again = found(NET, h.r("spare", true), &h.keys["spare"], None, None, None, 9);
    let again = Entry { n: r.len(), prev: r.head().unwrap(), ..again };
    assert!(r.clone().offer(again).is_err(), "once");
    let v = r.view();
    assert_eq!((v.founder.as_str(), v.gateway.as_str(), v.owner.as_deref()), ("studio", "studio", Some(h.owner().as_str())));
    assert_eq!(v.member("desk").unwrap().device, Some(h.devices["desk"].public().id()));
    assert_eq!(v.member("desk").unwrap().machine, h.machines["desk"].public());
    // Only found names an owner.
    let mut e = h.entry(&r, Op::Add, h.r("spare", false), &[]);
    e.owner = Some(h.owner());
    h.sign(&r, &mut e, "studio");
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
}

#[test]
fn an_ordinary_host_is_added_by_any_window_host_present_and_removed_by_one() {
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
fn an_admit_counts_an_admin_only_with_its_own_devices_presence_for_that_entry() {
    let h = hosts();
    let r = network(&h);
    let e = h.draft(&r, Op::Add, h.r("spare", false));
    let try_with = |presence: Option<PresenceBlock>| {
        let mut e = e.clone();
        e.sign_with("desk", &h.keys["desk"], presence);
        r.clone().offer(e)
    };
    // No presence at all.
    assert_eq!(try_with(None).unwrap_err().code, "presence");
    // Another device's, another purpose's, another entry's.
    assert_eq!(try_with(Some(h.block("nook", ADMIT, &e))).unwrap_err().code, "presence");
    assert_eq!(try_with(Some(h.block("desk", WIDEN, &e))).unwrap_err().code, "presence");
    let other = h.draft(&r, Op::Add, h.r("tablet", false));
    assert_eq!(try_with(Some(h.block("desk", ADMIT, &other))).unwrap_err().code, "presence", "a lifted block");
    // Its own, for this entry.
    assert_eq!(try_with(Some(h.block("desk", ADMIT, &e))).unwrap(), Offered::Appended);
    // A presence block survives the chain's JSON, and replay checks it.
    let mut taken = r.clone();
    taken.offer(h.entry(&r, Op::Add, h.r("spare", false), &["desk"])).unwrap();
    let back = Roster::from_json_with(h.policy(), &taken.to_json()).unwrap();
    assert_eq!(back.view(), taken.view());
    let mut v = taken.to_json();
    v[4]["signers"][0]["presence"]["act"] = json!(xenonet_id::codec::b64(&[7u8; 32]));
    assert!(Roster::from_json_with(h.policy(), &v).is_err(), "a changed block");
    // With no person chain to check against, nothing counts.
    assert!(Roster::from_json(&taken.to_json()).is_err());
}

#[test]
fn a_revoked_device_or_a_head_it_wasnt_active_at_doesnt_count() {
    let h = hosts();
    let r = network(&h);
    let before = h.head();
    // desk's device is revoked by the person.
    {
        let mut c = h.person.write().unwrap();
        let mut e = c.next(person::Body { op: person::Op::Revoke(person::Revocation::Device(h.devices["desk"].public().id())), at: T + 99 });
        let block = prove(&h.devices["nook"], Some(&FixedCheck(true)), person::REVOKE, &e.digest(), "", T).unwrap();
        e.sign(&h.devices["nook"].public().id(), &h.devices["nook"], Some(block)).unwrap();
        c.offer(e).unwrap();
    }
    let e = h.entry(&r, Op::Add, h.r("spare", false), &["desk"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "presence", "revoked at the head it names");
    // Named at the old head, it still counts: the chain checks by what an
    // entry signs, and a live receiver refuses an old head itself.
    let mut old = h.draft(&r, Op::Add, h.r("spare", false));
    old.person_head = Some(before);
    h.sign(&r, &mut old, "desk");
    assert_eq!(r.clone().offer(old).unwrap(), Offered::Appended);
    // A head the person chain never had.
    let mut e = h.draft(&r, Op::Add, h.r("spare", false));
    e.person_head = Some(xenonet_id::codec::b64(&[1u8; 32]));
    h.sign(&r, &mut e, "nook");
    assert_eq!(r.clone().offer(e).unwrap_err().code, "presence");
}

#[test]
fn window_hosts_gateway_moves_and_their_removal_need_the_founder_or_two() {
    let h = hosts();
    let r = network(&h);
    let strong = [
        (Op::Add, h.r("tablet", true)),
        (Op::Window, r.host_ref("laptop").map(|x| HostRef { window: true, ..x }).unwrap()),
        (Op::Remove, r.host_ref("desk").unwrap()),
        (Op::Gateway, r.host_ref("desk").unwrap()),
    ];
    for (op, host) in strong {
        let what = format!("{} {}", op.as_str(), host.id);
        let with = |signers: &[&'static str]| r.clone().offer(h.entry(&r, op, host.clone(), signers));
        let one = if host.id == "desk" { "nook" } else { "desk" };
        assert_eq!(with(&[one]).unwrap_err().code, "needs-approval", "{what}: one window host");
        assert_eq!(with(&[one, "laptop"]).unwrap_err().code, "needs-approval", "{what}: an ordinary host doesn't count");
        assert_eq!(with(&["studio"]).unwrap(), Offered::Appended, "{what}: the founder");
        assert_eq!(with(&["desk", "nook"]).unwrap(), Offered::Appended, "{what}: two window hosts");
        // Two admins, one of them without presence, where it's needed.
        let mut e = h.entry(&r, op, host.clone(), &["desk"]);
        e.sign_by("nook", &h.keys["nook"]);
        let needs = e.presence_for(&r.view()).is_some();
        assert_eq!(r.clone().offer(e).map_err(|e| e.code), if needs { Err("presence") } else { Ok(Offered::Appended) }, "{what}");
    }
    // A gateway entry moves the gateway, with a higher version each time.
    let mut r = r;
    let e = h.entry(&r, Op::Gateway, h.r("desk", true), &["studio"]);
    r.offer(e).unwrap();
    assert_eq!((r.view().gateway.as_str(), r.view().gateway_version), ("desk", 1));
    let mut stale = r.draft(Op::Gateway, h.r("nook", true), Some(GatewayAt { address: "x.example".into(), version: 1 }), 5);
    stale.person_head = Some(h.head());
    h.sign(&r, &mut stale, "studio");
    assert_eq!(r.offer(stale).unwrap_err().code, "stale");
}

#[test]
fn settings_narrow_by_one_admin_or_the_machine_and_widen_by_the_founder_or_two() {
    let h = hosts();
    let mut r = network(&h);
    let set = |start, wake| Settings { start, wake };
    let network_both = set(Start::Network, Wake::Network);
    // A widening: one admin waits for a second; the founder alone is enough.
    assert_eq!(r.clone().offer(h.settings(&r, "laptop", network_both.clone(), &["desk"])).unwrap_err().code, "needs-approval");
    assert_eq!(r.clone().offer(h.settings(&r, "laptop", network_both.clone(), &["laptop"])).unwrap_err().code, "needs-approval", "not by itself");
    assert_eq!(r.clone().offer(h.settings(&r, "laptop", network_both.clone(), &["desk", "nook"])).unwrap(), Offered::Appended);
    r.offer(h.settings(&r, "laptop", network_both.clone(), &["studio"])).unwrap();
    let m = r.view().member("laptop").unwrap().clone();
    assert_eq!((m.settings, m.settings_by.map(|b| b.0)), (network_both.clone(), Some(vec!["studio".to_string()])));
    // Widening needs presence from each admin that counts.
    let mut e = h.draft(&r, Op::Settings, r.host_ref("desk").unwrap());
    e.settings = Some(set(Start::Delegate, Wake::Consent));
    e.sign_by("studio", &h.keys["studio"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "presence");
    // Narrowing: any admin, or the machine itself, with no presence.
    let mut narrow = h.draft(&r, Op::Settings, r.host_ref("laptop").unwrap());
    narrow.settings = Some(set(Start::Off, Wake::Consent));
    narrow.sign_by("laptop", &h.keys["laptop"]);
    assert_eq!(r.clone().offer(narrow.clone()).unwrap(), Offered::Appended);
    narrow.signers.clear();
    narrow.sign_by("nook", &h.keys["nook"]);
    r.offer(narrow).unwrap();
    // Settings are start and wake alone: what a delegated run may do is
    // the person's delegation, in their chain. The same settings again
    // change nothing.
    assert_eq!(set(Start::Delegate, Wake::Consent).to_json(), json!({ "start": "delegate", "wake": "consent" }));
    let delegate = set(Start::Delegate, Wake::Consent);
    r.offer(h.settings(&r, "laptop", delegate.clone(), &["studio"])).unwrap();
    assert_eq!(r.clone().offer(h.settings(&r, "laptop", delegate, &["studio"])).unwrap_err().code, "no-change");
    // Stepping back from delegate to the network is a narrowing.
    r.offer(h.settings(&r, "laptop", set(Start::Network, Wake::Consent), &["laptop"])).unwrap();
    // A settings entry about a machine names it as the chain has it.
    let mut e = h.settings(&r, "laptop", set(Start::Off, Wake::Consent), &[]);
    e.host.device = None;
    e.sign_by("laptop", &h.keys["laptop"]);
    assert_eq!(r.offer(e).unwrap_err().code, "no-host");
}

#[test]
fn two_window_hosts_approving_apart_make_one_entry_that_counts() {
    let h = hosts();
    let mut r = network(&h);
    let desk = h.entry(&r, Op::Gateway, h.r("desk", true), &["desk"]);
    assert_eq!(r.clone().offer(desk.clone()).unwrap_err().code, "needs-approval");
    let mut nook = desk.clone();
    nook.signers.clear();
    h.sign(&r, &mut nook, "nook");
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
    let mut e = h.draft(&r, Op::Add, h.r("tablet", false));
    e.n += 1;
    h.sign(&r, &mut e, "desk");
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
    h.sign(&r, &mut e, "studio");
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    let mut e = r.draft(Op::Gateway, h.r("desk", true), Some(GatewayAt { address: String::new(), version: 1 }), 5);
    h.sign(&r, &mut e, "studio");
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request", "nor an empty one");
    // A host id that isn't one.
    let mut e = r.draft(Op::Add, HostRef { id: "Spare Host".into(), ..h.r("spare", false) }, None, 5);
    h.sign(&r, &mut e, "studio");
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    // Five signers, where an entry carries four at most.
    let e = h.entry(&r, Op::Add, h.r("tablet", false), &["studio"]);
    r.offer(e).unwrap();
    let e = h.entry(&r, Op::Add, h.r("spare", true), &["studio", "desk", "nook", "laptop", "tablet"]);
    assert_eq!(r.clone().offer(e).unwrap_err().code, "bad-request");
    let back = Roster::from_json_with(h.policy(), &r.to_json()).unwrap();
    assert_eq!(back.view(), r.view());
}

#[test]
fn the_whole_chain_is_checked_and_survives_its_json() {
    let h = hosts();
    let r = network(&h);
    let back = Roster::from_json_with(h.policy(), &r.to_json()).unwrap();
    assert_eq!(back.view(), r.view());
    assert_eq!(back.head(), r.head());
    let read = |v: &Value| Roster::from_json_with(h.policy(), v);
    // A changed entry breaks its own signature, or the next one's prev.
    let mut v = r.to_json();
    v[2]["host"]["window"] = json!(false);
    assert!(read(&v).is_err());
    let mut v = r.to_json();
    v[1]["at"] = json!(77);
    let err = read(&v).unwrap_err();
    assert_eq!(err.code, "roster-refused", "{err}");
    // An entry dropped from the middle leaves a gap.
    let mut v = r.to_json();
    v.as_array_mut().unwrap().remove(1);
    assert!(read(&v).is_err());
    // A key id that doesn't match the keys.
    let mut v = r.to_json();
    v[1]["host"]["keyId"] = json!("xnk1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert!(read(&v).is_err());
    assert_eq!(r.since(2).len(), 2);
    assert_eq!(r.since(99).len(), 0);
    assert!(format!("{:?}", h.policy()).contains("with person chains"));
}
