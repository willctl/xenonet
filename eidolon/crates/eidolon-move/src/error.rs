use eidolon_model::{HolderKey, ModelError, Refusal};
use eidolon_vault::VaultError;
use std::fmt;

/// Why a move, or the bookkeeping around one, did not finish.
#[derive(Debug)]
pub enum MoveError {
    /// A rule on this machine said no. Nothing was handed over.
    Refused(Refusal),
    /// The other machine said no, with its reason.
    RefusedByPeer(Refusal),
    /// The stream ended before the move did.
    Cut,
    /// The stream failed.
    Stream(std::io::Error),
    /// The other side sent something this protocol does not allow.
    Protocol(String),
    /// A move to this machine was started and not finished.
    MovePending { to: HolderKey, generation: u64 },
    /// There is no unfinished move to settle.
    NoPending,
    /// The key store has no key for this eidolon.
    NoKey,
    /// The vault or a key operation failed.
    Vault(VaultError),
    /// A value could not be made or signed.
    Model(ModelError),
    /// A file of the holding is missing or damaged.
    BadFile { what: &'static str, why: String },
    /// A file operation failed.
    Disk { what: &'static str, source: std::io::Error },
}

impl fmt::Display for MoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoveError::Refused(r) => write!(f, "{r}"),
            MoveError::RefusedByPeer(r) => write!(f, "The other machine refused: {} {}", r.message, r.fix),
            MoveError::Cut => f.write_str("The connection ended before the move finished. Run the move again; it sends only what is missing."),
            MoveError::Stream(e) => write!(f, "The connection failed: {e}. Run the move again."),
            MoveError::Protocol(why) => write!(f, "The other machine broke the move protocol: {why}. Check that both sides run the same version."),
            MoveError::MovePending { to, generation } => write!(
                f,
                "A move to {} (generation {generation}) was started and not finished, so this copy stays closed. Run the move to the same machine again, or settle it by hand if you know where the eidolon is.",
                to.to_b64()
            ),
            MoveError::NoPending => f.write_str("No move is waiting to be settled here."),
            MoveError::NoKey => f.write_str("This machine has no key for that eidolon, so it cannot be moved from here."),
            MoveError::Vault(e) => write!(f, "{e}"),
            MoveError::Model(e) => write!(f, "{e}"),
            MoveError::BadFile { what, why } => write!(f, "The holding's {what} is missing or damaged ({why}). Restore it from the machine that sent the eidolon."),
            MoveError::Disk { what, source } => write!(f, "Could not {what}: {source}"),
        }
    }
}

impl std::error::Error for MoveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MoveError::Stream(e) | MoveError::Disk { source: e, .. } => Some(e),
            MoveError::Vault(e) => Some(e),
            MoveError::Refused(e) | MoveError::RefusedByPeer(e) => Some(e),
            _ => None,
        }
    }
}

impl From<Refusal> for MoveError {
    fn from(r: Refusal) -> MoveError {
        MoveError::Refused(r)
    }
}

impl From<VaultError> for MoveError {
    fn from(e: VaultError) -> MoveError {
        MoveError::Vault(e)
    }
}

impl From<ModelError> for MoveError {
    fn from(e: ModelError) -> MoveError {
        MoveError::Model(e)
    }
}

impl MoveError {
    pub(crate) fn disk(what: &'static str) -> impl FnOnce(std::io::Error) -> MoveError {
        move |source| MoveError::Disk { what, source }
    }

    pub(crate) fn from_stream(e: std::io::Error) -> MoveError {
        match e.kind() {
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => MoveError::Cut,
            _ => MoveError::Stream(e),
        }
    }
}
