//! What the MCP servers call that belongs to a slice of core/ not in Rust
//! yet (docs/design/rust-core.md, "The order of the port"). Each says so
//! when it's reached, rather than guess at another slice's answer, and
//! goes once that slice lands:
//!
//! - `local_doctor`: the doctor's own checks, which the doctor tool runs
//!   when no XenoNet answers (core/doctor/checkup.js's fallback, over
//!   core/doctor/index.js and its context). With a XenoNet running, the
//!   doctor tool asks it, in Rust.
//! - `fixture_service`: `xenonet mcp assistant --fixture`, the assistant's
//!   six tools over made-up data, which runs the assistant's service
//!   (core/assistant/service.js, with projection.js, cursors.js and
//!   actors/limits.js) over core/assistant/fixtures.js in this process.
//!   The service runs inside XenoNet too, so it's the assistant slice's to
//!   port, not this crate's.

use serde_json::Value;

/// The doctor's checks, run here because no XenoNet answers on `port`.
pub fn local_doctor(port: f64) -> Result<Value, String> {
    let _ = port;
    Err("XenoNet isn't running, and the doctor's own checks (core/doctor/index.js) aren't in the Rust xenonet yet, so this server can't run them without it. Your user can open XenoNet, or run node cli/xenonet.js doctor.".into())
}

/// The assistant's service over made-up data, for --fixture.
pub fn fixture_service() -> Result<(), String> {
    Err("xenonet mcp assistant --fixture needs the assistant's service and its made-up data (core/assistant/service.js, core/assistant/fixtures.js), which aren't in the Rust xenonet yet; run node cli/xenonet.js mcp assistant --fixture for now".into())
}
