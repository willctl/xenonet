use std::ffi::OsString;
use std::fmt;

/// Why a command could not be run. A command that ran and failed is an
/// `Output` with a status, not an error.
#[derive(Debug)]
pub enum ExecError {
    /// The command has no program name.
    EmptyProgram,
    /// The program isn't on the `PATH` the command names (or this process's).
    NotFound { program: OsString },
    /// The program was found but could not be started.
    Spawn { program: OsString, source: std::io::Error },
    /// Reading from or waiting on a running child failed.
    Io { what: &'static str, source: std::io::Error },
    /// The runner has no answer for this command (a fake with no call left).
    Unexpected { message: String },
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::EmptyProgram => write!(f, "The command has no program name. Give Command::new a program."),
            ExecError::NotFound { program } => write!(f, "The program {} was not found on PATH. Install it, or give its full path.", program.display()),
            ExecError::Spawn { program, source } => write!(f, "The program {} could not be started: {source}. Check that it is executable.", program.display()),
            ExecError::Io { what, source } => write!(f, "Could not {what}: {source}."),
            ExecError::Unexpected { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExecError::Spawn { source, .. } | ExecError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
