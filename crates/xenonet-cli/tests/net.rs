// xenonet net and xenonet svc, against the binary: a service of its own on
// a loopback port the OS picks (its actor gateway on another) with a
// temporary home and state folder, and the command talking to it. Making
// a network and publishing a service are the person's, so where these
// tests run under an agent on macOS the refusal is what's checked; where
// process checks can't tell (Linux, Windows) or the person runs them, the
// network is made, its LAN door checked, a service published and taken
// off, and the network left.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use serde_json::Value;
use std::io::{Read, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// XenoNet's own ports, the person's XenoNet's: never handed to a test.
const XENONET: std::ops::RangeInclusive<u16> = 47840..=47842;

/// `N` free ports on 127.0.0.1, each different. The OS picks each one for
/// a listener on port 0; every listener is held until all the ports are
/// read, so no two are the same, then released for the test to listen on.
/// A fixed port can already be taken, by another test binary or by an
/// outgoing connection, since the ephemeral range those draw from covers
/// most high ports.
fn free_ports<const N: usize>() -> [u16; N] {
    let mut held = Vec::new();
    let mut ports = Vec::new();
    while ports.len() < N {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port on loopback");
        let port = listener.local_addr().expect("a bound listener's address").port();
        if !XENONET.contains(&port) {
            ports.push(port);
        }
        held.push(listener);
    }
    drop(held);
    std::array::from_fn(|i| ports[i])
}

struct Service {
    child: Child,
    dir: PathBuf,
    port: u16,
    gateway_port: u16,
}

impl Service {
    fn start(port: u16, gateway_port: u16) -> Service {
        let dir = std::env::temp_dir().join(format!("xenonet-cli-net-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["home/Projects", "state", "cache", "runtime"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        let child =
            Command::new(env!("CARGO_BIN_EXE_xenonet")).arg("serve").envs(Service::env(&dir, port, gateway_port)).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let s = Service { child, dir, port, gateway_port };
        let end = Instant::now() + Duration::from_secs(30);
        while status(&format!("127.0.0.1:{port}"), "GET", "/api/health", &[]) != 200 {
            assert!(Instant::now() < end, "the service didn't start on {port}");
            std::thread::sleep(Duration::from_millis(100));
        }
        s
    }

    fn env(dir: &std::path::Path, port: u16, gateway_port: u16) -> Vec<(&'static str, String)> {
        let p = |d: &str| dir.join(d).to_string_lossy().into_owned();
        vec![
            ("HOME", p("home")),
            ("XENONET_PORT", port.to_string()),
            ("XENONET_GATEWAY_PORT", gateway_port.to_string()),
            ("XENONET_STATE_DIR", p("state")),
            ("XENONET_CACHE_DIR", p("cache")),
            ("XENONET_RUNTIME_DIR", p("runtime")),
            ("XENONET_PROJECTS_DIR", p("home/Projects")),
            ("XENONET_INSTANCE", "net-test".into()),
        ]
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_xenonet")).args(args).arg("--no-color").envs(Service::env(&self.dir, self.port, self.gateway_port)).output().unwrap();
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
    // The service's API, its actor gateway, and the network's LAN door.
    let [port, gateway_port, lan] = free_ports();
    let s = Service::start(port, gateway_port);
    let lan = format!("127.0.0.1:{lan}");
    let lan = lan.as_str();
    let (code, out, err) = s.run(&["--help"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("net "), "the command is listed: {out}");
    let (code, out, _) = s.run(&["net", "--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("xenonet net join <address> <ticket>"), "{out}");
    let (code, out, _) = s.run(&["svc", "--help"]);
    assert_eq!(code, 0);
    assert!(out.contains("xenonet svc publish <name> http://127.0.0.1:<port> --paths"), "{out}");
    let (code, _, err) = s.run(&["svc", "publish", "files", "http://127.0.0.1:9"]);
    assert_eq!(code, 2, "no paths: {err}");
    let (code, out, err) = s.run(&["net", "create", "--name", "home", "--host", "studio", "--lan", lan, "--json"]);
    if code != 0 {
        assert!(err.contains("yours") || err.contains("agent") || err.contains("Claude Code") || err.contains("Codex"), "refused for what it is: {err}");
        assert_eq!(status(lan, "GET", "/net/v1/roster", &[]), 0, "nothing listens");
        let (code, _, err) = s.run(&["svc", "publish", "files", "http://127.0.0.1:9", "--paths", "/sync/v1/"]);
        assert_eq!(code, 1);
        assert!(err.contains("yours") || err.contains("agent") || err.contains("Claude Code") || err.contains("Codex"), "refused for what it is: {err}");
        return;
    }
    let st: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(st["network"]["name"], "home");
    assert_eq!(st["self"]["id"], "studio");
    assert_eq!(st["gateway"]["kind"], "loopback");
    // The LAN door: the network's routes for its own address alone.
    assert_eq!(status(lan, "GET", "/net/v1/roster", &[]), 401, "unsigned");
    assert_eq!(status(lan, "GET", "/net/v1/roster", &[("Host", "evil.example")]), 421);
    assert_eq!(status(lan, "GET", "/net/v1/roster", &[("Origin", "http://evil.example")]), 403);
    assert_eq!(status(lan, "POST", "/mcp", &[]), 404);
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
    // A service, published, listed and taken off. Programs on other hosts
    // would reach it at /api/svc/studio/files/…; this one alone has no
    // one to call it.
    let (code, out, err) = s.run(&["svc", "publish", "files", "http://127.0.0.1:9", "--paths", "/sync/v1/", "--paths", "/files/"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("/api/svc/studio/files/"), "{out}");
    let (code, _, err) = s.run(&["svc", "publish", "own", &format!("http://127.0.0.1:{}", s.port), "--paths", "/api/"]);
    assert_eq!(code, 1, "never XenoNet's own API");
    assert!(err.contains("XenoNet's own"), "{err}");
    let (code, _, err) = s.run(&["svc", "publish", "far", "http://192.168.1.5:9", "--paths", "/"]);
    assert_eq!(code, 1);
    assert!(err.contains("loopback"), "{err}");
    let (code, out, _) = s.run(&["svc", "list", "--json"]);
    assert_eq!(code, 0);
    let list: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(list["services"][0]["paths"], serde_json::json!(["/files/", "/sync/v1/"]), "{list}");
    let (code, out, _) = s.run(&["svc"]);
    assert_eq!(code, 0);
    assert!(out.contains("files") && out.contains("http://127.0.0.1:9") && out.contains("every host"), "{out}");
    let (code, _, err) = s.run(&["svc", "unpublish", "files"]);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = s.run(&["svc", "unpublish", "files"]);
    assert_eq!(code, 1);
    assert!(err.contains("no service named files"), "{err}");
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
    assert_eq!(status(lan, "GET", "/net/v1/roster", &[]), 0, "the LAN door closed");
}
