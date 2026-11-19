//! A test-only way to stop the process at a named step.
//!
//! This exists for the kill tests in `eidolon-cli/tests/kill.rs`, which run
//! the built binary and need it to die at one exact step. When the
//! environment variable `EIDOLON_TEST_ABORT_AT` is unset (every normal run)
//! nothing happens. When it names the step being passed, the process aborts
//! at once, as `kill -9` would: no destructor, no cleanup.
//!
//! The steps, in the order they happen:
//!
//! - seal: `after_take` (a facet's files are out of the cell, nothing is
//!   packed), `after_chunks` (every chunk and the sealed manifest are in the
//!   vault, the head is not recorded), `before_head_recorded` (the same, with
//!   the manifest read back), `after_head_recorded` (the holding names the
//!   new head, the state is not updated), `after_state_sealed` (the state
//!   says sealed, the cell has not been told).
//! - open: `after_unpack` (all files are unpacked in the staging folder, the
//!   cell has none of them), `before_state_open` (the files are in the
//!   cell, the state still says sealed), `after_state_open` (the state says
//!   open, the cell has not been told).

/// The variable that names the step to die at.
pub const ABORT_VAR: &str = "EIDOLON_TEST_ABORT_AT";

/// Aborts the process when `EIDOLON_TEST_ABORT_AT` is `step`.
pub fn abort_at(step: &str) {
    if std::env::var_os(ABORT_VAR).is_some_and(|v| v == step) {
        std::process::abort();
    }
}
