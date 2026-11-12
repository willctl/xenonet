//! xenonet-mcp: XenoNet's MCP servers (mcp/), in Rust, on rmcp
//! (docs/design/rust-core.md, "The MCP servers"). Each declares the JS
//! server's tools as it does (the same names, titles, descriptions,
//! annotations and schemas, `zod`), checks a call's arguments as the MCP
//! SDK does, and answers with the same text.
//!
//! - `serve`: the sessions' and actors' server (mcp/xenonet.js), `xenonet
//!   mcp`, in `xenonet`.
//! - `serve_assistant`: the read-only server for a remote actor paired in
//!   XenoNet's window (mcp/assistant.js), `xenonet mcp assistant`, in
//!   `assistant`.
//! - `serve_console`: the console operator (mcp/console.js), `xenonet mcp
//!   console`, in `console`, its tools answered through the `Control` the
//!   command gives it (core/control.js's client, natively).
//!
//! It imports xenonet-api and the base crates alone. What it needs of the
//! rest the composition root, the xenonet command that serves it, gives it:
//! the registry's apps (`Registry`), the console's control (`Control`) and
//! the assistant's made-up data (`fixture::Fixture`). With no XenoNet
//! answering, the doctor tool runs `xenonet doctor --json`, the binary it
//! is, so the checks run on this machine.

mod assistant;
mod console;
mod engine;
mod process;
mod tools;
mod xenonet;
mod zod;

/// What `xenonet mcp assistant --fixture` serves, which the root gives it,
/// and the contract that data is held to.
pub mod fixture {
    pub use crate::assistant::{contract, Answer, Fixture};
}

pub use console::Control;
pub use xenonet::{channel_enabled, server_name, Registry, StartApp};

/// Serves the sessions' server on stdio until the client goes; `xenonet
/// mcp` runs it, with the registry's apps.
pub async fn serve(registry: Registry) {
    xenonet::serve(registry).await;
}

/// Serves the console operator (mcp/console.js) on stdio until the client
/// goes, each tool answered through `control`; `installed` for installed
/// Tools', which operate installed XenoNet only, with no eval or simulate.
pub async fn serve_console(control: std::sync::Arc<dyn Control>, installed: bool) -> Result<(), String> {
    console::serve(control, installed).await
}

/// Serves the assistant's read-only server (mcp/assistant.js) on stdio,
/// for `grant`, or over the made-up data of `fixture`, for the XenoNet on
/// `port` (XENONET_PORT, or 47840, when it's None), until the client goes.
pub async fn serve_assistant(grant: Option<String>, fixture: Option<std::sync::Arc<dyn fixture::Fixture>>, port: Option<u16>) -> Result<(), String> {
    assistant::serve(grant, fixture, port).await
}
