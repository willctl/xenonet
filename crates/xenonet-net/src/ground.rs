//! The facts this machine decides on (`xenonet_authz::Ground`), from its
//! own copies: the network's chain (members, admins, the founder, each
//! member's settings, device and machine key), the person's chain (devices,
//! delegations), this machine's token root certificates, and when
//! governance was last synced. A `Facts` is a snapshot, taken for one
//! decision. What only the engine holds (the asks waiting, consent rows,
//! revocations, actors' allow scopes, another machine's token roots as an
//! act carried them) is the runtime's to add over it (xenonet-runtime's
//! ground).

use crate::identity::Identity;
use std::sync::Arc;
use xenonet_authz::{
    AllowScope, Asked, Consent, DelegationId, Device, DeviceId, DeviceKind, Ground, MachineId, NetworkId, PersonId, PromptKind, Role, RootCertificate, SessionId, Set, Settings,
    StandingGrant, StartSetting, WakeSetting,
};
use xenonet_id::sign::VerifyingKey;
use xenonet_log::network::{self, Start, View, Wake};
use xenonet_log::person::{self, DeviceKind as Kind};

/// What a machine that isn't in a network calls itself in a request.
pub const LOCAL: &str = "local";

/// The facts, as of now.
#[derive(Clone)]
pub struct Facts {
    pub machine: MachineId,
    /// The network's chain as it stands, when this machine is in one.
    pub view: Option<View>,
    pub identity: Arc<Identity>,
    /// When governance was last synced from another member.
    pub synced_at: Option<i64>,
    /// Now, by the network's clock: what a request is timed by.
    pub now: i64,
}

impl Facts {
    /// The network's id, when there's one.
    pub fn network(&self) -> Option<NetworkId> {
        self.view.as_ref().and_then(|v| NetworkId::new(&v.network).ok())
    }

    fn own(&self) -> Option<&network::Member> {
        self.view.as_ref()?.member(self.machine.as_str())
    }
}

pub fn settings_of(s: &network::Settings) -> Settings {
    Settings {
        start: match s.start {
            Start::Off => StartSetting::Off,
            Start::You => StartSetting::You,
            Start::Network => StartSetting::Network,
            Start::Delegate => StartSetting::Delegate,
        },
        wake: match s.wake {
            Wake::Consent => WakeSetting::Consent,
            Wake::Network => WakeSetting::Network,
        },
    }
}

impl Ground for Facts {
    fn own_machine(&self) -> &MachineId {
        &self.machine
    }

    fn role(&self, network: &NetworkId, machine: &MachineId) -> Option<Role> {
        let v = self.view.as_ref().filter(|v| v.network == network.as_str())?;
        let m = v.member(machine.as_str())?;
        Some(if m.id == v.founder {
            Role::Founder
        } else if v.is_window(&m.id) {
            Role::Admin
        } else {
            Role::Member
        })
    }

    fn settings(&self, network: Option<&NetworkId>) -> Settings {
        let _ = network;
        self.own().map(|m| settings_of(&m.settings)).unwrap_or_default()
    }

    /// The person here; on a machine with none of its own (one that joined
    /// as a plain member), the network's owner, whose chain it holds.
    fn own_person(&self) -> Option<PersonId> {
        let own = self.identity.person_id().or_else(|| self.view.as_ref().and_then(|v| v.owner.clone()));
        own.and_then(|p| PersonId::new(&p).ok())
    }

    fn device(&self, person: &PersonId, device: &DeviceId) -> Option<Device> {
        let d = self.identity.active_device(person.as_str(), device.as_str())?;
        Some(Device { key: d.key, kind: if d.kind == Kind::Biometric { DeviceKind::Biometric } else { DeviceKind::Weaker } })
    }

    /// Revocations are the engine's (its revoked table): the runtime's
    /// ground answers them.
    fn revoked(&self, _revocation_id: &[u8]) -> bool {
        false
    }

    /// This machine's own certificates; another machine's travel with the
    /// act that uses them (the runtime's ground adds those).
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        self.identity.token_roots(key_id)
    }

    /// Another machine's key as the network's chain binds it; this
    /// machine's own, in a network or not, from its identity.
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        if machine == &self.machine {
            return self.identity.machine_key().ok().map(|k| k.public());
        }
        Some(self.view.as_ref()?.member(machine.as_str())?.machine.clone())
    }

    fn consent(&self, _session: &SessionId) -> Consent {
        Consent::default()
    }

    /// This machine's from its identity; another's as the network's chain
    /// names it.
    fn device_of(&self, machine: &MachineId) -> Option<DeviceId> {
        let id = if machine == &self.machine { self.identity.device_id() } else { self.view.as_ref()?.member(machine.as_str())?.device.clone() };
        id.and_then(|d| DeviceId::new(&d).ok())
    }

    /// The delegations in the person's chain this machine holds: its own
    /// person's, or the network's owner's.
    fn delegations(&self) -> Vec<StandingGrant> {
        let Some(person) = self.own_person() else { return Vec::new() };
        let Some(chain) = self.identity.chain_of(person.as_str()) else { return Vec::new() };
        chain.person().map(|p| p.delegations.values().filter_map(|(d, _)| grant_of(d)).collect()).unwrap_or_default()
    }

    /// Actors' allow scopes are the engine's rows: the runtime's ground
    /// answers them.
    fn allow_scope(&self, _token: &[u8]) -> Option<AllowScope> {
        None
    }

    /// The prompts waiting are the engine's too.
    fn ask(&self, _ask: &xenonet_authz::AskId) -> Option<Asked> {
        None
    }

    fn governance_synced_at(&self) -> Option<i64> {
        self.synced_at
    }
}

/// A delegation as a decision reads it, with its id, which an intent names.
/// One that names a device id that doesn't read is taken as naming the
/// ones that do; one whose id doesn't read is none (the chain never takes
/// one).
pub fn grant_of(d: &person::Delegation) -> Option<StandingGrant> {
    let devices = match &d.devices {
        person::DeviceSet::All => Set::All,
        person::DeviceSet::Only(list) => Set::Only(list.iter().filter_map(|id| DeviceId::new(id).ok()).collect()),
    };
    let answers = if d.covers("answer") { d.prompts.iter().filter_map(|p| PromptKind::parse(p)).collect() } else { Vec::new() };
    Some(StandingGrant { id: DelegationId::new(&d.id).ok()?, devices, starts: d.covers("start"), answers, until: d.until })
}

/// What the engine reads this machine's governance through: the facts for
/// a decision, the person here, and asking another machine for a person's
/// entries this one lacks.
pub trait Governance: Send + Sync {
    fn facts(&self) -> Facts;
    fn identity(&self) -> Arc<Identity>;
    /// `host` sent a newer head of `person`'s chain than this machine holds.
    fn want_person(&self, host: &str, person: &str);
}

impl Governance for crate::node::Shared {
    fn facts(&self) -> Facts {
        crate::node::Shared::facts(self)
    }
    fn identity(&self) -> Arc<Identity> {
        crate::node::Shared::identity(self).clone()
    }
    fn want_person(&self, host: &str, person: &str) {
        crate::node::Shared::want_person(self, host, person);
    }
}
