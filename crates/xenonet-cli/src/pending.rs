//! The parts of the core the command calls that aren't wired to the Rust
//! here yet (the relay, the components, and driving XenoNet Dev); the rest are in cores/. Each is a function with the
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

/// runtimeDir()
pub fn runtime_dir() -> Res<String> {
    Err(needs("core/components/runtime.js"))
}

/// tar.c({ file, cwd, portable: true, gzip: true }, entries)
pub fn tar_create(_file: &str, _cwd: &str, _entries: &[String]) -> Res<()> {
    Err(Fail::new("this needs the tar package (cli/commands/setup.js), which isn't in the Rust xenonet yet", 1))
}

/// digest(bytes): its SHA-256, in hex.
pub fn digest(_bytes: &[u8]) -> Res<String> {
    Err(needs("core/components/packages.js"))
}

/// activatePackage(file, item)
pub fn activate_package(_file: &str, _item: &Value) -> Res<Value> {
    Err(needs("core/components/install.js"))
}

/// installLauncher(): the launcher installed, if one was.
pub fn install_launcher() -> Res<Option<Value>> {
    Err(needs("core/components/install.js"))
}

/// recoverServiceUpdate({ env })
pub fn recover_service_update(_env: &Value) -> Res<()> {
    Err(needs("core/components/install.js"))
}

/// installedRuntime(env)
pub fn installed_runtime(_env: &Value) -> Res<Value> {
    Err(needs("core/components/install.js"))
}

/// ensureService(runtime): the service's status.
pub fn ensure_service(_runtime: &Value) -> Res<Value> {
    Err(needs("core/components/runtime.js"))
}

/// connectService({ env }), then client.request(action, body), then close.
pub fn service_request(_env: &Value, _action: &str, _body: &Value) -> Res<Value> {
    Err(needs("core/components/runtime.js"))
}

/// componentUpdates()
pub fn component_updates() -> Res<Vec<Value>> {
    Err(needs("core/components/updates.js"))
}

/// updateComponent(item)
pub fn update_component(_item: &Value) -> Res<Value> {
    Err(needs("core/components/updates.js"))
}

/// installCommand(options): the command's state.
pub fn install_command(_opts: &Value) -> Res<Value> {
    Err(needs("core/command.js"))
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
