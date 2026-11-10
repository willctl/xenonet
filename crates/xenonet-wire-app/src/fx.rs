//! The plumbing around the cores, natively: child processes run and
//! started, the PATH as a shell reads it, the environment, and a thread
//! for the work whose futures aren't Send (the doctor's checks, the
//! assistant's answers, the sandboxes' actions), which waits on the
//! service's runtime for what it asks.
//!
//! That work is async Rust over a host that isn't Send, so it runs on a
//! thread of its own, one current-thread runtime with a LocalSet.

use serde_json::{json, Map, Value};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A lock taken whatever a panic elsewhere left in it: what's kept behind
/// these locks is whole between statements, so it's still good to read.
pub trait Locked<T> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> Locked<T> for std::sync::Mutex<T> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// process.platform.
pub fn platform() -> &'static str {
    xenonet_os::platform()
}

pub fn now_ms() -> f64 {
    xenonet_text::time::now_ms()
}

/// An error as a JSON object: its message, and its code when it has one.
pub fn error(message: impl Into<String>, code: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("message".into(), Value::String(message.into()));
    if let Some(c) = code {
        m.insert("code".into(), json!(c));
    }
    Value::Object(m)
}

type Job = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send>;

// The thread that work runs on, started the first time it's asked for.
fn jobs() -> &'static mpsc::UnboundedSender<Job> {
    static JOBS: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name("xenonet-local".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async move {
                    while let Some(job) = rx.recv().await {
                        tokio::task::spawn_local(job());
                    }
                });
            })
            .expect("a thread for the work that isn't Send");
        tx
    })
}

/// Runs a future that isn't Send to its end, on the thread kept for them,
/// and waits for it here: None when it stopped before it was done.
pub async fn local<F, Fut, T>(make: F) -> Option<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T> + 'static,
    T: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    let job: Job = Box::new(move || {
        Box::pin(async move {
            let _ = tx.send(make().await);
        })
    });
    jobs().send(job).ok()?;
    rx.await.ok()
}

/// What a child process came to, as execFile's callback has it: `code` 0,
/// the exit code, the error's code ("ENOENT") when it couldn't start, or 1
/// when it was killed (a timeout).
#[derive(Clone, Debug)]
pub struct Ran {
    pub code: Value,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl Ran {
    pub fn ok(&self) -> bool {
        self.code == json!(0)
    }

    pub fn to_value(&self) -> Value {
        json!({ "code": self.code, "stdout": self.stdout, "stderr": self.stderr, "timedOut": self.timed_out })
    }
}

/// How a child is run: its environment (None: the service's own), what goes
/// on its standard input, and the variables taken out.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub env: Option<Vec<(String, String)>>,
    pub remove: Vec<String>,
    pub input: Option<String>,
    pub cwd: Option<String>,
}

fn io_code(e: &std::io::Error) -> String {
    xenonet_os::fs::code_of(e).map_or_else(|| e.to_string(), str::to_string)
}

/// execFile(file, args, { timeout }): its output, whatever it came to.
pub async fn exec_file(file: &str, args: &[String], timeout_ms: u64, opts: RunOptions) -> Ran {
    let mut cmd = tokio::process::Command::new(file);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd.stdin(if opts.input.is_some() { Stdio::piped() } else { Stdio::null() });
    if let Some(env) = &opts.env {
        cmd.env_clear();
        cmd.envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    }
    for k in &opts.remove {
        cmd.env_remove(k);
    }
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran { code: json!(io_code(&e)), stdout: String::new(), stderr: String::new(), timed_out: false },
    };
    if let (Some(input), Some(mut stdin)) = (opts.input, child.stdin.take()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let mut out = child.stdout.take();
    let mut err = child.stderr.take();
    let reading = async move {
        let mut o = Vec::new();
        let mut e = Vec::new();
        let read_out = async {
            if let Some(s) = out.as_mut() {
                let _ = s.read_to_end(&mut o).await;
            }
        };
        let read_err = async {
            if let Some(s) = err.as_mut() {
                let _ = s.read_to_end(&mut e).await;
            }
        };
        tokio::join!(read_out, read_err);
        (o, e)
    };
    let waited = async {
        let (o, e) = reading.await;
        let status = child.wait().await;
        (o, e, status)
    };
    match tokio::time::timeout(Duration::from_millis(timeout_ms.max(1)), waited).await {
        Ok((o, e, status)) => {
            let code = match status {
                Ok(s) => match s.code() {
                    Some(c) => json!(c),
                    None => json!(1),
                },
                Err(e) => json!(io_code(&e)),
            };
            Ran { code, stdout: String::from_utf8_lossy(&o).into_owned(), stderr: String::from_utf8_lossy(&e).into_owned(), timed_out: false }
        }
        // Dropping the child kills it (kill_on_drop).
        Err(_) => Ran { code: json!(1), stdout: String::new(), stderr: String::new(), timed_out: true },
    }
}

/// A program started and let go, as open.js's start() does: detached, its
/// output ignored, its exit code too (explorer.exe exits 1 when it worked).
pub fn start_detached(file: &str, args: &[String]) {
    let mut cmd = std::process::Command::new(file);
    xenonet_os::process::Background::no_window(&mut cmd);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    if let Ok(mut child) = cmd.spawn() {
        // Reaped on a thread of its own, so it never lingers as a zombie.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// onPath(cmd, { env }): the program's file on PATH, or None.
pub fn on_path(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    xenonet_os::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut |f| xenonet_os::paths::can_run(f))
}

/// The service's environment, as process.env is.
pub fn env_map() -> Map<String, Value> {
    std::env::vars().map(|(k, v)| (k, Value::String(v))).collect()
}

/// A variable that's set and not empty.
/// The xenonet binary XenoNet's entries in other programs run (runtime.js's
/// xenonetBin): XENONET_BIN, installed XenoNet's launcher, or this one.
pub fn xenonet_bin(home: &str) -> String {
    let env: xenonet_components::runtime::Env = std::env::vars().map(|(k, v)| (k, Value::String(v))).collect();
    let m = xenonet_components::runtime::Machine { platform: platform().into(), arch: std::env::consts::ARCH.into(), home: home.into() };
    let this = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    xenonet_components::runtime::xenonet_bin(&env, &m, &this)
}

pub fn env_set(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Child process environment for a spawn: the service's, with these on
/// top (a JSON object of strings), as `{ ...process.env, ...env }`.
pub fn env_pairs(env: &Map<String, Value>) -> Vec<(String, String)> {
    env.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), xenonet_text::text::display(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn what_isnt_send_runs_on_its_own_thread() {
        let here = std::thread::current().id();
        let (there, v) = local(|| async {
            let held = std::rc::Rc::new(7);
            tokio::task::yield_now().await;
            (std::thread::current().id(), *held)
        })
        .await
        .expect("its answer");
        assert_ne!(there, here);
        assert_eq!(v, 7);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn children_as_exec_file_has_them() {
        let r = exec_file("/bin/sh", &["-c".into(), "printf hi; exit 3".into()], 5000, RunOptions::default()).await;
        assert_eq!((r.code, r.stdout.as_str()), (json!(3), "hi"));
        let r = exec_file("/no/such/program", &[], 5000, RunOptions::default()).await;
        assert_eq!(r.code, json!("ENOENT"));
        let r = exec_file("/bin/sh", &["-c".into(), "sleep 5".into()], 50, RunOptions::default()).await;
        assert!(r.timed_out);
        let r = exec_file("/bin/cat", &[], 5000, RunOptions { input: Some("fed".into()), ..Default::default() }).await;
        assert_eq!(r.stdout, "fed");
    }
}
