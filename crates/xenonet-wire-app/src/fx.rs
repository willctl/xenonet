//! The plumbing the twins' JS does around the Rust cores, natively: child
//! processes as execFile and spawn run them, the PATH as core/paths.js's
//! onPath reads it, fetch, and tasks (crates/xenonet-app/src/task.rs) run
//! with tokio doing the asking.
//!
//! A task is async Rust that isn't Send (its asking is shared between the
//! future and whoever steps it), so the tasks run on a thread of their own,
//! one current-thread runtime with a LocalSet; what each one asks for runs
//! on the service's own runtime, which the task awaits.

use serde_json::{json, Map, Value};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use xenonet_app::task::{Done, Failure, Io, Outcome, Reply, Task};

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// What answers a task's requests ({ op, … }): its reply, or what it threw
/// (`{ message, code?, … }`, as task.js's describe() has it).
pub type Perform = Arc<dyn Fn(Value) -> BoxFuture<Reply> + Send + Sync>;

/// process.platform.
pub fn platform() -> &'static str {
    xenonet_service::borrowed::platform()
}

pub fn now_ms() -> f64 {
    xenonet_app::task::now_ms()
}

/// An error as a task reads it: its message, and its code when it has one.
pub fn error(message: impl Into<String>, code: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("message".into(), Value::String(message.into()));
    if let Some(c) = code {
        m.insert("code".into(), json!(c));
    }
    Value::Object(m)
}

/// A task's failure, as the JS would have thrown it: its message and other
/// fields; Rethrow is the error the last request threw.
pub fn failure_value(f: Failure, last: Option<Value>) -> Value {
    match f {
        Failure::Rethrow => last.unwrap_or_else(|| error("a task threw on an error it didn't have", None)),
        Failure::Error { message, fields, kind } => {
            let mut m = Map::new();
            m.insert("message".into(), Value::String(message));
            if kind != "Error" {
                m.insert("name".into(), json!(kind));
            }
            for (k, v) in fields {
                m.insert(k, v);
            }
            Value::Object(m)
        }
    }
}

/// The message of an error value (`err.message`).
pub fn message_of(e: &Value) -> String {
    match e.get("message") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => xenonet_base::text::js_string(v),
        None => xenonet_base::text::js_string(e),
    }
}

type Job = Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ()>>> + Send>;

// The thread the tasks run on, started the first time one is.
fn jobs() -> &'static mpsc::UnboundedSender<Job> {
    static JOBS: OnceLock<mpsc::UnboundedSender<Job>> = OnceLock::new();
    JOBS.get_or_init(|| {
        let (tx, mut rx) = mpsc::unbounded_channel::<Job>();
        std::thread::Builder::new()
            .name("xenonet-tasks".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async move {
                    while let Some(job) = rx.recv().await {
                        tokio::task::spawn_local(job());
                    }
                });
            })
            .expect("a thread for the tasks");
        tx
    })
}

/// Runs a task to its end: each request is answered by `perform` on the
/// service's runtime, a sleep here. Its value, or what it threw.
pub async fn drive<F, Fut>(make: F, perform: Perform) -> Result<Value, Value>
where
    F: FnOnce(Io) -> Fut + Send + 'static,
    Fut: Future<Output = Done> + 'static,
{
    let handle = tokio::runtime::Handle::current();
    let (tx, rx) = oneshot::channel();
    let job: Job = Box::new(move || {
        Box::pin(async move {
            let mut task = Task::new(make);
            let mut reply: Option<Reply> = None;
            let mut last: Option<Value> = None;
            let out = loop {
                match task.step(reply.take()) {
                    Outcome::Ask(q) => {
                        let r = if q.get("op").and_then(Value::as_str) == Some("sleep") {
                            let ms = q.get("ms").and_then(Value::as_f64).unwrap_or(0.0).max(0.0);
                            tokio::time::sleep(Duration::from_millis(ms as u64)).await;
                            Ok(Value::Null)
                        } else {
                            match handle.spawn(perform(q)).await {
                                Ok(r) => r,
                                Err(e) => Err(error(format!("a request failed: {e}"), None)),
                            }
                        };
                        if let Err(e) = &r {
                            last = Some(e.clone());
                        }
                        reply = Some(r);
                    }
                    Outcome::Done(v) => break Ok(v),
                    Outcome::Failed(f) => break Err(failure_value(f, last.take())),
                }
            };
            let _ = tx.send(out);
        })
    });
    if jobs().send(job).is_err() {
        return Err(error("XenoNet couldn't run a task", None));
    }
    rx.await.unwrap_or_else(|_| Err(error("a task stopped before it was done", None)))
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
    xenonet_base::fs::code_of(e).map_or_else(|| e.to_string(), str::to_string)
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
    xenonet_base::paths::on_path(cmd, &path, pathext.as_deref(), cfg!(windows), &mut |f| xenonet_base::paths::can_run(f))
}

/// The service's environment, as process.env is.
pub fn env_map() -> Map<String, Value> {
    std::env::vars().map(|(k, v)| (k, Value::String(v))).collect()
}

/// A variable that's set and not empty.
pub fn env_set(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// A fetch as the tasks ask for it ({ url, method, headers, body,
/// timeoutMs, read }), answered as task.js's fetchAsked() answers it.
pub async fn fetch_asked(q: &Value) -> Reply {
    let url = q.get("url").and_then(Value::as_str).unwrap_or("");
    let method = q.get("method").and_then(Value::as_str).unwrap_or("GET");
    let headers: Vec<(String, String)> =
        q.get("headers").and_then(Value::as_object).map(|h| h.iter().map(|(k, v)| (k.clone(), xenonet_base::text::js_string(v))).collect()).unwrap_or_default();
    let body = q.get("body").filter(|b| !b.is_null()).map(|b| match b {
        Value::String(s) => s.clone().into_bytes(),
        other => xenonet_base::json::stringify(other, 0).into_bytes(),
    });
    let timeout = q.get("timeoutMs").and_then(Value::as_f64).filter(|t| *t > 0.0).unwrap_or(300e3);
    let res = xenonet_service::net::fetch(method, url, &headers, body, Duration::from_millis(timeout as u64)).await.map_err(|m| fetch_error(&m))?;
    let mut out = json!({ "ok": res.ok(), "status": res.status, "statusText": "" });
    match q.get("read").and_then(Value::as_str) {
        Some("json") => match xenonet_base::json::parse(&res.text()) {
            Ok(v) => out["body"] = v,
            Err(_) => out["bodyError"] = json!(true),
        },
        Some("json!") => match xenonet_base::json::parse(&res.text()) {
            Ok(v) => out["body"] = v,
            Err(e) => return Err(error(format!("Unexpected token in JSON: {e:?}"), None)),
        },
        _ => {}
    }
    Ok(out)
}

/// What fetch threw, as describe() has it: a timeout is a TimeoutError.
pub fn fetch_error(message: &str) -> Value {
    let mut e = error(message, None);
    if message.contains("aborted due to timeout") {
        e["name"] = json!("TimeoutError");
        e["code"] = json!(23);
    } else {
        e["name"] = json!("TypeError");
    }
    e
}

/// Child process environment for a spawn: the service's, with these on
/// top (a JSON object of strings), as `{ ...process.env, ...env }`.
pub fn env_pairs(env: &Map<String, Value>) -> Vec<(String, String)> {
    env.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), xenonet_base::text::js_string(v))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn tasks_ask_and_the_service_answers() {
        let perform: Perform = Arc::new(|q: Value| {
            Box::pin(async move {
                if q["op"] == "twice" {
                    Ok(json!(q["n"].as_i64().unwrap() * 2))
                } else {
                    Err(error("no", Some("ENO")))
                }
            })
        });
        let out = drive(
            |io| async move {
                io.sleep(1.0).await;
                let a = io.ask(json!({ "op": "twice", "n": 3 })).await.map_err(|_| Failure::Rethrow)?;
                Ok(json!({ "a": a }))
            },
            perform.clone(),
        )
        .await;
        assert_eq!(out, Ok(json!({ "a": 6 })));
        let out = drive(
            |io| async move {
                io.ask(json!({ "op": "other" })).await.map_err(|_| Failure::Rethrow)?;
                Ok(Value::Null)
            },
            perform,
        )
        .await;
        assert_eq!(out, Err(json!({ "message": "no", "code": "ENO" })));
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
