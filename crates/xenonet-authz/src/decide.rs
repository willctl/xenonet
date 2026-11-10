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
//!    unrevoked device of this machine's person; an actor or an assistant
//!    holds a token for exactly them, whose root is still certified, with
//!    no id revoked, whose Datalog grants the action's right (and the
//!    gateway's, at the gateway or A2A) and whose own checks pass. Nobody
//!    else brings a token.
//! 4. **The rule table** (`TABLE`), one row per action and one column per
//!    kind of principal:
//!
//! ```text
//!                  machine   agent     actor     person    assistant
//!   message        reach     reach     reach     reach     -
//!   wake           wake      wake      wake      reach     -
//!   start          -         start     start     start     -
//!   answer         grant     -         -         presence  -
//!   claim          reach     claim     reach     reach     -
//!   handoff        reach     reach     reach     reach     -
//!   read           read      read      read      reach     read
//!   admit          -         -         -         presence  -
//!   widen          -         -         -         presence  -
//!   publish        -         -         -         presence  -
//!   register-actor -         -         reach     reach     -
//!   use-gateway    -         -         reach     -         -
//!   revoke         -         -         -         presence  -
//!
//!   -         refused
//!   reach     steps 1 to 3 are enough (an actor's token has the right)
//!   wake      the machine's wake setting is network, or the session's
//!             consent allows it: actors need consent for actors
//!   start     start off refuses everyone; you lets only the person;
//!             network and delegate let agents and actors too, but never
//!             an unchecked agent; a delegated run needs start delegate
//!             (the person needs none) and the person's standing grant for
//!             starts, with the agent's attestation at least the grant's
//!   claim     reach, but an unchecked agent claims only on its own machine
//!   read      the map: reach; transcripts and search: here, an agent
//!             process-checked at least; from another machine, an admin
//!             member, never an unchecked agent
//!   presence  the person, with a presence block for this act from the
//!             device that signed it; a weaker device is allowed and the
//!             decision says so
//!   grant     this machine itself, under the person's standing grant for
//!             answers, for a question whose asker is process-checked at
//!             least and at least what the grant requires
//! ```
//!
//! 5. **Hold**: start, answer, admit, widen and publish for another machine
//!    (a principal not of this machine, or the person over the network)
//!    hold instead of allowing when governance was last synced more than
//!    ten minutes ago, or never. This machine acting on its own is exempt,
//!    so a laptop on a plane still answers its own prompts.

use crate::proof::VerifiedPresence;
use crate::token::Bearer;
use crate::vocab::{Act, Action, Attestation, DeviceId, NetworkId, Principal, ReadScope, Request, Resource, StartMode};
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
    /// The person's standing grant for this machine.
    StandingGrant,
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
    Grant,
}

use Rule::{Claim, Grant, No, Presence, Reach, Read, Start, Wake};

/// The rule table (the module's doc draws it). Columns: machine, agent,
/// actor, person, assistant.
#[rustfmt::skip]
const TABLE: [(Action, [Rule; 5]); 13] = [
    //                       machine   agent     actor     person    assistant
    (Action::Message,       [Reach,    Reach,    Reach,    Reach,    No]),
    (Action::Wake,          [Wake,     Wake,     Wake,     Reach,    No]),
    (Action::Start,         [No,       Start,    Start,    Start,    No]),
    (Action::Answer,        [Grant,    No,       No,       Presence, No]),
    (Action::Claim,         [Reach,    Claim,    Reach,    Reach,    No]),
    (Action::Handoff,       [Reach,    Reach,    Reach,    Reach,    No]),
    (Action::Read,          [Read,     Read,     Read,     Reach,    Read]),
    (Action::Admit,         [No,       No,       No,       Presence, No]),
    (Action::Widen,         [No,       No,       No,       Presence, No]),
    (Action::Publish,       [No,       No,       No,       Presence, No]),
    (Action::RegisterActor, [No,       No,       Reach,    Reach,    No]),
    (Action::UseGateway,    [No,       No,       Reach,    No,       No]),
    (Action::Revoke,        [No,       No,       No,       Presence, No]),
];

/// The actions that hold for another machine on stale governance.
const HOLDS: [Action; 5] = [Action::Start, Action::Answer, Action::Admit, Action::Widen, Action::Publish];

fn rule(action: Action, principal: &Principal) -> Rule {
    let column = match principal {
        Principal::Machine { .. } => 0,
        Principal::Agent { .. } => 1,
        Principal::Actor { .. } => 2,
        Principal::Person(_) => 3,
        Principal::Assistant { .. } => 4,
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
            if HOLDS.contains(&request.action()) && !arrival.own {
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
        (Act::Answer { ask, .. }, _) if ground.asker(ask).is_none() => Err(Reason::new("ask", format!("no question {ask} waits here"))),
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
            match token.bearer() {
                Bearer::Actor { slug: s, machine: m } if s == slug && m == machine => {}
                _ => return Err(Reason::new("token", format!("that token isn't the actor {slug}'s on {machine}"))),
            }
            token_holds(token, request, ground)
        }
        Principal::Assistant { grant, machine } => {
            let token = token.ok_or_else(|| Reason::new("token", "an assistant proves itself with its grant's token"))?;
            match token.bearer() {
                Bearer::Assistant { grant: g, machine: m, revision } if g == grant && m == machine => {
                    if ground.assistant_grant(grant) != Some(*revision) {
                        return Err(Reason::new("revoked", format!("the grant {grant} was changed or taken back")));
                    }
                }
                _ => return Err(Reason::new("token", format!("that token isn't the grant {grant}'s on {machine}"))),
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

fn grant_now(ground: &dyn Ground, request: &Request) -> Option<StandingGrant> {
    ground.standing_grant(ground.own_machine()).filter(|g| g.until >= request.at())
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
            if matches!(principal, Principal::Person(_)) {
                if *mode == StartMode::Delegated && !grant_now(ground, request).is_some_and(|g| g.starts) {
                    return Err(Reason::new("grant", "a delegated run needs the person's standing grant for starts on this machine"));
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
                let Some(grant) = grant_now(ground, request).filter(|g| g.starts) else {
                    return Err(Reason::new("grant", "a delegated run needs the person's standing grant for starts on this machine"));
                };
                if attestation.is_some_and(|a| a < grant.requires) {
                    return Err(Reason::new("attestation", format!("the person's standing grant covers agents {} at least", grant.requires.as_str())));
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
        Presence => {
            let Principal::Person(signed) = principal else {
                return Err(Reason::new("presence", "only the person is proven present"));
            };
            let p = presence.ok_or_else(|| Reason::new("presence", format!("a {action} needs the person there: a presence block for it")))?;
            if p.act() != &request.act_hash() || p.person() != signed.person() || p.device() != signed.device() {
                return Err(Reason::new("presence", "that presence is for another act or another device"));
            }
            let device = ground.device(signed.person(), signed.device()).ok_or_else(|| Reason::new("presence", "that device was revoked"))?;
            let note = (device.kind == DeviceKind::Weaker).then_some("the person's device proves presence with a passphrase or polkit, which is weaker than biometric");
            Ok((Authority::Presence { device: signed.device().clone(), kind: device.kind }, note))
        }
        Grant => {
            let own = ground.own_machine();
            if !matches!(principal, Principal::Machine { id } if id == own) || !arrival.own {
                return Err(Reason::new("grant", "only this machine answers under the person's standing grant, for its own runs"));
            }
            let Act::Answer { ask, .. } = request.act() else {
                return Err(Reason::new("act", "an answer's act is an answer"));
            };
            let Some(grant) = grant_now(ground, request).filter(|g| g.answers) else {
                return Err(Reason::new("grant", "no standing grant of the person's answers prompts on this machine"));
            };
            let asker = ground.asker(ask).unwrap_or(Attestation::Unchecked);
            if asker < Attestation::ProcessChecked.max(grant.requires) {
                return Err(Reason::new("attestation", format!("the standing grant doesn't answer for a run whose agent is {}", asker.as_str())));
            }
            Ok((Authority::StandingGrant, None))
        }
    }
}
