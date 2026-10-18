//! The parts of the core the command calls that aren't wired to the Rust
//! here yet (the relay); the rest are in cores/. Each is a function with the
//! JS function's shape, which says what it needs and exits 1.

use crate::out::Fail;
use serde_json::Value;

fn needs(module: &str) -> Fail {
    Fail::new(format!("this needs {module}, which isn't in the Rust xenonet yet"), 1)
}

type Res<T> = Result<T, Fail>;

/// runRelay({ port, sinceMs, detail, start, demo }): the exit code.
pub fn run_relay(_opts: &Value) -> Res<i32> {
    Err(needs("core/hosts/relay.js"))
}
