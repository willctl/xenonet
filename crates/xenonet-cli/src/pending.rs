//! The parts of the core the command calls that aren't wired to the Rust
//! here yet (the relay, and driving XenoNet Dev); the rest are in cores/. Each is a function with the
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

/// runScript(xenonet(instance), steps, say)
pub fn run_script(_instance: &str, _steps: &[Value], _say: &mut dyn FnMut(&Value)) -> Res<()> {
    Err(needs("core/script.js"))
}

/// A script that's a JS module: its default export, given the instance and
/// a step function.
pub fn run_module(_file: &str, _instance: &str) -> Res<()> {
    Err(needs("core/script.js"))
}

/// xenonet(instance).launch(options)
pub fn control_launch(_instance: &str, _opts: &Value) -> Res<()> {
    Err(needs("core/control.js"))
}

/// xenonet(instance).driver(op, arg)
pub fn control_driver(_instance: &str, _op: &str, _arg: &Value) -> Res<Value> {
    Err(needs("core/control.js"))
}

/// xenonet(instance).record(options)
pub fn control_record(_instance: &str, _opts: &Value) -> Res<Value> {
    Err(needs("core/control.js"))
}

/// xenonet(instance).quit()
pub fn control_quit(_instance: &str) -> Res<()> {
    Err(needs("core/control.js"))
}
