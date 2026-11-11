//! Intents (docs/design/goals.md): work the person asks for at one of
//! their devices, worked on the machine it names under one of their
//! delegations, and what came of it. The engine's driver implements it;
//! the routes and the command reach it here.

use super::door::Attestation;
use super::{pending, R};
use serde_json::Value;

const INTENTS: &str = "intents";

/// Who reports on an intent through this machine's own door: a session,
/// with how its process was checked, or an actor, by its token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reporter {
    Session { id: String, attestation: Attestation },
    Actor { token: String },
}

pub trait Intents: Send + Sync {
    /// The intents this machine keeps, the newest first, and what a new
    /// one may name (`{ intents, machines, delegations, projects }`).
    fn list(&self) -> R<Value> {
        pending(INTENTS)
    }

    /// The person's new intent (`{ goal, target, project, app?,
    /// delegation?, hours? }`), signed by their device: at the window, the
    /// device's signature; anywhere else it needs their presence, which
    /// this machine asks only at the window. The intent as kept.
    fn make(&self, _body: Value, _at_window: bool) -> R<Value> {
        pending(INTENTS)
    }

    /// The person's cancel of the intent `id`, signed as `make` is.
    fn cancel(&self, _id: &str, _at_window: bool) -> R<Value> {
        pending(INTENTS)
    }

    /// A note (`{ text }`) or the outcome (`{ outcome, failed? }`) of an
    /// intent worked here, from its agent or an actor whose grant names it:
    /// `step` is note or done.
    fn report(&self, _by: Reporter, _id: &str, _step: &str, _body: Value) -> R<Value> {
        pending(INTENTS)
    }
}

impl Intents for super::Pending {}
