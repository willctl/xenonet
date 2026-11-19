//! Eidolon's chunk store: chunks named by their content, encrypted at rest.
//!
//! - [`keys`]: the eidolon key, the two keys made from it, and sealing and
//!   opening a chunk.
//! - [`wrap`]: the eidolon key wrapped to another machine's X25519 key.
//! - [`keyring`]: the eidolon key kept in the machine's key store.
//! - [`vault`]: sealed chunks on disk.

mod error;
pub mod keyring;
pub mod keys;
pub mod vault;
pub mod wrap;

pub use error::VaultError;
pub use keyring::KeyRing;
pub use keys::{ChunkName, EidolonKey, Keys, MAX_CHUNK_BYTES};
pub use vault::Vault;
pub use wrap::{unwrap_key, wrap_key, WrappedKey};
