//! Async Rust whose waiting is done by someone else: the JS around a twin,
//! during the port. A task is an `async` block that, wherever the JS
//! awaited a fetch, a child process, a timer or a callback it was given,
//! awaits `io.ask(request)` instead. Stepping the task runs it until it
//! asks for something (`Outcome::Ask`), then stops; whoever runs it does
//! what was asked and steps it again with the reply, until it's done.
//!
//! So the logic of an async JS module ports as it reads, await for await,
//! and its tests' fakes still answer it. In the binary, the same tasks run
//! with tokio doing the asking.

use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// What a request came back with: its value, or what it threw (its
/// message, and the error's other fields the logic reads: code, name,
/// causeCode…).
pub type Reply = Result<Value, Value>;

/// Why a task ended without its value.
#[derive(Clone, Debug, PartialEq)]
pub enum Failure {
    /// An Error with this message and these other fields (code, status…),
    /// a TypeError when `kind` says so.
    Error { message: String, fields: Map<String, Value>, kind: &'static str },
    /// The error the last request threw, thrown on as it was.
    Rethrow,
}

impl Failure {
    pub fn msg(message: impl Into<String>) -> Self {
        Failure::Error { message: message.into(), fields: Map::new(), kind: "Error" }
    }

    pub fn coded(message: impl Into<String>, code: &str) -> Self {
        let mut fields = Map::new();
        fields.insert("code".into(), json!(code));
        Failure::Error { message: message.into(), fields, kind: "Error" }
    }

    pub fn with(message: impl Into<String>, fields: Map<String, Value>) -> Self {
        Failure::Error { message: message.into(), fields, kind: "Error" }
    }

    pub fn type_error(message: impl Into<String>) -> Self {
        Failure::Error { message: message.into(), fields: Map::new(), kind: "TypeError" }
    }

    /// The message, as `err.message` reads it (Rethrow has none of its own).
    pub fn message(&self) -> &str {
        match self {
            Failure::Error { message, .. } => message,
            Failure::Rethrow => "",
        }
    }
}

impl From<crate::fsx::FsError> for Failure {
    fn from(e: crate::fsx::FsError) -> Self {
        let mut fields = Map::new();
        if let Some(c) = e.code {
            fields.insert("code".into(), json!(c));
        }
        Failure::Error { message: e.message, fields, kind: "Error" }
    }
}

pub type Done = Result<Value, Failure>;

enum Slot {
    Idle,
    Asked(Value),
    Replied(Reply),
}

/// How a task asks for what it waits on.
#[derive(Clone)]
pub struct Io {
    slot: Rc<RefCell<Slot>>,
    last_error: Rc<RefCell<Option<Value>>>,
}

struct AskFuture {
    slot: Rc<RefCell<Slot>>,
    request: Option<Value>,
}

impl Future for AskFuture {
    type Output = Reply;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Reply> {
        if let Some(q) = self.request.take() {
            *self.slot.borrow_mut() = Slot::Asked(q);
            return Poll::Pending;
        }
        let mut slot = self.slot.borrow_mut();
        match std::mem::replace(&mut *slot, Slot::Idle) {
            Slot::Replied(r) => Poll::Ready(r),
            other => {
                *slot = other;
                Poll::Pending
            }
        }
    }
}

impl Io {
    /// Asks for `request` ({ op, … }) and waits for the reply.
    pub async fn ask(&self, request: Value) -> Reply {
        let r = AskFuture { slot: self.slot.clone(), request: Some(request) }.await;
        if let Err(e) = &r {
            *self.last_error.borrow_mut() = Some(e.clone());
        }
        r
    }

    /// Waits this long (setTimeout's ms).
    pub async fn sleep(&self, ms: f64) {
        let _ = self.ask(json!({ "op": "sleep", "ms": xenonet_base::json::number(ms) })).await;
    }
}

/// A task: its future, and how it asks.
pub struct Task {
    future: Pin<Box<dyn Future<Output = Done>>>,
    io: Io,
}

/// Where a step left the task.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Ask(Value),
    Done(Value),
    Failed(Failure),
}

impl Task {
    pub fn new<F, Fut>(f: F) -> Task
    where
        F: FnOnce(Io) -> Fut,
        Fut: Future<Output = Done> + 'static,
    {
        let io = Io { slot: Rc::new(RefCell::new(Slot::Idle)), last_error: Rc::new(RefCell::new(None)) };
        Task { future: Box::pin(f(io.clone())), io }
    }

    /// A task that's done already.
    pub fn ready(v: Done) -> Task {
        Task::new(move |_| async move { v })
    }

    /// Runs the task until it asks for something or ends; `reply` answers
    /// what it asked last (None the first time).
    pub fn step(&mut self, reply: Option<Reply>) -> Outcome {
        if let Some(r) = reply {
            *self.io.slot.borrow_mut() = Slot::Replied(r);
        }
        let mut cx = Context::from_waker(Waker::noop());
        match self.future.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(v)) => Outcome::Done(v),
            Poll::Ready(Err(f)) => Outcome::Failed(f),
            Poll::Pending => match std::mem::replace(&mut *self.io.slot.borrow_mut(), Slot::Idle) {
                Slot::Asked(q) => Outcome::Ask(q),
                _ => Outcome::Failed(Failure::msg("a task waited on something that isn't a request")),
            },
        }
    }
}

/// Runs a task to its end, answering each request with `answer`, for
/// tests and for callers that can answer at once.
pub fn run_with(mut task: Task, mut answer: impl FnMut(&Value) -> Reply) -> Done {
    let mut reply = None;
    loop {
        match task.step(reply.take()) {
            Outcome::Ask(q) => reply = Some(answer(&q)),
            Outcome::Done(v) => return Ok(v),
            Outcome::Failed(f) => return Err(f),
        }
    }
}

/// The time now in ms, as Date.now() gives it.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_and_carries_on_with_the_reply() {
        let t = Task::new(|io| async move {
            let a = io.ask(json!({ "op": "twice", "n": 2 })).await.map_err(|_| Failure::Rethrow)?;
            let b = io.ask(json!({ "op": "twice", "n": a })).await;
            match b {
                Ok(v) => Ok(json!({ "got": v })),
                Err(e) => Err(Failure::msg(e["message"].as_str().unwrap_or("").to_string())),
            }
        });
        let out = run_with(t, |q| Ok(json!(q["n"].as_i64().unwrap() * 2)));
        assert_eq!(out, Ok(json!({ "got": 8 })));
        let t = Task::new(|io| async move {
            io.ask(json!({ "op": "x" })).await.map_err(|e| Failure::msg(e["message"].as_str().unwrap_or("").to_string()))?;
            Ok(Value::Null)
        });
        assert_eq!(run_with(t, |_| Err(json!({ "message": "no" }))), Err(Failure::msg("no")));
    }
}
