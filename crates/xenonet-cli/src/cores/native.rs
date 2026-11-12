//! What the cores ask of this machine: a program run as execFile runs it,
//! a CLI found on PATH, and a GET to a server on this machine. Files are
//! read and written through the service's compose::runtime::fs, with their errors
//! worded as Node words them.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
            let code = xenonet_os::fs::code_of(&e).unwrap_or("ENOENT");
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
    xenonet_os::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut xenonet_os::paths::can_run)
}

/// onPath(cmd) with `canRun` as fs.accessSync(f): any file that's there.
pub fn on_path_existing(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    xenonet_os::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut |f| std::fs::metadata(f).is_ok())
}

/// Date.now().
pub fn now() -> f64 {
    xenonet_text::time::now_ms()
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
    let a = xenonet_api::http::request(port, "GET", &path, &[], None, Some(Duration::from_millis(ms.max(1.0) as u64))).map_err(|e| {
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
}
