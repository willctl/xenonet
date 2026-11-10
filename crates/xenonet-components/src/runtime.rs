//! Where the service's runtime files are, the
//! descriptor that says how to reach it (its port and token), and starting
//! it when it isn't running.

use crate::host::{Spawn, Starter};
use serde_json::{json, Map, Value};
use xenonet_os::error::Failure;
use xenonet_os::files::{self, FsError};
use xenonet_os::nodepath::NATIVE;
use xenonet_text::time::now_ms;

/// The variables of the environment a call was given.
pub type Env = Map<String, Value>;

/// The machine's own: its platform and arch, the home folder, and what
/// process.env says where the env given doesn't apply.
#[derive(Clone, Debug)]
pub struct Machine {
    pub platform: String,
    pub arch: String,
    pub home: String,
}

pub fn var<'a>(env: &'a Env, k: &str) -> Option<&'a str> {
    env.get(k).and_then(Value::as_str)
}

// `env.X || fallback`.
fn set<'a>(env: &'a Env, k: &str) -> Option<&'a str> {
    var(env, k).filter(|v| !v.is_empty())
}

/// The runtime folder: XENONET_RUNTIME_DIR, or runtime in XenoNet's state
/// folder.
pub fn runtime_dir(env: &Env, m: &Machine) -> String {
    if let Some(d) = set(env, "XENONET_RUNTIME_DIR") {
        return d.into();
    }
    let state = xenonet_os::dirs::app_dir("state", var(env, "XDG_STATE_HOME"), var(env, "LOCALAPPDATA"), &m.platform, &m.home, "XenoNet");
    NATIVE.join(&[&state, "runtime"])
}

/// The port the service is for: XENONET_PORT, or 47840.
/// The xenonet binary's file name on `platform`.
pub fn bin_name(platform: &str) -> &'static str {
    if platform == "win32" {
        "xenonet.exe"
    } else {
        "xenonet"
    }
}

/// The launcher's own copy of the binary, which never moves
/// (docs/design/contracts.md, section 12): `<runtime>/launcher/xenonet`.
pub fn launcher_bin(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), "launcher", bin_name(&m.platform)])
}

/// The binary that what XenoNet writes into other programs runs: hooks,
/// MCP servers, the command on PATH, a local host's relay, a tunnel's
/// adapter (runtime.js's xenonetBin). XENONET_BIN when it's set; installed
/// XenoNet's launcher (XENONET_LAUNCHER_BIN, or the binary beside the
/// launcher in XENONET_LAUNCHER_ROOT); else `this`, the running binary.
pub fn xenonet_bin(env: &Env, m: &Machine, this: &str) -> String {
    if let Some(b) = set(env, "XENONET_BIN").or_else(|| set(env, "XENONET_LAUNCHER_BIN")) {
        return b.into();
    }
    if let Some(root) = set(env, "XENONET_LAUNCHER_ROOT") {
        return xenonet_os::nodepath::Style::of(&m.platform).join(&[root, bin_name(&m.platform)]);
    }
    this.into()
}

pub fn port_of(env: &Env) -> String {
    set(env, "XENONET_PORT").unwrap_or("47840").to_string()
}

pub fn descriptor_file(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), &format!("service-{}.json", port_of(env))])
}

/// What `xenonet service stop` leaves beside the descriptor: a supervisor
/// that was waiting for another service to give up the port (one Desktop
/// started before the service was registered) reads it as "stay stopped"
/// rather than starting its own the moment the port is free.
pub fn stop_file(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), &format!("service-{}.stop", port_of(env))])
}

/// The service's descriptor: its port and the token its hello wants.
pub fn read_descriptor(env: &Env, m: &Machine) -> Result<(f64, String), FsError> {
    let file = descriptor_file(env, m);
    let text = files::read_text(&file)?;
    let d = xenonet_text::json::parse(&text).map_err(|e| files::said(e.to_string()))?;
    let port = d.get("port").and_then(Value::as_f64).filter(|p| p.fract() == 0.0 && (1.0..=65535.0).contains(p));
    let token = d.get("token").map(xenonet_text::text::display).filter(|t| crate::packages::sha_re().is_match(t));
    match (port, token) {
        (Some(p), Some(t)) => Ok((p, t)),
        _ => Err(files::said("Invalid XenoNet service connection file.")),
    }
}

/// The hello the service is sent, with the token from its descriptor.
pub fn hello(token: &str, role: &str) -> Value {
    json!({ "token": token, "role": role, "api": crate::protocol::api_value(crate::protocol::SERVICE_API) })
}

/// What the service's binary runs with: `args` when they're given, else
/// `serve`.
pub fn service_args(args: Option<&Value>) -> Vec<String> {
    match args {
        Some(Value::Array(a)) => a.iter().map(xenonet_text::text::display).collect(),
        _ => vec!["serve".into()],
    }
}

/// Starts the service unless one answers: "reused", or "started" once
/// the one started answers. It runs detached, as a standalone service, its
/// output in the runtime folder's service log: `exec` (the xenonet binary)
/// with `args`, in `root`.
pub async fn ensure_service(host: &dyn Starter, root: &str, exec: &str, args: &[String], env: &Env, timeout: f64, m: &Machine) -> Result<Value, Failure> {
    match host.reach(env).await {
        Ok(()) => return Ok(json!("reused")),
        Err(e) if e.message.contains("incompatible") => return Err(e),
        Err(_) => {}
    }
    let dir = runtime_dir(env, m);
    files::mkdir_p(&dir, Some(0o700))?;
    let log = NATIVE.join(&[&dir, &format!("service-{}.log", port_of(env))]);
    let mut child_env = env.clone();
    child_env.insert("XENONET_STANDALONE".into(), json!("1"));
    child_env.shift_remove("ELECTRON_RUN_AS_NODE");
    child_env.shift_remove("XENONET_EXEC");
    host.spawn(&Spawn { command: exec.into(), args: args.to_vec(), cwd: Some(root.into()), env: child_env, log: Some(log) }).await?;
    let until = now_ms() + timeout;
    while now_ms() < until {
        if host.reach(env).await.is_ok() {
            return Ok(json!("started"));
        }
        host.failed().await?;
        host.sleep(200.0).await;
    }
    Err(Failure::msg("XenoNet Service did not start. See its service log."))
}

// Spelled as on a unix host, where Node's path module is posix; on Windows
// the JS tests run against the Rust (test:rust) check the same. (Two
// attributes: clippy takes only a plain cfg(test) for test code.)
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn the_binary_entries_run() {
        let m = Machine { platform: "darwin".into(), arch: "arm64".into(), home: "/h".into() };
        let env = |pairs: &[(&str, &str)]| -> Env { pairs.iter().map(|(k, v)| ((*k).to_string(), json!(v))).collect() };
        assert_eq!(xenonet_bin(&env(&[]), &m, "/c/target/debug/xenonet"), "/c/target/debug/xenonet");
        assert_eq!(xenonet_bin(&env(&[("XENONET_LAUNCHER_ROOT", "/r/launcher")]), &m, "/x"), "/r/launcher/xenonet");
        assert_eq!(xenonet_bin(&env(&[("XENONET_LAUNCHER_BIN", "/l/xenonet"), ("XENONET_LAUNCHER_ROOT", "/r/launcher")]), &m, "/x"), "/l/xenonet");
        assert_eq!(xenonet_bin(&env(&[("XENONET_BIN", "/b"), ("XENONET_LAUNCHER_BIN", "/l/xenonet")]), &m, "/x"), "/b");
        let win = Machine { platform: "win32".into(), ..m };
        assert_eq!(xenonet_bin(&env(&[("XENONET_LAUNCHER_ROOT", r"C:\r\launcher")]), &win, "/x"), r"C:\r\launcher\xenonet.exe");
        assert_eq!(launcher_bin(&env(&[("XENONET_RUNTIME_DIR", "/r")]), &Machine { platform: "darwin".into(), arch: "arm64".into(), home: "/h".into() }), "/r/launcher/xenonet");
    }
}
