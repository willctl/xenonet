//! The typed local protocol between the service and the binary's drivers
//! for its cores (docs/design/direction.md, "The target shape"): the core
//! traits the routes call, JSON-shaped for now (cores/); who an event is
//! from, the sink a driver's events and log lines reach the service
//! through, and what every driver is given (wire.rs); the event stream a
//! relay follows (events.rs); who sent a request (peer.rs); and the client
//! that reaches a running XenoNet over its HTTP API (client.rs, over the
//! HTTP/1.1 of http.rs), which the command, the hook and the MCP servers
//! use. The surfaces import it; it depends on none of them.

#![expect(
    clippy::result_large_err,
    reason = "a core's error carries what the JS errors do (a status, a code, extra fields), and is rare: boxing it would only make each route wordier"
)]

pub mod attest;
pub mod bearer;
pub mod client;
pub mod cores;
pub mod events;
pub mod http;
pub mod limits;
pub mod needs_you;
pub mod peer;
pub mod wire;

pub use wire::From;
