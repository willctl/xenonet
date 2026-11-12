//! Cloud assistants: the remote actors that read XenoNet through `xenonet
//! mcp assistant` (docs/design/cloud-assistants.md).
//!
//! What a grant may see (`projection`), how its reads are checked and
//! answered (`service`, over `source`, with the made-up world's in
//! `fixtures`), the audit of what it read (`audit`), its credential on disk
//! (`credentials`), and the tunnels XenoNet runs for them (`tunnels`, with
//! `tunnel_client`, the tunnel's own process). The grants and their
//! standing are `connections`', kept in the engine's store; the page
//! cursors and the cores the service runs them as are the composition
//! root's (xenonet-service's compose/assistant/).

pub mod audit;
pub mod credentials;
pub mod crypto;
pub mod fixtures;
pub mod projection;
pub mod service;
pub mod source;
pub mod tunnel_client;
pub mod tunnels;
