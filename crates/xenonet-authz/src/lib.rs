//! Who may do what, decided where an act arrives (docs/design/direction.md,
//! "Authorization"). Every incoming act goes through one function,
//! [`decide`], which judges a closed [`Request`] on the facts the receiver
//! holds itself ([`Ground`]), a Biscuit [`Token`] when the sender is an
//! actor (a bot or a cloud assistant too), and a [`VerifiedPresence`] when
//! the act needs the person there. What the sender says of its own strength is never an
//! input, and message text can't be one: it's [`Information`], which
//! converts to nothing.
//!
//! - `vocab`: the principals, actions, resources, acts and doors a request
//!   is made of, and the hash a person's device signs for an act.
//! - `info`: `Information<T>`, text that informs and never authorizes.
//! - `ground`: the facts the receiver holds, as a trait, so another project
//!   can bring its own store.
//! - `proof`: a person's signed act and their presence, each checked
//!   against the device key the ground gives, for the one act it names.
//! - `token`: token roots certified by a machine's signing key, and the
//!   actors' tokens they mint, attenuate and verify.
//! - `decide`: the rule table, and the decision with what it allowed.
//!
//! It depends on `xenonet-id` alone, so it can be taken without the rest of
//! XenoNet.

mod decide;
mod ground;
mod info;
mod proof;
mod token;
mod vocab;

pub use decide::{decide, Authority, Authorized, Decision, Reason, HOLD_AFTER_MS};
pub use ground::{Consent, Device, DeviceKind, Ground, Role, Settings, StandingGrant, StartSetting, WakeSetting};
pub use info::Information;
pub use proof::{PersonSigned, VerifiedPresence};
pub use token::{ActorGrant, Bearer, Narrowing, RootCertificate, Token, TokenRoot, ACTOR_PREFIX, CURSOR_MS, GATEWAY_PREFIX};
pub use vocab::{
    act_hash, Act, Action, ActorSlug, AgentId, AnswerDecision, AppId, AskId, Attestation, DeviceId, Door, Folder, MachineId, NetworkId, PersonId, Principal, ReadScope, Request,
    Resource, Revocation, ServiceId, SessionId, StartMode,
};

use std::fmt;

/// Why something couldn't be made or read: a short code and words for a
/// person. A decision's refusal is a [`Reason`], not this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthzError {
    pub code: &'static str,
    pub message: String,
}

impl AuthzError {
    pub fn new(code: &'static str, message: impl Into<String>) -> AuthzError {
        AuthzError { code, message: message.into() }
    }
}

impl fmt::Display for AuthzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AuthzError {}

impl From<xenonet_id::IdError> for AuthzError {
    fn from(e: xenonet_id::IdError) -> Self {
        AuthzError { code: e.code, message: e.message }
    }
}

pub type R<T> = Result<T, AuthzError>;
