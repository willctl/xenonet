//! Spaces of shared context: every ctx act by every kind of principal,
//! here and from another member, at every level the space's chain may give
//! them; a space this machine doesn't hold; a grant's context key bound
//! only over its own machine's connection; and the schemas Xenolith's
//! grants share the parser with.

use crate::common::*;
use xenonet_authz::*;
use Attestation::{HarnessSigned, ProcessChecked, Unchecked};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Machine,
    Agent(Attestation),
    Actor { rights: bool },
    Person { presence: bool },
}

const KINDS: [Kind; 8] = [
    Kind::Machine,
    Kind::Agent(Unchecked),
    Kind::Agent(ProcessChecked),
    Kind::Agent(HarnessSigned),
    Kind::Actor { rights: true },
    Kind::Actor { rights: false },
    Kind::Person { presence: true },
    Kind::Person { presence: false },
];

/// The rules for an act on a space, as a person would say them.
fn expected(kind: Kind, remote: bool, need: SpaceLevel, have: Option<SpaceLevel>) -> bool {
    let shared = have.is_some_and(|h| h >= need);
    shared
        && match kind {
            Kind::Machine => true,
            Kind::Agent(a) => need != SpaceLevel::Admin && (a >= ProcessChecked || (!remote && need <= SpaceLevel::Append)),
            Kind::Actor { rights } => rights,
            Kind::Person { presence } => need != SpaceLevel::Admin || presence,
        }
}

#[test]
fn every_ctx_act_by_every_principal_at_every_level() {
    let mut f = fixture();
    f.world.roles.insert((network(), machine("m2")), Role::Member);
    // Minted once: a token with every right, and one with no ctx right.
    let tokens: Vec<(bool, bool, Token)> = [(false, true), (false, false), (true, true), (true, false)]
        .into_iter()
        .map(|(remote, all)| {
            let rights = if all { Action::ALL.to_vec() } else { vec![Action::Message] };
            (remote, all, if remote { &f.m2_root } else { &f.m1_root }.mint_actor(&actor_grant("muse", &rights)).unwrap())
        })
        .collect();
    let mut n = 0;
    for kind in KINDS {
        for remote in [false, true] {
            for need in SpaceLevel::ALL {
                for have in [None, Some(SpaceLevel::Read), Some(SpaceLevel::Append), Some(SpaceLevel::Write), Some(SpaceLevel::Admin)] {
                    f.world.spaces.insert(space(), have);
                    let (here, door, net) = if remote { (machine("m2"), Door::Network, Some(network())) } else { (machine("m1"), Door::Local, None) };
                    let (resource, act) = (Resource::Space(space()), Act::Ctx(need));
                    let all = matches!(kind, Kind::Actor { rights: true });
                    let token = tokens.iter().find(|(r, a, _)| *r == remote && *a == all).map(|(_, _, t)| t).unwrap();
                    let principal = match kind {
                        Kind::Machine => Principal::Machine { id: here },
                        Kind::Agent(attestation) => Principal::Agent { id: AgentId::new("a1").unwrap(), machine: here, attestation },
                        Kind::Actor { .. } => Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: here },
                        Kind::Person { .. } => as_person(&f.world, &bio_device(), &resource, &act),
                    };
                    let r = Request::new(principal, resource, act, door, NOW, net);
                    let p = match kind {
                        Kind::Person { presence: true } if need == SpaceLevel::Admin => Some(presence(&f.world, &bio_device(), &r)),
                        _ => None,
                    };
                    let tok = matches!(kind, Kind::Actor { .. }).then_some(token);
                    let d = decide(tok, &r, &f.world, p.as_ref());
                    assert_eq!(d.is_allow(), expected(kind, remote, need, have), "{kind:?} remote {remote}, needs {need:?}, has {have:?}: {:?}", d.reason());
                    n += 1;
                }
            }
        }
    }
    assert_eq!(n, 8 * 2 * 4 * 5);
}

#[test]
fn the_rows_worth_reading() {
    let mut f = fixture();
    let read = |f: &Fixture, principal: Principal, need: SpaceLevel| {
        let r = Request::new(principal, Resource::Space(space()), Act::Ctx(need), Door::Local, NOW, None);
        decide(None, &r, &f.world, None).reason().map(|r| r.code().to_string())
    };
    let agent = |a| Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m1"), attestation: a };
    // At append: reading and appending; writing is refused for its level.
    assert_eq!(read(&f, agent(HarnessSigned), SpaceLevel::Append), None);
    assert_eq!(read(&f, agent(HarnessSigned), SpaceLevel::Write).as_deref(), Some("level"));
    // An unchecked agent writes nowhere, whatever the space gives.
    f.world.spaces.insert(space(), Some(SpaceLevel::Admin));
    assert_eq!(read(&f, agent(Unchecked), SpaceLevel::Write).as_deref(), Some("attestation"));
    assert_eq!(read(&f, agent(HarnessSigned), SpaceLevel::Admin).as_deref(), Some("principal"));
    // The person administers only with a presence block for it.
    let (res, act) = (Resource::Space(space()), Act::Ctx(SpaceLevel::Admin));
    let person = as_person(&f.world, &bio_device(), &res, &act);
    let r = Request::new(person, res, act, Door::Local, NOW, None);
    assert_eq!(decide(None, &r, &f.world, None).reason().unwrap().code(), "presence");
    assert!(decide(None, &r, &f.world, Some(&presence(&f.world, &bio_device(), &r))).is_allow());
    // A space this machine doesn't hold, or one that isn't shared with them.
    f.world.spaces.clear();
    assert_eq!(read(&f, Principal::Machine { id: machine("m1") }, SpaceLevel::Read).as_deref(), Some("resource"));
    f.world.spaces.insert(space(), None);
    assert_eq!(read(&f, Principal::Machine { id: machine("m1") }, SpaceLevel::Read).as_deref(), Some("level"));
    // A ctx act is done to a space, and nothing else is.
    let wrong = Request::new(agent(HarnessSigned), Resource::Machine(machine("m1")), Act::Ctx(SpaceLevel::Read), Door::Local, NOW, None);
    assert_eq!(decide(None, &wrong, &f.world, None).reason().unwrap().code(), "resource");
}

#[test]
fn an_actor_reads_a_space_through_the_gateway_with_both_rights() {
    let f = fixture();
    let actor = Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: machine("m1") };
    let r = Request::new(actor, Resource::Space(space()), Act::Ctx(SpaceLevel::Read), Door::Gateway, NOW, None);
    let both = f.m1_root.mint_actor(&actor_grant("muse", &[Action::CtxRead, Action::UseGateway])).unwrap();
    let ctx_only = f.m1_root.mint_actor(&actor_grant("muse", &[Action::CtxRead])).unwrap();
    let other = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Read, Action::UseGateway])).unwrap();
    assert!(decide(Some(&both), &r, &f.world, None).is_allow());
    assert_eq!(decide(Some(&ctx_only), &r, &f.world, None).reason().unwrap().code(), "right");
    assert_eq!(decide(Some(&other), &r, &f.world, None).reason().unwrap().code(), "right", "reading the map isn't reading a space");
}

#[test]
fn a_grants_key_binds_only_over_its_own_machines_connection() {
    let mut f = fixture();
    f.world.roles.insert((network(), machine("m2")), Role::Member);
    let key = "xns1_aaaqeayeaudaocajbifqydiob4ibceqt";
    let binding = Binding::new(Some(key), None).unwrap();
    let token = f.m2_root.mint_actor_with(&actor_grant("xenolith", &[Action::CtxRead, Action::CtxAppend]), &Schema::XENONET, &binding).unwrap();
    // It travels and reads back with its key.
    let back = Token::parse(&token.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &f.world, NOW).unwrap();
    assert_eq!(back.key(), Some(key));
    // Over m2's own connection the key is m2's actor's; over another
    // machine's it binds nothing.
    assert_eq!(back.key_over(&machine("m2")), Some(key));
    assert_eq!(back.key_over(&machine("m3")), None);
    assert_eq!(back.key_over(&machine("m1")), None);
    // And decide refuses the grant from any machine but its own: the
    // principal's machine is the connection's.
    let over = |m: &str| {
        Request::new(
            Principal::Actor { slug: ActorSlug::new("xenolith").unwrap(), machine: machine(m) },
            Resource::Space(space()),
            Act::Ctx(SpaceLevel::Read),
            Door::Network,
            NOW,
            Some(network()),
        )
    };
    assert!(decide(Some(&back), &over("m2"), &f.world, None).is_allow());
    f.world.roles.insert((network(), machine("m3")), Role::Member);
    assert_eq!(decide(Some(&back), &over("m3"), &f.world, None).reason().unwrap().code(), "token");
    // A key that isn't a signing key's id isn't one.
    assert!(Binding::new(Some("xnk1_aaaqeayeaudaocajbifqydiob4ibceqt"), None).is_err());
    assert!(Binding::new(None, Some("Xenolith/Sync")).is_err());
}

#[test]
fn xenoliths_grants_share_the_parser_and_hold_only_ctx_rights() {
    let f = fixture();
    let ctx = [Action::CtxRead, Action::CtxAppend];
    let grant = actor_grant("client-1", &ctx);
    let key = Binding::new(Some("xns1_aaaqeayeaudaocajbifqydiob4ibceqt"), None).unwrap();
    let theirs = f.m1_root.mint_actor_with(&grant, &Schema::XENOLITH, &key).unwrap();
    let text = theirs.encode("xenolith_client_").unwrap();
    let back = Token::parse_with(&text, "xenolith_client_", &Schema::XENOLITH, &f.world, NOW).unwrap();
    assert_eq!(back.rights(), &ctx);
    // Xenolith's grants hold no XenoNet right and no forward.
    assert!(f.m1_root.mint_actor_with(&actor_grant("client-1", &[Action::Message]), &Schema::XENOLITH, &Binding::default()).is_err());
    let fwd = Binding::new(None, Some("xenolith/sync/2")).unwrap();
    assert!(f.m1_root.mint_actor_with(&grant, &Schema::XENOLITH, &fwd).is_err());
    // A XenoNet grant with a right Xenolith's schema hasn't, or with a
    // forward, read as Xenolith's: refused.
    let wide = f.m1_root.mint_actor(&actor_grant("muse", &[Action::Message, Action::CtxRead])).unwrap().encode(ACTOR_PREFIX).unwrap();
    assert_eq!(Token::parse_with(&wide, ACTOR_PREFIX, &Schema::XENOLITH, &f.world, NOW).unwrap_err().code, "token");
    let forwarding = f.m1_root.mint_actor_with(&grant, &Schema::XENONET, &fwd).unwrap().encode(ACTOR_PREFIX).unwrap();
    assert_eq!(Token::parse(&forwarding, ACTOR_PREFIX, &f.world, NOW).unwrap().forward(), Some("xenolith/sync/2"));
    assert_eq!(Token::parse_with(&forwarding, ACTOR_PREFIX, &Schema::XENOLITH, &f.world, NOW).unwrap_err().code, "token");
    // What the person approves covers the key and the forward.
    let rows = [7; 32];
    let hashes = [Binding::default().scopes(&rows), key.scopes(&rows), fwd.scopes(&rows), Binding::default().scopes(&[8; 32])];
    let distinct: std::collections::HashSet<_> = hashes.iter().collect();
    assert_eq!(distinct.len(), hashes.len());
}
