//! The generic MCP adapter: what each MCP server on the map offers, for
//! the console to show (the McpInfo core). The apps the registry marks as
//! listing their servers' status (`mcpStatus`) are asked, and the one
//! server already configured is started and listed, never a tool called,
//! a resource read or a sign-in started.
//!
//! - `info`: a catalog collected as a state machine, and what of it the
//!   console may see (no configuration, no credentials).
//! - `discovery`: how to reach a configured server and how far to page.
//! - `catalog`: the McpInfo core, with a small MCP client of its own over
//!   a program's standard input and output or Streamable HTTP.

pub mod catalog;
pub mod discovery;
pub mod info;
