//! The trait a cell driver implements (`adytum-lima` for Lima VMs, later
//! `adytum-oci` for containers) and the errors it returns.
//!
//! A driver touches only cells Adytum named. Names arrive as [`CellName`],
//! which cannot hold anything outside `adytum-*`; [`cell_name`] is the one
//! place text becomes a name, so a refusal reads the same everywhere.

mod error;

pub use error::DriverError;

use std::path::Path;
use std::time::Duration;

use adytum_exec::{Duplex, Output};
use adytum_model::{CellName, CellSpec, CellState};
use async_trait::async_trait;

/// Turns text into a cell name, or refuses it as `name_not_ours` before any
/// tool has run. Use this on every name that came from a person or a file.
pub fn cell_name(text: &str) -> Result<CellName, DriverError> {
    adytum_policy::name_is_ours(text)?;
    text.parse::<CellName>().map_err(|_| DriverError::NotFound { name: text.to_owned() })
}

/// How a command runs in a cell.
#[derive(Debug, Clone, Default)]
pub struct ExecOpts {
    /// Variables set for the command, as `(name, value)`. Values reach the
    /// guest as arguments of `env`, so keep secrets out of them.
    pub env: Vec<(String, String)>,
    /// Bytes written to the command's stdin, which is then closed.
    pub stdin: Option<Vec<u8>>,
    /// The longest the command may run.
    pub deadline: Option<Duration>,
}

/// A cell a driver reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Its name.
    pub name: CellName,
    /// Where it is now.
    pub state: CellState,
}

/// What every driver can do. Each method takes a cell by name and refuses a
/// name that is not Adytum's before it starts any tool.
#[async_trait]
pub trait Driver: Send + Sync {
    /// Makes the cell from the spec. It is stopped afterwards.
    async fn create(&self, spec: &CellSpec) -> Result<(), DriverError>;

    /// Starts the cell and waits until commands can run in it.
    async fn start(&self, name: &CellName) -> Result<(), DriverError>;

    /// Runs a command in the cell. `argv` reaches the guest as separate
    /// arguments, never as a shell string. A command that ran and failed is
    /// an `Output` with its status, not an error.
    async fn exec(&self, name: &CellName, argv: &[String], opts: ExecOpts) -> Result<Output, DriverError>;

    /// A command's stdin and stdout as one byte stream.
    async fn stream(&self, name: &CellName, argv: &[String]) -> Result<Duplex, DriverError>;

    /// Copies a file or folder from the host into the cell. `guest` is an
    /// absolute path without `..`.
    async fn copy_in(&self, name: &CellName, host: &Path, guest: &str) -> Result<(), DriverError>;

    /// Copies a file or folder out of the cell to the host.
    async fn copy_out(&self, name: &CellName, guest: &str, host: &Path) -> Result<(), DriverError>;

    /// Stops the cell.
    async fn stop(&self, name: &CellName) -> Result<(), DriverError>;

    /// Removes the cell and everything in it.
    async fn remove(&self, name: &CellName) -> Result<(), DriverError>;

    /// The cells this driver holds.
    async fn list(&self) -> Result<Vec<Listed>, DriverError>;

    /// Makes `to` as a copy of the stopped cell `from`, with the settings
    /// the spec lists (`to.clone_from` names the source). Stopped afterwards.
    /// A driver that cannot copy a cell refuses with `Unsupported`.
    async fn clone(&self, from: &CellName, to: &CellSpec) -> Result<(), DriverError> {
        let _ = (from, to);
        Err(DriverError::Unsupported { what: "cloning a cell".to_owned() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use adytum_model::RefusalCode;

    #[test]
    fn a_name_outside_ours_is_refused_as_name_not_ours() {
        for bad in ["pane", "xenonet-vm1", "xenonet-base-ubuntu", "default", "adytum-", "Adytum-a"] {
            match cell_name(bad) {
                Err(DriverError::Refused(r)) => assert_eq!(r.code, RefusalCode::NameNotOurs, "{bad}"),
                other => panic!("{bad}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_name_of_ours_passes() {
        assert_eq!(cell_name("adytum-test-1").map(|n| n.to_string()).ok().as_deref(), Some("adytum-test-1"));
    }
}
