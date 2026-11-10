//! The components for the command (xenonet setup, service and update):
//! installing and switching components, the service's lifecycle and the
//! updates (xenonet-components), with what they wait on done here: the
//! component service's socket (JSON lines, a hello with the descriptor's
//! token), starting the service detached, a staged runtime's startup
//! check, the install lock's look at a process, and the release's
//! downloads.

use super::net::{block_on, fetch};
use crate::out::Fail;
use crate::sys;
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use xenonet_components::host::{self, Connected, Got, Spawn};
use xenonet_components::install::{self, Ctx, Life};
use xenonet_components::runtime::{self, Env, Machine};
use xenonet_components::{protocol, updates};
use xenonet_os::error::Failure;
use xenonet_service::rpc::{Handler, Rpc, MAX_LINE};

/// How long a service gets to stop during an update (lifecycle.js's
/// stopTimeout), and to start (ensureService's timeout).
const STOP_MS: f64 = 15000.0;
const START_MS: f64 = 60000.0;

/// The machine's own: { platform, arch, home }.
pub fn machine() -> Machine {
    Machine { platform: sys::platform().into(), arch: super::arch().into(), home: sys::home() }
}

/// install.js's ctx: the machine and process.env's LOCALAPPDATA.
fn ctx() -> Ctx {
    Ctx { machine: machine(), local_app_data: std::env::var("LOCALAPPDATA").ok() }
}

/// process.env with what the command was given over it.
fn env_with(given: &Value) -> Env {
    let mut env = super::env_map();
    for (k, v) in given.as_object().into_iter().flatten() {
        env.insert(k.clone(), v.clone());
    }
    env
}

fn failed(f: Failure) -> Fail {
    Fail::error(f.message)
}

// ---------- the component service's socket ----------

/// A connection to the service, after its hello: what it said of itself.
pub struct Client {
    rpc: Rpc,
    pub info: Value,
}

impl Client {
    pub async fn request(&self, method: &str, params: Value, timeout_ms: f64) -> Result<Value, String> {
        self.rpc.request(method, params, Duration::from_millis(timeout_ms as u64)).await
    }
    pub fn close(&self) {
        self.rpc.close();
    }
}

/// connectService({ env, role }): the service the descriptor in the
/// runtime folder names, its hello answered and its interface one this
/// XenoNet speaks.
pub async fn connect_service(env: &Env, role: &str) -> Result<Client, Failure> {
    let (port, token) = runtime::read_descriptor(env, &machine())?;
    let socket = tokio::net::TcpStream::connect(("127.0.0.1", port as u16)).await.map_err(|e| {
        let code = xenonet_os::fs::code_of(&e).unwrap_or("ECONNREFUSED");
        Failure::coded(format!("connect {code} 127.0.0.1:{port}"), code)
    })?;
    // The service asks nothing of the command.
    let handle: Handler = Arc::new(|_, _| Box::pin(async { Err("unsupported request".to_string()) }));
    let (rpc, _events) = Rpc::new(socket, handle, &[], MAX_LINE);
    let info = match rpc.request("hello", runtime::hello(&token, role), Duration::from_millis(5000)).await {
        Ok(i) => i,
        Err(m) => {
            rpc.close();
            return Err(Failure::msg(m));
        }
    };
    if let Err(m) = protocol::negotiate(Some(&protocol::api_value(protocol::SERVICE_API)), info.get("api")) {
        rpc.close();
        return Err(Failure::msg(m));
    }
    Ok(Client { rpc, info })
}

// ---------- processes ----------

/// Whether a process is there: Ok when it is, ESRCH (or EPERM) when not.
#[cfg_attr(unix, expect(unsafe_code, reason = "kill(2) with signal 0, which std doesn't wrap"))]
fn alive(pid: f64) -> Result<(), Failure> {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with signal 0 sends nothing; it only checks the
        // process.
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        let code = if e.raw_os_error() == Some(libc::EPERM) { "EPERM" } else { "ESRCH" };
        Err(Failure::coded(format!("kill {code}"), code))
    }
    #[cfg(not(unix))]
    {
        let r = super::native::run("tasklist", &["/FI".into(), format!("PID eq {pid}"), "/NH".into()], 5000.0, None);
        if r.stdout.split_whitespace().any(|w| w == pid.to_string()) {
            Ok(())
        } else {
            Err(Failure::coded("kill ESRCH", "ESRCH"))
        }
    }
}

/// Starts a program detached, its output appended to its log, as the
/// service is started: it goes on when the command ends.
fn spawn_detached(q: &Spawn) -> Result<std::process::Child, Failure> {
    let log = q.log.clone().unwrap_or_default();
    let mut open = std::fs::OpenOptions::new();
    open.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let out = open.open(&log).map_err(|e| Failure::msg(xenonet_os::fs::FsError::of(e, "open", &log).message))?;
    let err = out.try_clone().map_err(|e| Failure::msg(e.to_string()))?;
    let mut cmd = std::process::Command::new(&q.command);
    cmd.args(&q.args).stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    if let Some(cwd) = &q.cwd {
        cmd.current_dir(cwd);
    }
    cmd.env_clear();
    for (k, v) in &q.env {
        cmd.env(k, xenonet_text::text::display(v));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A process group of its own, as detached: true gives it.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW.
        cmd.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    cmd.spawn().map_err(|e| {
        let code = xenonet_os::fs::code_of(&e).unwrap_or("ENOENT");
        Failure::coded(format!("spawn {} {code}", q.command), code)
    })
}

async fn pause(ms: f64) {
    tokio::time::sleep(Duration::from_millis(if ms.is_finite() && ms > 0.0 { ms as u64 } else { 0 })).await;
}

// ---------- what the components wait on ----------

/// ensureService's: whether a service answers, spawning one (or the
/// supervisor's start, for a service registered to start at login), and
/// the failure of the child it started, if it failed.
#[derive(Default)]
struct Ensure {
    child: RefCell<Option<std::process::Child>>,
}

#[async_trait(?Send)]
impl host::Starter for Ensure {
    async fn reach(&self, env: &Env) -> Result<(), Failure> {
        connect_service(env, "tools").await?.close();
        Ok(())
    }
    async fn spawn(&self, spawn: &Spawn) -> Result<(), Failure> {
        // A service registered to start at login is started by its
        // supervisor, never beside the copy the supervisor keeps.
        let port = spawn.env.get("XENONET_PORT").map(xenonet_text::text::display).filter(|p| !p.is_empty()).unwrap_or_else(|| "47840".into());
        if let Some(place) = super::supervisor::registered_for(&port) {
            xenonet_components::supervisor::start(&place, &xenonet_components::supervisor::Disk, &mut xenonet_components::supervisor::System).map_err(Failure::msg)?;
            return Ok(());
        }
        *self.child.borrow_mut() = Some(spawn_detached(spawn)?);
        Ok(())
    }
    async fn failed(&self) -> Result<(), Failure> {
        match self.child.borrow_mut().as_mut().map(std::process::Child::try_wait) {
            Some(Ok(Some(st))) => Err(Failure::msg(format!("XenoNet Service exited ({}). See its service log.", st.code().map_or_else(|| "null".into(), |c| c.to_string())))),
            Some(Err(e)) => Err(Failure::msg(e.to_string())),
            _ => Ok(()),
        }
    }
    async fn sleep(&self, ms: f64) {
        pause(ms).await;
    }
}

/// ensureService(runtime): "reused" or "started".
async fn ensure(runtime: &Value) -> Result<Value, Failure> {
    let root = runtime.get("root").map(xenonet_text::text::display).unwrap_or_default();
    let exec = runtime.get("exec").map(xenonet_text::text::display).unwrap_or_default();
    let args = runtime::service_args(runtime.get("args"));
    let env: Env = runtime.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
    runtime::ensure_service(&Ensure::default(), &root, &exec, &args, &env, START_MS, &machine()).await
}

/// The service's connections, by the handle each was given.
#[derive(Default)]
pub struct Clients {
    open: RefCell<HashMap<u64, Rc<Client>>>,
    next: Cell<u64>,
}

impl Clients {
    pub async fn connect(&self, env: &Env) -> Result<Connected, Failure> {
        let c = connect_service(env, "tools").await?;
        let id = self.next.get() + 1;
        self.next.set(id);
        let info = c.info.clone();
        self.open.borrow_mut().insert(id, Rc::new(c));
        Ok(Connected { id, info })
    }
    pub async fn request(&self, client: u64, method: &str, params: Value) -> Result<Value, Failure> {
        let Some(c) = self.open.borrow().get(&client).cloned() else { return Err(Failure::msg("that connection is gone")) };
        c.request(method, params, 150000.0).await.map_err(Failure::msg)
    }
    pub fn close(&self, client: u64) {
        if let Some(c) = self.open.borrow_mut().remove(&client) {
            c.close();
        }
    }
}

impl Drop for Clients {
    fn drop(&mut self) {
        for c in self.open.borrow().values() {
            c.close();
        }
    }
}

/// What a service update waits on: the service's connections, starting a
/// runtime, a staged one's startup check, the clock, a pause and the
/// install lock's look at a process.
struct Updater {
    env: Env,
    clients: Clients,
}

impl Updater {
    fn new(env: Env) -> Self {
        Updater { env, clients: Clients::default() }
    }
}

#[async_trait(?Send)]
impl host::Service for Updater {
    async fn connect(&self, _env: &Env) -> Result<Connected, Failure> {
        self.clients.connect(&self.env).await
    }
    async fn request(&self, client: u64, method: &str, params: Value) -> Result<Value, Failure> {
        self.clients.request(client, method, params).await
    }
    async fn close(&self, client: u64) {
        self.clients.close(client);
    }
    async fn sleep(&self, ms: f64) {
        pause(ms).await;
    }
}

#[async_trait(?Send)]
impl host::Lifecycle for Updater {
    async fn start(&self, runtime: &Value) -> Result<Value, Failure> {
        ensure(runtime).await
    }
    async fn check(&self, staged: &Value) -> Result<(), Failure> {
        install::check_runtime(staged, sys::platform())
    }
    async fn alive(&self, pid: f64) -> Result<(), Failure> {
        alive(pid)
    }
    async fn now(&self) -> f64 {
        super::native::now()
    }
}

/// componentUpdates' fetch of the release's feed.
struct Releases;

#[async_trait(?Send)]
impl host::Feed for Releases {
    async fn get(&self, url: &str, timeout_ms: f64) -> Result<Got, Failure> {
        let res = fetch("GET", url, &[], None, Some(timeout_ms)).await?;
        Ok(Got { status: res.status, body: res.body })
    }
}

// ---------- what the commands call ----------

/// activatePackage(file, item)
pub fn activate_package(file: &str, item: &Value) -> Result<Value, Fail> {
    let (env, ctx) = (super::env_map(), ctx());
    let host = Updater::new(env.clone());
    block_on(install::activate_package(&host, file, item, &env, &ctx, &Life { stop_timeout: STOP_MS })).map_err(Fail::error)?.map_err(failed)
}

/// installBundle(bundle, { command }): what a bundle brings that isn't
/// installed yet, with the launcher and, when `command`, the xenonet
/// command on PATH; the runtime to start the service with.
pub fn install_bundle(bundle: &str, command: bool) -> Result<Value, Fail> {
    let (env, ctx) = (super::env_map(), ctx());
    let host = Updater::new(env.clone());
    block_on(install::install_bundle(&host, bundle, &env, command, &ctx, &Life { stop_timeout: STOP_MS })).map_err(Fail::error)?.map_err(failed)
}

/// currentComponent(component): the installed one, or None; and whether
/// it has the xenonet binary (a Service or Tools from before 0.4.0 has a
/// node and XenoNet's JS instead).
pub fn current_component(component: &str) -> Result<Option<(Value, bool)>, Fail> {
    let m = machine();
    let current = xenonet_components::packages::current_component(component, &super::env_map(), &m).map_err(super::fs_fail)?;
    Ok(current.map(|c| {
        let binary = install::has_binary(&c, &m.platform);
        (c, binary)
    }))
}

/// installLauncher(): the launcher installed, if one was.
pub fn install_launcher() -> Result<Option<Value>, Fail> {
    install::install_launcher(&super::env_map(), &ctx()).map_err(failed)
}

/// recoverServiceUpdate({ env })
pub fn recover_service_update(env: &Value) -> Result<(), Fail> {
    let (env, ctx) = (env_with(env), ctx());
    let host = Updater::new(env.clone());
    block_on(install::recover_service_update(&host, &env, &ctx, &Life { stop_timeout: STOP_MS })).map_err(Fail::error)?.map(|_| ()).map_err(failed)
}

/// installedRuntime(env)
pub fn installed_runtime(env: &Value) -> Result<Value, Fail> {
    install::installed_runtime(&env_with(env), &ctx()).map_err(failed)
}

/// ensureService(runtime): the service's status.
pub fn ensure_service(runtime: &Value) -> Result<Value, Fail> {
    block_on(ensure(runtime)).map_err(Fail::error)?.map_err(failed)
}

/// stopFile(env): what `xenonet service stop` leaves for a supervisor
/// waiting for the port.
pub fn stop_file(env: &Value) -> String {
    runtime::stop_file(&env_with(env), &machine())
}

/// connectService({ env }), then client.request(action, body), then close.
pub fn service_request(env: &Value, action: &str, body: &Value) -> Result<Value, Fail> {
    let env = env_with(env);
    block_on(async {
        let c = connect_service(&env, "tools").await.map_err(failed)?;
        let r = c.request(action, body.clone(), 150000.0).await.map_err(Fail::error);
        c.close();
        r
    })
    .map_err(Fail::error)?
}

/// componentUpdates()
pub fn component_updates() -> Result<Vec<Value>, Fail> {
    let (env, m) = (super::env_map(), machine());
    let list = block_on(updates::component_updates(&Releases, &env, &m)).map_err(Fail::error)?.map_err(failed)?;
    Ok(list.as_array().cloned().unwrap_or_default())
}

/// downloadComponent(item): the package, from the release's own address,
/// no bigger than it says and kept only when its SHA-256 is the release's.
fn download(item: &Value) -> Result<String, Fail> {
    let (file, have) = updates::download_plan(item, &super::env_map(), &machine()).map_err(super::fs_fail)?;
    if have {
        return Ok(file);
    }
    let url = item.get("url").map(xenonet_text::text::display).unwrap_or_default();
    let res = block_on(fetch("GET", &url, &[], None, Some(300000.0))).map_err(Fail::error)?;
    let res = match res {
        Ok(r) if r.ok() => r,
        _ => return Err(Fail::error("XenoNet could not download this component.")),
    };
    let size = item.get("size").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let sha = item.get("sha256").map(xenonet_text::text::display).unwrap_or_default();
    let mut d = updates::Download::new(&file, size, &sha).map_err(super::fs_fail)?;
    let done = d.write(&res.body).and_then(|()| d.finish());
    d.discard();
    done.map_err(super::fs_fail)
}

/// updateComponent(item): { component, status: "current", version } for
/// one that isn't newer, else what activating its download came to.
pub fn update_component(item: &Value) -> Result<Value, Fail> {
    if !xenonet_text::text::truthy(item.get("available")) {
        let mut o = Map::new();
        if let Some(c) = item.get("component") {
            o.insert("component".into(), c.clone());
        }
        o.insert("status".into(), json!("current"));
        if let Some(v) = item.get("current") {
            o.insert("version".into(), v.clone());
        }
        return Ok(Value::Object(o));
    }
    let file = download(item)?;
    activate_package(&file, item)
}
