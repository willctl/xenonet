//! The doctor inside the service (core/doctor/service.js, index.js,
//! context.js; crates/xenonet-doctor) and the diagnostics bundle
//! (core/diagnostics.js, crates/xenonet-app/src/diagnostics.rs).
//!
//! Each check is a task that asks for what it reads of the machine, as the
//! JS's `ctx` (machine() with doctorContext's overrides) answered it: its
//! files and programs here, and XenoNet's own state through the other cores
//! (the map's health and live state, the store's load report, the drift
//! log, the process check, actors, the gateway, wakes, hosts). The checks
//! run side by side, each given 8 seconds. A run happens a minute after the
//! service starts, then every half hour, and when asked; new problems and
//! each result go out as the doctor's events. The demo checks only itself,
//! never your machine.

use crate::fx::Locked;
use crate::fx::{self, now_ms, Perform, RunOptions};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use xenonet_doctor::{checks, run};
use xenonet_schema::drift::DriftLog;
use xenonet_service::config::Config;
use xenonet_service::cores::{CoreError, Cores, Diagnostics, Doctor, R};
use xenonet_service::glue::From;
use xenonet_service::wire::Emitter;

const CHECK_MS: u64 = 8000;
const DAY_MS: f64 = 24.0 * 3600e3;
const EVERY_MS: u64 = 30 * 60 * 1000;
const FIRST_AFTER_MS: u64 = 60 * 1000;

/// The checks the demo runs: itself, never your machine.
pub const DEMO_ONLY: [&str; 9] =
    ["xenonet.server", "xenonet.sources", "xenonet.blocks", "contracts.drift", "claude.transcripts", "codex.sessions", "system.projects", "system.runtime", "hosts"];

const STATE_SCHEMAS: [&str; 2] = [include_str!("../../../schemas/state/v1.json"), include_str!("../../../schemas/state/v2.json")];
const STATE_VERSION: f64 = 2.0;

/// What the doctor reads XenoNet's own state through.
pub struct Parts {
    pub cores: Cores,
    pub config: Config,
    /// config.js's settings, for `paths`.
    pub paths: Value,
    pub version: String,
    pub drift: Arc<Mutex<DriftLog>>,
    pub emitter: Emitter,
}

impl Parts {
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

fn fs_error(e: xenonet_base::fs::FsError) -> Value {
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
    if xenonet_sources::git::needs_dev_tools(fx::platform(), git.as_deref()) {
        let r = fx::exec_file("/usr/bin/xcode-select", &["-p".into()], 3000, RunOptions::default()).await;
        if !r.ok() {
            ok = false;
        }
    }
    ok
}

struct Disk;

impl xenonet_base::dirs::Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        std::fs::metadata(p).is_ok()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, xenonet_base::dirs::Unreadable> {
        xenonet_base::paths::read_dir_sorted(std::path::Path::new(p))
            .map(|v| v.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .map_err(|_| xenonet_base::dirs::Unreadable)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(0.0)
    }
}

/// findCodexBinary() (core/dirs.js): XENONET_CODEX_BIN, the ChatGPT app's
/// own codex, or "codex".
pub fn codex_bin(home: &str) -> String {
    if let Ok(b) = std::env::var("XENONET_CODEX_BIN") {
        return b;
    }
    let env = xenonet_base::dirs::Env {
        claude_desktop_dir: None,
        local_app_data: fx::env_set("LOCALAPPDATA"),
        app_data: fx::env_set("APPDATA"),
        xdg_config_home: fx::env_set("XDG_CONFIG_HOME"),
    };
    xenonet_base::dirs::bundled_codex_binaries(&env, fx::platform(), home, &mut Disk).into_iter().next().unwrap_or_else(|| "codex".into())
}

/// machine().launcher(): the launcher hooks and MCP servers run, when
/// there is one, what it says of itself, its version and the one this
/// XenoNet ships, the launcher script built in.
pub async fn launcher() -> Result<Value, Value> {
    let env = fx::env_map();
    let home = xenonet_service::config::home_dir(&|k| std::env::var(k).ok());
    let m = xenonet_components::runtime::Machine { platform: fx::platform().into(), arch: String::new(), home };
    let dir = xenonet_base::paths::join(&xenonet_components::runtime::runtime_dir(&env, &m), "launcher");
    // The xenonet binary first, which entries run now; it's a copy of the
    // Tools' own, so it's as new as they are.
    let bin = xenonet_base::paths::join(&dir, if cfg!(windows) { "xenonet.exe" } else { "xenonet" });
    if std::fs::metadata(&bin).is_ok() {
        let r = run_program(&bin, &["--launcher".into()], 8000.0).await;
        let said = r["stdout"].as_str().and_then(|s| xenonet_base::json::parse(s).ok()).unwrap_or(Value::Null);
        let ships = xenonet_components::install::launcher_version(Some(xenonet_components::install::LAUNCHER_SCRIPT));
        let version = said.get("launcher").cloned().unwrap_or_else(|| xenonet_base::json::number(ships));
        return Ok(json!({ "dir": dir, "said": said, "version": version, "ships": xenonet_base::json::number(ships) }));
    }
    let node = xenonet_base::paths::join(&dir, if cfg!(windows) { "node.exe" } else { "node" });
    let entry = xenonet_base::paths::join_all(&[&dir, "cli", "xenonet.js"]);
    if std::fs::metadata(&node).is_err() || std::fs::metadata(&entry).is_err() {
        return Ok(Value::Null);
    }
    let r = run_program(&node, &[entry.clone(), "--launcher".into()], 8000.0).await;
    let said = r["stdout"].as_str().and_then(|s| xenonet_base::json::parse(s).ok()).unwrap_or(Value::Null);
    let read = |f: &str| std::fs::read(f).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| fs_error(xenonet_base::fs::FsError::of(e, "open", f)));
    let version = xenonet_components::install::launcher_version(Some(&read(&entry)?));
    let ships = xenonet_components::install::launcher_version(Some(xenonet_components::install::LAUNCHER_SCRIPT));
    Ok(json!({ "dir": dir, "said": said, "version": xenonet_base::json::number(version), "ships": xenonet_base::json::number(ships) }))
}

/// inspectState(dir) (core/statefile.js): state.json's version and
/// whether it fits its schema, and the files XenoNet moved aside.
pub fn inspect_state(dir: &str) -> Value {
    let file = xenonet_base::paths::join(dir, "state.json");
    let names: Vec<String> =
        xenonet_base::paths::read_dir_sorted(std::path::Path::new(dir)).map(|v| v.iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    let (mut broken, mut backups) = (Vec::new(), Vec::new());
    for name in &names {
        let mid = |prefix: &str| name.strip_prefix(prefix).and_then(|r| r.strip_suffix(".json"));
        if mid("state.broken-").is_some_and(|m| !m.is_empty()) {
            broken.push(xenonet_base::paths::join(dir, name));
        } else if let Some(m) = mid("state.v") {
            let digits = m.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0 && (digits == m.len() || (m.as_bytes()[digits] == b'-' && m.len() > digits + 1)) {
                backups.push(xenonet_base::paths::join(dir, name));
            }
        }
    }
    let exists = names.iter().any(|n| n == "state.json");
    let mut out = json!({ "file": file, "exists": exists, "known": xenonet_base::json::number(STATE_VERSION), "version": null, "unreadable": null, "issues": [], "broken": broken, "backups": backups });
    if !exists {
        return out;
    }
    let text = match std::fs::read(&file) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            let f = xenonet_base::fs::FsError::of(e, "open", &file);
            out["unreadable"] = json!(format!("can't be read ({})", f.code.unwrap_or(f.message)));
            return out;
        }
    };
    let Ok(data) = xenonet_base::json::parse(&text) else {
        out["unreadable"] = json!("isn't valid JSON");
        return out;
    };
    if !data.is_object() {
        out["unreadable"] = json!("isn't a JSON object");
        return out;
    }
    let version = data.get("version").cloned().unwrap_or(json!(1));
    out["version"] = version.clone();
    if let Some(v) = version.as_f64().filter(|v| v.fract() == 0.0 && (1.0..=STATE_VERSION).contains(v)) {
        let docs: Vec<Value> = STATE_SCHEMAS.iter().filter_map(|t| xenonet_base::json::parse(t).ok()).collect();
        if let Ok(checker) = xenonet_schema::check::compile(&docs) {
            let checked = checker.check(checker.root(v as usize - 1), &xenonet_schema::check::Data::from(&data), false);
            out["issues"] = Value::Array(checked.issues.iter().map(|i| json!({ "path": i.path, "code": i.code, "message": i.message })).collect());
        }
    }
    out
}

/// The process lookups' self-test (core/procs.js's selfTest): whether a
/// connection this process makes to itself comes back as this process,
/// with the processes above it and the program it runs.
fn procs_self_test() -> Value {
    struct Run;
    impl xenonet_engine::procs::Io for Run {
        fn exec(&mut self, call: xenonet_engine::procs::Call) -> Result<String, xenonet_engine::procs::Failure> {
            let mut cmd = std::process::Command::new(&call.file);
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
    let p = xenonet_engine::procs::Procs::new(fx::platform());
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

/// `toLocaleString("en-US", { weekday: "short", hour: "numeric", minute:
/// "2-digit" })`, in this machine's time zone: Sat 8:48 PM.
#[cfg_attr(unix, expect(unsafe_code, reason = "localtime_r(3), which std doesn't wrap"))]
pub fn local_time(ms: f64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return "Invalid Date".into();
    }
    #[cfg(unix)]
    {
        let t = (ms / 1000.0).floor() as libc::time_t;
        // SAFETY: tm is a plain C struct, for which all zeros is a valid
        // value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r only reads `t` and fills `tm`, both alive for
        // the call.
        if !unsafe { libc::localtime_r(&raw const t, &raw mut tm) }.is_null() {
            let h = match tm.tm_hour % 12 {
                0 => 12,
                h => h,
            };
            return format!("{} {h}:{:02} {}", DAYS[tm.tm_wday.clamp(0, 6) as usize], tm.tm_min, if tm.tm_hour < 12 { "AM" } else { "PM" });
        }
    }
    let secs = (ms / 1000.0).floor() as i64;
    let day = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{} {h}:{minute:02} {}", DAYS[((day + 4).rem_euclid(7)) as usize], if hour < 12 { "AM" } else { "PM" })
}

/// One run's machine: what doctorContext gave machine(), for each check.
struct Machine {
    parts: Arc<Parts>,
    paths: Value,
}

fn text(v: Option<&Value>) -> String {
    v.map(xenonet_base::text::js_string).unwrap_or_default()
}

fn missing() -> fx::BoxFuture<Result<Value, Value>> {
    Box::pin(async { Ok(json!({ "missing": true })) })
}

impl Machine {
    fn path(&self, k: &str) -> Option<String> {
        self.paths.get(k).filter(|v| xenonet_base::text::truthy(Some(v))).map(xenonet_base::text::js_string)
    }

    /// ctx[fn](...args): { value } or { missing }, or what it threw.
    async fn call(self: Arc<Self>, f: String, args: Vec<Value>) -> Result<Value, Value> {
        let a = |i: usize| text(args.get(i));
        let cores = &self.parts.cores;
        let value = match f.as_str() {
            "exists" => json!(!a(0).is_empty() && std::fs::metadata(a(0)).is_ok()),
            "readdir" => {
                let dir = a(0);
                match xenonet_base::paths::read_dir_sorted(std::path::Path::new(&dir)) {
                    Ok(v) => Value::Array(v.iter().map(|e| json!(e.file_name().to_string_lossy())).collect()),
                    Err(e) => return Err(fs_error(xenonet_base::fs::FsError::of(e, "scandir", &dir))),
                }
            }
            "access" => {
                if !writable(&a(0)) {
                    return Err(json!({ "message": format!("EACCES: permission denied, access '{}'", a(0)), "code": "EACCES" }));
                }
                Value::Null
            }
            "statfs" => match statfs(&a(0)) {
                Some(v) => v,
                None => return Err(json!({ "message": format!("ENOENT: no such file or directory, statfs '{}'", a(0)), "code": "ENOENT" })),
            },
            "which" => fx::on_path(&a(0)).map_or(Value::Null, Value::String),
            "run" => {
                let argv: Vec<String> = args.get(1).and_then(Value::as_array).map(|l| l.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default();
                run_program(&a(0), &argv, args.get(2).and_then(Value::as_f64).unwrap_or(5000.0)).await
            }
            "command" => {
                let dir = xenonet_app::command::command_dir(&self.parts.home(), fx::platform(), fx::env_set("LOCALAPPDATA").as_deref());
                let st = xenonet_app::command::command_state(Some(&dir), std::env::var("PATH").ok().as_deref(), fx::platform());
                json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
            }
            "savedPath" => {
                if fx::platform() != "win32" {
                    Value::Null
                } else {
                    let env = fx::env_map();
                    let reg = xenonet_app::command::reg_exe(&env);
                    let (mut read, mut dirs) = (false, Vec::new());
                    for key in xenonet_app::command::PATH_KEYS {
                        let r = fx::exec_file(&reg, &xenonet_app::command::reg_query(key), 5000, RunOptions::default()).await;
                        if !r.ok() {
                            continue;
                        }
                        read = true;
                        dirs.extend(xenonet_app::command::reg_path(&r.stdout, &env));
                    }
                    if read {
                        json!(dirs)
                    } else {
                        Value::Null
                    }
                }
            }
            "procs" => tokio::task::spawn_blocking(procs_self_test).await.unwrap_or_else(|_| json!({ "supported": false })),
            "errors" => match self.path("stateDir") {
                Some(dir) => Value::Array(xenonet_base::log::read_logs(std::path::Path::new(&xenonet_base::log::log_dir(&dir)), now_ms() - DAY_MS, "error", None, None, 500)),
                None => Value::Null,
            },
            "launcher" => launcher().await?,
            "stateFile" => inspect_state(&a(0)),
            "drift" => {
                let (file, looked, records) = self.parts.drift.locked().view();
                json!({ "file": file, "lookedAt": looked.map_or(Value::Null, xenonet_base::json::number), "records": records })
            }
            "verify" => cores.verify.state(),
            "gitAvailable" => json!(git_available().await),
            // How the service started at login stands (the supervisor's
            // review): it asks launchctl, systemctl or schtasks, so off
            // the runtime's threads.
            "login" => tokio::task::spawn_blocking(xenonet_components::supervisor::review_here).await.unwrap_or(Value::Null),
            f if f.starts_with("actors.") => {
                let Some(actors) = cores.actors.clone() else { return Ok(json!({ "missing": true })) };
                match &f["actors.".len()..] {
                    "expired" => cores.engine_store.expired_actor_tokens().map_err(|e| fx::error(e.message, None))?,
                    "observers" => actors.observers_status(),
                    "tokenFiles" => {
                        let fresh = args.first().and_then(|o| o.get("fresh")) == Some(&json!(true));
                        actors.token_files(fresh).await.map_err(|e| fx::error(e.message, None))?
                    }
                    "gateway" => {
                        let Some(g) = self.parts.emitter.ctx().and_then(|c| c.gateway.get().cloned()) else { return Ok(json!({ "missing": true })) };
                        g.status().await.map_err(|e| fx::error(e.message, None))?
                    }
                    "wakes" => {
                        let Some(w) = cores.wakes.clone() else { return Ok(json!({ "missing": true })) };
                        w.list().map_err(|e| fx::error(e.message, None))?
                    }
                    _ => return Ok(json!({ "missing": true })),
                }
            }
            _ => return Ok(json!({ "missing": true })),
        };
        Ok(json!({ "value": value }))
    }

    fn perform(self: &Arc<Self>) -> Perform {
        let me = self.clone();
        Arc::new(move |q: Value| {
            let me = me.clone();
            match q.get("op").and_then(Value::as_str) {
                Some("now") => Box::pin(async { Ok(xenonet_base::json::number(now_ms())) }),
                Some("localTime") => {
                    let ms = q.get("ms").and_then(Value::as_f64).unwrap_or(f64::NAN);
                    Box::pin(async move { Ok(json!(local_time(ms))) })
                }
                Some("call") => {
                    let f = q.get("fn").and_then(Value::as_str).unwrap_or("").to_string();
                    let args = q.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
                    if f.is_empty() {
                        return missing();
                    }
                    Box::pin(me.call(f, args))
                }
                Some("findTokenFiles") => Box::pin(async { Ok(json!({ "checked": 0, "files": [] })) }),
                other => {
                    let o = other.unwrap_or("undefined").to_string();
                    Box::pin(async move { Err(fx::error(format!("no such request: {o}"), None)) })
                }
            }
        })
    }
}

/// What the checks read of the service as data (factsOf): of the live
/// state only its Claude plan reading, and of the health its sources and
/// parts.
async fn facts(parts: &Parts, by_hand: bool) -> Value {
    let c = &parts.cores;
    let health = match (c.state.sources(), c.state.blocks()) {
        (Ok(sources), Ok(blocks)) => json!({ "sources": sources, "blocks": blocks }),
        _ => Value::Null,
    };
    let live =
        c.state.live_snapshot().ok().map_or(Value::Null, |l| json!({ "limits": { "claude": l.get("limits").and_then(|x| x.get("claude")).cloned().unwrap_or(Value::Null) } }));
    let connect = if parts.config.demo { Value::Null } else { c.connect.preview().await.unwrap_or(Value::Null) };
    let state_load = c.store.load_report().unwrap_or(Value::Null);
    json!({
        "self": { "port": parts.config.port, "name": parts.config.name },
        "base": null,
        "baseName": null,
        "health": health,
        "live": live,
        "connect": connect,
        "paths": parts.paths,
        "home": parts.home(),
        "platform": fx::platform(),
        "electron": null,
        "node": null,
        "codexBin": codex_bin(&parts.home()),
        "byHand": by_hand,
        "stateLoad": state_load,
        "projectsDirSet": fx::env_set("XENONET_PROJECTS_DIR").is_some(),
        "actors": if c.actors.is_some() { "on" } else { "off" },
    })
}

/// The runtime check, for a binary that needs no Node: it is the runtime.
fn runtime_check(version: &str) -> Value {
    json!({ "status": "ok", "detail": format!("the xenonet binary {version}, which needs no Node") })
}

/// One check's verdict, as a task asking the machine.
async fn one(id: String, facts: Value, m: Arc<Machine>) -> Value {
    if id == "system.runtime" {
        return runtime_check(&m.parts.version);
    }
    let f = checks::Facts { data: facts };
    match fx::drive(move |io| async move { checks::run(io, id, f).await }, m.perform()).await {
        Ok(v) => v,
        Err(e) => json!({ "status": "warn", "detail": format!("the check failed: {}", fx::message_of(&e)) }),
    }
}

/// runDoctor(ctx, { only }): every check (or those `only` keeps), the
/// hosts' with the hub on, each in 8 seconds at most, and their summary.
pub async fn run_doctor(parts: Arc<Parts>, only: Option<&[String]>, by_hand: bool) -> Value {
    let started = now_ms();
    let hosts: Option<Vec<Value>> = parts.cores.hub.as_ref().and_then(|h| h.list().ok()).map(|l| l.as_array().cloned().unwrap_or_default());
    // Each check: its id, group, title, and the host's view for a host's.
    let mut all: Vec<(String, String, String, Option<Value>)> = checks::all().into_iter().map(|(id, g, t)| (id.to_string(), g.to_string(), t.to_string(), None)).collect();
    if let Some(hs) = &hosts {
        for h in hs {
            let id = text(h.get("id"));
            all.push((format!("hosts.{id}"), "Hosts".into(), format!("{id} is connected"), Some(h.clone())));
        }
        let plain: Vec<Value> =
            hs.iter().map(|h| if h.is_object() { json!({ "id": h.get("id"), "key": h.get("key"), "simulated": h.get("simulated") }) } else { h.clone() }).collect();
        let keyed = xenonet_hosts::doctor::keyed(&plain);
        if keyed.len() >= 2 {
            all.push(("hosts.keys.own".into(), "Hosts".into(), "Each host has a key of its own".into(), Some(json!({ "keyed": keyed }))));
        }
    }
    let picked: Vec<_> = all.into_iter().filter(|(id, ..)| only.is_none_or(|o| run::kept(id, o))).collect();
    let facts = facts(&parts, by_hand).await;
    let m = Arc::new(Machine { paths: parts.paths.clone(), parts: parts.clone() });
    let runs = picked.into_iter().map(|(id, group, title, host)| {
        let (facts, m) = (facts.clone(), m.clone());
        async move {
            let t0 = now_ms();
            let check = async {
                match (&host, id.as_str()) {
                    (Some(k), "hosts.keys.own") => xenonet_hosts::doctor::key_check(k["keyed"].as_array().map_or(&[][..], Vec::as_slice)),
                    (Some(h), _) => run::host_check(h, now_ms()),
                    (None, _) => one(id.clone(), facts, m).await,
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
            out.insert("ms".into(), xenonet_base::json::number((now_ms() - t0).max(0.0)));
            Value::Object(out)
        }
    });
    let results: Vec<Value> = join_all(runs.collect()).await;
    json!({ "at": xenonet_base::json::number(started), "ms": xenonet_base::json::number(now_ms() - started), "results": results, "summary": run::summarize(&results) })
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
        let errors = if c.demo {
            Vec::new()
        } else {
            xenonet_base::log::read_logs(std::path::Path::new(&xenonet_base::log::log_dir(&c.state_dir)), now_ms() - DAY_MS, "warn", None, None, 500)
        };
        let connect = if c.demo { None } else { p.cores.connect.preview().await.ok() };
        let launcher = launcher().await.ok().filter(|l| !l.is_null());
        let env = fx::env_map();
        let arch = crate::system::os::arch();
        let rel = release();
        let g = xenonet_app::diagnostics::Gathered {
            xenonet: Some(&xenonet),
            doctor: doctor.as_ref(),
            errors: &errors,
            connect: connect.as_ref(),
            launcher: launcher.as_ref(),
            env: &env,
            home: &c.home,
            now: now_ms(),
            platform: fx::platform(),
            arch,
            release: &rel,
            // The binary is its own runtime.
            node: "none",
            electron: None,
        };
        xenonet_app::diagnostics::make_bundle(&g).map_err(CoreError::new)
    }

    fn markdown(&self, bundle: &Value) -> R<String> {
        Ok(xenonet_app::diagnostics::bundle_markdown(bundle))
    }

    fn write_look(&self, bundle: &Value) -> R<String> {
        let root = self.parts.config.root.to_string_lossy().into_owned();
        xenonet_app::diagnostics::write_look(&self.parts.config.state_dir, bundle, &[root], now_ms()).map_err(|e| CoreError::new(e.message))
    }

    fn ask_command(&self, app: &str, model: Option<&str>, dir: &str) -> R<Value> {
        let root = self.parts.config.root.to_string_lossy().into_owned();
        Ok(xenonet_app::diagnostics::ask_command(app, model, &json!(dir), &[root]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(demo: bool, state: &str) -> Arc<Parts> {
        let args: Vec<String> = if demo { vec!["--demo".into()] } else { Vec::new() };
        let mut config = Config::from(&args, &|k: &str| (k == "HOME").then(|| "/home/nova".to_string()));
        config.state_dir = state.into();
        config.cache_dir = format!("{state}/cache");
        let paths = json!({ "stateDir": state, "cacheDir": config.cache_dir, "projectsDir": state, "claudeProjectsDir": "/nowhere/claude", "codexHome": "/nowhere/codex", "homeDir": "/home/nova" });
        Arc::new(Parts {
            cores: Cores::pending(false, false, false),
            config,
            paths,
            version: "0.0.0".into(),
            drift: Arc::new(Mutex::new(DriftLog::default())),
            emitter: Emitter::new(),
        })
    }

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-wire-doctor-{name}-{}", std::process::id()));
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
        let ids: Vec<String> = r["results"].as_array().unwrap().iter().map(|x| text(x.get("id"))).collect();
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

    #[test]
    fn state_files_and_times() {
        let dir = tmp("state");
        assert_eq!(inspect_state(&dir)["exists"], false);
        std::fs::write(format!("{dir}/state.json"), "{nope").unwrap();
        assert_eq!(inspect_state(&dir)["unreadable"], "isn't valid JSON");
        std::fs::write(format!("{dir}/state.broken-2026.json"), "{}").unwrap();
        assert_eq!(inspect_state(&dir)["broken"].as_array().unwrap().len(), 1);
        let t = local_time(0.0);
        assert!(t.ends_with("AM") || t.ends_with("PM"), "{t}");
        assert_eq!(local_time(f64::NAN), "Invalid Date");
    }
}
