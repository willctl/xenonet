//! The XenoNet service, `xenonet serve` (docs/design/rust-core.md, "The
//! service"): the composition root. It makes the cores and the service's
//! own parts, the component service's socket (component.rs), the actor
//! gateway (gateway/), A2A (a2a/), the network, the assistant's endpoint
//! and the terminals, hands them to the HTTP API (xenonet-http, behind its
//! parts' traits, parts.rs here) and listens (serve.rs).

#![expect(
    clippy::result_large_err,
    reason = "a core's error carries what the JS errors do (a status, a code, extra fields), and is rare: boxing it would only make each route wordier"
)]
#![expect(clippy::type_complexity, reason = "hooks are closures shared between tasks, typed where they're kept")]

pub mod a2a;
pub mod assistant_endpoint;
pub mod component;
pub mod compose;
pub mod gateway;
pub mod parts;
pub mod rpc;
pub mod serve;
pub mod terminals;

// The HTTP API is xenonet-http's, the cores' traits and what the drivers
// are given xenonet-api's, fetch and output lines xenonet-os's; the
// service's parts and its tests reach them at these paths.
pub use xenonet_api::{cores, wire};
pub use xenonet_http::{actor_caller, borrowed, config, ctx, events, glue, http, routes, server, static_files, upstream, window_key};
pub use xenonet_os::{lines, net};

pub use serve::run;
