//! The facts a decision rests on, as the receiver holds them: its own copy
//! of each network's chain (members, admins, the founder, each member's
//! settings), the person's chain (devices, revocations), the token roots it
//! trusts, its consent rows and the person's standing grant. They come
//! through a trait so the crate needs no store of its own, and another
//! project can bring its own. Everything here is the receiver's; nothing
//! comes from the sender's word.

use crate::vocab::{AskId, Attestation, DeviceId, GrantId, MachineId, NetworkId, PersonId, SessionId};
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

/// The person's standing grant for this machine (a delegation): signed by
/// the person, checked by whoever implements [`Ground`], with its limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StandingGrant {
    /// Runs may be started under it.
    pub starts: bool,
    /// It answers runs' permission prompts.
    pub answers: bool,
    /// The weakest attestation of an agent it covers: never below
    /// process-checked for answers, whatever it says.
    pub requires: Attestation,
    /// Milliseconds since 1970 when it ends.
    pub until: i64,
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

    /// The person's standing grant for `machine`, when one covers it.
    fn standing_grant(&self, machine: &MachineId) -> Option<StandingGrant>;

    /// How the agent of the run that asked `ask` was checked, when the
    /// question is waiting here.
    fn asker(&self, ask: &AskId) -> Option<Attestation>;

    /// The current revision of an assistant's read grant, when it's there.
    fn assistant_grant(&self, grant: &GrantId) -> Option<u64>;

    /// When this machine last synced governance (the chains) from another
    /// member, in milliseconds since 1970.
    fn governance_synced_at(&self) -> Option<i64>;
}
