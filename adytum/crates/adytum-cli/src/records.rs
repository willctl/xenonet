//! What the CLI remembers about a cell beside Lima: its wall and what the
//! policy needs to compare. One small file per cell under
//! `<ADYTUM_HOME>/cells/`. A ledger replaces this in a later slice.

use std::path::{Path, PathBuf};

use adytum_model::{CellName, NetworkName, Wall};
use adytum_policy::CellFacts;
use serde::{Deserialize, Serialize};

/// Stands in for the wall of a cell with no record, so the policy treats it
/// as a different wall from any a person names deliberately.
const UNRECORDED_WALL: &str = "wall_unrecorded";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub name: CellName,
    pub wall: Wall,
    pub sealed: bool,
    pub network: Option<NetworkName>,
    pub mount_sources: Vec<PathBuf>,
    pub secrets: Vec<String>,
}

impl Record {
    pub fn facts(&self) -> CellFacts {
        CellFacts {
            name: self.name.clone(),
            wall: self.wall.clone(),
            network: self.network.clone(),
            mount_sources: self.mount_sources.clone(),
            secrets: self.secrets.clone(),
            holds_open_eidolon: false,
        }
    }
}

/// Facts for a cell that exists but has no readable record.
pub fn unrecorded(name: &CellName) -> CellFacts {
    CellFacts {
        name: name.clone(),
        wall: UNRECORDED_WALL.parse().expect("the constant is a valid wall; a test checks it"),
        network: None,
        mount_sources: Vec::new(),
        secrets: Vec::new(),
        holds_open_eidolon: false,
    }
}

fn file(home: &Path, name: &CellName) -> PathBuf {
    home.join("cells").join(format!("{name}.json"))
}

/// The record for a cell, if there is a readable one.
pub fn read(home: &Path, name: &CellName) -> Option<Record> {
    let text = std::fs::read_to_string(file(home, name)).ok()?;
    serde_json::from_str(&text).ok().filter(|r: &Record| &r.name == name)
}

pub fn write(home: &Path, record: &Record) -> std::io::Result<()> {
    let path = file(home, &record.name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(record).map_err(std::io::Error::other)?;
    text.push('\n');
    std::fs::write(path, text)
}

/// Forgets a cell. A cell with no record is already forgotten.
pub fn remove(home: &Path, name: &CellName) -> std::io::Result<()> {
    match std::fs::remove_file(file(home, name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stand_in_wall_is_valid() {
        let name: CellName = "adytum-a".parse().unwrap();
        assert_eq!(unrecorded(&name).wall.as_str(), UNRECORDED_WALL);
    }
}
