//! The facts a decision rests on, as the receiver holds them: its own copy
//! of each network's chain (members, admins, the founder, each member's
//! settings), the person's chain (devices, revocations, delegations), the
//! token roots it trusts, its consent rows, the actors' allow scopes and
//! the prompts waiting. They come through a trait so the crate needs no
//! store of its own, and another project can bring its own. Everything
//! here is the receiver's; nothing comes from the sender's word.

use crate::vocab::{AskId, Attestation, DeviceId, MachineId, NetworkId, PersonId, PromptKind, SessionId};
use crate::RootCertificate;
use xenonet_id::sign::VerifyingKey;

/// A machine's place in a network, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    Member,
    Admin,
    /// The machine that founded it, which is an admin too.
    Founder,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Member => "member",
            Role::Admin => "admin",
            Role::Founder => "founder",
        }
    }
}

/// Who may start runs on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StartSetting {
    /// Nobody.
    Off,
    /// The person alone.
    #[default]
    You,
    /// The person, and the agents and actors of the network's members.
    Network,
    /// As network, and what they start may run under the person's standing
    /// grant.
    Delegate,
}

impl StartSetting {
    pub const ALL: [StartSetting; 4] = [StartSetting::Off, StartSetting::You, StartSetting::Network, StartSetting::Delegate];

    pub fn as_str(self) -> &'static str {
        match self {
            StartSetting::Off => "off",
            StartSetting::You => "you",
            StartSetting::Network => "network",
            StartSetting::Delegate => "delegate",
        }
    }
}

/// Who may wake a machine's sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WakeSetting {
    /// Those the session's consent rows allow.
    #[default]
    Consent,
    /// Any member's principal.
    Network,
}

impl WakeSetting {
    pub const ALL: [WakeSetting; 2] = [WakeSetting::Consent, WakeSetting::Network];

    pub fn as_str(self) -> &'static str {
        match self {
            WakeSetting::Consent => "consent",
            WakeSetting::Network => "network",
        }
    }
}

/// A machine's settings, as a network's chain records them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Settings {
    pub start: StartSetting,
    pub wake: WakeSetting,
}

/// Whom the person allowed to wake one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Consent {
    /// Agents and machines, here or on another member.
    pub here: bool,
    /// Actors too.
    pub actors: bool,
}

/// How a device proves the person is there, as their chain recorded it
/// when the device was added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// The key asks for a fingerprint or a face before each signature.
    Biometric,
    /// A passphrase or polkit: another program of the account could get
    /// past it.
    Weaker,
}

/// One of the person's devices, unrevoked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub key: VerifyingKey,
    pub kind: DeviceKind,
}

/// Everything of a kind, or only these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Set<T> {
    All,
    Only(Vec<T>),
}

impl<T: PartialEq> Set<T> {
    pub fn has(&self, x: &T) -> bool {
        match self {
            Set::All => true,
            Set::Only(list) => list.contains(x),
        }
    }
}

/// The person's delegation (a standing grant in their chain, signed by one
/// of their devices with their presence): the devices it names may run
/// provider sessions, and answer their prompts within its limits, for work
/// the person started on any device it names. The limits themselves (the
/// delegation judge's, today routine work inside the run's folder) are
/// checked by whoever answers under it, before it asks `decide`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingGrant {
    /// The person's devices it names, or all of them, those paired later
    /// too.
    pub devices: Set<DeviceId>,
    /// Runs may be started under it.
    pub starts: bool,
    /// The kinds of prompt an Allow under it may cover; none, and it
    /// answers nothing.
    pub answers: Vec<PromptKind>,
    /// Milliseconds since 1970 when it ends.
    pub until: i64,
}

/// What an actor's grant lets it Allow, as the row this machine keeps by
/// the grant's token holds it. Issued or widened with the person there;
/// a plain right to answer, without one, lets an actor Deny only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowScope {
    pub sessions: Set<SessionId>,
    pub kinds: Vec<PromptKind>,
    /// Milliseconds since 1970 when it ends.
    pub until: i64,
}

/// A permission prompt waiting here, as this machine holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    /// The session whose run asked it.
    pub session: SessionId,
    /// How that run's agent was checked.
    pub by: Attestation,
    pub kind: PromptKind,
    /// The machine whose principal started the run under the person's
    /// delegation; None for a run that wasn't started under it.
    pub delegated_by: Option<MachineId>,
}

/// The facts the receiver holds. Object-safe, so a decision takes
/// `&dyn Ground`. Each method answers from this machine's own copies and
/// never blocks on another machine.
pub trait Ground {
    /// This machine.
    fn own_machine(&self) -> &MachineId;

    /// `machine`'s role in `network`, or None when it isn't a member.
    fn role(&self, network: &NetworkId, machine: &MachineId) -> Option<Role>;

    /// This machine's settings as `network`'s chain records them, or its
    /// own when there's no network.
    fn settings(&self, network: Option<&NetworkId>) -> Settings;

    /// The person this machine is theirs, when it has one.
    fn own_person(&self) -> Option<PersonId>;

    /// `device`, when the person's chain has it as theirs and unrevoked.
    fn device(&self, person: &PersonId, device: &DeviceId) -> Option<Device>;

    /// Whether a revocation of this Biscuit revocation id is held here.
    fn revoked(&self, revocation_id: &[u8]) -> bool;

    /// The token root certificates held here whose key id (see
    /// [`RootCertificate::key_id`]) is `key_id`, of this machine and of
    /// others, unverified: the caller checks each against its machine's
    /// signing key.
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate>;

    /// `machine`'s signing key, as the network's chain binds it.
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey>;

    /// The consent rows for one of this machine's sessions.
    fn consent(&self, session: &SessionId) -> Consent;

    /// The person's device on `machine`, as the network's chain names it
    /// (this machine's own from its identity): where a delegation's work
    /// runs and where it was started.
    fn device_of(&self, machine: &MachineId) -> Option<DeviceId>;

    /// The delegations in this machine's person's chain, unended; whether
    /// one covers a device is `decide`'s to work out.
    fn delegations(&self) -> Vec<StandingGrant>;

    /// The allow scope of the actor grant whose token's first revocation id
    /// is `token`, when this machine keeps one.
    fn allow_scope(&self, token: &[u8]) -> Option<AllowScope>;

    /// The permission prompt `ask`, when it waits here.
    fn ask(&self, ask: &AskId) -> Option<Asked>;

    /// When this machine last synced governance (the chains) from another
    /// member, in milliseconds since 1970.
    fn governance_synced_at(&self) -> Option<i64>;
}
