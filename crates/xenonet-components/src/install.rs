//! core/components/install.js: installing and switching components. A
//! bundle installs what's missing and never upgrades what's there; a
//! runtime that fails its startup check is never selected; a Tools update
//! leaves the service alone; a service update stops the old one only when
//! no terminal is open in it, starts the new one and checks it answers as
//! the version it should be, and rolls back if it doesn't. A record of the
//! switch is kept until the new version answers, so an updater that
//! crashed can finish it, or restore the previous one, on the next launch.
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

// The node an earlier XenoNet copied into the launcher's folder, which the
// entries it wrote run.
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

/// Copies the Tools' binary to the launcher's, when it isn't the copy of
/// these Tools already (its .from file says which it is), through a
/// temporary file and a rename, so a hook starting meanwhile runs the old
/// or the new. One in use that can't be replaced now (on Windows) is
/// replaced next time.
pub fn write_launcher_bin(tools: &Value, to: &str, platform: &str) -> bool {
    let from = NATIVE.join(&[&js_string(&tools["root"]), "bin", bin_name(platform)]);
    if !fsx::exists(&from) {
        return false;
    }
    let source = format!("{}-{}", js_string(&tools["version"]), js_string(&tools["sha256"]));
    let marker = format!("{to}.from");
    if fsx::read_text(&marker).is_ok_and(|had| had == source) && fsx::exists(to) {
        return false;
    }
    let tmp = format!("{to}.{}.tmp", std::process::id());
    let copied = std::fs::copy(&from, &tmp).is_ok() && {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).is_ok()
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if copied && fsx::rename(&tmp, to).is_ok() {
        let _ = fsx::write_file(&marker, source.as_bytes(), None);
        return true;
    }
    let _ = fsx::rm_rf(&tmp);
    false
}

/// The launcher, in the runtime folder, which never moves (hooks, MCP
/// servers and the xenonet command run its path): the Tools' xenonet
/// binary, copied. A node launcher an earlier XenoNet put beside it, which
/// the entries it wrote still run, gets this XenoNet's script, which runs
/// the binary; XenoNet never puts a node there itself. None without Tools.
pub fn install_launcher(env: &Env, ctx: &Ctx) -> Result<Option<Value>, Failure> {
    let m = &ctx.machine;
    let Some(tools) = current_component("tools", env, m)? else { return Ok(None) };
    let dir = NATIVE.join(&[&runtime_dir(env, m), "launcher"]);
    let bin = launcher_bin(env, m);
    fsx::mkdir_p(&dir, None)?;
    write_launcher_bin(&tools, &bin, &m.platform);
    let entry = NATIVE.join(&[&dir, "cli", "xenonet.js"]);
    if fsx::exists(&NATIVE.join(&[&dir, earlier_node(&m.platform)])) && fsx::exists(&entry) {
        write_launcher(&entry, LAUNCHER_SCRIPT)?;
    }
    let bin = if fsx::exists(&bin) { json!(bin) } else { Value::Null };
    Ok(Some(json!({ "bin": bin, "root": dir })))
}

// Runs a program as spawnSync did: its output, and its exit code (None if
// it couldn't start, or was ended at the timeout).
fn run_sync(file: &str, args: &[String], cwd: &str, timeout: Duration) -> (Option<i32>, String) {
    use std::io::Read;
    let child = std::process::Command::new(file)
        .args(args)
        .current_dir(cwd)
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
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

/// How to start the selected service: its xenonet binary, with `serve`,
/// and the env it runs with (the runtime folder, and the launcher's binary
/// when there is one).
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
    Ok(json!({ "root": root, "exec": NATIVE.join(&[&root, "bin", bin_name(&m.platform)]), "args": ["serve"], "env": run_env }))
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

fn restore_selection(previous: Option<&Value>, env: &Env, m: &Machine) -> Result<(), FsError> {
    match previous.filter(|p| truthy(Some(p))) {
        Some(p) => select_component("service", p, env, m),
        None => fsx::rm_rf(&NATIVE.join(&[&runtime_dir(env, m), "components/service/current.json"])),
    }
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
    restore_selection(previous, env, m)?;
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

/// Installs a bundle's components that aren't installed yet (never
/// upgrading what's there), with the launcher and, when asked, the
/// xenonet command; says how to start the service.
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
    for item in items {
        if item.get("platform").and_then(Value::as_str) != Some(m.platform.as_str()) || item.get("arch").and_then(Value::as_str) != Some(m.arch.as_str()) {
            continue;
        }
        let component = item.get("component").map(js_string).unwrap_or_default();
        // A bundle never silently restarts or upgrades the service or Tools.
        if current_component(&component, env, m)?.is_some() {
            continue;
        }
        let file = NATIVE.join(&[bundle, &item.get("file").map(js_string).unwrap_or_default()]);
        let staged = stage_package(io, &file, item, env, m).await?;
        check_runtime(&staged, &m.platform)?;
        select_component(&component, &staged, env, m)?;
    }
    let launcher = install_launcher(env, ctx)?;
    if let (true, Some(bin)) = (command, launcher.as_ref().and_then(|l| l["bin"].as_str())) {
        let dir = xenonet_app::command::command_dir(&m.home, &m.platform, ctx.local_app_data.as_deref());
        xenonet_app::command::install_command(&dir, bin, false, crate::runtime::var(env, "PATH"), &m.platform)?;
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
        install_launcher(env, ctx)?;
        return Ok(
            json!({ "component": "tools", "version": staged["version"], "status": "updated", "detail": "New commands use this version. Connected MCP processes keep running." }),
        );
    }
    let state = running(io, env).await?;
    if truthy(Some(&state)) && open_terminals(&state)? {
        return Ok(json!({ "component": "service", "version": staged["version"], "status": "staged", "detail": "Close running terminals, then update the service." }));
    }
    write_journal(env, m, old.as_ref(), &staged)?;
    let switched: Result<(), Failure> = async {
        if truthy(Some(&state)) {
            stop_service(io, env, life).await?;
        }
        select_component("service", &staged, env, m)?;
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
            if truthy(Some(&live)) && live.get("version") != old.as_ref().and_then(|o| o.get("version")) {
                stop_service(io, env, life).await?;
            }
            restore_selection(old.as_ref(), env, m)?;
            if let Some(o) = &old {
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

    #[test]
    fn launcher_versions() {
        assert_eq!(launcher_version(Some("// xenonet-launcher 7\nrest")), 7.0);
        assert_eq!(launcher_version(Some("// an old one")), 1.0);
        assert_eq!(launcher_version(None), 0.0);
        assert_eq!(launcher_version(Some(LAUNCHER_SCRIPT)), 3.0, "the script built in");
    }
}
