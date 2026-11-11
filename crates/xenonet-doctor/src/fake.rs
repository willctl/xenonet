//! A made-up machine for the checks' tests: it answers what the tests ask
//! of it, and nothing else.

use crate::checks::DoctorHost;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::future::Future;
use xenonet_os::error::Failure;

/// Runs a check to its end: nothing a fake host does waits.
pub fn ready<F: Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match f.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("a fake host's check waited"),
    }
}

type Run = Box<dyn Fn(&str, &[&str]) -> Value>;

/// What the fake answers: a program's output by its file and arguments,
/// how the service starts at login, and the machine's network.
pub struct Fake {
    pub run: Run,
    pub login: Option<Value>,
    pub network: Option<Value>,
}

impl Default for Fake {
    fn default() -> Self {
        Fake { run: Box::new(|file, args| json!({ "stdout": format!("ran {file} {}", args.first().unwrap_or(&"")), "code": 0 })), login: None, network: None }
    }
}

#[async_trait(?Send)]
impl DoctorHost for Fake {
    async fn now(&self) -> f64 {
        42.0
    }
    async fn local_time(&self, ms: f64) -> String {
        format!("at {ms:?}")
    }
    async fn which(&self, _cmd: &str) -> Option<String> {
        Some("/x/tool".into())
    }
    async fn exists(&self, path: &str) -> bool {
        path == "/x/here"
    }
    async fn run(&self, file: &str, args: &[&str], _ms: f64) -> Value {
        (self.run)(file, args)
    }
    async fn read_text(&self, _path: &str) -> Option<String> {
        Some("hi".into())
    }
    async fn readdir(&self, path: &str) -> Result<Vec<String>, Failure> {
        if path == "/x/dir" {
            Ok(vec!["a".into(), "b".into()])
        } else {
            Err(Failure::coded("EACCES", "EACCES"))
        }
    }
    async fn writable(&self, _dir: &str) -> bool {
        unreachable!("writable")
    }
    async fn statfs(&self, _dir: &str) -> Option<Value> {
        unreachable!("statfs")
    }
    async fn command(&self) -> Value {
        unreachable!("command")
    }
    async fn saved_path(&self) -> Value {
        unreachable!("saved_path")
    }
    async fn procs(&self) -> Value {
        unreachable!("procs")
    }
    async fn git_available(&self) -> bool {
        unreachable!("git_available")
    }
    async fn launcher(&self) -> Result<Value, Failure> {
        unreachable!("launcher")
    }
    async fn errors(&self) -> Value {
        unreachable!("errors")
    }
    async fn drift(&self) -> Value {
        unreachable!("drift")
    }
    async fn login(&self) -> Option<Value> {
        self.login.clone()
    }
    async fn network(&self) -> Option<Value> {
        self.network.clone()
    }
    async fn restart_exe(&self) -> Option<Value> {
        Some(Value::Null)
    }
    async fn realpath(&self, _path: &str) -> Option<Value> {
        Some(Value::Null)
    }
}
