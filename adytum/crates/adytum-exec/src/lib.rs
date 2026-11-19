//! Commands run as argument lists, never shell strings: streamed output, a
//! deadline, a cancel, the whole process group stopped, and only the
//! variables the caller names.
//!
//! This is the only place Adytum starts a child process. Drivers take a
//! `&dyn Runner`, so tests can swap in a fake (`adytum-testkit`).
//!
//! The child's environment starts empty. Only what [`Command::env`] and
//! [`Command::pass_env`] name reaches it. A bare program name is looked up in
//! the `PATH` the command names (or this process's `PATH` if it names none),
//! never through a shell.
//!
//! On unix each child leads its own process group, and a deadline or a
//! cancel stops the whole group: SIGTERM, a short grace, then SIGKILL. On
//! Windows only the child itself is stopped; its own children are not.

mod command;
mod duplex;
mod error;
mod system;

pub use command::Command;
pub use duplex::{Duplex, DuplexControl};
pub use error::ExecError;
pub use system::SystemRunner;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};

/// What a finished command left behind.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Output {
    /// The exit code, or `None` if a signal ended the child (or it was
    /// stopped on a deadline or cancel).
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
    pub cancelled: bool,
}

impl Output {
    /// True when the child exited with code 0 and nothing stopped it.
    pub fn success(&self) -> bool {
        self.status == Some(0) && !self.timed_out && !self.cancelled
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// A piece of output, delivered as it arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

/// A cheap handle that stops a running command. Clones share one signal.
#[derive(Debug, Clone)]
pub struct Cancel {
    tx: std::sync::Arc<watch::Sender<bool>>,
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancel {
    pub fn new() -> Cancel {
        Cancel { tx: std::sync::Arc::new(watch::channel(false).0) }
    }

    /// Asks every command watching this handle to stop. Safe to call twice.
    pub fn cancel(&self) {
        self.tx.send_replace(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves once [`Cancel::cancel`] has been called (at once if it
    /// already was).
    pub async fn cancelled(&self) {
        let mut rx = self.tx.subscribe();
        // The sender lives in `self`, so the channel can't close under us.
        let _ = rx.wait_for(|stopped| *stopped).await;
    }
}

/// Runs commands. `SystemRunner` does it for real; the testkit's fake
/// replays recorded transcripts.
#[async_trait]
pub trait Runner: Send + Sync {
    /// Runs to the end and returns everything the child wrote.
    async fn run(&self, cmd: &Command, cancel: &Cancel) -> Result<Output, ExecError>;

    /// Like `run`, but hands output to `sink` as it arrives. The returned
    /// buffers may be empty in this mode.
    async fn stream(&self, cmd: &Command, cancel: &Cancel, sink: mpsc::Sender<Chunk>) -> Result<Output, ExecError>;

    /// The child's stdin and stdout as one byte stream. This is how bytes
    /// are piped into a VM over `limactl shell` without opening a port.
    async fn duplex(&self, cmd: &Command) -> Result<Duplex, ExecError>;
}
