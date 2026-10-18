//! core/components/ for the command (xenonet setup, service and update):
//! installing and switching components, the service's lifecycle and the
//! updates (xenonet-components' tasks), with what they wait on done here,
//! as the twins' runtime.js, install.js and updates.js did it: the
//! component service's socket (JSON lines, a hello with the descriptor's
//! token), starting the service detached, a staged runtime's startup
//! check, the install lock's look at a process, and the release's
//! downloads.

use super::tasks::{block_on, coded, drive, error, fetch, Failed, Perform};
use crate::out::Fail;
use crate::sys;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use xenonet_app::task::{Reply, Task};
use xenonet_components::install::{self, Ctx, Life};
use xenonet_components::runtime::{self, Env, Machine};
use xenonet_components::{protocol, updates};
use xenonet_service::rpc::{Handler, Rpc, MAX_LINE};

/// How long a service gets to stop during an update (lifecycle.js's
/// stopTimeout), and to start (ensureService's timeout).
const STOP_MS: f64 = 15000.0;
const START_MS: f64 = 60000.0;

/// The machine's own: { platform, arch, home }.
pub fn machine() -> Machine {
    Machine { platform: sys::platform().into(), arch: super::arch().into(), home: sys::home() }
}

/// install.js's ctx: the machine, the launcher script this XenoNet ships
/// (its links followed) and process.env's LOCALAPPDATA.
fn ctx() -> Ctx {
    let source = super::launcher_source();
    let source = std::fs::canonicalize(&source).map_or(source, |p| p.to_string_lossy().into_owned());
    Ctx { machine: machine(), launcher_source: source, local_app_data: std::env::var("LOCALAPPDATA").ok() }
}

/// process.env with what the command was given over it.
fn env_with(given: &Value) -> Env {
    let mut env = super::env_map();
    for (k, v) in given.as_object().into_iter().flatten() {
        env.insert(k.clone(), v.clone());
    }
    env
}

fn failed(f: Failed) -> Fail {
    Fail::error(f.message)
}

fn task_fail(f: xenonet_app::task::Failure) -> Fail {
    Fail::error(f.message().to_string())
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
pub async fn connect_service(env: &Env, role: &str) -> Result<Client, Value> {
    let (port, token) = runtime::read_descriptor(env, &machine()).map_err(|e| {
        let mut m = Map::new();
        m.insert("message".into(), json!(e.message));
        if let Some(c) = e.code {
            m.insert("code".into(), json!(c));
        }
        Value::Object(m)
    })?;
    let socket = tokio::net::TcpStream::connect(("127.0.0.1", port as u16)).await.map_err(|e| {
        let code = xenonet_base::fs::code_of(&e).unwrap_or("ECONNREFUSED");
        coded(format!("connect {code} 127.0.0.1:{port}"), code)
    })?;
    // The service asks nothing of the command.
    let handle: Handler = Arc::new(|_, _| Box::pin(async { Err("unsupported request".to_string()) }));
    let (rpc, _events) = Rpc::new(socket, handle, &[], MAX_LINE);
    let info = match rpc.request("hello", runtime::hello(&token, role), Duration::from_millis(5000)).await {
        Ok(i) => i,
        Err(m) => {
            rpc.close();
            return Err(error(m));
        }
    };
    if let Err(m) = protocol::negotiate(Some(&protocol::api_value(protocol::SERVICE_API)), info.get("api")) {
        rpc.close();
        return Err(error(m));
    }
    Ok(Client { rpc, info })
}

// ---------- processes ----------

/// process.kill(pid, 0): Ok when it's there, ESRCH (or EPERM) when not.
fn alive(pid: f64) -> Reply {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with signal 0 sends nothing; it only checks the
        // process.
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return Ok(json!(true));
        }
        let e = std::io::Error::last_os_error();
        let code = if e.raw_os_error() == Some(libc::EPERM) { "EPERM" } else { "ESRCH" };
        Err(coded(format!("kill {code}"), code))
    }
    #[cfg(not(unix))]
    {
        let r = super::native::run("tasklist", &["/FI".into(), format!("PID eq {pid}"), "/NH".into()], 5000.0, None);
        if r.stdout.split_whitespace().any(|w| w == pid.to_string()) {
            Ok(json!(true))
        } else {
            Err(coded("kill ESRCH", "ESRCH"))
        }
    }
}

/// Starts a program detached, its output appended to `log`, as the
/// service is started: it goes on when the command ends.
fn spawn_detached(q: &Value) -> Result<std::process::Child, Value> {
    let text = |k: &str| q.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    let log = q.get("log").or_else(|| q.get("logFile")).map(xenonet_base::text::js_string).unwrap_or_default();
    let mut open = std::fs::OpenOptions::new();
    open.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let out = open.open(&log).map_err(|e| error(xenonet_base::fs::FsError::of(e, "open", &log).message))?;
    let err = out.try_clone().map_err(|e| error(e.to_string()))?;
    let mut cmd = std::process::Command::new(text("command"));
    cmd.args(q.get("args").and_then(Value::as_array).into_iter().flatten().map(xenonet_base::text::js_string)).stdin(std::process::Stdio::null()).stdout(out).stderr(err);
    if let Some(cwd) = q.get("cwd").and_then(Value::as_str) {
        cmd.current_dir(cwd);
    }
    if let Some(env) = q.get("env").and_then(Value::as_object) {
        cmd.env_clear();
        for (k, v) in env {
            cmd.env(k, xenonet_base::text::js_string(v));
        }
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
        let code = xenonet_base::fs::code_of(&e).unwrap_or("ENOENT");
        coded(format!("spawn {} {code}", text("command")), code)
    })
}

// ---------- the tasks' requests ----------

/// ensureService's: connect (and close again), spawn, and the failure of
/// the child it started, if it failed.
struct Ensure {
    child: Option<std::process::Child>,
}

impl Perform for Ensure {
    async fn ask(&mut self, q: &Value) -> Reply {
        match q.get("op").and_then(Value::as_str) {
            Some("connect") => {
                let env: Env = q.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
                connect_service(&env, "tools").await?.close();
                Ok(Value::Null)
            }
            Some("spawn") => {
                self.child = Some(spawn_detached(q)?);
                Ok(Value::Null)
            }
            Some("failure") => match self.child.as_mut().map(std::process::Child::try_wait) {
                Some(Ok(Some(st))) => Err(error(format!("XenoNet Service exited ({}). See its service log.", st.code().map_or_else(|| "null".into(), |c| c.to_string())))),
                Some(Err(e)) => Err(error(e.to_string())),
                _ => Ok(Value::Null),
            },
            other => Err(error(format!("ensureService asked for {}", other.unwrap_or("undefined")))),
        }
    }
}

/// ensureService(runtime): "reused" or "started".
async fn ensure(runtime: &Value) -> Result<Value, Failed> {
    let root = runtime.get("root").map(xenonet_base::text::js_string).unwrap_or_default();
    let exec = runtime.get("exec").map(xenonet_base::text::js_string).unwrap_or_default();
    let env: Env = runtime.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
    let m = machine();
    let task = Task::new(move |io| async move { runtime::ensure_service(&io, &root, &exec, &env, START_MS, &m).await });
    drive(task, &mut Ensure { child: None }).await
}

/// lifecycle.js's operations, for a service update: the service's
/// connections, starting a runtime, a staged one's startup check, the
/// clock, a pause and the install lock's look at a process.
struct Lifecycle {
    env: Env,
    clients: HashMap<u64, Client>,
    next: u64,
}

impl Lifecycle {
    fn new(env: Env) -> Self {
        Lifecycle { env, clients: HashMap::new(), next: 0 }
    }
}

impl Perform for Lifecycle {
    async fn ask(&mut self, q: &Value) -> Reply {
        let client = || q.get("client").and_then(Value::as_u64).unwrap_or(0);
        match q.get("op").and_then(Value::as_str) {
            Some("connect") => {
                let c = connect_service(&self.env, "tools").await?;
                self.next += 1;
                let info = c.info.clone();
                self.clients.insert(self.next, c);
                Ok(json!({ "client": self.next, "info": info }))
            }
            Some("request") => {
                let Some(c) = self.clients.get(&client()) else { return Err(error("that connection is gone")) };
                c.request(q.get("method").and_then(Value::as_str).unwrap_or(""), q.get("params").cloned().unwrap_or(Value::Null), 150000.0).await.map_err(error)
            }
            Some("close") => {
                if let Some(c) = self.clients.remove(&client()) {
                    c.close();
                }
                Ok(Value::Null)
            }
            Some("start") => ensure(q.get("runtime").unwrap_or(&Value::Null)).await.map_err(|f| {
                let mut m = f.fields;
                m.insert("message".into(), json!(f.message));
                Value::Object(m)
            }),
            Some("check") => install::check_runtime(q.get("staged").unwrap_or(&Value::Null), sys::platform()).map(|()| Value::Null).map_err(|f| error(f.message())),
            Some("now") => Ok(xenonet_base::json::number(super::native::now())),
            Some("pause") => {
                tokio::time::sleep(Duration::from_millis(q.get("ms").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64)).await;
                Ok(Value::Null)
            }
            Some("alive") => alive(q.get("pid").and_then(Value::as_f64).unwrap_or(0.0)),
            other => Err(error(format!("a component update asked for {}", other.unwrap_or("undefined")))),
        }
    }
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        for c in self.clients.values() {
            c.close();
        }
    }
}

/// componentUpdates' fetch of the release's feed, and its body as JSON.
struct Feed {
    bodies: HashMap<u64, Vec<u8>>,
}

impl Perform for Feed {
    async fn ask(&mut self, q: &Value) -> Reply {
        match q.get("op").and_then(Value::as_str) {
            Some("fetch") => {
                let url = q.get("url").map(xenonet_base::text::js_string).unwrap_or_default();
                let res = fetch("GET", &url, &[], None, q.get("timeoutMs").and_then(Value::as_f64)).await?;
                let id = self.bodies.len() as u64 + 1;
                let out = json!({ "ok": res.ok(), "status": res.status, "res": id });
                self.bodies.insert(id, res.body);
                Ok(out)
            }
            Some("json") => {
                let body = self.bodies.get(&q.get("res").and_then(Value::as_u64).unwrap_or(0)).cloned().unwrap_or_default();
                xenonet_base::json::parse(&String::from_utf8_lossy(&body)).map_err(|e| json!({ "message": e.to_string(), "name": "SyntaxError" }))
            }
            other => Err(error(format!("componentUpdates asked for {}", other.unwrap_or("undefined")))),
        }
    }
}

// ---------- what the commands call ----------

/// activatePackage(file, item)
pub fn activate_package(file: &str, item: &Value) -> Result<Value, Fail> {
    let (file, item, env, ctx) = (file.to_string(), item.clone(), super::env_map(), ctx());
    let mut ops = Lifecycle::new(env.clone());
    let task = Task::new(move |io| async move { install::activate_package(&io, &file, &item, &env, &ctx, &Life { stop_timeout: STOP_MS }).await });
    block_on(async { drive(task, &mut ops).await }).map_err(Fail::error)?.map_err(failed)
}

/// installLauncher(): the launcher installed, if one was.
pub fn install_launcher() -> Result<Option<Value>, Fail> {
    install::install_launcher(&super::env_map(), &ctx()).map_err(task_fail)
}

/// recoverServiceUpdate({ env })
pub fn recover_service_update(env: &Value) -> Result<(), Fail> {
    let (env, ctx) = (env_with(env), ctx());
    let mut ops = Lifecycle::new(env.clone());
    let task = Task::new(move |io| async move { install::recover_service_update(&io, &env, &ctx, &Life { stop_timeout: STOP_MS }).await });
    block_on(async { drive(task, &mut ops).await }).map_err(Fail::error)?.map(|_| ()).map_err(failed)
}

/// installedRuntime(env)
pub fn installed_runtime(env: &Value) -> Result<Value, Fail> {
    install::installed_runtime(&env_with(env), &ctx()).map_err(task_fail)
}

/// ensureService(runtime): the service's status.
pub fn ensure_service(runtime: &Value) -> Result<Value, Fail> {
    block_on(ensure(runtime)).map_err(Fail::error)?.map_err(failed)
}

/// connectService({ env }), then client.request(action, body), then close.
pub fn service_request(env: &Value, action: &str, body: &Value) -> Result<Value, Fail> {
    let env = env_with(env);
    block_on(async {
        let c = connect_service(&env, "tools").await.map_err(|e| Fail::error(e.get("message").map(xenonet_base::text::js_string).unwrap_or_default()))?;
        let r = c.request(action, body.clone(), 150000.0).await.map_err(Fail::error);
        c.close();
        r
    })
    .map_err(Fail::error)?
}

/// componentUpdates()
pub fn component_updates() -> Result<Vec<Value>, Fail> {
    let (env, m) = (super::env_map(), machine());
    let task = Task::new(move |io| async move { updates::component_updates(&io, &env, &m).await });
    let list = block_on(async { drive(task, &mut Feed { bodies: HashMap::new() }).await }).map_err(Fail::error)?.map_err(failed)?;
    Ok(list.as_array().cloned().unwrap_or_default())
}

/// downloadComponent(item): the package, from the release's own address,
/// no bigger than it says and kept only when its SHA-256 is the release's.
fn download(item: &Value) -> Result<String, Fail> {
    let (file, have) = updates::download_plan(item, &super::env_map(), &machine()).map_err(super::fs_fail)?;
    if have {
        return Ok(file);
    }
    let url = item.get("url").map(xenonet_base::text::js_string).unwrap_or_default();
    let res = block_on(fetch("GET", &url, &[], None, Some(300000.0))).map_err(Fail::error)?;
    let res = match res {
        Ok(r) if r.ok() => r,
        _ => return Err(Fail::error("XenoNet could not download this component.")),
    };
    let size = item.get("size").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let sha = item.get("sha256").map(xenonet_base::text::js_string).unwrap_or_default();
    let mut d = updates::Download::new(&file, size, &sha).map_err(super::fs_fail)?;
    let done = d.write(&res.body).and_then(|()| d.finish());
    d.discard();
    done.map_err(super::fs_fail)
}

/// updateComponent(item): { component, status: "current", version } for
/// one that isn't newer, else what activating its download came to.
pub fn update_component(item: &Value) -> Result<Value, Fail> {
    if !xenonet_base::text::truthy(item.get("available")) {
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
