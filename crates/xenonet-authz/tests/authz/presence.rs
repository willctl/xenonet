//! The person: a device signature binds them to one act, and a presence
//! block to the same act from the same device, once.

use crate::common::*;
use xenonet_authz::*;
use xenonet_id::presence::PresenceCheck;
use xenonet_id::sign::Signer;

fn run() -> Resource {
    Resource::Run { machine: machine("m1"), app: AppId::new("claude").unwrap(), folder: Folder::new("/work").unwrap() }
}

fn answer(decision: AnswerDecision) -> Act {
    Act::Answer { ask: AskId::new("q1").unwrap(), decision }
}

fn request(world: &World, device: &dyn Signer, act: Act) -> Request {
    Request::new(as_person(world, device, &run(), &act), run(), act, Door::Local, NOW, None)
}

#[test]
fn an_answer_takes_a_presence_for_it() {
    let f = fixture();
    let r = request(&f.world, &bio_device(), answer(AnswerDecision::Allow));
    let p = presence(&f.world, &bio_device(), &r);
    let Decision::Allow(ok) = decide(None, &r, &f.world, Some(&p)) else { panic!("allowed") };
    assert_eq!(ok.authority(), &Authority::Presence { device: device_id(&bio_device()), kind: DeviceKind::Biometric });
    assert_eq!(ok.note(), None);
    assert_eq!(ok.act_hash(), &r.act_hash());
    assert_eq!(ok.action(), Action::Answer);
    // Without one: refused.
    assert_eq!(decide(None, &r, &f.world, None).reason().unwrap().code(), "presence");
}

#[test]
fn a_weaker_device_is_allowed_and_the_decision_says_so() {
    let f = fixture();
    let r = request(&f.world, &weak_device(), answer(AnswerDecision::Deny));
    let p = presence(&f.world, &weak_device(), &r);
    let Decision::Allow(ok) = decide(None, &r, &f.world, Some(&p)) else { panic!("allowed") };
    assert!(ok.note().unwrap().contains("weaker"));
}

#[test]
fn a_presence_for_another_act_or_device_is_refused() {
    let f = fixture();
    let allow = request(&f.world, &bio_device(), answer(AnswerDecision::Allow));
    let deny = request(&f.world, &bio_device(), answer(AnswerDecision::Deny));
    // A block made for allow, offered for deny.
    let block = presence_block(&bio_device(), &allow);
    assert!(VerifiedPresence::check(&mut PresenceCheck::new(), &block, &f.world, &deny).is_err());
    // A presence checked for allow, put on deny.
    let p = presence(&f.world, &bio_device(), &allow);
    assert_eq!(decide(None, &deny, &f.world, Some(&p)).reason().unwrap().code(), "presence");
    // A block from the weak device on an act the biometric one signed.
    let other = presence_block(&weak_device(), &allow);
    assert!(VerifiedPresence::check(&mut PresenceCheck::new(), &other, &f.world, &allow).is_err());
    // Once only.
    let mut seen = PresenceCheck::new();
    VerifiedPresence::check(&mut seen, &block, &f.world, &allow).unwrap();
    assert!(VerifiedPresence::check(&mut seen, &block, &f.world, &allow).is_err());
    // Too old.
    let late = Request::new(allow.principal().clone(), run(), allow.act().clone(), Door::Local, NOW + 61_000, None);
    assert!(VerifiedPresence::check(&mut PresenceCheck::new(), &block, &f.world, &late).is_err());
    // Not for an act that takes none.
    let msg = Request::new(as_person(&f.world, &bio_device(), &run(), &Act::Message), Resource::Machine(machine("m1")), Act::Message, Door::Local, NOW, None);
    assert!(VerifiedPresence::check(&mut PresenceCheck::new(), &block, &f.world, &msg).is_err());
}

#[test]
fn a_device_signature_is_for_one_act_from_an_unrevoked_device_of_this_machines_person() {
    let mut f = fixture();
    let act = answer(AnswerDecision::Allow);
    let sig = PersonSigned::sign(&bio_device(), &run(), &act).unwrap();
    let id = device_id(&bio_device());
    assert!(PersonSigned::verify(&f.world, &person(), &id, &run(), &answer(AnswerDecision::Deny), &sig).is_err(), "another act");
    assert!(PersonSigned::verify(&f.world, &person(), &device_id(&weak_device()), &run(), &act, &sig).is_err(), "another device");
    assert!(PersonSigned::verify(&f.world, &PersonId::new("prs_other").unwrap(), &id, &run(), &act, &sig).is_err(), "another person");
    // Signed for one act, put on another request.
    let signed = PersonSigned::verify(&f.world, &person(), &id, &run(), &act, &sig).unwrap();
    let wrong = Request::new(Principal::Person(signed.clone()), run(), answer(AnswerDecision::Deny), Door::Local, NOW, None);
    assert_eq!(decide(None, &wrong, &f.world, None).reason().unwrap().code(), "person");
    // This machine's person only.
    let r = Request::new(Principal::Person(signed.clone()), run(), act.clone(), Door::Local, NOW, None);
    let p = presence(&f.world, &bio_device(), &r);
    assert!(decide(None, &r, &f.world, Some(&p)).is_allow());
    f.world.person = Some(PersonId::new("prs_other").unwrap());
    assert_eq!(decide(None, &r, &f.world, Some(&p)).reason().unwrap().code(), "person");
    // Revoked once the ground holds it, even for a signature checked before.
    f.world.person = Some(person());
    f.world.devices.remove(&(person(), id.clone()));
    assert_eq!(decide(None, &r, &f.world, Some(&p)).reason().unwrap().code(), "person");
    assert!(PersonSigned::verify(&f.world, &person(), &id, &run(), &act, &sig).is_err());
}
