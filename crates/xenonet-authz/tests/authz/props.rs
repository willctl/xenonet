//! Properties over generated tokens: a narrowed token never allows what
//! its parent refuses, and a revoked id is refused, whichever block's.

use crate::common::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use std::cell::Cell;
use xenonet_authz::*;

fn rights(mask: u16) -> Vec<Action> {
    Action::ALL.into_iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, a)| a).collect()
}

fn narrowing() -> impl Strategy<Value = Narrowing> {
    prop_oneof![
        (0..Door::ALL.len()).prop_map(|i| Narrowing::Door(Door::ALL[i])),
        (-HOUR..2 * HOUR).prop_map(|d| Narrowing::Until(NOW + d)),
        any::<u16>().prop_map(|m| Narrowing::Actions(rights(m))),
        (0..ReadScope::ALL.len(), any::<bool>(), 0..CURSOR_MS * 2).prop_map(|(s, f, d)| Narrowing::Cursor { scope: ReadScope::ALL[s], filters: [u8::from(f); 32], until: NOW + d }),
    ]
}

// An act for each action this machine's actor might ask, with a read's
// scope and filters chosen too.
fn request(action: Action, door: Door, at: i64, scope: ReadScope, filters: Option<bool>) -> Request {
    let m1 = || Resource::Machine(machine("m1"));
    let run = Resource::Run { machine: machine("m1"), app: AppId::new("claude").unwrap(), folder: Folder::new("/w").unwrap() };
    let (resource, act) = match action {
        Action::Message => (m1(), Act::Message),
        Action::Wake => (Resource::Session(SessionId::new("s1").unwrap()), Act::Wake),
        Action::Start => (run, Act::Start { app: AppId::new("claude").unwrap(), folder: Folder::new("/w").unwrap(), mode: StartMode::Asks }),
        Action::Answer => (run, Act::Answer { ask: AskId::new("q1").unwrap(), decision: AnswerDecision::Allow }),
        Action::Claim => (m1(), Act::Claim),
        Action::Handoff => (m1(), Act::Handoff),
        Action::Read => (m1(), Act::Read { scope, filters: filters.map(|f| [u8::from(f); 32]) }),
        Action::Admit => (Resource::Network(network()), Act::Admit { machine: machine("m3"), role: Role::Member }),
        Action::Widen => (Resource::Network(network()), Act::Widen { machine: machine("m1"), settings: Settings::default() }),
        Action::Publish => (Resource::Network(network()), Act::Publish { service: ServiceId::new("web").unwrap() }),
        Action::RegisterActor => (m1(), Act::RegisterActor { slug: ActorSlug::new("x").unwrap() }),
        Action::UseGateway => (m1(), Act::UseGateway),
        Action::Revoke => (m1(), Act::Revoke { what: Revocation::Token(vec![1]) }),
    };
    Request::new(Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") }, resource, act, door, at, None)
}

fn world() -> Fixture {
    let mut f = fixture();
    // Settings that let an actor do all an actor may, so the token decides.
    f.world.settings = Settings { start: StartSetting::Network, wake: WakeSetting::Network };
    f.world.consent = Consent { here: true, actors: true };
    f
}

fn case() -> impl Strategy<Value = (u16, Vec<Narrowing>, usize, usize, i64, usize, Option<bool>)> {
    (any::<u16>(), proptest::collection::vec(narrowing(), 0..4), 0..Action::ALL.len(), 0..Door::ALL.len(), 0..2 * HOUR, 0..ReadScope::ALL.len(), any::<Option<bool>>())
}

#[test]
fn a_narrowed_token_never_allows_what_its_parent_refuses() {
    let f = world();
    let mut runner = TestRunner::new(Config { cases: 512, ..Config::default() });
    let (both, narrowed_away) = (Cell::new(0), Cell::new(0));
    runner
        .run(&case(), |(mask, narrowings, a, d, dt, s, filters)| {
            let parent = f.m1_root.mint_actor(&ActorGrant { until: NOW + HOUR, ..actor_grant("muse", &rights(mask)) }).unwrap();
            let mut child = parent.attenuate(&Narrowing::Until(NOW + 2 * HOUR)).unwrap();
            for n in &narrowings {
                child = child.attenuate(n).unwrap();
            }
            // A narrowing travels: what the receiver parses is what it judges.
            let child = Token::parse(&child.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &f.world, NOW).unwrap();
            let r = request(Action::ALL[a], Door::ALL[d], NOW + dt, ReadScope::ALL[s], filters);
            let by_child = decide(Some(&child), &r, &f.world, None).is_allow();
            let by_parent = decide(Some(&parent), &r, &f.world, None).is_allow();
            prop_assert!(!by_child || by_parent, "{r:?} {narrowings:?}");
            both.set(both.get() + usize::from(by_child));
            narrowed_away.set(narrowed_away.get() + usize::from(by_parent && !by_child));
            Ok(())
        })
        .unwrap();
    let (both, narrowed_away) = (both.get(), narrowed_away.get());
    // Some cases allowed by both and some narrowed away, so the property
    // wasn't vacuous (about 25 and 100 of 512 are expected).
    assert!(both > 0 && narrowed_away > 0, "both {both}, narrowed away {narrowed_away}");
}

#[test]
fn a_revoked_id_is_refused_whichever_block_it_names() {
    let f = world();
    let mut runner = TestRunner::new(Config { cases: 128, ..Config::default() });
    runner
        .run(&(any::<u16>(), proptest::collection::vec(narrowing(), 0..4), any::<prop::sample::Index>()), |(mask, narrowings, pick)| {
            let mut token = f.m1_root.mint_actor(&actor_grant("muse", &rights(mask | 1))).unwrap();
            for n in &narrowings {
                token = token.attenuate(n).unwrap();
            }
            let ids = token.revocation_ids();
            prop_assert_eq!(ids.len(), narrowings.len() + 1);
            let mut w = world().world;
            w.roots = f.world.roots.clone();
            w.revoked.insert(pick.get(&ids).clone());
            let text = token.encode(ACTOR_PREFIX).unwrap();
            prop_assert_eq!(Token::parse(&text, ACTOR_PREFIX, &w, NOW).unwrap_err().code, "revoked");
            let r = request(Action::Message, Door::Local, NOW, ReadScope::Map, None);
            let d = decide(Some(&token), &r, &w, None);
            prop_assert_eq!(d.reason().map(Reason::code), Some("revoked"));
            Ok(())
        })
        .unwrap();
}
