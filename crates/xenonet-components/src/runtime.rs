//! core/components/runtime.js: where the service's runtime files are, the
//! descriptor that says how to reach it (its port and token), and starting
//! it when it isn't running.

use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_app::fsx::{self, FsError};
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{now_ms, Failure, Io};

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
    let state = xenonet_base::dirs::app_dir("state", var(env, "XDG_STATE_HOME"), var(env, "LOCALAPPDATA"), &m.platform, &m.home, "XenoNet");
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
        return xenonet_app::nodepath::Style::of(&m.platform).join(&[root, bin_name(&m.platform)]);
    }
    this.into()
}

pub fn port_of(env: &Env) -> String {
    set(env, "XENONET_PORT").unwrap_or("47840").to_string()
}

pub fn descriptor_file(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), &format!("service-{}.json", port_of(env))])
}

fn token_re() -> &'static regex::Regex {
    static R: OnceLock<regex::Regex> = OnceLock::new();
    R.get_or_init(|| regex::Regex::new("^[a-f0-9]{64}$").expect("a fixed pattern"))
}

/// The service's descriptor: its port and the token its hello wants.
pub fn read_descriptor(env: &Env, m: &Machine) -> Result<(f64, String), FsError> {
    let file = descriptor_file(env, m);
    let text = fsx::read_text(&file)?;
    let d = xenonet_base::json::parse(&text).map_err(|e| fsx::said(e.to_string()))?;
    let port = d.get("port").and_then(Value::as_f64).filter(|p| p.fract() == 0.0 && (1.0..=65535.0).contains(p));
    let token = d.get("token").map(xenonet_base::text::js_string).filter(|t| token_re().is_match(t));
    match (port, token) {
        (Some(p), Some(t)) => Ok((p, t)),
        _ => Err(fsx::said("Invalid XenoNet service connection file.")),
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
        Some(Value::Array(a)) => a.iter().map(xenonet_base::text::js_string).collect(),
        _ => vec!["serve".into()],
    }
}

/// Starts the service unless one answers: "reused", or "started" once
/// the one started answers. It runs detached, as a standalone service, its
/// output in the runtime folder's service log: `exec` (the xenonet binary)
/// with `args`, in `root`.
///
/// Asks: { op: "connect" } (connect and close again), { op: "spawn", … }
/// and { op: "failure" } (throws what the child's exit or error was).
pub async fn ensure_service(io: &Io, root: &str, exec: &str, args: &[String], env: &Env, timeout: f64, m: &Machine) -> Result<Value, Failure> {
    match io.ask(json!({ "op": "connect", "env": env })).await {
        Ok(_) => return Ok(json!("reused")),
        Err(e) => {
            if e.get("message").and_then(Value::as_str).unwrap_or("").contains("incompatible") {
                return Err(Failure::Rethrow);
            }
        }
    }
    let dir = runtime_dir(env, m);
    fsx::mkdir_p(&dir, Some(0o700))?;
    let log = NATIVE.join(&[&dir, &format!("service-{}.log", port_of(env))]);
    let mut child_env = env.clone();
    child_env.insert("XENONET_STANDALONE".into(), json!("1"));
    child_env.shift_remove("ELECTRON_RUN_AS_NODE");
    child_env.shift_remove("XENONET_EXEC");
    io.ask(json!({ "op": "spawn", "command": exec, "args": args, "cwd": root, "env": child_env, "log": log })).await.map_err(|_| Failure::Rethrow)?;
    let until = now_ms() + timeout;
    while now_ms() < until {
        if io.ask(json!({ "op": "connect", "env": env })).await.is_ok() {
            return Ok(json!("started"));
        }
        io.ask(json!({ "op": "failure" })).await.map_err(|_| Failure::Rethrow)?;
        io.sleep(200.0).await;
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
