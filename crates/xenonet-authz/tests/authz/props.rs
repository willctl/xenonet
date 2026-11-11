//! Properties over generated tokens: a narrowed token never allows what
//! its parent refuses, a revoked id is refused, whichever block's, an
//! actor's Allow is never wider than its allow scope, and a space's share
//! never yields more than its level.

use crate::common::*;
use proptest::prelude::*;
use proptest::test_runner::{Config, TestRunner};
use std::cell::Cell;
use xenonet_authz::*;

fn rights(mask: u32) -> Vec<Action> {
    Action::ALL.into_iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, a)| a).collect()
}

fn narrowing() -> impl Strategy<Value = Narrowing> {
    prop_oneof![
        (0..Door::ALL.len()).prop_map(|i| Narrowing::Door(Door::ALL[i])),
        (-HOUR..2 * HOUR).prop_map(|d| Narrowing::Until(NOW + d)),
        any::<u32>().prop_map(|m| Narrowing::Actions(rights(m))),
        (0..ReadScope::ALL.len(), any::<bool>(), 0..CURSOR_MS * 2).prop_map(|(s, f, d)| Narrowing::Cursor {
            scope: ReadScope::ALL[s],
            filters: [u8::from(f); 32],
            until: NOW + d,
            after: None
        }),
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
        Action::Claim => (Resource::Project(project()), Act::Claim { project: project() }),
        Action::Handoff => (m1(), Act::Handoff),
        Action::Read => (m1(), Act::Read { scope, filters: filters.map(|f| [u8::from(f); 32]) }),
        Action::Admit => (Resource::Network(network()), Act::Admit { machine: machine("m3"), role: Role::Member }),
        Action::Widen => (Resource::Network(network()), Act::Widen { machine: machine("m1"), settings: Settings::default() }),
        Action::Publish => (Resource::Network(network()), Act::Publish { service: ServiceId::new("web").unwrap(), spec: [0; 32] }),
        Action::RegisterActor => (m1(), Act::RegisterActor { slug: ActorSlug::new("x").unwrap(), rights: vec![Action::Message], scopes: [0; 32] }),
        Action::UseGateway => (m1(), Act::UseGateway),
        Action::Revoke => (m1(), Act::Revoke { what: Revocation::Token(vec![1]) }),
        Action::Delegate => (Resource::Actor(ActorSlug::new("scout").unwrap()), Act::Delegate { project: None }),
        Action::CtxRead | Action::CtxAppend | Action::CtxWrite | Action::CtxAdmin => {
            let level = SpaceLevel::ALL.into_iter().find(|l| l.action() == action).unwrap();
            (Resource::Space(space()), Act::Ctx(level))
        }
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

fn case() -> impl Strategy<Value = (u32, Vec<Narrowing>, usize, usize, i64, usize, Option<bool>)> {
    (any::<u32>(), proptest::collection::vec(narrowing(), 0..4), 0..Action::ALL.len(), 0..Door::ALL.len(), 0..2 * HOUR, 0..ReadScope::ALL.len(), any::<Option<bool>>())
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
        .run(&(any::<u32>(), proptest::collection::vec(narrowing(), 0..4), any::<prop::sample::Index>()), |(mask, narrowings, pick)| {
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

#[test]
fn an_actors_allow_is_never_wider_than_its_scope() {
    let mut runner = TestRunner::new(Config { cases: 256, ..Config::default() });
    let kinds = |mask: u8| PromptKind::ALL.into_iter().enumerate().filter(|(i, _)| mask & (1 << i) != 0).map(|(_, k)| k).collect::<Vec<_>>();
    let sessions = ["s1", "s2", "s3"].map(|s| SessionId::new(s).unwrap());
    let (allowed, refused) = (Cell::new(0), Cell::new(0));
    runner
        .run(
            &(any::<u8>(), proptest::option::of(any::<u8>()), -HOUR..HOUR, 0..3usize, 0..PromptKind::ALL.len(), 0..Attestation::ALL.len(), any::<bool>()),
            |(kind_mask, session_mask, until, s, k, by, deny)| {
                let mut f = world();
                let token = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Answer])).unwrap();
                let id = token.revocation_ids().into_iter().next().unwrap();
                let within = match session_mask {
                    None => Set::All,
                    Some(m) => Set::Only(sessions.iter().enumerate().filter(|(i, _)| m & (1 << i) != 0).map(|(_, s)| s.clone()).collect()),
                };
                let scope = AllowScope { sessions: within, kinds: kinds(kind_mask), until: NOW + until };
                f.world.scopes.insert(id, scope.clone());
                let asked = Asked { session: sessions[s].clone(), by: Attestation::ALL[by], kind: PromptKind::ALL[k], delegated_by: None };
                f.world.asks.insert(AskId::new("q1").unwrap(), asked.clone());
                let decision = if deny { AnswerDecision::Deny } else { AnswerDecision::Allow };
                let r = Request::new(
                    Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") },
                    Resource::Session(asked.session.clone()),
                    Act::Answer { ask: AskId::new("q1").unwrap(), decision },
                    Door::Local,
                    NOW,
                    None,
                );
                let ok = decide(Some(&token), &r, &f.world, None).is_allow();
                if deny {
                    // A Deny with the right is always taken.
                    prop_assert!(ok);
                } else {
                    let inside = scope.until >= NOW && scope.sessions.has(&asked.session) && scope.kinds.contains(&asked.kind) && asked.by >= Attestation::ProcessChecked;
                    prop_assert_eq!(ok, inside, "{:?} {:?}", scope, asked);
                    allowed.set(allowed.get() + usize::from(ok));
                    refused.set(refused.get() + usize::from(!ok));
                }
                Ok(())
            },
        )
        .unwrap();
    assert!(allowed.get() > 0 && refused.get() > 0, "allowed {}, refused {}", allowed.get(), refused.get());
}

#[test]
fn a_share_never_yields_more_than_its_level() {
    // Whoever asks, however they came, with whatever token: an act on a
    // space is allowed only when the space gives them its level or more.
    let mut runner = TestRunner::new(Config { cases: 256, ..Config::default() });
    let (allowed, refused) = (Cell::new(0), Cell::new(0));
    runner
        .run(&(0..5usize, 0..4usize, 0..4usize, any::<bool>(), any::<u32>(), 0..Attestation::ALL.len(), any::<bool>()), |(have, need, kind, remote, mask, by, present)| {
            let mut f = world();
            let have = (have > 0).then(|| SpaceLevel::ALL[have - 1]);
            let need = SpaceLevel::ALL[need];
            f.world.spaces.insert(space(), have);
            f.world.roles.insert((network(), machine("m2")), Role::Member);
            let (here, door, net) = if remote { (machine("m2"), Door::Network, Some(network())) } else { (machine("m1"), Door::Local, None) };
            let (resource, act) = (Resource::Space(space()), Act::Ctx(need));
            // Minted only for an actor: minting is the slow part.
            let token = (kind == 2).then(|| if remote { &f.m2_root } else { &f.m1_root }.mint_actor(&actor_grant("muse", &rights(mask))).unwrap());
            let mut tok = None;
            let mut p = None;
            let principal = match kind {
                0 => Principal::Machine { id: here },
                1 => Principal::Agent { id: AgentId::new("a1").unwrap(), machine: here, attestation: Attestation::ALL[by] },
                2 => {
                    tok = token.as_ref();
                    Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: here }
                }
                _ => as_person(&f.world, &bio_device(), &resource, &act),
            };
            let r = Request::new(principal, resource, act, door, NOW, net);
            let block = (present && kind == 3 && r.act().presence_what().is_some()).then(|| presence(&f.world, &bio_device(), &r));
            if block.is_some() {
                p = block.as_ref();
            }
            let ok = decide(tok, &r, &f.world, p).is_allow();
            prop_assert!(!ok || have.is_some_and(|h| h >= need), "{r:?} allowed at {have:?}");
            allowed.set(allowed.get() + usize::from(ok));
            refused.set(refused.get() + usize::from(!ok));
            Ok(())
        })
        .unwrap();
    assert!(allowed.get() > 0 && refused.get() > 0, "allowed {}, refused {}", allowed.get(), refused.get());
}
