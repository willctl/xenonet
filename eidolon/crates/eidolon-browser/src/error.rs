use adytum_driver::DriverError;
use std::fmt;

/// Why setting up, reaching or supervising the browser failed.
#[derive(Debug)]
pub enum BrowserError {
    /// The cell's driver said no or failed.
    Driver(DriverError),
    /// A command of the plan ran and failed.
    Step {
        /// What the step was for.
        why: String,
        /// The exit code, or `None` if a signal or deadline ended it.
        status: Option<i32>,
        /// The end of what it wrote on stderr.
        stderr: String,
    },
    /// The signing key Mozilla's repository uses is not the one we pin.
    Fingerprint {
        /// The fingerprint the plan pins.
        expected: String,
        /// The fingerprints the downloaded key has.
        found: Vec<String>,
    },
    /// The browser's protocol failed.
    Bidi(eidolon_bidi::Error),
    /// Every attempt to reach the browser failed.
    GaveUp {
        /// How many times it tried.
        attempts: u32,
        /// What the last attempt said.
        last: String,
    },
}

impl fmt::Display for BrowserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Driver(e) => write!(f, "{e}"),
            Self::Step { why, status, stderr } => {
                let how = status.map_or_else(|| "was stopped".to_owned(), |s| format!("exited with status {s}"));
                let detail = if stderr.is_empty() { String::new() } else { format!(" It said: {stderr}") };
                write!(f, "Setting up the browser failed at \"{why}\": the command {how}.{detail} Fix that and run the setup again; finished steps are safe to repeat.")
            }
            Self::Fingerprint { expected, found } => write!(
                f,
                "The key Mozilla's repository signs with is not the one we pin (wanted {expected}, found {}). Nothing was trusted. Check Mozilla's documentation before changing the pin.",
                if found.is_empty() { "none".to_owned() } else { found.join(", ") }
            ),
            Self::Bidi(e) => write!(f, "{e}"),
            Self::GaveUp { attempts, last } => write!(f, "The browser did not answer after {attempts} attempts; the last said: {last}. Start it with `systemctl start eidolon-browser` in the cell and check `journalctl -u eidolon-browser`."),
        }
    }
}

impl std::error::Error for BrowserError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Driver(e) => Some(e),
            Self::Bidi(e) => Some(e),
            _ => None,
        }
    }
}

impl From<DriverError> for BrowserError {
    fn from(e: DriverError) -> Self {
        Self::Driver(e)
    }
}

impl From<eidolon_bidi::Error> for BrowserError {
    fn from(e: eidolon_bidi::Error) -> Self {
        Self::Bidi(e)
    }
}

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, BrowserError>;
