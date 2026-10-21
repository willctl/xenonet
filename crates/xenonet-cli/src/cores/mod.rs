//! The parts of the core the command calls, in Rust: each function has the
//! shape of the JS function it stands for (JSON in and out where the JS
//! passes objects), and calls the slice's Rust (docs/design/rust-core.md).
//! Where a twin's JS did the plumbing (child processes, files, fetch,
//! timers, sockets), it's done here, natively; those drivers are in the
//! modules below. Only a script that's a JS module still runs in Node,
//! since its default export is JS (control.rs's run_module).
//!
//! ```text
//! function                 core                                       commands
//! create_world             core/demo.js createWorld                   drift --demo (no XenoNet)
//! connect_preview…remove   core/engine/connect.js createConnect       connect --here, disconnect (no XenoNet)
//! checkup_here             core/doctor/checkup.js, with no XenoNet    doctor, diagnose (no XenoNet)
//! machine_launcher         core/doctor/index.js machine().launcher    diagnose
//! write_listen_settings,   core/delivery/setup.js                     doctor --live
//! live_setup
//! make_bundle…ask_command  core/diagnostics.js                        diagnose
//! decider_*                core/decide.js, core/secret-store.js       decide
//! refusals_*, refusal_     core/refusals/store.js, detect.js          refusals
//! signal
//! is_disabled…delete_      core/assistant/credentials.js              assistant
//! credential
//! run_relay                core/hosts/relay.js runRelay               relay
//! runtime_dir…update_      core/components/                           setup, service, update
//! component
//! supervisor::*            docs/design/always-on.md                   service install, uninstall,
//!                                                                     status, start, run
//! command_dir…uninstall_   core/command.js                            install, uninstall, setup
//! command
//! run_script, control_*    core/script.js, core/control.js            console, script, film
//! ```
#![expect(
    clippy::unnecessary_wraps,
    reason = "each core function answers as a Result, as its JS throws, so a command calls them all alike, whether or not this port of one can fail"
)]

mod command;
mod components;
mod connect;
mod control;
mod demo;
mod native;
mod relay;
mod secrets;
mod setup;
pub mod supervisor;
mod tar;
mod tasks;

pub use components::{activate_package, component_updates, ensure_service, install_launcher, installed_runtime, recover_service_update, service_request, update_component};
pub use control::{control_driver, control_launch, control_quit, control_record, run_module, run_script, ConsoleControl};
pub use relay::run_relay;

use crate::out::Fail;
use crate::sys;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use xenonet_engine::peer::Thrown;
use xenonet_sources::refusals::store::{Refusals, StoreError};

type Res<T> = Result<T, Fail>;

/// What a core threw, as the command says it (`xenonet: <message>`).
pub(crate) fn thrown(t: Thrown) -> Fail {
    Fail::error(t.message)
}

fn fs_fail(e: xenonet_base::fs::FsError) -> Fail {
    Fail::error(e.message)
}

fn store_fail(e: StoreError) -> Fail {
    Fail::error(e.message)
}

/// process.arch.
pub(crate) fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    }
}

/// process.env, as a plain object.
pub(crate) fn env_map() -> serde_json::Map<String, Value> {
    std::env::vars().map(|(k, v)| (k, Value::String(v))).collect()
}

/// The xenonet binary XenoNet's entries in other programs run
/// (runtime.js's xenonetBin): XENONET_BIN, installed XenoNet's launcher, or
/// this binary.
pub fn xenonet_bin() -> String {
    let this = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    xenonet_components::runtime::xenonet_bin(&env_map(), &components::machine(), &this)
}

// ---------- the demo ----------

/// createWorld(): the demo's made-up world, in a temporary folder.
pub fn create_world() -> Res<crate::config::Config> {
    demo::create_world()
}

// ---------- connecting the agents ----------

/// createConnect({ port, root, exec }).preview()
pub fn connect_preview(opts: &Value) -> Res<Value> {
    connect::preview(opts)
}

/// createConnect({ port, root, exec }).apply(apps)
pub fn connect_apply(opts: &Value, apps: &[String]) -> Res<Value> {
    connect::apply(opts, apps)
}

/// createConnect({ port, root, exec }).remove()
pub fn connect_remove(opts: &Value) -> Res<Value> {
    connect::remove(opts)
}

// ---------- the doctor ----------

/// checkup({ ...flags, only }) with no XenoNet running: the checks run
/// here, on the folders of the XenoNet asked about. XenoNet Dev keeps its
/// state.json and drift.json apart from the installed XenoNet's; in them,
/// the projects folder it would read, the one you chose included, as
/// state.json has it.
pub fn checkup_here(flags: &crate::args::Flags, only: Option<&[String]>) -> Res<Value> {
    let cfg = crate::config::config()?;
    let demo = crate::config::demo();
    let name = if demo { None } else { crate::config::stands_for(&flags.pick()) };
    let (state_dir, cache_dir) = match name {
        Some(n) => (crate::config::instance_dir("state", n), crate::config::instance_dir("cache", n)),
        None => (cfg.state_dir.clone(), cfg.cache_dir.clone()),
    };
    let saved = xenonet_state::workspace::saved_root(&sys::join(&state_dir, "state.json")).map(Value::String);
    // XENONET_PROJECTS_DIR doesn't apply to the demo, whose world has its own.
    let env_dir = if demo { None } else { std::env::var("XENONET_PROJECTS_DIR").ok() };
    let root = xenonet_state::workspace::root(env_dir.as_deref(), saved.as_ref(), &cfg.home_dir);
    let paths = json!({
        "homeDir": cfg.home_dir,
        "projectsDir": root["path"],
        "codexHome": cfg.codex_home,
        "claudeProjectsDir": cfg.claude_projects_dir,
        "stateDir": state_dir,
        "cacheDir": cache_dir,
    });
    Ok(xenonet_mcp::doctor::checkup_here(&paths, only))
}

/// machine().launcher()
pub fn machine_launcher() -> Res<Value> {
    xenonet_mcp::doctor::launcher().map(|l| l.unwrap_or(Value::Null)).map_err(Fail::error)
}

/// writeListenSettings({ dir, port }): the file's path.
pub fn write_listen_settings(dir: &str, port: &Value) -> Res<String> {
    setup::write_listen_settings(dir, port)
}

/// liveSetup({ port, listenFile })
pub fn live_setup(port: &Value, listen_file: Option<&str>) -> Res<Value> {
    setup::live_setup(port, listen_file)
}

// ---------- diagnostics ----------

/// os.release(): the kernel's release.
#[cfg_attr(unix, expect(unsafe_code, reason = "uname(3), which std doesn't wrap"))]
fn release() -> String {
    #[cfg(unix)]
    {
        // SAFETY: utsname is a plain C struct of character arrays, for
        // which all zeros is a valid value.
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        // SAFETY: `u` is a struct uname(3) may write, alive for the call.
        if unsafe { libc::uname(&raw mut u) } == 0 {
            // SAFETY: uname filled `release` with a NUL-terminated string,
            // and `u` outlives the borrow.
            let c = unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) };
            return c.to_string_lossy().into_owned();
        }
    }
    String::new()
}

/// makeBundle({ xenonet, doctor, errors, connect, launcher }). The binary
/// has no Node, so the bundle says none.
pub fn make_bundle(input: &Value) -> Res<Value> {
    let errors = input.get("errors").and_then(Value::as_array).cloned().unwrap_or_default();
    let env = env_map();
    let (home, platform, arch, release) = (sys::home(), sys::platform(), arch(), release());
    let have = |k: &str| input.get(k).filter(|v| !v.is_null());
    let g = xenonet_app::diagnostics::Gathered {
        xenonet: have("xenonet"),
        doctor: have("doctor"),
        errors: &errors,
        connect: have("connect"),
        launcher: have("launcher"),
        env: &env,
        home: &home,
        now: native::now(),
        platform,
        arch,
        release: &release,
        node: "none",
        electron: None,
    };
    xenonet_app::diagnostics::make_bundle(&g).map_err(Fail::error)
}

/// bundleMarkdown(bundle)
pub fn bundle_markdown(bundle: &Value) -> Res<String> {
    Ok(xenonet_app::diagnostics::bundle_markdown(bundle))
}

/// writeLook(stateDir, bundle, { sources }): the folder it wrote.
pub fn write_look(state_dir: &str, bundle: &Value, sources: &[String]) -> Res<String> {
    xenonet_app::diagnostics::write_look(state_dir, bundle, sources, native::now()).map_err(fs_fail)
}

/// askCommand({ app, model, dir, sources }): { file, args, cwd }.
pub fn ask_command(app: &str, model: Option<&str>, dir: &str, sources: &[String]) -> Res<Value> {
    Ok(xenonet_app::diagnostics::ask_command(app, model, &json!(dir), sources))
}

// ---------- the decision model ----------

/// createDecider({ secrets: createSecretStore() }).status()
pub fn decider_status() -> Res<Value> {
    let (_, model) = xenonet_app::decide::settings(sys::var("XENONET_DECISIONS_URL").as_deref(), sys::var("XENONET_DECISION_MODEL").as_deref());
    let store = secrets::store();
    let has = store.has(&mut secrets::Runner, xenonet_app::decide::KEY_TARGET).unwrap_or(false);
    Ok(json!({ "model": model, "key": if has { "stored" } else { "missing" }, "store": store.kind() }))
}

/// …storeKey(key)
pub fn decider_store_key(key: &str) -> Res<Value> {
    let checked = xenonet_app::decide::check_key(key).map_err(|e| Fail::error(e.message))?;
    let store = secrets::store();
    store.set(&mut secrets::Runner, xenonet_app::decide::KEY_TARGET, &checked, xenonet_app::decide::KEY_COMMENT).map_err(Fail::error)?;
    Ok(json!({ "key": "stored", "store": store.kind() }))
}

/// …forgetKey()
pub fn decider_forget_key() -> Res<()> {
    let _ = secrets::store().remove(&mut secrets::Runner, xenonet_app::decide::KEY_TARGET);
    Ok(())
}

// ---------- refusals ----------

thread_local! {
    // The command's one store for a folder, as the JS made one
    // createRefusals for the run: the records it kept are known to it.
    static STORES: RefCell<HashMap<String, Refusals>> = RefCell::new(HashMap::new());
}

fn with_store<T>(dir: &str, f: impl FnOnce(&mut Refusals) -> T) -> T {
    STORES.with(|s| f(s.borrow_mut().entry(dir.to_string()).or_insert_with(|| Refusals::new(dir, &sys::home()))))
}

/// contextOn(dir)
pub fn refusals_context_on(dir: &str) -> Res<bool> {
    Ok(xenonet_sources::refusals::store::context_on(std::path::Path::new(dir), std::env::var("XENONET_REFUSAL_CONTEXT").ok().as_deref()))
}

/// setContext(dir, on)
pub fn refusals_set_context(dir: &str, on: bool) -> Res<()> {
    xenonet_sources::refusals::store::set_context(std::path::Path::new(dir), on).map(|_| ()).map_err(store_fail)
}

/// createRefusals({ dir, context }).list({ since, provider })
pub fn refusals_list(dir: &str, since: f64, provider: Option<&str>) -> Res<Vec<Value>> {
    Ok(with_store(dir, |s| s.list(since, provider, None)))
}

/// …get(id)
pub fn refusals_get(dir: &str, id: &str) -> Res<Option<Value>> {
    Ok(with_store(dir, |s| s.get(id)))
}

/// zlib.gunzipSync(fs.readFileSync(…contextFile(id))): a record's context.
pub fn refusals_context(dir: &str, id: &str) -> Res<Vec<u8>> {
    use std::io::Read;
    let file = with_store(dir, |s| s.context_file(id));
    let shown = file.to_string_lossy().into_owned();
    let bytes = std::fs::read(&file).map_err(|e| fs_fail(xenonet_base::fs::FsError::of(e, "open", &shown)))?;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut out).map_err(|_| Fail::error("incorrect header check"))?;
    Ok(out)
}

/// …exportLines({ since, provider, withContext, scrubbed })
pub fn refusals_export_lines(dir: &str, opts: &Value) -> Res<Vec<Value>> {
    let since = opts.get("since").and_then(Value::as_f64).unwrap_or(0.0);
    let provider = opts.get("provider").and_then(Value::as_str).filter(|p| !p.is_empty());
    let on = |k: &str| opts.get(k) == Some(&Value::Bool(true));
    Ok(with_store(dir, |s| s.export_lines(since, provider, None, on("withContext"), on("scrubbed"))))
}

/// …capture(id): { ok, already, why }.
pub fn refusals_capture(dir: &str, id: &str) -> Res<Value> {
    with_store(dir, |s| s.capture(id)).map_err(store_fail)
}

/// …record(signal, { sessionId, app, agentId, file, cwd }): the record
/// kept, or None for one kept already. Its context is kept when keeping
/// context is on, asked as each is recorded.
pub fn refusals_record(dir: &str, signal: &Value, at: &Value) -> Res<Option<Value>> {
    if with_store(dir, |s| s.known(signal, at)) {
        return Ok(None);
    }
    let context = refusals_context_on(dir)?;
    with_store(dir, |s| s.record(signal, at, native::now(), context)).map_err(store_fail)
}

/// claudeSignal(raw) or codexSignal(raw), by the transcript's app: any app
/// but Claude Code is read as Codex.
pub fn refusal_signal(app: &str, raw: &Value) -> Res<Option<Value>> {
    Ok(if xenonet_base::apps::tailed_app(app, "codex") == "claude" {
        xenonet_sources::refusals::detect::claude_signal(Some(raw))
    } else {
        xenonet_sources::refusals::detect::codex_signal(Some(raw))
    })
}

// ---------- remote actors' connections ----------

/// isDisabled(dir)
pub fn is_disabled(dir: &str) -> Res<bool> {
    Ok(xenonet_assistant::credentials::is_disabled(dir))
}

/// markedRevoked(dir): the grants revoked.
pub fn marked_revoked(dir: &str) -> Res<Vec<Value>> {
    Ok(xenonet_assistant::credentials::marked_revoked(dir).into_iter().map(Value::String).collect())
}

/// markDisabled(dir)
pub fn mark_disabled(dir: &str) -> Res<()> {
    xenonet_assistant::credentials::mark_disabled(dir, native::now()).map_err(fs_fail)
}

/// markRevoked(dir, grant)
pub fn mark_revoked(dir: &str, grant: &str) -> Res<()> {
    xenonet_assistant::credentials::mark_revoked(dir, &json!(grant), native::now()).map_err(fs_fail)
}

/// deleteCredential(dir, grant)
pub fn delete_credential(dir: &str, grant: &str) -> Res<()> {
    xenonet_assistant::credentials::delete_credential(dir, &json!(grant)).map_err(fs_fail)
}

// ---------- components ----------

/// runtimeDir()
pub fn runtime_dir() -> Res<String> {
    Ok(xenonet_components::runtime::runtime_dir(&env_map(), &components::machine()))
}

/// tar.c({ file, cwd, portable: true, gzip: true }, entries)
pub fn tar_create(file: &str, cwd: &str, entries: &[String]) -> Res<()> {
    tar::create(file, cwd, entries).map_err(|e| Fail::error(e.to_string()))
}

/// digest(bytes): its SHA-256, in hex.
pub fn digest(bytes: &[u8]) -> Res<String> {
    Ok(xenonet_components::packages::digest(bytes))
}

// ---------- the command on your PATH ----------

/// commandDir()
pub fn command_dir() -> Res<String> {
    Ok(xenonet_app::command::command_dir(&sys::home(), sys::platform(), std::env::var("LOCALAPPDATA").ok().as_deref()))
}

/// installCommand({ dir, bin, force }): the command's state, its script
/// running `bin`.
pub fn install_command(opts: &Value) -> Res<Value> {
    command::install(opts)
}

/// commandState({ dir })
pub fn command_state(dir: &str) -> Res<Value> {
    Ok(command::state(dir))
}

/// uninstallCommand({ dir })
pub fn uninstall_command(dir: &str) -> Res<Value> {
    command::uninstall(dir)
}
