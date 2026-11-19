//! The `adytum` command. [`run`] takes the arguments, a driver and two
//! writers, so tests drive it in-process with a fake driver; the binary
//! hands it the Lima driver and the real stdout and stderr.
//!
//! Every command except `help` and `doctor` opens the ledger
//! (`<ADYTUM_HOME>/adytum.sqlite`) and reconciles it with the driver first.
//! What reconcile did goes to stderr when it did anything, and into `--json`
//! output as `"reconciled"`. A cell the ledger never recorded is reported by
//! `ls` and never touched. [`cell_wall`] reads a cell's wall from the ledger
//! for callers outside this crate.
//!
//! Every command takes `--json`: one JSON object on stdout, and an error as
//! `{"error": {"code", "message", "fix"}}`. Without it, an error reads
//! `adytum: <message> <fix>` on stderr. Exit codes: 0 done, 1 refused or
//! failed, 2 usage error.

mod args;
mod commands;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use adytum_driver::{Driver, DriverError};
use adytum_model::{CellName, Wall};
use serde_json::{json, Value};

pub use adytum_ledger::{Ledger, LedgerError};
pub use args::USAGE;

/// The wall of a cell, read from the ledger in `home`. A cell the ledger
/// never recorded is [`LedgerError::NotRecorded`].
pub fn cell_wall(home: &Path, name: &CellName) -> Result<Wall, LedgerError> {
    let ledger = Ledger::open(home)?;
    ledger.cells()?.into_iter().find(|c| &c.spec.name == name).map(|c| c.spec.wall).ok_or_else(|| LedgerError::NotRecorded(name.clone()))
}

/// What a run needs besides the arguments.
pub struct Context {
    /// The driver that makes and runs cells.
    pub driver: Arc<dyn Driver>,
    /// Adytum's folder (`$ADYTUM_HOME`), where the ledger and Lima's folder live.
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

impl From<LedgerError> for Failure {
    fn from(e: LedgerError) -> Self {
        match e {
            LedgerError::Refused(r) => Failure::from(DriverError::Refused(r)),
            LedgerError::Driver(e) => Failure::from(e),
            LedgerError::NotRecorded(n) => Failure {
                code: "not_recorded".to_owned(),
                message: format!("The ledger has no record of a cell named {n}, so Adytum leaves it alone."),
                fix: "Run `adytum ls` to see the cells it made.".to_owned(),
                exit: 1,
            },
            LedgerError::Exists(n) => Failure {
                code: "name_taken".to_owned(),
                message: format!("The ledger already has a cell named {n}."),
                fix: "Pick another name or remove that cell first.".to_owned(),
                exit: 1,
            },
            other => {
                let code = match &other {
                    LedgerError::SetUpAgain(_) | LedgerError::Corrupt(_) => "ledger_unreadable",
                    _ => "ledger_failed",
                };
                let text = other.to_string();
                Failure { code: code.to_owned(), message: text.strip_prefix("adytum: ").unwrap_or(&text).to_owned(), fix: String::new(), exit: 1 }
            }
        }
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

impl Done {
    pub(crate) fn new(json: Value, text: String) -> Done {
        Done { json, text, exit: 0 }
    }
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
        write_line(err, format!("adytum: {} {}", failure.message, failure.fix).trim_end());
    }
    failure.exit
}

#[cfg(test)]
mod tests;
