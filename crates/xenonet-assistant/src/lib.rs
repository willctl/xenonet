//! core/assistant/ in Rust: the read-only interface a remote actor (a
//! cloud assistant) reaches through `xenonet mcp assistant`
//! (docs/design/cloud-assistants.md).
//!
//! What a grant may see, how its reads are checked and answered, its
//! credential on disk and its page tokens are here. The schemas stay
//! zod's (core/schema/assistant.js), the grants and their standing stay
//! the engine's (core/actors/connections.js), and the twins ask for both.

pub mod audit;
pub mod client;
pub mod credentials;
pub mod crypto;
pub mod cursors;
pub mod fixtures;
pub mod projection;
pub mod protocol;
pub mod service;
pub mod source;
pub mod tunnel_client;
pub mod tunnels;
