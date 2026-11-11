//! The doctor inside the service (the Doctor core, over `checks` and
//! `run`) and the diagnostics bundle (the Diagnostics core, over
//! `diagnostics`).
//!
//! Each check asks what it reads of the machine through `DoctorHost`
//! (`Here`): its files and programs here, and XenoNet's own state through
//! the other cores (the map's health and live state, the store's load report, the drift
//! log, the process check, actors, the gateway, wakes). The checks
//! run side by side, each given 8 seconds. A run happens a minute after the
//! service starts, then every half hour, and when asked; new problems and
//! each result go out as the doctor's events. The demo checks only itself,
//! never your machine.

use crate::checks::{self, DoctorHost};
use crate::run;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use xenonet_api::cores::{CoreError, Cores, Diagnostics, Doctor, R};
use xenonet_api::wire::{Emitter, GatewayStatus, Slot};
use xenonet_api::From;
use xenonet_os::error::Failure;
use xenonet_os::fx;
use xenonet_os::fx::RunOptions;
use xenonet_os::sync::Locked;
use xenonet_schema::drift::DriftLog;
use xenonet_text::time::now_ms;

const CHECK_MS: u64 = 8000;
const DAY_MS: f64 = 24.0 * 3600e3;
const EVERY_MS: u64 = 30 * 60 * 1000;
const FIRST_AFTER_MS: u64 = 60 * 1000;

/// The checks the demo runs: itself, never your machine.
pub const DEMO_ONLY: [&str; 9] =
    ["xenonet.server", "xenonet.sources", "xenonet.blocks", "contracts.drift", "claude.transcripts", "codex.sessions", "grok.sessions", "system.projects", "system.runtime"];

/// The service's settings the doctor reads.
#[derive(Clone, Debug)]
pub struct Settings {
    pub demo: bool,
    pub port: u16,
    /// The app's name, which the desktop app passes on.
    pub name: String,
    /// installed, demo, dev, or the instance XenoNet Dev names.
    pub instance: Option<String>,
    /// XENONET_LAUNCHER_EXEC: installed XenoNet's launcher started it.
    pub launcher_exec: Option<String>,
    pub home: String,
    pub state_dir: String,
    /// Where XenoNet's files are.
    pub root: std::path::PathBuf,
}

/// What the doctor reads XenoNet's own state through.
pub struct Parts {
    pub cores: Cores,
    pub config: Settings,
    /// config.js's settings, for `paths`.
    pub paths: Value,
    pub version: String,
    pub drift: Arc<Mutex<DriftLog>>,
    pub emitter: Emitter,
    /// The actor gateway, once the service has made it.
    pub gateway: Slot<Arc<dyn GatewayStatus>>,
    /// Why the store in a state folder can't be used, if it can't
    /// (`xenonet_store::standing`, which the composition gives).
    pub standing: fn(&str) -> Option<String>,
    /// The machine signing key's row (its verdict), from the key folder:
    /// where the key is, and that it opens; None before it's made
    /// (`xenonet_keys::machine_standing`, which the composition gives).
    pub machine_key: fn(&std::path::Path) -> Option<Value>,
    /// Where the network and the person are kept, while the store is open.
    pub net: Option<Arc<dyn xenonet_net::persist::Persist>>,
}

impl Parts {
    // The network as this machine keeps it, with its folder's key; None
    // with no store, where there's no network either.
    fn network(&self) -> Option<xenonet_net::files::Dir> {
        let kept = self.net.clone()?;
        Some(xenonet_net::files::Dir::new(std::path::Path::new(&self.config.state_dir).join("network"), kept))
    }

    fn home(&self) -> String {
        self.config.home.clone()
    }
}

// ---------- the machine, as the checks ask it ----------

#[cfg(unix)]
#[expect(unsafe_code, reason = "access(2), which std doesn't wrap")]
fn writable(p: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(p) else { return false };
    // SAFETY: access() only reads the NUL-terminated path, which outlives
    // the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(p: &str) -> bool {
    std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly())
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "statvfs(3), which std doesn't wrap")]
fn statfs(p: &str) -> Option<Value> {
    // Its counts are 32 bits on macOS and 64 on Linux; JS numbers either way.
    fn n(v: impl Into<u64>) -> f64 {
        v.into() as f64
    }
    let c = std::ffi::CString::new(p).ok()?;
    // SAFETY: statvfs is a plain C struct of numbers, for which all zeros is
    // a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: the path is NUL-terminated and `st` a struct statvfs(3) may
    // write, both alive for the call.
    if unsafe { libc::statvfs(c.as_ptr(), &raw mut st) } != 0 {
        return None;
    }
    Some(json!({ "bsize": n(st.f_frsize), "blocks": n(st.f_blocks), "bfree": n(st.f_bfree), "bavail": n(st.f_bavail), "files": n(st.f_files), "ffree": n(st.f_ffree) }))
}

#[cfg(not(unix))]
fn statfs(_: &str) -> Option<Value> {
    None
}

fn fs_error(e: xenonet_os::fs::FsError) -> Value {
    let mut m = Map::new();
    m.insert("message".into(), json!(e.message));
    if let Some(c) = e.code {
        m.insert("code".into(), json!(c));
    }
    Value::Object(m)
}

// run(file, args, ms) as machine() runs a program: never an error, and
// without ELECTRON_RUN_AS_NODE.
async fn run_program(file: &str, args: &[String], ms: f64) -> Value {
    fx::exec_file(file, args, ms.max(1.0) as u64, RunOptions { remove: vec!["ELECTRON_RUN_AS_NODE".into()], ..Default::default() }).await.to_value()
}

/// gitAvailable() (core/git.js): git on PATH, and on a Mac, the Command
/// Line Tools it needs.
async fn git_available() -> bool {
    let git = fx::on_path("git");
    let mut ok = git.is_some();
    if xenonet_sources::git::needs_dev_tools(xenonet_os::platform(), git.as_deref()) {
        let r = fx::exec_file("/usr/bin/xcode-select", &["-p".into()], 3000, RunOptions::default()).await;
        if !r.ok() {
            ok = false;
        }
    }
    ok
}

struct Disk;

impl xenonet_adapter::dirs::Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        std::fs::metadata(p).is_ok()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, xenonet_adapter::dirs::Unreadable> {
        xenonet_os::paths::read_dir_sorted(std::path::Path::new(p))
            .map(|v| v.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .map_err(|_| xenonet_adapter::dirs::Unreadable)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_os::fs::mtime_ms_of(&m).ok()).unwrap_or(0.0)
    }
}

/// The facts the adapters' checks read that the doctor gathers here
/// (Codex's command, `codexBin`), in the adapters' order.
fn adapter_facts(home: &str) -> Vec<(&'static str, Value)> {
    let env = xenonet_adapter::dirs::Env {
        claude_desktop_dir: None,
        local_app_data: fx::env_set("LOCALAPPDATA"),
        app_data: fx::env_set("APPDATA"),
        xdg_config_home: fx::env_set("XDG_CONFIG_HOME"),
    };
    xenonet_adapter::platforms().iter().flat_map(|p| p.doctor_facts(home, &env, &mut Disk)).collect()
}

/// machine().launcher(): the launcher hooks and MCP servers run, when
/// there is one, what it says of itself, its version and the one this
/// XenoNet ships, the launcher script built in.
pub async fn launcher() -> Result<Value, Value> {
    let env = fx::env_map();
    let home = xenonet_os::home_dir(&|k| std::env::var(k).ok());
    let m = xenonet_components::runtime::Machine { platform: xenonet_os::platform().into(), arch: String::new(), home };
    let dir = xenonet_os::paths::join(&xenonet_components::runtime::runtime_dir(&env, &m), "launcher");
    // The xenonet binary first, which entries run now; it's a copy of the
    // Tools' own, so it's as new as they are.
    let bin = xenonet_os::paths::join(&dir, if cfg!(windows) { "xenonet.exe" } else { "xenonet" });
    if std::fs::metadata(&bin).is_ok() {
        let r = run_program(&bin, &["--launcher".into()], 8000.0).await;
        let said = r["stdout"].as_str().and_then(|s| xenonet_text::json::parse(s).ok()).unwrap_or(Value::Null);
        let ships = xenonet_components::install::launcher_version(Some(xenonet_components::install::LAUNCHER_SCRIPT));
        let version = said.get("launcher").cloned().unwrap_or_else(|| xenonet_text::json::number(ships));
        return Ok(json!({ "dir": dir, "said": said, "version": version, "ships": xenonet_text::json::number(ships) }));
    }
    let node = xenonet_os::paths::join(&dir, if cfg!(windows) { "node.exe" } else { "node" });
    let entry = xenonet_os::paths::join_all(&[&dir, "cli", "xenonet.js"]);
    if std::fs::metadata(&node).is_err() || std::fs::metadata(&entry).is_err() {
        return Ok(Value::Null);
    }
    let r = run_program(&node, &[entry.clone(), "--launcher".into()], 8000.0).await;
    let said = r["stdout"].as_str().and_then(|s| xenonet_text::json::parse(s).ok()).unwrap_or(Value::Null);
    let read = |f: &str| std::fs::read(f).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| fs_error(xenonet_os::fs::FsError::of(e, "open", f)));
    let version = xenonet_components::install::launcher_version(Some(&read(&entry)?));
    let ships = xenonet_components::install::launcher_version(Some(xenonet_components::install::LAUNCHER_SCRIPT));
    Ok(json!({ "dir": dir, "said": said, "version": xenonet_text::json::number(version), "ships": xenonet_text::json::number(ships) }))
}

/// The program a restart of the service runs (installed_runtime's
/// service_root), when this service is an installed Service's: XenoNet Dev,
/// or a binary named by XENONET_SERVICE_BIN, isn't started from there, so
/// a restart doesn't move it. Null otherwise.
pub fn restart_exe() -> Value {
    use xenonet_components::{install, packages, runtime};
    let env = fx::env_map();
    let home = xenonet_os::home_dir(&|k| std::env::var(k).ok());
    let m = runtime::Machine { platform: xenonet_os::platform().into(), arch: String::new(), home };
    let dir = xenonet_os::paths::join(&packages::components_dir(&env, &m), "service");
    let (Some(me), Ok(dir)) = (xenonet_os::fs::running_exe(), xenonet_os::fs::realpath_native(&dir)) else { return Value::Null };
    if !std::path::Path::new(&me).starts_with(&dir) {
        return Value::Null;
    }
    let Ok(Some(service)) = packages::current_component("service", &env, &m) else { return Value::Null };
    if !install::has_binary(&service, &m.platform) {
        return Value::Null;
    }
    let (root, _) = install::service_root(&service, &env, &m);
    let exe = xenonet_os::paths::join_all(&[&root, "bin", runtime::bin_name(&m.platform)]);
    xenonet_os::fs::realpath_native(&exe).map_or(Value::Null, Value::String)
}

/// The process lookups' self-test (core/procs.js's selfTest): whether a
/// connection this process makes to itself comes back as this process,
/// with the processes above it and the program it runs.
fn procs_self_test() -> Value {
    struct Run;
    impl xenonet_engine::procs::Io for Run {
        fn exec(&mut self, call: xenonet_engine::procs::Call) -> Result<String, xenonet_engine::procs::Failure> {
            let mut cmd = std::process::Command::new(&call.file);
            xenonet_os::process::Background::no_window(&mut cmd);
            // Start times in English and UTC, and text as UTF-8.
            cmd.args(&call.args)
                .env("LC_TIME", "C")
                .env("LC_CTYPE", "UTF-8")
                .env("TZ", "UTC")
                .env_remove("LC_ALL")
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let out = cmd.output().map_err(|e| xenonet_engine::procs::Failure { message: e.to_string(), stdout: None })?;
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            if out.status.success() {
                Ok(stdout)
            } else {
                Err(xenonet_engine::procs::Failure { message: format!("Command failed: {} {}", call.file, call.args.join(" ")), stdout: Some(stdout) })
            }
        }
        fn now(&mut self) -> f64 {
            now_ms()
        }
    }
    let p = xenonet_engine::procs::Procs::new(xenonet_os::platform());
    if !p.supported {
        return json!({ "supported": false });
    }
    let none = json!({ "supported": true, "peer": false, "chain": false, "exe": false });
    let Ok(server) = std::net::TcpListener::bind("127.0.0.1:0") else { return none };
    let Ok(addr) = server.local_addr() else { return none };
    let Ok(client) = std::net::TcpStream::connect(addr) else { return none };
    let accepted = server.accept().ok();
    let me = f64::from(std::process::id());
    let (pid, first) = match &accepted {
        Some((socket, _)) => {
            let local = socket.local_addr().map_or(Value::Null, |a| json!(a.port()));
            let remote = socket.peer_addr().map_or(Value::Null, |a| json!(a.port()));
            let (pid, _) = p.peer(&mut Run, Some(&local), Some(&remote), None);
            let pid = pid.as_f64();
            let first = pid.and_then(|pid| p.chain(&mut Run, Some(pid), true, true).ok()).and_then(|c| c.into_iter().next());
            (pid, first)
        }
        None => (None, None),
    };
    drop(client);
    let exe = first.as_ref().map(|l| l.exe.clone().flatten().map_or(Value::Null, Value::String));
    let (peer, chain, exe) = xenonet_engine::procs::self_test(me, pid, first.as_ref().map(|l| (l.row.pid, exe.as_ref())));
    json!({ "supported": true, "peer": peer, "chain": chain, "exe": exe })
}

/// One run's machine: this one's files and programs, and XenoNet's own
/// state through the other cores.
struct Machine {
    parts: Arc<Parts>,
    paths: Value,
}

fn failure(e: CoreError) -> Failure {
    Failure::msg(e.message)
}

impl Machine {
    fn path(&self, k: &str) -> Option<String> {
        self.paths.get(k).filter(|v| xenonet_text::text::truthy(Some(v))).map(xenonet_text::text::display)
    }
}

/// The folders of the user's saved Path, read from the registry on
/// Windows; null elsewhere.
async fn saved_path() -> Value {
    if xenonet_os::platform() != "win32" {
        return Value::Null;
    }
    let env = fx::env_map();
    let reg = xenonet_components::command::reg_exe(&env);
    let (mut read, mut dirs) = (false, Vec::new());
    for key in xenonet_components::command::PATH_KEYS {
        let r = fx::exec_file(&reg, &xenonet_components::command::reg_query(key), 5000, RunOptions::default()).await;
        if !r.ok() {
            continue;
        }
        read = true;
        dirs.extend(xenonet_components::command::reg_path(&r.stdout, &env));
    }
    if read {
        json!(dirs)
    } else {
        Value::Null
    }
}

/// The machine as the checks ask it. A check runs on a thread of its own
/// (fx::local); what it asks that waits runs on the service's runtime, as
/// the other cores do.
struct Here {
    m: Arc<Machine>,
    rt: tokio::runtime::Handle,
}

impl Here {
    async fn on<T: Send + 'static>(&self, f: impl std::future::Future<Output = T> + Send + 'static, or: T) -> T {
        self.rt.spawn(f).await.unwrap_or(or)
    }
}

#[async_trait(?Send)]
impl DoctorHost for Here {
    async fn now(&self) -> f64 {
        now_ms()
    }
    async fn local_time(&self, ms: f64) -> String {
        xenonet_os::clock::local_time(ms)
    }
    async fn which(&self, cmd: &str) -> Option<String> {
        fx::on_path(cmd)
    }
    async fn exists(&self, path: &str) -> bool {
        !path.is_empty() && std::fs::metadata(path).is_ok()
    }
    async fn run(&self, file: &str, args: &[&str], ms: f64) -> Value {
        let (file, args): (String, Vec<String>) = (file.into(), args.iter().map(ToString::to_string).collect());
        self.on(async move { run_program(&file, &args, ms).await }, Value::Null).await
    }
    async fn read_text(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
    async fn readdir(&self, path: &str) -> Result<Vec<String>, Failure> {
        xenonet_os::paths::read_dir_sorted(std::path::Path::new(path))
            .map(|v| v.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .map_err(|e| xenonet_os::fs::FsError::of(e, "scandir", path).into())
    }
    async fn writable(&self, dir: &str) -> bool {
        writable(dir)
    }
    async fn statfs(&self, dir: &str) -> Option<Value> {
        statfs(dir)
    }
    async fn store(&self, dir: &str) -> Option<String> {
        (self.m.parts.standing)(dir)
    }
    async fn command(&self) -> Value {
        let dir = xenonet_components::command::command_dir(&self.m.parts.home(), xenonet_os::platform(), fx::env_set("LOCALAPPDATA").as_deref());
        let st = xenonet_components::command::command_state(Some(&dir), std::env::var("PATH").ok().as_deref(), xenonet_os::platform());
        json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
    }
    async fn saved_path(&self) -> Value {
        self.on(saved_path(), Value::Null).await
    }
    async fn procs(&self) -> Value {
        let unsupported = || json!({ "supported": false });
        self.on(async move { tokio::task::spawn_blocking(procs_self_test).await.unwrap_or_else(|_| unsupported()) }, unsupported()).await
    }
    async fn git_available(&self) -> bool {
        self.on(git_available(), false).await
    }
    async fn launcher(&self) -> Result<Value, Failure> {
        self.on(async { launcher().await.map_err(|e| Failure::from_value(&e)) }, Ok(Value::Null)).await
    }
    async fn errors(&self) -> Value {
        match self.m.path("stateDir") {
            Some(dir) => Value::Array(xenonet_os::log::read_logs(std::path::Path::new(&xenonet_os::log::log_dir(&dir)), now_ms() - DAY_MS, "error", None, None, 500)),
            None => Value::Null,
        }
    }
    async fn drift(&self) -> Value {
        let (file, looked, records) = self.m.parts.drift.locked().view();
        json!({ "file": file, "lookedAt": looked.map_or(Value::Null, xenonet_text::json::number), "records": records })
    }
    // How the service started at login stands (the supervisor's review): it
    // asks launchctl, systemctl or schtasks, so off the runtime's threads.
    async fn login(&self) -> Option<Value> {
        Some(self.on(async { tokio::task::spawn_blocking(xenonet_components::supervisor::review_here).await.unwrap_or(Value::Null) }, Value::Null).await)
    }
    // The network's gateway, for the firewall's check: whether it's this
    // machine, and its address.
    async fn network(&self) -> Option<Value> {
        Some(self.m.parts.network().map_or(Value::Null, |d| xenonet_net::doctor::standing(&d)))
    }
    async fn restart_exe(&self) -> Option<Value> {
        Some(restart_exe())
    }
    async fn realpath(&self, path: &str) -> Option<Value> {
        Some(xenonet_os::fs::realpath_native(path).map_or(Value::Null, Value::String))
    }
    async fn verify(&self) -> Option<Value> {
        Some(self.m.parts.cores.verify.state())
    }
    async fn actors_expired(&self) -> Result<Option<Value>, Failure> {
        if self.m.parts.cores.actors.is_none() {
            return Ok(None);
        }
        self.m.parts.cores.engine_store.expired_actor_tokens().map(Some).map_err(failure)
    }
    async fn actors_observers(&self) -> Option<Value> {
        self.m.parts.cores.actors.as_ref().map(|a| a.observers_status())
    }
    async fn actors_token_files(&self, fresh: bool) -> Result<Option<Value>, Failure> {
        let Some(actors) = self.m.parts.cores.actors.clone() else { return Ok(None) };
        self.on(async move { actors.token_files(fresh).await.map(Some).map_err(failure) }, Ok(None)).await
    }
    async fn actors_gateway(&self) -> Result<Option<Value>, Failure> {
        if self.m.parts.cores.actors.is_none() {
            return Ok(None);
        }
        let Some(g) = self.m.parts.gateway.get().cloned() else { return Ok(None) };
        self.on(async move { g.status().await.map(Some).map_err(failure) }, Ok(None)).await
    }
    async fn actors_wakes(&self) -> Result<Option<Value>, Failure> {
        if self.m.parts.cores.actors.is_none() {
            return Ok(None);
        }
        let Some(w) = self.m.parts.cores.wakes.clone() else { return Ok(None) };
        w.list().map(Some).map_err(failure)
    }
}

/// What the checks read of the service as data (factsOf): of the live
/// state only the apps' plan readings (the registry's `limits`), and of
/// the health its sources and parts.
async fn facts(parts: &Parts, by_hand: bool) -> Value {
    let c = &parts.cores;
    let health = match (c.state.sources(), c.state.blocks()) {
        (Ok(sources), Ok(blocks)) => json!({ "sources": sources, "blocks": blocks }),
        _ => Value::Null,
    };
    let live = c.state.live_snapshot().ok().map_or(Value::Null, |l| {
        let limits: Map<String, Value> = xenonet_adapter::apps::apps()
            .iter()
            .filter_map(|a| a.limits.as_deref())
            .map(|k| (k.to_string(), l.get("limits").and_then(|x| x.get(k)).cloned().unwrap_or(Value::Null)))
            .collect();
        json!({ "limits": limits })
    });
    let connect = if parts.config.demo { Value::Null } else { c.connect.preview().await.unwrap_or(Value::Null) };
    let mut facts = json!({
        "self": { "port": parts.config.port, "name": parts.config.name, "exe": xenonet_os::fs::running_exe() },
        "base": null,
        "baseName": null,
        "health": health,
        "live": live,
        "connect": connect,
        "paths": parts.paths,
        "home": parts.home(),
        "platform": xenonet_os::platform(),
        "electron": null,
        "node": null,
    });
    if let Value::Object(m) = &mut facts {
        m.extend(adapter_facts(&parts.home()).into_iter().map(|(k, v)| (k.to_string(), v)));
        m.extend(
            json!({
                "byHand": by_hand,
                "projectsDirSet": fx::env_set("XENONET_PROJECTS_DIR").is_some(),
                "grokKeySet": fx::env_set("XAI_API_KEY").is_some(),
                "actors": if c.actors.is_some() { "on" } else { "off" },
            })
            .as_object()
            .cloned()
            .unwrap_or_default(),
        );
    }
    facts
}

/// The runtime check, for a binary that needs no Node: it is the runtime.
fn runtime_check(version: &str) -> Value {
    json!({ "status": "ok", "detail": format!("the xenonet binary {version}, which needs no Node") })
}

/// One check's verdict, asking this machine.
async fn one(id: String, facts: Value, m: Arc<Machine>) -> Value {
    if id == "system.runtime" {
        return runtime_check(&m.parts.version);
    }
    let f = checks::Facts { data: facts };
    let rt = tokio::runtime::Handle::current();
    let ran = fx::local(move || async move {
        let here = Here { m, rt };
        checks::run(&here, id, f).await
    })
    .await;
    match ran {
        Some(Ok(v)) => v,
        Some(Err(e)) => json!({ "status": "warn", "detail": format!("the check failed: {}", e.message) }),
        None => json!({ "status": "warn", "detail": "the check failed: it stopped before it was done" }),
    }
}

/// runDoctor(ctx, { only }): every check (or those `only` keeps), each in
/// 8 seconds at most, and their summary.
pub async fn run_doctor(parts: Arc<Parts>, only: Option<&[String]>, by_hand: bool) -> Value {
    let started = now_ms();
    // Each check: its id, group, title, and its verdict when it's ready.
    let mut all: Vec<(String, String, String, Option<Value>)> = checks::all().into_iter().map(|(id, g, t)| (id.to_string(), g.to_string(), t.to_string(), None)).collect();
    // The network across machines, as this machine keeps it, when it's in
    // one: each verdict is ready. How this machine asks the person and
    // keeps their root key, once an identity was set up here.
    let checks: Vec<_> = parts.network().map_or_else(Vec::new, |d| xenonet_net::doctor::checks(&d, now_ms()).into_iter().chain(xenonet_net::doctor::identity_checks(&d)).collect());
    for (id, title, verdict) in checks {
        all.push((id.into(), "Network".into(), title.into(), Some(json!({ "network": verdict }))));
    }
    // Where the machine signing key is, once it's made: a key in the TPM
    // or the Secure Enclave that doesn't open is a failure here, since
    // XenoNet never makes a new one in its place.
    if let Some(verdict) = (parts.machine_key)(&std::path::Path::new(&parts.config.state_dir).join("identity")) {
        all.push(("network.signing".into(), "Network".into(), "This machine's signing key is where it was made".into(), Some(json!({ "network": verdict }))));
    }
    let picked: Vec<_> = all.into_iter().filter(|(id, ..)| only.is_none_or(|o| run::kept(id, o))).collect();
    let facts = facts(&parts, by_hand).await;
    let m = Arc::new(Machine { paths: parts.paths.clone(), parts: parts.clone() });
    let runs = picked.into_iter().map(|(id, group, title, ready)| {
        let (facts, m) = (facts.clone(), m.clone());
        async move {
            let t0 = now_ms();
            let check = async {
                match &ready {
                    Some(k) => k["network"].clone(),
                    None => one(id.clone(), facts, m).await,
                }
            };
            let r = tokio::time::timeout(Duration::from_millis(CHECK_MS), check)
                .await
                .unwrap_or_else(|_| json!({ "status": "warn", "detail": format!("didn't finish in {} seconds", CHECK_MS / 1000) }));
            let mut out = Map::new();
            out.insert("id".into(), json!(id));
            out.insert("group".into(), json!(group));
            out.insert("title".into(), json!(title));
            for (k, v) in r.as_object().into_iter().flatten() {
                out.insert(k.clone(), v.clone());
            }
            out.insert("ms".into(), xenonet_text::json::number((now_ms() - t0).max(0.0)));
            Value::Object(out)
        }
    });
    let results: Vec<Value> = join_all(runs.collect()).await;
    json!({ "at": xenonet_text::json::number(started), "ms": xenonet_text::json::number(now_ms() - started), "results": results, "summary": run::summarize(&results) })
}

// Promise.all: each started at once, the answers in order.
async fn join_all<F: std::future::Future<Output = Value> + Send + 'static>(futures: Vec<F>) -> Vec<Value> {
    let handles: Vec<_> = futures.into_iter().map(tokio::spawn).collect();
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await.unwrap_or_else(|e| json!({ "status": "warn", "detail": format!("the check failed: {e}") })));
    }
    out
}

// ---------- the service ----------

struct Running {
    id: u64,
    epoch: u64,
    by_hand: bool,
    done: watch::Receiver<Option<Value>>,
}

#[derive(Default)]
struct St {
    last: Option<Value>,
    running: Option<Running>,
    epoch: u64,
    next: u64,
}

pub struct DoctorService {
    parts: Arc<Parts>,
    only: Option<Vec<String>>,
    file: Option<String>,
    before: Value,
    st: Arc<Mutex<St>>,
    timer: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

async fn finished(mut rx: watch::Receiver<Option<Value>>) -> Value {
    loop {
        if let Some(v) = rx.borrow().clone() {
            return v;
        }
        if rx.changed().await.is_err() {
            return rx.borrow().clone().unwrap_or(Value::Null);
        }
    }
}

impl DoctorService {
    pub fn new(parts: Arc<Parts>) -> Arc<Self> {
        let demo = parts.config.demo;
        let file = (!demo).then(|| std::path::Path::new(&parts.config.state_dir).join("doctor.json").to_string_lossy().into_owned());
        let before = run::read_run(file.as_deref()).unwrap_or(Value::Null);
        Arc::new(DoctorService {
            only: demo.then(|| DEMO_ONLY.iter().map(ToString::to_string).collect()),
            file,
            before,
            parts,
            st: Arc::new(Mutex::new(St::default())),
            timer: Mutex::new(None),
        })
    }

    /// One run at a time; a request during a run gets that run's result,
    /// unless a result was reported or a fresh run asked for meanwhile, or
    /// a run you asked for comes during a scheduled one.
    pub async fn run(self: &Arc<Self>, fresh: bool, by_hand: bool) -> Value {
        // The run this request gets: one under way, or a new one.
        let joined = {
            let mut s = self.st.locked();
            if fresh {
                s.epoch += 1;
            }
            match &s.running {
                Some(r) if r.epoch == s.epoch && (r.by_hand || !by_hand) => Err(r.done.clone()),
                _ => {
                    let previous = s.running.as_ref().map(|r| r.done.clone());
                    s.next += 1;
                    let (id, epoch) = (s.next, s.epoch);
                    let (tx, rx) = watch::channel(None);
                    s.running = Some(Running { id, epoch, by_hand, done: rx });
                    Ok((tx, previous, id, epoch))
                }
            }
        };
        let (tx, previous, id, epoch) = match joined {
            Ok(new) => new,
            Err(under_way) => return finished(under_way).await,
        };
        let me = self.clone();
        let done = tx;
        let waiting = done.subscribe();
        tokio::spawn(async move {
            if let Some(p) = previous {
                finished(p).await;
            }
            let result = run_doctor(me.parts.clone(), me.only.as_deref(), by_hand).await;
            let found = {
                let mut s = me.st.locked();
                let found = if epoch == s.epoch {
                    let prev = s.last.clone().unwrap_or_else(|| me.before.clone());
                    let found = run::new_problems(&prev, &result);
                    s.last = Some(result.clone());
                    Some(found)
                } else {
                    None
                };
                if s.running.as_ref().is_some_and(|r| r.id == id) {
                    s.running = None;
                }
                found
            };
            if let Some(found) = found {
                run::write_run(me.file.as_deref(), &result);
                me.parts.emitter.emit(From::Doctor, "result", result.clone());
                if !found.is_empty() {
                    me.parts.emitter.emit(From::Doctor, "problems", Value::Array(found));
                }
            }
            let _ = done.send(Some(result));
        });
        finished(waiting).await
    }
}

/// The Doctor trait, for the routes.
pub struct DoctorCore(pub Arc<DoctorService>);

#[async_trait]
impl Doctor for DoctorCore {
    fn last(&self) -> Option<Value> {
        self.0.st.locked().last.clone()
    }

    async fn check(&self, fresh: bool, by_hand: bool) -> R<Value> {
        Ok(self.0.run(fresh, by_hand).await)
    }

    fn report(&self, result: Value) -> R<()> {
        {
            let mut s = self.0.st.locked();
            s.epoch += 1;
            s.last = Some(result.clone());
        }
        self.0.parts.emitter.emit(From::Doctor, "result", result);
        Ok(())
    }

    fn summarize(&self, results: &Value) -> R<Value> {
        Ok(run::summarize(results.as_array().map_or(&[][..], Vec::as_slice)))
    }

    fn start(&self) {
        let me = self.0.clone();
        let t = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(FIRST_AFTER_MS)).await;
            loop {
                let m = me.clone();
                tokio::spawn(async move {
                    m.run(false, false).await;
                });
                tokio::time::sleep(Duration::from_millis(EVERY_MS)).await;
            }
        });
        if let Some(old) = self.0.timer.locked().replace(t) {
            old.abort();
        }
    }

    fn stop(&self) {
        if let Some(t) = self.0.timer.locked().take() {
            t.abort();
        }
    }
}

/// What XenoNet can tell someone helping you, safe to share
/// (core/diagnostics.js): scrubbed as it's made, refused if anything
/// private is left; and the folder and command for asking an agent on this
/// computer about it.
pub struct Diagnose {
    pub parts: Arc<Parts>,
    pub doctor: Arc<DoctorService>,
}

/// os.release().
#[cfg_attr(unix, expect(unsafe_code, reason = "uname(3), which std doesn't wrap"))]
fn release() -> String {
    #[cfg(unix)]
    {
        // SAFETY: utsname is a plain C struct, for which all zeros is a
        // valid value.
        let mut u: libc::utsname = unsafe { std::mem::zeroed() };
        // SAFETY: uname only fills `u`, which is alive for the call.
        if unsafe { libc::uname(&raw mut u) } == 0 {
            // SAFETY: release is a NUL-terminated string uname wrote.
            return unsafe { std::ffi::CStr::from_ptr(u.release.as_ptr()) }.to_string_lossy().into_owned();
        }
    }
    String::new()
}

#[async_trait]
impl Diagnostics for Diagnose {
    async fn bundle(&self) -> R<Value> {
        let p = &self.parts;
        let c = &p.config;
        // Installed XenoNet's service is told of the launcher's binary, and,
        // started by an earlier XenoNet, of its node.
        let packaged = c.launcher_exec.is_some() || fx::env_set("XENONET_LAUNCHER_BIN").is_some();
        let xenonet = json!({ "version": p.version, "instance": c.instance, "packaged": packaged, "running": true });
        let doctor = self.doctor.st.locked().last.clone();
        let errors =
            if c.demo { Vec::new() } else { xenonet_os::log::read_logs(std::path::Path::new(&xenonet_os::log::log_dir(&c.state_dir)), now_ms() - DAY_MS, "warn", None, None, 500) };
        let connect = if c.demo { None } else { p.cores.connect.preview().await.ok() };
        let launcher = launcher().await.ok().filter(|l| !l.is_null());
        let env = fx::env_map();
        let arch = xenonet_os::system::os::arch();
        let rel = release();
        let g = crate::diagnostics::Gathered {
            xenonet: Some(&xenonet),
            doctor: doctor.as_ref(),
            errors: &errors,
            connect: connect.as_ref(),
            launcher: launcher.as_ref(),
            env: &env,
            home: &c.home,
            now: now_ms(),
            platform: xenonet_os::platform(),
            arch,
            release: &rel,
            // The binary is its own runtime.
            node: "none",
            electron: None,
        };
        crate::diagnostics::make_bundle(&g).map_err(CoreError::new)
    }

    fn markdown(&self, bundle: &Value) -> R<String> {
        Ok(crate::diagnostics::bundle_markdown(bundle))
    }

    fn write_look(&self, bundle: &Value) -> R<String> {
        let root = self.parts.config.root.to_string_lossy().into_owned();
        crate::diagnostics::write_look(&self.parts.config.state_dir, bundle, &[root], now_ms()).map_err(|e| CoreError::new(e.message))
    }

    fn ask_command(&self, app: &str, model: Option<&str>, dir: &str) -> R<Value> {
        let root = self.parts.config.root.to_string_lossy().into_owned();
        Ok(crate::diagnostics::ask_command(app, model, &json!(dir), &[root]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(demo: bool, state: &str) -> Arc<Parts> {
        // The service's settings for HOME=/home/nova, with the state folder given.
        let config = Settings {
            demo,
            port: if demo { 47841 } else { 47840 },
            name: if demo { "XenoNet (demo)" } else { "XenoNet" }.into(),
            instance: demo.then(|| "demo".into()),
            launcher_exec: None,
            home: "/home/nova".into(),
            state_dir: state.into(),
            root: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap().to_path_buf(),
        };
        let paths = json!({ "stateDir": state, "cacheDir": format!("{state}/cache"), "projectsDir": state, "claudeProjectsDir": "/nowhere/claude", "codexHome": "/nowhere/codex", "homeDir": "/home/nova" });
        Arc::new(Parts {
            cores: Cores::pending(false, false),
            config,
            paths,
            version: "0.0.0".into(),
            drift: Arc::new(Mutex::new(DriftLog::default())),
            emitter: Emitter::new(),
            gateway: Slot::default(),
            standing: |_| None,
            machine_key: |_| None,
            net: None,
        })
    }

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-doctor-runtime-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    // test/doctor.test.js's demo case: the demo checks only itself, and the
    // checks that need the map say so while it isn't there.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_demo_checks_only_itself() {
        let d = DoctorService::new(parts(true, &tmp("demo")));
        let r = d.run(false, false).await;
        let ids: Vec<String> = r["results"].as_array().unwrap().iter().map(|x| x.get("id").map(xenonet_text::text::display).unwrap_or_default()).collect();
        assert!(ids.iter().all(|id| DEMO_ONLY.iter().any(|o| run::kept(id, &[o.to_string()]))), "{ids:?}");
        let server = r["results"].as_array().unwrap().iter().find(|x| x["id"] == "xenonet.server").unwrap();
        assert_eq!(server["status"], "ok");
        assert_eq!(r["results"].as_array().unwrap().iter().find(|x| x["id"] == "system.runtime").unwrap()["status"], "ok");
        assert_eq!(
            r["summary"]["ok"].as_u64().unwrap()
                + r["summary"]["skip"].as_u64().unwrap()
                + r["summary"]["info"].as_u64().unwrap()
                + r["summary"]["warn"].as_u64().unwrap()
                + r["summary"]["fail"].as_u64().unwrap(),
            ids.len() as u64
        );
        let core = DoctorCore(d.clone());
        assert_eq!(core.last().unwrap()["at"], r["at"]);
        // A run reported from elsewhere (the doctor simulation) is the last.
        core.report(json!({ "at": 1, "results": [], "summary": {} })).unwrap();
        assert_eq!(core.last().unwrap()["at"], 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn one_run_at_a_time_and_new_problems_once() {
        let state = tmp("real");
        let d = DoctorService::new(parts(false, &state));
        let (a, b) = tokio::join!(d.run(false, false), d.run(false, false));
        assert_eq!(a["at"], b["at"], "a request during a run gets that run");
        let c = d.run(true, false).await;
        assert_ne!(c["at"], a["at"], "a fresh one runs again");
        assert!(std::path::Path::new(&state).join("doctor.json").exists(), "the last run is kept");
    }

    // A store this build doesn't read makes the state check say to set the
    // machine up again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_store_to_set_up_again_fails_the_state_check() {
        let state = tmp("set-up-again");
        let mut p = Arc::into_inner(parts(false, &state)).unwrap();
        p.standing = |dir| Some(format!("{dir}/engine.sqlite is from an earlier XenoNet"));
        let d = DoctorService::new(Arc::new(p));
        let r = d.run(false, false).await;
        let check = r["results"].as_array().unwrap().iter().find(|x| x["id"] == "xenonet.state").unwrap().clone();
        assert_eq!(check["status"], "fail");
        assert!(check["fix"].as_str().unwrap().starts_with("Set this machine up again"), "{check}");
    }

    // The machine signing key's row shows once the key is made, from the
    // identity folder, and a key that doesn't open fails it.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_machine_key_has_its_row_once_made() {
        let state = tmp("machine-key");
        let row = |r: &Value| r["results"].as_array().unwrap().iter().find(|x| x["id"] == "network.signing").cloned();
        assert_eq!(row(&DoctorService::new(parts(false, &state)).run(false, false).await), None, "no key yet");
        let mut p = Arc::into_inner(parts(false, &state)).unwrap();
        p.machine_key = |dir| dir.ends_with("identity").then(|| json!({ "status": "fail", "detail": "the TPM doesn't have it any more" }));
        let r = DoctorService::new(Arc::new(p)).run(false, false).await;
        let check = row(&r).unwrap();
        assert_eq!((check["status"].as_str(), check["group"].as_str()), (Some("fail"), Some("Network")), "{check}");
    }

    #[test]
    fn local_times() {
        let t = xenonet_os::clock::local_time(0.0);
        assert!(t.ends_with("AM") || t.ends_with("PM"), "{t}");
        assert_eq!(xenonet_os::clock::local_time(f64::NAN), "Invalid Date");
    }
}
