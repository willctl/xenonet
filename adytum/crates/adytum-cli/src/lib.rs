//! The `adytum` command. [`run`] takes the arguments, a driver and two
//! writers, so tests drive it in-process with a fake driver; the binary
//! hands it the Lima driver and the real stdout and stderr.
//!
//! Every command takes `--json`: one JSON object on stdout, and an error as
//! `{"error": {"code", "message", "fix"}}`. Without it, an error reads
//! `adytum: <message> <fix>` on stderr. Exit codes: 0 done, 1 refused or
//! failed, 2 usage error.

mod args;
mod commands;
mod records;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use adytum_driver::{Driver, DriverError};
use serde_json::{json, Value};

pub use args::USAGE;

/// What a run needs besides the arguments.
pub struct Context {
    /// The driver that makes and runs cells.
    pub driver: Arc<dyn Driver>,
    /// Adytum's folder (`$ADYTUM_HOME`), where the cell records are kept.
    pub home: PathBuf,
}

/// A command that did not finish, ready to print.
pub(crate) struct Failure {
    pub code: String,
    pub message: String,
    pub fix: String,
    pub exit: i32,
}

impl From<DriverError> for Failure {
    fn from(e: DriverError) -> Self {
        let (code, message, fix) = e.parts();
        Failure { code, message, fix, exit: 1 }
    }
}

impl From<args::Usage> for Failure {
    fn from(u: args::Usage) -> Self {
        Failure { code: "usage".to_owned(), message: u.message, fix: u.fix, exit: 2 }
    }
}

/// What a finished command shows.
pub(crate) struct Done {
    /// The JSON object for `--json`.
    pub json: Value,
    /// The text for a person.
    pub text: String,
    /// The exit code: 1 when a command ran in a cell and failed.
    pub exit: i32,
}

fn write_line(w: &mut dyn Write, text: &str) {
    // A closed pipe is the reader's choice, not an error worth reporting.
    let _ = writeln!(w, "{text}");
}

/// Runs one command line and returns the exit code.
pub async fn run(tokens: &[String], ctx: &Context, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let json_wanted = args::wants_json(tokens);
    let parsed = match args::parse(tokens) {
        Ok(p) => p,
        Err(usage) => return fail(Failure::from(usage), json_wanted, out, err),
    };
    if parsed.command == args::Command::Help {
        write_line(out, USAGE.trim_end());
        return 0;
    }
    match commands::execute(parsed.command, ctx, out, err, parsed.json).await {
        Ok(done) => {
            if parsed.json {
                write_line(out, &done.json.to_string());
            } else if !done.text.is_empty() {
                write_line(out, &done.text);
            }
            done.exit
        }
        Err(failure) => fail(failure, parsed.json, out, err),
    }
}

fn fail(failure: Failure, json: bool, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if json {
        write_line(out, &json!({ "error": { "code": failure.code, "message": failure.message, "fix": failure.fix } }).to_string());
    } else {
        write_line(err, &format!("adytum: {} {}", failure.message, failure.fix));
    }
    failure.exit
}

#[cfg(test)]
mod tests;
