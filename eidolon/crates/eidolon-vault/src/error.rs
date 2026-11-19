use crate::keys::ChunkName;
use std::fmt;

/// Why a vault or key operation failed.
#[derive(Debug)]
pub enum VaultError {
    /// A chunk did not open, or opened to something other than what its name
    /// says: it was damaged or replaced, so it is refused.
    ChangedChunk,
    /// A chunk inflated past the size a chunk may have.
    ChunkTooLarge,
    /// The vault has no chunk by this name.
    MissingChunk(ChunkName),
    /// A name that is not 64 lowercase hex characters.
    BadName,
    /// A wrapped key that did not open: wrong machine, wrong context, or
    /// damaged on the way.
    CannotUnwrap,
    /// The system gave no random numbers, or a key was refused as unusable.
    Crypto(&'static str),
    /// The machine's key store failed.
    KeyStore(String),
    /// The key store holds something under the eidolon's name that is not an
    /// eidolon key.
    BadStoredKey,
    /// A file operation failed.
    Io { what: &'static str, source: std::io::Error },
}

impl VaultError {
    pub(crate) fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> VaultError {
        move |source| VaultError::Io { what, source }
    }
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VaultError::ChangedChunk => f.write_str("A chunk does not match its name, so it was damaged or replaced. Fetch it again from the machine that has it."),
            VaultError::ChunkTooLarge => f.write_str("A chunk is larger than a chunk may be, so it was refused."),
            VaultError::MissingChunk(n) => write!(f, "The vault has no chunk {n}. Fetch it from the machine that has it."),
            VaultError::BadName => f.write_str("A chunk name is 64 lowercase hex characters."),
            VaultError::CannotUnwrap => f.write_str("The eidolon key did not open here: it was wrapped for another machine or purpose, or was damaged."),
            VaultError::Crypto(why) => write!(f, "Encryption failed: {why}."),
            VaultError::KeyStore(why) => write!(f, "The key store failed: {why}"),
            VaultError::BadStoredKey => f.write_str("The key store holds something that is not an eidolon key under this eidolon's name. Remove it and restore the key."),
            VaultError::Io { what, source } => write!(f, "Could not {what}: {source}"),
        }
    }
}

impl std::error::Error for VaultError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VaultError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
