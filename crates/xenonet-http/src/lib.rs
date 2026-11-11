//! XenoNet's local HTTP API (docs/design/rust-core.md, "The service"): the
//! routes on 127.0.0.1 (routes/), the one table each is checked by before
//! its handler runs (routes/rules.rs), the requests and answers (http.rs,
//! server.rs), the event stream (events.rs), the cores' events on their
//! way to the pages (glue.rs) and the page's files (static_files.rs).
//!
//! A surface: it imports xenonet-api and the base crates alone
//! (scripts/rust-layers.mjs). The cores come through xenonet-api's traits,
//! and the service's own parts (the actor gateway, the component service,
//! the assistant's endpoint, A2A, the network) through the traits in
//! parts.rs, which the composition root, xenonet-service, implements.

#![expect(
    clippy::result_large_err,
    reason = "a core's error carries what the JS errors do (a status, a code, extra fields), and is rare: boxing it would only make each route wordier"
)]

pub mod actor_caller;
pub mod borrowed;
pub mod config;
pub mod ctx;
pub mod events;
pub mod glue;
pub mod http;
pub mod parts;
pub mod routes;
pub mod server;
pub mod static_files;
pub mod upstream;
pub mod window_key;

// The cores' traits and what the drivers are given are xenonet-api's.
pub use xenonet_api::{cores, wire};
