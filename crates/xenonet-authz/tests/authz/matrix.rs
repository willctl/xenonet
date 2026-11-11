//! Every action (an answer's Allow and Deny apart), by every kind of
//! principal, here and from another machine in every role, under every
//! start and wake setting, consent, delegation and governance age: `decide` against `expected`, a plain
//! statement of the rules written apart from the code. `ROWS` spells out
//! the cases worth reading, and checks both.

use crate::common::*;
use xenonet_authz::*;
use Attestation::{HarnessSigned, ProcessChecked, Unchecked};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Machine,
    Agent(Attestation),
    /// An actor whose token has every right, or none.
    Actor {
        rights: bool,
    },
    /// The person, with a presence for the act or without.
    Person {
        presence: bool,
    },
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

/// Here at a local door, or from the network with the sender's machine in
/// that role (None: not a member).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Here,
    Remote(Option<Role>),
}

const PLACES: [Place; 5] = [Place::Here, Place::Remote(None), Place::Remote(Some(Role::Member)), Place::Remote(Some(Role::Admin)), Place::Remote(Some(Role::Founder))];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Do {
    Message,
    Wake,
    StartAsks,
    StartDelegated,
    Answer,
    Deny,
    Claim,
    Handoff,
    ReadMap,
    ReadTranscripts,
    Admit,
    Widen,
    Publish,
    RegisterActor,
    UseGateway,
    Revoke,
}

const DOS: [Do; 16] = [
    Do::Message,
    Do::Wake,
    Do::StartAsks,
    Do::StartDelegated,
    Do::Answer,
    Do::Deny,
    Do::Claim,
    Do::Handoff,
    Do::ReadMap,
    Do::ReadTranscripts,
    Do::Admit,
    Do::Widen,
    Do::Publish,
    Do::RegisterActor,
    Do::UseGateway,
    Do::Revoke,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Allow,
    Hold,
    Refuse,
}

#[derive(Debug, Clone, Copy)]
struct Case {
    what: Do,
    kind: Kind,
    place: Place,
    start: StartSetting,
    wake: WakeSetting,
    consent: Consent,
    grant: bool,
    stale: bool,
}

/// The rules, as a person would say them.
fn expected(c: &Case) -> Outcome {
    use Kind::*;
    let remote = c.place != Place::Here;
    let role = match c.place {
        Place::Here => None,
        Place::Remote(r) => r,
    };
    // From another machine: a member's, or the person.
    let reaches = match c.kind {
        Person { .. } => true,
        _ => !remote || role.is_some(),
    };
    let tokened = !matches!(c.kind, Actor { rights: false });
    let present = matches!(c.kind, Person { presence: true });
    let attested = |least| match c.kind {
        Agent(a) => a >= least,
        _ => true,
    };
    let starts = || match c.kind {
        _ if c.start == StartSetting::Off => false,
        Person { .. } => c.what == Do::StartAsks || c.grant,
        Agent(Unchecked) | Machine => false,
        _ if c.start == StartSetting::You => false,
        _ if c.what == Do::StartAsks => true,
        _ => c.start == StartSetting::Delegate && c.grant && attested(ProcessChecked),
    };
    let allowed = reaches
        && tokened
        && match c.what {
            Do::Message | Do::Handoff => true,
            Do::Wake => match c.kind {
                Person { .. } => true,
                Actor { .. } => c.wake == WakeSetting::Network || c.consent.actors,
                _ => c.wake == WakeSetting::Network || c.consent.here,
            },
            Do::StartAsks | Do::StartDelegated => starts(),
            // An Allow: the person there, or this machine under the
            // delegation (its question is a command prompt, from a run m2
            // started under it, and the delegation names every device). No
            // actor here has an allow scope.
            Do::Answer => present || (c.kind == Machine && !remote && c.grant),
            // A Deny narrows: the person, this machine, a window host, an
            // actor with the right; never an agent.
            Do::Deny => match c.kind {
                Person { .. } => true,
                Machine => !remote || role >= Some(Role::Admin),
                Actor { .. } => true,
                Agent(_) => false,
            },
            Do::Claim => !(remote && c.kind == Agent(Unchecked)),
            Do::ReadMap => true,
            Do::ReadTranscripts => match c.kind {
                Person { .. } => true,
                _ if !remote => attested(ProcessChecked),
                _ => role >= Some(Role::Admin) && attested(ProcessChecked),
            },
            Do::Admit | Do::Widen | Do::Publish | Do::Revoke => present,
            // Issuing or widening an actor's grant needs the person there.
            Do::RegisterActor => matches!(c.kind, Actor { .. }) || present,
            Do::UseGateway => matches!(c.kind, Actor { .. }),
        };
    let holds = matches!(c.what, Do::StartAsks | Do::StartDelegated | Do::Answer | Do::Admit | Do::Widen | Do::Publish);
    match (allowed, holds && remote && c.stale) {
        (false, _) => Outcome::Refuse,
        (true, true) => Outcome::Hold,
        (true, false) => Outcome::Allow,
    }
}

fn act_of(what: Do) -> (Resource, Act) {
    let m1 = || Resource::Machine(machine("m1"));
    let run = || Resource::Run { machine: machine("m1"), app: AppId::new("claude").unwrap(), folder: Folder::new("/work").unwrap() };
    let start = |mode| Act::Start { app: AppId::new("claude").unwrap(), folder: Folder::new("/work").unwrap(), mode };
    let n1 = || Resource::Network(network());
    match what {
        Do::Message => (m1(), Act::Message),
        Do::Wake => (Resource::Session(SessionId::new("s1").unwrap()), Act::Wake),
        Do::StartAsks => (run(), start(StartMode::Asks)),
        Do::StartDelegated => (run(), start(StartMode::Delegated)),
        Do::Answer => (run(), Act::Answer { ask: AskId::new("q1").unwrap(), decision: AnswerDecision::Allow }),
        Do::Deny => (run(), Act::Answer { ask: AskId::new("q1").unwrap(), decision: AnswerDecision::Deny }),
        Do::Claim => (m1(), Act::Claim),
        Do::Handoff => (m1(), Act::Handoff),
        Do::ReadMap => (m1(), Act::Read { scope: ReadScope::Map, filters: None }),
        Do::ReadTranscripts => (m1(), Act::Read { scope: ReadScope::Transcripts, filters: None }),
        Do::Admit => (n1(), Act::Admit { machine: machine("m3"), role: Role::Member }),
        Do::Widen => (n1(), Act::Widen { machine: machine("m1"), settings: Settings { start: StartSetting::Network, wake: WakeSetting::Network } }),
        Do::Publish => (n1(), Act::Publish { service: ServiceId::new("web").unwrap() }),
        Do::RegisterActor => (m1(), Act::RegisterActor { slug: ActorSlug::new("helper").unwrap(), rights: vec![Action::Message], scopes: [0; 32] }),
        Do::UseGateway => (m1(), Act::UseGateway),
        Do::Revoke => (m1(), Act::Revoke { what: Revocation::Device(device_id(&weak_device())) }),
    }
}

/// What each case needs made once: tokens, and the person's signature and
/// presence for each act.
struct Made {
    f: Fixture,
    tokens: Vec<(MachineId, bool, Token)>,
    person: Vec<(Do, Principal, VerifiedPresence)>,
}

fn made() -> Made {
    let f = fixture();
    let mut tokens = Vec::new();
    for (root, m) in [(&f.m1_root, "m1"), (&f.m2_root, "m2")] {
        for rights in [true, false] {
            let all = if rights { Action::ALL.to_vec() } else { Vec::new() };
            tokens.push((machine(m), rights, root.mint_actor(&actor_grant("muse", &all)).unwrap()));
        }
    }
    let person = DOS
        .iter()
        .map(|&what| {
            let (resource, act) = act_of(what);
            let principal = as_person(&f.world, &bio_device(), &resource, &act);
            let r = Request::new(principal.clone(), resource, act.clone(), Door::Local, NOW, None);
            let p = act.presence_what().map(|_| presence(&f.world, &bio_device(), &r));
            // Acts that take no presence get one for another act, which the
            // decision never reads.
            let p = p.unwrap_or_else(|| {
                let (res, act) = act_of(Do::Admit);
                let other = Request::new(as_person(&f.world, &bio_device(), &res, &act), res, act, Door::Local, NOW, None);
                presence(&f.world, &bio_device(), &other)
            });
            (what, principal, p)
        })
        .collect();
    Made { f, tokens, person }
}

fn run_case(m: &mut Made, c: &Case) -> Decision {
    let w = &mut m.f.world;
    w.settings = Settings { start: c.start, wake: c.wake };
    w.consent = c.consent;
    w.delegations = if c.grant { vec![delegation()] } else { Vec::new() };
    w.synced = Some(if c.stale { NOW - 11 * 60_000 } else { NOW - 60_000 });
    match c.place {
        Place::Remote(Some(role)) => {
            w.roles.insert((network(), machine("m2")), role);
        }
        _ => {
            w.roles.remove(&(network(), machine("m2")));
        }
    }
    let (here, door, net) = match c.place {
        Place::Here => (machine("m1"), Door::Local, None),
        Place::Remote(_) => (machine("m2"), Door::Network, Some(network())),
    };
    let (resource, act) = act_of(c.what);
    let mut token = None;
    let mut presence = None;
    let principal = match c.kind {
        Kind::Machine => Principal::Machine { id: here },
        Kind::Agent(attestation) => Principal::Agent { id: AgentId::new("a1").unwrap(), machine: here, attestation },
        Kind::Actor { rights } => {
            token = m.tokens.iter().find(|(tm, r, _)| *tm == here && *r == rights).map(|(_, _, t)| t);
            Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: here }
        }
        Kind::Person { presence: with } => {
            let (_, principal, p) = m.person.iter().find(|(d, _, _)| *d == c.what).unwrap();
            presence = with.then_some(p);
            principal.clone()
        }
    };
    let r = Request::new(principal, resource, act, door, NOW, net);
    decide(token, &r, &m.f.world, presence)
}

fn outcome(d: &Decision) -> Outcome {
    match d {
        Decision::Allow(_) => Outcome::Allow,
        Decision::Hold(_) => Outcome::Hold,
        Decision::Refuse(_) => Outcome::Refuse,
    }
}

#[test]
fn every_case_decides_as_the_rules_say() {
    let mut m = made();
    let consents = [(false, false), (true, false), (false, true), (true, true)].map(|(here, actors)| Consent { here, actors });
    let (mut n, mut allowed, mut held) = (0, 0, 0);
    for what in DOS {
        for kind in KINDS {
            for place in PLACES {
                for start in StartSetting::ALL {
                    for wake in WakeSetting::ALL {
                        for consent in consents {
                            for grant in [false, true] {
                                for stale in [false, true] {
                                    let c = Case { what, kind, place, start, wake, consent, grant, stale };
                                    let d = run_case(&mut m, &c);
                                    let want = expected(&c);
                                    assert_eq!(outcome(&d), want, "{c:?}: {:?}", d.reason());
                                    n += 1;
                                    allowed += usize::from(want == Outcome::Allow);
                                    held += usize::from(want == Outcome::Hold);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(n, 16 * 8 * 5 * 4 * 2 * 4 * 2 * 2);
    // Neither side of the matrix is empty.
    assert!(allowed > 1000 && held > 100, "{allowed} allowed, {held} held of {n}");
}

/// The cases worth reading, each with the reason's code when refused.
#[rustfmt::skip]
const ROWS: [(Do, Kind, Place, StartSetting, Outcome, &str); 29] = [
    // Messages: members, local agents, actors with the right.
    (Do::Message,         Kind::Agent(Unchecked),           Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::Message,         Kind::Agent(HarnessSigned),       Place::Remote(None),               StartSetting::You,      Outcome::Refuse, "member"),
    (Do::Message,         Kind::Actor { rights: false },    Place::Here,                       StartSetting::You,      Outcome::Refuse, "right"),
    (Do::ReadMap,         Kind::Actor { rights: false },    Place::Here,                       StartSetting::You,      Outcome::Refuse, "right"),
    // Starts: off refuses all; you is the person's; never an unchecked agent.
    (Do::StartAsks,       Kind::Person { presence: false }, Place::Here,                       StartSetting::Off,      Outcome::Refuse, "settings"),
    (Do::StartAsks,       Kind::Person { presence: false }, Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::StartAsks,       Kind::Agent(HarnessSigned),       Place::Here,                       StartSetting::You,      Outcome::Refuse, "settings"),
    (Do::StartAsks,       Kind::Agent(ProcessChecked),      Place::Remote(Some(Role::Member)), StartSetting::Network,  Outcome::Allow,  ""),
    (Do::StartAsks,       Kind::Agent(Unchecked),           Place::Here,                       StartSetting::Network,  Outcome::Refuse, "attestation"),
    (Do::StartDelegated,  Kind::Actor { rights: true },     Place::Here,                       StartSetting::Network,  Outcome::Refuse, "settings"),
    (Do::StartDelegated,  Kind::Actor { rights: true },     Place::Here,                       StartSetting::Delegate, Outcome::Refuse, "grant"),
    (Do::StartAsks,       Kind::Machine,                    Place::Here,                       StartSetting::Network,  Outcome::Refuse, "principal"),
    // An Allow: the person present, or this machine under the delegation;
    // an actor only inside an allow scope.
    (Do::Answer,          Kind::Person { presence: false }, Place::Here,                       StartSetting::You,      Outcome::Refuse, "presence"),
    (Do::Answer,          Kind::Person { presence: true },  Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::Answer,          Kind::Agent(HarnessSigned),       Place::Here,                       StartSetting::Delegate, Outcome::Refuse, "principal"),
    (Do::Answer,          Kind::Machine,                    Place::Here,                       StartSetting::You,      Outcome::Refuse, "grant"),
    (Do::Answer,          Kind::Actor { rights: true },     Place::Here,                       StartSetting::You,      Outcome::Refuse, "scope"),
    // A Deny: no presence, and no window host's relay but an admin's.
    (Do::Deny,            Kind::Person { presence: false }, Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::Deny,            Kind::Machine,                    Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::Deny,            Kind::Machine,                    Place::Remote(Some(Role::Member)), StartSetting::You,      Outcome::Refuse, "role"),
    (Do::Deny,            Kind::Machine,                    Place::Remote(Some(Role::Admin)),  StartSetting::You,      Outcome::Allow,  ""),
    (Do::Deny,            Kind::Actor { rights: true },     Place::Remote(Some(Role::Member)), StartSetting::You,      Outcome::Allow,  ""),
    (Do::Deny,            Kind::Agent(HarnessSigned),       Place::Here,                       StartSetting::You,      Outcome::Refuse, "principal"),
    // Claims: an unchecked agent only on its own machine.
    (Do::Claim,           Kind::Agent(Unchecked),           Place::Here,                       StartSetting::You,      Outcome::Allow,  ""),
    (Do::Claim,           Kind::Agent(Unchecked),           Place::Remote(Some(Role::Admin)),  StartSetting::You,      Outcome::Refuse, "attestation"),
    // Reads that matter: attested here, admins from elsewhere.
    (Do::ReadTranscripts, Kind::Agent(Unchecked),           Place::Here,                       StartSetting::You,      Outcome::Refuse, "attestation"),
    (Do::ReadTranscripts, Kind::Machine,                    Place::Remote(Some(Role::Member)), StartSetting::You,      Outcome::Refuse, "role"),
    (Do::ReadTranscripts, Kind::Machine,                    Place::Remote(Some(Role::Admin)),  StartSetting::You,      Outcome::Allow,  ""),
    // The widest acts: the person, present.
    (Do::Admit,           Kind::Actor { rights: true },     Place::Here,                       StartSetting::You,      Outcome::Refuse, "principal"),
];

#[test]
fn the_rows_worth_reading() {
    let mut m = made();
    for (what, kind, place, start, want, code) in ROWS {
        let c = Case { what, kind, place, start, wake: WakeSetting::Consent, consent: Consent::default(), grant: false, stale: false };
        assert_eq!(expected(&c), want, "the model, {c:?}");
        let d = run_case(&mut m, &c);
        assert_eq!(outcome(&d), want, "{c:?}: {:?}", d.reason());
        assert_eq!(d.reason().map_or("", Reason::code), code, "{c:?}");
    }
}

#[test]
fn a_remote_start_holds_on_stale_governance_and_a_local_one_doesnt() {
    let mut m = made();
    let c = |place| Case {
        what: Do::StartAsks,
        kind: Kind::Agent(HarnessSigned),
        place,
        start: StartSetting::Network,
        wake: WakeSetting::Consent,
        consent: Consent::default(),
        grant: false,
        stale: true,
    };
    let held = run_case(&mut m, &c(Place::Remote(Some(Role::Member))));
    assert_eq!(held.reason().unwrap().code(), "stale");
    assert!(run_case(&mut m, &c(Place::Here)).is_allow());
    // Never synced counts as stale.
    m.f.world.synced = None;
    let r = Request::new(
        Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine("m2"), attestation: HarnessSigned },
        act_of(Do::StartAsks).0,
        act_of(Do::StartAsks).1,
        Door::Network,
        NOW,
        Some(network()),
    );
    m.f.world.roles.insert((network(), machine("m2")), Role::Member);
    m.f.world.settings.start = StartSetting::Network;
    assert!(matches!(decide(None, &r, &m.f.world, None), Decision::Hold(_)));
}

#[test]
fn a_delegation_answers_only_what_it_names_on_the_devices_it_names() {
    let mut f = fixture();
    let (resource, act) = act_of(Do::Answer);
    let r = Request::new(Principal::Machine { id: machine("m1") }, resource, act, Door::Local, NOW, None);
    let ask = AskId::new("q1").unwrap();
    let decides = |f: &Fixture| decide(None, &r, &f.world, None);
    f.world.delegations = vec![delegation()];
    let Decision::Allow(a) = decides(&f) else { panic!("allowed") };
    assert_eq!(a.authority(), &Authority::StandingGrant);
    // Only for a run whose agent was checked.
    for (by, ok) in [(Unchecked, false), (ProcessChecked, true), (HarnessSigned, true)] {
        f.world.asks.insert(ask.clone(), asked(by));
        assert_eq!(decides(&f).is_allow(), ok, "{by:?}");
    }
    // The devices: this machine's and the one the run was started from,
    // both named and both still the person's.
    let only = |ds: &[&dyn xenonet_id::sign::Signer]| StandingGrant { devices: Set::Only(ds.iter().map(|d| device_id(*d)).collect()), ..delegation() };
    for (grant, ok) in [(only(&[&bio_device()]), false), (only(&[&weak_device()]), false), (only(&[&bio_device(), &weak_device()]), true)] {
        f.world.delegations = vec![grant];
        assert_eq!(decides(&f).is_allow(), ok);
    }
    f.world.devices.remove(&(person(), device_id(&weak_device())));
    assert_eq!(decides(&f).reason().unwrap().code(), "grant", "a revoked device is named by no delegation");
    let mut f = fixture();
    // Another kind of prompt, an ended delegation, none.
    for grant in [StandingGrant { answers: vec![PromptKind::Read, PromptKind::Edit], ..delegation() }, StandingGrant { until: NOW - 1, ..delegation() }] {
        f.world.delegations = vec![grant];
        assert_eq!(decides(&f).reason().unwrap().code(), "grant");
    }
    // A run that wasn't started under it.
    f.world.delegations = vec![delegation()];
    f.world.asks.insert(ask.clone(), Asked { delegated_by: None, ..asked(HarnessSigned) });
    assert_eq!(decides(&f).reason().unwrap().code(), "grant");
    // A question that isn't waiting here.
    f.world.asks.clear();
    assert_eq!(decides(&f).reason().unwrap().code(), "ask");
}

#[test]
fn a_delegated_start_needs_a_delegation_naming_both_devices() {
    let mut f = fixture();
    f.world.settings.start = StartSetting::Delegate;
    f.world.roles.insert((network(), machine("m2")), Role::Member);
    let (resource, act) = act_of(Do::StartDelegated);
    let agent = |m: &str| Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine(m), attestation: ProcessChecked };
    let from_m2 = Request::new(agent("m2"), resource.clone(), act.clone(), Door::Network, NOW, Some(network()));
    let only_here = StandingGrant { devices: Set::Only(vec![device_id(&bio_device())]), ..delegation() };
    f.world.delegations = vec![only_here.clone()];
    assert_eq!(decide(None, &from_m2, &f.world, None).reason().unwrap().code(), "grant", "m2's device isn't named");
    // Here, the device is this machine's own, which it names.
    assert!(decide(None, &Request::new(agent("m1"), resource.clone(), act.clone(), Door::Local, NOW, None), &f.world, None).is_allow());
    // A machine with no device of the person's isn't named by any.
    f.world.delegations = vec![delegation()];
    assert!(decide(None, &from_m2, &f.world, None).is_allow());
    f.world.machine_devices.remove(&machine("m2"));
    assert_eq!(decide(None, &from_m2, &f.world, None).reason().unwrap().code(), "grant");
    // One that doesn't start.
    f.world.machine_devices.insert(machine("m2"), device_id(&weak_device()));
    f.world.delegations = vec![StandingGrant { starts: false, ..delegation() }];
    assert_eq!(decide(None, &from_m2, &f.world, None).reason().unwrap().code(), "grant");
}

#[test]
fn a_resource_or_door_that_doesnt_fit_is_refused() {
    let f = fixture();
    let agent = |m: &str| Principal::Agent { id: AgentId::new("a1").unwrap(), machine: machine(m), attestation: HarnessSigned };
    let refuse = |r: Request| decide(None, &r, &f.world, None).reason().map(Reason::code);
    // Another machine's resource: it decides for itself.
    assert_eq!(refuse(Request::new(agent("m1"), Resource::Machine(machine("m2")), Act::Claim, Door::Local, NOW, None)), Some("resource"));
    // A wake is done to a session.
    assert_eq!(refuse(Request::new(agent("m1"), Resource::Machine(machine("m1")), Act::Wake, Door::Local, NOW, None)), Some("resource"));
    // A start whose run isn't its act's.
    let other = Resource::Run { machine: machine("m1"), app: AppId::new("codex").unwrap(), folder: Folder::new("/work").unwrap() };
    assert_eq!(refuse(Request::new(agent("m1"), other, act_of(Do::StartAsks).1, Door::Local, NOW, None)), Some("resource"));
    // Another machine's agent at a local door; this machine's from the
    // network; anything but an actor at the gateway.
    assert_eq!(refuse(Request::new(agent("m2"), Resource::Machine(machine("m1")), Act::Claim, Door::Local, NOW, None)), Some("door"));
    assert_eq!(refuse(Request::new(agent("m1"), Resource::Machine(machine("m1")), Act::Claim, Door::Network, NOW, Some(network()))), Some("door"));
    assert_eq!(refuse(Request::new(agent("m1"), Resource::Machine(machine("m1")), Act::Claim, Door::Gateway, NOW, None)), Some("door"));
    // From the network without naming one, or one this machine isn't in.
    assert_eq!(refuse(Request::new(agent("m2"), Resource::Machine(machine("m1")), Act::Claim, Door::Network, NOW, None)), Some("network"));
    assert_eq!(refuse(Request::new(agent("m2"), Resource::Machine(machine("m1")), Act::Claim, Door::Network, NOW, Some(NetworkId::new("n9").unwrap()))), Some("network"));
    // A token where none is taken.
    let t = f.m1_root.mint_actor(&actor_grant("muse", &Action::ALL)).unwrap();
    let r = Request::new(agent("m1"), Resource::Machine(machine("m1")), Act::Claim, Door::Local, NOW, None);
    assert_eq!(decide(Some(&t), &r, &f.world, None).reason().unwrap().code(), "token");
}
