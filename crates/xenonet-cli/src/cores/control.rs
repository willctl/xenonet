//! core/control.js and core/script.js for the command (xenonet console,
//! script and film): an instance of XenoNet Dev started, driven through its
//! control channel and quit (xenonet-app's control, each method a task),
//! or installed XenoNet's console through the component service
//! (xenonet-components' console); and steps run against either, one after
//! another (xenonet-app's script). What a task waits on (the instance's
//! server, the machine's processes, starting XenoNet Dev) is done here, as
//! the twins' `system` and consoleOps did it.
//!
//! A script that's a JS module is the one thing that runs in Node: its
//! default export is JS, given core/control.js's instance (run_module).

use super::components::connect_service;
use super::tasks::{block_on, coded, drive, error, fetch_asked, Failed, Perform};
use crate::out::Fail;
use crate::sys;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use xenonet_app::control::{self, Instance, Opts};
use xenonet_app::script::{self, Answer, Arg, Then};
use xenonet_app::task::{Reply, Task};
use xenonet_components::console::Console;

/// Who a command drives: an instance of XenoNet Dev from this checkout,
/// or installed XenoNet.
enum Target {
    Dev(Instance),
    Installed(Console),
}

impl Target {
    fn name(&self) -> String {
        match self {
            Target::Dev(i) => i.name.clone(),
            Target::Installed(_) => "XenoNet".into(),
        }
    }
}

/// xenonet(id): installed XenoNet on XENONET_PORT (or 47840), or the
/// instance of XenoNet Dev by that id.
fn target(id: &str) -> Result<Target, Fail> {
    if id == "installed" {
        let mut env = super::env_map();
        let port = sys::var("XENONET_PORT").unwrap_or_else(|| "47840".into());
        env.insert("XENONET_PORT".into(), json!(port));
        return Ok(Target::Installed(Console { env, machine: super::components::machine() }));
    }
    let root = sys::xenonet_root();
    let tmpdir = std::env::temp_dir().to_string_lossy().trim_end_matches(['/', '\\']).to_string();
    let (appdata, xdg, home) = (std::env::var("APPDATA").ok(), std::env::var("XDG_CONFIG_HOME").ok(), sys::home());
    let o = Opts {
        port: None,
        user_data: None,
        log_dir: None,
        settle_ms: 15000.0,
        term_ms: 5000.0,
        root: &root,
        tmpdir: &tmpdir,
        platform: sys::platform(),
        appdata: appdata.as_deref(),
        xdg_config_home: xdg.as_deref(),
        home: &home,
    };
    Instance::new(id, &o).map(Target::Dev).map_err(Fail::error)
}

// ---------- what an instance's tasks ask of this machine ----------

#[cfg(unix)]
fn signal_of(name: &str) -> libc::c_int {
    match name {
        "SIGKILL" => libc::SIGKILL,
        "SIGINT" => libc::SIGINT,
        "SIGHUP" => libc::SIGHUP,
        "SIGQUIT" => libc::SIGQUIT,
        "0" => 0,
        _ => libc::SIGTERM,
    }
}

/// process.kill(pid, 0): whether the process is there (or there and not
/// ours to signal).
fn alive(pid: f64) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) with signal 0 sends nothing; it only checks the
        // process.
        if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            return true;
        }
        std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let r = super::native::run("tasklist", &["/FI".into(), format!("PID eq {pid}"), "/NH".into()], 5000.0, None);
        r.stdout.split_whitespace().any(|w| w == pid.to_string())
    }
}

/// process.kill(pid, signal), quietly.
fn kill(pid: f64, signal: &str) {
    #[cfg(unix)]
    {
        // SAFETY: kill(2) only sends a signal; the pid is one the task
        // found holding the instance's lock, or started itself.
        unsafe {
            libc::kill(pid as libc::pid_t, signal_of(signal));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        let _ = super::native::run("taskkill", &["/PID".into(), pid.to_string(), "/T".into(), "/F".into()], 5000.0, None);
    }
}

/// electronCommand(): XenoNet Dev.app for this checkout, or the Electron
/// npm installed (require("electron")), with its arguments.
fn electron_command(root: &str) -> Result<Value, Value> {
    let (command, args) = control::electron_command(root, &sys::home(), sys::platform(), std::env::var("XENONET_SOFTWARE_RENDERING").is_ok_and(|v| v == "1"));
    let command = match command {
        Some(c) => c,
        None => {
            let dir = xenonet_base::paths::join_all(&[root, "node_modules", "electron"]);
            let path = std::fs::read_to_string(xenonet_base::paths::join(&dir, "path.txt"))
                .map_err(|_| error("Electron failed to install correctly. Please delete `node_modules/electron` and run \"npx install-electron --no\" manually."))?;
            let dist = sys::var("ELECTRON_OVERRIDE_DIST_PATH").unwrap_or_else(|| xenonet_base::paths::join(&dir, "dist"));
            xenonet_base::paths::join(&dist, path.trim())
        }
    };
    Ok(json!({ "command": command, "args": args }))
}

/// The stdio a detached child is started with: its output to a log, or
/// nowhere.
fn spawn(q: &Value, log: Option<&str>) -> Result<std::process::Child, Value> {
    let text = |k: &str| q.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    let mut cmd = std::process::Command::new(text("command"));
    cmd.args(q.get("args").and_then(Value::as_array).into_iter().flatten().map(xenonet_base::text::js_string)).stdin(std::process::Stdio::null());
    match log {
        Some(file) => {
            let out = std::fs::OpenOptions::new().create(true).append(true).open(file).map_err(|e| error(xenonet_base::fs::FsError::of(e, "open", file).message))?;
            let err = out.try_clone().map_err(|e| error(e.to_string()))?;
            cmd.stdout(out).stderr(err);
        }
        None => {
            cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        }
    }
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
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    cmd.spawn().map_err(|e| {
        let code = xenonet_base::fs::code_of(&e).unwrap_or("ENOENT");
        coded(format!("spawn {} {code}", text("command")), code)
    })
}

/// control.js's machine(system): fetch, the processes, the environment,
/// Electron's command, starting XenoNet Dev and whether it has exited.
struct Dev {
    root: String,
    child: Option<std::process::Child>,
}

impl Perform for Dev {
    async fn ask(&mut self, q: &Value) -> Reply {
        let pid = || q.get("pid").and_then(Value::as_f64).unwrap_or(0.0);
        match q.get("op").and_then(Value::as_str) {
            Some("fetch") => fetch_asked(q).await,
            Some("alive") => Ok(json!(alive(pid()))),
            Some("kill") => {
                kill(pid(), q.get("signal").and_then(Value::as_str).unwrap_or("SIGTERM"));
                Ok(Value::Null)
            }
            Some("env") => Ok(Value::Object(super::env_map())),
            Some("command") => electron_command(&self.root),
            Some("spawn") => {
                let log = q.get("logFile").map(xenonet_base::text::js_string).unwrap_or_default();
                self.child = Some(spawn(q, Some(&log))?);
                Ok(Value::Null)
            }
            Some("exited") => Ok(match self.child.as_mut().map(std::process::Child::try_wait) {
                Some(Ok(Some(st))) => st.code().map_or_else(|| exit_signal(&st), Value::from),
                _ => Value::Null,
            }),
            other => Err(error(format!("control asked for {}", other.unwrap_or("undefined")))),
        }
    }
}

#[cfg(unix)]
fn exit_signal(st: &std::process::ExitStatus) -> Value {
    use std::os::unix::process::ExitStatusExt;
    st.signal().map_or(Value::Null, |s| {
        json!(if s == libc::SIGTERM {
            "SIGTERM".to_string()
        } else if s == libc::SIGKILL {
            "SIGKILL".to_string()
        } else {
            format!("SIG{s}")
        })
    })
}

#[cfg(not(unix))]
fn exit_signal(_: &std::process::ExitStatus) -> Value {
    Value::Null
}

/// consoleOps: the service's connections, starting Desktop as it
/// registered itself, and the API's reads.
struct Installed {
    env: Map<String, Value>,
    clients: HashMap<u64, super::components::Client>,
    bodies: HashMap<u64, Vec<u8>>,
    next: u64,
}

impl Perform for Installed {
    async fn ask(&mut self, q: &Value) -> Reply {
        let id = |k: &str| q.get(k).and_then(Value::as_u64).unwrap_or(0);
        match q.get("op").and_then(Value::as_str) {
            Some("connect") => {
                let c = connect_service(&self.env, "tools").await?;
                self.next += 1;
                let info = c.info.clone();
                self.clients.insert(self.next, c);
                Ok(json!({ "client": self.next, "info": info }))
            }
            Some("request") => {
                let Some(c) = self.clients.get(&id("client")) else { return Err(error("that connection is gone")) };
                c.request(q.get("method").and_then(Value::as_str).unwrap_or(""), q.get("params").cloned().unwrap_or(Value::Null), 150000.0).await.map_err(error)
            }
            Some("close") => {
                if let Some(c) = self.clients.remove(&id("client")) {
                    c.close();
                }
                Ok(Value::Null)
            }
            Some("spawn") => spawn(q, None).map(|_| Value::Null),
            Some("fetch") => {
                let url = q.get("url").map(xenonet_base::text::js_string).unwrap_or_default();
                let res = super::tasks::fetch("GET", &url, &[], None, None).await?;
                self.next += 1;
                self.bodies.insert(self.next, res.body);
                Ok(json!({ "res": self.next }))
            }
            Some("json") => {
                let body = self.bodies.get(&id("res")).cloned().unwrap_or_default();
                xenonet_base::json::parse(&String::from_utf8_lossy(&body)).map_err(|e| json!({ "message": e.to_string(), "name": "SyntaxError" }))
            }
            other => Err(error(format!("the console asked for {}", other.unwrap_or("undefined")))),
        }
    }
}

impl Drop for Installed {
    fn drop(&mut self) {
        for c in self.clients.values() {
            c.close();
        }
    }
}

// ---------- the client's methods ----------

fn num(a: &Value, k: &str, d: f64) -> f64 {
    a.get(k).and_then(Value::as_f64).unwrap_or(d)
}

/// One of the client's methods, `kind`, with its arguments, as the twin's
/// run(kind, args) made it.
async fn method(t: &Target, kind: &str, a: Value) -> Result<Value, Failed> {
    match t {
        Target::Dev(i) => {
            let (i, kind, root) = (i.clone(), kind.to_string(), i.root.clone());
            let task = Task::new(move |io| async move {
                let opts = a.get("opts");
                match kind.as_str() {
                    "health" => Ok(Value::Bool(i.health(&io).await)),
                    "call" => i.call(&io, a.get("body").unwrap_or(&Value::Null), num(&a, "wait", 20000.0)).await,
                    "ping" => i.ping(&io).await,
                    "ready" => i.ready(&io, num(&a, "timeout", 60000.0)).await,
                    "launch" => i.launch(&io, a.get("show"), a.get("restart") == Some(&Value::Bool(true)), num(&a, "timeout", 120000.0)).await,
                    "quit" => i.quit(&io, num(&a, "timeout", 20000.0)).await,
                    "driver" => i.driver(&io, a.get("method").unwrap_or(&Value::Null), a.get("args").and_then(Value::as_array).map_or(&[][..], Vec::as_slice)).await,
                    "simulate" => i.simulate(&io, a.get("kind").unwrap_or(&Value::Null), a.get("a"), a.get("b")).await,
                    "screenshot" | "input" | "window" | "record" | "logs" => i.call(&io, &Instance::body(&kind, None, opts), 20000.0).await,
                    "eval" => i.call(&io, &Instance::body("eval", Some(("expression", a.get("expression").cloned().unwrap_or(Value::Null))), opts), 20000.0).await,
                    "api" => i.api(&io, a.get("route").and_then(Value::as_str).unwrap_or("")).await,
                    _ => Err(xenonet_app::task::Failure::msg(format!("no control task {kind}"))),
                }
            });
            drive(task, &mut Dev { root, child: None }).await
        }
        Target::Installed(c) => {
            if matches!(kind, "eval" | "simulate") {
                return Err(Failed { message: "This action is available only in development instances.".into(), fields: Map::new() });
            }
            let c = Console { env: c.env.clone(), machine: c.machine.clone() };
            let env = c.env.clone();
            // health() is whether ping answers anything.
            let health = kind == "health";
            let kind = if health { "ping".to_string() } else { kind.to_string() };
            let task = Task::new(move |io| async move {
                let opts = a.get("opts");
                let with = |op: &str| {
                    let mut b = Map::new();
                    b.insert("op".into(), json!(op));
                    for (k, v) in opts.and_then(Value::as_object).into_iter().flatten() {
                        b.insert(k.clone(), v.clone());
                    }
                    Value::Object(b)
                };
                match kind.as_str() {
                    "call" => c.call(&io, a.get("body").unwrap_or(&Value::Null)).await,
                    "ping" => c.ping(&io).await,
                    "ready" => c.ready(&io, num(&a, "timeout", 60000.0)).await,
                    "quit" => c.quit(&io).await,
                    "launch" => {
                        let show = a.get("show").filter(|s| !s.is_null()).map_or_else(|| "inactive".to_string(), xenonet_base::text::js_string);
                        c.launch(&io, &show, a.get("restart") == Some(&Value::Bool(true)), num(&a, "timeout", 60000.0)).await
                    }
                    "driver" => {
                        c.call(&io, &json!({ "op": "driver", "method": a.get("method").cloned().unwrap_or(Value::Null), "args": a.get("args").cloned().unwrap_or(json!([])) }))
                            .await
                    }
                    "screenshot" | "input" | "window" | "logs" | "record" => c.call(&io, &with(&kind)).await,
                    "api" => c.api(&io, a.get("route").and_then(Value::as_str).unwrap_or("")).await,
                    _ => Err(xenonet_app::task::Failure::msg(format!("no console task {kind}"))),
                }
            });
            let out = drive(task, &mut Installed { env, clients: HashMap::new(), bodies: HashMap::new(), next: 0 }).await?;
            Ok(if health { json!(xenonet_base::text::truthy(Some(&out))) } else { out })
        }
    }
}

fn fail(f: Failed) -> Fail {
    Fail::error(f.message)
}

fn sync(id: &str, kind: &str, args: Value) -> Result<Value, Fail> {
    let t = target(id)?;
    block_on(method(&t, kind, args)).map_err(Fail::error)?.map_err(fail)
}

/// xenonet(instance).launch(options)
pub fn control_launch(instance: &str, opts: &Value) -> Result<(), Fail> {
    let mut a = Map::new();
    for k in ["show", "restart", "timeout"] {
        if let Some(v) = opts.get(k) {
            a.insert(k.into(), v.clone());
        }
    }
    sync(instance, "launch", Value::Object(a)).map(|_| ())
}

/// xenonet(instance).driver(op, arg): with no argument for null.
pub fn control_driver(instance: &str, op: &str, arg: &Value) -> Result<Value, Fail> {
    let args = if arg.is_null() { json!([]) } else { json!([arg]) };
    sync(instance, "driver", json!({ "method": op, "args": args }))
}

/// xenonet(instance).record(options)
pub fn control_record(instance: &str, opts: &Value) -> Result<Value, Fail> {
    sync(instance, "record", json!({ "opts": opts }))
}

/// xenonet(instance).quit()
pub fn control_quit(instance: &str) -> Result<(), Fail> {
    sync(instance, "quit", json!({})).map(|_| ())
}

/// The console operator's way in (xenonet mcp console): any of the
/// client's methods, `kind`, with its arguments as `method` takes them.
pub struct ConsoleControl;

impl xenonet_mcp::Control for ConsoleControl {
    fn method(&self, instance: &str, kind: &str, args: Value) -> Result<Value, String> {
        sync(instance, kind, args).map_err(|f| f.message)
    }
}

// ---------- scripts ----------

/// An argument a step calls with, as JSON: what's undefined left out.
fn value_of(a: Arg) -> Option<Value> {
    match a {
        Arg::V(v) => Some(v),
        Arg::Undef => None,
        Arg::Obj(fields) => Some(Value::Object(fields.into_iter().filter_map(|(k, a)| value_of(a).map(|v| (k.to_string(), v))).collect())),
    }
}

/// A step's call: the driver, or one of the client's own methods, with its
/// arguments as the JS passed them.
async fn step_call(t: &Target, target: &str, args: Vec<Arg>) -> Result<Value, Failed> {
    let mut args: Vec<Option<Value>> = args.into_iter().map(value_of).collect();
    let mut take = || if args.is_empty() { None } else { args.remove(0) };
    match target {
        "driver" => {
            let name = take().unwrap_or(Value::Null);
            let rest: Vec<Value> = std::iter::from_fn(|| (!args.is_empty()).then(|| args.remove(0).unwrap_or(Value::Null))).collect();
            method(t, "driver", json!({ "method": name, "args": rest })).await
        }
        "launch" | "quit" => {
            let o = take().unwrap_or(json!({}));
            method(t, target, o).await
        }
        "ping" => method(t, "ping", json!({})).await,
        "eval" => {
            let expression = take().unwrap_or(Value::Null);
            let opts = take().unwrap_or(json!({}));
            method(t, "eval", json!({ "expression": expression, "opts": opts })).await
        }
        "simulate" => {
            let kind = take().unwrap_or(Value::Null);
            let o = take().unwrap_or(json!({}));
            let mut a = json!({ "kind": kind });
            for k in ["a", "b"] {
                if let Some(v) = o.get(k) {
                    a[k] = v.clone();
                }
            }
            method(t, "simulate", a).await
        }
        other => {
            let opts = take();
            method(t, other, opts.map_or(json!({}), |o| json!({ "opts": o }))).await
        }
    }
}

/// runStep(s, step, say): one step, saying what it has to show.
async fn run_step(t: &Target, step: &Value, say: &mut dyn FnMut(&Value)) -> Result<(), Fail> {
    let ctx = script::Context { name: t.name(), film_seconds: xenonet_state::frames::film_play_seconds(f64::NAN) };
    let (mut state, mut out) = script::begin(step, &ctx);
    let mut thrown: Option<Failed> = None;
    loop {
        for x in out.say {
            say(&value_of(x).unwrap_or_else(|| json!("undefined")));
        }
        let answer = match out.then {
            Then::Done => return Ok(()),
            Then::Rethrow => return Err(thrown.map_or_else(|| Fail::error("a step failed"), fail)),
            Then::Fail { message, .. } => return Err(Fail::error(message)),
            Then::Sleep(ms) => {
                tokio::time::sleep(std::time::Duration::from_millis(if ms.is_finite() && ms > 0.0 { ms as u64 } else { 0 })).await;
                Answer::Undefined
            }
            Then::Call { target, args } => match step_call(t, target, args).await {
                Ok(v) => Answer::Ok(v),
                Err(f) => {
                    let m = f.message.clone();
                    thrown = Some(f);
                    Answer::Threw(m)
                }
            },
        };
        (state, out) = script::resume(state, answer);
    }
}

/// runScript(xenonet(instance), steps, say)
pub fn run_script(instance: &str, steps: &[Value], say: &mut dyn FnMut(&Value)) -> Result<(), Fail> {
    let t = target(instance)?;
    block_on(async {
        for step in steps {
            run_step(&t, step, say).await?;
        }
        Ok(())
    })
    .map_err(Fail::error)?
}

/// A script that's a JS module: its default export, given core/control.js's
/// instance and a step function, in Node, from XenoNet's checkout. What it
/// says goes to standard output as each step says it, dimmed unless
/// `plain`, or nowhere when `quiet`.
pub fn run_module(full: &str, file: &str, instance: &str, quiet: bool, plain: bool) -> Result<(), Fail> {
    let root = sys::xenonet_root();
    let control = xenonet_base::paths::join_all(&[&root, "core", "control.js"]);
    if !std::path::Path::new(&control).is_file() {
        return Err(Fail::new(format!("{file} is a JS module, which runs only with XenoNet's checkout (core/control.js); run it there with node cli/xenonet.js script run"), 1));
    }
    let Some(node) = super::native::on_path("node") else { return Err(Fail::new(format!("{file} is a JS module, which runs in Node, and there's no node on your PATH"), 1)) };
    // What went wrong, if anything: { message, expected }.
    let said = std::env::temp_dir().join(format!("xenonet-script-{}-{}.json", std::process::id(), super::native::now()));
    let code = r#"
const [root, full, file, instance, quiet, plain, said] = process.argv.slice(1);
const { pathToFileURL } = await import("node:url");
const fs = await import("node:fs");
const fail = (message, expected) => (fs.writeFileSync(said, JSON.stringify({ message, expected })), process.exit(1));
try {
  const { xenonet } = await import(pathToFileURL(`${root}/core/control.js`).href);
  const { runStep } = await import(pathToFileURL(`${root}/core/script.js`).href);
  const s = xenonet(instance);
  const dim = (t) => (plain === "1" ? t : `\x1b[2m${t}\x1b[22m`);
  const say = quiet === "1" ? () => {} : (v) => console.log(dim(typeof v === "string" ? v : JSON.stringify(v)));
  const mod = await import(pathToFileURL(full).href);
  if (typeof mod.default !== "function") fail(`${file} needs a default export: async function ({ xenonet, step }) { … }`, true);
  await mod.default({ xenonet: s, step: (st) => runStep(s, st, say) });
} catch (err) {
  fail(String(err?.message ?? err), false);
}
"#;
    let status = std::process::Command::new(node)
        .args(["--input-type=module", "-e", code, &root, full, file, instance, if quiet { "1" } else { "0" }, if plain { "1" } else { "0" }, &said.to_string_lossy()])
        .status()
        .map_err(|e| Fail::error(format!("couldn't start node: {e}")))?;
    let why = std::fs::read_to_string(&said).ok().and_then(|t| xenonet_base::json::parse(&t).ok());
    let _ = std::fs::remove_file(&said);
    if let Some(w) = why {
        let message = w.get("message").map(xenonet_base::text::js_string).unwrap_or_default();
        return Err(if w.get("expected") == Some(&json!(true)) { Fail::new(message, 1) } else { Fail::error(message) });
    }
    if !status.success() {
        return Err(Fail::error(format!("{file} ended with {status}")));
    }
    Ok(())
}
