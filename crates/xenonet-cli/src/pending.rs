//! The parts of the core the command calls that aren't in Rust yet. Each
//! is a function with the JS function's shape (JSON in and out where the
//! JS passes objects), which says what it needs and exits 1. The commands
//! around them are whole, so wiring the real core in is a change here.
//!
//! ```text
//! function                 JS                                         commands
//! create_world             core/demo.js createWorld                   drift (no XenoNet, --demo)
//! connect_preview          core/engine/connect.js createConnect       connect --here, disconnect (no XenoNet)
//!                          …preview
//! connect_apply            …apply                                     connect --here
//! connect_remove           …remove                                    disconnect (no XenoNet)
//! checkup_here             core/doctor/checkup.js checkup, with no    doctor, diagnose (no XenoNet)
//!                          XenoNet running (core/doctor/index.js,
//!                          context.js)
//! machine_launcher         core/doctor/index.js machine().launcher    diagnose
//! write_listen_settings    core/delivery/setup.js writeListenSettings doctor --live
//! live_setup               core/delivery/setup.js liveSetup           doctor --live
//! make_bundle              core/diagnostics.js makeBundle             diagnose
//! bundle_markdown          core/diagnostics.js bundleMarkdown         diagnose
//! write_look               core/diagnostics.js writeLook              diagnose --ask
//! ask_command              core/diagnostics.js askCommand             diagnose --ask
//! decider_status           core/decide.js createDecider().status,     decide
//!                          with core/secret-store.js
//! decider_store_key        …storeKey                                  decide key
//! decider_forget_key       …forgetKey                                 decide forget
//! refusals_context_on      core/refusals/store.js contextOn           refusals
//! refusals_set_context     core/refusals/store.js setContext          refusals context
//! refusals_list            core/refusals/store.js createRefusals      refusals list, capture --all
//!                          …list
//! refusals_get             …get                                       refusals show
//! refusals_context         …contextFile, gunzipped                    refusals show --context
//! refusals_export_lines    …exportLines                               refusals export
//! refusals_capture         …capture                                   refusals capture
//! refusals_record          …record                                    refusals scan
//! refusal_signal           core/refusals/detect.js claudeSignal,      refusals scan
//!                          codexSignal
//! is_disabled              core/assistant/credentials.js isDisabled   assistant (no XenoNet)
//! marked_revoked           core/assistant/credentials.js markedRevoked assistant (no XenoNet)
//! mark_disabled            core/assistant/credentials.js markDisabled assistant disable
//! mark_revoked             core/assistant/credentials.js markRevoked  assistant revoke
//! delete_credential        core/assistant/credentials.js              assistant revoke
//!                          deleteCredential
//! run_relay                core/hosts/relay.js runRelay               relay
//! runtime_dir              core/components/runtime.js runtimeDir      setup
//! tar_create               the tar package (setup.js's tar.c)         setup
//! digest                   core/components/packages.js digest         setup
//! activate_package         core/components/install.js activatePackage setup
//! install_launcher         core/components/install.js installLauncher setup
//! recover_service_update   core/components/install.js                 service start
//!                          recoverServiceUpdate
//! installed_runtime        core/components/install.js installedRuntime service start
//! ensure_service           core/components/runtime.js ensureService   service start
//! service_request          core/components/runtime.js connectService  service status, stop
//!                          …request
//! component_updates        core/components/updates.js componentUpdates update
//! update_component         core/components/updates.js updateComponent update
//! command_dir              core/command.js commandDir                 install, uninstall
//! install_command          core/command.js installCommand             install, setup
//! command_state            core/command.js commandState               uninstall
//! uninstall_command        core/command.js uninstallCommand           uninstall
//! run_script               core/script.js runScript (core/control.js) console, script run
//! run_module               core/script.js, a JS module's default      script run <file.mjs>
//!                          export run with Node
//! control_launch           core/control.js xenonet(id).launch         film, script run
//! control_driver           core/control.js xenonet(id).driver         film, script
//! control_record           core/control.js xenonet(id).record         film
//! control_quit             core/control.js xenonet(id).quit           film
//! ```

use crate::out::Fail;
use serde_json::Value;

fn needs(module: &str) -> Fail {
    Fail::new(format!("this needs {module}, which isn't in the Rust xenonet yet"), 1)
}

type Res<T> = Result<T, Fail>;

// ---------- the demo ----------

/// createWorld(): the demo's made-up world, in a temporary folder.
pub fn create_world() -> Fail {
    needs("core/demo.js")
}

// ---------- connecting the agents ----------

/// createConnect({ port, root, exec }).preview()
pub fn connect_preview(_opts: &Value) -> Res<Value> {
    Err(needs("core/engine/connect.js"))
}

/// createConnect({ port, root, exec }).apply(apps)
pub fn connect_apply(_opts: &Value, _apps: &[String]) -> Res<Value> {
    Err(needs("core/engine/connect.js"))
}

/// createConnect({ port, root, exec }).remove()
pub fn connect_remove(_opts: &Value) -> Res<Value> {
    Err(needs("core/engine/connect.js"))
}

// ---------- the doctor ----------

/// checkup({ ...flags, only }) with no XenoNet running: the checks run
/// here, on the folders of the XenoNet asked about.
pub fn checkup_here(_flags: &Value, _only: Option<&[String]>) -> Res<Value> {
    Err(needs("core/doctor/checkup.js"))
}

/// machine().launcher()
pub fn machine_launcher() -> Res<Value> {
    Err(needs("core/doctor/index.js"))
}

/// writeListenSettings({ dir, port }): the file's path.
pub fn write_listen_settings(_dir: &str, _port: &Value) -> Res<String> {
    Err(needs("core/delivery/setup.js"))
}

/// liveSetup({ port, listenFile })
pub fn live_setup(_port: &Value, _listen_file: Option<&str>) -> Res<Value> {
    Err(needs("core/delivery/setup.js"))
}

// ---------- diagnostics ----------

/// makeBundle({ xenonet, doctor, errors, connect, launcher })
pub fn make_bundle(_input: &Value) -> Res<Value> {
    Err(needs("core/diagnostics.js"))
}

/// bundleMarkdown(bundle)
pub fn bundle_markdown(_bundle: &Value) -> Res<String> {
    Err(needs("core/diagnostics.js"))
}

/// writeLook(stateDir, bundle, { sources }): the folder it wrote.
pub fn write_look(_state_dir: &str, _bundle: &Value, _sources: &[String]) -> Res<String> {
    Err(needs("core/diagnostics.js"))
}

/// askCommand({ app, model, dir, sources }): { args, cwd }.
pub fn ask_command(_app: &str, _model: Option<&str>, _dir: &str, _sources: &[String]) -> Res<Value> {
    Err(needs("core/diagnostics.js"))
}

// ---------- the decision model ----------

/// createDecider({ secrets: createSecretStore() }).status()
pub fn decider_status() -> Res<Value> {
    Err(needs("core/decide.js"))
}

/// …storeKey(key)
pub fn decider_store_key(_key: &str) -> Res<Value> {
    Err(needs("core/decide.js"))
}

/// …forgetKey()
pub fn decider_forget_key() -> Res<()> {
    Err(needs("core/decide.js"))
}

// ---------- refusals ----------

/// contextOn(dir)
pub fn refusals_context_on(_dir: &str) -> Res<bool> {
    Err(needs("core/refusals/store.js"))
}

/// setContext(dir, on)
pub fn refusals_set_context(_dir: &str, _on: bool) -> Res<()> {
    Err(needs("core/refusals/store.js"))
}

/// createRefusals({ dir, context }).list({ since, provider })
pub fn refusals_list(_dir: &str, _since: f64, _provider: Option<&str>) -> Res<Vec<Value>> {
    Err(needs("core/refusals/store.js"))
}

/// …get(id)
pub fn refusals_get(_dir: &str, _id: &str) -> Res<Option<Value>> {
    Err(needs("core/refusals/store.js"))
}

/// zlib.gunzipSync(fs.readFileSync(…contextFile(id))): a record's context.
pub fn refusals_context(_dir: &str, _id: &str) -> Res<Vec<u8>> {
    Err(needs("core/refusals/store.js"))
}

/// …exportLines({ since, provider, withContext, scrubbed })
pub fn refusals_export_lines(_dir: &str, _opts: &Value) -> Res<Vec<Value>> {
    Err(needs("core/refusals/store.js"))
}

/// …capture(id): { ok, already, why }.
pub fn refusals_capture(_dir: &str, _id: &str) -> Res<Value> {
    Err(needs("core/refusals/store.js"))
}

/// …record(signal, { sessionId, app, agentId, file, cwd }): the record
/// kept, or None for one kept already.
pub fn refusals_record(_dir: &str, _signal: &Value, _at: &Value) -> Res<Option<Value>> {
    Err(needs("core/refusals/store.js"))
}

/// claudeSignal(raw) or codexSignal(raw), by the transcript's app.
pub fn refusal_signal(_app: &str, _raw: &Value) -> Res<Option<Value>> {
    Err(needs("core/refusals/detect.js"))
}

// ---------- remote actors' connections ----------

/// isDisabled(dir)
pub fn is_disabled(_dir: &str) -> Res<bool> {
    Err(needs("core/assistant/credentials.js"))
}

/// markedRevoked(dir): the grants revoked.
pub fn marked_revoked(_dir: &str) -> Res<Vec<Value>> {
    Err(needs("core/assistant/credentials.js"))
}

/// markDisabled(dir)
pub fn mark_disabled(_dir: &str) -> Res<()> {
    Err(needs("core/assistant/credentials.js"))
}

/// markRevoked(dir, grant)
pub fn mark_revoked(_dir: &str, _grant: &str) -> Res<()> {
    Err(needs("core/assistant/credentials.js"))
}

/// deleteCredential(dir, grant)
pub fn delete_credential(_dir: &str, _grant: &str) -> Res<()> {
    Err(needs("core/assistant/credentials.js"))
}

// ---------- hosts ----------

/// runRelay({ port, sinceMs, detail, start, demo }): the exit code.
pub fn run_relay(_opts: &Value) -> Res<i32> {
    Err(needs("core/hosts/relay.js"))
}

// ---------- components ----------

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

// ---------- the command on your PATH ----------

/// commandDir()
pub fn command_dir() -> Res<String> {
    Err(needs("core/command.js"))
}

/// installCommand(options): the command's state.
pub fn install_command(_opts: &Value) -> Res<Value> {
    Err(needs("core/command.js"))
}

/// commandState({ dir })
pub fn command_state(_dir: &str) -> Res<Value> {
    Err(needs("core/command.js"))
}

/// uninstallCommand({ dir })
pub fn uninstall_command(_dir: &str) -> Res<Value> {
    Err(needs("core/command.js"))
}

// ---------- driving XenoNet Dev ----------

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
