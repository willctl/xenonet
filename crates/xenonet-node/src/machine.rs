//! Asynchronous work whose decisions are Rust's and whose reading is JS's:
//! a Rust function runs on a thread of its own, and each time it needs
//! something from the outside (a command's output, the clock, a store, a
//! fake a test passed in) it asks, and waits. The twin drives it
//! (js/machine.js): it takes each question, answers it, awaiting what it
//! must, and hands the answer back, until the function is done.
//!
//! JS waits only while the function computes between questions, and only
//! one of these runs at a time, since only JS's one thread moves them on.
//! So a lock the functions share is never held across a question: another
//! one may need it before this one is asked on.

use crate::js::{Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

enum Out {
    Ask(Value),
    All(Vec<Value>),
    Done(Jv),
    Fail(String, Option<String>),
}

// Jv holds raw napi values only when built on JS's thread, which these never are.
unsafe impl Send for Out {}

/// What the twin answered: the value, or the error it threw, as
/// `{message, code?, stdout?}`.
pub type Answer = std::result::Result<Value, Value>;

/// The asking end, handed to the function.
pub struct Asker {
    out: SyncSender<Out>,
    answers: Receiver<Value>,
}

fn answer_of(v: Value) -> Answer {
    match v {
        Value::Object(mut m) => match m.remove("error") {
            Some(e) => Err(e),
            None => Ok(m.remove("ok").unwrap_or(Value::Null)),
        },
        _ => Err(json!({ "message": "the twin gave no answer" })),
    }
}

impl Asker {
    /// Asks the twin one thing.
    pub fn ask(&self, question: Value) -> Answer {
        if self.out.send(Out::Ask(question)).is_err() {
            return Err(json!({ "message": "no longer wanted" }));
        }
        match self.answers.recv() {
            Ok(v) => answer_of(v),
            Err(_) => Err(json!({ "message": "no longer wanted" })),
        }
    }

    /// Asks several things at once (the twin answers them together, as
    /// Promise.all), the answers in order.
    pub fn ask_all(&self, questions: Vec<Value>) -> Vec<Answer> {
        let n = questions.len();
        if n == 0 {
            return Vec::new();
        }
        let gone = || (0..n).map(|_| Err(json!({ "message": "no longer wanted" }))).collect();
        if self.out.send(Out::All(questions)).is_err() {
            return gone();
        }
        match self.answers.recv() {
            Ok(Value::Array(a)) => a.into_iter().map(answer_of).collect(),
            _ => gone(),
        }
    }
}

/// An error's message (and code) as the twin throws it.
pub struct Thrown {
    pub message: String,
    pub code: Option<String>,
}

impl From<String> for Thrown {
    fn from(message: String) -> Self {
        Thrown { message, code: None }
    }
}

impl From<&str> for Thrown {
    fn from(message: &str) -> Self {
        Thrown { message: message.into(), code: None }
    }
}

/// A function on its way, for the twin to drive.
#[napi]
pub struct Machine {
    from: Receiver<Out>,
    to: SyncSender<Value>,
    started: bool,
    over: bool,
}

/// Starts `f` on a thread of its own; its first question (or its answer)
/// waits for the twin's first `next()`.
pub fn start<F>(f: F) -> Machine
where
    F: FnOnce(&Asker) -> std::result::Result<Jv, Thrown> + Send + 'static,
{
    let (out, from) = sync_channel(1);
    let (to, answers) = sync_channel(1);
    std::thread::spawn(move || {
        let asker = Asker { out, answers };
        let done = match catch_unwind(AssertUnwindSafe(|| f(&asker))) {
            Ok(Ok(v)) => Out::Done(v),
            Ok(Err(t)) => Out::Fail(t.message, t.code),
            Err(p) => Out::Fail(format!("XenoNet's Rust core failed: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()), None),
        };
        let _ = asker.out.send(done);
    });
    Machine { from, to, started: false, over: false }
}

#[napi]
impl Machine {
    /// The answer to the last question (none the first time), and back the
    /// next question: `{ask}` or `{all}`, or `{done}` or `{error, code?}`.
    #[napi]
    pub fn next(&mut self, answer: MaybeJson) -> Result<Jv> {
        if self.over {
            return Err(Error::new(Status::GenericFailure, "this machine is done".to_string()));
        }
        if self.started {
            self.to.send(answer.0.unwrap_or(Value::Null)).map_err(|_| Error::new(Status::GenericFailure, "the Rust side went away".to_string()))?;
        }
        self.started = true;
        let out = self.from.recv().map_err(|_| Error::new(Status::GenericFailure, "the Rust side went away".to_string()))?;
        Ok(match out {
            Out::Ask(q) => Jv::Obj(vec![("ask", Jv::Json(q))]),
            Out::All(qs) => Jv::Obj(vec![("all", Jv::Json(Value::Array(qs)))]),
            Out::Done(v) => {
                self.over = true;
                Jv::Obj(vec![("done", v)])
            }
            Out::Fail(message, code) => {
                self.over = true;
                let mut o = vec![("error", Jv::Json(Value::String(message)))];
                if let Some(c) = code {
                    o.push(("code", Jv::Json(Value::String(c))));
                }
                Jv::Obj(o)
            }
        })
    }
}
