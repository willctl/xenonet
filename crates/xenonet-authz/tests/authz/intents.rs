//! Intents: the person makes one on a device, for a machine their
//! delegation names, and that machine decides it under that delegation
//! alone; reports on it come from the machine it's worked on, or there
//! from its agent or an actor whose grant names it, and a cancel is the
//! person's.

use crate::common::*;
use xenonet_authz::*;

fn why(d: &Decision) -> &'static str {
    d.reason().map_or("allowed", Reason::code)
}

fn dlg(id: &str) -> DelegationId {
    DelegationId::new(id).unwrap()
}

fn intend(delegation: &str, until: i64) -> Act {
    Act::Intend {
        goal: [7; 32],
        project: project(),
        app: AppId::new("claude").unwrap(),
        delegation: dlg(delegation),
        until,
        network: network(),
        nonce: [1; 16],
        arbiter: machine("m2"),
    }
}

fn worked_here() -> Resource {
    Resource::Intent { machine: machine("m1"), id: None }
}

/// The person at m2's device (the weaker one) makes an intent for m1, this
/// machine, under dlg_a.
fn made_on_m2(f: &Fixture, act: &Act, at: i64) -> Request {
    let p = as_person(&f.world, &weak_device(), &worked_here(), act);
    Request::new(p, worked_here(), act.clone(), Door::Network, at, Some(network()))
}

#[test]
fn an_intent_runs_under_the_delegation_it_names_and_no_other() {
    let mut f = fixture();
    f.world.delegations = vec![delegation()];
    let act = intend("dlg_a", NOW + HOUR);
    assert!(decide(None, &made_on_m2(&f, &act, NOW), &f.world, None).is_allow());
    // Another delegation that covers it is no stand-in for the one named.
    let named_b = intend("dlg_b", NOW + HOUR);
    assert_eq!(why(&decide(None, &made_on_m2(&f, &named_b, NOW), &f.world, None)), "grant");
    f.world.delegations = vec![StandingGrant { id: dlg("dlg_b"), ..delegation() }];
    assert!(decide(None, &made_on_m2(&f, &named_b, NOW), &f.world, None).is_allow());
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "grant", "dlg_a is gone");
    // Ended, it covers nothing; one that doesn't start covers nothing.
    f.world.delegations = vec![StandingGrant { until: NOW - 1, ..delegation() }];
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "grant");
    f.world.delegations = vec![StandingGrant { starts: false, ..delegation() }];
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "grant");
}

#[test]
fn the_delegation_names_both_devices() {
    let mut f = fixture();
    // Only this machine's device: the device the intent was made on isn't
    // named.
    f.world.delegations = vec![StandingGrant { devices: Set::Only(vec![device_id(&bio_device())]), ..delegation() }];
    let act = intend("dlg_a", NOW + HOUR);
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "grant");
    // Only the maker's: this machine isn't named.
    f.world.delegations = vec![StandingGrant { devices: Set::Only(vec![device_id(&weak_device())]), ..delegation() }];
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "grant");
}

#[test]
fn an_intent_is_the_persons_for_the_machine_it_names_until_it_ends() {
    let mut f = fixture();
    f.world.delegations = vec![delegation()];
    let act = intend("dlg_a", NOW + HOUR);
    // Another machine decides an intent worked there.
    let there = Resource::Intent { machine: machine("m2"), id: None };
    let p = as_person(&f.world, &weak_device(), &there, &act);
    assert_eq!(why(&decide(None, &Request::new(p, there, act.clone(), Door::Network, NOW, Some(network())), &f.world, None)), "resource");
    // Past its end.
    let ended = intend("dlg_a", NOW - 1);
    assert_eq!(why(&decide(None, &made_on_m2(&f, &ended, NOW), &f.world, None)), "ended");
    // No machine, agent or actor makes one.
    for p in [Principal::Machine { id: machine("m2") }, Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m2"), attestation: Attestation::HarnessSigned }] {
        let r = Request::new(p, worked_here(), act.clone(), Door::Network, NOW, Some(network()));
        assert_eq!(why(&decide(None, &r, &f.world, None)), "principal");
    }
    // Start off refuses it.
    f.world.settings.start = StartSetting::Off;
    assert_eq!(why(&decide(None, &made_on_m2(&f, &act, NOW), &f.world, None)), "settings");
    // A signature for another intent doesn't verify for this one.
    let p = as_person(&f.world, &weak_device(), &worked_here(), &intend("dlg_a", NOW + 2 * HOUR));
    let r = Request::new(p, worked_here(), act, Door::Network, NOW, Some(network()));
    f.world.settings.start = StartSetting::You;
    assert_eq!(why(&decide(None, &r, &f.world, None)), "person");
}

#[test]
fn an_intent_for_any_machine_is_decided_by_one_its_delegation_names() {
    let mut f = fixture();
    let any = Resource::Intent { machine: MachineId::any(), id: None };
    let act = intend("dlg_a", NOW + HOUR);
    let made = |f: &Fixture| {
        let p = as_person(&f.world, &weak_device(), &any, &act);
        Request::new(p, any.clone(), act.clone(), Door::Network, NOW, Some(network()))
    };
    f.world.delegations = vec![delegation()];
    assert!(decide(None, &made(&f), &f.world, None).is_allow(), "the delegation names this machine's device and the maker's");
    // A delegation that names only the maker's device: this machine isn't
    // one an any intent may be worked on.
    f.world.delegations = vec![StandingGrant { devices: Set::Only(vec![device_id(&weak_device())]), ..delegation() }];
    assert_eq!(why(&decide(None, &made(&f), &f.world, None)), "grant");
    // The signature names `any`: the same act for one machine isn't it.
    f.world.delegations = vec![delegation()];
    let p = as_person(&f.world, &weak_device(), &worked_here(), &act);
    assert_eq!(why(&decide(None, &Request::new(p, any.clone(), act.clone(), Door::Network, NOW, Some(network())), &f.world, None)), "person");
    // Nor is the arbiter's name to be changed under it.
    let mut other = act.clone();
    if let Act::Intend { arbiter, .. } = &mut other {
        *arbiter = machine("m3");
    }
    let p = as_person(&f.world, &weak_device(), &any, &act);
    assert_eq!(why(&decide(None, &Request::new(p, any, other, Door::Network, NOW, Some(network())), &f.world, None)), "person");
    assert!(MachineId::new(ANY_MACHINE).unwrap().is_any() && !machine("any").is_any());
}

#[test]
fn an_intent_from_another_machine_holds_on_stale_governance() {
    let mut f = fixture();
    f.world.delegations = vec![delegation()];
    f.world.synced = Some(NOW - 11 * 60_000);
    let d = decide(None, &made_on_m2(&f, &intend("dlg_a", NOW + HOUR), NOW), &f.world, None);
    assert!(matches!(d, Decision::Hold(_)), "{d:?}");
}

fn int_a() -> IntentId {
    IntentId::new("int_a").unwrap()
}

/// int_a, made on m2 and worked here by agent a1.
fn holding(open: bool) -> Fixture {
    let mut f = fixture();
    f.world.intents.insert(int_a(), HeldIntent { maker: machine("m2"), target: machine("m1"), agent: Some(AgentId::new("a1").unwrap()), open });
    f
}

fn report(principal: Principal, target: &str, step: IntentStep, door: Door) -> Request {
    let resource = Resource::Intent { machine: machine(target), id: Some(int_a()) };
    let network = (door == Door::Network).then(network);
    Request::new(principal, resource, Act::Report { intent: int_a(), step }, door, NOW, network)
}

fn agent(id: &str, attestation: Attestation) -> Principal {
    Principal::Agent { id: AgentId::new(id).unwrap(), machine: machine("m1"), attestation }
}

#[test]
fn its_agent_reports_at_its_own_door_and_no_other_session() {
    let f = holding(true);
    for step in [IntentStep::Note, IntentStep::Done] {
        assert!(decide(None, &report(agent("a1", Attestation::ProcessChecked), "m1", step, Door::Local), &f.world, None).is_allow(), "{step:?}");
        assert_eq!(why(&decide(None, &report(agent("a2", Attestation::HarnessSigned), "m1", step, Door::Local), &f.world, None)), "intent", "another session");
        assert_eq!(why(&decide(None, &report(agent("a1", Attestation::Unchecked), "m1", step, Door::Local), &f.world, None)), "attestation");
    }
    // Its run started is its machine's word.
    assert_eq!(why(&decide(None, &report(agent("a1", Attestation::HarnessSigned), "m1", IntentStep::Start, Door::Local), &f.world, None)), "principal");
    assert!(decide(None, &report(Principal::Machine { id: machine("m1") }, "m1", IntentStep::Start, Door::Local), &f.world, None).is_allow());
}

#[test]
fn reports_from_another_machine_come_from_the_one_it_is_worked_on() {
    let mut f = fixture();
    // int_a, made here and worked on m2.
    f.world.intents.insert(int_a(), HeldIntent { maker: machine("m1"), target: machine("m2"), agent: None, open: true });
    let m2 = Principal::Machine { id: machine("m2") };
    for step in [IntentStep::Start, IntentStep::Note, IntentStep::Done] {
        assert!(decide(None, &report(m2.clone(), "m2", step, Door::Network), &f.world, None).is_allow(), "{step:?}");
    }
    // A report that says it's worked elsewhere than it is.
    assert_eq!(why(&decide(None, &report(m2, "m3", IntentStep::Done, Door::Network), &f.world, None)), "intent");
    // Another member, saying it's worked on m2.
    f.world.roles.insert((network(), machine("m3")), Role::Member);
    let m3 = Principal::Machine { id: machine("m3") };
    assert_eq!(why(&decide(None, &report(m3, "m2", IntentStep::Done, Door::Network), &f.world, None)), "intent");
    // m2's agent, through m2: only its own machine's door takes an agent's.
    let theirs = Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m2"), attestation: Attestation::ProcessChecked };
    assert_eq!(why(&decide(None, &report(theirs, "m2", IntentStep::Done, Door::Network), &f.world, None)), "intent");
}

#[test]
fn a_cancel_is_the_persons_and_an_ended_intent_takes_no_report() {
    let f = holding(true);
    let resource = Resource::Intent { machine: machine("m1"), id: Some(int_a()) };
    let act = Act::Report { intent: int_a(), step: IntentStep::Cancel };
    let p = as_person(&f.world, &weak_device(), &resource, &act);
    assert!(decide(None, &Request::new(p, resource, act, Door::Network, NOW, Some(network())), &f.world, None).is_allow());
    for p in [Principal::Machine { id: machine("m2") }, agent("a1", Attestation::HarnessSigned)] {
        let door = if p.machine() == Some(&machine("m1")) { Door::Local } else { Door::Network };
        assert_eq!(why(&decide(None, &report(p, "m1", IntentStep::Cancel, door), &f.world, None)), "principal");
    }
    let ended = holding(false);
    assert_eq!(why(&decide(None, &report(agent("a1", Attestation::HarnessSigned), "m1", IntentStep::Done, Door::Local), &ended.world, None)), "intent");
    // One this machine doesn't hold.
    let none = fixture();
    assert_eq!(why(&decide(None, &report(Principal::Machine { id: machine("m1") }, "m1", IntentStep::Done, Door::Local), &none.world, None)), "intent");
}

#[test]
fn an_actor_reports_on_the_intents_its_grant_names() {
    let mut f = holding(true);
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Read, Action::Report])).unwrap();
    let muse = Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") };
    let r = report(muse.clone(), "m1", IntentStep::Note, Door::Local);
    assert_eq!(why(&decide(Some(&token), &r, &f.world, None)), "scope");
    f.world.grant_intents = Some(vec![int_a()]);
    assert!(decide(Some(&token), &r, &f.world, None).is_allow());
    assert!(decide(Some(&token), &report(muse.clone(), "m1", IntentStep::Done, Door::Local), &f.world, None).is_allow());
    assert_eq!(why(&decide(Some(&token), &report(muse.clone(), "m1", IntentStep::Start, Door::Local), &f.world, None)), "principal");
    // Without the right in its token.
    let plain = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Read])).unwrap();
    assert_eq!(why(&decide(Some(&plain), &r, &f.world, None)), "right");
}

#[test]
fn an_answer_for_an_intents_run_is_under_its_delegation_alone() {
    let mut f = fixture();
    f.world.delegations = vec![delegation()];
    let me = Principal::Machine { id: machine("m1") };
    let run = Resource::Session(SessionId::new("s1").unwrap());
    let allow = Act::Answer { ask: AskId::new("q1").unwrap(), decision: AnswerDecision::Allow };
    let request = Request::new(me, run, allow, Door::Local, NOW, None);
    let ask = |delegation: Option<&str>| Asked { delegation: delegation.map(dlg), ..asked(Attestation::ProcessChecked) };
    f.world.asks.insert(AskId::new("q1").unwrap(), ask(Some("dlg_a")));
    assert!(decide(None, &request, &f.world, None).is_allow());
    f.world.asks.insert(AskId::new("q1").unwrap(), ask(Some("dlg_b")));
    assert_eq!(why(&decide(None, &request, &f.world, None)), "grant", "its delegation was revoked; another doesn't stand in");
    f.world.asks.insert(AskId::new("q1").unwrap(), ask(None));
    assert!(decide(None, &request, &f.world, None).is_allow(), "a delegated run that names none takes any");
}
