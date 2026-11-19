//! What a command reports when it does not finish: a stable code, what
//! happened and what to do. Exit 1 for a refusal or failure, 2 for a
//! mistake in how the command was typed.

use eidolon_facet::FacetError;
use eidolon_model::{ModelError, Refusal};
use eidolon_move::MoveError;
use eidolon_pack::PackError;
use eidolon_vault::VaultError;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    /// Stable, for scripts: a refusal's code (`second_seat`) or a short word.
    pub code: String,
    pub message: String,
    pub fix: String,
    pub exit: u8,
}

impl CliError {
    pub fn failed(code: &str, message: impl Into<String>, fix: impl Into<String>) -> CliError {
        CliError { code: code.to_owned(), message: message.into(), fix: fix.into(), exit: 1 }
    }

    pub fn usage(message: impl Into<String>, fix: impl Into<String>) -> CliError {
        CliError { code: "usage".to_owned(), message: message.into(), fix: fix.into(), exit: 2 }
    }

    /// An error whose text already says what to do.
    fn sentence(code: &str, text: String) -> CliError {
        let text = text.strip_prefix("eidolon: ").map_or_else(|| text.clone(), str::to_owned);
        CliError::failed(code, text, "")
    }

    pub fn io(what: &str, e: &std::io::Error) -> CliError {
        CliError::failed("io", format!("Could not {what}: {e}."), "Check the path and its permissions, then try again.")
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({ "ok": false, "code": self.code, "message": self.message, "fix": self.fix })
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.fix.is_empty() {
            write!(f, "eidolon: {}", self.message)
        } else {
            write!(f, "eidolon: {} {}", self.message, self.fix)
        }
    }
}

impl std::error::Error for CliError {}

impl From<Refusal> for CliError {
    fn from(r: Refusal) -> CliError {
        CliError::failed(r.code.as_str(), r.message, r.fix)
    }
}

impl From<ModelError> for CliError {
    fn from(e: ModelError) -> CliError {
        CliError::sentence("bad_value", e.to_string())
    }
}

impl From<VaultError> for CliError {
    fn from(e: VaultError) -> CliError {
        match e {
            VaultError::ChangedChunk | VaultError::ChunkTooLarge | VaultError::MissingChunk(_) => {
                CliError::failed("changed_chunk", e.to_string(), "Move the eidolon again from the machine that sealed it; the damaged chunk is sent again.")
            }
            other => CliError::sentence("vault", other.to_string()),
        }
    }
}

impl From<PackError> for CliError {
    fn from(e: PackError) -> CliError {
        match e {
            PackError::Vault(v) => v.into(),
            other => CliError::sentence("pack", other.to_string()),
        }
    }
}

impl From<FacetError> for CliError {
    fn from(e: FacetError) -> CliError {
        match e {
            FacetError::Pack(p) => p.into(),
            FacetError::Refused(r) => r.into(),
            other => CliError::sentence("facet", other.to_string()),
        }
    }
}

impl From<MoveError> for CliError {
    fn from(e: MoveError) -> CliError {
        match e {
            MoveError::Refused(r) => r.into(),
            MoveError::RefusedByPeer(r) => CliError::failed(r.code.as_str(), format!("The other machine refused: {}", r.message), r.fix),
            MoveError::Vault(v) => v.into(),
            MoveError::Model(m) => m.into(),
            other => CliError::sentence("move", other.to_string()),
        }
    }
}

impl From<adytum_driver::DriverError> for CliError {
    fn from(e: adytum_driver::DriverError) -> CliError {
        let (code, message, fix) = e.parts();
        CliError::failed(&code, message, fix)
    }
}

impl From<adytum_ledger::LedgerError> for CliError {
    fn from(e: adytum_ledger::LedgerError) -> CliError {
        CliError::sentence("adytum", e.to_string())
    }
}

impl From<eidolon_browser::BrowserError> for CliError {
    fn from(e: eidolon_browser::BrowserError) -> CliError {
        CliError::sentence("browser", e.to_string())
    }
}

impl From<eidolon_view::ViewError> for CliError {
    fn from(e: eidolon_view::ViewError) -> CliError {
        match e {
            eidolon_view::ViewError::Open => CliError::failed(
                "view_open",
                "The sign-in window is open, so the browser is not driven now.",
                "Press Enter in the terminal running `eidolon view`, or stop that command, then try again.",
            ),
            other => CliError::sentence("view", other.to_string()),
        }
    }
}

impl From<eidolon_mcp::Error> for CliError {
    fn from(e: eidolon_mcp::Error) -> CliError {
        CliError::sentence("mcp", e.to_string())
    }
}

impl From<serde_json::Error> for CliError {
    fn from(e: serde_json::Error) -> CliError {
        CliError::failed("bad_json", format!("That is not the JSON this command expects: {e}."), "Check the file and try again.")
    }
}

pub type Result<T> = std::result::Result<T, CliError>;
