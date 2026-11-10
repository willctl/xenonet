//! What the install's work waits on, which whoever runs it does: the
//! running service's connection, starting a program, a staged runtime's
//! startup check, whether a process is there, a fetch, the clock and a
//! pause. The command does it with its own sockets and processes; the
//! tests, with made-up ones.

use crate::runtime::Env;
use async_trait::async_trait;
use serde_json::Value;
use xenonet_os::error::Failure;

/// A connection to the running service, after its hello: its handle, and
/// what the service said of itself.
pub struct Connected {
    pub id: u64,
    pub info: Value,
}

/// A program to start detached: it goes on when whoever started it ends,
/// its output appended to `log` when there's one.
pub struct Spawn {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Its whole environment.
    pub env: Env,
    pub log: Option<String>,
}

/// What a fetch answered.
pub struct Got {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Got {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// The running service's connection.
#[async_trait(?Send)]
pub trait Service {
    /// Connects to the service the runtime folder's descriptor names, and
    /// says hello: one of an interface this XenoNet doesn't speak is
    /// refused as incompatible.
    async fn connect(&self, env: &Env) -> Result<Connected, Failure>;
    async fn request(&self, client: u64, method: &str, params: Value) -> Result<Value, Failure>;
    async fn close(&self, client: u64);
    async fn sleep(&self, ms: f64);
}

/// What starting the service waits on.
#[async_trait(?Send)]
pub trait Starter {
    /// Whether a service answers: connected to and let go again.
    async fn reach(&self, env: &Env) -> Result<(), Failure>;
    async fn spawn(&self, spawn: &Spawn) -> Result<(), Failure>;
    /// What the program it started came to, when it ended or failed.
    async fn failed(&self) -> Result<(), Failure>;
    async fn sleep(&self, ms: f64);
}

/// What installing and switching components waits on.
#[async_trait(?Send)]
pub trait Lifecycle: Service {
    /// Starts a runtime, unless a service answers: "reused" or "started".
    async fn start(&self, runtime: &Value) -> Result<Value, Failure>;
    /// A staged runtime's startup check.
    async fn check(&self, staged: &Value) -> Result<(), Failure>;
    /// Whether a process is there: an error coded ESRCH when it isn't.
    async fn alive(&self, pid: f64) -> Result<(), Failure>;
    async fn now(&self) -> f64;
}

/// What the console of an installed XenoNet waits on: the service, Desktop
/// started as it registered itself, and the API's answers.
#[async_trait(?Send)]
pub trait Desktop: Service {
    async fn spawn(&self, spawn: &Spawn) -> Result<(), Failure>;
    /// A GET of the service's API, its body read as JSON.
    async fn get_json(&self, url: &str) -> Result<Value, Failure>;
}

/// What the updates' feed waits on.
#[async_trait(?Send)]
pub trait Feed {
    async fn get(&self, url: &str, timeout_ms: f64) -> Result<Got, Failure>;
}

/// Runs a future to its end that never waits, as a fake host's work does.
#[cfg(test)]
pub(crate) fn ready<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match f.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("a fake host's work waited"),
    }
}
