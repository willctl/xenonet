//! Asynchronous work whose decisions are Rust's and whose reading is JS's:
//! a Rust function runs on a thread of its own, and each time it needs
//! something from the outside (a command's output, the clock, a store, a
//! collaborator's method, a fake a test passed in) it asks, and waits. The
//! twin drives it (js/machine.js): it takes each question, answers it,
//! awaiting what it must, and hands the answer back, until the function is
//! done.
//!
//! JS waits only while the function computes between questions, and only
//! one of these runs at a time, since only JS's one thread moves them on.
//! So a lock the functions share is never held across a question: another
//! one may need it before this one is asked on.
//!
//! What a question's answer threw comes back with a handle; a function
//! that fails with it has the twin throw that very value.

use crate::args::{arg_to_js, JsArg};
use crate::js::Jv;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use xenonet_engine::peer::{self, Thrown};
use xenonet_engine::sql::Arg;

enum Out {
    Ask(Arg),
    All(Vec<Arg>),
    Done(Done),
    Fail(Thrown),
}

/// What a function comes to: a value built as Jv, or one as JS gave it.
pub enum Done {
    Jv(Jv),
    Arg(Arg),
}

impl From<Jv> for Done {
    fn from(v: Jv) -> Self {
        Done::Jv(v)
    }
}

impl From<Arg> for Done {
    fn from(v: Arg) -> Self {
        Done::Arg(v)
    }
}

// Jv holds raw napi values only when built on JS's thread, which these never are.
unsafe impl Send for Out {}

/// What the twin answered: the value, or what it threw.
pub type Answer = std::result::Result<Value, Value>;

/// The asking end, handed to the function.
pub struct Asker {
    out: SyncSender<Out>,
    answers: Receiver<Arg>,
}

fn gone() -> Thrown {
    Thrown::new("no longer wanted")
}

fn thrown_of(e: Arg) -> Thrown {
    let message = match e.field("message") {
        Arg::Text(s) => s,
        Arg::Undef => String::new(),
        other => xenonet_engine::store::js_text(&other),
    };
    let code = match e.field("code") {
        Arg::Undef | Arg::Null => None,
        c => Some(c.to_json()),
    };
    let handle = e.field("handle").as_f64().map(|h| h as u64);
    let stdout = e.field("stdout").as_str().map(str::to_string);
    Thrown { message, code, handle, stdout }
}

fn answer_arg(v: Arg) -> std::result::Result<Arg, Thrown> {
    match v.field("error") {
        Arg::Undef => Ok(v.field("ok")),
        e => Err(thrown_of(e)),
    }
}

impl Asker {
    /// Asks the twin one thing, as JS values.
    pub fn ask_arg(&self, question: Arg) -> std::result::Result<Arg, Thrown> {
        self.out.send(Out::Ask(question)).map_err(|_| gone())?;
        self.answers.recv().map(answer_arg).unwrap_or_else(|_| Err(gone()))
    }

    /// Asks several things at once (the twin answers them together, as
    /// Promise.all), the answers in order.
    pub fn ask_all_arg(&self, questions: Vec<Arg>) -> Vec<std::result::Result<Arg, Thrown>> {
        let n = questions.len();
        if n == 0 {
            return Vec::new();
        }
        if self.out.send(Out::All(questions)).is_err() {
            return (0..n).map(|_| Err(gone())).collect();
        }
        match self.answers.recv() {
            Ok(Arg::List(a)) => a.into_iter().map(answer_arg).collect(),
            _ => (0..n).map(|_| Err(gone())).collect(),
        }
    }

    /// Asks one thing, as JSON: the error as {message, code?, stdout?}.
    pub fn ask(&self, question: Value) -> Answer {
        let q = crate::args::value_to_arg(&question);
        match self.ask_arg(q) {
            Ok(v) => Ok(v.to_json()),
            Err(t) => Err(thrown_json(&t)),
        }
    }

    pub fn ask_all(&self, questions: Vec<Value>) -> Vec<Answer> {
        self.ask_all_arg(questions.iter().map(crate::args::value_to_arg).collect()).into_iter().map(|r| r.map(|v| v.to_json()).map_err(|t| thrown_json(&t))).collect()
    }
}

fn thrown_json(t: &Thrown) -> Value {
    let mut o = json!({ "message": t.message });
    if let Some(c) = &t.code {
        o["code"] = c.clone();
    }
    if let Some(s) = &t.stdout {
        o["stdout"] = Value::String(s.clone());
    }
    o
}

/// A machine's collaborators, through the twin: `call` and `get` questions.
pub struct AskCalls<'a>(pub &'a Asker);

impl peer::Calls for AskCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> peer::Result<Arg> {
        self.0.ask_arg(Arg::Obj(vec![("op".into(), Arg::text("call")), ("path".into(), Arg::text(path)), ("method".into(), Arg::text(method)), ("args".into(), Arg::List(args))]))
    }
    fn get(&mut self, path: &str) -> peer::Result<Arg> {
        self.0.ask_arg(Arg::Obj(vec![("op".into(), Arg::text("get")), ("path".into(), Arg::text(path))]))
    }
}

/// A function on its way, for the twin to drive.
#[napi]
pub struct Machine {
    from: Receiver<Out>,
    to: SyncSender<Arg>,
    started: bool,
    over: bool,
}

/// Starts `f` on a thread of its own; its first question (or its answer)
/// waits for the twin's first `next()`.
pub fn start<F, D>(f: F) -> Machine
where
    F: FnOnce(&Asker) -> std::result::Result<D, Thrown> + Send + 'static,
    D: Into<Done>,
{
    let (out, from) = sync_channel(1);
    let (to, answers) = sync_channel(1);
    std::thread::spawn(move || {
        let asker = Asker { out, answers };
        let done = match catch_unwind(AssertUnwindSafe(|| f(&asker))) {
            Ok(Ok(v)) => Out::Done(v.into()),
            Ok(Err(t)) => Out::Fail(t),
            Err(p) => Out::Fail(Thrown::new(format!("XenoNet's Rust core failed: {}", p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()))),
        };
        let _ = asker.out.send(done);
    });
    Machine { from, to, started: false, over: false }
}

#[napi]
impl Machine {
    /// The answer to the last question (none the first time), and back the
    /// next question: `{ask}` or `{all}`, or `{done}` or `{error, code?,
    /// handle?}`.
    #[napi]
    pub fn next(&mut self, env: Env, answer: JsArg) -> Result<Jv> {
        if self.over {
            return Err(Error::new(Status::GenericFailure, "this machine is done".to_string()));
        }
        if self.started {
            self.to.send(answer.0).map_err(|_| Error::new(Status::GenericFailure, "the Rust side went away".to_string()))?;
        }
        self.started = true;
        let out = self.from.recv().map_err(|_| Error::new(Status::GenericFailure, "the Rust side went away".to_string()))?;
        let raw = |a: &Arg| -> Result<Jv> { Ok(Jv::Raw(unsafe { arg_to_js(env.raw(), a)? })) };
        Ok(match out {
            Out::Ask(q) => Jv::Obj(vec![("ask", raw(&q)?)]),
            Out::All(qs) => Jv::Obj(vec![("all", raw(&Arg::List(qs))?)]),
            Out::Done(d) => {
                self.over = true;
                Jv::Obj(vec![("done", match d {
                    Done::Jv(v) => v,
                    Done::Arg(a) => raw(&a)?,
                })])
            }
            Out::Fail(t) => {
                self.over = true;
                let mut o = vec![("error", Jv::Json(Value::String(t.message)))];
                if let Some(c) = t.code {
                    o.push(("code", Jv::Json(c)));
                }
                if let Some(h) = t.handle {
                    o.push(("handle", Jv::Num(h as f64)));
                }
                Jv::Obj(o)
            }
        })
    }
}
