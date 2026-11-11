//! core/assistant/ in Rust: the read-only interface a remote actor (a
//! cloud assistant) reaches through `xenonet mcp assistant`
//! (docs/design/cloud-assistants.md).
//!
//! What a grant may see, how its reads are checked and answered, its
//! credential on disk and its page tokens are here. The grants and their
//! standing are the actors' (xenonet-actors' connections.rs), kept in the
//! engine's store. `runtime` runs them on this machine: the connections,
//! the reads and their audit, and the tunnels, as the service's cores.

pub mod audit;
pub mod client;
pub mod credentials;
pub mod crypto;
pub mod fixtures;
pub mod pages;
pub mod projection;
pub mod protocol;
pub mod runtime;
pub mod service;
pub mod source;
pub mod tunnel_client;
pub mod tunnels;

/// A project's id as remote readers see it, the pattern its schemas use.
pub use xenonet_model::assistant::PROJECT_ID;
