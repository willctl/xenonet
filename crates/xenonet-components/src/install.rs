//! core/components/install.js: installing and switching components. A
//! bundle installs what's missing, replaces what's from before 0.4.0 (a
//! node and XenoNet's JS, with no xenonet binary) and never upgrades what
//! else is there; a runtime that fails its startup check is never
//! selected; a Tools update
//! leaves the service alone; a service update stops the old one only when
//! no terminal is open in it, starts the new one and checks it answers as
//! the version it should be, and rolls back if it doesn't. A record of the
//! switch is kept until the new version answers, so an updater that
//! crashed can finish it, or restore the previous one, on the next launch.
//! The selected service runs from a copy of its version's folder, `active`,
//! whose path stays the same from one version to the next.
//!
//! What it waits on is asked of whoever runs it (task.rs): { op: "connect"
//! }, "request" and "close" (the service), "start" (a runtime), "check" (a
//! staged component's startup check), "now", "pause", and "alive" for the
//! install lock.

use crate::packages::{current_component, lock, newer, select_component, stage_package, unlock};
use crate::runtime::{bin_name, launcher_bin, runtime_dir, Env, Machine};
use serde_json::{json, Value};
use std::time::Duration;
use xenonet_app::fsx::{self, FsError};
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;

/// What installing needs to know of the machine besides an env: where the
/// xenonet command goes.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub machine: Machine,
    /// process.env's LOCALAPPDATA, which the command's folder follows on
    /// Windows.
    pub local_app_data: Option<String>,
}

/// The node launcher script this XenoNet ships
/// (core/components/launcher.cjs), built in, so a package needs none of
/// XenoNet's JS beside its binary. It runs the Tools' binary for entries an
/// earlier XenoNet wrote (docs/design/contracts.md, section 12).
pub const LAUNCHER_SCRIPT: &str = include_str!("../../../core/components/launcher.cjs");

// The node an earlier XenoNet put in its packages and copied into the
// launcher's folder, which the entries it wrote run.
fn earlier_node(platform: &str) -> &'static str {
    if platform == "win32" {
        "node.exe"
    } else {
        "node"
    }
}

/// The version a launcher script says it is: 1 for the one before it
/// said, 0 for none.
pub fn launcher_version(text: Option<&str>) -> f64 {
    let t = text.unwrap_or("");
    if let Some(rest) = t.strip_prefix("// xenonet-launcher ") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            return digits.parse::<f64>().unwrap_or(f64::NAN);
        }
    }
    if t.is_empty() {
        0.0
    } else {
        1.0
    }
}

/// Writes the launcher script when there's none or this XenoNet ships a
/// newer one, through a temporary file and a rename, so a hook starting
/// meanwhile reads the old or the new, never half of either. One that
/// can't be replaced now is replaced next time.
pub fn write_launcher(entry: &str, source: &str) -> Result<bool, FsError> {
    let had = std::fs::read(entry).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    if let Some(h) = &had {
        if launcher_version(Some(h)) >= launcher_version(Some(source)) {
            return Ok(false);
        }
    }
    fsx::mkdir_p(&NATIVE.dirname(entry), None)?;
    let tmp = format!("{entry}.{}.tmp", std::process::id());
    match fsx::write_file(&tmp, source.as_bytes(), None).and_then(|()| fsx::rename(&tmp, entry)) {
        Ok(()) => Ok(true),
        Err(_) => {
            let _ = fsx::rm_rf(&tmp);
            Ok(false)
        }
    }
}

/// Why the launcher's binary is still the old one when it couldn't be
/// replaced, and when it will be.
pub const LAUNCHER_PENDING: &str = "XenoNet replaces it the next time it starts, or the next time you run xenonet update";

/// Puts `tmp` in `to`'s place (`rename` as fs.renameSync). Windows won't
/// replace a program that's running, and the launcher runs as long as an
/// MCP server or the xenonet command does, but it lets the running file be
/// renamed: that copy goes aside, as `<to>.<pid>-<ms>.old`, the new one
/// takes its name, and `sweep_aside` deletes the old one on a later start,
/// once nothing runs it. If the new one can't take the name, the old one
/// goes back.
pub fn replace_file(tmp: &str, to: &str, rename: &mut dyn FnMut(&str, &str) -> Result<(), FsError>) -> Result<(), FsError> {
    let first = match rename(tmp, to) {
        Ok(()) => return Ok(()),
        Err(e) => e,
    };
    if !fsx::exists(to) {
        return Err(first);
    }
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let aside = format!("{to}.{}-{ms}.old", std::process::id());
    rename(to, &aside)?;
    match rename(tmp, to) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = rename(&aside, to);
            Err(e)
        }
    }
}

/// Whether `entry` is a copy of `name` that `replace_file` put aside.
pub fn is_aside(entry: &str, name: &str) -> bool {
    entry.strip_prefix(name).is_some_and(|rest| rest.starts_with('.')) && std::path::Path::new(entry).extension().is_some_and(|x| x == "old")
}

/// Deletes the copies `replace_file` put aside beside `to`. One still
/// running (on Windows) stays until a later start.
pub fn sweep_aside(to: &str) {
    let (dir, name) = (NATIVE.dirname(to), NATIVE.basename(to, None));
    for entry in fsx::read_dir_names(&dir).unwrap_or_default() {
        if is_aside(&entry, &name) {
            let _ = fsx::unlink(&NATIVE.join(&[&dir, &entry]));
        }
    }
}

/// Copies the Tools' binary to the launcher's, when it isn't the copy of
/// these Tools already (its .from file says which it is), through a
/// temporary file and a rename, so a hook starting meanwhile runs the old
/// or the new; a launcher that's running is put aside first (see
/// `replace_file`). Ok(true) when it was written, Ok(false) when there was
/// nothing to do, and why not when it couldn't be.
pub fn write_launcher_bin(tools: &Value, to: &str, platform: &str) -> Result<bool, String> {
    sweep_aside(to);
    let from = NATIVE.join(&[&js_string(&tools["root"]), "bin", bin_name(platform)]);
    if !fsx::exists(&from) {
        return Ok(false);
    }
    let source = format!("{}-{}", js_string(&tools["version"]), js_string(&tools["sha256"]));
    let marker = format!("{to}.from");
    if fsx::read_text(&marker).is_ok_and(|had| had == source) && fsx::exists(to) {
        return Ok(false);
    }
    let tmp = format!("{to}.{}.tmp", std::process::id());
    let copied = std::fs::copy(&from, &tmp).map(|_| ()).map_err(|e| e.to_string());
    #[cfg(unix)]
    let copied = copied.and_then(|()| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
    });
    let done = copied.and_then(|()| replace_file(&tmp, to, &mut fsx::rename).map_err(|e| e.message));
    match done {
        Ok(()) => {
            let _ = fsx::write_file(&marker, source.as_bytes(), None);
            Ok(true)
        }
        Err(why) => {
            let _ = fsx::rm_rf(&tmp);
            Err(why)
        }
    }
}

/// The launcher, in the runtime folder, which never moves (hooks, MCP
/// servers and the xenonet command run its path): the Tools' xenonet
/// binary, copied. A node launcher an earlier XenoNet put beside it, which
/// the entries it wrote still run, gets this XenoNet's script, which runs
/// the binary; XenoNet never puts a node there itself. None without Tools.
/// `pending` says why the binary is still an earlier Tools' when it
/// couldn't be replaced, and when it will be.
pub fn install_launcher(env: &Env, ctx: &Ctx) -> Result<Option<Value>, Failure> {
    let m = &ctx.machine;
    let Some(tools) = current_component("tools", env, m)? else { return Ok(None) };
    let dir = NATIVE.join(&[&runtime_dir(env, m), "launcher"]);
    let bin = launcher_bin(env, m);
    fsx::mkdir_p(&dir, None)?;
    let pending = write_launcher_bin(&tools, &bin, &m.platform).err().map(|why| format!("the launcher couldn't be replaced ({why}); {LAUNCHER_PENDING}"));
    let entry = NATIVE.join(&[&dir, "cli", "xenonet.js"]);
    if fsx::exists(&NATIVE.join(&[&dir, earlier_node(&m.platform)])) && fsx::exists(&entry) {
        write_launcher(&entry, LAUNCHER_SCRIPT)?;
    }
    let bin = if fsx::exists(&bin) { json!(bin) } else { Value::Null };
    Ok(Some(json!({ "bin": bin, "root": dir, "pending": pending })))
}

// Runs a program as spawnSync did: its output, and its exit code (None if
// it couldn't start, or was ended at the timeout).
fn run_sync(file: &str, args: &[String], cwd: &str, timeout: Duration) -> (Option<i32>, String) {
    use std::io::Read;
    let mut command = std::process::Command::new(file);
    xenonet_base::process::Background::no_window(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    // A binary just unpacked can be busy for a moment on Linux: a fork on
    // another thread holds it open for writing until that child execs.
    let mut child = command.spawn();
    for _ in 0..20 {
        match &child {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(50));
                child = command.spawn();
            }
            _ => break,
        }
    }
    let Ok(mut child) = child else { return (None, String::new()) };
    let mut out = child.stdout.take().expect("stdout is piped above");
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        String::from_utf8_lossy(&s).into_owned()
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    (status, reader.join().unwrap_or_default())
}

/// Whether a selected Service or Tools has its xenonet binary. One from
/// before 0.4.0 has none: its package was a node and XenoNet's JS, which
/// this XenoNet replaces (install_bundle, or an update) rather than runs,
/// except to bring back a service that couldn't be replaced
/// (installed_runtime).
pub fn has_binary(selected: &Value, platform: &str) -> bool {
    let root = js_string(selected.get("root").unwrap_or(&Value::Null));
    std::fs::metadata(NATIVE.join(&[&root, "bin", bin_name(platform)])).is_ok_and(|meta| meta.is_file())
}

/// The startup check of a staged component: its xenonet binary runs,
/// without ELECTRON_RUN_AS_NODE, and says the version it was promised.
pub fn check_runtime(staged: &Value, platform: &str) -> Result<(), Failure> {
    let root = js_string(staged.get("root").unwrap_or(&Value::Null));
    let bin = NATIVE.join(&[&root, "bin", bin_name(platform)]);
    let (status, out) = run_sync(&bin, &["--version".into(), "--json".into()], &root, Duration::from_secs(15));
    let version = xenonet_base::json::parse(&out).ok().and_then(|v| v.get("version").cloned()).filter(|v| !v.is_null());
    if status != Some(0) || version.is_none() {
        return Err(Failure::msg("The new component runtime did not pass its startup check."));
    }
    if version.as_ref() != staged.get("version") {
        return Err(Failure::msg("The new component command did not pass its startup check."));
    }
    Ok(())
}

/// The folder the selected Service runs from,
/// `<runtime>/components/service/active`: a copy of the selected version's
/// own folder. A firewall lets a program in by its path, so the service
/// keeps one path across updates (docs/guide/components.md). Its `.from`
/// file says which version it's a copy of, as the launcher's does.
pub const ACTIVE: &str = "active";

fn service_dir(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&crate::packages::components_dir(env, m), "service"])
}

// What active/.from says of a copy of `selected`: its version and sha256.
fn active_mark(selected: &Value) -> String {
    format!("{}-{}", js_string(&selected["version"]), js_string(&selected["sha256"]))
}

/// Where the selected Service runs from: `active` when it's a copy of that
/// version, with its binary. Otherwise its version's own folder, and why,
/// unless there's no `active` at all, as before this version's first
/// activation.
pub fn service_root(service: &Value, env: &Env, m: &Machine) -> (String, Option<String>) {
    let own = js_string(&service["root"]);
    let active = NATIVE.join(&[&service_dir(env, m), ACTIVE]);
    if !fsx::exists(&active) {
        return (own, None);
    }
    let said = fsx::read_text(&NATIVE.join(&[&active, ".from"])).unwrap_or_default();
    if said.is_empty() {
        return (own, Some(format!("{ACTIVE} doesn't say which version it's a copy of")));
    }
    if said != active_mark(service) {
        return (own, Some(format!("{ACTIVE} is a copy of {said}, not {}", active_mark(service))));
    }
    if !has_binary(&json!({ "root": active }), &m.platform) {
        return (own, Some(format!("{ACTIVE} has no {}", bin_name(&m.platform))));
    }
    (active, None)
}

// Renames a folder; on Windows, which can hold a program a moment after it
// exits, tried again for up to 30 s, as stage_package's last rename is.
async fn rename_dir(io: &Io, from: &str, to: &str) -> Result<(), FsError> {
    let until = xenonet_app::task::now_ms() + 30000.0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let code = xenonet_base::fs::code_of(&e);
                if !cfg!(windows) || !matches!(code, Some("EPERM" | "EACCES" | "EBUSY")) || xenonet_app::task::now_ms() >= until {
                    return Err(FsError::of(e, "rename", from));
                }
                io.sleep(250.0).await;
            }
        }
    }
}

/// Makes `active` a copy of the selected Service, with the service stopped:
/// what an activation cut short left (`active.next`, `active.prev`) goes,
/// the version's folder is copied to `active.next` and marked, then
/// `active` moves aside and `active.next` takes its place. An `active` of
/// that version already stays as it is. A Service from before 0.4.0 runs
/// from its own folder, so there's no `active`, nor with none selected.
pub async fn activate_service(io: &Io, env: &Env, m: &Machine) -> Result<(), FsError> {
    let dir = service_dir(env, m);
    let at = |name: &str| NATIVE.join(&[&dir, name]);
    let (active, next, prev) = (at(ACTIVE), at("active.next"), at("active.prev"));
    fsx::rm_rf(&next)?;
    fsx::rm_rf(&prev)?;
    let selected = current_component("service", env, m)?;
    let Some(selected) = selected.filter(|s| has_binary(s, &m.platform)) else { return fsx::rm_rf(&active) };
    if service_root(&selected, env, m).0 == active {
        return Ok(());
    }
    let from = js_string(&selected["root"]);
    crate::tray::copy_tree(std::path::Path::new(&from), std::path::Path::new(&next)).map_err(|e| FsError::of(e, "copy", &from))?;
    fsx::write_file(&NATIVE.join(&[&next, ".from"]), active_mark(&selected).as_bytes(), None)?;
    if fsx::exists(&active) {
        rename_dir(io, &active, &prev).await?;
    }
    fsx::rename(&next, &active)?;
    // One that can't go now goes at the next activation.
    let _ = fsx::rm_rf(&prev);
    Ok(())
}

// activate_service, whose failure leaves the service to run from its
// version's folder (service_root): it still starts, though a firewall may
// ask about it again. So it's said, not thrown.
async fn activate(io: &Io, env: &Env, m: &Machine) {
    if let Err(e) = activate_service(io, env, m).await {
        eprintln!("[xenonet] XenoNet Service runs from its version's folder this time: {}", e.message);
    }
}

/// How to start the selected service: its xenonet binary, with `serve`,
/// and the env it runs with (the runtime folder, and the launcher's binary
/// when there is one). It runs from `active` (service_root), or its
/// version's folder when that isn't a copy of it. A Service from before
/// 0.4.0 runs its node and server, as that XenoNet ran it, told of the
/// node launcher its entries run, so an update that couldn't replace it
/// can bring it back as it was.
pub fn installed_runtime(env: &Env, ctx: &Ctx) -> Result<Value, Failure> {
    let m = &ctx.machine;
    let Some(service) = current_component("service", env, m)? else { return Err(Failure::msg("XenoNet Service is not installed. Use the bundle installer.")) };
    let launcher = install_launcher(env, ctx)?;
    let root = js_string(&service["root"]);
    let mut run_env = env.clone();
    run_env.insert("XENONET_RUNTIME_DIR".into(), json!(runtime_dir(env, m)));
    if let Some(bin) = launcher.as_ref().map(|l| &l["bin"]).filter(|b| !b.is_null()) {
        run_env.insert("XENONET_LAUNCHER_BIN".into(), bin.clone());
    }
    if has_binary(&service, &m.platform) {
        let (root, why) = service_root(&service, env, m);
        if let Some(why) = why {
            eprintln!("[xenonet] XenoNet Service runs from {root}: {why}");
        }
        return Ok(json!({ "root": root, "exec": NATIVE.join(&[&root, "bin", bin_name(&m.platform)]), "args": ["serve"], "env": run_env }));
    }
    let dir = NATIVE.join(&[&runtime_dir(env, m), "launcher"]);
    let node = NATIVE.join(&[&dir, earlier_node(&m.platform)]);
    if fsx::exists(&node) && fsx::exists(&NATIVE.join(&[&dir, "cli", "xenonet.js"])) {
        run_env.insert("XENONET_LAUNCHER_EXEC".into(), json!(node));
        run_env.insert("XENONET_LAUNCHER_ROOT".into(), json!(dir));
    }
    Ok(json!({
        "root": root,
        "exec": NATIVE.join(&[&root, earlier_node(&m.platform)]),
        "args": [NATIVE.join(&[&root, "server", "index.js"])],
        "env": run_env,
    }))
}

fn journal_file(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), "service-update.json"])
}

fn pointer(v: Option<&Value>) -> Value {
    match v.filter(|v| truthy(Some(v))) {
        None => Value::Null,
        Some(v) => {
            let mut p = serde_json::Map::new();
            for k in ["version", "sha256"] {
                if let Some(x) = v.get(k) {
                    p.insert(k.into(), x.clone());
                }
            }
            Value::Object(p)
        }
    }
}

fn write_journal(env: &Env, m: &Machine, previous: Option<&Value>, next: &Value) -> Result<(), FsError> {
    let file = journal_file(env, m);
    let tmp = format!("{file}.{}.tmp", std::process::id());
    fsx::write_file(&tmp, xenonet_base::json::stringify(&json!({ "previous": pointer(previous), "next": pointer(Some(next)) }), 0).as_bytes(), Some(0o600))?;
    fsx::rename(&tmp, &file)
}

fn clear_journal(env: &Env, m: &Machine) -> Result<(), FsError> {
    fsx::rm_rf(&journal_file(env, m))
}

// Selects the previous service again, and runs it from `active`, with the
// service stopped.
async fn restore_selection(io: &Io, previous: Option<&Value>, env: &Env, m: &Machine) -> Result<(), FsError> {
    match previous.filter(|p| truthy(Some(p))) {
        Some(p) => select_component("service", p, env, m)?,
        None => fsx::rm_rf(&NATIVE.join(&[&runtime_dir(env, m), "components/service/current.json"]))?,
    }
    activate(io, env, m).await;
    Ok(())
}

/// The lifecycle's own settings: how long a service gets to stop.
pub struct Life {
    pub stop_timeout: f64,
}

fn incompatible(e: &Value) -> bool {
    e.get("message").and_then(Value::as_str).unwrap_or("").contains("incompatible")
}

// The running service's status, or null; a service of an incompatible
// interface is said so.
async fn running(io: &Io, env: &Env) -> Result<Value, Failure> {
    let client = match io.ask(json!({ "op": "connect", "env": env })).await {
        Ok(c) => c,
        Err(e) if incompatible(&e) => return Err(Failure::Rethrow),
        Err(_) => return Ok(Value::Null),
    };
    if !truthy(client.get("client")) {
        return Ok(Value::Null);
    }
    let r = io.ask(json!({ "op": "request", "client": client["client"], "method": "status" })).await;
    let closed = io.ask(json!({ "op": "close", "client": client["client"] })).await;
    if closed.is_err() {
        return Err(Failure::Rethrow);
    }
    r.map_err(|_| Failure::Rethrow)
}

async fn now(io: &Io) -> Result<f64, Failure> {
    let n = io.ask(json!({ "op": "now" })).await.map_err(|_| Failure::Rethrow)?;
    Ok(xenonet_base::text::js_number(Some(&n)))
}

// The service answers `stop` and shuts down a moment later, so on a busy
// machine its answer can be lost with the connection: the rpc's
// "XenoNet disconnected; the action was not retried.".
fn dropped(e: &Value) -> bool {
    e.get("message").and_then(Value::as_str).is_some_and(|m| m.starts_with("XenoNet disconnected"))
}

// Stops the running service and waits until it has gone. A stop sent and
// then cut off may have worked: whether the service has gone decides, as it
// does for one that answered. An answer that says no is a failure.
async fn stop_service(io: &Io, env: &Env, life: &Life) -> Result<(), Failure> {
    let Ok(client) = io.ask(json!({ "op": "connect", "env": env })).await else { return Ok(()) };
    if !truthy(client.get("client")) {
        return Ok(());
    }
    let r = io.ask(json!({ "op": "request", "client": client["client"], "method": "stop" })).await;
    let closed = io.ask(json!({ "op": "close", "client": client["client"] })).await;
    if closed.is_err() || r.as_ref().is_err_and(|e| !dropped(e)) {
        return Err(Failure::Rethrow);
    }
    let deadline = now(io).await? + life.stop_timeout;
    while truthy(Some(&running(io, env).await?)) {
        if now(io).await? >= deadline {
            return Err(Failure::msg("The running service did not stop. Its selected version was left unchanged."));
        }
        io.ask(json!({ "op": "pause", "ms": 100 })).await.map_err(|_| Failure::Rethrow)?;
    }
    Ok(())
}

async fn start_version(io: &Io, version: &Value, env: &Env, ctx: &Ctx) -> Result<(), Failure> {
    let rt = installed_runtime(env, ctx)?;
    io.ask(json!({ "op": "start", "runtime": rt })).await.map_err(|_| Failure::Rethrow)?;
    let state = running(io, env).await?;
    let is = state.get("version");
    if is != Some(version) {
        let said = is.filter(|v| !v.is_null()).map_or_else(|| "no service".to_string(), js_string);
        return Err(Failure::msg(format!("Expected XenoNet Service {}, but {said} is running.", js_string(version))));
    }
    Ok(())
}

fn open_terminals(state: &Value) -> Result<bool, Failure> {
    match state.get("terminals") {
        Some(Value::Array(ts)) => Ok(ts.iter().any(|t| !truthy(t.get("exited")))),
        _ => Err(Failure::type_error("Cannot read properties of undefined (reading 'some')")),
    }
}

// The record kept while a service is switched, if there's one: finishes
// the switch when the new version is running, else restores the previous.
async fn recover(io: &Io, env: &Env, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    let m = &ctx.machine;
    let text = match fsx::read_text(&journal_file(env, m)) {
        Ok(t) => t,
        Err(e) if e.is("ENOENT") => return Ok(Value::Null),
        Err(e) => return Err(e.into()),
    };
    let record = xenonet_base::json::parse(&text).map_err(|e| Failure::msg(e.to_string()))?;
    let valid = |v: Option<&Value>| truthy(v) && crate::packages::is_semver(v.and_then(|v| v.get("version"))) && crate::packages::is_sha(v.and_then(|v| v.get("sha256")));
    let previous = record.get("previous");
    let next = match record.get("next") {
        Some(next) if valid(Some(next)) && (previous == Some(&Value::Null) || valid(previous)) => next,
        _ => return Err(Failure::msg("Invalid service update recovery record.")),
    };
    let previous = previous.filter(|p| !p.is_null());
    let state = running(io, env).await?;
    if truthy(Some(&state)) && state.get("version") == next.get("version") {
        select_component("service", next, env, m)?;
        clear_journal(env, m)?;
        return Ok(json!("completed"));
    }
    if truthy(Some(&state)) && state.get("version") != previous.and_then(|p| p.get("version")) {
        if open_terminals(&state)? {
            return Err(Failure::msg("Close running terminals before recovering the interrupted service update."));
        }
        stop_service(io, env, life).await?;
    }
    restore_selection(io, previous, env, m).await?;
    if let Some(p) = previous {
        start_version(io, &p["version"], env, ctx).await?;
    }
    clear_journal(env, m)?;
    Ok(json!("restored"))
}

/// Recovers an interrupted service update, if there's a record of one.
pub async fn recover_service_update(io: &Io, env: &Env, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    // Ordinary launches, including those during an update, take no lock.
    if !fsx::exists(&journal_file(env, &ctx.machine)) {
        return Ok(Value::Null);
    }
    let held = lock(io, env, &ctx.machine).await?;
    let r = recover(io, env, ctx, life).await;
    unlock(&held)?;
    r
}

/// Installs a bundle's components that aren't installed yet, and replaces
/// any from before 0.4.0 (see has_binary), with the launcher and, when
/// asked, the xenonet command; says how to start the service. A bundle
/// never silently restarts or upgrades a Service or Tools that has its
/// binary. On a Mac, a newer tray than the one installed replaces it.
pub async fn install_bundle(io: &Io, bundle: &str, env: &Env, command: bool, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    let held = lock(io, env, &ctx.machine).await?;
    let r = install_bundle_locked(io, bundle, env, command, ctx, life).await;
    unlock(&held)?;
    r
}

async fn install_bundle_locked(io: &Io, bundle: &str, env: &Env, command: bool, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    let m = &ctx.machine;
    recover(io, env, ctx, life).await?;
    let text = fsx::read_text(&NATIVE.join(&[bundle, "components.json"]))?;
    let manifest = xenonet_base::json::parse(&text).map_err(|e| Failure::msg(e.to_string()))?;
    let Some(Value::Array(items)) = manifest.get("packages") else { return Err(Failure::type_error("manifest.packages is not iterable")) };
    let mut earlier = None;
    for item in items {
        if !crate::packages::fits_here(item, &m.platform, &m.arch) {
            continue;
        }
        let component = item.get("component").map(js_string).unwrap_or_default();
        let old = current_component(&component, env, m)?;
        // The tray, on a Mac: installed when it's missing or this one is
        // newer. Nothing runs it from here; xenonet service install puts it
        // in Applications and starts it at login.
        if component == crate::packages::TRAY {
            let wanted = match &old {
                None => true,
                Some(o) => newer(item.get("version"), o.get("version")).map_err(Failure::msg)?,
            };
            if wanted {
                let file = NATIVE.join(&[bundle, &item.get("file").map(js_string).unwrap_or_default()]);
                let staged = stage_package(io, &file, item, env, m).await?;
                select_component(&component, &staged, env, m)?;
            }
            continue;
        }
        if old.as_ref().is_some_and(|o| has_binary(o, &m.platform)) {
            continue;
        }
        let file = NATIVE.join(&[bundle, &item.get("file").map(js_string).unwrap_or_default()]);
        let staged = stage_package(io, &file, item, env, m).await?;
        check_runtime(&staged, &m.platform)?;
        // A service from before 0.4.0 may be running: it's switched as an
        // update switches it, once the Tools and the launcher it names are
        // in place.
        match old {
            Some(o) if component == "service" => earlier = Some((o, staged)),
            _ => {
                select_component(&component, &staged, env, m)?;
                // A first Service: nothing of it runs yet.
                if component == "service" {
                    activate(io, env, m).await;
                }
            }
        }
    }
    let launcher = install_launcher(env, ctx)?;
    if let (true, Some(bin)) = (command, launcher.as_ref().and_then(|l| l["bin"].as_str())) {
        let dir = xenonet_app::command::command_dir(&m.home, &m.platform, ctx.local_app_data.as_deref());
        xenonet_app::command::install_command(&dir, bin, false, crate::runtime::var(env, "PATH"), &m.platform)?;
    }
    if let Some((old, staged)) = earlier {
        switch_service(io, env, ctx, life, Some(&old), &staged).await?;
    }
    installed_runtime(env, ctx)
}

/// Switches to a downloaded package, as the JS's activatePackage: a newer
/// Tools at once; a newer service only when no terminal is open in it,
/// with a record of the switch until the new one answers, and the old one
/// back if it doesn't.
pub async fn activate_package(io: &Io, file: &str, item: &Value, env: &Env, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    let held = lock(io, env, &ctx.machine).await?;
    let r = activate_locked(io, file, item, env, ctx, life).await;
    unlock(&held)?;
    r
}

async fn activate_locked(io: &Io, file: &str, item: &Value, env: &Env, ctx: &Ctx, life: &Life) -> Result<Value, Failure> {
    let m = &ctx.machine;
    recover(io, env, ctx, life).await?;
    let component = item.get("component").map(js_string).unwrap_or_default();
    let old = current_component(&component, env, m)?;
    if let Some(o) = &old {
        if !newer(item.get("version"), o.get("version")).map_err(Failure::msg)? {
            return Ok(json!({ "component": component, "version": o["version"], "status": "current" }));
        }
    }
    let staged = stage_package(io, file, item, env, m).await?;
    io.ask(json!({ "op": "check", "staged": staged })).await.map_err(|_| Failure::Rethrow)?;
    if component == "tools" {
        select_component("tools", &staged, env, m)?;
        let pending = install_launcher(env, ctx)?.and_then(|l| l["pending"].as_str().map(str::to_string));
        let detail = match pending {
            Some(why) => format!("New commands use this version once the launcher is replaced: {why}. Connected MCP processes keep running."),
            None => "New commands use this version. Connected MCP processes keep running.".into(),
        };
        return Ok(json!({ "component": "tools", "version": staged["version"], "status": "updated", "detail": detail }));
    }
    switch_service(io, env, ctx, life, old.as_ref(), &staged).await
}

/// Switches the selected service from `old` (or none) to `staged`, as an
/// update does: never while a terminal is open in the running service,
/// which leaves the new version staged; with a record of the switch until
/// the new version answers; and back to the old one when it doesn't.
async fn switch_service(io: &Io, env: &Env, ctx: &Ctx, life: &Life, old: Option<&Value>, staged: &Value) -> Result<Value, Failure> {
    let m = &ctx.machine;
    let state = running(io, env).await?;
    if truthy(Some(&state)) && open_terminals(&state)? {
        return Ok(json!({ "component": "service", "version": staged["version"], "status": "staged", "detail": "Close running terminals, then update the service." }));
    }
    write_journal(env, m, old, staged)?;
    let switched: Result<(), Failure> = async {
        if truthy(Some(&state)) {
            stop_service(io, env, life).await?;
        }
        select_component("service", staged, env, m)?;
        activate(io, env, m).await;
        start_version(io, &staged["version"], env, ctx).await?;
        clear_journal(env, m)?;
        Ok(())
    }
    .await;
    if let Err(err) = switched {
        let why = message(io, &err);
        let rolled: Result<(), Failure> = async {
            // Never finalize the new version when its startup check failed.
            let live = running(io, env).await?;
            if truthy(Some(&live)) && live.get("version") != old.and_then(|o| o.get("version")) {
                stop_service(io, env, life).await?;
            }
            restore_selection(io, old, env, m).await?;
            if let Some(o) = old {
                start_version(io, &o["version"], env, ctx).await?;
            }
            clear_journal(env, m)?;
            Ok(())
        }
        .await;
        if let Err(rollback) = rolled {
            return Err(Failure::msg(format!("The service update failed: {why} Recovery is pending: {}", message(io, &rollback))));
        }
        return Err(Failure::msg(format!("The service update failed. {}{why}", if old.is_some() { "The previous version was restored. " } else { "" })));
    }
    Ok(json!({ "component": "service", "version": staged["version"], "status": "updated" }))
}

// err.message, for a failure of the Rust's own or one a request threw.
fn message(io: &Io, f: &Failure) -> String {
    match f {
        Failure::Rethrow => io.last_message(),
        other @ Failure::Error { .. } => other.message().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("xenonet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    #[test]
    fn a_running_launcher_goes_aside_for_the_new_one() {
        let dir = scratch("replace");
        let (tmp, to) = (NATIVE.join(&[&dir, "xenonet.1.tmp"]), NATIVE.join(&[&dir, "xenonet"]));
        std::fs::write(&tmp, "new").unwrap();
        std::fs::write(&to, "old, running").unwrap();
        // As Windows has it: a running program can be renamed, but not
        // replaced.
        let mut calls = Vec::new();
        let mut windows = |from: &str, onto: &str| {
            calls.push(format!("{} -> {}", NATIVE.basename(from, None), NATIVE.basename(onto, None)));
            if fsx::exists(onto) {
                return Err(fsx::coded("EPERM", format!("EPERM: operation not permitted, rename '{from}' -> '{onto}'")));
            }
            fsx::rename(from, onto)
        };
        replace_file(&tmp, &to, &mut windows).unwrap();
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        let aside: Vec<String> = fsx::read_dir_names(&dir).unwrap().into_iter().filter(|n| is_aside(n, "xenonet")).collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(std::fs::read_to_string(NATIVE.join(&[&dir, &aside[0]])).unwrap(), "old, running");
        assert_eq!(calls.len(), 3, "{calls:?}");
        // A later start deletes it, and nothing else beside the launcher.
        std::fs::write(NATIVE.join(&[&dir, "xenonet.from"]), "1.0.0").unwrap();
        std::fs::write(NATIVE.join(&[&dir, "other.old"]), "not ours").unwrap();
        sweep_aside(&to);
        assert_eq!(fsx::read_dir_names(&dir).unwrap(), ["other.old", "xenonet", "xenonet.from"]);

        // One that can't be renamed either stays as it was, and says why.
        std::fs::write(&tmp, "newer").unwrap();
        let mut locked = |from: &str, onto: &str| Err(fsx::coded("EBUSY", format!("EBUSY: resource busy or locked, rename '{from}' -> '{onto}'")));
        assert!(replace_file(&tmp, &to, &mut locked).unwrap_err().message.contains("EBUSY"));
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Holds a copy of this test binary running, for the test below: it
    /// runs this test alone, which waits while its file is there.
    #[test]
    #[ignore = "run by a_launcher_running_on_windows_is_replaced"]
    fn hold_while_asked() {
        if let Some(file) = std::env::var_os("XENONET_HOLD_FILE") {
            let until = std::time::Instant::now() + Duration::from_secs(60);
            while std::path::Path::new(&file).exists() && std::time::Instant::now() < until {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_launcher_running_on_windows_is_replaced() {
        let dir = scratch("launcher-running");
        let (tools, launcher) = (NATIVE.join(&[&dir, "tools"]), NATIVE.join(&[&dir, "launcher"]));
        std::fs::create_dir_all(NATIVE.join(&[&tools, "bin"])).unwrap();
        std::fs::create_dir_all(&launcher).unwrap();
        std::fs::write(NATIVE.join(&[&tools, "bin", "xenonet.exe"]), "the new Tools' binary").unwrap();
        let to = NATIVE.join(&[&launcher, "xenonet.exe"]);
        // A program that runs from the launcher's path: this test binary.
        std::fs::copy(std::env::current_exe().unwrap(), &to).unwrap();
        let hold = NATIVE.join(&[&dir, "hold"]);
        std::fs::write(&hold, "").unwrap();
        let mut running = std::process::Command::new(&to)
            .args(["--ignored", "--exact", "install::tests::hold_while_asked"])
            .env("XENONET_HOLD_FILE", &hold)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        // Windows refuses a plain rename onto it.
        std::fs::write(NATIVE.join(&[&launcher, "plain.tmp"]), "x").unwrap();
        assert!(std::fs::rename(NATIVE.join(&[&launcher, "plain.tmp"]), &to).is_err(), "the running copy is locked");
        let tools_json = json!({ "root": tools, "version": "1.1.0", "sha256": "ab" });
        assert_eq!(write_launcher_bin(&tools_json, &to, "win32"), Ok(true));
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "the new Tools' binary");
        let aside = || fsx::read_dir_names(&launcher).unwrap().into_iter().filter(|n| is_aside(n, "xenonet.exe")).count();
        assert_eq!(aside(), 1, "the running copy, put aside");
        sweep_aside(&to);
        assert_eq!(aside(), 1, "kept while it runs");
        std::fs::remove_file(&hold).unwrap();
        running.wait().unwrap();
        // Windows can hold a file a moment after its process exits (a virus
        // scanner, say): a later start tries again.
        for _ in 0..50 {
            sweep_aside(&to);
            if aside() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(aside(), 0, "and deleted once it stopped");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn launcher_versions() {
        assert_eq!(launcher_version(Some("// xenonet-launcher 7\nrest")), 7.0);
        assert_eq!(launcher_version(Some("// an old one")), 1.0);
        assert_eq!(launcher_version(None), 0.0);
        assert_eq!(launcher_version(Some(LAUNCHER_SCRIPT)), 3.0, "the script built in");
    }
}

// Moving from 0.3.0, whose packages were a node and XenoNet's JS, on a
// made-up linux-x64 machine whose xenonet binaries are shell scripts that
// answer the startup check. (Two attributes: clippy takes only a plain
// cfg(test) for test code.)
#[cfg(test)]
#[cfg(unix)]
mod from_earlier {
    use super::*;
    use crate::packages::digest;
    use crate::tar::tests::archive;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use xenonet_app::task::{run_with, Done, Task};

    struct Place {
        dir: PathBuf,
        env: Env,
        ctx: Ctx,
    }

    impl Place {
        fn new(name: &str) -> Place {
            let dir = std::env::temp_dir().join(format!("xenonet-install-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let mut env = Env::new();
            env.insert("XENONET_RUNTIME_DIR".into(), json!(dir.join("runtime").to_string_lossy()));
            let machine = Machine { platform: "linux".into(), arch: "x64".into(), home: dir.to_string_lossy().into_owned() };
            Place { env, ctx: Ctx { machine, local_app_data: None }, dir }
        }

        fn runtime(&self) -> PathBuf {
            self.dir.join("runtime")
        }

        fn selected(&self, component: &str) -> Value {
            current_component(component, &self.env, &self.ctx.machine).unwrap().unwrap()
        }

        /// The install 0.3.0 left: each component a node and its JS in
        /// <version>-<sha16> beside current.json, and the launcher's node
        /// and cli/xenonet.js.
        fn earlier(&self) {
            for component in ["service", "tools"] {
                let sha = digest(format!("XenoNet {component} 0.3.0").as_bytes());
                let root = self.runtime().join("components").join(component).join(format!("0.3.0-{}", &sha[..16]));
                std::fs::create_dir_all(root.join("server")).unwrap();
                std::fs::create_dir_all(root.join("cli")).unwrap();
                std::fs::write(root.join("node"), "0.3.0's node").unwrap();
                std::fs::write(root.join("server/index.js"), "").unwrap();
                std::fs::write(root.join("cli/xenonet.js"), "").unwrap();
                select_component(component, &json!({ "version": "0.3.0", "sha256": sha }), &self.env, &self.ctx.machine).unwrap();
            }
            let launcher = self.runtime().join("launcher");
            std::fs::create_dir_all(launcher.join("cli")).unwrap();
            std::fs::write(launcher.join("node"), "0.3.0's node").unwrap();
            std::fs::write(launcher.join("cli/xenonet.js"), "// xenonet-launcher 2 (protocol 1)\n").unwrap();
        }

        /// A package of `version`, in a bundle folder with its
        /// components.json: (file, item).
        fn package(&self, component: &str, version: &str) -> (String, Value) {
            let meta = json!({ "format": 1, "component": component, "version": version, "platform": "linux", "arch": "x64", "dataSchema": 1, "api": { "min": 1, "max": 1 } });
            let bin = format!("#!/bin/sh\nprintf '{{\"version\":\"{version}\"}}'\n");
            let bytes = archive(&[("component.json", b'0', meta.to_string().as_bytes(), 0o644), ("bin/xenonet", b'0', bin.as_bytes(), 0o755)]);
            let name = format!("XenoNet-{component}-{version}-linux-x64.tar.gz");
            let bundle = self.dir.join("bundle");
            std::fs::create_dir_all(&bundle).unwrap();
            std::fs::write(bundle.join(&name), &bytes).unwrap();
            let mut item = meta;
            item["file"] = json!(name);
            item["size"] = json!(bytes.len());
            item["sha256"] = json!(digest(&bytes));
            (bundle.join(&name).to_string_lossy().into_owned(), item)
        }

        fn bundle(&self, version: &str) -> String {
            let items: Vec<Value> = ["service", "tools"].iter().map(|c| self.package(c, version).1).collect();
            let bundle = self.dir.join("bundle");
            std::fs::write(bundle.join("components.json"), json!({ "format": 1, "packages": items }).to_string()).unwrap();
            bundle.to_string_lossy().into_owned()
        }
    }

    impl Drop for Place {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// A service that answers status and stop: `state` is what status
    /// says (None: it's down), and each start's runtime is kept; a start of
    /// `failing` fails.
    #[derive(Default)]
    struct Service {
        state: Option<Value>,
        stops: usize,
        starts: Vec<Value>,
    }

    fn run(p: &Place, svc: &Rc<RefCell<Service>>, failing: &str, task: Task) -> Done {
        let (env, m, svc, failing) = (p.env.clone(), p.ctx.machine.clone(), svc.clone(), failing.to_string());
        run_with(task, move |q| {
            let mut s = svc.borrow_mut();
            match q["op"].as_str().unwrap_or("") {
                "connect" if s.state.is_some() => Ok(json!({ "client": 1 })),
                "connect" => Err(json!({ "message": "not running" })),
                "request" if q["method"] == "status" => Ok(s.state.clone().unwrap_or(Value::Null)),
                "request" => {
                    s.stops += 1;
                    s.state = None;
                    Ok(json!({ "stopping": true }))
                }
                "start" => {
                    s.starts.push(q["runtime"].clone());
                    let version = current_component("service", &env, &m).unwrap().unwrap()["version"].clone();
                    if version == failing {
                        return Err(json!({ "message": "XenoNet Service exited (1). See its service log." }));
                    }
                    s.state = Some(json!({ "version": version, "terminals": [] }));
                    Ok(json!("started"))
                }
                "now" => Ok(json!(0)),
                "close" | "pause" | "check" | "sleep" => Ok(Value::Null),
                op => Err(json!({ "message": format!("nothing here answers {op}") })),
            }
        })
    }

    fn bundle_task(p: &Place, bundle: &str) -> Task {
        let (bundle, env, ctx) = (bundle.to_string(), p.env.clone(), p.ctx.clone());
        Task::new(move |io| async move { install_bundle(&io, &bundle, &env, false, &ctx, &Life { stop_timeout: 0.0 }).await })
    }

    fn version(v: &Value) -> &str {
        v["version"].as_str().unwrap()
    }

    #[test]
    fn a_bundle_replaces_a_service_and_tools_from_before_0_4_and_starts_the_new_service() {
        let p = Place::new("bundle-earlier");
        p.earlier();
        let old = p.selected("service");
        assert!(!has_binary(&old, "linux") && !has_binary(&p.selected("tools"), "linux"));
        let svc = Rc::new(RefCell::new(Service::default()));
        let runtime = run(&p, &svc, "", bundle_task(&p, &p.bundle("0.4.0"))).unwrap();
        let (service, tools) = (p.selected("service"), p.selected("tools"));
        assert_eq!((version(&service), version(&tools)), ("0.4.0", "0.4.0"));
        assert!(has_binary(&service, "linux") && has_binary(&tools, "linux"));
        let s = svc.borrow();
        assert_eq!(s.starts.len(), 1, "started once, and checked");
        let launcher = p.runtime().join("launcher");
        assert_eq!(s.starts[0]["env"]["XENONET_LAUNCHER_BIN"], json!(launcher.join("xenonet").to_string_lossy()), "after the Tools, so it names their launcher");
        assert_eq!(runtime["args"], json!(["serve"]));
        let active = p.runtime().join("components/service/active");
        assert_eq!((runtime["root"].clone(), runtime["exec"].clone()), (json!(active.to_string_lossy()), json!(active.join("bin/xenonet").to_string_lossy())));
        assert_eq!(s.starts[0]["exec"], runtime["exec"], "started from its copy");
        assert!(launcher_version(std::fs::read_to_string(launcher.join("cli/xenonet.js")).ok().as_deref()) >= 3.0, "0.3.0's entries now run the binary");
        assert_eq!(std::fs::read_to_string(PathBuf::from(js_string(&old["root"])).join("node")).unwrap(), "0.3.0's node", "the old files kept");
        assert!(!p.runtime().join("service-update.json").exists());
    }

    #[test]
    fn a_running_0_3_service_with_a_terminal_open_runs_on_with_its_node_until_the_next_try() {
        let p = Place::new("bundle-terminal");
        p.earlier();
        let old = p.selected("service");
        let svc = Rc::new(RefCell::new(Service { state: Some(json!({ "version": "0.3.0", "terminals": [{ "id": 1, "exited": false }] })), ..Service::default() }));
        let bundle = p.bundle("0.4.0");
        let runtime = run(&p, &svc, "", bundle_task(&p, &bundle)).unwrap();
        assert_eq!((version(&p.selected("service")), version(&p.selected("tools"))), ("0.3.0", "0.4.0"), "the Tools switch; the service waits");
        assert_eq!((svc.borrow().stops, svc.borrow().starts.len()), (0, 0));
        let root = PathBuf::from(js_string(&old["root"]));
        let launcher = p.runtime().join("launcher");
        assert_eq!(runtime["exec"], json!(root.join("node").to_string_lossy()));
        assert_eq!(runtime["args"], json!([root.join("server/index.js").to_string_lossy()]));
        assert_eq!(runtime["env"]["XENONET_LAUNCHER_EXEC"], json!(launcher.join("node").to_string_lossy()));
        assert_eq!(runtime["env"]["XENONET_LAUNCHER_ROOT"], json!(launcher.to_string_lossy()));

        svc.borrow_mut().state = Some(json!({ "version": "0.3.0", "terminals": [{ "id": 1, "exited": true }] }));
        run(&p, &svc, "", bundle_task(&p, &bundle)).unwrap();
        assert_eq!((svc.borrow().stops, svc.borrow().starts.len(), version(&p.selected("service"))), (1, 1, "0.4.0"));
    }

    #[test]
    fn an_update_that_does_not_start_brings_back_the_0_3_service_with_its_node() {
        let p = Place::new("activate-earlier");
        p.earlier();
        let old = p.selected("service");
        let svc = Rc::new(RefCell::new(Service { state: Some(json!({ "version": "0.3.0", "terminals": [] })), ..Service::default() }));
        let (file, item) = p.package("service", "0.4.0");
        let (env, ctx) = (p.env.clone(), p.ctx.clone());
        let task = Task::new(move |io| async move { activate_package(&io, &file, &item, &env, &ctx, &Life { stop_timeout: 0.0 }).await });
        let err = run(&p, &svc, "0.4.0", task).unwrap_err();
        assert!(err.message().starts_with("The service update failed. The previous version was restored. XenoNet Service exited"), "{}", err.message());
        assert_eq!(version(&p.selected("service")), "0.3.0");
        let s = svc.borrow();
        assert_eq!((s.stops, s.starts.len(), s.state.as_ref().map(|v| v["version"].clone())), (1, 2, Some(json!("0.3.0"))));
        let root = PathBuf::from(js_string(&old["root"]));
        assert_eq!(
            (s.starts[1]["exec"].clone(), s.starts[1]["args"].clone()),
            (json!(root.join("node").to_string_lossy()), json!([root.join("server/index.js").to_string_lossy()]))
        );
        assert!(!p.runtime().join("service-update.json").exists());
        assert!(!p.runtime().join("components/service/active").exists(), "0.3.0 runs from its own folder");
    }

    #[test]
    fn the_service_runs_from_one_folder_through_an_install_updates_and_a_rollback() {
        let p = Place::new("active");
        let svc = Rc::new(RefCell::new(Service::default()));
        run(&p, &svc, "", bundle_task(&p, &p.bundle("0.4.0"))).unwrap();
        let dir = p.runtime().join("components/service");
        let mark = || std::fs::read_to_string(dir.join("active/.from")).unwrap();
        assert_eq!(mark(), format!("0.4.0-{}", js_string(&p.selected("service")["sha256"])));
        let update = |version: &str, failing: &str| {
            let (file, item) = p.package("service", version);
            let (env, ctx) = (p.env.clone(), p.ctx.clone());
            run(&p, &svc, failing, Task::new(move |io| async move { activate_package(&io, &file, &item, &env, &ctx, &Life { stop_timeout: 0.0 }).await }))
        };
        update("0.5.0", "").unwrap();
        assert!(mark().starts_with("0.5.0-"));
        update("0.6.0", "0.6.0").unwrap_err();
        assert!(mark().starts_with("0.5.0-"), "the rollback's copy");
        // What a switch cut short left goes at the next.
        std::fs::create_dir_all(dir.join("active.next/bin")).unwrap();
        std::fs::create_dir_all(dir.join("active.prev")).unwrap();
        update("0.6.1", "").unwrap();
        assert!(mark().starts_with("0.6.1-"));
        let exec = json!(dir.join("active/bin/xenonet").to_string_lossy());
        let s = svc.borrow();
        assert_eq!(s.starts.len(), 4);
        assert!(s.starts.iter().all(|rt| rt["exec"] == exec && rt["root"] == json!(dir.join("active").to_string_lossy())), "{:?}", s.starts);
        let mut left: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with(char::is_numeric)).collect();
        left.sort();
        assert_eq!(left, ["active", "current.json"]);
    }
}
