//! Actors and answers: a plain right to answer is a Deny's; an Allow needs
//! an allow scope on the actor's grant, and is never decided wider than it.

use crate::common::*;
use xenonet_authz::*;

fn session(s: &str) -> Resource {
    Resource::Session(SessionId::new(s).unwrap())
}

fn answer(decision: AnswerDecision) -> Act {
    Act::Answer { ask: AskId::new("q1").unwrap(), decision }
}

fn actor(m: &str) -> Principal {
    Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine(m) }
}

fn scope() -> AllowScope {
    AllowScope { sessions: Set::Only(vec![SessionId::new("s1").unwrap()]), kinds: vec![PromptKind::Command], until: NOW + HOUR }
}

#[test]
fn a_plain_answer_right_denies_and_never_allows() {
    let f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Answer])).unwrap();
    let deny = Request::new(actor("m1"), session("s1"), answer(AnswerDecision::Deny), Door::Local, NOW, None);
    assert!(decide(Some(&token), &deny, &f.world, None).is_allow());
    let allow = Request::new(actor("m1"), session("s1"), answer(AnswerDecision::Allow), Door::Local, NOW, None);
    assert_eq!(decide(Some(&token), &allow, &f.world, None).reason().unwrap().code(), "scope");
    // No right to answer at all: neither.
    let none = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message])).unwrap();
    assert_eq!(decide(Some(&none), &deny, &f.world, None).reason().unwrap().code(), "right");
}

#[test]
fn an_allow_scope_allows_inside_it_and_nothing_outside() {
    let mut f = fixture();
    let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Answer])).unwrap();
    let id = token.revocation_ids().into_iter().next().unwrap();
    f.world.scopes.insert(id.clone(), scope());
    let allow = |at: i64| Request::new(actor("m1"), session("s1"), answer(AnswerDecision::Allow), Door::Local, at, None);
    let Decision::Allow(ok) = decide(Some(&token), &allow(NOW), &f.world, None) else { panic!("allowed") };
    assert_eq!(ok.authority(), &Authority::AllowScope { revocation_id: id.clone() });
    // After until.
    assert_eq!(decide(Some(&token), &allow(NOW + HOUR + 1), &f.world, None).reason().unwrap().code(), "scope");
    // Another session's question.
    f.world.asks.insert(AskId::new("q1").unwrap(), Asked { session: SessionId::new("s2").unwrap(), ..asked(Attestation::HarnessSigned) });
    let other = Request::new(actor("m1"), session("s2"), answer(AnswerDecision::Allow), Door::Local, NOW, None);
    assert_eq!(decide(Some(&token), &other, &f.world, None).reason().unwrap().code(), "scope");
    // Another kind of prompt.
    f.world.asks.insert(AskId::new("q1").unwrap(), Asked { kind: PromptKind::Web, ..asked(Attestation::HarnessSigned) });
    assert_eq!(decide(Some(&token), &allow(NOW), &f.world, None).reason().unwrap().code(), "scope");
    // A run whose agent is unchecked, whatever the scope says.
    f.world.scopes.insert(id, AllowScope { sessions: Set::All, kinds: PromptKind::ALL.to_vec(), until: NOW + HOUR });
    f.world.asks.insert(AskId::new("q1").unwrap(), asked(Attestation::Unchecked));
    assert_eq!(decide(Some(&token), &allow(NOW), &f.world, None).reason().unwrap().code(), "attestation");
    // An answer whose session isn't the question's.
    f.world.asks.insert(AskId::new("q1").unwrap(), asked(Attestation::HarnessSigned));
    assert_eq!(decide(Some(&token), &other, &f.world, None).reason().unwrap().code(), "ask");
}

#[test]
fn an_actor_of_another_machine_denies_but_its_scope_isnt_held_here() {
    let mut f = fixture();
    f.world.roles.insert((network(), machine("m2")), Role::Member);
    let token = f.m2_root.mint_actor(&actor_grant("muse", &[Action::Answer])).unwrap();
    // A scope keyed by its id here would be another machine's grant's;
    // this machine keeps rows only for grants it issued, and decides by
    // what it holds.
    let deny = Request::new(actor("m2"), session("s1"), answer(AnswerDecision::Deny), Door::Network, NOW, Some(network()));
    assert!(decide(Some(&token), &deny, &f.world, None).is_allow());
    let allow = Request::new(actor("m2"), session("s1"), answer(AnswerDecision::Allow), Door::Network, NOW, Some(network()));
    assert_eq!(decide(Some(&token), &allow, &f.world, None).reason().unwrap().code(), "scope");
}
