//! The doctor run on this machine, with no XenoNet to ask
//! (core/doctor/checkup.js's fallback, which the doctor tool and `xenonet
//! doctor` take when nothing answers): xenonet-doctor's checks, each a
//! task, asking this machine for what they read as core/doctor/index.js's
//! machine() answered them. The checks that need XenoNet running are
//! skipped by the checks themselves.
//!
//! Each check runs on a thread of its own and gets 8 seconds, as the JS
//! ran them side by side, each raced against its time limit.

use serde_json::{json, Map, Value};
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use xenonet_app::task::{Failure, Outcome, Reply, Task};
use xenonet_doctor::{checks, run};

const CHECK_MS: u64 = 8000;
const DAY_MS: f64 = 24.0 * 3600e3;

/// The schemas state.json is checked against, as schemas/ has them.
const STATE_SCHEMAS: [&str; 2] = [include_str!("../../../schemas/state/v1.json"), include_str!("../../../schemas/state/v2.json")];
const STATE_VERSION: f64 = 2.0;

/// The folders the checks look at (core/config.js's, with the projects
/// folder in use): stateDir, cacheDir, projectsDir, claudeProjectsDir,
/// codexHome and homeDir.
pub type Paths = Value;

fn now() -> f64 {
    xenonet_app::task::now_ms()
}

fn home() -> String {
    std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default()
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

// ---------- programs ----------

/// What a program said, as the doctor's run() resolves it: { code, stdout,
/// stderr, timedOut }, never an error.
fn run_program(file: &str, args: &[String], ms: f64, env: &[(&str, &str)], drop: &[&str]) -> Value {
    let mut cmd = Command::new(file);
    cmd.args(args).env_remove("ELECTRON_RUN_AS_NODE").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for k in drop {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({ "code": xenonet_base::fs::code_of(&e).unwrap_or("ENOENT"), "stdout": "", "stderr": "", "timedOut": false }),
    };
    let read = |p: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut p) = p {
                let _ = p.read_to_end(&mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let limit = Duration::from_millis(if ms.is_finite() && ms > 0.0 { ms as u64 } else { 5000 });
    let started = Instant::now();
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (Some(s), false),
            Ok(None) if started.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break (None, false),
        }
    };
    // err ? (err.code ?? 1) : 0: an exit code, or 1 for one ended by a
    // signal or the time limit.
    let code = match status {
        Some(s) if s.success() => 0,
        Some(s) => s.code().unwrap_or(1),
        None => 1,
    };
    json!({ "code": code, "stdout": out.join().unwrap_or_default(), "stderr": err.join().unwrap_or_default(), "timedOut": timed_out })
}

/// onPath(cmd): the first one on PATH that can run.
fn which(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    xenonet_base::paths::on_path(cmd, &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut xenonet_base::paths::can_run)
}

/// gitAvailable() (core/git.js): git on PATH, and on a Mac, the Command
/// Line Tools it needs.
fn git_available() -> bool {
    let git = which("git");
    let mut ok = git.is_some();
    if xenonet_sources::git::needs_dev_tools(platform(), git.as_deref()) {
        let r = run_program("/usr/bin/xcode-select", &["-p".into()], 3000.0, &[], &[]);
        // Without them, git is a stub that only asks to install them. The
        // doctor's own check says so; JS's console.log of it would land on
        // an MCP server's protocol.
        if r["code"] != json!(0) {
            ok = false;
        }
    }
    ok
}

/// findCodexBinary() (core/dirs.js): XENONET_CODEX_BIN, the ChatGPT app's
/// own codex, or "codex".
fn codex_bin() -> String {
    if let Some(b) = var("XENONET_CODEX_BIN") {
        return b;
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
    let env = xenonet_base::dirs::Env { claude_desktop_dir: None, local_app_data: var("LOCALAPPDATA"), app_data: var("APPDATA"), xdg_config_home: var("XDG_CONFIG_HOME") };
    xenonet_base::dirs::bundled_codex_binaries(&env, platform(), &home(), &mut Disk).into_iter().next().unwrap_or_else(|| "codex".into())
}

// ---------- the machine's other answers ----------

/// The launcher hooks and MCP servers run, when there is one: its folder,
/// what it says of itself (`node cli/xenonet.js --launcher`), its version
/// and the version this XenoNet ships (machine().launcher()). `source` is
/// the launcher script this XenoNet ships (core/components/launcher.cjs).
pub fn launcher(source: &str) -> Result<Option<Value>, String> {
    let env: Map<String, Value> = std::env::vars().map(|(k, v)| (k, Value::String(v))).collect();
    let m = xenonet_components::runtime::Machine { platform: platform().into(), arch: String::new(), home: home() };
    let runtime = xenonet_components::runtime::runtime_dir(&env, &m);
    let dir = xenonet_base::paths::join(&runtime, "launcher");
    let node = xenonet_base::paths::join(&dir, if cfg!(windows) { "node.exe" } else { "node" });
    let entry = xenonet_base::paths::join_all(&[&dir, "cli", "xenonet.js"]);
    if std::fs::metadata(&node).is_err() || std::fs::metadata(&entry).is_err() {
        return Ok(None);
    }
    let r = run_program(&node, &[entry.clone(), "--launcher".into()], 8000.0, &[], &[]);
    let said = r["stdout"].as_str().and_then(|s| xenonet_base::json::parse(s).ok()).unwrap_or(Value::Null);
    let read = |f: &str| std::fs::read(f).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| xenonet_base::fs::FsError::of(e, "open", f).message);
    let version = xenonet_components::install::launcher_version(Some(&read(&entry)?));
    let ships = xenonet_components::install::launcher_version(Some(&read(source)?));
    Ok(Some(json!({ "dir": dir, "said": said, "version": xenonet_base::json::number(version), "ships": xenonet_base::json::number(ships) })))
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

/// readDriftFile in the cache folder, or an empty record of none.
fn drift(cache_dir: &str) -> Value {
    let file = xenonet_base::paths::join(cache_dir, xenonet_schema::drift::DRIFT_FILE);
    match xenonet_schema::drift::read_drift_file(std::path::Path::new(&file)) {
        Some((looked, records)) => json!({ "file": file, "lookedAt": looked.map_or(Value::Null, xenonet_base::json::number), "records": records }),
        None => json!({ "file": file, "lookedAt": null, "records": [] }),
    }
}

/// The process lookups' self-test (core/procs.js's selfTest): whether a
/// connection this process makes to itself comes back as this process,
/// with the processes above it and the program it runs.
fn procs() -> Value {
    struct Run;
    impl xenonet_engine::procs::Io for Run {
        fn exec(&mut self, call: xenonet_engine::procs::Call) -> Result<String, xenonet_engine::procs::Failure> {
            // Start times in English and UTC, and text as UTF-8.
            let r = run_program(&call.file, &call.args, 5000.0, &[("LC_TIME", "C"), ("LC_CTYPE", "UTF-8"), ("TZ", "UTC")], &["LC_ALL"]);
            let stdout = r["stdout"].as_str().unwrap_or("").to_string();
            if r["code"] == json!(0) {
                Ok(stdout)
            } else {
                Err(xenonet_engine::procs::Failure { message: format!("Command failed: {} {}", call.file, call.args.join(" ")), stdout: Some(stdout) })
            }
        }
        fn now(&mut self) -> f64 {
            now()
        }
    }
    let p = xenonet_engine::procs::Procs::new(platform());
    if !p.supported {
        return json!({ "supported": false });
    }
    let Ok(server) = std::net::TcpListener::bind("127.0.0.1:0") else { return json!({ "supported": true, "peer": false, "chain": false, "exe": false }) };
    let Ok(client) = std::net::TcpStream::connect(server.local_addr().expect("an address")) else {
        return json!({ "supported": true, "peer": false, "chain": false, "exe": false });
    };
    let accepted = server.accept().ok();
    let me = f64::from(std::process::id());
    let (pid, first) = match &accepted {
        Some((socket, _)) => {
            let local = socket.local_addr().map(|a| json!(a.port())).unwrap_or(Value::Null);
            let remote = socket.peer_addr().map(|a| json!(a.port())).unwrap_or(Value::Null);
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

#[cfg(unix)]
fn writable(p: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(p) else { return false };
    // SAFETY: `c` is a NUL-terminated string that outlives the call, and
    // access(2) only reads it.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(p: &str) -> bool {
    std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly())
}

#[cfg(unix)]
fn statfs(p: &str) -> Option<Value> {
    let c = std::ffi::CString::new(p).ok()?;
    // SAFETY: statvfs is a plain C struct of numbers, for which all zeros
    // is a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated string and `st` a struct statvfs(3)
    // may write, both alive for the call.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(
        json!({ "bsize": st.f_frsize as f64, "blocks": st.f_blocks as f64, "bfree": st.f_bfree as f64, "bavail": st.f_bavail as f64, "files": st.f_files as f64, "ffree": st.f_ffree as f64 }),
    )
}

#[cfg(not(unix))]
fn statfs(_: &str) -> Option<Value> {
    None
}

/// What the machine() a check is run on knows.
struct Machine {
    paths: Value,
    home: String,
    launcher_source: String,
}

fn text(v: Option<&Value>) -> String {
    v.map(xenonet_base::text::js_string).unwrap_or_default()
}

impl Machine {
    fn path(&self, k: &str) -> Option<String> {
        self.paths.get(k).filter(|v| xenonet_base::text::truthy(Some(v))).map(xenonet_base::text::js_string)
    }

    /// ctx[fn](...args): { value } or { missing } (a ctx without it), or
    /// what it threw.
    fn call(&self, f: &str, args: &[Value]) -> Reply {
        let a = |i: usize| text(args.get(i));
        let value = match f {
            "exists" => json!(std::fs::metadata(a(0)).is_ok() && !a(0).is_empty()),
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
            "which" => which(&a(0)).map_or(Value::Null, Value::String),
            "run" => {
                let argv: Vec<String> = args.get(1).and_then(Value::as_array).map(|l| l.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default();
                let ms = args.get(2).and_then(Value::as_f64).unwrap_or(5000.0);
                run_program(&a(0), &argv, ms, &[], &[])
            }
            "command" => {
                let st = xenonet_app::command::command_state(
                    Some(&xenonet_app::command::command_dir(&self.home, platform(), var("LOCALAPPDATA").as_deref())),
                    std::env::var("PATH").ok().as_deref(),
                    platform(),
                );
                json!({ "file": st.file, "installed": st.installed, "ours": st.ours, "runs": st.runs, "broken": st.broken, "onPath": st.on_path })
            }
            "savedPath" => {
                if platform() != "win32" {
                    Value::Null
                } else {
                    let env: Map<String, Value> = std::env::vars().map(|(k, v)| (k, Value::String(v))).collect();
                    let reg = xenonet_app::command::reg_exe(&env);
                    let (mut read, mut dirs) = (false, Vec::new());
                    for key in xenonet_app::command::PATH_KEYS {
                        let r = run_program(&reg, &xenonet_app::command::reg_query(key), 5000.0, &[], &[]);
                        if r["code"] != json!(0) {
                            continue;
                        }
                        read = true;
                        dirs.extend(xenonet_app::command::reg_path(r["stdout"].as_str().unwrap_or(""), &env));
                    }
                    if read {
                        json!(dirs)
                    } else {
                        Value::Null
                    }
                }
            }
            "procs" => procs(),
            "errors" => match self.path("stateDir") {
                Some(dir) => Value::Array(xenonet_base::log::read_logs(std::path::Path::new(&xenonet_base::log::log_dir(&dir)), now() - DAY_MS, "error", None, None, 500)),
                None => Value::Null,
            },
            "launcher" => launcher(&self.launcher_source).map_err(|m| json!({ "message": m }))?.unwrap_or(Value::Null),
            "stateFile" => inspect_state(&a(0)),
            "drift" => self.path("cacheDir").map_or(Value::Null, |d| drift(&d)),
            "gitAvailable" => json!(git_available()),
            // What only a running XenoNet has: its actors, its process
            // checks and its hosts.
            _ => return Ok(json!({ "missing": true })),
        };
        Ok(json!({ "value": value }))
    }

    fn answer(&self, q: &Value) -> Reply {
        match q.get("op").and_then(Value::as_str) {
            Some("now") => Ok(xenonet_base::json::number(now())),
            Some("sleep") => {
                std::thread::sleep(Duration::from_millis(q.get("ms").and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u64));
                Ok(Value::Null)
            }
            Some("call") => {
                let f = q.get("fn").and_then(Value::as_str).unwrap_or("");
                let args = q.get("args").and_then(Value::as_array).cloned().unwrap_or_default();
                self.call(f, &args)
            }
            other => Err(json!({ "message": format!("no such request here: {}", other.unwrap_or("undefined")) })),
        }
    }
}

fn fs_error(e: xenonet_base::fs::FsError) -> Value {
    let mut m = Map::new();
    m.insert("message".into(), json!(e.message));
    if let Some(c) = e.code {
        m.insert("code".into(), json!(c));
    }
    Value::Object(m)
}

/// One check's verdict, on this thread.
fn check(id: &str, facts: Value, m: &Machine) -> Value {
    let f = checks::Facts { data: facts };
    let id_owned = id.to_string();
    let mut task = Task::new(move |io| async move { checks::run(io, id_owned, f).await });
    let mut reply = None;
    let mut last: Option<Value> = None;
    loop {
        match task.step(reply.take()) {
            Outcome::Ask(q) => {
                let r = m.answer(&q);
                if let Err(e) = &r {
                    last = Some(e.clone());
                }
                reply = Some(r);
            }
            Outcome::Done(v) => return v,
            Outcome::Failed(f) => {
                let message = match f {
                    Failure::Error { message, .. } => message,
                    Failure::Rethrow => last.as_ref().and_then(|e| e.get("message")).map(xenonet_base::text::js_string).unwrap_or_default(),
                };
                return json!({ "status": "warn", "detail": format!("the check failed: {message}") });
            }
        }
    }
}

/// The checks the binary knows no Node for: it is the runtime.
fn runtime_check() -> Value {
    json!({ "status": "ok", "detail": format!("the xenonet binary {}, which needs no Node", crate::tools::VERSION) })
}

/// runDoctor(machine({ paths }), { only }): every check (or those `only`
/// keeps), each in 8 seconds at most, and their summary.
pub fn run_doctor(paths: &Paths, only: Option<&[String]>, launcher_source: &str) -> Value {
    let started = now();
    // The home folder config.js has: the demo world's, for the demo.
    let home = paths.get("homeDir").and_then(Value::as_str).map_or_else(home, String::from);
    let facts = json!({
        "self": null, "base": null, "baseName": null, "health": null, "live": null, "connect": null,
        "paths": paths, "home": home, "platform": platform(), "electron": null, "node": null,
        "codexBin": codex_bin(), "byHand": null, "stateLoad": null,
        "projectsDirSet": var("XENONET_PROJECTS_DIR").is_some(), "actors": "none",
    });
    let picked: Vec<&(&str, &str, &str)> = checks::CHECKS.iter().filter(|(id, ..)| only.is_none_or(|o| run::kept(id, o))).collect();
    let mut waiting = Vec::new();
    for (i, (id, ..)) in picked.iter().enumerate() {
        let (tx, rx) = mpsc::channel();
        let (id, facts, m) = (id.to_string(), facts.clone(), Machine { paths: paths.clone(), home: home.clone(), launcher_source: launcher_source.to_string() });
        let t0 = now();
        std::thread::spawn(move || {
            let r = if id == "system.runtime" { runtime_check() } else { check(&id, facts, &m) };
            let _ = tx.send(r);
        });
        waiting.push((i, t0, rx));
    }
    let deadline = Instant::now() + Duration::from_millis(CHECK_MS);
    let mut results = Vec::new();
    for ((id, group, title), (_, t0, rx)) in picked.iter().zip(waiting) {
        let left = deadline.saturating_duration_since(Instant::now());
        let r = rx.recv_timeout(left).unwrap_or_else(|_| json!({ "status": "warn", "detail": format!("didn't finish in {} seconds", CHECK_MS / 1000) }));
        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert("group".into(), json!(group));
        out.insert("title".into(), json!(title));
        for (k, v) in r.as_object().into_iter().flatten() {
            out.insert(k.clone(), v.clone());
        }
        out.insert("ms".into(), xenonet_base::json::number((now() - t0).max(0.0)));
        results.push(Value::Object(out));
    }
    json!({ "at": xenonet_base::json::number(started), "ms": xenonet_base::json::number(now() - started), "results": results, "summary": run::summarize(&results) })
}

/// The launcher script this XenoNet ships (core/components/launcher.cjs),
/// in XenoNet's own folder: the first of the folders the binary is in, and
/// those above it, with XenoNet's package.json and cli/ (a checkout's, for
/// target/debug/xenonet), else the binary's own folder.
pub fn launcher_source() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let root = exe
        .ancestors()
        .skip(1)
        .take(5)
        .find(|d| d.join("package.json").is_file() && d.join("cli").is_dir())
        .or_else(|| exe.parent())
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    xenonet_base::paths::join_all(&[&root, "core", "components", "launcher.cjs"])
}

/// checkup()'s fallback: the doctor run here on these folders, by no
/// XenoNet.
pub fn checkup_here(paths: &Paths, only: Option<&[String]>, launcher_source: &str) -> Value {
    let mut r = run_doctor(paths, only, launcher_source);
    r["by"] = Value::Null;
    r
}

/// The folders of the XenoNet asked about, as config.js and workspace.js
/// find them in a process with no --demo: XENONET_STATE_DIR and
/// XENONET_CACHE_DIR, else the instance's own (XenoNet Dev's when that's
/// the one asked about, `name`), and the projects folder in use, as
/// state.json has it.
pub fn paths_for(name: Option<&str>) -> Paths {
    let home = home();
    let app = |kind: &str, n: &str| {
        xenonet_base::dirs::app_dir(
            kind,
            std::env::var(if kind == "state" { "XDG_STATE_HOME" } else { "XDG_CACHE_HOME" }).ok().as_deref(),
            std::env::var("LOCALAPPDATA").ok().as_deref(),
            platform(),
            &home,
            n,
        )
    };
    let dir = |kind: &str, v: &str| match var(v) {
        Some(set) => xenonet_base::paths::resolve(&set),
        None => xenonet_base::paths::resolve(&app(kind, name.unwrap_or("XenoNet"))),
    };
    let (state_dir, cache_dir) = (dir("state", "XENONET_STATE_DIR"), dir("cache", "XENONET_CACHE_DIR"));
    let claude = std::env::var("CLAUDE_CONFIG_DIR").unwrap_or_else(|_| xenonet_base::paths::join(&home, ".claude"));
    let saved = xenonet_state::workspace::saved_root(&xenonet_base::paths::join(&state_dir, "state.json")).map(Value::String);
    let root = xenonet_state::workspace::root(std::env::var("XENONET_PROJECTS_DIR").ok().as_deref(), saved.as_ref(), &home);
    json!({
        "homeDir": home,
        "projectsDir": root["path"],
        "codexHome": xenonet_base::paths::resolve(&std::env::var("CODEX_HOME").unwrap_or_else(|_| xenonet_base::paths::join(&home, ".codex"))),
        "claudeProjectsDir": xenonet_base::paths::join(&claude, "projects"),
        "stateDir": state_dir,
        "cacheDir": cache_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-mcp-doctor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    // test/cli.test.js's case: with no XenoNet, the checks run here, and
    // the ones that need XenoNet say so.
    #[test]
    fn the_checks_run_here_on_the_folders_given() {
        let state = tmp("state");
        let chosen = tmp("code");
        std::fs::write(format!("{state}/state.json"), format!("{{\"version\":2,\"workspace\":{{\"root\":\"{chosen}\"}}}}")).unwrap();
        let paths = json!({ "stateDir": state, "cacheDir": tmp("cache"), "projectsDir": chosen, "claudeProjectsDir": "/nowhere/claude", "codexHome": "/nowhere/codex", "homeDir": "/nowhere" });
        let r = checkup_here(&paths, Some(&["system.runtime".into(), "xenonet.server".into(), "system.projects".into(), "xenonet.state".into()]), "/nowhere/launcher.cjs");
        let got: Vec<(String, String)> = r["results"].as_array().unwrap().iter().map(|x| (text(x.get("id")), text(x.get("status")))).collect();
        assert_eq!(
            got,
            [("xenonet.server".into(), "warn".into()), ("xenonet.state".into(), "warn".into()), ("system.projects".into(), "ok".into()), ("system.runtime".into(), "ok".into())]
        );
        // A state.json of a workspace alone lacks the parts XenoNet writes.
        assert_eq!(r["results"][1]["detail"], json!("state.json has 5 parts XenoNet can't use, such as layout: expected record, received undefined"));
        assert_eq!(r["results"][2]["detail"], json!(format!("{chosen}, 0 entries")));
        assert_eq!(r["by"], Value::Null);
        assert_eq!(r["summary"]["warn"], json!(2));
    }

    #[test]
    fn state_json_as_the_doctor_reads_it() {
        let dir = tmp("inspect");
        assert_eq!(inspect_state(&dir)["exists"], json!(false));
        std::fs::write(format!("{dir}/state.json"), "{").unwrap();
        std::fs::write(format!("{dir}/state.broken-1.json"), "{").unwrap();
        std::fs::write(format!("{dir}/state.v1.json"), "{}").unwrap();
        let st = inspect_state(&dir);
        assert_eq!((st["unreadable"].clone(), st["broken"].as_array().unwrap().len(), st["backups"].as_array().unwrap().len()), (json!("isn't valid JSON"), 1, 1));
        std::fs::write(format!("{dir}/state.json"), "{\"version\":2,\"theme\":7}").unwrap();
        let st = inspect_state(&dir);
        assert_eq!(st["version"], json!(2));
        assert!(!st["issues"].as_array().unwrap().is_empty(), "a theme that isn't one: {st}");
    }
}
