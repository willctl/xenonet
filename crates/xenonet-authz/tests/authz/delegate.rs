//! Handing work to an outbound actor (an agent this machine calls over
//! A2A): the person, or an agent inside the scope the person registered it
//! with, through this machine's own door; nobody else.

use crate::common::*;
use xenonet_authz::*;

fn scout() -> ActorSlug {
    ActorSlug::new("scout").unwrap()
}

fn agent(id: &str, attestation: Attestation) -> Principal {
    Principal::Agent { id: AgentId::new(id).unwrap(), machine: machine("m1"), attestation }
}

/// scout, which the claude:fix session and the atlas project's sessions
/// may call for an hour.
fn world() -> Fixture {
    let mut f = fixture();
    let scope = OutboundScope { agents: Set::Only(vec![AgentId::new("claude:fix").unwrap()]), projects: Set::Only(vec![ProjectId::new("atlas").unwrap()]), until: NOW + HOUR };
    f.world.outbound.insert(scout(), scope);
    f
}

fn delegation(principal: Principal, project: Option<&str>, door: Door, at: i64) -> Request {
    let act = Act::Delegate { project: project.map(|p| ProjectId::new(p).unwrap()) };
    Request::new(principal, Resource::Actor(scout()), act, door, at, None)
}

fn why(d: &Decision) -> &'static str {
    d.reason().map_or("allowed", Reason::code)
}

#[test]
fn a_session_in_the_scope_delegates() {
    let f = world();
    let by_id = delegation(agent("claude:fix", Attestation::ProcessChecked), None, Door::Local, NOW);
    assert!(decide(None, &by_id, &f.world, None).is_allow());
    let by_project = delegation(agent("codex:other", Attestation::ProcessChecked), Some("atlas"), Door::Local, NOW);
    assert!(decide(None, &by_project, &f.world, None).is_allow());
}

#[test]
fn a_session_outside_the_scope_is_refused() {
    let f = world();
    for project in [None, Some("billing")] {
        let r = delegation(agent("codex:other", Attestation::HarnessSigned), project, Door::Local, NOW);
        assert_eq!(why(&decide(None, &r, &f.world, None)), "scope", "{project:?}");
    }
}

#[test]
fn an_unchecked_agent_never_delegates() {
    let f = world();
    let r = delegation(agent("claude:fix", Attestation::Unchecked), Some("atlas"), Door::Local, NOW);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "attestation");
}

#[test]
fn the_scope_ends() {
    let f = world();
    let r = delegation(agent("claude:fix", Attestation::ProcessChecked), None, Door::Local, NOW + 2 * HOUR);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "scope");
}

#[test]
fn an_actor_this_machine_doesnt_have_is_refused() {
    let f = fixture();
    let r = delegation(agent("claude:fix", Attestation::ProcessChecked), None, Door::Local, NOW);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "resource");
}

#[test]
fn only_from_this_machines_own_door() {
    let f = world();
    // Another machine's agent, even one the scope would name.
    let remote = Principal::Agent { id: AgentId::new("claude:fix").unwrap(), machine: machine("m2"), attestation: Attestation::HarnessSigned };
    let r = Request::new(remote, Resource::Actor(scout()), Act::Delegate { project: None }, Door::Network, NOW, Some(network()));
    assert_eq!(why(&decide(None, &r, &f.world, None)), "door");
    // A machine, on its own.
    let r = delegation(Principal::Machine { id: machine("m1") }, None, Door::Local, NOW);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "principal");
}

#[test]
fn an_actor_doesnt_delegate_even_with_the_right() {
    let f = world();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &Action::ALL)).unwrap();
    let muse = Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") };
    for door in [Door::Local, Door::Gateway, Door::A2a] {
        let r = delegation(muse.clone(), None, door, NOW);
        assert!(!decide(Some(&token), &r, &f.world, None).is_allow(), "{door:?}");
    }
}

#[test]
fn the_person_delegates() {
    let f = world();
    let resource = Resource::Actor(scout());
    let act = Act::Delegate { project: None };
    let person = as_person(&f.world, &bio_device(), &resource, &act);
    let r = Request::new(person, resource, act, Door::Local, NOW, None);
    assert!(decide(None, &r, &f.world, None).is_allow());
}

#[test]
fn a_delegation_is_done_to_an_actor() {
    let f = world();
    let r = Request::new(agent("claude:fix", Attestation::ProcessChecked), Resource::Machine(machine("m1")), Act::Delegate { project: None }, Door::Local, NOW, None);
    assert_eq!(why(&decide(None, &r, &f.world, None)), "resource");
}
