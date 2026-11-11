//! Where the engine decides an act (docs/design/direction.md,
//! "Authorization"): a `Ground` over the network's facts (xenonet-net's
//! `Facts`: the network's chain, the person's chain, when governance was
//! synced) with what only the engine holds added, the asks waiting here;
//! and the requests the engine builds from an act and the authority it
//! came with. Every act from another machine goes through `decide` here,
//! and so do the person's acts through this machine's own API. What the
//! sender says of its own strength is never read: it isn't in a body any
//! more, and a request has no field for it.
//!
//! Who's who at the receiver:
//!
//! - the person: their device's signature over the act, checked against
//!   this machine's copy of their chain; for an act that takes a presence
//!   block, that block alone, which the device signs over the act too, so
//!   the person is asked once; for an Allow (and anything else that widens)
//!   the block is taken once, within a minute;
//! - an agent on another machine: vouched for by that machine, which
//!   checked its process before it sent (process-checked);
//! - an actor: its token. Here, the token it called with; from another
//!   machine, a token that machine minted for the act, which travels in
//!   the act's authority with its root's certificate, checked against
//!   that machine's signing key in the network's chain. Revocations are
//!   the issuing machine's: it checked the actor's own token before it
//!   minted one for the act, and only that machine could mint for it;
//! - another machine itself: a member of the network; for a Deny, a window
//!   host relaying its window's, with no signature of the person's.

use crate::runtime::Runtime;
use std::collections::HashMap;
use std::sync::Arc;
use xenonet_authz::{
    decide, Act, Action, ActorGrant, ActorSlug, AllowScope, AskId, Asked, Attestation, Consent, Decision, Device, DeviceId, Door, Ground, MachineId, Narrowing, NetworkId,
    PersonId, PersonSigned, Principal, PromptKind, Request, Resource, Role, RootCertificate, SessionId, Set, Settings, StandingGrant, Token, VerifiedPresence, ACTOR_PREFIX,
};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::VerifyingKey;
use xenonet_net::ground::{Facts, Governance};
use xenonet_net::identity::Identity;
use xenonet_os::sync::ReadWrite;

/// The network's facts, with the asks waiting here, the revoked tokens,
/// and the token roots of other machines that came with an act.
pub struct EngineGround {
    pub facts: Facts,
    /// Each ask waiting here: its session, how its caller was checked when
    /// it asked, its kind, and the machine it was started from under the
    /// person's delegation.
    pub asks: HashMap<String, Asked>,
    /// The engine's store, whose revoked table holds every token taken back.
    pub store: Option<xenonet_store::engine::Shared>,
    /// Another machine's root certificates an act carried, unverified: a
    /// token's parse checks each against that machine's signing key.
    pub roots: Vec<RootCertificate>,
}

impl Ground for EngineGround {
    fn own_machine(&self) -> &MachineId {
        self.facts.own_machine()
    }
    fn role(&self, network: &NetworkId, machine: &MachineId) -> Option<Role> {
        self.facts.role(network, machine)
    }
    fn settings(&self, network: Option<&NetworkId>) -> Settings {
        self.facts.settings(network)
    }
    fn own_person(&self) -> Option<PersonId> {
        self.facts.own_person()
    }
    fn device(&self, person: &PersonId, device: &DeviceId) -> Option<Device> {
        self.facts.device(person, device)
    }
    /// Revoked when the revoked table holds it, or when the store can't
    /// say.
    fn revoked(&self, revocation_id: &[u8]) -> bool {
        let id = xenonet_id::codec::hex(revocation_id);
        self.facts.revoked(revocation_id) || self.store.as_ref().is_some_and(|s| s.with(|s| s.revoked(&id)).map_or(true, |r| r.is_some()))
    }
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        let mut out = self.facts.token_roots(key_id);
        out.extend(self.roots.iter().filter(|c| c.key_id() == key_id && c.machine() != self.facts.own_machine()).cloned());
        out
    }
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        self.facts.signing_key(machine)
    }
    fn consent(&self, session: &SessionId) -> Consent {
        self.facts.consent(session)
    }
    fn device_of(&self, machine: &MachineId) -> Option<DeviceId> {
        self.facts.device_of(machine)
    }
    fn delegations(&self) -> Vec<StandingGrant> {
        self.facts.delegations()
    }
    /// The allow scope in the grant scopes row kept by this token's id. A
    /// token minted elsewhere has no row here, so it allows nothing.
    fn allow_scope(&self, token: &[u8]) -> Option<AllowScope> {
        let store = self.store.as_ref()?;
        let row = store.with(|s| s.scope(&xenonet_id::codec::hex(token))).ok().flatten()?;
        allow_scope_of(&serde_json::from_str(row.text("allows")?).ok()?)
    }
    fn ask(&self, ask: &AskId) -> Option<Asked> {
        self.asks.get(ask.as_str()).cloned()
    }
    fn governance_synced_at(&self) -> Option<i64> {
        self.facts.governance_synced_at()
    }
}

/// This machine's governance, once the network's link plugged it in.
pub fn governance(w: &Runtime) -> Option<Arc<dyn Governance>> {
    w.net.governance.reading().clone()
}

/// Plugs the governance in: what `network::plug` does, and the tests.
pub fn set_governance(w: &Runtime, g: Option<Arc<dyn Governance>>) {
    *w.net.governance.writing() = g;
}

/// The ground for one decision, with the asks waiting here and any
/// `extra` (an ask about to be made, for the person's delegation).
pub fn ground(w: &Runtime, extra: Option<(&str, crate::asks::Waiting)>) -> Option<EngineGround> {
    let facts = governance(w)?.facts();
    let mut waiting = crate::asks::waiting(w);
    if let Some((id, a)) = extra {
        waiting.push((id.to_string(), a));
    }
    let asks = waiting.into_iter().filter_map(|(id, a)| Some((id, asked(&facts.machine, a)?))).collect();
    Some(EngineGround { facts, asks, store: Some(w.store.clone()), roots: Vec::new() })
}

// An ask as a decision reads it: its caller as the process check found it
// when it asked (only a run XenoNet started asks, but the check may have
// seen nothing of the caller); one started under the delegation from this
// machine names it.
fn asked(own: &MachineId, a: crate::asks::Waiting) -> Option<Asked> {
    let delegated_by = match a.delegated_from {
        None => None,
        Some(None) => Some(own.clone()),
        Some(Some(host)) => Some(MachineId::new(&host).ok()?),
    };
    Some(Asked { session: SessionId::new(&a.session).ok()?, by: a.by, kind: a.kind, delegated_by })
}

/// An allow scope from its row's JSON: `{ sessions: "all" | [ids], kinds:
/// [prompt kinds], until }`. One that doesn't read is none.
pub fn allow_scope_of(v: &serde_json::Value) -> Option<AllowScope> {
    let sessions = match v.get("sessions")? {
        serde_json::Value::String(s) if s == "all" => Set::All,
        serde_json::Value::Array(a) => Set::Only(a.iter().map(|s| s.as_str().and_then(|s| SessionId::new(s).ok())).collect::<Option<_>>()?),
        _ => return None,
    };
    let kinds = v.get("kinds")?.as_array()?.iter().map(|k| k.as_str().and_then(PromptKind::parse)).collect::<Option<_>>()?;
    Some(AllowScope { sessions, kinds, until: v.get("until")?.as_i64()? })
}

/// Why something wasn't allowed, in words for a receipt or a refusal.
pub fn why(d: &Decision) -> String {
    match d {
        Decision::Allow(_) => String::new(),
        Decision::Hold(r) => format!("not now: {r}"),
        Decision::Refuse(r) => r.to_string(),
    }
}

/// The authority a body carries (`network/message/v2`'s and
/// `network/operator/v2`'s `authority`): the person's, or an actor's
/// token with its root's certificate.
#[derive(Debug, Clone, Default)]
pub struct Carried {
    /// The person, their device, and its signature over the act unless the
    /// presence block stands for it.
    pub person: Option<(String, String, Option<Vec<u8>>)>,
    pub presence: Option<PresenceBlock>,
    pub person_head: Option<String>,
    pub token: Option<String>,
    pub root: Option<RootCertificate>,
}

impl Carried {
    pub fn parse(v: &serde_json::Value) -> Carried {
        let p = v.get("person");
        let text = |v: Option<&serde_json::Value>, k: &str| v.and_then(|v| v.get(k)).and_then(serde_json::Value::as_str).map(str::to_string);
        let person = match (text(p, "person"), text(p, "device")) {
            (Some(a), Some(b)) => Some((a, b, text(p, "sig").and_then(|s| xenonet_id::codec::unb64(&s)))),
            _ => None,
        };
        Carried {
            person,
            presence: v.get("presence").and_then(PresenceBlock::parse),
            person_head: text(Some(v), "personHead"),
            token: text(Some(v), "token"),
            root: v.get("root").and_then(RootCertificate::parse),
        }
    }
}

/// The authority this machine's person gives an act made here: for an act
/// that takes a presence block, the block alone, which the device signs
/// over the act (one prompt: Touch ID once); for any other, the device's
/// signature over it. With the head of their chain. Asks the person, so
/// only for an act from XenoNet's window (`at_window`).
pub fn authority(identity: &Identity, at_window: bool, resource: &Resource, act: &Act, reason: &str, now: i64) -> Result<serde_json::Value, String> {
    let hash = xenonet_authz::act_hash(resource, act);
    let person = identity.person_id().ok_or("this machine has no identity of yours yet: make or join a network from XenoNet's window")?;
    let mut out = if let Some(what) = act.presence_what() {
        if !identity.can_act() {
            return Err("this machine has no identity of yours yet: make or join a network from XenoNet's window".into());
        }
        let block = identity.prove(at_window, what, &hash, reason, now).map_err(|e| e.message)?;
        serde_json::json!({ "person": { "person": person, "device": block.device }, "presence": block.to_json() })
    } else {
        let (device, sig) = identity.sign_act(at_window, &hash, reason).map_err(|e| e.message)?;
        serde_json::json!({ "person": { "person": person, "device": device, "sig": xenonet_id::codec::b64(&sig) } })
    };
    if let Some(head) = identity.head() {
        out["personHead"] = serde_json::Value::String(head);
    }
    Ok(out)
}

/// The person as an act's principal, from the authority it carried: their
/// device's signature over `resource` and `act`, checked against this
/// machine's copy of their chain. When the sender held a newer head of it
/// than this machine does, it's asked for the rest.
pub fn person(w: &Runtime, g: &EngineGround, from: Option<&str>, carried: &Carried, resource: &Resource, act: &Act) -> Result<Principal, String> {
    let Some((person, device, sig)) = &carried.person else { return Err("an act as you comes with your device's signature: do it from XenoNet's window".into()) };
    if let (Some(head), Some(host), Some(gov)) = (&carried.person_head, from, governance(w)) {
        if gov.identity().where_is(person, head).1.is_none() {
            gov.want_person(host, person);
        }
    }
    let person = PersonId::new(person).map_err(|e| e.message)?;
    let device = DeviceId::new(device).map_err(|e| e.message)?;
    let signed = match (sig, &carried.presence) {
        (Some(sig), _) => PersonSigned::verify(g, &person, &device, resource, act, sig),
        // The block stands for the signature, from the device it names.
        (None, Some(block)) if block.device == device.as_str() => PersonSigned::from_presence(g, &person, block, resource, act),
        _ => return Err("an act as you comes with your device's signature, or its presence for the act".into()),
    };
    signed.map(Principal::Person).map_err(|e| e.message)
}

/// How long a token minted for one act to another machine lasts: past the
/// week the outbox sends a message again while that machine is away.
pub const ACT_TOKEN_MS: i64 = 8 * 24 * 3600 * 1000;

/// A token for this machine's actor `slug` with `rights`, minted from this
/// start's token root: for `network` alone when it names one, until
/// `until` or the root's end, narrowed to `door` when it names one.
pub fn mint(w: &Runtime, slug: &str, rights: Vec<Action>, network: Option<NetworkId>, until: Option<i64>, door: Option<Door>) -> Result<Token, String> {
    let gov = governance(w).ok_or("this machine's governance isn't plugged in, so it mints no tokens")?;
    let facts = gov.facts();
    let root = facts.identity.token_root(&facts.machine, facts.now).map_err(|e| e.message)?;
    let end = root.certificate().until();
    let grant = ActorGrant { actor: ActorSlug::new(slug).map_err(|e| e.message)?, network, rights, until: until.map_or(end, |u| u.min(end)) };
    let token = root.mint_actor(&grant).map_err(|e| e.message)?;
    match door {
        Some(d) => token.attenuate(&Narrowing::Door(d)).map_err(|e| e.message),
        None => Ok(token),
    }
}

/// The authority an act of this machine's actor `slug` carries to another
/// machine: a token for `rights` minted for the act, in this network, for
/// `ACT_TOKEN_MS`, and its root's certificate, which that machine checks against
/// this one's signing key in the network's chain. The actor's own token
/// was decided here before.
pub fn actor_authority(w: &Runtime, slug: &str, rights: Vec<Action>) -> Result<serde_json::Value, String> {
    let gov = governance(w).ok_or("this machine's governance isn't plugged in, so it mints no tokens")?;
    let facts = gov.facts();
    let token = mint(w, slug, rights, facts.network(), Some(facts.now + ACT_TOKEN_MS), None)?;
    Ok(serde_json::json!({ "token": token.encode(ACTOR_PREFIX).map_err(|e| e.message)?, "root": token.root().to_json() }))
}

/// An actor of the machine `from` as an act's principal, by the token that
/// machine minted for the act: parsed against the root certificate the act
/// carried (kept in `g` for the decision), which must be that machine's.
pub fn actor(g: &mut EngineGround, from: &MachineId, carried: &Carried) -> Result<(Principal, Token), String> {
    let text = carried.token.as_deref().ok_or("an actor's act from another machine comes with a token its machine minted for it")?;
    if let Some(r) = &carried.root {
        g.roots.push(r.clone());
    }
    let token = Token::parse(text, ACTOR_PREFIX, g, g.facts.now).map_err(|e| e.message)?;
    let b = token.bearer();
    if &b.machine != from {
        return Err(format!("that token isn't for an actor of {from}"));
    }
    Ok((Principal::Actor { slug: b.slug.clone(), machine: b.machine.clone() }, token))
}

/// The person's presence for issuing this machine's actor `slug` a grant
/// with `rights` and the scopes whose hash is `scopes`, or widening one to
/// them: their device's signature over that act and a presence block for
/// it, made at XenoNet's window (it asks them: Touch ID on a Mac), then
/// decided here. Only from the window (`at_window`), where the person can
/// be asked. A machine whose person isn't set up yet sets them up first,
/// as the first act as them does.
pub fn present(w: &Runtime, at_window: bool, slug: &str, rights: Vec<Action>, scopes: [u8; 32], reason: &str) -> Result<(), String> {
    if !at_window {
        return Err("that gives an actor more, which asks you, so it's done from XenoNet's window".into());
    }
    let g = ground(w, None).ok_or("this machine's governance isn't plugged in, so it issues no grants")?;
    let identity = g.facts.identity.clone();
    if identity.person_id().is_none() {
        identity.found(true, g.facts.machine.as_str(), g.facts.now).map_err(|e| e.message)?;
    }
    let resource = Resource::Machine(g.facts.machine.clone());
    let act = Act::RegisterActor { slug: ActorSlug::new(slug).map_err(|e| e.message)?, rights, scopes };
    let auth = authority(&identity, true, &resource, &act, reason, g.facts.now)?;
    let carried = Carried::parse(&auth);
    let p = person(w, &g, None, &carried, &resource, &act)?;
    judge(&g, &local(&g, p, resource, act), carried.presence.as_ref())
}

/// Decides `request` here, taking the presence block it came with once
/// (the blocks taken are kept across restarts). A block that doesn't
/// count for this act is no presence at all, which decide refuses where
/// the act needs one.
pub fn decide_with(g: &EngineGround, request: &Request, presence: Option<&PresenceBlock>) -> Decision {
    decide(None, request, g, verified(g, request, presence).ok().flatten().as_ref())
}

/// `judge`, for an act whose principal brought a token (an actor's).
pub fn judge_by(g: &EngineGround, request: &Request, token: Option<&Token>, presence: Option<&PresenceBlock>) -> Result<(), String> {
    let v = match verified(g, request, presence) {
        Ok(v) => v,
        Err(e) => return Err(format!("that presence doesn't count: {e}")),
    };
    match decide(token, request, g, v.as_ref()) {
        Decision::Allow(_) => Ok(()),
        d => Err(why(&d)),
    }
}

fn verified(g: &EngineGround, request: &Request, presence: Option<&PresenceBlock>) -> Result<Option<VerifiedPresence>, String> {
    let Some(block) = presence else { return Ok(None) };
    g.facts
        .identity
        .with_seen(|seen| VerifiedPresence::check(seen, block, g, request).map_err(|e| xenonet_net::NetError::new(403, "presence", e.message)))
        .map(Some)
        .map_err(|e| e.message)
}

/// `decide_with`, as Ok or why not, with what was wrong with a presence
/// block when there was one and it didn't count.
pub fn judge(g: &EngineGround, request: &Request, presence: Option<&PresenceBlock>) -> Result<(), String> {
    judge_by(g, request, None, presence)
}

/// A request at this machine's own door, now.
pub fn local(g: &EngineGround, principal: Principal, resource: Resource, act: Act) -> Request {
    Request::new(principal, resource, act, Door::Local, g.facts.now, None)
}

/// A request from another machine of the network, now.
pub fn remote(g: &EngineGround, principal: Principal, resource: Resource, act: Act) -> Request {
    Request::new(principal, resource, act, Door::Network, g.facts.now, g.facts.network())
}

/// An agent on `machine`, vouched for by that machine: process-checked,
/// since a machine sends only what its routes checked.
pub fn agent(id: &str, machine: &MachineId) -> Result<Principal, String> {
    let id = xenonet_authz::AgentId::new(id).map_err(|e| e.message)?;
    Ok(Principal::Agent { id, machine: machine.clone(), attestation: Attestation::ProcessChecked })
}

pub fn machine(id: &str) -> Result<MachineId, String> {
    MachineId::new(id).map_err(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_authz::{AnswerDecision, Folder, StartMode};
    use xenonet_id::presence::FixedCheck;
    use xenonet_id::sign::{SeedSigner, Signer};
    use xenonet_id::vault::{DeviceKey, Vault};
    use xenonet_log::network::{self as chain, Member, View};

    struct Fake;

    impl Vault for Fake {
        fn machine_signer(&self, dir: &std::path::Path) -> xenonet_id::R<Box<dyn Signer>> {
            Ok(Box::new(SeedSigner::load_or_create(&dir.join("machine.key"))?))
        }
        fn device(&self, dir: &std::path::Path) -> xenonet_id::R<Option<DeviceKey>> {
            Ok(Some(DeviceKey { signer: Box::new(SeedSigner::load_or_create(&dir.join("device.key"))?), check: Some(Box::new(FixedCheck(true))), biometric: true }))
        }
        fn seal(&self, _to: &VerifyingKey, _secret: &[u8]) -> xenonet_id::R<Option<Vec<u8>>> {
            Ok(None)
        }
        fn open(&self, _dir: &std::path::Path, _sealed: &[u8], _reason: &str) -> xenonet_id::R<Vec<u8>> {
            Err(xenonet_id::IdError::internal("none"))
        }
        fn asking(&self, _reason: &str, f: &mut dyn FnMut()) {
            f();
        }
    }

    const NET: &str = "net_0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_792_512_345_678;

    fn member(id: &str, window: bool, settings: chain::Settings) -> Member {
        let keys = xenonet_id::keys::HostKey::generate().unwrap();
        Member {
            id: id.into(),
            keys: *keys.public(),
            window,
            device: None,
            machine: SeedSigner::from_seed(&[3; 32], xenonet_id::sign::KeyHome::File).unwrap().public(),
            settings,
            settings_by: None,
            since: 0,
        }
    }

    /// laptop's ground in a network studio founded, desk and vm1 in it too;
    /// laptop's person founded on its own device. ask_1 waits here: a
    /// command prompt from claude:run, which wasn't started under a
    /// delegation.
    fn ground(name: &str, settings: chain::Settings, synced: Option<i64>) -> (EngineGround, Arc<Identity>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("xenonet-runtime-ground-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let identity = Identity::open(&dir, Some(Arc::new(Fake))).unwrap();
        identity.found(true, "laptop", NOW).unwrap();
        let mut view = View { network: NET.into(), founder: "studio".into(), owner: identity.person_id(), gateway: "studio".into(), ..View::default() };
        for (id, window, s) in
            [("studio", true, chain::Settings::default()), ("desk", true, chain::Settings::default()), ("vm1", false, chain::Settings::default()), ("laptop", true, settings)]
        {
            view.members.insert(id.into(), member(id, window, s));
        }
        let facts = Facts { machine: MachineId::new("laptop").unwrap(), view: Some(view), identity: identity.clone(), synced_at: synced, now: NOW };
        let asks = HashMap::from([("ask_1".to_string(), waiting(None))]);
        (EngineGround { facts, asks, store: None, roots: Vec::new() }, identity, dir)
    }

    fn waiting(delegated_by: Option<&str>) -> Asked {
        Asked {
            session: SessionId::new("claude:run").unwrap(),
            by: Attestation::ProcessChecked,
            kind: PromptKind::Command,
            delegated_by: delegated_by.map(|m| MachineId::new(m).unwrap()),
        }
    }

    fn run() -> Resource {
        Resource::Run { machine: MachineId::new("laptop").unwrap(), app: xenonet_authz::AppId::new("claude").unwrap(), folder: Folder::new("~/atlas").unwrap() }
    }

    fn start(mode: StartMode) -> Act {
        Act::Start { app: xenonet_authz::AppId::new("claude").unwrap(), folder: Folder::new("~/atlas").unwrap(), mode }
    }

    fn session() -> Resource {
        Resource::Session(SessionId::new("claude:run").unwrap())
    }

    fn answer(decision: AnswerDecision) -> Act {
        Act::Answer { ask: AskId::new("ask_1").unwrap(), decision }
    }

    /// The person's authority for an act, as this machine makes it, and the
    /// principal a receiver takes from it, as `person` does.
    fn authorized(identity: &Identity, g: &EngineGround, resource: &Resource, act: &Act) -> (Principal, Carried) {
        let carried = Carried::parse(&authority(identity, true, resource, act, "", NOW).unwrap());
        let (person, device, sig) = carried.person.clone().unwrap();
        let person = PersonId::new(&person).unwrap();
        let signed = match &sig {
            Some(sig) => PersonSigned::verify(g, &person, &DeviceId::new(&device).unwrap(), resource, act, sig),
            None => PersonSigned::from_presence(g, &person, carried.presence.as_ref().unwrap(), resource, act),
        };
        (Principal::Person(signed.unwrap()), carried)
    }

    fn signed(identity: &Identity, g: &EngineGround, resource: &Resource, act: &Act) -> Principal {
        authorized(identity, g, resource, act).0
    }

    #[test]
    fn the_chain_says_who_may_start_here_and_the_person_needs_their_signature() {
        let (g, identity, dir) = ground("start", chain::Settings::default(), Some(NOW));
        let studio = MachineId::new("studio").unwrap();
        // Start: you, by default: an agent on another member is refused.
        let d = decide_with(&g, &remote(&g, agent("claude:a", &studio).unwrap(), run(), start(StartMode::Asks)), None);
        assert!(!d.is_allow() && why(&d).contains("start setting is you"), "{}", why(&d));
        // The person, signed by their device, from another member: a start
        // takes no presence, so the device signs it.
        let (p, carried) = authorized(&identity, &g, &run(), &start(StartMode::Asks));
        assert!(carried.person.as_ref().unwrap().2.is_some() && carried.presence.is_none());
        assert!(decide_with(&g, &remote(&g, p, run(), start(StartMode::Asks)), None).is_allow());
        // A signature over another act doesn't verify for this one.
        let other = Resource::Run { machine: MachineId::new("laptop").unwrap(), app: xenonet_authz::AppId::new("codex").unwrap(), folder: Folder::new("~/atlas").unwrap() };
        let auth = Carried::parse(&authority(&identity, true, &other, &start(StartMode::Asks), "", NOW).unwrap());
        let (person, device, sig) = auth.person.unwrap();
        assert!(PersonSigned::verify(&g, &PersonId::new(&person).unwrap(), &DeviceId::new(&device).unwrap(), &run(), &start(StartMode::Asks), &sig.unwrap()).is_err());
        // Not from the window: nothing is signed, nothing asks.
        assert!(authority(&identity, false, &run(), &start(StartMode::Asks), "", NOW).is_err());
        // A machine that isn't in the network.
        let d = decide_with(&g, &remote(&g, agent("claude:a", &MachineId::new("stranger").unwrap()).unwrap(), run(), start(StartMode::Asks)), None);
        assert!(!d.is_allow());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_delegation_in_the_persons_chain_lets_delegated_runs_start_and_answers_them() {
        let delegate = chain::Settings { start: chain::Start::Delegate, wake: chain::Wake::Consent };
        let (mut g, identity, dir) = ground("delegate", delegate, Some(NOW));
        let laptop = MachineId::new("laptop").unwrap();
        let studio = MachineId::new("studio").unwrap();
        let delegated = |g: &EngineGround, by: &MachineId| decide_with(g, &local_or_remote(g, by, start(StartMode::Delegated)), None);
        // No delegation yet: delegated work is refused, here and from studio.
        assert_eq!(delegated(&g, &laptop).reason().map(xenonet_authz::Reason::code), Some("grant"));
        // The person's standard delegation, signed by laptop's device with
        // its presence, names every device: laptop's own work runs.
        identity.delegate(true, xenonet_log::person::Delegation::standard("dlg_0123456789abcdef".into(), NOW + 1000), NOW).unwrap();
        g.facts.identity = identity.clone();
        assert!(delegated(&g, &laptop).is_allow());
        // studio has no device of the person's, so no delegation names it.
        assert_eq!(delegated(&g, &studio).reason().map(xenonet_authz::Reason::code), Some("grant"));
        // This machine allows a command prompt of a run started under it,
        // from here; not one of a run that wasn't.
        let me = Principal::Machine { id: laptop.clone() };
        assert!(!decide_with(&g, &local(&g, me.clone(), session(), answer(AnswerDecision::Allow)), None).is_allow(), "not started under it");
        g.asks.insert("ask_1".into(), waiting(Some("laptop")));
        assert!(decide_with(&g, &local(&g, me.clone(), session(), answer(AnswerDecision::Allow)), None).is_allow());
        // A web prompt isn't one the standard delegation answers.
        g.asks.insert("ask_1".into(), Asked { kind: PromptKind::Web, ..waiting(Some("laptop")) });
        assert!(!decide_with(&g, &local(&g, me.clone(), session(), answer(AnswerDecision::Allow)), None).is_allow());
        // Ended: nothing is answered under it, and nothing delegated starts.
        identity.undelegate(true, "dlg_0123456789abcdef", NOW).unwrap();
        g.asks.insert("ask_1".into(), waiting(Some("laptop")));
        assert!(!decide_with(&g, &local(&g, me, session(), answer(AnswerDecision::Allow)), None).is_allow());
        assert!(!delegated(&g, &laptop).is_allow());
        let _ = std::fs::remove_dir_all(dir);
    }

    fn local_or_remote(g: &EngineGround, by: &MachineId, act: Act) -> Request {
        let p = agent("claude:a", by).unwrap();
        if by == g.own_machine() {
            local(g, p, run(), act)
        } else {
            remote(g, p, run(), act)
        }
    }

    #[test]
    fn an_allow_asks_once_and_needs_presence_and_a_deny_asks_nothing() {
        let (g, identity, dir) = ground("answer", chain::Settings::default(), Some(NOW));
        let allow = answer(AnswerDecision::Allow);
        // The authority an Allow carries is a presence block alone: one
        // prompt, not a signature and a block.
        let (p, carried) = authorized(&identity, &g, &session(), &allow);
        assert!(carried.person.as_ref().unwrap().2.is_none());
        let block = carried.presence.unwrap();
        let req = remote(&g, p.clone(), session(), allow.clone());
        // No presence: refused.
        assert!(!decide_with(&g, &req, None).is_allow());
        assert!(decide_with(&g, &req, Some(&block)).is_allow());
        // Once: the same block again is refused.
        assert!(!decide_with(&g, &req, Some(&block)).is_allow(), "a block counts once");
        // A Deny's block lifted onto the Allow doesn't count.
        let deny = answer(AnswerDecision::Deny);
        let deny_block = Carried::parse(&authority(&identity, true, &session(), &deny, "", NOW).unwrap()).presence.unwrap();
        assert!(!decide_with(&g, &req, Some(&deny_block)).is_allow());
        // A Deny needs no presence: the person's device, this machine for
        // its window, a window host relaying its window's; not a member that
        // has no window.
        let p_deny = signed(&identity, &g, &session(), &deny);
        assert!(decide_with(&g, &remote(&g, p_deny, session(), deny.clone()), None).is_allow());
        let me = Principal::Machine { id: MachineId::new("laptop").unwrap() };
        assert!(decide_with(&g, &local(&g, me, session(), deny.clone()), None).is_allow());
        let studio = Principal::Machine { id: MachineId::new("studio").unwrap() };
        assert!(decide_with(&g, &remote(&g, studio.clone(), session(), deny.clone()), None).is_allow());
        let vm1 = Principal::Machine { id: MachineId::new("vm1").unwrap() };
        assert_eq!(decide_with(&g, &remote(&g, vm1, session(), deny.clone()), None).reason().map(xenonet_authz::Reason::code), Some("role"));
        // A window host relays a Deny, never an Allow.
        assert!(!decide_with(&g, &remote(&g, studio, session(), allow.clone()), None).is_allow());
        let _ = std::fs::remove_dir_all(dir);
        // Governance synced long ago: an Allow from another machine holds;
        // a Deny doesn't.
        let (g, identity, dir) = ground("stale", chain::Settings::default(), Some(NOW - 11 * 60_000));
        let (p, carried) = authorized(&identity, &g, &session(), &allow);
        let d = decide_with(&g, &remote(&g, p, session(), allow), carried.presence.as_ref());
        assert!(matches!(d, Decision::Hold(_)), "{}", why(&d));
        let studio = Principal::Machine { id: MachineId::new("studio").unwrap() };
        assert!(decide_with(&g, &remote(&g, studio, session(), deny), None).is_allow());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_ask_keeps_what_the_process_check_found_of_its_caller() {
        let delegate = chain::Settings { start: chain::Start::Delegate, wake: chain::Wake::Consent };
        let (mut g, identity, dir) = ground("asker", delegate, Some(NOW));
        identity.delegate(true, xenonet_log::person::Delegation::standard("dlg_0123456789abcdef".into(), NOW + 1000), NOW).unwrap();
        g.facts.identity = identity.clone();
        let laptop = MachineId::new("laptop").unwrap();
        let ask = |by| crate::asks::Waiting { session: "claude:run".into(), kind: PromptKind::Command, by, delegated_from: Some(None) };
        // The ask names its caller as the check found it.
        assert_eq!(asked(&laptop, ask(Attestation::ProcessChecked)).unwrap().by, Attestation::ProcessChecked);
        assert_eq!(asked(&laptop, ask(Attestation::Unchecked)).unwrap().by, Attestation::Unchecked);
        // A run started under the delegation is allowed by this machine
        // when the check saw its caller; one it couldn't see isn't, though
        // the session names a run XenoNet started.
        let me = Principal::Machine { id: laptop.clone() };
        g.asks.insert("ask_1".into(), asked(&laptop, ask(Attestation::ProcessChecked)).unwrap());
        assert!(decide_with(&g, &local(&g, me.clone(), session(), answer(AnswerDecision::Allow)), None).is_allow());
        g.asks.insert("ask_1".into(), asked(&laptop, ask(Attestation::Unchecked)).unwrap());
        assert!(!decide_with(&g, &local(&g, me, session(), answer(AnswerDecision::Allow)), None).is_allow());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_allow_scope_reads_from_its_row() {
        let scope = allow_scope_of(&serde_json::json!({ "sessions": ["claude:run"], "kinds": ["command", "edit"], "until": NOW })).unwrap();
        assert_eq!((scope.kinds, scope.until), (vec![PromptKind::Command, PromptKind::Edit], NOW));
        assert_eq!(allow_scope_of(&serde_json::json!({ "sessions": "all", "kinds": [], "until": 1 })).unwrap().sessions, Set::All);
        for bad in [
            serde_json::json!({ "sessions": "some", "kinds": [], "until": 1 }),
            serde_json::json!({ "sessions": "all", "kinds": ["fly"], "until": 1 }),
            serde_json::json!({ "sessions": ["two words"], "kinds": [], "until": 1 }),
            serde_json::json!({ "sessions": "all", "kinds": [] }),
        ] {
            assert!(allow_scope_of(&bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_revoked_device_signs_nothing_that_counts() {
        let (g, identity, dir) = ground("revoked", chain::Settings::default(), Some(NOW));
        let act = Act::Message;
        let resource = Resource::Machine(MachineId::new("laptop").unwrap());
        let carried = Carried::parse(&authority(&identity, true, &resource, &act, "", NOW).unwrap());
        // The person's own device revokes itself.
        let device = identity.device_id().unwrap();
        let mut c = identity.chain();
        let mut e = c.next(xenonet_log::person::Body { op: xenonet_log::person::Op::Revoke(xenonet_log::person::Revocation::Device(device.clone())), at: NOW });
        let block = identity.prove(true, xenonet_log::person::REVOKE, &e.digest(), "", NOW).unwrap();
        let key = SeedSigner::load_or_create(&dir.join("device.key")).unwrap();
        e.sign(&device, &key, Some(block)).unwrap();
        c.offer(e.clone()).unwrap();
        identity.offer(&identity.person_id().unwrap(), &[e.to_json()]).unwrap();
        let (person, device, sig) = carried.person.unwrap();
        assert!(PersonSigned::verify(&g, &PersonId::new(&person).unwrap(), &DeviceId::new(&device).unwrap(), &resource, &act, &sig.unwrap()).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
