use eidolon_browser::BrowserError;
use std::fmt;
use std::net::IpAddr;

/// Why opening, closing or checking the sign-in window failed.
#[derive(Debug)]
pub enum ViewError {
    /// A plan in the cell failed.
    Browser(BrowserError),
    /// The window may listen on 127.0.0.1 only.
    NotLoopback {
        /// The address asked for.
        address: IpAddr,
    },
    /// The system could not give a random secret.
    NoRandom(String),
    /// A file or socket on the host could not be used.
    Io {
        /// What was being done.
        what: &'static str,
        /// The cause.
        source: std::io::Error,
    },
    /// The sign-in window is open, so automation is off.
    Open,
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browser(e) => write!(f, "{e}"),
            Self::NotLoopback { address } => write!(f, "The sign-in window listens on 127.0.0.1 only, and {address} is not that. Use 127.0.0.1."),
            Self::NoRandom(why) => write!(f, "The system gave no random bytes for the window's secret ({why}). Nothing was opened; try again."),
            Self::Io { what, source } => write!(f, "Could not {what}: {source}."),
            Self::Open => write!(f, "The sign-in window is open; close it to drive the browser."),
        }
    }
}

impl std::error::Error for ViewError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Browser(e) => Some(e),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<BrowserError> for ViewError {
    fn from(e: BrowserError) -> Self {
        Self::Browser(e)
    }
}

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, ViewError>;
