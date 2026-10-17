//! The XenoNet service, `xenonet serve` (docs/design/rust-core.md, "The
//! service"): what `node server/index.js` is.

/// Runs the service with the command's arguments (after `serve`) until it's
/// stopped; the exit code.
pub fn run(_args: &[String], _version: &str) -> i32 {
    eprintln!("xenonet: the Rust service isn't ready yet");
    1
}
