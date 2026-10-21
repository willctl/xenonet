// xenonet net, against the binary: a service of its own on 127.0.0.1:48111
// (its actor gateway on 48112) with a temporary home and state folder, and
// the command talking to it. Making a network is the person's, so where
// these tests run under an agent on macOS the refusal is what's checked;
// where process checks can't tell (Linux, Windows) or the person runs them,
// the network is made, its LAN door checked, and left.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::Value;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: &str = "48111";
const GATEWAY_PORT: &str = "48112";
const LAN: &str = "127.0.0.1:48114";

struct Service {
    child: Child,
    dir: PathBuf,
}

impl Service {
    fn start() -> Service {
        let dir = std::env::temp_dir().join(format!("xenonet-cli-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["home/Projects", "state", "cache", "runtime"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let child = Command::new(env!("CARGO_BIN_EXE_xenonet")).arg("serve").envs(Service::env(&dir)).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let s = Service { child, dir };
        let end = Instant::now() + Duration::from_secs(30);
        while status(&format!("127.0.0.1:{PORT}"), "GET", "/api/health", &[]) != 200 {
            assert!(Instant::now() < end, "the service didn't start on {PORT}");
            std::thread::sleep(Duration::from_millis(100));
        }
        s
    }

    fn env(dir: &std::path::Path) -> Vec<(&'static str, String)> {
        let p = |d: &str| dir.join(d).to_string_lossy().into_owned();
        vec![
            ("HOME", p("home")),
            ("XENONET_PORT", PORT.into()),
            ("XENONET_GATEWAY_PORT", GATEWAY_PORT.into()),
            ("XENONET_STATE_DIR", p("state")),
            ("XENONET_CACHE_DIR", p("cache")),
            ("XENONET_RUNTIME_DIR", p("runtime")),
            ("XENONET_PROJECTS_DIR", p("home/Projects")),
            ("XENONET_INSTANCE", "net-test".into()),
        ]
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_xenonet")).args(args).arg("--no-color").envs(Service::env(&self.dir)).output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One request by hand: its status.
fn status(addr: &str, method: &str, path: &str, headers: &[(&str, &str)]) -> u16 {
    let Ok(mut s) = std::net::TcpStream::connect(addr) else { return 0 };
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n", headers.iter().find(|(k, _)| *k == "Host").map_or(addr, |(_, v)| *v));
    for (k, v) in headers.iter().filter(|(k, _)| *k != "Host") {
        text.push_str(k);
        text.push_str(": ");
        text.push_str(v);
        text.push_str("\r\n");
    }
    text.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
    if s.write_all(text.as_bytes()).is_err() {
        return 0;
    }
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

#[test]
fn net_makes_a_network_or_is_refused_from_under_an_agent() {
    let s = Service::start();
    let (code, out, err) = s.run(&["--help"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("net "), "the command is listed: {out}");
    let (code, out, _) = s.run(&["net", "--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("xenonet net join <address> <ticket>"), "{out}");
    let (code, out, err) = s.run(&["net", "create", "--name", "home", "--host", "studio", "--lan", LAN, "--json"]);
    if code != 0 {
        assert!(err.contains("yours") || err.contains("agent") || err.contains("Claude Code") || err.contains("Codex"), "refused for what it is: {err}");
        assert_eq!(status(LAN, "GET", "/net/v1/roster", &[]), 0, "nothing listens");
        return;
    }
    let st: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(st["network"]["name"], "home");
    assert_eq!(st["self"]["id"], "studio");
    assert_eq!(st["gateway"]["kind"], "loopback");
    // The LAN door: the network's routes for its own address alone.
    assert_eq!(status(LAN, "GET", "/net/v1/roster", &[]), 401, "unsigned");
    assert_eq!(status(LAN, "GET", "/net/v1/roster", &[("Host", "evil.example")]), 421);
    assert_eq!(status(LAN, "GET", "/net/v1/roster", &[("Origin", "http://evil.example")]), 403);
    assert_eq!(status(LAN, "POST", "/mcp", &[]), 404);
    let (code, out, _) = s.run(&["net", "status"]);
    assert_eq!(code, 0);
    assert!(out.contains("home") && out.contains("studio"), "{out}");
    let (code, out, _) = s.run(&["net", "hosts", "--json"]);
    assert_eq!(code, 0);
    let hosts: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(hosts.as_array().unwrap().len(), 1);
    let (code, _, err) = s.run(&["net", "create", "--name", "again", "--host", "studio"]);
    assert_eq!(code, 1);
    assert!(err.contains("already"), "{err}");
    if !cfg!(target_os = "macos") {
        let (code, _, err) = s.run(&["net", "add-machine", "laptop"]);
        assert_eq!(code, 1);
        assert!(err.contains("window"), "{err}");
    }
    let (code, _, err) = s.run(&["net", "leave"]);
    assert_eq!(code, 1, "the founder and gateway leaves only with --force");
    assert!(err.contains("--force"), "{err}");
    let (code, out, err) = s.run(&["net", "leave", "--force"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("left home"), "{out}");
    assert_eq!(status(LAN, "GET", "/net/v1/roster", &[]), 0, "the LAN door closed");
}
