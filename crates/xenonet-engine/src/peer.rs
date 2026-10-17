//! Calls to the parts a module was given, as its JS made them: a method of
//! one of its collaborators (the actors, a store, procs, a fake a test
//! passed in), looked up when it's called, so whatever stands in its place
//! then is what's called. Behind a twin, the call goes to the JS object;
//! in the binary, to the Rust part.

use crate::sql::Arg;
use serde_json::Value;

/// What a call threw: its message and code, and a handle the twin keeps
/// the thrown value under, so that what's thrown on is the same value.
#[derive(Clone, Debug, PartialEq)]
pub struct Thrown {
    pub message: String,
    pub code: Option<Value>,
    pub handle: Option<u64>,
    /// What a failed command wrote anyway.
    pub stdout: Option<String>,
}

impl Thrown {
    pub fn new(message: impl Into<String>) -> Self {
        Thrown { message: message.into(), code: None, handle: None, stdout: None }
    }
    pub fn coded(message: impl Into<String>, code: &str) -> Self {
        Thrown { message: message.into(), code: Some(Value::String(code.into())), handle: None, stdout: None }
    }
}

impl From<crate::sql::SqlError> for Thrown {
    fn from(e: crate::sql::SqlError) -> Self {
        Thrown { message: e.message().to_string(), code: e.code().map(|c| Value::String(c.into())), handle: None, stdout: None }
    }
}

impl std::fmt::Display for Thrown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type Result<T> = std::result::Result<T, Thrown>;

pub trait Calls {
    /// `<path>.<method>(...args)`, from the module's own collaborators
    /// (a path such as "actors" or "actors.store"), its answer awaited
    /// where the call is asynchronous.
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg>;
    /// `<path>`, read.
    fn get(&mut self, path: &str) -> Result<Arg>;
    /// What `<path>` is, as typeof says it ("null" for null), without
    /// reading it whole: a collaborator's field can be a whole object graph.
    fn kind(&mut self, path: &str) -> Result<String>;
    /// The time, by the module's clock.
    fn now(&mut self) -> f64 {
        self.call("clock", "now", vec![]).ok().and_then(|a| a.as_f64()).unwrap_or(f64::NAN)
    }
}

/// `<path>?.<method>?.()`: Undef when there's nothing to call.
pub fn opt_call(calls: &mut dyn Calls, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
    if matches!(calls.kind(path)?.as_str(), "undefined" | "null") {
        return Ok(Arg::Undef);
    }
    if calls.kind(&format!("{path}.{method}"))? != "function" {
        return Ok(Arg::Undef);
    }
    calls.call(path, method, args)
}

/// A list as a call answered it: its items, or none.
pub fn list(a: Arg) -> Vec<Arg> {
    match a {
        Arg::List(l) => l,
        _ => Vec::new(),
    }
}

impl From<String> for Thrown {
    fn from(message: String) -> Self {
        Thrown::new(message)
    }
}

impl From<&str> for Thrown {
    fn from(message: &str) -> Self {
        Thrown::new(message)
    }
}
