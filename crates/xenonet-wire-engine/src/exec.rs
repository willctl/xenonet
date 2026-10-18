//! Child processes as node:child_process's execFile runs them for the
//! cores (ps, lsof, netstat, codesign, git, tmux…): the output, or an error
//! worded as execFile's ("Command failed: ps -axo …", "spawn tmux ENOENT")
//! that still carries what the command wrote, within a time limit and a
//! limit on what's kept of the output.

use serde_json::Value;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use xenonet_engine::peer::Thrown;

/// How a command is run.
#[derive(Clone, Debug)]
pub struct Spec {
    pub file: String,
    pub args: Vec<String>,
    /// Variables set (Some) or taken out (None) of XenoNet's own.
    pub env: Vec<(String, Option<String>)>,
    pub cwd: Option<String>,
    pub timeout: Duration,
    /// What's kept of stdout at most (execFile's maxBuffer).
    pub max: usize,
    /// What goes in on stdin.
    pub input: Option<Vec<u8>>,
}

impl Spec {
    pub fn new(file: &str, args: &[String], timeout_ms: u64) -> Spec {
        Spec { file: file.into(), args: args.to_vec(), env: Vec::new(), cwd: None, timeout: Duration::from_millis(timeout_ms), max: 16 << 20, input: None }
    }
}

/// What a command that failed gave: execFile's error, as its message and
/// code (the exit code, or the system's code when it couldn't start), and
/// what it wrote anyway.
#[derive(Clone, Debug)]
pub struct Failed {
    pub message: String,
    pub code: Option<Value>,
    pub killed: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Failed {
    pub fn thrown(&self) -> Thrown {
        Thrown { message: self.message.clone(), code: self.code.clone(), handle: None, stdout: Some(self.stdout.clone()) }
    }
}

fn reader(r: Option<impl Read + Send + 'static>, max: usize) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut r) = r {
            let mut chunk = [0u8; 8192];
            loop {
                match r.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    // Past the limit, what comes is read and let go, so the
                    // command isn't blocked on a full pipe.
                    Ok(n) if buf.len() < max => buf.extend_from_slice(&chunk[..n.min(max - buf.len())]),
                    Ok(_) => {}
                }
            }
        }
        buf
    })
}

/// Runs a command to its end: its stdout, or why it failed.
pub fn run(spec: &Spec) -> Result<String, Failed> {
    run_full(spec).map(|(out, _)| out)
}

/// Runs a command: its stdout and stderr, or why it failed.
pub fn run_full(spec: &Spec) -> Result<(String, String), Failed> {
    let mut cmd = Command::new(&spec.file);
    cmd.args(&spec.args).stdin(if spec.input.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in &spec.env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    if let Some(d) = &spec.cwd {
        cmd.current_dir(d);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = cmd.spawn().map_err(|e| {
        let code = xenonet_base::fs::code_of(&e);
        Failed {
            message: format!("spawn {} {}", spec.file, code.unwrap_or("EUNKNOWN")),
            code: code.map(|c| Value::String(c.into())),
            killed: false,
            stdout: String::new(),
            stderr: String::new(),
        }
    })?;
    if let (Some(input), Some(mut stdin)) = (spec.input.clone(), child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut stdin, &input);
        });
    }
    let out = reader(child.stdout.take(), spec.max);
    let err = reader(child.stderr.take(), spec.max);
    let deadline = Instant::now() + spec.timeout;
    let mut killed = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if !spec.timeout.is_zero() && Instant::now() >= deadline => {
                let _ = child.kill();
                killed = true;
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(_) => break None,
        }
    };
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).into_owned();
    match status {
        Some(s) if s.success() && !killed => Ok((stdout, stderr)),
        s => {
            let line = std::iter::once(spec.file.as_str()).chain(spec.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
            let code = s.and_then(|s| s.code()).map(Value::from);
            Err(Failed { message: format!("Command failed: {line}\n{stderr}"), code, killed, stdout, stderr })
        }
    }
}

/// Several commands at once, their answers in order.
pub fn run_all(specs: &[Spec]) -> Vec<Result<String, Failed>> {
    if specs.len() <= 1 {
        return specs.iter().map(run).collect();
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = specs.iter().map(|spec| s.spawn(move || run(spec))).collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| Err(Failed { message: "the command's thread failed".into(), code: None, killed: false, stdout: String::new(), stderr: String::new() }))
            })
            .collect()
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_command_gives_its_output_or_fails_as_exec_file_says() {
        assert_eq!(run(&Spec::new("echo", &["hi".into()], 5000)).unwrap(), "hi\n");
        let f = run(&Spec::new("sh", &["-c".into(), "echo out; echo err >&2; exit 3".into()], 5000)).unwrap_err();
        assert_eq!((f.message.as_str(), f.code.clone(), f.stdout.as_str()), ("Command failed: sh -c echo out; echo err >&2; exit 3\nerr\n", Some(Value::from(3)), "out\n"));
        let f = run(&Spec::new("no-such-command-xenonet", &[], 5000)).unwrap_err();
        assert_eq!((f.message.as_str(), f.code), ("spawn no-such-command-xenonet ENOENT", Some(Value::String("ENOENT".into()))));
        let f = run(&Spec::new("sleep", &["5".into()], 50)).unwrap_err();
        assert!(f.killed);
        let all = run_all(&[Spec::new("echo", &["a".into()], 5000), Spec::new("echo", &["b".into()], 5000)]);
        assert_eq!(all.into_iter().map(|r| r.unwrap()).collect::<Vec<_>>(), vec!["a\n", "b\n"]);
    }
}
