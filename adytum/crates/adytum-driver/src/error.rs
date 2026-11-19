use std::fmt;

use adytum_exec::ExecError;
use adytum_model::Refusal;

/// Why a driver call did not work.
#[derive(Debug)]
pub enum DriverError {
    /// A rule said no before anything ran.
    Refused(Refusal),
    /// The tool ran and failed.
    Tool {
        /// What was run, such as `limactl create`.
        what: String,
        /// Its exit code, or `None` if a signal or a deadline ended it.
        status: Option<i32>,
        /// The end of what it wrote on stderr.
        stderr: String,
    },
    /// No such cell.
    NotFound {
        /// The name asked for.
        name: String,
    },
    /// The tool could not be started or read.
    Exec(ExecError),
    /// A file or folder on the host could not be used.
    Io {
        /// What was being done, such as "write the template".
        what: &'static str,
        /// The cause.
        source: std::io::Error,
    },
    /// A path that cannot be used, with the rule it broke.
    BadPath {
        /// The path as given.
        path: String,
        /// The rule, as a sentence fragment such as "it must be absolute".
        rule: &'static str,
    },
    /// This driver does not do what the spec or call asks.
    Unsupported {
        /// What is not supported, as a short phrase.
        what: String,
    },
}

impl DriverError {
    /// A stable code, a sentence saying what happened, and a sentence saying
    /// what to do. The CLI prints these and a script matches on the code.
    pub fn parts(&self) -> (String, String, String) {
        match self {
            Self::Refused(r) => (code_text(r), r.message.clone(), r.fix.clone()),
            Self::Tool { what, status, stderr } => {
                let how = status.map_or_else(|| "was stopped".to_owned(), |s| format!("exited with status {s}"));
                let detail = if stderr.is_empty() { String::new() } else { format!(" It said: {stderr}") };
                ("tool_failed".to_owned(), format!("{what} {how}.{detail}"), "Run `adytum doctor` once it exists, or check the message above and try again.".to_owned())
            }
            Self::NotFound { name } => ("not_found".to_owned(), format!("There is no cell named {name}."), "Run `adytum ls` to see the cells.".to_owned()),
            Self::Exec(e) => ("exec_failed".to_owned(), e.to_string(), "Check that limactl is installed and on PATH.".to_owned()),
            Self::Io { what, source } => ("io".to_owned(), format!("Could not {what}: {source}."), "Check that the folder exists and can be written.".to_owned()),
            Self::BadPath { path, rule } => ("bad_path".to_owned(), format!("The path {path:?} cannot be used: {rule}."), "Give an absolute path without .. in it.".to_owned()),
            Self::Unsupported { what } => ("unsupported".to_owned(), format!("This driver does not support {what}."), "Remove it from the spec.".to_owned()),
        }
    }
}

fn code_text(r: &Refusal) -> String {
    // The stable name the refusal code has in JSON.
    serde_json::to_value(r.code).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_else(|| "refused".to_owned())
}

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "{r}"),
            other => {
                let (_, message, fix) = other.parts();
                write!(f, "adytum: {message} {fix}")
            }
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(r) => Some(r),
            Self::Exec(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<Refusal> for DriverError {
    fn from(r: Refusal) -> Self {
        Self::Refused(r)
    }
}

impl From<ExecError> for DriverError {
    fn from(e: ExecError) -> Self {
        Self::Exec(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adytum_model::RefusalCode;

    #[test]
    fn refusal_codes_match_their_json_names() {
        let r = Refusal::new(RefusalCode::AutomationPortPublished, "m", "f");
        assert_eq!(DriverError::Refused(r).parts().0, "automation_port_published");
        let r = Refusal::new(RefusalCode::NameNotOurs, "m", "f");
        assert_eq!(DriverError::Refused(r).parts().0, "name_not_ours");
    }

    #[test]
    fn a_tool_failure_shows_its_stderr() {
        let e = DriverError::Tool { what: "limactl create".into(), status: Some(1), stderr: "disk full".into() };
        let text = e.to_string();
        assert!(text.starts_with("adytum: limactl create exited with status 1."), "{text}");
        assert!(text.contains("disk full"));
    }
}
