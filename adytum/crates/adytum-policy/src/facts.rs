use std::path::PathBuf;

use adytum_model::{CellName, NetworkName, Wall};

/// What Adytum knows about one cell that exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellFacts {
    /// Its name.
    pub name: CellName,
    /// Its wall.
    pub wall: Wall,
    /// The network it joined.
    pub network: Option<NetworkName>,
    /// The host folders mounted into it.
    pub mount_sources: Vec<PathBuf>,
    /// The names of the secrets it was given.
    pub secrets: Vec<String>,
    /// Whether an open eidolon is laid out in it.
    pub holds_open_eidolon: bool,
}

/// The cells that exist now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct World {
    /// One entry per cell.
    pub cells: Vec<CellFacts>,
}

impl World {
    pub(crate) fn find(&self, name: &CellName) -> Option<&CellFacts> {
        self.cells.iter().find(|c| &c.name == name)
    }
}
