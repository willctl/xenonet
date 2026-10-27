//! The runner (core/delivery/runs.js): headless turns of Claude Code and
//! Codex that this machine starts, for a session it resumes. What runs,
//! quoted how, and whether it may start now are xenonet-engine's
//! (`delivery::runs`); the processes, their logs in `runs/` and the
//! records in delivery-runs.json are here.

use crate::js::{now_ms, platform, Locked};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenonet_base::dirs::{self, CliEnv, Look, Unreadable};
use xenonet_engine::delivery::runs::{self, Run};

/// The disk as it is.
struct Disk;

impl Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        Path::new(p).exists()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, Unreadable> {
        let names = std::fs::read_dir(p).map_err(|_| Unreadable)?;
        let mut out: Vec<String> = names.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
        out.sort();
        Ok(out)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)
    }
}

/// What this process's environment says of the CLIs.
fn cli_env() -> CliEnv {
    let var = |k: &str| std::env::var(k).ok();
    CliEnv {
        path: var("PATH").unwrap_or_default(),
        pathext: var("PATHEXT"),
        claude_bin: var("XENONET_CLAUDE_BIN"),
        codex_bin: var("XENONET_CODEX_BIN"),
        dirs: dirs::Env {
            claude_desktop_dir: var("XENONET_CLAUDE_DESKTOP_DIR"),
            local_app_data: var("LOCALAPPDATA"),
            app_data: var("APPDATA"),
            xdg_config_home: var("XDG_CONFIG_HOME"),
        },
    }
}

/// A file only you can read, written whole: a temporary file, then a
/// rename, so a reader never sees half of it.
fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, file)
}

/// A run's log, up to `LOG_MAX`, then a line that says the rest isn't
/// kept.
struct Log {
    file: std::fs::File,
    written: usize,
}

impl Log {
    fn open(path: &Path) -> Option<Log> {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let mut o = std::fs::OpenOptions::new();
        o.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        Some(Log { file: o.open(path).ok()?, written: 0 })
    }

    fn write(&mut self, chunk: &[u8]) {
        if self.written >= runs::LOG_MAX {
            return;
        }
        let part = &chunk[..chunk.len().min(runs::LOG_MAX - self.written)];
        self.written += part.len();
        let _ = self.file.write_all(part);
        if self.written >= runs::LOG_MAX {
            let _ = self.file.write_all(b"\n[XenoNet keeps the first 1 MB of a run's output]\n");
        }
    }
}

/// Reads one of a run's outputs into its log to the end, the part past the
/// log's limit read and let go, so the run never waits on a full pipe.
fn drain(mut from: impl Read + Send + 'static, log: Arc<Mutex<Log>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 {
                break;
            }
            log.locked().write(&buf[..n]);
        }
    });
}

#[cfg(windows)]
fn verbatim(cmd: &mut Command, args: &[String]) {
    use std::os::windows::process::CommandExt;
    for a in args {
        cmd.raw_arg(a);
    }
}

// Only Windows has a command line to pass as written.
#[cfg(not(windows))]
fn verbatim(cmd: &mut Command, args: &[String]) {
    cmd.args(args);
}

/// One turn: of `app`'s CLI for `key` (the session it's for), with `args`,
/// in `cwd`, with `env` alone and `text` on its standard input.
pub struct Turn<'a> {
    pub key: &'a str,
    pub app: &'a str,
    pub args: &'a [String],
    pub cwd: &'a str,
    pub env: &'a [(String, String)],
    pub text: String,
}

pub struct Runner {
    /// XenoNet's state folder: the logs and the records' file are there.
    /// None keeps both in memory.
    dir: Option<PathBuf>,
    runs: Mutex<Vec<Run>>,
    /// The sessions whose runs this runner watches.
    watching: Mutex<HashSet<String>>,
}

fn alive(pid: f64) -> bool {
    pid >= 0.0 && pid <= f64::from(u32::MAX) && xenonet_base::process::running(pid as u32, true)
}

impl Runner {
    pub fn new(dir: Option<PathBuf>) -> Arc<Runner> {
        let runs = dir.as_ref().and_then(|d| std::fs::read_to_string(d.join("delivery-runs.json")).ok()).map(|t| runs::read(&t)).unwrap_or_default();
        Arc::new(Runner { dir, runs: Mutex::new(runs), watching: Mutex::new(HashSet::new()) })
    }

    /// Whether a run for `key` is under way: one this runner watches, or
    /// one a record says still runs.
    pub fn running(&self, key: &str) -> bool {
        self.watching.locked().contains(key) || runs::admit(&self.runs.locked(), key, now_ms(), usize::MAX, &mut alive).is_some()
    }

    pub fn list(&self) -> Vec<Run> {
        self.runs.locked().clone()
    }

    fn save(&self) {
        let text = {
            let mut r = self.runs.locked();
            *r = runs::kept(&r, now_ms());
            runs::write(&r)
        };
        if let Some(d) = &self.dir {
            let _ = write_private(&d.join("delivery-runs.json"), &text);
        }
    }

    /// Logs a week old or more go when a run starts.
    fn sweep(logs: &Path) {
        let Ok(names) = std::fs::read_dir(logs) else { return };
        let files: Vec<(String, f64)> = names
            .filter_map(Result::ok)
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)))
            .collect();
        for name in runs::stale_logs(&files, now_ms()) {
            let _ = std::fs::remove_file(logs.join(name));
        }
    }

    /// Whether a run for `key` may start now, or why not.
    fn admit(&self, key: &str) -> Result<(), String> {
        if self.watching.locked().contains(key) {
            return Err("a run is under way for it".into());
        }
        runs::admit(&self.runs.locked(), key, now_ms(), runs::CAP, &mut alive).map_or(Ok(()), Err)
    }

    /// One turn, with the program found for its app (`dirs::find_cli`) from
    /// `home`: its process's id, or why it didn't start.
    pub fn run(self: &Arc<Self>, t: Turn<'_>, home: &str) -> Result<u32, String> {
        self.admit(t.key)?;
        let program = dirs::find_cli(t.app, &cli_env(), platform(), home, &mut Disk, &mut xenonet_base::paths::can_run).ok_or_else(|| format!("{} isn't installed here", t.app))?;
        self.start(t, &program)
    }

    /// One turn of `program`, once it may start.
    fn start(self: &Arc<Self>, t: Turn<'_>, program: &str) -> Result<u32, String> {
        let Turn { key, app, args, cwd, env, text } = t;
        let how = runs::launch(program, args, cfg!(windows))?;
        let id = xenonet_engine::network::new_id("run").map_err(|t| t.message)?;
        let logs = self.dir.as_ref().map(|d| d.join("runs"));
        if let Some(l) = &logs {
            Self::sweep(l);
        }
        let log = logs.as_ref().and_then(|l| Log::open(&l.join(format!("{id}.log")))).map(|l| Arc::new(Mutex::new(l)));
        let out = || if log.is_some() { Stdio::piped() } else { Stdio::null() };
        let mut cmd = Command::new(&how.program);
        xenonet_base::process::Background::no_window(&mut cmd);
        if how.verbatim {
            verbatim(&mut cmd, &how.args);
        } else {
            cmd.args(&how.args);
        }
        cmd.current_dir(cwd).env_clear().envs(env.iter().map(|(k, v)| (k, v))).stdin(Stdio::piped()).stdout(out()).stderr(out());
        let at = now_ms();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!("{app} isn't installed here")),
            Err(e) => return Err(e.to_string()),
        };
        let pid = child.id();
        self.watching.locked().insert(key.to_string());
        self.runs.locked().push(Run { id, key: key.to_string(), app: app.to_string(), pid: Some(f64::from(pid)), at, end: None, exit: None, ms: None });
        self.save();
        if let Some(log) = &log {
            if let Some(o) = child.stdout.take() {
                drain(o, log.clone());
            }
            if let Some(e) = child.stderr.take() {
                drain(e, log.clone());
            }
        }
        if let Some(mut stdin) = child.stdin.take() {
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        // One turn, half an hour at most; then it's let go.
        let me = self.clone();
        let key = key.to_string();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs_f64(runs::TURN_MS / 1000.0);
            let code = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Err(_) => break None,
                    Ok(None) if Instant::now() >= deadline => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(250)),
                }
            };
            let end = now_ms();
            if let Some(r) = me.runs.locked().iter_mut().find(|r| r.pid == Some(f64::from(pid)) && r.key == key && r.end.is_none()) {
                r.end = Some(end);
                r.exit = code.map(f64::from);
                r.ms = Some(end - r.at);
            }
            me.watching.locked().remove(&key);
            me.save();
        });
        Ok(pid)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_logged_and_its_exit_kept_and_a_restart_reads_it() {
        let dir = std::env::temp_dir().join(format!("xenonet-runner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A script stands in for Claude Code, in a folder on a PATH of its
        // own: it reads its prompt, writes to both outputs and fails.
        let bin = dir.join("claude");
        std::fs::write(&bin, "#!/bin/sh\nread line\necho \"out: $line\"\necho err >&2\nsleep 0.3\nexit 3\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cwd = dir.to_string_lossy().into_owned();
        let found = dirs::find_cli("claude", &CliEnv { path: cwd.clone(), ..CliEnv::default() }, platform(), "/nowhere", &mut Disk, &mut xenonet_base::paths::can_run).unwrap();
        assert_eq!(found, bin.to_string_lossy());
        let r = Runner::new(Some(dir.clone()));
        let env = [("PATH".to_string(), "/usr/bin:/bin".to_string())];
        let turn = |key| Turn { key, app: "claude", args: &[], cwd: &cwd, env: &env, text: "hello\n".into() };
        r.admit("claude:a").unwrap();
        let pid = r.start(turn("claude:a"), &found).unwrap();
        assert!(r.running("claude:a"));
        assert_eq!(r.admit("claude:a"), Err("a run is under way for it".into()), "one per session");
        assert!(Runner::new(Some(dir.clone())).running("claude:a"), "a restart counts it while it runs");
        let end = Instant::now() + Duration::from_secs(10);
        while r.running("claude:a") {
            assert!(Instant::now() < end, "the run ended");
            std::thread::sleep(Duration::from_millis(50));
        }
        let rec = r.list().into_iter().find(|x| x.pid == Some(f64::from(pid))).unwrap();
        assert_eq!(rec.exit, Some(3.0));
        assert!(rec.ms.is_some_and(|ms| ms >= 0.0));
        let log = std::fs::read_to_string(dir.join("runs").join(format!("{}.log", rec.id))).unwrap();
        assert!(log.contains("out: hello") && log.contains("err"), "{log}");
        let again = Runner::new(Some(dir.clone()));
        assert_eq!(again.list(), r.list(), "the records are read back");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
