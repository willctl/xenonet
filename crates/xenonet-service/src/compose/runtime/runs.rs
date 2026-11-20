//! The runner: headless turns of Claude Code and
//! Codex that this machine starts, for a session it resumes. What runs,
//! quoted how, and whether it may start now are xenonet-engine's
//! (`delivery::runs`); the processes, their logs in `runs/` and their
//! records, kept in the store, are here. Each run's processes are one
//! process group (Adytum's exec): a run counts until they've all ended,
//! and its cap stops them all.

use crate::compose::runtime::platform;
use adytum::exec::{Command, Spawned};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_adapter::dirs::{self, CliEnv, Look, Unreadable};
use xenonet_engine::delivery::runs::{self, Run};
use xenonet_os::sync::Locked;
use xenonet_store::delivery::RunRow;
use xenonet_store::Db;
use xenonet_text::time::now_ms;

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
        std::fs::metadata(p).ok().and_then(|m| xenonet_os::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)
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

/// How an app is started as an ACP agent here, as its adapter finds it on
/// this process's PATH or where its installer puts it under `home`.
pub(crate) fn acp_command(app: &dyn xenonet_adapter::Platform, home: &str) -> Option<xenonet_adapter::AcpCommand> {
    app.acp(&cli_env(), platform(), home, &mut Disk, &mut xenonet_os::paths::can_run)
}

/// Opens a file only you can read: made with mode 0600 on unix, so it's
/// never readable by others, not even for a moment.
fn private(o: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
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
        Some(Log { file: private(std::fs::OpenOptions::new().create(true).append(true)).open(path).ok()?, written: 0 })
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
/// log's limit read and let go, so the run never waits on a full pipe. With
/// no log it is read and dropped.
fn drain(mut from: impl Read + Send + 'static, log: Option<Arc<Mutex<Log>>>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        while let Ok(n) = from.read(&mut buf) {
            if n == 0 {
                break;
            }
            if let Some(log) = &log {
                log.locked().write(&buf[..n]);
            }
        }
    });
}

/// The command for `how`: its arguments as given, or on Windows (where a
/// command line is one string) as written.
#[cfg(windows)]
fn command(how: &runs::Launch) -> Command {
    let cmd = Command::new(&how.program);
    // Adytum's Spawned starts it with no window (CREATE_NO_WINDOW, 0x0800_0000).
    if how.verbatim {
        how.args.iter().fold(cmd, Command::raw_arg)
    } else {
        cmd.args(&how.args)
    }
}

// Only Windows has a command line to pass as written.
#[cfg(not(windows))]
fn command(how: &runs::Launch) -> Command {
    Command::new(&how.program).args(&how.args)
    // Never built on Windows, where Spawned adds CREATE_NO_WINDOW (no_window).
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
    /// The run's sign-in, for an app that takes it on a descriptor (unix:
    /// Claude Code's CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR, which `env`
    /// then names as 3). It is written into a pipe the run gets the read end
    /// of, so it is in no environment a command could read.
    pub sign_in: Option<&'a str>,
}

/// The descriptor number a run reads its sign-in from.
pub const SIGN_IN_FD: i32 = 3;

/// A run in its process group: adytum-exec's, or on unix one that was given
/// a descriptor, which adytum-exec can't do.
enum Proc {
    Group(Spawned),
    #[cfg(unix)]
    Held(xenonet_os::process::Held),
}

impl Proc {
    fn pid(&self) -> u32 {
        match self {
            Proc::Group(s) => s.pid(),
            #[cfg(unix)]
            Proc::Held(h) => h.pid(),
        }
    }

    fn stdin(&mut self) -> Option<std::process::ChildStdin> {
        match self {
            Proc::Group(s) => s.stdin.take(),
            #[cfg(unix)]
            Proc::Held(h) => h.stdin.take(),
        }
    }

    fn stdout(&mut self) -> Option<std::process::ChildStdout> {
        match self {
            Proc::Group(s) => s.stdout.take(),
            #[cfg(unix)]
            Proc::Held(h) => h.stdout.take(),
        }
    }

    fn stderr(&mut self) -> Option<std::process::ChildStderr> {
        match self {
            Proc::Group(s) => s.stderr.take(),
            #[cfg(unix)]
            Proc::Held(h) => h.stderr.take(),
        }
    }

    #[cfg(not(windows))]
    fn group_running(&self) -> bool {
        match self {
            Proc::Group(s) => s.group_running(),
            #[cfg(unix)]
            Proc::Held(h) => h.group_running(),
        }
    }

    /// How it ended (its code, whether the deadline stopped it), once it has.
    fn try_wait(&mut self) -> Result<Option<(Option<i32>, bool)>, ()> {
        match self {
            Proc::Group(s) => s.try_wait().map(|e| e.map(|e| (e.code, e.timed_out))).map_err(|_| ()),
            #[cfg(unix)]
            Proc::Held(h) => h.try_wait().map(|e| e.map(|e| (e.code, e.timed_out))).map_err(|_| ()),
        }
    }

    #[cfg(not(windows))]
    fn kill_group(&mut self) {
        match self {
            Proc::Group(s) => s.kill_group(),
            #[cfg(unix)]
            Proc::Held(h) => h.kill_group(),
        }
    }

    /// Windows: the child, for a job object (adytum-exec's only).
    #[cfg(windows)]
    fn child_mut(&mut self) -> &mut std::process::Child {
        match self {
            Proc::Group(s) => s.child_mut(),
        }
    }
}

/// Starts `how`: with a descriptor holding `token` on unix when there is
/// one, else as adytum-exec does.
fn spawn(how: &runs::Launch, cwd: &str, env: &[(String, String)], token: Option<&str>) -> Result<Proc, ExecError> {
    let deadline = Duration::from_secs_f64(runs::TURN_MS / 1000.0);
    #[cfg(unix)]
    if let Some(token) = token {
        return spawn_held(how, cwd, env, token, deadline);
    }
    #[cfg(not(unix))]
    let _ = token;
    let cmd = command(how).cwd(cwd).deadline(deadline);
    let cmd = env.iter().fold(cmd, |c, (k, v)| c.env(k, v));
    Spawned::start(&cmd).map(Proc::Group)
}

/// A run that reads its sign-in from descriptor 3: a pipe whose read end
/// the run has as descriptor 3, with the token and a newline written into
/// it once the run has started, and our end closed (`Held::start`, which
/// says what is left of a window). The run has descriptors 0 to 3 and no
/// other. Made with std's command, since adytum-exec has no way to pass a
/// descriptor and is Adytum's own repository.
#[cfg(unix)]
fn spawn_held(how: &runs::Launch, cwd: &str, env: &[(String, String)], token: &str, deadline: Duration) -> Result<Proc, ExecError> {
    let mut c = std::process::Command::new(&how.program);
    c.args(&how.args).env_clear().envs(env.iter().map(|(k, v)| (k, v))).current_dir(cwd);
    xenonet_os::process::Held::start(c, Some(deadline), Some(xenonet_os::process::Give { line: token, at: SIGN_IN_FD }))
        .map(Proc::Held)
        .map_err(|source| ExecError::Spawn { program: how.program.clone().into(), source })
}

/// The records, and the sessions whose runs this runner has under way:
/// one lock, so admitting a run and counting it are one step.
#[derive(Default)]
struct State {
    runs: Vec<Run>,
    /// Each run this runner admitted, from then until it and everything it
    /// started have ended.
    watching: Vec<String>,
    /// The runs asked to stop (an intent cancelled or expired), by key,
    /// until they have.
    stopping: Vec<String>,
}

pub struct Runner {
    /// XenoNet's state folder, where the logs go; None keeps them in
    /// memory.
    dir: Option<PathBuf>,
    /// The store the records are kept in; None keeps them in memory.
    db: Option<Arc<Db>>,
    state: Mutex<State>,
    /// Writes of the records, one at a time.
    saving: Mutex<()>,
}

fn run_of(r: RunRow) -> Run {
    Run { id: r.id, key: r.key, app: r.app, pid: r.pid, at: r.at, end: r.end, exit: r.exit, ms: r.ms, timed_out: r.timed_out }
}

fn row_of(r: &Run) -> RunRow {
    RunRow { id: r.id.clone(), key: r.key.clone(), app: r.app.clone(), pid: r.pid, at: r.at, end: r.end, exit: r.exit, ms: r.ms, timed_out: r.timed_out }
}

fn alive(pid: f64) -> bool {
    pid >= 0.0 && pid <= f64::from(u32::MAX) && xenonet_os::process::running(pid as u32, true)
}

impl Runner {
    pub fn new(dir: Option<PathBuf>, db: Option<Arc<Db>>) -> Arc<Runner> {
        let runs = db.as_ref().and_then(|d| d.runs().ok()).map(|rows| rows.into_iter().map(run_of).collect()).unwrap_or_default();
        Arc::new(Runner { dir, db, state: Mutex::new(State { runs, watching: Vec::new(), stopping: Vec::new() }), saving: Mutex::new(()) })
    }

    /// Whether a run for `key` is under way: one this runner watches, or
    /// one a record says still runs.
    pub fn running(&self, key: &str) -> bool {
        let s = self.state.locked();
        runs::admit_with(&s.runs, &s.watching, key, now_ms(), usize::MAX, &mut alive).is_some()
    }

    pub fn list(&self) -> Vec<Run> {
        self.state.locked().runs.clone()
    }

    fn save(&self) {
        let _one = self.saving.locked();
        let rows: Vec<RunRow> = {
            let mut s = self.state.locked();
            s.runs = runs::kept(&s.runs, now_ms());
            s.runs.iter().map(row_of).collect()
        };
        if let Some(db) = &self.db {
            let _ = db.put_runs(&rows);
        }
    }

    /// Lets go of a run admitted for `key`.
    fn release(&self, key: &str) {
        let mut s = self.state.locked();
        s.watching.retain(|k| k != key);
        s.stopping.retain(|k| k != key);
    }

    /// Stops the run under way for `key`, and everything it started, when
    /// this runner watches one: whether it does. One started before a
    /// restart runs on until it ends.
    pub fn stop(&self, key: &str) -> bool {
        let mut s = self.state.locked();
        if !s.watching.iter().any(|k| k == key) {
            return false;
        }
        if !s.stopping.iter().any(|k| k == key) {
            s.stopping.push(key.to_string());
        }
        true
    }

    /// Logs a week old or more go when a run starts.
    fn sweep(logs: &Path) {
        let Ok(names) = std::fs::read_dir(logs) else { return };
        let files: Vec<(String, f64)> = names
            .filter_map(Result::ok)
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.metadata().ok().and_then(|m| xenonet_os::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)))
            .collect();
        for name in runs::stale_logs(&files, now_ms()) {
            let _ = std::fs::remove_file(logs.join(name));
        }
    }

    /// Whether a run for `key` may start now, or why not. One that may is
    /// counted at once, under the same lock, so two messages to one session
    /// can't both start a run; `release` lets it go if it doesn't start.
    fn admit(&self, key: &str) -> Result<(), String> {
        let mut s = self.state.locked();
        if let Some(why) = runs::admit_with(&s.runs, &s.watching, key, now_ms(), runs::CAP, &mut alive) {
            return Err(why);
        }
        s.watching.push(key.to_string());
        Ok(())
    }

    /// One turn, with the program found for its app (`dirs::find_cli`) from
    /// `home`: its process's id, or why it didn't start.
    pub fn run(self: &Arc<Self>, t: Turn<'_>, home: &str) -> Result<u32, String> {
        self.admit(t.key)?;
        let key = t.key.to_string();
        // An app find_cli doesn't know (Grok Build) is on the PATH, or
        // where its adapter finds it.
        let env = cli_env();
        let found = dirs::find_cli(t.app, &env, platform(), home, &mut Disk, &mut xenonet_os::paths::can_run)
            .or_else(|| xenonet_os::paths::on_path(t.app, &env.path, env.pathext.as_deref(), cfg!(windows), &mut xenonet_os::paths::can_run))
            .or_else(|| xenonet_adapter::platforms().get(t.app)?.program(&env.dirs, platform(), home, &mut Disk));
        let started = match found {
            Some(program) => self.start(t, &program),
            None => Err(format!("{} isn't installed here", t.app)),
        };
        if started.is_err() {
            self.release(&key);
        }
        started
    }

    /// One turn of `program`, given rather than found: XENONET_RUNS_BIN's,
    /// which stands in for every app's CLI in the tests.
    pub fn run_program(self: &Arc<Self>, t: Turn<'_>, program: &str) -> Result<u32, String> {
        self.admit(t.key)?;
        let key = t.key.to_string();
        let started = self.start(t, program);
        if started.is_err() {
            self.release(&key);
        }
        started
    }

    /// One turn of `program`, once `admit` let it start.
    fn start(self: &Arc<Self>, t: Turn<'_>, program: &str) -> Result<u32, String> {
        let Turn { key, app, args, cwd, env, text, sign_in } = t;
        let how = runs::launch(program, args, cfg!(windows))?;
        let id = xenonet_engine::network::new_id("run").map_err(|t| t.message)?;
        let logs = self.dir.as_ref().map(|d| d.join("runs"));
        if let Some(l) = &logs {
            Self::sweep(l);
        }
        let log_path = logs.as_ref().map(|l| l.join(format!("{id}.log")));
        let log = log_path.as_ref().and_then(|p| Log::open(p)).map(|l| Arc::new(Mutex::new(l)));
        let at = now_ms();
        // Its own group, so stopping the turn stops what it started.
        let mut child = match spawn(&how, cwd, env, sign_in) {
            Ok(c) => c,
            Err(e) => {
                // No run, so no log of one.
                if let Some(p) = &log_path {
                    drop(log);
                    let _ = std::fs::remove_file(p);
                }
                let missing =
                    matches!(&e, adytum::Error::ProgramNotFound { .. }) || matches!(&e, adytum::Error::Spawn { source, .. } if source.kind() == std::io::ErrorKind::NotFound);
                return Err(if missing { format!("{app} isn't installed here") } else { e.to_string() });
            }
        };
        let pid = child.pid();
        // A job object: Adytum's exec has no process group on Windows.
        #[cfg(windows)]
        let tree = xenonet_os::process::Tree::of(child.child_mut());
        let run = Run { id, key: key.to_string(), app: app.to_string(), pid: Some(f64::from(pid)), at, end: None, exit: None, ms: None, timed_out: false };
        self.state.locked().runs.push(run);
        self.save();
        if let Some(o) = child.stdout() {
            drain(o, log.clone());
        }
        if let Some(e) = child.stderr() {
            drain(e, log.clone());
        }
        if let Some(mut stdin) = child.stdin() {
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        // One turn, half an hour at most; then it and everything it started
        // are stopped. It's under way until they've all ended.
        let me = self.clone();
        let key = key.to_string();
        std::thread::spawn(move || {
            let mut code = None;
            let mut timed_out = false;
            #[cfg(windows)]
            let started = std::time::Instant::now();
            loop {
                // The turn has ended, and so has everything it started.
                #[cfg(windows)]
                let others = {
                    // Adytum's exec deadline stops only the child here, so
                    // the job (the child's whole tree) is stopped by this.
                    if started.elapsed().as_secs_f64() * 1000.0 >= runs::TURN_MS && tree.others() {
                        tree.kill(child.child_mut());
                        timed_out = true;
                    }
                    tree.others()
                };
                #[cfg(not(windows))]
                let others = child.group_running();
                match child.try_wait() {
                    Ok(Some((exit, late))) if !others => {
                        code = exit;
                        timed_out |= late;
                        break;
                    }
                    Err(()) => break,
                    _ => {}
                }
                if me.state.locked().stopping.contains(&key) {
                    #[cfg(windows)]
                    tree.kill(child.child_mut());
                    #[cfg(not(windows))]
                    child.kill_group();
                    break;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            let end = now_ms();
            {
                let mut s = me.state.locked();
                if let Some(r) = s.runs.iter_mut().find(|r| r.pid == Some(f64::from(pid)) && r.key == key && r.end.is_none()) {
                    r.end = Some(end);
                    r.exit = code.map(f64::from);
                    r.ms = Some(end - r.at);
                    r.timed_out = timed_out;
                }
            }
            // Saved before the session is let go, so whoever sees it free
            // (a restart, the next run) reads how this one ended.
            me.save();
            me.release(&key);
        });
        Ok(pid)
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_run_is_logged_and_its_exit_kept_and_a_restart_reads_it() {
        let dir = std::env::temp_dir().join(format!("xenonet-runner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A script stands in for Claude Code, in a folder on a PATH of its
        // own: it reads its prompt, writes to both outputs, waits until the
        // test lets it go, and fails.
        let bin = dir.join("claude");
        let go = dir.join("go");
        std::fs::write(&bin, "#!/bin/sh\nread line\necho \"out: $line\"\necho err >&2\nwhile [ ! -f \"$GO\" ]; do sleep 0.02; done\nexit 3\n").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cwd = dir.to_string_lossy().into_owned();
        let found = dirs::find_cli("claude", &CliEnv { path: cwd.clone(), ..CliEnv::default() }, platform(), "/nowhere", &mut Disk, &mut xenonet_os::paths::can_run).unwrap();
        assert_eq!(found, bin.to_string_lossy());
        let db = Arc::new(Db::memory().unwrap());
        let r = Runner::new(Some(dir.clone()), Some(db.clone()));
        let env = [("PATH".to_string(), "/usr/bin:/bin".to_string()), ("GO".to_string(), go.to_string_lossy().into_owned())];
        let turn = |key| Turn { key, app: "claude", args: &[], cwd: &cwd, env: &env, text: "hello\n".into(), sign_in: None };
        // Admitted is counted at once: a second message to the session, or
        // a fifth run, can't start one beside it before it has started.
        r.admit("claude:a").unwrap();
        assert_eq!(r.admit("claude:a"), Err("a run is under way for it".into()), "one per session");
        for k in ["claude:b", "claude:c", "claude:d"] {
            r.admit(k).unwrap();
        }
        assert_eq!(r.admit("claude:e"), Err("XenoNet runs at most 4 turns at once on this machine".into()));
        for k in ["claude:b", "claude:c", "claude:d"] {
            r.release(k);
        }
        let pid = r.start(turn("claude:a"), &found).unwrap();
        assert!(r.running("claude:a"));
        assert_eq!(r.admit("claude:a"), Err("a run is under way for it".into()));
        assert!(Runner::new(Some(dir.clone()), Some(db.clone())).running("claude:a"), "a restart counts it while it runs");
        // A run that doesn't start leaves no log, and its session free.
        assert_eq!(r.run(Turn { app: "nothing-here", ..turn("claude:x") }, "/nowhere"), Err("nothing-here isn't installed here".into()));
        assert!(!r.running("claude:x"));
        r.admit("claude:y").unwrap();
        assert_eq!(r.start(turn("claude:y"), &dir.join("gone").to_string_lossy()), Err("claude isn't installed here".into()));
        r.release("claude:y");
        assert_eq!(std::fs::read_dir(dir.join("runs")).unwrap().count(), 1, "only the run that started has a log");
        std::fs::write(&go, "").unwrap();
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
        let again = Runner::new(Some(dir.clone()), Some(db));
        assert_eq!(again.list(), r.list(), "the records are read back");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_runs_sign_in_comes_on_descriptor_3_and_in_no_environment() {
        let dir = std::env::temp_dir().join(format!("xenonet-runner-fd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Stands in for Claude Code: opens /dev/fd/3 (a pipe, as on Linux)
        // for a line, then for the rest, which ends at once only if no
        // process holds the write end open (XenoNet's was closed and wasn't
        // passed on), and lists any descriptor past 3.
        let bin = dir.join("claude");
        std::fs::write(
            &bin,
            "#!/bin/sh\nread line\nIFS= read -r tok < /dev/fd/3\nrest=$(cat < /dev/fd/3)\necho \"len: ${#tok} rest: ${#rest}\"\necho \"env: $(env | grep -c made-up-token)\"\nn=4; while [ $n -lt 120 ]; do [ -e /dev/fd/$n ] && echo \"open $n\"; n=$((n+1)); done\necho \"fds done\"\nexit 0\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cwd = dir.to_string_lossy().into_owned();
        let r = Runner::new(Some(dir.clone()), None);
        let env = [("PATH".to_string(), "/usr/bin:/bin".to_string()), ("CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_string(), "3".to_string())];
        r.admit("claude:fd").unwrap();
        let turn = Turn { key: "claude:fd", app: "claude", args: &[], cwd: &cwd, env: &env, text: "hello\n".into(), sign_in: Some("made-up-token-0123") };
        let pid = r.start(turn, &bin.to_string_lossy()).unwrap();
        let end = Instant::now() + Duration::from_secs(10);
        while r.running("claude:fd") {
            assert!(Instant::now() < end, "the run ended: an end of the pair is still held");
            std::thread::sleep(Duration::from_millis(50));
        }
        let rec = r.list().into_iter().find(|x| x.pid == Some(f64::from(pid))).unwrap();
        let log = std::fs::read_to_string(dir.join("runs").join(format!("{}.log", rec.id))).unwrap();
        assert!(log.contains("len: 18 rest: 0"), "the run read its token from descriptor 3 and then the end: {log}");
        assert!(log.contains("env: 0\n"), "it is in no environment: {log}");
        assert!(log.contains("fds done") && !log.contains("open "), "the run has no descriptor past 3: {log}");
        assert!(!log.contains("made-up-token"), "the token isn't printed: {log}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
