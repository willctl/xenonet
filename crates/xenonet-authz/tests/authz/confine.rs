//! How a start is confined. A start that isn't the person's own runs at
//! `vendor`, the app's own sandbox: from another machine, under a
//! delegation, for an intent or from an actor, a start whose act says `off`
//! or nothing is refused, never defaulted, so an older sender that leaves
//! the field out is refused too. A `cell` start is the person's own alone,
//! until each wall has keys of its own. The person's own start may be any
//! level.

use crate::common::*;
use xenonet_authz::*;

fn start(mode: StartMode, confine: Option<Confine>) -> (Resource, Act) {
    let (app, folder) = (AppId::new("claude").unwrap(), Folder::new("/work").unwrap());
    (Resource::Run { machine: machine("m1"), app: app.clone(), folder: folder.clone() }, Act::Start { app, folder, mode, confine })
}

fn on() -> Confine {
    Confine::Vendor { domains: vec!["github.com".into()] }
}

fn cell() -> Confine {
    Confine::Cell { wall: "wall_nova_0123456789abcdef".into() }
}

/// This machine takes starts from the network and, under a delegation of
/// the person's, delegated ones.
fn world() -> Fixture {
    let mut f = fixture();
    f.world.settings = Settings { start: StartSetting::Delegate, wake: WakeSetting::Consent };
    f.world.delegations = vec![delegation()];
    f
}

fn why(d: &Decision) -> &'static str {
    d.reason().map_or("allowed", Reason::code)
}

#[test]
fn an_agent_from_another_machine_starts_only_sandboxed() {
    let f = world();
    let agent = Principal::Agent { id: AgentId::new("a2").unwrap(), machine: machine("m2"), attestation: Attestation::ProcessChecked };
    for mode in [StartMode::Asks, StartMode::Delegated] {
        let ask = |confine| {
            let (resource, act) = start(mode, confine);
            decide(None, &Request::new(agent.clone(), resource, act, Door::Network, NOW, Some(network())), &f.world, None)
        };
        assert_eq!(why(&ask(None)), "sandbox", "{mode:?}: the field missing is refused");
        assert_eq!(why(&ask(Some(Confine::Off))), "sandbox", "{mode:?}: off is refused");
        assert!(ask(Some(on())).is_allow(), "{mode:?}: at vendor it runs");
        assert!(ask(Some(Confine::Vendor { domains: vec![] })).is_allow(), "{mode:?}: with no domains too");
        assert_eq!(why(&ask(Some(cell()))), "cell", "{mode:?}: a cell is not for anyone but the person yet");
    }
}

#[test]
fn an_agent_on_this_machine_starts_only_sandboxed_too() {
    let f = world();
    let agent = Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m1"), attestation: Attestation::ProcessChecked };
    for (confine, code) in [(None, "sandbox"), (Some(Confine::Off), "sandbox"), (Some(cell()), "cell")] {
        let (resource, act) = start(StartMode::Asks, confine);
        let r = Request::new(agent.clone(), resource, act, Door::Local, NOW, None);
        assert_eq!(why(&decide(None, &r, &f.world, None)), code);
    }
    let (resource, act) = start(StartMode::Asks, Some(on()));
    assert!(decide(None, &Request::new(agent, resource, act, Door::Local, NOW, None), &f.world, None).is_allow());
}

#[test]
fn an_actor_starts_only_sandboxed() {
    let f = world();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Start])).unwrap();
    let bearer = token.bearer().clone();
    let principal = Principal::Actor { slug: bearer.slug.clone(), machine: bearer.machine.clone() };
    for (confine, code) in [(None, "sandbox"), (Some(Confine::Off), "sandbox"), (Some(cell()), "cell"), (Some(on()), "allowed")] {
        let allowed = code == "allowed";
        let (resource, act) = start(StartMode::Asks, confine);
        let d = decide(Some(&token), &Request::new(principal.clone(), resource, act, Door::Local, NOW, None), &f.world, None);
        assert_eq!((d.is_allow(), why(&d)), (allowed, code));
    }
}

#[test]
fn the_persons_own_start_may_be_either() {
    let f = world();
    for mode in [StartMode::Asks, StartMode::Delegated] {
        for confine in [None, Some(Confine::Off), Some(on()), Some(cell())] {
            let (resource, act) = start(mode, confine.clone());
            let principal = as_person(&f.world, &bio_device(), &resource, &act);
            let r = Request::new(principal, resource, act, Door::Local, NOW, None);
            let d = decide(None, &r, &f.world, None);
            assert!(d.is_allow(), "{mode:?} {confine:?}: {:?}", d.reason().map(Reason::text));
        }
    }
}

#[test]
fn what_the_person_signs_includes_the_level_the_domains_and_the_wall() {
    let f = world();
    let (resource, signed) = start(StartMode::Asks, Some(on()));
    let sig = PersonSigned::sign(&bio_device(), &resource, &signed).unwrap();
    // The control: the act as it was signed, sandbox and all, verifies.
    assert!(PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &signed, &sig).is_ok(), "the signed act, with its level, is the one that verifies");
    let others = [
        None,
        Some(Confine::Vendor { domains: vec![] }),
        Some(Confine::Vendor { domains: vec!["example.org".into()] }),
        Some(cell()),
        Some(Confine::Cell { wall: "wall_other_0123456789abcdef".into() }),
    ];
    for other in others {
        let (_, act) = start(StartMode::Asks, other);
        assert!(
            PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &act, &sig).is_err(),
            "a signature for one level, domain list or wall isn't for another"
        );
    }
}

#[test]
fn a_cell_signed_with_its_wall_verifies_and_another_wall_does_not() {
    let f = world();
    let (resource, signed) = start(StartMode::Asks, Some(cell()));
    let sig = PersonSigned::sign(&bio_device(), &resource, &signed).unwrap();
    assert!(PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &signed, &sig).is_ok());
    let (_, other) = start(StartMode::Asks, Some(Confine::Cell { wall: "wall_atlas_0123456789abcdef".into() }));
    assert!(PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &other, &sig).is_err());
}

#[test]
fn off_signs_as_nothing_so_an_older_senders_own_start_still_verifies() {
    let f = world();
    let (resource, signed) = start(StartMode::Asks, None);
    let sig = PersonSigned::sign(&bio_device(), &resource, &signed).unwrap();
    let (_, off) = start(StartMode::Asks, Some(Confine::Off));
    assert!(PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &off, &sig).is_ok(), "no confinement is the same to the signature, said or not");
}
