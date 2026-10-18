//! core/delivery/setup.js for `xenonet doctor --live`: what live delivery
//! needs on this machine (xenonet-engine's delivery setup), with the
//! commands, files and OpenCode servers it reads done here, as the twin's
//! liveSetup and createOpenCode did them.

use super::native::{fs_call, local_get, now, run, text};
use crate::out::Fail;
use crate::sys;
use serde_json::Value;
use xenonet_engine::delivery::{opencode, setup};
use xenonet_engine::peer::{Calls, Result, Thrown};
use xenonet_engine::sql::Arg;

/// How long a session listens after its turns (LISTEN_SECONDS).
const LISTEN_SECONDS: f64 = 300.0;

fn argv(a: Option<&Arg>) -> Vec<String> {
    match a {
        Some(Arg::List(l)) => l.iter().map(text).collect(),
        _ => Vec::new(),
    }
}

/// What liveSetup asks for: "exec" (a command's output, trimmed, or null;
/// through the shell on Windows, for a .cmd), "servers" (the OpenCode
/// servers running), and "fs" for the listen settings.
struct Setup;

impl Calls for Setup {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("fs", m) => fs_call(m, &args),
            ("", "exec") => {
                let (file, rest) = (args.first().map(text).unwrap_or_default(), argv(args.get(1)));
                let r =
                    if cfg!(windows) { run("cmd.exe", &[vec!["/d".into(), "/s".into(), "/c".into(), file], rest].concat(), 5000.0, None) } else { run(&file, &rest, 5000.0, None) };
                Ok(if r.ok() { Arg::Text(xenonet_base::text::js_trim(&r.stdout).to_string()) } else { Arg::Null })
            }
            ("", "servers") => {
                let servers = opencode::OpenCode::new(10e3).servers(&mut OpenCode)?;
                Ok(Arg::List(servers.iter().map(opencode::Server::to_arg).collect()))
            }
            _ => Err(Thrown::new(format!("live setup asked for {path}.{method}"))),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Err(Thrown::new(format!("live setup asked for {path}")))
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("undefined".into())
    }
}

/// What createOpenCode's servers() asks for: the time, the OpenCode
/// processes (through ps), the ports one listens on (lsof, netstat or
/// /proc) and a GET to one.
struct OpenCode;

impl Calls for OpenCode {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let windows = cfg!(windows);
        match (path, method) {
            ("", "now") => Ok(Arg::Num(now())),
            ("", "processes") => Ok(Arg::List(opencode::open_code_processes(self, windows)?.iter().map(opencode::Proc::to_arg).collect())),
            ("", "ports") => {
                let pid = args.first().and_then(Arg::as_f64).unwrap_or(f64::NAN);
                Ok(Arg::List(opencode::listening_ports(self, pid, sys::platform())?.into_iter().map(Arg::Num).collect()))
            }
            // A command's output, or nothing when it fails.
            ("", "ps" | "exec") => {
                let r = run(&args.first().map(text).unwrap_or_default(), &argv(args.get(1)), 3000.0, None);
                Ok(Arg::Text(if r.ok() { r.stdout } else { String::new() }))
            }
            ("", "readdir") => fs_call("readdirSync", &args),
            ("", "readlink") => fs_call("readlinkSync", &args),
            ("", "readFile") => fs_call("readFileSync", &args),
            // A GET, as { ok, status, json }; JSON read only from an answer
            // that's ok.
            ("", "get") => {
                let url = args.first().map(text).unwrap_or_default();
                let ms = args.get(1).and_then(Arg::as_f64).unwrap_or(3000.0);
                let (status, body) = local_get(&url, ms).map_err(Thrown::new)?;
                let ok = (200..300).contains(&status);
                let mut out = vec![("ok".to_string(), Arg::Bool(ok)), ("status".to_string(), Arg::Num(f64::from(status)))];
                if ok {
                    let json = xenonet_base::json::parse(&String::from_utf8_lossy(&body)).map_err(|e| Thrown::new(e.to_string()))?;
                    out.push(("json".into(), super::native::arg_of(&json)));
                }
                Ok(Arg::Obj(out))
            }
            _ => Err(Thrown::new(format!("OpenCode's servers asked for {path}.{method}"))),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Err(Thrown::new(format!("OpenCode's servers asked for {path}")))
    }
    fn kind(&mut self, _: &str) -> Result<String> {
        Ok("undefined".into())
    }
}

/// writeListenSettings({ dir, port }): the file's path.
pub fn write_listen_settings(dir: &str, port: &Value) -> std::result::Result<String, Fail> {
    setup::write_listen_settings(&mut Setup, dir, &super::native::arg_of(port), &Arg::Num(LISTEN_SECONDS)).map_err(super::thrown)
}

/// liveSetup({ port, listenFile })
pub fn live_setup(port: &Value, listen_file: Option<&str>) -> std::result::Result<Value, Fail> {
    setup::live_setup(&mut Setup, &super::native::arg_of(port), &Arg::opt_text(listen_file), &Arg::Text(sys::home())).map_err(super::thrown)
}
