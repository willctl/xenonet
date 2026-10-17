//! What the MCP servers call that belongs to a slice of core/ not in Rust
//! yet (docs/design/rust-core.md, "The order of the port"). Each says so
//! when it's reached, rather than guess at another slice's answer, and
//! goes once that slice lands:
//!
//! - `local_doctor`: the doctor's own checks, which the doctor tool runs
//!   when no XenoNet answers (core/doctor/checkup.js's fallback, over
//!   core/doctor/index.js and its context). With a XenoNet running, the
//!   doctor tool asks it, in Rust.

use serde_json::Value;

/// The doctor's checks, run here because no XenoNet answers on `port`.
pub fn local_doctor(port: f64) -> Result<Value, String> {
    let _ = port;
    Err("XenoNet isn't running, and the doctor's own checks (core/doctor/index.js) aren't in the Rust xenonet yet, so this server can't run them without it. Your user can open XenoNet, or run node cli/xenonet.js doctor.".into())
}
