//! Xenolith's MCP server, which both programs serve: `xenolith serve` over stdio and
//! `xenolith-service serve --http` over Streamable HTTP.
//!
//! - `mcp`: the server's state, its tools and the JSON-RPC messages.
//! - `resources`: pages as MCP resources, and the start rules as an MCP prompt.

pub mod mcp;
pub mod resources;
