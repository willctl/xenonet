// Signing a curator's draft as the person: the chain takes what's signed
// for the act asked, and a draft that does anything else, to another
// space or person, or whose act isn't its entry's, is refused unsigned.
// The person's device is a fake that says yes.

use super::*;
use std::sync::Arc;
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::person::{Device, Devices, PersonChain};
use xenonet_log::space::{found, Owner, Placed, Space, SpacePolicy};
use xenonet_model::ids::EntityRef;

const N1: &str = "net_00000000000000000000000000000001";

struct Person(PersonChain);

impl Devices for Person {
    fn person_device(&self, _person: &str, head: &str, device: &str) -> Option<Device> {
        self.0.device_at(device, head).cloned()
    }
}

fn identity(name: &str) -> (Arc<Identity>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("xn-space-sign-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let id = Identity::open(dir.join("identity"), Arc::new(crate::persist::Memory::default()), Some(Arc::new(xenonet_keys::Fake { allow: true }))).unwrap();
    id.found(true, "m1", 1_800_000_000_000).unwrap();
    (id, dir)
}

// A space founded under a standalone owner key, curated by `curator`.
fn space(curator: &SeedSigner, owner: &SeedSigner, devices: Arc<dyn Devices>) -> Space {
    let genesis =
        found(EntityRef::Name { kind: "project".into(), name: "atlas".into() }, Owner::Key(owner.public()), curator, Some(owner), "atlas", [1; 16], 1_800_000_000_000).unwrap();
    Space::found(SpacePolicy::new(Some(devices), None), genesis).unwrap()
}

fn draft(e: &Entry) -> Value {
    json!({ "entry": e.to_json(), "act": b64(&e.digest()) })
}

#[test]
fn a_draft_is_signed_for_the_act_asked_alone_and_the_chain_takes_it() {
    let (id, dir) = identity("asked");
    let person = id.person_id().unwrap();
    let (curator, owner) = (SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap(), SeedSigner::from_seed(&[2; 32], KeyHome::File).unwrap());
    let mut s = space(&curator, &owner, Arc::new(Person(id.chain())));
    let sid = s.id().to_string();
    let now = 1_800_000_000_100;

    // The move to the person, with the owner key's signature already on.
    let mut mv = s.next(Op::MoveOwner { to: person.clone() }, id.head(), now);
    mv.sign(&owner.public().id(), &owner, None).unwrap();
    assert!(sign(&id, false, &sid, &Asked::MoveOwner, &draft(&mv), now).is_err_and(|e| e.code == "window"), "only from the window");
    let mut elsewhere = s.next(Op::MoveOwner { to: "prs_mmmmmmmmmmmmmmmmmmmmmmmmmmmmmmmm".into() }, id.head(), now);
    elsewhere.sign(&owner.public().id(), &owner, None).unwrap();
    assert_eq!(sign(&id, true, &sid, &Asked::MoveOwner, &draft(&elsewhere), now).unwrap_err().code, "not-that-draft", "a move to someone else");
    assert_eq!(sign(&id, true, "spc_other", &Asked::MoveOwner, &draft(&mv), now).unwrap_err().code, "not-that-draft", "another space");
    let mut lying = draft(&mv);
    lying["act"] = json!(b64(&[7; 32]));
    assert_eq!(sign(&id, true, &sid, &Asked::MoveOwner, &lying, now).unwrap_err().code, "not-that-draft", "an act that isn't the entry's");
    let share = Asked::Share { network: N1.into(), level: Level::Append };
    assert_eq!(sign(&id, true, &sid, &share, &draft(&mv), now).unwrap_err().code, "not-that-draft", "asked to share, drafted a move");

    let signed = sign(&id, true, &sid, &Asked::MoveOwner, &draft(&mv), now).unwrap();
    let mut e = Entry::parse(&signed).unwrap();
    e.sign(&curator.public().id(), &curator, None).unwrap();
    assert_eq!(s.offer(e).unwrap(), Placed::Applied, "the chain takes the person's move");

    // Then shared into N1 at append: what's asked, at that level.
    let sh = s.next(Op::Share { subject: Subject::Network(N1.into()), level: Level::Append, endpoint: None, after: vec![] }, id.head(), now);
    let wider = Asked::Share { network: N1.into(), level: Level::Read };
    assert_eq!(sign(&id, true, &sid, &wider, &draft(&sh), now).unwrap_err().code, "not-that-draft", "another level than asked");
    let mut e = Entry::parse(&sign(&id, true, &sid, &share, &draft(&sh), now).unwrap()).unwrap();
    e.sign(&curator.public().id(), &curator, None).unwrap();
    assert_eq!(s.offer(e).unwrap(), Placed::Applied);
    assert_eq!(s.level(&Subject::Network(N1.into())), Some(Level::Append));

    // And unshared: a side entry, the device's signature alone.
    let side = s.unshare(Subject::Network(N1.into()), id.head(), now);
    let d = json!({ "side": side.to_json(), "act": b64(&side.digest()) });
    let unshare = Asked::Unshare { network: N1.into() };
    assert!(!unshare.widens());
    let signed = sign(&id, true, &sid, &unshare, &d, now).unwrap();
    assert!(signed["signers"][0].get("presence").is_none(), "narrowing takes no presence");
    s.offer_side(Side::parse(&signed).unwrap()).unwrap();
    assert_eq!(s.level(&Subject::Network(N1.into())), None, "unshared");
    let _ = std::fs::remove_dir_all(dir);
}
