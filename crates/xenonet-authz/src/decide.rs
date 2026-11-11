//! The one decision, at the receiver: [`decide`] judges a request on the
//! receiver's own facts and returns Allow (with what it allowed), Hold or
//! Refuse. The sender's own check is a courtesy, and what a sender says of
//! its strength is never read: a remote agent's attestation is its
//! machine's statement, a person is a checked device signature, presence
//! is a checked presence block, an actor is a checked token.
//!
//! A request goes through these steps, in order, and the first refusal is
//! the answer:
//!
//! 1. **The resource** fits the action and is this machine's: a machine or
//!    run here, a network this machine is in, a session here.
//! 2. **The door**: at a local door, only this machine's principals and
//!    the person; from the network, a member of the network the envelope
//!    names, never this machine itself; at the gateway and A2A, only this
//!    machine's actors.
//! 3. **Who**: the person's device signature is for this act, from an
//!    unrevoked device of this machine's person; an actor (a bot or a cloud
//!    assistant too) holds a token for exactly them, whose root is still certified, with
//!    no id revoked, whose Datalog grants the action's right (and the
//!    gateway's, at the gateway or A2A) and whose own checks pass. Nobody
//!    else brings a token.
//! 4. **The rule table** (`TABLE`), one row per action and one column per
//!    kind of principal:
//!
//! ```text
//!                  machine   agent     actor     person
//!   message        reach     reach     reach     reach
//!   wake           wake      wake      wake      reach
//!   start          -         start     start     start
//!   answer         answer    -         answer    answer
//!   claim          reach     claim     reach     reach
//!   handoff        reach     reach     reach     reach
//!   read           read      read      read      reach
//!   admit          -         -         -         presence
//!   widen          -         -         -         presence
//!   publish        -         -         -         presence
//!   register-actor -         -         reach     presence
//!   use-gateway    -         -         reach     -
//!   revoke         -         -         -         presence
//!
//!   -         refused
//!   reach     steps 1 to 3 are enough (an actor's token has the right)
//!   wake      the machine's wake setting is network, or the session's
//!             consent allows it: actors need consent for actors
//!   start     start off refuses everyone; you lets only the person;
//!             network and delegate let agents and actors too, but never
//!             an unchecked agent; a delegated run needs start delegate
//!             (the person needs none) and a delegation of the person's
//!             for starts that names this machine's device and the device
//!             the start came from
//!   claim     reach, but an unchecked agent claims only on its own machine
//!   read      the map: reach; transcripts, search and state: here, an
//!             agent process-checked at least; from another machine, an admin
//!             member, never an unchecked agent
//!   presence  the person, with a presence block for this act from the
//!             device that signed it; a weaker device is allowed and the
//!             decision says so. Register-actor is issuing an actor's
//!             grant or widening it; narrowing and revoking need no act
//!   answer    a Deny can only narrow, so it needs no presence: the person;
//!             this machine for its own window; a window host (an admin)
//!             relaying its window's; an actor whose token has the right.
//!             An Allow widens, and never for a run whose agent is
//!             unchecked: the person, with a presence block for it; an
//!             actor, inside the allow scope its grant's row holds here
//!             (sessions, prompt kinds, until); this machine itself, for a
//!             run started under a delegation of the person's that answers
//!             that kind of prompt and names this machine's device and the
//!             device the run was started from
//! ```
//!
//! 5. **Hold**: start, answer, admit, widen and publish for another machine
//!    (a principal not of this machine, or the person over the network)
//!    hold instead of allowing when governance was last synced more than
//!    ten minutes ago, or never. This machine acting on its own is exempt,
//!    so a laptop on a plane still answers its own prompts, and so is a
//!    Deny, which only narrows.

use crate::proof::VerifiedPresence;
use crate::vocab::{Act, Action, AnswerDecision, Attestation, DeviceId, NetworkId, Principal, ReadScope, Request, Resource, StartMode};
use crate::{DeviceKind, Door, Ground, Role, StandingGrant, StartSetting, Token, WakeSetting};
use std::fmt;

/// How stale governance may be before acts for another machine hold.
pub const HOLD_AFTER_MS: i64 = 10 * 60_000;

/// Why a request was held or refused: a short code and words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    code: &'static str,
    text: String,
}

impl Reason {
    fn new(code: &'static str, text: impl Into<String>) -> Reason {
        Reason { code, text: text.into() }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// What the allowance rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authority {
    /// The principal is this machine's, or a member's over the network.
    Membership { network: Option<NetworkId>, role: Option<Role> },
    /// A token's right, by the token's first revocation id.
    Token { revocation_id: Vec<u8> },
    /// The person's device signed the act.
    Person { device: DeviceId },
    /// The person was there, at a device of this kind.
    Presence { device: DeviceId, kind: DeviceKind },
    /// A delegation of the person's.
    StandingGrant,
    /// An actor's allow scope, by its token's first revocation id.
    AllowScope { revocation_id: Vec<u8> },
}

/// An allowed request, as only [`decide`] makes one: who, what, to what,
/// the act's hash and the authority it rests on, with a note when the
/// allowance is weaker than it could be.
#[derive(Debug)]
pub struct Authorized {
    principal: Principal,
    action: Action,
    resource: Resource,
    act_hash: [u8; 32],
    network: Option<NetworkId>,
    authority: Authority,
    note: Option<&'static str>,
}

impl Authorized {
    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    pub fn action(&self) -> Action {
        self.action
    }

    pub fn resource(&self) -> &Resource {
        &self.resource
    }

    pub fn act_hash(&self) -> &[u8; 32] {
        &self.act_hash
    }

    pub fn network(&self) -> Option<&NetworkId> {
        self.network.as_ref()
    }

    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    /// Said when the allowance is weaker than it could be (a device that
    /// isn't biometric).
    pub fn note(&self) -> Option<&'static str> {
        self.note
    }
}

#[derive(Debug)]
pub enum Decision {
    /// Boxed: what was allowed is far larger than a reason.
    Allow(Box<Authorized>),
    /// Not now: ask again once governance is synced.
    Hold(Reason),
    Refuse(Reason),
}

impl Decision {
    pub fn is_allow(&self) -> bool {
        matches!(self, Decision::Allow(_))
    }

    /// Why it was held or refused.
    pub fn reason(&self) -> Option<&Reason> {
        match self {
            Decision::Allow(_) => None,
            Decision::Hold(r) | Decision::Refuse(r) => Some(r),
        }
    }
}

/// One cell of the rule table: what else a principal of that kind needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    No,
    Reach,
    Wake,
    Start,
    Claim,
    Read,
    Presence,
    Answer,
}

use Rule::{Answer, Claim, No, Presence, Reach, Read, Start, Wake};

/// The rule table (the module's doc draws it). Columns: machine, agent,
/// actor, person.
#[rustfmt::skip]
const TABLE: [(Action, [Rule; 4]); 13] = [
    //                       machine   agent     actor     person
    (Action::Message,       [Reach,    Reach,    Reach,    Reach]),
    (Action::Wake,          [Wake,     Wake,     Wake,     Reach]),
    (Action::Start,         [No,       Start,    Start,    Start]),
    (Action::Answer,        [Answer,   No,       Answer,   Answer]),
    (Action::Claim,         [Reach,    Claim,    Reach,    Reach]),
    (Action::Handoff,       [Reach,    Reach,    Reach,    Reach]),
    (Action::Read,          [Read,     Read,     Read,     Reach]),
    (Action::Admit,         [No,       No,       No,       Presence]),
    (Action::Widen,         [No,       No,       No,       Presence]),
    (Action::Publish,       [No,       No,       No,       Presence]),
    (Action::RegisterActor, [No,       No,       Reach,    Presence]),
    (Action::UseGateway,    [No,       No,       Reach,    No]),
    (Action::Revoke,        [No,       No,       No,       Presence]),
];

/// The actions that hold for another machine on stale governance.
const HOLDS: [Action; 5] = [Action::Start, Action::Answer, Action::Admit, Action::Widen, Action::Publish];

fn rule(action: Action, principal: &Principal) -> Rule {
    let column = match principal {
        Principal::Machine { .. } => 0,
        Principal::Agent { .. } => 1,
        Principal::Actor { .. } => 2,
        Principal::Person(_) => 3,
    };
    TABLE.iter().find(|(a, _)| *a == action).map_or(No, |(_, row)| row[column])
}

// How a request came in: whether its principal is this machine's own (or
// the person here), and the role of its machine when it came from another.
struct Arrival {
    own: bool,
    role: Option<Role>,
}

type Step<T> = Result<T, Reason>;

/// Decides `request` at this machine. `token` is the token the request
/// came with, verified (`Token::parse`); `presence` the person's presence
/// for this act, checked (`VerifiedPresence::check`).
pub fn decide(token: Option<&Token>, request: &Request, ground: &dyn Ground, presence: Option<&VerifiedPresence>) -> Decision {
    let judged = (|| -> Step<(Arrival, Authority, Option<&'static str>)> {
        resource(request, ground)?;
        let arrival = arrival(request, ground)?;
        let authority = who(token, request, ground, &arrival)?;
        let (authority, note) = row(request, ground, presence, &arrival, authority)?;
        Ok((arrival, authority, note))
    })();
    match judged {
        Err(reason) => Decision::Refuse(reason),
        Ok((arrival, authority, note)) => {
            // A Deny only narrows, so it never waits on governance.
            let deny = matches!(request.act(), Act::Answer { decision: AnswerDecision::Deny, .. });
            if HOLDS.contains(&request.action()) && !arrival.own && !deny {
                let stale = ground.governance_synced_at().is_none_or(|t| request.at() - t > HOLD_AFTER_MS);
                if stale {
                    return Decision::Hold(Reason::new(
                        "stale",
                        format!("this machine hasn't synced governance from another member in ten minutes, so it holds a {} for another machine", request.action().as_str()),
                    ));
                }
            }
            Decision::Allow(Box::new(Authorized {
                principal: request.principal().clone(),
                action: request.action(),
                resource: request.resource().clone(),
                act_hash: request.act_hash(),
                network: request.network().cloned(),
                authority,
                note,
            }))
        }
    }
}

// Step 1: the resource fits the action, and it's this machine's.
fn resource(request: &Request, ground: &dyn Ground) -> Step<()> {
    let own = ground.own_machine();
    let fits = matches!(
        (request.action(), request.resource()),
        (Action::Message, Resource::Machine(_) | Resource::Session(_))
            | (Action::Wake, Resource::Session(_))
            | (Action::Start, Resource::Run { .. })
            | (Action::Answer, Resource::Run { .. } | Resource::Session(_))
            | (Action::Claim | Action::Handoff | Action::Read | Action::RegisterActor | Action::UseGateway, Resource::Machine(_))
            | (Action::Admit | Action::Widen | Action::Publish, Resource::Network(_))
            | (Action::Revoke, Resource::Machine(_) | Resource::Network(_))
    );
    if !fits {
        return Err(Reason::new("resource", format!("a {} isn't done to that", request.action().as_str())));
    }
    match request.resource() {
        Resource::Machine(m) | Resource::Run { machine: m, .. } if m != own => return Err(Reason::new("resource", format!("{m} decides for itself; this is {own}"))),
        Resource::Network(n) if ground.role(n, own).is_none() => return Err(Reason::new("resource", format!("this machine isn't in {n}"))),
        _ => {}
    }
    match (request.act(), request.resource()) {
        (Act::Start { app, folder, .. }, Resource::Run { app: a, folder: f, .. }) if app != a || folder != f => {
            Err(Reason::new("resource", "the start names another run than its resource"))
        }
        (Act::Answer { ask, .. }, resource) => match ground.ask(ask) {
            None => Err(Reason::new("ask", format!("no question {ask} waits here"))),
            Some(a) if matches!(resource, Resource::Session(s) if s != &a.session) => Err(Reason::new("ask", format!("{ask} wasn't asked by that session"))),
            Some(_) => Ok(()),
        },
        _ => Ok(()),
    }
}

// Step 2: the door, and the network it came from.
fn arrival(request: &Request, ground: &dyn Ground) -> Step<Arrival> {
    let own = ground.own_machine();
    let machine = request.principal().machine();
    if let Some(n) = request.network() {
        if ground.role(n, own).is_none() {
            return Err(Reason::new("network", format!("this machine isn't in {n}")));
        }
    }
    match request.door() {
        Door::Local => match machine {
            Some(m) if m != own => Err(Reason::new("door", format!("{m} is another machine, and this came in at a local door"))),
            _ => Ok(Arrival { own: true, role: None }),
        },
        Door::Network => {
            let Some(n) = request.network() else {
                return Err(Reason::new("network", "an act from another machine names the network it came through"));
            };
            match machine {
                None => Ok(Arrival { own: false, role: None }),
                Some(m) if m == own => Err(Reason::new("door", "this machine's own came in from the network")),
                Some(m) => match ground.role(n, m) {
                    Some(role) => Ok(Arrival { own: false, role: Some(role) }),
                    None => Err(Reason::new("member", format!("{m} isn't a member of {n}"))),
                },
            }
        }
        Door::Gateway | Door::A2a => match request.principal() {
            Principal::Actor { machine, .. } if machine == own => Ok(Arrival { own: true, role: None }),
            _ => Err(Reason::new("door", format!("only this machine's actors come in through its {}", if request.door() == Door::Gateway { "gateway" } else { "A2A door" }))),
        },
    }
}

// Step 3: the principal is who it says, by its own proof.
fn who(token: Option<&Token>, request: &Request, ground: &dyn Ground, arrival: &Arrival) -> Step<Authority> {
    let membership = || Authority::Membership { network: request.network().cloned(), role: arrival.role };
    match request.principal() {
        Principal::Actor { slug, machine } => {
            let token = token.ok_or_else(|| Reason::new("token", "an actor proves itself with its token"))?;
            let b = token.bearer();
            if &b.slug != slug || &b.machine != machine {
                return Err(Reason::new("token", format!("that token isn't the actor {slug}'s on {machine}")));
            }
            token_holds(token, request, ground)
        }
        _ if token.is_some() => Err(Reason::new("token", format!("a {} doesn't come with a token", request.principal().kind()))),
        Principal::Person(signed) => {
            if signed.act() != &request.act_hash() {
                return Err(Reason::new("person", "the person's device signed another act"));
            }
            if ground.own_person().as_ref() != Some(signed.person()) {
                return Err(Reason::new("person", format!("{} isn't this machine's person", signed.person())));
            }
            if ground.device(signed.person(), signed.device()).is_none() {
                return Err(Reason::new("person", format!("{} isn't one of the person's devices, or it was revoked", signed.device())));
            }
            Ok(Authority::Person { device: signed.device().clone() })
        }
        Principal::Machine { .. } | Principal::Agent { .. } => Ok(membership()),
    }
}

// A token still stands (its root held and certified now, nothing revoked)
// and its Datalog allows the request.
fn token_holds(token: &Token, request: &Request, ground: &dyn Ground) -> Step<Authority> {
    if let Some(n) = token.network() {
        if request.network().is_some_and(|r| r != n) {
            return Err(Reason::new("token", format!("that token acts only in {n}")));
        }
    }
    let root = token.root();
    if !ground.token_roots(root.key_id()).contains(root) {
        return Err(Reason::new("root", "that token's root isn't held here any more"));
    }
    let certified = ground.signing_key(root.machine()).ok_or_else(|| Reason::new("root", format!("{} isn't a machine this one knows the key of", root.machine())))?;
    root.verify(&certified, request.at()).map_err(|e| Reason::new("root", e.message))?;
    let ids = token.revocation_ids();
    if ids.iter().any(|id| ground.revoked(id)) {
        return Err(Reason::new("revoked", "that token was revoked"));
    }
    token.authorize(request).map_err(|e| Reason::new("right", e))?;
    Ok(Authority::Token { revocation_id: ids.into_iter().next().unwrap_or_default() })
}

// A delegation of the person's, unended at the request's time, that names
// this machine's device and `from`'s (the device the work was started on)
// and passes `covers`.
fn delegation(ground: &dyn Ground, request: &Request, from: Option<&DeviceId>, covers: impl Fn(&StandingGrant) -> bool) -> Option<StandingGrant> {
    let person = ground.own_person()?;
    let here = ground.device_of(ground.own_machine())?;
    let from = from?;
    let held = |d: &DeviceId| ground.device(&person, d).is_some();
    if !held(&here) || !held(from) {
        return None;
    }
    ground.delegations().into_iter().find(|g| g.until >= request.at() && g.devices.has(&here) && g.devices.has(from) && covers(g))
}

// The device a principal's work comes from: the person's signing device,
// or the device of the machine an agent or actor is on.
fn origin(ground: &dyn Ground, principal: &Principal) -> Option<DeviceId> {
    match principal {
        Principal::Person(signed) => Some(signed.device().clone()),
        _ => principal.machine().and_then(|m| ground.device_of(m)),
    }
}

// Step 4: the request's cell in the rule table.
fn row(request: &Request, ground: &dyn Ground, presence: Option<&VerifiedPresence>, arrival: &Arrival, authority: Authority) -> Step<(Authority, Option<&'static str>)> {
    let principal = request.principal();
    let action = request.action().as_str();
    match rule(request.action(), principal) {
        No => Err(Reason::new("principal", format!("a {} doesn't {action}", principal.kind()))),
        Reach => Ok((authority, None)),
        Wake => {
            if ground.settings(request.network()).wake == WakeSetting::Network {
                return Ok((authority, None));
            }
            let Resource::Session(session) = request.resource() else {
                return Err(Reason::new("resource", "a wake is done to a session"));
            };
            let consent = ground.consent(session);
            let (allowed, whom) = if matches!(principal, Principal::Actor { .. }) { (consent.actors, "actors") } else { (consent.here, "agents") };
            if allowed {
                Ok((authority, None))
            } else {
                Err(Reason::new("consent", format!("the person hasn't let {whom} wake {session}, and this machine's wake setting is consent")))
            }
        }
        Start => {
            let setting = ground.settings(request.network()).start;
            if setting == StartSetting::Off {
                return Err(Reason::new("settings", "this machine starts no runs: its start setting is off"));
            }
            let Act::Start { mode, .. } = request.act() else {
                return Err(Reason::new("act", "a start's act is a start"));
            };
            let attestation = match principal {
                Principal::Agent { attestation, .. } => Some(*attestation),
                _ => None,
            };
            let delegated = || delegation(ground, request, origin(ground, principal).as_ref(), |g| g.starts).is_some();
            let no_grant =
                || Reason::new("grant", "a delegated run needs a delegation of the person's for starts that names this machine's device and the one it was started from");
            if matches!(principal, Principal::Person(_)) {
                if *mode == StartMode::Delegated && !delegated() {
                    return Err(no_grant());
                }
                return Ok((authority, None));
            }
            if attestation == Some(Attestation::Unchecked) {
                return Err(Reason::new("attestation", "an unchecked agent never starts runs"));
            }
            if setting == StartSetting::You {
                return Err(Reason::new("settings", "only the person starts runs here: this machine's start setting is you"));
            }
            if *mode == StartMode::Delegated {
                if setting != StartSetting::Delegate {
                    return Err(Reason::new("settings", "a delegated run needs this machine's start setting to be delegate"));
                }
                if !delegated() {
                    return Err(no_grant());
                }
            }
            Ok((authority, None))
        }
        Claim => match principal {
            Principal::Agent { attestation: Attestation::Unchecked, .. } if !arrival.own => Err(Reason::new("attestation", "an unchecked agent claims only on its own machine")),
            _ => Ok((authority, None)),
        },
        Read => {
            let Act::Read { scope, .. } = request.act() else {
                return Err(Reason::new("act", "a read's act is a read"));
            };
            if *scope == ReadScope::Map {
                return Ok((authority, None));
            }
            let attestation = match principal {
                Principal::Agent { attestation, .. } => Some(*attestation),
                _ => None,
            };
            if arrival.own {
                if attestation.is_some_and(|a| a < Attestation::ProcessChecked) {
                    return Err(Reason::new("attestation", format!("an unchecked agent doesn't read {}", scope.as_str())));
                }
                return Ok((authority, None));
            }
            if arrival.role.is_none_or(|r| r < Role::Admin) {
                return Err(Reason::new("role", format!("only an admin member reads {} from another machine", scope.as_str())));
            }
            if attestation == Some(Attestation::Unchecked) {
                return Err(Reason::new("attestation", format!("an unchecked agent doesn't read {}", scope.as_str())));
            }
            Ok((authority, None))
        }
        Presence => present(request, ground, presence),
        Answer => answer(request, ground, presence, arrival, authority),
    }
}

// The person, there for this act: a presence block for it from the device
// that signed it, from a device they still hold.
fn present(request: &Request, ground: &dyn Ground, presence: Option<&VerifiedPresence>) -> Step<(Authority, Option<&'static str>)> {
    let Principal::Person(signed) = request.principal() else {
        return Err(Reason::new("presence", "only the person is proven present"));
    };
    let action = request.action().as_str();
    let p = presence.ok_or_else(|| Reason::new("presence", format!("a {action} needs the person there: a presence block for it")))?;
    if p.act() != &request.act_hash() || p.person() != signed.person() || p.device() != signed.device() {
        return Err(Reason::new("presence", "that presence is for another act or another device"));
    }
    let device = ground.device(signed.person(), signed.device()).ok_or_else(|| Reason::new("presence", "that device was revoked"))?;
    let note = (device.kind == DeviceKind::Weaker).then_some("the person's device proves presence with a passphrase, polkit or a PIN, which is weaker than biometric");
    Ok((Authority::Presence { device: signed.device().clone(), kind: device.kind }, note))
}

// An answer: a Deny from whoever may relay one, an Allow only with the
// person there or under an allow scope or a delegation (the module's doc
// says which).
fn answer(request: &Request, ground: &dyn Ground, presence: Option<&VerifiedPresence>, arrival: &Arrival, authority: Authority) -> Step<(Authority, Option<&'static str>)> {
    let Act::Answer { ask, decision } = request.act() else {
        return Err(Reason::new("act", "an answer's act is an answer"));
    };
    // Step 1 found it waiting here.
    let asked = ground.ask(ask).ok_or_else(|| Reason::new("ask", format!("no question {ask} waits here")))?;
    let principal = request.principal();
    if *decision == AnswerDecision::Deny {
        return match principal {
            Principal::Machine { .. } if !arrival.own && arrival.role.is_none_or(|r| r < Role::Admin) => {
                Err(Reason::new("role", "only a window host relays its window's Deny from another machine"))
            }
            Principal::Person(signed) => Ok((Authority::Person { device: signed.device().clone() }, None)),
            _ => Ok((authority, None)),
        };
    }
    if asked.by < Attestation::ProcessChecked {
        return Err(Reason::new("attestation", format!("nothing allows a prompt from a run whose agent is {}", asked.by.as_str())));
    }
    match principal {
        Principal::Person(_) => present(request, ground, presence),
        Principal::Actor { .. } => {
            let Authority::Token { revocation_id } = authority else {
                return Err(Reason::new("token", "an actor proves itself with its token"));
            };
            let scope =
                ground.allow_scope(&revocation_id).ok_or_else(|| Reason::new("scope", "that actor's grant has no allow scope here: its plain right to answer is a Deny's"))?;
            if scope.until < request.at() {
                return Err(Reason::new("scope", "that actor's allow scope ended"));
            }
            if !scope.sessions.has(&asked.session) {
                return Err(Reason::new("scope", format!("that actor's allow scope doesn't cover {}", asked.session)));
            }
            if !scope.kinds.contains(&asked.kind) {
                return Err(Reason::new("scope", format!("that actor's allow scope doesn't cover {} prompts", asked.kind.as_str())));
            }
            Ok((Authority::AllowScope { revocation_id }, None))
        }
        Principal::Machine { id } => {
            if id != ground.own_machine() || !arrival.own {
                return Err(Reason::new("grant", "only this machine allows under the person's delegation, for its own runs"));
            }
            let from = asked.delegated_by.as_ref().ok_or_else(|| Reason::new("grant", "that run wasn't started under the person's delegation, so its prompts are the person's"))?;
            let covered = delegation(ground, request, ground.device_of(from).as_ref(), |g| g.answers.contains(&asked.kind)).is_some();
            if !covered {
                return Err(Reason::new("grant", format!("no delegation of the person's answers {} prompts for a run started from {from} on this machine", asked.kind.as_str())));
            }
            Ok((Authority::StandingGrant, None))
        }
        Principal::Agent { .. } => Err(Reason::new("principal", "an agent doesn't answer")),
    }
}
