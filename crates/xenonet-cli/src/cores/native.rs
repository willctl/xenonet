//! What the cores ask of this machine, done natively: what a twin's JS did
//! with node:child_process, node:fs and fetch. A program runs as execFile
//! runs it, a CLI is found on PATH, and a file is read or written as
//! node:fs does, with its errors worded as Node words them.

use serde_json::Value;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use xenonet_base::fs::FsError;
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::Arg;

/// What a program said: its exit code (None when it couldn't start, or
/// was ended at its time limit), its output, and why it failed, worded as
/// execFile's error is.
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    /// err.message, when execFile would have given an error.
    pub error: Option<String>,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// execFile(file, args, { timeout, windowsHide, env without
/// ELECTRON_RUN_AS_NODE }), with `input` on its standard input when given.
pub fn run(file: &str, args: &[String], timeout_ms: f64, input: Option<&str>) -> Ran {
    run_in(file, args, timeout_ms, input, None, &[])
}

/// run() in a folder, with these variables set besides the process's own.
pub fn run_in(file: &str, args: &[String], timeout_ms: f64, input: Option<&str>, cwd: Option<&str>, env: &[(String, String)]) -> Ran {
    let mut cmd = Command::new(file);
    cmd.args(args).env_remove("ELECTRON_RUN_AS_NODE").stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    hide_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let code = xenonet_base::fs::code_of(&e).unwrap_or("ENOENT");
            return Ran { code: None, stdout: String::new(), stderr: String::new(), error: Some(format!("spawn {file} {code}")) };
        }
    };
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = stdin.write_all(text.as_bytes());
        });
    }
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut out);
            }
            String::from_utf8_lossy(&out).into_owned()
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let limit = (timeout_ms.is_finite() && timeout_ms > 0.0).then(|| Duration::from_millis(timeout_ms as u64));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if limit.is_some_and(|l| started.elapsed() >= l) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(_) => break None,
        }
    };
    let (stdout, stderr) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
    let code = status.and_then(|s| s.code());
    let shown = std::iter::once(file.to_string()).chain(args.iter().cloned()).collect::<Vec<_>>().join(" ");
    let error = match status {
        Some(s) if s.success() => None,
        _ => Some(format!("Command failed: {shown}\n{stderr}")),
    };
    Ran { code, stdout, stderr, error }
}

#[cfg(windows)]
fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    // CREATE_NO_WINDOW, as windowsHide does.
    cmd.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn hide_window(_: &mut Command) {}

/// onPath(cmd): the first `cmd` on PATH that can run, or None.
pub fn on_path(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    xenonet_base::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut xenonet_base::paths::can_run)
}

/// onPath(cmd) with `canRun` as fs.accessSync(f): any file that's there.
pub fn on_path_existing(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    xenonet_base::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut |f| std::fs::metadata(f).is_ok())
}

/// Date.now().
pub fn now() -> f64 {
    xenonet_app::task::now_ms()
}

/// A JSON value as an argument to a core.
pub fn arg_of(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::Text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(arg_of).collect()),
        Value::Object(m) => Arg::Obj(m.iter().map(|(k, v)| (k.clone(), arg_of(v))).collect()),
    }
}

/// An argument's text, as String(v) has it.
pub fn text(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

fn thrown(e: FsError) -> Thrown {
    Thrown { message: e.message, code: e.code.map(|c| Box::new(Value::String(c))), handle: None, stdout: None }
}

/// A node:fs call a core makes, synchronously: `fs.<method>(...args)`.
#[expect(clippy::result_large_err, reason = "what a core's Calls answer, which throws a Thrown")]
pub fn fs_call(method: &str, args: &[Arg]) -> Result<Arg, Thrown> {
    let p = |i: usize| args.get(i).map(text).unwrap_or_default();
    let of = |e: std::io::Error, syscall: &str, path: &str| thrown(FsError::of(e, syscall, path));
    match method {
        "existsSync" => Ok(Arg::Bool(xenonet_app::fsx::exists(&p(0)))),
        "readFileSync" => {
            let file = p(0);
            let bytes = std::fs::read(&file).map_err(|e| of(e, "open", &file))?;
            // With an encoding, text; without, the bytes.
            Ok(if args.len() > 1 && !args[1].is_nullish() { Arg::Text(String::from_utf8_lossy(&bytes).into_owned()) } else { Arg::Bytes(bytes) })
        }
        "writeFileSync" => {
            let file = p(0);
            let data = match args.get(1) {
                Some(Arg::Bytes(b)) => b.clone(),
                Some(a) => text(a).into_bytes(),
                None => Vec::new(),
            };
            std::fs::write(&file, data).map_err(|e| of(e, "open", &file))?;
            Ok(Arg::Undef)
        }
        "mkdirSync" => {
            let dir = p(0);
            let recursive = args.get(1).is_some_and(|o| o.field("recursive").truthy());
            let r = if recursive { std::fs::create_dir_all(&dir) } else { std::fs::create_dir(&dir) };
            r.map_err(|e| of(e, "mkdir", &dir))?;
            Ok(Arg::Undef)
        }
        "copyFileSync" => {
            let (from, to) = (p(0), p(1));
            std::fs::copy(&from, &to).map_err(|e| of(e, "copyfile", &format!("{from}' -> '{to}")))?;
            Ok(Arg::Undef)
        }
        "renameSync" => {
            let (from, to) = (p(0), p(1));
            std::fs::rename(&from, &to).map_err(|e| of(e, "rename", &format!("{from}' -> '{to}")))?;
            Ok(Arg::Undef)
        }
        "readdirSync" => {
            let dir = p(0);
            let names = xenonet_base::paths::read_dir_sorted(std::path::Path::new(&dir)).map_err(|e| of(e, "scandir", &dir))?;
            Ok(Arg::List(names.iter().map(|e| Arg::Text(e.file_name().to_string_lossy().into_owned())).collect()))
        }
        "readlinkSync" => {
            let link = p(0);
            let to = std::fs::read_link(&link).map_err(|e| of(e, "readlink", &link))?;
            Ok(Arg::Text(to.to_string_lossy().into_owned()))
        }
        other => Err(Thrown::new(format!("fs.{other} isn't one XenoNet's cores call"))),
    }
}

/// A GET to a server on this machine, within `ms`: its status and body.
/// Err with fetch's words when nothing answered.
pub fn local_get(url: &str, ms: f64) -> Result<(u16, Vec<u8>), String> {
    let parsed = url::Url::parse(url).map_err(|_| "Failed to parse URL".to_string())?;
    let host = parsed.host_str().unwrap_or("");
    if parsed.scheme() != "http" || !matches!(host, "127.0.0.1" | "localhost") {
        return Err(format!("fetch failed: XenoNet reads only servers on this machine here, not {url}"));
    }
    let port = parsed.port().unwrap_or(80);
    let path = match parsed.query() {
        Some(q) => format!("{}?{q}", parsed.path()),
        None => parsed.path().to_string(),
    };
    let a = xenonet_base::http::request(port, "GET", &path, &[], None, Some(Duration::from_millis(ms.max(1.0) as u64))).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut || e.kind() == std::io::ErrorKind::WouldBlock {
            "The operation was aborted due to timeout".to_string()
        } else {
            "fetch failed".to_string()
        }
    })?;
    let body = a.body.map_err(|_| "fetch failed".to_string())?;
    Ok((a.status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_run_as_exec_file_runs_them() {
        if cfg!(windows) {
            return;
        }
        let r = run("/bin/sh", &["-c".into(), "printf out; printf err >&2; exit 3".into()], 5000.0, None);
        assert_eq!((r.code, r.stdout.as_str(), r.stderr.as_str()), (Some(3), "out", "err"));
        assert!(r.error.as_deref().is_some_and(|e| e.starts_with("Command failed: /bin/sh -c")));
        let slow = run("/bin/sh", &["-c".into(), "sleep 5".into()], 100.0, None);
        assert!(slow.code.is_none() && slow.error.is_some());
        let fed = run("/bin/cat", &[], 5000.0, Some("fed"));
        assert_eq!(fed.stdout, "fed");
        let none = run("/no/such/program", &[], 1000.0, None);
        assert_eq!(none.error.as_deref(), Some("spawn /no/such/program ENOENT"));
    }

    #[test]
    fn files_as_node_fs_has_them() {
        let dir = std::env::temp_dir().join(format!("xenonet-cli-native-{}", std::process::id()));
        let d = dir.to_string_lossy().into_owned();
        let f = format!("{d}/a/b.json");
        fs_call("mkdirSync", &[Arg::text(format!("{d}/a")), Arg::Obj(vec![("recursive".into(), Arg::Bool(true))])]).unwrap();
        fs_call("writeFileSync", &[Arg::text(f.clone()), Arg::text("{}")]).unwrap();
        assert_eq!(fs_call("readFileSync", &[Arg::text(f.clone()), Arg::text("utf8")]).unwrap(), Arg::text("{}"));
        assert_eq!(fs_call("existsSync", &[Arg::text(f.clone())]).unwrap(), Arg::Bool(true));
        let gone = fs_call("readFileSync", &[Arg::text(format!("{d}/none")), Arg::text("utf8")]).unwrap_err();
        assert_eq!(gone.code, Some(Value::String("ENOENT".into())));
        assert!(gone.message.starts_with("ENOENT: no such file or directory, open"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
