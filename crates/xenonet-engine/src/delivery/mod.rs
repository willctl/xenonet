//! Live delivery (core/delivery/): into a session now, by the fastest way
//! in that's allowed, under the policy's guards.
//!
//! Each module asks for what it needs of the machine (the clock, files,
//! commands, HTTP) and of the other cores through a trait of its own, so
//! the binary brings the real ones and a test a fake.

pub mod apps;
pub mod channel;
pub mod delegation;
pub mod opencode;
pub mod policy;
pub mod resume;
pub mod router;
pub mod runs;
pub mod selector;
pub mod setup;
pub mod starts;
pub mod tmux;
pub mod usage;

use serde_json::Value;

/// What a step of delivery failed with: words for the sender, the audit
/// or the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Error(message.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// How a way in went: where the message went (a pane, a server, a pid),
/// or why it didn't.
pub type Delivered = std::result::Result<String, String>;

/// The files delivery keeps and reads: the policy's, its audit log, the
/// wake budget's counts and the listen settings. Errors are the system's
/// words for them.
pub trait Files {
    fn read_file(&mut self, path: &str) -> std::result::Result<String, String>;
    fn mkdir_all(&mut self, dir: &str) -> std::result::Result<(), String>;
    /// Written whole; `mode` is the file's permission bits where the
    /// system has them.
    fn write_file(&mut self, path: &str, text: &str, mode: Option<u32>) -> std::result::Result<(), String>;
    fn append_file(&mut self, path: &str, text: &str, mode: Option<u32>) -> std::result::Result<(), String>;
    fn rename(&mut self, from: &str, to: &str) -> std::result::Result<(), String>;
}

/// A number as JS writes it in text: 20, 1.5, NaN.
pub fn number_text(n: f64) -> String {
    xenonet_text::json::number_string(n)
}

/// A JSON value as text, as a template literal writes it ("" for what's
/// missing or null).
pub fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => xenonet_text::text::display(v),
    }
}

/// A JSON value as `${v}` writes it: null for null, [object Object] for an
/// object.
pub fn template(v: &Value) -> String {
    xenonet_text::text::display(v)
}

/// JS's truthiness of a JSON value, a missing one false.
pub fn truthy(v: Option<&Value>) -> bool {
    xenonet_text::text::truthy(v)
}
