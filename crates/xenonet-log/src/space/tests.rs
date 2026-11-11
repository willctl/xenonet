//! The space chain's rules: each op by each kind of signer with and
//! without presence, a curator's fork, competing takes, an unshare beside
//! a share made in a partition, and members that saw different branches.

use super::*;
use crate::person::{self, DeviceKind, NewDevice, PersonChain};
use xenonet_id::sign::{KeyHome, SeedSigner};

const T: i64 = 1_792_512_000_000;

fn key(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

/// A presence block from `device` for a space act whose digest is `act`.
fn block(device: &SeedSigner, act: [u8; 32]) -> PresenceBlock {
    let mut b = PresenceBlock { device: device.public().id(), what: WIDEN.into(), act, nonce: [9; 16], at: T, sig: Vec::new() };
    b.sig = device.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
    b
}

/// The owner's person chain (root 1; devices 2 and 3; 4 paired and then
/// removed), the curators (10, 11, 12), a stranger (20) and the space,
/// founded by curator 10 about the owner and owned by them.
struct World {
    person: PersonChain,
    d1: SeedSigner,
    d2: SeedSigner,
    gone: SeedSigner,
    c1: SeedSigner,
    c2: SeedSigner,
    c3: SeedSigner,
    stranger: SeedSigner,
}

impl World {
    fn new() -> World {
        let root = key(1);
        let (d1, d2, gone) = (key(2), key(3), key(4));
        let mut c = PersonChain::new();
        c.offer(person::found_salted(&root, [0x5a; person::SALT_BYTES], T - 1000).unwrap()).unwrap();
        for d in [&d1, &d2, &gone] {
            let mut add = c.next(person::Body { op: person::Op::AddDevice(NewDevice { key: d.public(), kind: DeviceKind::Weaker, label: "a device".into() }), at: T - 900 });
            let mut b = PresenceBlock { device: d.public().id(), what: person::PAIR.into(), act: add.digest(), nonce: [1; 16], at: T, sig: Vec::new() };
            b.sig = d.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
            add.sign(&root.public().id(), &root, Some(b)).unwrap();
            c.offer(add).unwrap();
        }
        let mut rm = c.next(person::Body { op: person::Op::RemoveDevice { device: gone.public().id() }, at: T - 800 });
        let mut b = PresenceBlock { device: d1.public().id(), what: person::REVOKE.into(), act: rm.digest(), nonce: [2; 16], at: T, sig: Vec::new() };
        b.sig = d1.sign(Purpose::PRESENCE, &b.signed_bytes()).unwrap();
        rm.sign(&root.public().id(), &root, Some(b)).unwrap();
        c.offer(rm).unwrap();
        World { person: c, d1, d2, gone, c1: key(10), c2: key(11), c3: key(12), stranger: key(20) }
    }

    fn pid(&self) -> String {
        self.person.id().unwrap().to_string()
    }

    fn head(&self) -> Option<String> {
        self.person.head()
    }

    fn policy(&self) -> SpacePolicy {
        SpacePolicy::new(Some(Arc::new(self.person.clone())), None)
    }

    fn genesis(&self, about: EntityRef) -> Entry {
        found(about, Owner::Person(self.pid()), &self.c1, None, "will", [7; 16], T).unwrap()
    }

    fn space(&self) -> Space {
        Space::found(self.policy(), self.genesis(EntityRef::Person(self.pid()))).unwrap()
    }
}

fn subject(n: u8) -> Subject {
    Subject::Key(key(100 + n).public().id())
}

/// Signs `e` with each of `by`, a presence block from it when asked.
fn signed(mut e: Entry, by: &[(&SeedSigner, bool)]) -> Entry {
    for (k, presence) in by {
        let b = presence.then(|| block(k, e.digest()));
        e.sign(&k.public().id(), *k, b).unwrap();
    }
    e
}

fn signed_side(mut s: Side, by: &[(&SeedSigner, bool)]) -> Side {
    for (k, presence) in by {
        let b = presence.then(|| block(k, s.digest()));
        s.sign(&k.public().id(), *k, b).unwrap();
    }
    s
}

fn share(sp: &Space, w: &World, s: Subject, level: Level, after: Vec<String>) -> Entry {
    sp.next(Op::Share { subject: s, level, endpoint: None, after }, w.head(), T + sp.len() as i64)
}

#[test]
fn a_space_is_founded_by_its_curator_and_named_by_its_genesis() {
    let w = World::new();
    let g = w.genesis(EntityRef::Person(w.pid()));
    let sp = Space::found(w.policy(), g.clone()).unwrap();
    let hash: [u8; 32] = unb64_n(&g.hash()).unwrap();
    assert_eq!(sp.id(), SpaceId::of_genesis(&hash).as_str(), "the id is the model's, from entry 0's hash");
    let v = sp.view();
    assert_eq!((v.about.clone(), v.name.as_str(), v.epoch, v.curator.id()), (EntityRef::Person(w.pid()), "will", 0, w.c1.public().id()));
    assert!(!v.owner_signed, "named as owner, the person hasn't signed anything yet");
    // Another nonce, another space.
    let other = found(EntityRef::Person(w.pid()), Owner::Person(w.pid()), &w.c1, None, "will", [8; 16], T).unwrap();
    assert_ne!(Space::found(w.policy(), other).unwrap().id(), sp.id());
    // Signed by someone else as well, or with a presence: refused.
    let extra = signed(g.clone(), &[(&w.stranger, false)]);
    assert_eq!(Space::found(w.policy(), extra).unwrap_err().code, "space-refused");
    let mut presence = Entry::new(0, "", g.body.clone());
    presence.sign(&w.c1.public().id(), &w.c1, Some(block(&w.c1, g.digest()))).unwrap();
    assert!(Space::found(w.policy(), presence).is_err());
    // A key owner signs its founding too.
    let owner = key(30);
    let g = found(EntityRef::Name { kind: "project".into(), name: "atlas".into() }, Owner::Key(owner.public()), &w.c1, Some(&owner), "atlas", [7; 16], T).unwrap();
    assert_eq!(g.signers.len(), 2);
    let sp = Space::found(SpacePolicy::default(), g.clone()).unwrap();
    assert!(sp.view().weaker() && sp.view().owner_signed);
    let alone = Entry { signers: g.signers[..1].to_vec(), ..g };
    assert!(Space::found(SpacePolicy::default(), alone).is_err(), "a key owner that didn't sign");
}

#[test]
fn entries_read_back_from_their_json() {
    let w = World::new();
    let mut sp = w.space();
    let e = signed(share(&sp, &w, subject(1), Level::Append, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(Entry::parse(&e.to_json()).unwrap(), e);
    sp.offer(e).unwrap();
    let u = signed_side(sp.unshare(subject(1), w.head(), T), &[(&w.d2, false)]);
    assert_eq!(Side::parse(&u.to_json()).unwrap(), u);
    sp.offer_side(u).unwrap();
    let t = signed_side(sp.curator_side(true, w.c2.public(), w.head(), T), &[(&w.d1, true)]);
    assert_eq!(Side::parse(&t.to_json()).unwrap(), t);
    sp.offer_side(t).unwrap();
    let back = Space::from_json(w.policy(), &sp.to_json()).unwrap();
    assert_eq!(back.view(), sp.view());
    assert_eq!(back.level(&subject(1)), None);
}

/// The ops of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Do {
    ShareWiden,
    ShareNarrow,
    Bind,
    Unshare,
    Take,
    Move,
}

/// Who signs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    OwnerAndCurator,
    OtherDeviceAndCurator,
    OwnerAlone,
    CuratorAlone,
    StrangerAndCurator,
    RemovedAndCurator,
    Stranger,
}

const DOS: [Do; 6] = [Do::ShareWiden, Do::ShareNarrow, Do::Bind, Do::Unshare, Do::Take, Do::Move];
const BYS: [By; 7] = [By::OwnerAndCurator, By::OtherDeviceAndCurator, By::OwnerAlone, By::CuratorAlone, By::StrangerAndCurator, By::RemovedAndCurator, By::Stranger];

/// The rules, said plainly: sequenced entries need an active device of the
/// owner's and the curator; what widens needs the device's presence; side
/// entries are the owner's (a move's the curator's too); an unshare takes
/// no presence.
fn expected(what: Do, by: By, presence: bool) -> bool {
    let owner_device = matches!(by, By::OwnerAndCurator | By::OtherDeviceAndCurator | By::OwnerAlone);
    let with_curator = !matches!(by, By::OwnerAlone | By::Stranger);
    let only_owner_and_curator = matches!(by, By::OwnerAndCurator | By::OtherDeviceAndCurator | By::CuratorAlone);
    match what {
        Do::ShareWiden | Do::Bind => owner_device && with_curator && presence,
        Do::ShareNarrow => owner_device && with_curator,
        Do::Unshare => matches!(by, By::OwnerAlone),
        Do::Take => matches!(by, By::OwnerAlone) && presence,
        Do::Move => owner_device && with_curator && only_owner_and_curator && presence,
    }
}

fn run(what: Do, by: By, presence: bool) -> Result<Placed, ChainError> {
    let w = World::new();
    let about = if what == Do::Bind { EntityRef::Name { kind: "identity".into(), name: "will".into() } } else { EntityRef::Person(w.pid()) };
    let mut sp = Space::found(w.policy(), w.genesis(about)).unwrap();
    // A share to narrow, made rightly first.
    let first = signed(share(&sp, &w, subject(1), Level::Write, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(sp.offer(first).unwrap(), Placed::Applied);
    let signers: Vec<(&SeedSigner, bool)> = match by {
        By::OwnerAndCurator => vec![(&w.d1, presence), (&w.c1, false)],
        By::OtherDeviceAndCurator => vec![(&w.d2, presence), (&w.c1, false)],
        By::OwnerAlone => vec![(&w.d1, presence)],
        By::CuratorAlone => vec![(&w.c1, presence)],
        By::StrangerAndCurator => vec![(&w.stranger, presence), (&w.c1, false)],
        By::RemovedAndCurator => vec![(&w.gone, presence), (&w.c1, false)],
        By::Stranger => vec![(&w.stranger, presence)],
    };
    match what {
        Do::ShareWiden => sp.offer(signed(share(&sp, &w, subject(2), Level::Read, vec![]), &signers)),
        Do::ShareNarrow => sp.offer(signed(share(&sp, &w, subject(1), Level::Read, vec![]), &signers)),
        Do::Bind => sp.offer(signed(sp.next(Op::Bind { about: EntityRef::Person(w.pid()) }, w.head(), T), &signers)),
        Do::Unshare => sp.offer_side(signed_side(sp.unshare(subject(1), w.head(), T), &signers)),
        Do::Take => sp.offer_side(signed_side(sp.curator_side(true, w.c2.public(), w.head(), T), &signers)),
        Do::Move => sp.offer_side(signed_side(sp.curator_side(false, w.c2.public(), w.head(), T), &signers)),
    }
}

#[test]
fn every_op_by_every_signer_with_and_without_presence() {
    let mut n = 0;
    for what in DOS {
        for by in BYS {
            for presence in [false, true] {
                let got = run(what, by, presence);
                let want = expected(what, by, presence);
                assert_eq!(got.as_ref().is_ok_and(|p| *p == Placed::Applied), want, "{what:?} by {by:?}, presence {presence}: {got:?}");
                if !want {
                    assert!(got.is_err(), "{what:?} by {by:?}, presence {presence}: refused, not held: {got:?}");
                }
                n += 1;
            }
        }
    }
    assert_eq!(n, 6 * 7 * 2);
}

#[test]
fn the_rows_worth_reading() {
    // Without presence, a widening says so.
    assert_eq!(run(Do::ShareWiden, By::OwnerAndCurator, false).unwrap_err().code, "presence");
    assert_eq!(run(Do::Take, By::OwnerAlone, false).unwrap_err().code, "presence");
    // The owner alone can't extend the chain; the curator sequences.
    assert_eq!(run(Do::ShareWiden, By::OwnerAlone, true).unwrap_err().code, "space-refused");
    // The curator alone has nobody's approval.
    assert_eq!(run(Do::ShareNarrow, By::CuratorAlone, false).unwrap_err().code, "needs-approval");
    // A move without the curator it moves from isn't a move.
    assert_eq!(run(Do::Move, By::OwnerAlone, true).unwrap_err().code, "needs-approval");
    // A device the person removed signs nothing.
    assert_eq!(run(Do::ShareNarrow, By::RemovedAndCurator, false).unwrap_err().code, "space-refused");
}

#[test]
fn a_curators_fork_freezes_the_chain_until_the_owner_takes_it() {
    let w = World::new();
    let mut sp = w.space();
    let a = signed(share(&sp, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    let b = signed(share(&sp, &w, subject(2), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(sp.offer(a.clone()).unwrap(), Placed::Applied);
    assert_eq!(sp.level(&subject(1)), Some(Level::Read));
    // The curator signs another entry at the same place.
    assert_eq!(sp.offer(b).unwrap(), Placed::Held);
    assert_eq!(sp.frozen(), Some(1));
    assert_eq!(sp.len(), 1, "nothing past the fork is applied");
    assert_eq!(sp.level(&subject(1)), None);
    // What follows either side stays held.
    let mut after = Entry::new(
        2,
        a.hash(),
        Body { space: sp.id().into(), epoch: 0, op: Op::Share { subject: subject(3), level: Level::Read, endpoint: None, after: vec![] }, person_head: w.head(), at: T },
    );
    after = signed(after, &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(sp.offer(after).unwrap(), Placed::Held);
    // The owner takes the chain from before the fork to another curator,
    // which sequences from there.
    let take = signed_side(sp.curator_side(true, w.c2.public(), w.head(), T), &[(&w.d1, true)]);
    assert_eq!(sp.offer_side(take).unwrap(), Placed::Applied);
    assert_eq!((sp.frozen(), sp.view().epoch, sp.view().curator.id()), (None, 1, w.c2.public().id()));
    assert!(sp.superseded(&w.c1.public().id()));
    let again = signed(share(&sp, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c2, false)]);
    assert_eq!(sp.offer(again).unwrap(), Placed::Applied);
    assert_eq!(sp.level(&subject(1)), Some(Level::Read));
    // The old curator's entries are refused at the head now.
    let old = signed(share(&sp, &w, subject(4), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(old.epoch, 1);
    assert!(sp.offer(old).is_err());
}

#[test]
fn of_two_takes_in_a_partition_one_curator_wins_everywhere() {
    let w = World::new();
    let mut sp = w.space();
    sp.offer(signed(share(&sp, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    let base = sp.clone();
    // Apart: d1 takes the space to c2, d2 to c3, and c1, not knowing,
    // sequences one more share.
    let t2 = signed_side(base.curator_side(true, w.c2.public(), w.head(), T), &[(&w.d1, true)]);
    let t3 = signed_side(base.curator_side(true, w.c3.public(), w.head(), T + 1), &[(&w.d2, true)]);
    let late = signed(share(&base, &w, subject(2), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    let mut x = base.clone();
    x.offer_side(t2.clone()).unwrap();
    let by_c2 = signed(share(&x, &w, subject(3), Level::Read, vec![]), &[(&w.d1, true), (&w.c2, false)]);
    x.offer(by_c2.clone()).unwrap();
    let mut y = base.clone();
    y.offer_side(t3.clone()).unwrap();
    let by_c3 = signed(share(&y, &w, subject(4), Level::Read, vec![]), &[(&w.d2, true), (&w.c3, false)]);
    y.offer(by_c3.clone()).unwrap();
    // After the heal each has everything, in another order.
    x.offer(late.clone()).unwrap();
    x.offer_side(t3.clone()).unwrap();
    x.offer(by_c3.clone()).unwrap();
    y.offer(by_c2.clone()).unwrap();
    y.offer(late.clone()).unwrap();
    y.offer_side(t2.clone()).unwrap();
    assert_eq!(x.view(), y.view(), "every member applies the same rule");
    let winner = if t2.digest() < t3.digest() { &w.c2 } else { &w.c3 };
    assert_eq!(x.view().curator.id(), winner.public().id());
    assert_eq!(x.curators_by(), [if t2.digest() < t3.digest() { t2.id() } else { t3.id() }]);
    // The late share by c1 was after the base: superseded, held as
    // evidence, never applied.
    assert_eq!(x.level(&subject(2)), None);
    assert!(x.held().iter().any(|e| e.hash() == late.hash()));
    // The winner's share is in; the loser's isn't.
    let (won, lost) = if t2.digest() < t3.digest() { (subject(3), subject(4)) } else { (subject(4), subject(3)) };
    assert_eq!((x.level(&won), x.level(&lost)), (Some(Level::Read), None));
}

#[test]
fn a_share_made_beside_an_unshare_leaves_the_unshare_in_force() {
    let w = World::new();
    let mut a = w.space();
    a.offer(signed(share(&a, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    let mut b = a.clone();
    // Apart: on one side the owner widens subject 1 to write, through the
    // curator; on the other, from another device, unshares it.
    let wider = signed(share(&a, &w, subject(1), Level::Write, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    a.offer(wider.clone()).unwrap();
    assert_eq!(a.level(&subject(1)), Some(Level::Write));
    let un = signed_side(b.unshare(subject(1), w.head(), T), &[(&w.d2, false)]);
    b.offer_side(un.clone()).unwrap();
    assert_eq!(b.level(&subject(1)), None);
    // Healed: the unshare stands on both.
    a.offer_side(un.clone()).unwrap();
    b.offer(wider).unwrap();
    assert_eq!((a.level(&subject(1)), b.level(&subject(1))), (None, None));
    assert_eq!(a.view(), b.view());
    assert!(a.unshared_at(&subject(1)).is_some());
    // Sharing again is a share that lists the unshare as seen.
    assert_eq!(a.unshares_of(&subject(1)), vec![un.id()]);
    let again = signed(share(&a, &w, subject(1), Level::Read, a.unshares_of(&subject(1))), &[(&w.d1, true), (&w.c1, false)]);
    a.offer(again).unwrap();
    assert_eq!(a.level(&subject(1)), Some(Level::Read));
    assert_eq!(a.unshared_at(&subject(1)), None);
}

#[test]
fn a_member_that_saw_one_branch_converges_when_it_meets_the_other() {
    let w = World::new();
    let mut p = w.space();
    for n in 1..=3 {
        p.offer(signed(share(&p, &w, subject(n), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    }
    // q saw only the first share, then the owner's take to c2 and what c2
    // sequenced.
    let mut q = Space::found(w.policy(), p.entries()[0].clone()).unwrap();
    q.offer(p.entries()[1].clone()).unwrap();
    let take = signed_side(q.curator_side(true, w.c2.public(), w.head(), T), &[(&w.d1, true)]);
    q.offer_side(take).unwrap();
    q.offer(signed(share(&q, &w, subject(9), Level::Append, vec![]), &[(&w.d1, true), (&w.c2, false)])).unwrap();
    // They meet and give each other all they have.
    let (pj, qj) = (p.to_json(), q.to_json());
    for e in qj["entries"].as_array().unwrap() {
        p.offer(Entry::parse(e).unwrap()).ok();
    }
    for s in qj["sides"].as_array().unwrap() {
        p.offer_side(Side::parse(s).unwrap()).unwrap();
    }
    for e in pj["entries"].as_array().unwrap() {
        q.offer(Entry::parse(e).unwrap()).ok();
    }
    assert_eq!(p.view(), q.view());
    assert_eq!((p.level(&subject(9)), p.level(&subject(2))), (Some(Level::Append), None), "c1's entries past the base are superseded");
}

#[test]
fn binding_needs_an_owner_who_owns_the_entity() {
    struct Projects(String);
    impl Owners for Projects {
        fn owner_of(&self, e: &EntityRef) -> Option<String> {
            matches!(e, EntityRef::Project(_)).then(|| self.0.clone())
        }
    }
    let w = World::new();
    let name = EntityRef::Name { kind: "project".into(), name: "atlas".into() };
    let project = EntityRef::Project(xenonet_model::ids::ProjectId::from_bytes(&[3; 20]));
    let bind = |sp: &Space, about: &EntityRef| signed(sp.next(Op::Bind { about: about.clone() }, w.head(), T), &[(&w.d1, true), (&w.c1, false)]);
    // With nothing that says who owns the project: refused.
    let mut sp = Space::found(w.policy(), w.genesis(name.clone())).unwrap();
    assert!(sp.offer(bind(&sp, &project)).is_err());
    // Owned by someone else.
    let other = SpacePolicy::new(Some(Arc::new(w.person.clone())), Some(Arc::new(Projects("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()))));
    let mut sp2 = Space::found(other, w.genesis(name.clone())).unwrap();
    assert!(sp2.offer(bind(&sp2, &project)).is_err());
    assert!(sp.offer(bind(&sp, &EntityRef::Person("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()))).is_err(), "another person");
    // The owner's own project: bound, once.
    let mine = SpacePolicy::new(Some(Arc::new(w.person.clone())), Some(Arc::new(Projects(w.pid()))));
    let mut sp3 = Space::found(mine, w.genesis(name.clone())).unwrap();
    assert_eq!(sp3.offer(bind(&sp3, &project)).unwrap(), Placed::Applied);
    assert!(sp3.is_about(&project, Some(&Projects(w.pid()))));
    assert!(!sp3.is_about(&project, Some(&Projects("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()))));
    assert_eq!(sp3.offer(bind(&sp3, &project)).unwrap_err().code, "taken");
    // A space founded about an entity counts as its only once its owner
    // has signed something there.
    let sp4 = Space::found(w.policy(), w.genesis(EntityRef::Person(w.pid()))).unwrap();
    assert!(!sp4.is_about(&EntityRef::Person(w.pid()), None));
    let mut sp4 = sp4;
    sp4.offer(signed(share(&sp4, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    assert!(sp4.is_about(&EntityRef::Person(w.pid()), None));
    // A name is nobody's.
    sp.offer(signed(share(&sp, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    assert!(!sp.is_about(&name, None));
}

#[test]
fn a_key_owned_space_moves_to_a_person_before_it_widens_into_a_network() {
    let w = World::new();
    let owner = key(30);
    let g = found(EntityRef::Name { kind: "project".into(), name: "atlas".into() }, Owner::Key(owner.public()), &w.c1, Some(&owner), "atlas", [7; 16], T).unwrap();
    let mut sp = Space::found(w.policy(), g).unwrap();
    let net = Subject::Network("net_00112233445566778899aabbccddeeff".into());
    // A key owner shares with a key, no presence to bring.
    let peer =
        signed(sp.next(Op::Share { subject: subject(1), level: Level::Append, endpoint: Some("ab".repeat(32)), after: vec![] }, None, T), &[(&owner, false), (&w.c1, false)]);
    assert_eq!(sp.offer(peer).unwrap(), Placed::Applied);
    // Not into a network, and binding waits too.
    assert!(sp.offer(signed(sp.next(Op::Share { subject: net.clone(), level: Level::Read, endpoint: None, after: vec![] }, None, T), &[(&owner, false), (&w.c1, false)])).is_err());
    assert!(sp.offer(signed(sp.next(Op::Bind { about: EntityRef::Person(w.pid()) }, None, T), &[(&owner, false), (&w.c1, false)])).is_err());
    // The move takes the key, the person's device with presence, the curator.
    let mv = sp.next(Op::MoveOwner { to: w.pid() }, w.head(), T);
    assert!(sp.offer(signed(mv.clone(), &[(&owner, false), (&w.c1, false)])).is_err(), "without the person");
    assert!(sp.offer(signed(mv.clone(), &[(&owner, false), (&w.d1, false), (&w.c1, false)])).is_err(), "without their presence");
    assert_eq!(sp.offer(signed(mv, &[(&owner, false), (&w.d1, true), (&w.c1, false)])).unwrap(), Placed::Applied);
    assert_eq!(sp.view().owner, Owner::Person(w.pid()));
    assert!(!sp.view().weaker());
    // Now the person shares it into a network, with presence.
    let into = signed(sp.next(Op::Share { subject: net.clone(), level: Level::Read, endpoint: None, after: vec![] }, w.head(), T), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(sp.offer(into).unwrap(), Placed::Applied);
    assert_eq!(sp.level_for(&Member { networks: &["net_00112233445566778899aabbccddeeff"], ..Member::default() }), Some(Level::Read));
    // A person's space doesn't move again, and the old key signs nothing
    // in the chain.
    assert!(sp.offer(signed(sp.next(Op::MoveOwner { to: w.pid() }, w.head(), T), &[(&w.d1, true), (&w.c1, false)])).is_err());
    assert!(sp.offer(signed(share(&sp, &w, subject(5), Level::Read, vec![]), &[(&owner, false), (&w.c1, false)])).is_err());
    // An unshare by an owner it had still narrows.
    let un = signed_side(sp.unshare(subject(1), None, T), &[(&owner, false)]);
    assert_eq!(sp.offer_side(un).unwrap(), Placed::Applied);
    assert_eq!(sp.level(&subject(1)), None);
}

#[test]
fn the_curator_and_the_owner_are_admins_and_the_curator_isnt_unshared() {
    let w = World::new();
    let mut sp = w.space();
    let c1 = w.c1.public().id();
    assert_eq!(sp.level_for(&Member { key: Some(&c1), ..Member::default() }), Some(Level::Admin));
    let pid = w.pid();
    assert_eq!(sp.level_for(&Member { person: Some(&pid), ..Member::default() }), Some(Level::Admin));
    assert_eq!(sp.level_for(&Member { key: Some("xns1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), ..Member::default() }), None);
    let un = signed_side(sp.unshare(Subject::Key(c1.clone()), w.head(), T), &[(&w.d1, false)]);
    assert!(sp.offer_side(un).is_err(), "an unshare can't remove the curator");
    assert!(sp.offer(signed(share(&sp, &w, Subject::Key(c1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).is_err());
    // The highest of a member's shares counts.
    let k = key(101).public().id();
    sp.offer(signed(share(&sp, &w, Subject::Key(k.clone()), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    sp.offer(signed(share(&sp, &w, Subject::Person("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()), Level::Write, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    assert_eq!(sp.level_for(&Member { key: Some(&k), person: Some("prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"), ..Member::default() }), Some(Level::Write));
}

#[test]
fn entries_for_another_space_or_badly_formed_are_refused() {
    let w = World::new();
    let mut sp = w.space();
    let other = Space::found(w.policy(), found(EntityRef::Person(w.pid()), Owner::Person(w.pid()), &w.c1, None, "will", [1; 16], T).unwrap()).unwrap();
    let theirs = signed(share(&other, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert!(sp.offer(theirs).is_err());
    let side = signed_side(other.unshare(subject(1), w.head(), T), &[(&w.d1, false)]);
    assert!(sp.offer_side(side).is_err());
    assert!(Subject::parse("key:xns1_short").is_none() && Subject::parse("network:net_x").is_none() && Subject::parse("actor:a@prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_none());
    assert_eq!(Subject::parse(&subject(1).to_string()), Some(subject(1)));
    let unsorted = Op::Share { subject: subject(1), level: Level::Read, endpoint: None, after: vec!["b".repeat(43), "a".repeat(43)] };
    assert!(sp.offer(signed(sp.next(unsorted, w.head(), T), &[(&w.d1, true), (&w.c1, false)])).is_err());
    // A duplicate is dropped quietly.
    let e = signed(share(&sp, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)]);
    assert_eq!(sp.offer(e.clone()).unwrap(), Placed::Applied);
    assert_eq!(sp.offer(e).unwrap(), Placed::Duplicate);
}

#[test]
fn a_take_waits_for_its_base_and_a_strangers_never_counts() {
    let w = World::new();
    let mut p = w.space();
    p.offer(signed(share(&p, &w, subject(1), Level::Read, vec![]), &[(&w.d1, true), (&w.c1, false)])).unwrap();
    let take = signed_side(p.curator_side(true, w.c2.public(), w.head(), T), &[(&w.d1, true)]);
    // q has only entry 0: the take's base isn't there yet.
    let mut q = Space::found(w.policy(), p.entries()[0].clone()).unwrap();
    assert_eq!(q.offer_side(take.clone()).unwrap(), Placed::Held);
    assert_eq!(q.view().epoch, 0);
    q.offer(p.entries()[1].clone()).unwrap();
    assert_eq!((q.view().epoch, q.view().curator.id()), (1, w.c2.public().id()));
    assert_eq!(q.curators_by(), [take.id()]);
    // A take signed by a stranger, its base unknown, waits and never
    // counts once the base is there; one whose base is there is refused.
    let mut r = Space::found(w.policy(), p.entries()[0].clone()).unwrap();
    let theirs = signed_side(p.curator_side(true, w.c3.public(), w.head(), T), &[(&w.stranger, false)]);
    assert_eq!(r.offer_side(theirs.clone()).unwrap(), Placed::Held);
    r.offer(p.entries()[1].clone()).unwrap();
    assert_eq!(r.view().epoch, 0);
    assert!(p.offer_side(theirs).is_err());
}
