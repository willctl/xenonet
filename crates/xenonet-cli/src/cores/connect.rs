//! core/engine/connect.js for the command: what XenoNet adds to Claude
//! Code's and Codex's settings on this computer, shown first and taken out
//! again (xenonet-engine's connect), with the files and the CLIs it reads
//! done here, as the twin's createConnect did them.

use super::native::{fs_call, on_path_existing, run, text};
use crate::out::Fail;
use crate::sys;
use serde_json::Value;
use xenonet_engine::connect::{Connect, Instance, Options};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;

/// What the core asks for: "fs" (node:fs), "cli" (a CLI on PATH, or null)
/// and "run" (a CLI's output, or what it wrote to stderr, thrown).
struct Machine;

impl Calls for Machine {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("fs", m) => fs_call(m, &args),
            ("", "cli") => Ok(on_path_existing(&args.first().map(text).unwrap_or_default()).map_or(Arg::Null, Arg::Text)),
            ("", "run") => {
                let file = args.first().map(text).unwrap_or_default();
                let argv: Vec<String> = match args.get(1) {
                    Some(Arg::List(l)) => l.iter().map(text).collect(),
                    _ => Vec::new(),
                };
                let r = run(&file, &argv, 20000.0, None);
                match r.error {
                    None => Ok(Arg::Text(r.stdout)),
                    Some(message) => {
                        let said = xenonet_base::text::js_trim(&r.stderr).to_string();
                        Err(Thrown::new(if said.is_empty() { message } else { said }))
                    }
                }
            }
            _ => Err(Thrown::new(format!("connect asked for {path}.{method}"))),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Err(Thrown::new(format!("connect asked for {path}")))
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("undefined".into())
    }
}

/// createConnect({ port, bin }) as the command makes it, for this
/// computer's own settings: its home folder, CLAUDE_CONFIG_DIR, CODEX_HOME
/// and the instances XenoNet Dev runs as.
fn core(opts: &Value) -> Connect {
    let t = |k: &str| opts.get(k).map(xenonet_base::text::js_string).unwrap_or_default();
    Connect::new(Options {
        port: opts.get("port").map_or(Arg::Undef, super::native::arg_of),
        demo: Arg::Bool(false),
        bin: t("bin"),
        home: sys::home(),
        claude_config_dir: std::env::var("CLAUDE_CONFIG_DIR").ok(),
        codex_home: std::env::var("CODEX_HOME").ok(),
        console_mcp: sys::var("XENONET_LAUNCHER_BIN").or_else(|| sys::var("XENONET_LAUNCHER_EXEC")).is_some(),
        platform: sys::platform().into(),
        instances: xenonet_app::instances::INSTANCES.iter().map(|i| Instance { id: i.id.into(), name: i.name.into(), port: Arg::Num(f64::from(i.port)) }).collect(),
    })
}

pub fn preview(opts: &Value) -> std::result::Result<Value, Fail> {
    core(opts).preview(&mut Machine).map_err(super::thrown)
}

pub fn apply(opts: &Value, apps: &[String]) -> std::result::Result<Value, Fail> {
    let apps: Vec<Arg> = apps.iter().map(|a| Arg::Text(a.clone())).collect();
    core(opts).apply(&mut Machine, &apps).map_err(super::thrown)
}

pub fn remove(opts: &Value) -> std::result::Result<Value, Fail> {
    core(opts).remove(&mut Machine).map_err(super::thrown)
}
