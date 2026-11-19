use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::duplex::{Duplex, DuplexControl};
use crate::{Cancel, Chunk, Command, ExecError, Output, Runner};

/// How long a stopped group gets to exit after SIGTERM before SIGKILL.
const GRACE: Duration = Duration::from_secs(2);

/// Runs commands for real.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    grace: Duration,
}

impl Default for SystemRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemRunner {
    pub fn new() -> SystemRunner {
        SystemRunner { grace: GRACE }
    }

    /// How long a stopped command gets between SIGTERM and SIGKILL.
    #[must_use]
    pub fn with_grace(mut self, grace: Duration) -> SystemRunner {
        self.grace = grace;
        self
    }
}

/// Finds the program: a name with a path separator is used as given, a bare
/// name is searched for in `PATH` (the command's, else this process's).
fn resolve(cmd: &Command) -> Result<PathBuf, ExecError> {
    let program = cmd.program();
    if program.is_empty() {
        return Err(ExecError::EmptyProgram);
    }
    let as_path = Path::new(program);
    if as_path.components().count() > 1 || program.to_string_lossy().contains(std::path::MAIN_SEPARATOR) {
        return if as_path.is_file() { Ok(as_path.to_path_buf()) } else { Err(ExecError::NotFound { program: program.to_owned() }) };
    }
    let path = cmd.env_vars().get(std::ffi::OsStr::new("PATH")).cloned().or_else(|| std::env::var_os("PATH")).unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(program);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
        if cfg!(windows) {
            let exe = dir.join(format!("{}.exe", program.to_string_lossy()));
            if is_executable(&exe) {
                return Ok(exe);
            }
        }
    }
    Err(ExecError::NotFound { program: program.to_owned() })
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

struct Spawned {
    child: Child,
    /// The group to stop, on unix: the child's pid, since it leads its own.
    pgid: Option<u32>,
}

fn spawn(cmd: &Command, stdin: Stdio, stderr_piped: bool) -> Result<Spawned, ExecError> {
    let program = resolve(cmd)?;
    let mut c = tokio::process::Command::new(&program);
    c.args(cmd.arg_list()).env_clear().envs(cmd.env_vars()).stdin(stdin).stdout(Stdio::piped()).kill_on_drop(true);
    c.stderr(if stderr_piped { Stdio::piped() } else { Stdio::null() });
    if let Some(dir) = cmd.working_dir() {
        c.current_dir(dir);
    }
    #[cfg(unix)]
    c.process_group(0);
    #[cfg(windows)]
    c.as_std_mut().creation_flags(0x0800_0000);
    let child = c.spawn().map_err(|source| ExecError::Spawn { program: cmd.program().to_owned(), source })?;
    let pgid = if cfg!(unix) { child.id() } else { None };
    Ok(Spawned { child, pgid })
}

#[cfg(unix)]
fn signal_group(pgid: u32, kill: bool) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    let Ok(raw) = i32::try_from(pgid) else { return };
    // A group with nobody left in it answers ESRCH; that is the goal.
    let _ = killpg(Pid::from_raw(raw), if kill { Signal::SIGKILL } else { Signal::SIGTERM });
}

#[cfg(not(unix))]
fn signal_group(_pgid: u32, _kill: bool) {}

/// Stops the child and everything in its group: SIGTERM, a grace, SIGKILL.
/// On Windows only the child itself is killed.
async fn stop(child: &mut Child, pgid: Option<u32>, grace: Duration) {
    match pgid {
        Some(pgid) => {
            signal_group(pgid, false);
            let _ = tokio::time::timeout(grace, child.wait()).await;
            // Whatever ignored SIGTERM, or outlived the leader, goes now.
            signal_group(pgid, true);
        }
        None => {
            let _ = child.start_kill();
        }
    }
    let _ = child.wait().await;
}

fn spawn_reader<R>(mut stream: R, sink: Option<mpsc::Sender<Chunk>>, wrap: fn(Vec<u8>) -> Chunk) -> JoinHandle<Vec<u8>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut all = Vec::new();
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => match &sink {
                    // A sink nobody listens to shouldn't stop the command.
                    Some(tx) => {
                        let _ = tx.send(wrap(buf[..n].to_vec())).await;
                    }
                    None => all.extend_from_slice(&buf[..n]),
                },
            }
        }
        all
    })
}

async fn write_stdin(mut stdin: tokio::process::ChildStdin, bytes: Vec<u8>) {
    // The child may exit without reading everything; that is its business.
    let _ = stdin.write_all(&bytes).await;
    let _ = stdin.shutdown().await;
}

async fn drive(grace: Duration, cmd: &Command, cancel: &Cancel, sink: Option<mpsc::Sender<Chunk>>) -> Result<Output, ExecError> {
    let stdin = if cmd.stdin().is_some() { Stdio::piped() } else { Stdio::null() };
    let Spawned { mut child, pgid } = spawn(cmd, stdin, true)?;
    if let (Some(bytes), Some(pipe)) = (cmd.stdin(), child.stdin.take()) {
        tokio::spawn(write_stdin(pipe, bytes.to_vec()));
    }
    let out = child.stdout.take().map(|s| spawn_reader(s, sink.clone(), Chunk::Stdout));
    let err = child.stderr.take().map(|s| spawn_reader(s, sink, Chunk::Stderr));

    let deadline = async {
        match cmd.deadline_limit() {
            Some(d) => tokio::time::sleep(d).await,
            None => std::future::pending().await,
        }
    };
    let mut timed_out = false;
    let mut cancelled = false;
    let status = tokio::select! {
        waited = child.wait() => Some(waited.map_err(|source| ExecError::Io { what: "wait for the command", source })?),
        () = deadline => { timed_out = true; None }
        () = cancel.cancelled() => { cancelled = true; None }
    };
    let code = match status {
        Some(s) => s.code(),
        None => {
            stop(&mut child, pgid, grace).await;
            None
        }
    };
    let stdout = collect(out, !(timed_out || cancelled)).await;
    let stderr = collect(err, !(timed_out || cancelled)).await;
    Ok(Output { status: code, stdout, stderr, timed_out, cancelled })
}

/// Waits for a reader to see end of file. After a stop the pipes should be
/// closed already; a straggler that escaped the group doesn't get to hold
/// the call up for long.
async fn collect(reader: Option<JoinHandle<Vec<u8>>>, patient: bool) -> Vec<u8> {
    let Some(handle) = reader else { return Vec::new() };
    if patient {
        handle.await.unwrap_or_default()
    } else {
        tokio::time::timeout(Duration::from_secs(1), handle).await.ok().and_then(Result::ok).unwrap_or_default()
    }
}

#[async_trait]
impl Runner for SystemRunner {
    async fn run(&self, cmd: &Command, cancel: &Cancel) -> Result<Output, ExecError> {
        drive(self.grace, cmd, cancel, None).await
    }

    async fn stream(&self, cmd: &Command, cancel: &Cancel, sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError> {
        drive(self.grace, cmd, cancel, Some(sink)).await
    }

    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError> {
        let Spawned { mut child, pgid } = spawn(cmd, Stdio::piped(), true)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(ExecError::Io { what: "open the command's pipes", source: std::io::Error::other("the pipes were not created") });
        };
        let stderr = child.stderr.take().map(|s| spawn_reader(s, None, Chunk::Stderr));
        let control = SystemControl { child, pgid, stderr, deadline: cmd.deadline_limit(), grace: self.grace };
        Ok(Duplex::from_parts(Box::new(stdout), Box::new(stdin), Box::new(control)))
    }
}

struct SystemControl {
    child: Child,
    pgid: Option<u32>,
    stderr: Option<JoinHandle<Vec<u8>>>,
    deadline: Option<Duration>,
    grace: Duration,
}

#[async_trait]
impl DuplexControl for SystemControl {
    async fn finish(mut self: Box<Self>) -> Result<Output, ExecError> {
        let mut timed_out = false;
        let code = match self.deadline {
            Some(limit) => match tokio::time::timeout(limit, self.child.wait()).await {
                Ok(waited) => waited.map_err(|source| ExecError::Io { what: "wait for the command", source })?.code(),
                Err(_) => {
                    timed_out = true;
                    stop(&mut self.child, self.pgid, self.grace).await;
                    None
                }
            },
            None => self.child.wait().await.map_err(|source| ExecError::Io { what: "wait for the command", source })?.code(),
        };
        let stderr = collect(self.stderr.take(), !timed_out).await;
        Ok(Output { status: code, stdout: Vec::new(), stderr, timed_out, cancelled: false })
    }

    fn kill(&mut self) {
        match self.pgid {
            Some(pgid) => signal_group(pgid, true),
            None => {
                let _ = self.child.start_kill();
            }
        }
    }
}
