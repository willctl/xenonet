//! A made-up world for the tests: three machines in one network, one
//! person with a biometric device (on m1, this machine) and a weaker one
//! (on m2), token roots for two of the machines, a question waiting, a
//! space it holds that gives everyone append, and a ground that answers
//! from plain fields.

use std::collections::{HashMap, HashSet};
use xenonet_authz::*;
use xenonet_id::presence::{prove, FixedCheck, PresenceBlock, PresenceCheck};
use xenonet_id::sign::{KeyHome, SeedSigner, Signer, VerifyingKey};

/// Now, in milliseconds since 1970: some day in 2027.
pub const NOW: i64 = 1_800_000_000_000;
pub const HOUR: i64 = 3_600_000;
pub const YEAR: i64 = 365 * 24 * HOUR;

pub fn machine(s: &str) -> MachineId {
    MachineId::new(s).unwrap()
}

pub fn network() -> NetworkId {
    NetworkId::new("n1").unwrap()
}

/// The project the tests claim in, which every actor grant here names.
pub fn project() -> ProjectId {
    ProjectId::new("prj_atlas").unwrap()
}

pub fn person() -> PersonId {
    PersonId::new("prs_owner").unwrap()
}

pub fn signer(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
}

/// This machine's signing key, another member's, and a stranger's.
pub fn m1_key() -> SeedSigner {
    signer(1)
}
pub fn m2_key() -> SeedSigner {
    signer(2)
}
pub fn m3_key() -> SeedSigner {
    signer(3)
}

/// The person's devices.
pub fn bio_device() -> SeedSigner {
    signer(21)
}
pub fn weak_device() -> SeedSigner {
    signer(22)
}

pub fn device_id(s: &dyn Signer) -> DeviceId {
    DeviceId::new(&s.public().id()).unwrap()
}

pub struct World {
    pub own: MachineId,
    pub roles: HashMap<(NetworkId, MachineId), Role>,
    pub settings: Settings,
    pub person: Option<PersonId>,
    pub devices: HashMap<(PersonId, DeviceId), Device>,
    pub revoked: HashSet<Vec<u8>>,
    pub roots: Vec<RootCertificate>,
    pub keys: HashMap<MachineId, VerifyingKey>,
    pub consent: Consent,
    pub delegations: Vec<StandingGrant>,
    pub machine_devices: HashMap<MachineId, DeviceId>,
    pub scopes: HashMap<Vec<u8>, AllowScope>,
    /// The projects every actor grant here may claim in.
    pub projects: Option<Vec<ProjectId>>,
    pub asks: HashMap<AskId, Asked>,
    pub synced: Option<i64>,
    pub outbound: HashMap<ActorSlug, OutboundScope>,
    /// The spaces this machine holds, and the level each gives every
    /// principal.
    pub spaces: HashMap<SpaceId, Option<SpaceLevel>>,
}

impl Ground for World {
    fn own_machine(&self) -> &MachineId {
        &self.own
    }
    fn role(&self, network: &NetworkId, machine: &MachineId) -> Option<Role> {
        self.roles.get(&(network.clone(), machine.clone())).copied()
    }
    fn settings(&self, _network: Option<&NetworkId>) -> Settings {
        self.settings
    }
    fn own_person(&self) -> Option<PersonId> {
        self.person.clone()
    }
    fn device(&self, person: &PersonId, device: &DeviceId) -> Option<Device> {
        self.devices.get(&(person.clone(), device.clone())).cloned()
    }
    fn revoked(&self, revocation_id: &[u8]) -> bool {
        self.revoked.contains(revocation_id)
    }
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        self.roots.iter().filter(|r| r.key_id() == key_id).cloned().collect()
    }
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        self.keys.get(machine).cloned()
    }
    fn consent(&self, _session: &SessionId) -> Consent {
        self.consent
    }
    fn device_of(&self, machine: &MachineId) -> Option<DeviceId> {
        self.machine_devices.get(machine).cloned()
    }
    fn delegations(&self) -> Vec<StandingGrant> {
        self.delegations.clone()
    }
    fn allow_scope(&self, token: &[u8]) -> Option<AllowScope> {
        self.scopes.get(token).cloned()
    }
    fn grant_projects(&self, _token: &[u8]) -> Option<Vec<ProjectId>> {
        self.projects.clone()
    }
    fn ask(&self, ask: &AskId) -> Option<Asked> {
        self.asks.get(ask).cloned()
    }
    fn governance_synced_at(&self) -> Option<i64> {
        self.synced
    }
    fn outbound(&self, actor: &ActorSlug) -> Option<OutboundScope> {
        self.outbound.get(actor).cloned()
    }
    fn holds_space(&self, space: &SpaceId) -> bool {
        self.spaces.contains_key(space)
    }
    fn space_level(&self, space: &SpaceId, _principal: &Principal) -> Option<SpaceLevel> {
        self.spaces.get(space).copied().flatten()
    }
}

/// The space the world holds.
pub fn space() -> SpaceId {
    SpaceId::new("spc_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap()
}

/// The question q1: session s1's run asks to run a command, a run m2
/// started under the person's delegation.
pub fn asked(by: Attestation) -> Asked {
    Asked { session: SessionId::new("s1").unwrap(), by, kind: PromptKind::Command, delegated_by: Some(machine("m2")) }
}

/// A delegation of every device, for starts and command prompts, for an
/// hour.
pub fn delegation() -> StandingGrant {
    StandingGrant { devices: Set::All, starts: true, answers: vec![PromptKind::Command], until: NOW + HOUR }
}

/// The world and the token roots of m1 (this machine) and m2.
pub struct Fixture {
    pub world: World,
    pub m1_root: TokenRoot,
    pub m2_root: TokenRoot,
}

pub fn fixture() -> Fixture {
    let m1_root = TokenRoot::new(machine("m1"), &m1_key(), NOW - HOUR, NOW + YEAR).unwrap();
    let m2_root = TokenRoot::new(machine("m2"), &m2_key(), NOW - HOUR, NOW + YEAR).unwrap();
    let n = network();
    let world = World {
        own: machine("m1"),
        roles: HashMap::from([((n.clone(), machine("m1")), Role::Founder), ((n.clone(), machine("m2")), Role::Member)]),
        settings: Settings::default(),
        person: Some(person()),
        devices: HashMap::from([
            ((person(), device_id(&bio_device())), Device { key: bio_device().public(), kind: DeviceKind::Biometric }),
            ((person(), device_id(&weak_device())), Device { key: weak_device().public(), kind: DeviceKind::Weaker }),
        ]),
        revoked: HashSet::new(),
        roots: vec![m1_root.certificate().clone(), m2_root.certificate().clone()],
        keys: HashMap::from([(machine("m1"), m1_key().public()), (machine("m2"), m2_key().public()), (machine("m3"), m3_key().public())]),
        consent: Consent::default(),
        delegations: Vec::new(),
        machine_devices: HashMap::from([(machine("m1"), device_id(&bio_device())), (machine("m2"), device_id(&weak_device()))]),
        scopes: HashMap::new(),
        projects: Some(vec![project()]),
        asks: HashMap::from([(AskId::new("q1").unwrap(), asked(Attestation::HarnessSigned))]),
        synced: Some(NOW - 60_000),
        outbound: HashMap::new(),
        spaces: HashMap::from([(space(), Some(SpaceLevel::Append))]),
    };
    Fixture { world, m1_root, m2_root }
}

pub fn actor_grant(slug: &str, rights: &[Action]) -> ActorGrant {
    ActorGrant { actor: ActorSlug::new(slug).unwrap(), network: None, rights: rights.to_vec(), until: NOW + HOUR }
}

/// The person at `device`, signing `act` done to `resource`.
pub fn as_person(world: &World, device: &dyn Signer, resource: &Resource, act: &Act) -> Principal {
    let sig = PersonSigned::sign(device, resource, act).unwrap();
    Principal::Person(PersonSigned::verify(world, &person(), &device_id(device), resource, act, &sig).unwrap())
}

/// A presence from `device` for `request`'s act, as the device makes it.
pub fn presence_block(device: &dyn Signer, request: &Request) -> PresenceBlock {
    prove(device, Some(&FixedCheck(true)), request.act().presence_what().unwrap(), &request.act_hash(), "test", request.at()).unwrap()
}

/// The same, checked by the receiver.
pub fn presence(world: &World, device: &dyn Signer, request: &Request) -> VerifiedPresence {
    VerifiedPresence::check(&mut PresenceCheck::new(), &presence_block(device, request), world, request).unwrap()
}
