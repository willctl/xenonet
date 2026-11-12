//! Connecting the agents from the command: what XenoNet adds to Claude
//! Code's, Codex's and Grok Build's settings on this computer, shown first
//! and taken out again (xenonet-adapter's connect, through the adapters
//! in place), with the files and the CLIs it reads done here.

use super::native::{on_path_existing, run};
use crate::out::Fail;
use crate::sys;
use serde_json::Value;
use xenonet_adapter::connect::{Connect, Instance, Machine, Options, Problem};
use xenonet_os::fs::FsError;
use xenonet_runtime::fs;

/// This computer: its files, and the agents' CLIs on PATH.
struct Here;

fn problem(e: FsError) -> Problem {
    Problem { message: e.message, code: e.code }
}

impl Machine for Here {
    fn read_text(&mut self, file: &str) -> Result<String, Problem> {
        fs::read_text(file).map_err(problem)
    }
    fn write_text(&mut self, file: &str, text: &str) -> Result<(), Problem> {
        fs::write_file(file, text.as_bytes(), None).map_err(problem)
    }
    fn exists(&mut self, path: &str) -> bool {
        fs::exists(path)
    }
    fn make_dir(&mut self, dir: &str) -> Result<(), Problem> {
        fs::mkdir_all(dir).map_err(problem)
    }
    fn copy(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs::copy(from, to).map_err(problem)
    }
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Problem> {
        fs::rename(from, to).map_err(problem)
    }
    fn remove(&mut self, file: &str) -> Result<(), Problem> {
        fs::remove_if_there(file).map_err(problem)
    }
    fn cli(&mut self, name: &str) -> Option<String> {
        on_path_existing(name)
    }
    /// A CLI's output, or what it wrote to stderr as the problem.
    fn run(&mut self, bin: &str, args: &[String]) -> Result<String, Problem> {
        let r = run(bin, args, 20000.0, None);
        match r.error {
            None => Ok(r.stdout),
            Some(message) => {
                let said = xenonet_text::text::trim(&r.stderr).to_string();
                Err(Problem::new(if said.is_empty() { message } else { said }))
            }
        }
    }
}

/// A connect for this computer's own settings, with the port and binary
/// the command was given: its home folder, CLAUDE_CONFIG_DIR, CODEX_HOME,
/// GROK_HOME and the instances XenoNet Dev runs as.
fn core(opts: &Value) -> Connect {
    let t = |k: &str| opts.get(k).map(xenonet_text::text::display).unwrap_or_default();
    Connect::new(Options {
        port: xenonet_text::text::to_number(opts.get("port")),
        demo: false,
        bin: t("bin"),
        home: sys::home(),
        claude_config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
        codex_home: std::env::var("CODEX_HOME").ok(),
        grok_home: std::env::var("GROK_HOME").ok(),
        console_mcp: sys::var("XENONET_LAUNCHER_BIN").or_else(|| sys::var("XENONET_LAUNCHER_EXEC")).is_some(),
        platform: sys::platform().into(),
        instances: xenonet_api::instances::INSTANCES.iter().map(|i| Instance { id: i.id.into(), name: i.name.into(), port: f64::from(i.port) }).collect(),
    })
}

fn fail(p: Problem) -> Fail {
    Fail::error(p.message)
}

pub fn preview(opts: &Value) -> Result<Value, Fail> {
    core(opts).preview(&mut Here).map_err(fail)
}

pub fn apply(opts: &Value, apps: &[String]) -> Result<Value, Fail> {
    core(opts).apply(&mut Here, apps).map_err(fail)
}

pub fn remove(opts: &Value) -> Result<Value, Fail> {
    core(opts).remove(&mut Here).map_err(fail)
}
