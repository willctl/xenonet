//! Who signed what (docs/design/authorization.md): the keys XenoNet signs
//! with, what each signature is for, where a key is kept, and the presence
//! block that proves a person was at a device when it signed. Nothing here
//! knows about networks, frames or chains, so another project can take it
//! alone.
//!
//! - `codec`: base64url, base32, base58btc, SHA-256 and random bytes.
//! - `keys`: a machine's transport keys (Ed25519 to sign frames, X25519 to
//!   be sealed to) in an owner-only file, and their key id.
//! - `sign`: a signature's `Purpose`, the `VerifyingKey` any signature is
//!   checked with, and the `Signer` that holds a private key wherever it
//!   lives (a file, the OS keychain, a secure element).
//! - `store`: where secrets are kept (`KeyStore`), and a signer whose key
//!   is in one.
//! - `presence`: the person's OS check (`UserCheck`), the `PresenceBlock`
//!   a device signs after it, and the receiver's `PresenceCheck`, which
//!   takes each block once, for the one act it names, within 60 seconds.
//! - `did`: a key as a did:key.

pub mod codec;
pub mod did;
pub mod keys;
pub mod presence;
pub mod sign;
pub mod store;

use std::fmt;

/// Why something failed: a short code and words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    pub code: &'static str,
    pub message: String,
}

impl IdError {
    pub fn new(code: &'static str, message: impl Into<String>) -> IdError {
        IdError { code, message: message.into() }
    }

    /// Something on this machine failed: a file, randomness, a key store.
    pub fn internal(message: impl Into<String>) -> IdError {
        IdError::new("internal", message)
    }

    /// Refused: a signature that doesn't check, a block that's stale or seen.
    pub fn refused(message: impl Into<String>) -> IdError {
        IdError::new("refused", message)
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IdError {}

impl From<std::io::Error> for IdError {
    fn from(e: std::io::Error) -> Self {
        IdError::internal(format!("a key's file: {e}"))
    }
}

pub type R<T> = Result<T, IdError>;
