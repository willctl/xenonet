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
//! - `serve_console`: the console operator (mcp/console.js), which stays
//!   JS for now: it drives XenoNet Desktop through core/control.js, which
//!   the desktop app, the end-to-end tests and the scripts use too.
//!
//! What they need of other slices of core/ that aren't in Rust yet is in
//! `pending`.

mod assistant;
mod engine;
mod pending;
mod process;
mod tools;
mod xenonet;
mod zod;

pub use xenonet::{channel_enabled, server_name};

/// Serves the sessions' server on stdio until the client goes; `xenonet
/// mcp` runs it.
pub async fn serve() {
    xenonet::serve().await;
}

/// Serves the console operator (mcp/console.js) on stdio. It isn't in Rust:
/// its tools drive XenoNet Desktop through core/control.js, which stays JS
/// for the desktop app, the end-to-end tests and the scripts, and its
/// development instances run the checkout's Electron, so Node is there
/// whenever they are.
pub async fn serve_console() -> Result<(), String> {
    Err("the console operator isn't in the Rust xenonet yet: run node mcp/console.js (or node cli/xenonet.js mcp console) from XenoNet's checkout".into())
}

/// Serves the assistant's read-only server (mcp/assistant.js) on stdio,
/// for `grant`, or over made-up data with `fixture`, for the XenoNet on
/// `port` (XENONET_PORT, or 47840, when it's None), until the client goes.
pub async fn serve_assistant(grant: Option<String>, fixture: bool, port: Option<u16>) -> Result<(), String> {
    assistant::serve(grant, fixture, port).await
}
