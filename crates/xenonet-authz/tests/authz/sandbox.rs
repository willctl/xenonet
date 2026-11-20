//! A start that isn't the person's own runs sandboxed: from another
//! machine, under a delegation, for an intent or from an actor, a start
//! whose act says no sandbox is refused, never defaulted, so an older
//! sender that leaves the field out is refused too. The person's own start
//! may be either.

use crate::common::*;
use xenonet_authz::*;

fn start(mode: StartMode, sandbox: Option<Sandbox>) -> (Resource, Act) {
    let (app, folder) = (AppId::new("claude").unwrap(), Folder::new("/work").unwrap());
    (Resource::Run { machine: machine("m1"), app: app.clone(), folder: folder.clone() }, Act::Start { app, folder, mode, sandbox })
}

fn on() -> Sandbox {
    Sandbox { domains: vec!["github.com".into()] }
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
        let ask = |sandbox| {
            let (resource, act) = start(mode, sandbox);
            decide(None, &Request::new(agent.clone(), resource, act, Door::Network, NOW, Some(network())), &f.world, None)
        };
        assert_eq!(why(&ask(None)), "sandbox", "{mode:?}: the field missing or off is refused");
        assert!(ask(Some(on())).is_allow(), "{mode:?}: sandboxed it runs");
        assert!(ask(Some(Sandbox::default())).is_allow(), "{mode:?}: with no domains too");
    }
}

#[test]
fn an_agent_on_this_machine_starts_only_sandboxed_too() {
    let f = world();
    let agent = Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m1"), attestation: Attestation::ProcessChecked };
    let (resource, act) = start(StartMode::Asks, None);
    let r = Request::new(agent.clone(), resource, act, Door::Local, NOW, None);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "sandbox");
    let (resource, act) = start(StartMode::Asks, Some(on()));
    assert!(decide(None, &Request::new(agent, resource, act, Door::Local, NOW, None), &f.world, None).is_allow());
}

#[test]
fn an_actor_starts_only_sandboxed() {
    let f = world();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Start])).unwrap();
    let bearer = token.bearer().clone();
    let principal = Principal::Actor { slug: bearer.slug.clone(), machine: bearer.machine.clone() };
    for (sandbox, allowed) in [(None, false), (Some(on()), true)] {
        let (resource, act) = start(StartMode::Asks, sandbox);
        let d = decide(Some(&token), &Request::new(principal.clone(), resource, act, Door::Local, NOW, None), &f.world, None);
        assert_eq!((d.is_allow(), if allowed { "allowed" } else { "sandbox" }), (allowed, why(&d)));
    }
}

#[test]
fn the_persons_own_start_may_be_either() {
    let f = world();
    for mode in [StartMode::Asks, StartMode::Delegated] {
        for sandbox in [None, Some(on())] {
            let (resource, act) = start(mode, sandbox.clone());
            let principal = as_person(&f.world, &bio_device(), &resource, &act);
            let r = Request::new(principal, resource, act, Door::Local, NOW, None);
            let d = decide(None, &r, &f.world, None);
            assert!(d.is_allow(), "{mode:?} {sandbox:?}: {:?}", d.reason().map(Reason::text));
        }
    }
}

#[test]
fn what_the_person_signs_includes_the_sandbox() {
    let f = world();
    let (resource, signed) = start(StartMode::Asks, Some(on()));
    let sig = PersonSigned::sign(&bio_device(), &resource, &signed).unwrap();
    for other in [None, Some(Sandbox::default()), Some(Sandbox { domains: vec!["example.org".into()] })] {
        let (_, act) = start(StartMode::Asks, other);
        assert!(PersonSigned::verify(&f.world, &person(), &device_id(&bio_device()), &resource, &act, &sig).is_err(), "a signature for one sandbox isn't for another");
    }
}
