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
//!   this machine's copy of their chain, and for an answer (and anything
//!   else that needs them there) a presence block for the act, taken once;
//! - an agent on another machine: vouched for by that machine, which
//!   checked its process before it sent (process-checked);
//! - an actor on another machine: the same until actor tokens travel with
//!   its acts (they're the actor tokens' commit);
//! - another machine itself: a member of the network.

use crate::runtime::Runtime;
use std::collections::HashMap;
use std::sync::Arc;
use xenonet_authz::{
    decide, Act, AskId, Attestation, Consent, Decision, Device, DeviceId, Door, GrantId, Ground, MachineId, NetworkId, PersonId, PersonSigned, Principal, Request, Resource, Role,
    RootCertificate, SessionId, Settings, StandingGrant, VerifiedPresence,
};
use xenonet_id::presence::PresenceBlock;
use xenonet_id::sign::VerifyingKey;
use xenonet_net::ground::{Facts, Governance};
use xenonet_net::identity::Identity;
use xenonet_os::sync::ReadWrite;

/// The network's facts, with the asks waiting here.
pub struct EngineGround {
    pub facts: Facts,
    /// Each ask waiting here, with how its run's agent was checked: a run
    /// XenoNet started, so process-checked.
    pub asks: HashMap<String, Attestation>,
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
    fn revoked(&self, revocation_id: &[u8]) -> bool {
        self.facts.revoked(revocation_id)
    }
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        self.facts.token_roots(key_id)
    }
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        self.facts.signing_key(machine)
    }
    fn consent(&self, session: &SessionId) -> Consent {
        self.facts.consent(session)
    }
    fn standing_grant(&self, machine: &MachineId) -> Option<StandingGrant> {
        self.facts.standing_grant(machine)
    }
    fn asker(&self, ask: &AskId) -> Option<Attestation> {
        self.asks.get(ask.as_str()).copied()
    }
    fn assistant_grant(&self, grant: &GrantId) -> Option<u64> {
        self.facts.assistant_grant(grant)
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
/// `extra` (an ask about to be made, for the standing grant).
pub fn ground(w: &Runtime, extra: Option<&str>) -> Option<EngineGround> {
    let facts = governance(w)?.facts();
    let mut asks: HashMap<String, Attestation> = crate::asks::waiting_ids(w).into_iter().map(|id| (id, Attestation::ProcessChecked)).collect();
    if let Some(id) = extra {
        asks.insert(id.to_string(), Attestation::ProcessChecked);
    }
    Some(EngineGround { facts, asks })
}

/// Why something wasn't allowed, in words for a receipt or a refusal.
pub fn why(d: &Decision) -> String {
    match d {
        Decision::Allow(_) => String::new(),
        Decision::Hold(r) => format!("not now: {r}"),
        Decision::Refuse(r) => r.to_string(),
    }
}

/// The person's authority as a body carries it (`network/message/v2`'s
/// and `network/operator/v2`'s `authority`).
#[derive(Debug, Clone, Default)]
pub struct Carried {
    pub person: Option<(String, String, Vec<u8>)>,
    pub presence: Option<PresenceBlock>,
    pub person_head: Option<String>,
}

impl Carried {
    pub fn parse(v: &serde_json::Value) -> Carried {
        let p = v.get("person");
        let text = |v: Option<&serde_json::Value>, k: &str| v.and_then(|v| v.get(k)).and_then(serde_json::Value::as_str).map(str::to_string);
        let person = match (text(p, "person"), text(p, "device"), text(p, "sig").and_then(|s| xenonet_id::codec::unb64(&s))) {
            (Some(a), Some(b), Some(c)) => Some((a, b, c)),
            _ => None,
        };
        Carried { person, presence: v.get("presence").and_then(PresenceBlock::parse), person_head: text(Some(v), "personHead") }
    }
}

/// The authority this machine's person gives an act made here: their
/// device's signature over it, with a presence block when it needs one,
/// and the head of their chain. Asks the person, so only for an act from
/// XenoNet's window (`at_window`).
pub fn authority(identity: &Identity, at_window: bool, resource: &Resource, act: &Act, reason: &str, now: i64) -> Result<serde_json::Value, String> {
    let hash = xenonet_authz::act_hash(resource, act);
    let person = identity.person_id().ok_or("this machine has no identity of yours yet: make or join a network from XenoNet's window")?;
    let (device, sig) = identity.sign_act(at_window, &hash, reason).map_err(|e| e.message)?;
    let mut out = serde_json::json!({ "person": { "person": person, "device": device, "sig": xenonet_id::codec::b64(&sig) } });
    if let Some(what) = act.presence_what() {
        let block = identity.prove(at_window, what, &hash, reason, now).map_err(|e| e.message)?;
        out["presence"] = block.to_json();
    }
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
    PersonSigned::verify(g, &person, &device, resource, act, sig).map(Principal::Person).map_err(|e| e.message)
}

/// Decides `request` here, taking the presence block it came with once
/// (the blocks taken are kept across restarts). A block that doesn't
/// count for this act is no presence at all, which decide refuses where
/// the act needs one.
pub fn decide_with(g: &EngineGround, request: &Request, presence: Option<&PresenceBlock>) -> Decision {
    decide(None, request, g, verified(g, request, presence).ok().flatten().as_ref())
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
    let v = match verified(g, request, presence) {
        Ok(v) => v,
        Err(e) => return Err(format!("that presence doesn't count: {e}")),
    };
    match decide(None, request, g, v.as_ref()) {
        Decision::Allow(_) => Ok(()),
        d => Err(why(&d)),
    }
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
    /// laptop's person founded on its own device.
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
        (EngineGround { facts, asks: HashMap::from([("ask_1".to_string(), Attestation::ProcessChecked)]) }, identity, dir)
    }

    fn run() -> Resource {
        Resource::Run { machine: MachineId::new("laptop").unwrap(), app: xenonet_authz::AppId::new("claude").unwrap(), folder: Folder::new("~/atlas").unwrap() }
    }

    fn start(mode: StartMode) -> Act {
        Act::Start { app: xenonet_authz::AppId::new("claude").unwrap(), folder: Folder::new("~/atlas").unwrap(), mode }
    }

    fn signed(identity: &Identity, g: &EngineGround, resource: &Resource, act: &Act) -> Principal {
        let auth = authority(identity, true, resource, act, "", NOW).unwrap();
        let carried = Carried::parse(&auth);
        let (person, device, sig) = carried.person.unwrap();
        Principal::Person(PersonSigned::verify(g, &PersonId::new(&person).unwrap(), &DeviceId::new(&device).unwrap(), resource, act, &sig).unwrap())
    }

    #[test]
    fn the_chain_says_who_may_start_here_and_the_person_needs_their_signature() {
        let (g, identity, dir) = ground("start", chain::Settings::default(), Some(NOW));
        let studio = MachineId::new("studio").unwrap();
        // Start: you, by default: an agent on another member is refused.
        let d = decide_with(&g, &remote(&g, agent("claude:a", &studio).unwrap(), run(), start(StartMode::Asks)), None);
        assert!(!d.is_allow() && why(&d).contains("start setting is you"), "{}", why(&d));
        // The person, signed by their device, from another member.
        let p = signed(&identity, &g, &run(), &start(StartMode::Asks));
        assert!(decide_with(&g, &remote(&g, p, run(), start(StartMode::Asks)), None).is_allow());
        // A signature over another act doesn't verify for this one.
        let other = Resource::Run { machine: MachineId::new("laptop").unwrap(), app: xenonet_authz::AppId::new("codex").unwrap(), folder: Folder::new("~/atlas").unwrap() };
        let auth = Carried::parse(&authority(&identity, true, &other, &start(StartMode::Asks), "", NOW).unwrap());
        let (person, device, sig) = auth.person.unwrap();
        assert!(PersonSigned::verify(&g, &PersonId::new(&person).unwrap(), &DeviceId::new(&device).unwrap(), &run(), &start(StartMode::Asks), &sig).is_err());
        // Not from the window: nothing is signed, nothing asks.
        assert!(authority(&identity, false, &run(), &start(StartMode::Asks), "", NOW).is_err());
        // A machine that isn't in the network.
        let d = decide_with(&g, &remote(&g, agent("claude:a", &MachineId::new("stranger").unwrap()).unwrap(), run(), start(StartMode::Asks)), None);
        assert!(!d.is_allow());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn network_and_delegate_let_agents_start_and_the_grant_answers_its_runs() {
        let delegate = chain::Settings { start: chain::Start::Delegate, wake: chain::Wake::Consent, grant: Some(chain::Grant::standard(NOW + 1000)) };
        let (g, _identity, dir) = ground("delegate", delegate, Some(NOW));
        let studio = MachineId::new("studio").unwrap();
        assert!(decide_with(&g, &remote(&g, agent("claude:a", &studio).unwrap(), run(), start(StartMode::Delegated)), None).is_allow());
        // The machine answers its own runs under the grant, for an ask
        // waiting here.
        let answer = |ask: &str| Act::Answer { ask: AskId::new(ask).unwrap(), decision: AnswerDecision::Allow };
        let me = Principal::Machine { id: MachineId::new("laptop").unwrap() };
        let session = Resource::Session(SessionId::new("claude:run").unwrap());
        assert!(decide_with(&g, &local(&g, me.clone(), session.clone(), answer("ask_1")), None).is_allow());
        assert!(!decide_with(&g, &local(&g, me, session, answer("ask_2")), None).is_allow(), "an ask that doesn't wait here");
        let _ = std::fs::remove_dir_all(dir);
        // Network, no grant: the machine answers nothing itself.
        let network = chain::Settings { start: chain::Start::Network, ..chain::Settings::default() };
        let (g, _identity, dir) = ground("network", network, Some(NOW));
        assert!(decide_with(&g, &remote(&g, agent("claude:a", &studio).unwrap(), run(), start(StartMode::Asks)), None).is_allow());
        assert!(!decide_with(&g, &remote(&g, agent("claude:a", &studio).unwrap(), run(), start(StartMode::Delegated)), None).is_allow());
        let me = Principal::Machine { id: MachineId::new("laptop").unwrap() };
        let d = decide_with(&g, &local(&g, me, Resource::Session(SessionId::new("claude:run").unwrap()), answer("ask_1")), None);
        assert!(!d.is_allow(), "no standing grant");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_answer_needs_presence_for_it_once_and_holds_on_stale_governance() {
        let (g, identity, dir) = ground("answer", chain::Settings::default(), Some(NOW));
        let session = Resource::Session(SessionId::new("claude:run").unwrap());
        let act = Act::Answer { ask: AskId::new("ask_1").unwrap(), decision: AnswerDecision::Allow };
        let p = signed(&identity, &g, &session, &act);
        let req = remote(&g, p.clone(), session.clone(), act.clone());
        // No presence: refused.
        assert!(!decide_with(&g, &req, None).is_allow());
        let carried = Carried::parse(&authority(&identity, true, &session, &act, "", NOW).unwrap());
        let block = carried.presence.unwrap();
        assert!(decide_with(&g, &req, Some(&block)).is_allow());
        // Once: the same block again is refused.
        assert!(!decide_with(&g, &req, Some(&block)).is_allow(), "a block counts once");
        // A block lifted onto another answer (deny) doesn't count.
        let deny = Act::Answer { ask: AskId::new("ask_1").unwrap(), decision: AnswerDecision::Deny };
        let fresh = Carried::parse(&authority(&identity, true, &session, &act, "", NOW).unwrap()).presence.unwrap();
        let p_deny = signed(&identity, &g, &session, &deny);
        assert!(!decide_with(&g, &remote(&g, p_deny, session.clone(), deny), Some(&fresh)).is_allow());
        let _ = std::fs::remove_dir_all(dir);
        // Governance synced long ago: an answer from another machine holds.
        let (g, identity, dir) = ground("stale", chain::Settings::default(), Some(NOW - 11 * 60_000));
        let p = signed(&identity, &g, &session, &act);
        let block = Carried::parse(&authority(&identity, true, &session, &act, "", NOW).unwrap()).presence.unwrap();
        let d = decide_with(&g, &remote(&g, p, session, act), Some(&block));
        assert!(matches!(d, Decision::Hold(_)), "{}", why(&d));
        let _ = std::fs::remove_dir_all(dir);
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
        assert!(PersonSigned::verify(&g, &PersonId::new(&person).unwrap(), &DeviceId::new(&device).unwrap(), &resource, &act, &sig).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
