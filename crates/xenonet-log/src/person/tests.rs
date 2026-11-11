//! The person chain's rules: each row of the table, presence blocks bound
//! to their entry, forks, replay, and devices that went away.

use super::*;
use crate::chain::Offered;
use xenonet_id::presence::{prove, FixedCheck, PresenceBlock};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};

const T: i64 = 1_792_512_000_000;

fn key(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

/// A P-256 key made here, standing in for a secure element's.
struct P256(ring::signature::EcdsaKeyPair);

impl P256 {
    fn new() -> P256 {
        let rng = ring::rand::SystemRandom::new();
        let alg = &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
        let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(alg, &rng).unwrap();
        P256(ring::signature::EcdsaKeyPair::from_pkcs8(alg, doc.as_ref(), &rng).unwrap())
    }
}

impl Signer for P256 {
    fn public(&self) -> VerifyingKey {
        use ring::signature::KeyPair as _;
        VerifyingKey::P256(self.0.public_key().as_ref().to_vec())
    }
    fn home(&self) -> KeyHome {
        KeyHome::Hardware
    }
    fn sign(&self, purpose: Purpose, msg: &[u8]) -> xenonet_id::R<Vec<u8>> {
        Ok(self.0.sign(&ring::rand::SystemRandom::new(), &purpose.bytes(msg)).unwrap().as_ref().to_vec())
    }
    fn asks_person(&self) -> bool {
        true
    }
}

fn block(device: &dyn Signer, what: &str, e: &Entry) -> PresenceBlock {
    prove(device, Some(&FixedCheck(true)), what, &e.digest(), "", T + 5).unwrap()
}

/// `signer` signs `e`, with a presence block from `present` (itself, for
/// a device).
fn sign(e: &mut Entry, signer: &dyn Signer, present: Option<&dyn Signer>) {
    let presence = present.map(|d| block(d, e.op.presence_for(), e));
    e.sign(&signer.public().id(), signer, presence).unwrap();
}

fn add(c: &PersonChain, device: &dyn Signer, kind: DeviceKind, label: &str) -> Entry {
    c.next(Body { op: Op::AddDevice(NewDevice { key: device.public(), kind, label: label.into() }), at: T + c.len() as i64 })
}

fn remove(c: &PersonChain, device: &dyn Signer) -> Entry {
    c.next(Body { op: Op::RemoveDevice { device: device.public().id() }, at: T + c.len() as i64 })
}

fn revoke(c: &PersonChain, what: Revocation) -> Entry {
    c.next(Body { op: Op::Revoke(what), at: T + c.len() as i64 })
}

const TOKEN: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

/// The root key (1) founds; entry 1 pairs laptop (2) under its own
/// presence; entry 2 pairs desk (3) under laptop's.
fn person() -> (PersonChain, SeedSigner, SeedSigner, SeedSigner) {
    let (root, laptop, desk) = (key(1), key(2), key(3));
    let mut c = PersonChain::new();
    c.offer(found(&root, T).unwrap()).unwrap();
    let mut e = add(&c, &laptop, DeviceKind::Biometric, "laptop");
    sign(&mut e, &root, Some(&laptop));
    c.offer(e).unwrap();
    let mut e = add(&c, &desk, DeviceKind::Weaker, "desk");
    sign(&mut e, &root, Some(&laptop));
    c.offer(e).unwrap();
    (c, root, laptop, desk)
}

#[test]
fn found_is_entry_0_signed_by_the_root_alone() {
    let (root, other) = (key(1), key(9));
    let c = PersonChain::new();
    let e = found(&root, T).unwrap();
    let mut both = e.clone();
    sign(&mut both, &other, None);
    assert_eq!(c.clone().offer(both).unwrap_err().code, "person-refused", "alone");
    let mut forged = e.clone();
    forged.signers[0].sig = other.sign(Purpose::PERSON, b"x").unwrap();
    assert_eq!(c.clone().offer(forged).unwrap_err().message, "the root key signs the first entry, alone");
    let mut wrong_purpose = e.clone();
    wrong_purpose.signers[0].sig = root.sign(Purpose::ROSTER, &e.signing_bytes()[Purpose::PERSON.tag().len() + 1..]).unwrap();
    assert!(c.clone().offer(wrong_purpose).is_err(), "a roster signature isn't a person entry's");
    let mut with_presence = e.clone();
    with_presence.signers[0].presence = Some(block(&other, "pair", &e));
    assert!(c.clone().offer(with_presence).is_err());
    let mut first = c.next(Body { op: Op::Revoke(Revocation::Token(TOKEN.into())), at: T });
    sign(&mut first, &root, None);
    assert!(c.clone().offer(first).unwrap_err().message.contains("starts with"));

    let mut c = c;
    assert_eq!(c.offer(e.clone()).unwrap(), Offered::Appended);
    let p = c.person().unwrap();
    assert_eq!(p.root, root.public());
    assert!(is_person_id(&p.id), "{}", p.id);
    assert_eq!(p.id, person_id(&e));
    assert_eq!(c.id(), Some(p.id.as_str()));
    // Another root, another person.
    assert_ne!(person_id(&found(&other, T).unwrap()), p.id);
    // A salt of its own, kept; and the same root with another salt is
    // another person.
    assert_ne!(p.salt, [0; SALT_BYTES]);
    assert_eq!(found_salted(&root, [3; SALT_BYTES], T).unwrap().op, Op::Found { root: root.public(), salt: [3; SALT_BYTES] });
    assert_ne!(person_id(&found(&root, T).unwrap()), p.id, "a new salt each founding");
    let mut unsalted = e.to_json();
    unsalted.as_object_mut().unwrap().remove("salt");
    assert!(Entry::parse(&unsalted).is_err(), "an entry 0 with no salt is refused");
    // Once.
    let mut again = c.next(Body { op: Op::Found { root: other.public(), salt: [1; SALT_BYTES] }, at: T });
    sign(&mut again, &root, None);
    assert!(c.offer(again).unwrap_err().message.contains("one root key"));
}

#[test]
fn the_first_device_is_paired_by_the_root_under_its_own_presence() {
    let (root, laptop) = (key(1), key(2));
    let mut c = PersonChain::new();
    c.offer(found(&root, T).unwrap()).unwrap();
    let e = add(&c, &laptop, DeviceKind::Biometric, "laptop");
    let with = |signer: &dyn Signer, present: Option<&dyn Signer>| {
        let mut e = e.clone();
        sign(&mut e, signer, present);
        c.clone().offer(e)
    };
    let err = with(&root, None).unwrap_err();
    assert_eq!((err.code, err.message.as_str()), ("person-refused", "the root key's entries carry a device's presence"));
    let err = with(&root, Some(&key(7))).unwrap_err();
    assert!(err.message.contains("doesn't hold"), "a stranger's presence: {err}");
    let err = with(&laptop, Some(&laptop)).unwrap_err();
    assert!(err.message.contains("isn't one of this person's keys"), "the new device can't sign itself in: {err}");
    // A block for `revoke` doesn't pair.
    let mut wrong = e.clone();
    let b = prove(&laptop, Some(&FixedCheck(true)), "revoke", &wrong.digest(), "", T).unwrap();
    wrong.sign(&root.public().id(), &root, Some(b)).unwrap();
    assert!(c.clone().offer(wrong).unwrap_err().message.contains("approves revoke"));
    assert_eq!(with(&root, Some(&laptop)).unwrap(), Offered::Appended);
}

#[test]
fn a_device_is_added_or_removed_by_the_root_or_two_devices() {
    let (c, root, laptop, desk) = person();
    let tablet = key(4);
    for e in [add(&c, &tablet, DeviceKind::Biometric, "tablet"), remove(&c, &desk)] {
        let what = e.op.as_str();
        let with = |signers: &[(&dyn Signer, Option<&dyn Signer>)]| {
            let mut e = e.clone();
            for (s, p) in signers {
                sign(&mut e, *s, *p);
            }
            c.clone().offer(e)
        };
        assert_eq!(with(&[(&laptop, Some(&laptop))]).unwrap_err().code, "needs-approval", "{what}: one device");
        assert_eq!(with(&[(&laptop, None)]).unwrap_err().code, "person-refused", "{what}: a device without presence");
        assert_eq!(with(&[(&laptop, Some(&laptop)), (&desk, None)]).unwrap_err().code, "person-refused", "{what}: the second without presence");
        assert!(with(&[(&laptop, Some(&laptop)), (&desk, Some(&laptop))]).unwrap_err().message.contains("another device's"), "{what}: one device's presence twice");
        assert_eq!(with(&[(&laptop, Some(&laptop)), (&desk, Some(&desk))]).unwrap(), Offered::Appended, "{what}: two devices");
        assert_eq!(with(&[(&root, Some(&desk))]).unwrap(), Offered::Appended, "{what}: the root with a device's presence");
        assert_eq!(with(&[(&root, Some(&desk)), (&laptop, Some(&laptop))]).unwrap(), Offered::Appended, "{what}: more than enough");
    }
    let p = c.person().unwrap();
    assert_eq!(p.active_devices().count(), 2);
    assert_eq!(p.device(&laptop.public().id()).unwrap().kind, DeviceKind::Biometric);
    assert_eq!(p.device(&desk.public().id()).unwrap().kind, DeviceKind::Weaker);
}

#[test]
fn two_devices_approving_apart_make_one_entry_that_counts() {
    let (mut c, _, laptop, desk) = person();
    let base = add(&c, &key(4), DeviceKind::Biometric, "tablet");
    let mut a = base.clone();
    sign(&mut a, &laptop, Some(&laptop));
    assert_eq!(c.clone().offer(a.clone()).unwrap_err().code, "needs-approval");
    let mut b = base.clone();
    sign(&mut b, &desk, Some(&desk));
    a.merge_signers(&b);
    a.merge_signers(&b);
    assert_eq!(a.signers.len(), 2);
    assert_eq!(c.offer(a).unwrap(), Offered::Appended);
}

#[test]
fn a_device_key_comes_once_and_never_as_the_root() {
    let (mut c, root, laptop, desk) = person();
    let mut e = add(&c, &laptop, DeviceKind::Weaker, "again");
    sign(&mut e, &root, Some(&desk));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "taken");
    let mut e = add(&c, &root, DeviceKind::Weaker, "root");
    sign(&mut e, &root, Some(&desk));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "taken");
    // Removed, it isn't paired again: a machine that comes back makes a
    // new key.
    let mut e = remove(&c, &desk);
    sign(&mut e, &root, Some(&laptop));
    c.offer(e).unwrap();
    let mut e = add(&c, &desk, DeviceKind::Weaker, "desk");
    sign(&mut e, &root, Some(&laptop));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "taken");
    let mut e = remove(&c, &desk);
    sign(&mut e, &root, Some(&laptop));
    assert_eq!(c.offer(e).unwrap_err().code, "no-device", "removed twice");
}

#[test]
fn one_device_with_presence_revokes_a_token_or_a_device() {
    let (mut c, root, laptop, desk) = person();
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &desk, None);
    assert_eq!(c.clone().offer(e).unwrap_err().code, "person-refused", "without presence");
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    assert!(c.person().unwrap().is_revoked(TOKEN));
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &laptop, Some(&laptop));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "no-change");
    // The root, with a device's presence, may too.
    let other = "ff".repeat(32);
    let mut e = revoke(&c, Revocation::Token(other.clone()));
    sign(&mut e, &root, Some(&laptop));
    c.offer(e).unwrap();
    // A stolen laptop, revoked from the desk.
    let mut e = revoke(&c, Revocation::Device(laptop.public().id()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    let d = c.person().unwrap().device(&laptop.public().id()).unwrap();
    assert!(!d.active() && d.revoked.is_some());
    let mut e = revoke(&c, Revocation::Device(laptop.public().id()));
    sign(&mut e, &desk, Some(&desk));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "no-change");
    let mut e = revoke(&c, Revocation::Device(key(8).public().id()));
    sign(&mut e, &desk, Some(&desk));
    assert_eq!(c.offer(e).unwrap_err().code, "no-device");
}

#[test]
fn a_device_that_went_away_cant_sign_or_vouch() {
    let (mut c, root, laptop, desk) = person();
    let mut e = revoke(&c, Revocation::Device(laptop.public().id()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    // Revoked: its signature, its presence for the root, its half of two.
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &laptop, Some(&laptop));
    assert!(c.clone().offer(e).unwrap_err().message.contains("was revoked"));
    let mut e = add(&c, &key(4), DeviceKind::Biometric, "tablet");
    sign(&mut e, &root, Some(&laptop));
    assert!(c.clone().offer(e).unwrap_err().message.contains("doesn't hold"));
    let mut e = add(&c, &key(4), DeviceKind::Biometric, "tablet");
    sign(&mut e, &laptop, Some(&laptop));
    sign(&mut e, &desk, Some(&desk));
    assert!(c.clone().offer(e).is_err());
    // A revoked device may still be removed, to tidy up.
    let mut e = remove(&c, &laptop);
    sign(&mut e, &root, Some(&desk));
    c.offer(e).unwrap();
    // Removed: the same.
    let mut e = remove(&c, &desk);
    sign(&mut e, &root, Some(&desk));
    c.offer(e).unwrap();
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &desk, Some(&desk));
    assert!(c.clone().offer(e).unwrap_err().message.contains("was removed"));
    assert_eq!(c.person().unwrap().active_devices().count(), 0);
    // With no device left, the root pairs a new one under that one's own
    // presence.
    let tablet = P256::new();
    let mut e = add(&c, &tablet, DeviceKind::Biometric, "tablet");
    sign(&mut e, &root, Some(&tablet));
    c.offer(e).unwrap();
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &tablet, Some(&tablet));
    c.offer(e).unwrap();
}

#[test]
fn a_presence_block_lifted_off_another_entry_is_refused() {
    let (c, root, laptop, desk) = person();
    let tablet = key(4);
    let mut other = remove(&c, &desk);
    sign(&mut other, &root, Some(&laptop));
    let lifted = other.signers[0].presence.clone().unwrap();
    // Onto an entry that says something else, at the same place.
    let mut e = remove(&c, &laptop);
    e.sign(&root.public().id(), &root, Some(lifted.clone())).unwrap();
    assert!(c.clone().offer(e).unwrap_err().message.contains("another act"));
    // Onto the same op at another time: another entry, another id.
    let mut later = remove(&c, &desk);
    later.at += 1;
    later.sign(&root.public().id(), &root, Some(lifted.clone())).unwrap();
    assert!(c.clone().offer(later).unwrap_err().message.contains("another act"));
    // From another person's chain, where the same entry would follow
    // another head.
    let mut theirs = PersonChain::new();
    theirs.offer(found(&key(11), T).unwrap()).unwrap();
    let mut pair = add(&theirs, &laptop, DeviceKind::Biometric, "laptop");
    sign(&mut pair, &key(11), Some(&laptop));
    let mut e = add(&c, &tablet, DeviceKind::Biometric, "tablet");
    e.sign(&root.public().id(), &root, pair.signers[0].presence.clone()).unwrap();
    assert!(c.clone().offer(e).is_err());
    // A changed block doesn't check.
    let mut e = remove(&c, &desk);
    let mut b = block(&laptop, REVOKE, &e);
    b.at += 1;
    e.sign(&root.public().id(), &root, Some(b)).unwrap();
    assert!(c.clone().offer(e).unwrap_err().message.contains("signature"));
    // A block that names its own device on a signer that isn't it.
    let mut e = add(&c, &tablet, DeviceKind::Biometric, "tablet");
    let b = block(&laptop, PAIR, &e);
    sign(&mut e, &laptop, Some(&laptop));
    e.sign(&desk.public().id(), &desk, Some(b)).unwrap();
    assert!(c.clone().offer(e).unwrap_err().message.contains("another device's"));
    // The age isn't the chain's to check: a block made a day before is
    // taken as the entry it signs.
    let mut e = remove(&c, &desk);
    let old = prove(&laptop, Some(&FixedCheck(true)), REVOKE, &e.digest(), "", T - 86_400_000).unwrap();
    e.sign(&root.public().id(), &root, Some(old)).unwrap();
    c.clone().offer(e).unwrap();
}

#[test]
fn duplicates_are_dropped_and_forks_and_gaps_refused() {
    let (mut c, root, laptop, desk) = person();
    let base = c.clone();
    let mut a = revoke(&base, Revocation::Token(TOKEN.into()));
    sign(&mut a, &laptop, Some(&laptop));
    let mut b = revoke(&base, Revocation::Token(TOKEN.into()));
    sign(&mut b, &desk, Some(&desk));
    assert_eq!(c.offer(a).unwrap(), Offered::Appended);
    assert_eq!(c.offer(b).unwrap(), Offered::Duplicate, "the same revocation, signed apart");
    let mut f = revoke(&base, Revocation::Device(desk.public().id()));
    sign(&mut f, &laptop, Some(&laptop));
    assert_eq!(c.clone().offer(f).unwrap_err().code, "person-fork");
    let mut g = remove(&c, &desk);
    g.n += 1;
    sign(&mut g, &root, Some(&laptop));
    assert_eq!(c.clone().offer(g).unwrap_err().code, "person-gap");
    let mut p = remove(&c, &desk);
    p.prev = base.head().unwrap();
    sign(&mut p, &root, Some(&laptop));
    assert_eq!(c.offer(p).unwrap_err().code, "person-fork");
    // Five signers, where an entry carries four at most; one twice.
    let mut e = revoke(&c, Revocation::Device(desk.public().id()));
    for s in [&laptop, &desk, &root, &key(5), &key(6)] {
        sign(&mut e, s, Some(&laptop));
    }
    assert_eq!(c.clone().offer(e).unwrap_err().code, "bad-request");
    let mut e = revoke(&c, Revocation::Device(desk.public().id()));
    sign(&mut e, &laptop, Some(&laptop));
    let twice = e.signers[0].clone();
    e.signers.push(twice);
    assert_eq!(c.offer(e).unwrap_err().message, "an entry is signed by each key once");
}

#[test]
fn the_whole_chain_replays_from_its_json() {
    let (mut c, root, laptop, desk) = person();
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    let tablet = P256::new();
    let mut e = add(&c, &tablet, DeviceKind::Biometric, "tablet");
    sign(&mut e, &laptop, Some(&laptop));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    let mut e = remove(&c, &laptop);
    sign(&mut e, &root, Some(&tablet));
    c.offer(e).unwrap();
    let v = c.to_json();
    let back = PersonChain::from_json(&v).unwrap();
    assert_eq!(back.person(), c.person());
    assert_eq!(back.head(), c.head());
    for e in c.entries() {
        assert_eq!(Entry::parse(&e.to_json()).as_ref(), Ok(e), "an entry reads back as another");
    }
    // A changed entry breaks its signature, its presence or the next prev.
    let mut w = v.clone();
    w[1]["device"]["label"] = Value::from("work laptop");
    assert!(PersonChain::from_json(&w).is_err());
    let mut w = v.clone();
    w[3]["revoke"]["token"] = Value::from("ff".repeat(32));
    assert!(PersonChain::from_json(&w).is_err());
    let mut w = v.clone();
    w[2]["signers"][0]["presence"]["at"] = Value::from(1);
    assert!(PersonChain::from_json(&w).is_err());
    let mut w = v.clone();
    w[2]["signers"][0].as_object_mut().unwrap().remove("presence");
    assert_eq!(PersonChain::from_json(&w).unwrap_err().code, "person-refused", "a presence dropped");
    let mut w = v.clone();
    w.as_array_mut().unwrap().remove(2);
    assert_eq!(PersonChain::from_json(&w).unwrap_err().code, "person-gap");
    let mut w = v.clone();
    w[1]["device"]["id"] = Value::from(key(9).public().id());
    assert_eq!(PersonChain::from_json(&w).unwrap_err().code, "bad-request");
    let mut w = v;
    w[1]["device"]["kind"] = Value::from("strong");
    assert_eq!(PersonChain::from_json(&w).unwrap_err().code, "bad-request");
}

#[test]
fn a_device_and_a_token_are_looked_up_at_a_past_head() {
    let (mut c, root, laptop, desk) = person();
    let at_desk = c.head().unwrap();
    let mut e = revoke(&c, Revocation::Device(laptop.public().id()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    let at_revoked = c.head().unwrap();
    let mut e = revoke(&c, Revocation::Token(TOKEN.into()));
    sign(&mut e, &desk, Some(&desk));
    c.offer(e).unwrap();
    let mut e = remove(&c, &desk);
    sign(&mut e, &root, Some(&desk));
    c.offer(e).unwrap();
    let genesis = c.entries()[0].hash();
    let (l, d) = (laptop.public().id(), desk.public().id());
    assert!(c.device_at(&l, &genesis).is_none(), "not paired yet");
    assert!(c.device_at(&l, &at_desk).is_some());
    assert!(c.device_at(&d, &at_desk).is_some(), "active from the entry that added it");
    assert!(c.device_at(&l, &at_revoked).is_none(), "revoked by then");
    assert!(c.device_at(&d, &at_revoked).is_some());
    assert!(c.device_at(&d, &c.head().unwrap()).is_none(), "removed by then");
    assert!(c.device_at(&d, "not-a-head").is_none());
    assert!(!c.revoked_at(TOKEN, &at_revoked));
    assert!(c.revoked_at(TOKEN, &c.head().unwrap()));
    // Through what another chain's policy holds.
    let devices: std::sync::Arc<dyn Devices> = std::sync::Arc::new(c.clone());
    let id = c.id().unwrap().to_string();
    assert_eq!(devices.person_device(&id, &at_desk, &l).map(|d| d.label), Some("laptop".into()));
    assert!(devices.person_device("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &at_desk, &l).is_none(), "another person");
}

#[test]
fn an_entry_that_couldnt_be_read_back_is_refused() {
    let (c, root, laptop, _) = person();
    for label in [String::new(), "x".repeat(LABEL_MAX + 1), "line\nbreak".into()] {
        let mut e = add(&c, &key(4), DeviceKind::Biometric, &label);
        sign(&mut e, &root, Some(&laptop));
        assert_eq!(c.clone().offer(e).unwrap_err().code, "bad-request", "{label:?}");
    }
    for token in ["", "abc", "ABCDEF0123456789ABCDEF0123456789", &"0".repeat(258)] {
        let mut e = revoke(&c, Revocation::Token(token.into()));
        sign(&mut e, &laptop, Some(&laptop));
        assert_eq!(c.clone().offer(e).unwrap_err().code, "bad-request", "{token:?}");
    }
    let mut e = revoke(&c, Revocation::Device("laptop".into()));
    sign(&mut e, &laptop, Some(&laptop));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "bad-request");
}

fn delegation(devices: DeviceSet, work: &[&str], prompts: &[&str], until: i64) -> Delegation {
    Delegation {
        id: "dlg_0123456789abcdef".into(),
        devices,
        work: work.iter().map(ToString::to_string).collect(),
        prompts: prompts.iter().map(ToString::to_string).collect(),
        limits: LIMITS[0].into(),
        until,
    }
}

fn delegate(c: &PersonChain, d: Delegation) -> Entry {
    c.next(Body { op: Op::Delegate(d), at: T + c.len() as i64 })
}

#[test]
fn a_delegation_is_issued_and_widened_with_presence_and_narrowed_or_ended_without() {
    let (mut c, root, laptop, desk) = person();
    let wide = delegation(DeviceSet::All, &["answer", "start"], &["command", "edit", "read"], T + 86_400_000);
    // Issuing widens: a device's signature alone isn't enough.
    let mut e = delegate(&c, wide.clone());
    sign(&mut e, &laptop, None);
    assert!(c.clone().offer(e).unwrap_err().message.contains("presence"));
    // With its presence, one device issues it; so does the root with a
    // device's.
    let mut by_root = delegate(&c, wide.clone());
    sign(&mut by_root, &root, Some(&laptop));
    assert_eq!(c.clone().offer(by_root).unwrap(), Offered::Appended);
    let mut e = delegate(&c, wide.clone());
    sign(&mut e, &laptop, Some(&laptop));
    c.offer(e).unwrap();
    let p = c.person().unwrap();
    assert_eq!(p.delegations_of(&laptop.public().id()).collect::<Vec<_>>(), vec![&wide]);
    assert_eq!(p.delegations_of(&desk.public().id()).count(), 1, "all devices, the weaker one too");
    // A block for another op's word doesn't count.
    let mut lifted = delegate(&c, Delegation { until: T + 2 * 86_400_000, ..wide.clone() });
    let b = prove(&laptop, Some(&FixedCheck(true)), REVOKE, &lifted.digest(), "", T).unwrap();
    lifted.sign(&laptop.public().id(), &laptop, Some(b)).unwrap();
    assert!(c.clone().offer(lifted).unwrap_err().message.contains("approves revoke"));
    // Later, more prompts or another kind of work: each widens.
    for wider in [Delegation { until: T + 2 * 86_400_000, ..wide.clone() }, delegation(DeviceSet::All, &["answer", "start"], &["command", "edit", "read", "web"], T + 86_400_000)] {
        let mut e = delegate(&c, wider);
        sign(&mut e, &desk, None);
        assert!(c.clone().offer(e).unwrap_err().message.contains("presence"));
    }
    // Narrowing: fewer devices, less work, fewer prompts, sooner. No
    // presence, by any device of the person's.
    let narrow = delegation(DeviceSet::Only(vec![laptop.public().id()]), &["answer"], &["read"], T + 3_600_000);
    let mut e = delegate(&c, narrow.clone());
    sign(&mut e, &desk, None);
    c.offer(e).unwrap();
    let p = c.person().unwrap();
    assert_eq!(p.delegations_of(&laptop.public().id()).collect::<Vec<_>>(), vec![&narrow]);
    assert_eq!(p.delegations_of(&desk.public().id()).count(), 0);
    // From named devices back to all of them widens again.
    let mut e = delegate(&c, Delegation { devices: DeviceSet::All, ..narrow.clone() });
    sign(&mut e, &desk, None);
    assert!(c.clone().offer(e).unwrap_err().message.contains("presence"));
    // The same again changes nothing; ending it needs no presence.
    let mut same = delegate(&c, narrow.clone());
    sign(&mut same, &desk, None);
    assert_eq!(c.clone().offer(same).unwrap_err().code, "no-change");
    let mut end = c.next(Body { op: Op::Undelegate { id: narrow.id.clone() }, at: T + 50 });
    sign(&mut end, &desk, None);
    c.offer(end).unwrap();
    assert!(c.person().unwrap().delegations.is_empty());
    let mut again = c.next(Body { op: Op::Undelegate { id: narrow.id.clone() }, at: T + 51 });
    sign(&mut again, &desk, None);
    assert_eq!(c.offer(again).unwrap_err().code, "no-delegation");
}

#[test]
fn a_delegation_names_only_devices_the_person_holds_and_reads_back() {
    let (mut c, _root, laptop, desk) = person();
    let stranger = key(9);
    let mut e = delegate(&c, delegation(DeviceSet::Only(vec![stranger.public().id()]), &["start"], &[], T + 1000));
    sign(&mut e, &laptop, Some(&laptop));
    assert_eq!(c.clone().offer(e).unwrap_err().code, "no-device");
    // Lists are sorted, without repeats, and of the known words.
    let mut ids = [laptop.public().id(), desk.public().id()];
    ids.sort();
    for bad in [
        delegation(DeviceSet::Only(vec![ids[1].clone(), ids[0].clone()]), &["start"], &[], T),
        delegation(DeviceSet::All, &["start", "answer"], &[], T),
        delegation(DeviceSet::All, &[], &[], T),
        delegation(DeviceSet::All, &["start"], &["read", "read"], T),
        delegation(DeviceSet::All, &["fly"], &[], T),
        Delegation { id: "dlg_x".into(), ..delegation(DeviceSet::All, &["start"], &[], T) },
    ] {
        let mut e = delegate(&c, bad);
        sign(&mut e, &laptop, Some(&laptop));
        assert!(c.clone().offer(e).is_err());
    }
    let d = delegation(DeviceSet::Only(ids.to_vec()), &["answer", "start"], &["command", "read"], T + 1000);
    let mut e = delegate(&c, d.clone());
    sign(&mut e, &laptop, Some(&laptop));
    c.offer(e).unwrap();
    // A device that's revoked drops out of what it names.
    let mut r = revoke(&c, Revocation::Device(desk.public().id()));
    sign(&mut r, &laptop, Some(&laptop));
    c.offer(r).unwrap();
    assert_eq!(c.person().unwrap().delegations_of(&desk.public().id()).count(), 0);
    assert_eq!(c.person().unwrap().delegations_of(&laptop.public().id()).count(), 1);
    // The whole chain reads back the same.
    let back = PersonChain::from_json(&c.to_json()).unwrap();
    assert_eq!(back.person(), c.person());
    assert_eq!(back.head(), c.head());
}
