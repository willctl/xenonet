//! core/delivery/setup.js for `xenonet doctor --live`: what live delivery
//! needs on this machine (xenonet-engine's delivery setup), with the
//! commands, files and OpenCode servers it reads done here.

use super::native::{local_get, now, run};
use crate::out::Fail;
use crate::sys;
use serde_json::Value;
use xenonet_engine::delivery::{opencode, setup, Files};
use xenonet_service::compose::runtime::fs;

/// How long a session listens after its turns (LISTEN_SECONDS).
const LISTEN_SECONDS: f64 = 300.0;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_string()).collect()
}

/// What the live setup asks for: a command's output (trimmed, or None;
/// through the shell on Windows, for a .cmd), the OpenCode servers
/// running, and the files for the listen settings.
struct Setup;

impl setup::Host for Setup {
    fn exec(&mut self, cmd: &str, args: &[&str]) -> Option<String> {
        let r = if cfg!(windows) { run("cmd.exe", &[strings(&["/d", "/s", "/c", cmd]), strings(args)].concat(), 5000.0, None) } else { run(cmd, &strings(args), 5000.0, None) };
        r.ok().then(|| xenonet_text::text::trim(&r.stdout).to_string())
    }
    fn servers(&mut self) -> Vec<opencode::Server> {
        opencode::OpenCode::new(10e3).servers(&mut OpenCode)
    }
}

impl Files for Setup {
    fn read_file(&mut self, path: &str) -> Result<String, String> {
        fs::read_text(path).map_err(|e| e.message)
    }
    fn mkdir_all(&mut self, dir: &str) -> Result<(), String> {
        fs::mkdir_all(dir).map_err(|e| e.message)
    }
    fn write_file(&mut self, path: &str, body: &str, mode: Option<u32>) -> Result<(), String> {
        fs::write_file(path, body.as_bytes(), mode).map_err(|e| e.message)
    }
    fn append_file(&mut self, path: &str, body: &str, mode: Option<u32>) -> Result<(), String> {
        fs::append_file(path, body.as_bytes(), mode).map_err(|e| e.message)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        fs::rename(from, to).map_err(|e| e.message)
    }
}

/// What finding OpenCode's servers asks for: the time, the OpenCode
/// processes (through ps), the ports one listens on (lsof, netstat or
/// /proc) and a GET to one.
struct OpenCode;

impl opencode::System for OpenCode {
    /// A command's output, or nothing when it fails.
    fn run(&mut self, cmd: &str, args: &[&str]) -> String {
        let r = run(cmd, &strings(args), 3000.0, None);
        if r.ok() {
            r.stdout
        } else {
            String::new()
        }
    }
    fn read_file(&mut self, path: &str) -> Option<String> {
        fs::read_text(path).ok()
    }
    fn readdir(&mut self, path: &str) -> Option<Vec<String>> {
        fs::read_dir(path).ok()
    }
    fn readlink(&mut self, path: &str) -> Option<String> {
        fs::read_link(path).ok()
    }
}

impl opencode::Host for OpenCode {
    fn now(&mut self) -> f64 {
        now()
    }
    fn processes(&mut self) -> Vec<opencode::Proc> {
        opencode::open_code_processes(self, cfg!(windows))
    }
    fn ports(&mut self, pid: f64) -> Vec<f64> {
        opencode::listening_ports(self, pid, sys::platform())
    }
    /// A GET's JSON, from an answer that's ok.
    fn get(&mut self, url: &str, ms: f64) -> Option<Value> {
        let (status, body) = local_get(url, ms).ok()?;
        if !(200..300).contains(&status) {
            return None;
        }
        xenonet_text::json::parse(&String::from_utf8_lossy(&body)).ok()
    }
    fn post(&mut self, _: &str, _: &str) -> Result<opencode::Posted, String> {
        Err("the doctor posts nothing to OpenCode".into())
    }
}

/// writeListenSettings({ dir, port }): the file's path.
pub fn write_listen_settings(dir: &str, port: &Value) -> Result<String, Fail> {
    setup::write_listen_settings(&mut Setup, dir, xenonet_text::text::to_number(Some(port)), LISTEN_SECONDS).map_err(|e| Fail::error(e.0))
}

/// liveSetup({ port, listenFile })
pub fn live_setup(port: &Value, listen_file: Option<&str>) -> Result<Value, Fail> {
    Ok(setup::live_setup(&mut Setup, xenonet_text::text::to_number(Some(port)), listen_file, &sys::home()))
}
