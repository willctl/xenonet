//! What the MCP servers call that belongs to a slice of core/ not in Rust
//! yet (docs/design/rust-core.md, "The order of the port"). Each says so
//! when it's reached, rather than guess at another slice's answer, and
//! goes once that slice lands:
//!
//! - `fixture_service`: `xenonet mcp assistant --fixture`, the assistant's
//!   six tools over made-up data, which runs the assistant's service
//!   (core/assistant/service.js, with projection.js, cursors.js and
//!   actors/limits.js) over core/assistant/fixtures.js in this process.

/// The assistant's service over made-up data, for --fixture.
pub fn fixture_service() -> Result<(), String> {
    Err("xenonet mcp assistant --fixture needs the assistant's service and its made-up data (core/assistant/service.js, core/assistant/fixtures.js), which aren't in the Rust xenonet yet; run node cli/xenonet.js mcp assistant --fixture for now".into())
}
