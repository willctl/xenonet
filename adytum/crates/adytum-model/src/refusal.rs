use std::fmt;

use serde::{Deserialize, Serialize};

/// Which rule refused. The strings are stable: callers match on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    /// A port is published on an address other than localhost.
    PortNotLocalhost,
    /// The automation port is published.
    AutomationPortPublished,
    /// A sealed cell has a mount.
    SealedCellMount,
    /// The name is not one Adytum made.
    NameNotOurs,
    /// A cell with this name exists.
    NameTaken,
    /// Two walls would share a network.
    WallNetwork,
    /// Two walls would share a mount source.
    WallMount,
    /// A clone would cross a wall.
    WallClone,
    /// Two walls would share a secret.
    WallSecret,
    /// A cell that holds an open eidolon would be cloned.
    CloneOpenEidolon,
}

/// A rule said no: what, how to fix it, and a code to match on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    /// The rule that refused.
    pub code: RefusalCode,
    /// What was refused and why, as a sentence.
    pub message: String,
    /// What to change, as a sentence.
    pub fix: String,
}

impl Refusal {
    /// A refusal with its message and fix.
    pub fn new(code: RefusalCode, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self { code, message: message.into(), fix: fix.into() }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "adytum: {} {}", self.message, self.fix)
    }
}

impl std::error::Error for Refusal {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_as_stable_snake_case() {
        let pairs = [
            (RefusalCode::PortNotLocalhost, "port_not_localhost"),
            (RefusalCode::AutomationPortPublished, "automation_port_published"),
            (RefusalCode::SealedCellMount, "sealed_cell_mount"),
            (RefusalCode::NameNotOurs, "name_not_ours"),
            (RefusalCode::NameTaken, "name_taken"),
            (RefusalCode::WallNetwork, "wall_network"),
            (RefusalCode::WallMount, "wall_mount"),
            (RefusalCode::WallClone, "wall_clone"),
            (RefusalCode::WallSecret, "wall_secret"),
            (RefusalCode::CloneOpenEidolon, "clone_open_eidolon"),
        ];
        for (code, text) in pairs {
            assert_eq!(serde_json::to_string(&code).unwrap(), format!("\"{text}\""));
        }
    }

    #[test]
    fn display_is_prefix_message_fix() {
        let r = Refusal::new(RefusalCode::NameTaken, "That name is taken.", "Pick another.");
        assert_eq!(r.to_string(), "adytum: That name is taken. Pick another.");
    }
}
