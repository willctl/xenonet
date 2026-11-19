use std::net::IpAddr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{invalid, ModelError};
use crate::ids::CellId;
use crate::names::{CellName, NetworkName, Wall};

/// Which driver runs a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverKind {
    /// A Lima VM.
    Lima,
    /// A podman container.
    Podman,
    /// A docker container.
    Docker,
}

/// The CPU an image is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Arch {
    /// 64-bit ARM.
    Aarch64,
    /// 64-bit x86.
    X86_64,
}

/// An image by where it is and the hash it must have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawImagePin", deny_unknown_fields)]
pub struct ImagePin {
    /// An https URL or an absolute local file path.
    pub location: String,
    /// `sha256:` and 64 lowercase hex characters.
    pub digest: String,
    /// The CPU it is for.
    pub arch: Arch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImagePin {
    location: String,
    digest: String,
    arch: Arch,
}

impl ImagePin {
    /// A pin, checked.
    pub fn new(location: String, digest: String, arch: Arch) -> Result<Self, ModelError> {
        let url = location.strip_prefix("https://").is_some_and(|rest| !rest.is_empty());
        let path = location.starts_with('/') && location.len() > 1;
        if !(url || path) || location.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(invalid("image location", "an https URL or an absolute file path, without spaces"));
        }
        let hex = digest.strip_prefix("sha256:").unwrap_or("");
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(invalid("image digest", "sha256: and 64 lowercase hex characters"));
        }
        Ok(Self { location, digest, arch })
    }
}

impl TryFrom<RawImagePin> for ImagePin {
    type Error = ModelError;

    fn try_from(raw: RawImagePin) -> Result<Self, ModelError> {
        Self::new(raw.location, raw.digest, raw.arch)
    }
}

/// A guest port made reachable on a host address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// The port inside the cell.
    pub guest: u16,
    /// The port on the host.
    pub host: u16,
    /// The host address it is published on.
    pub bind: IpAddr,
}

/// A host folder shown inside a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    /// The folder on the host.
    pub source: PathBuf,
    /// Where it appears in the cell.
    pub target: String,
    /// Whether the cell may write to it.
    pub writable: bool,
}

/// A secret by name; its value is never in a spec.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretRef {
    /// The secret's name in the key store.
    pub name: String,
}

/// What a cell may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resources {
    /// Virtual CPUs.
    pub cpus: u8,
    /// Memory in MiB.
    pub memory_mib: u32,
    /// Disk in GiB.
    pub disk_gib: u32,
}

impl Default for Resources {
    fn default() -> Self {
        Self { cpus: 2, memory_mib: 2048, disk_gib: 10 }
    }
}

/// Everything needed to make a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellSpec {
    /// What the driver calls it.
    pub name: CellName,
    /// Which driver runs it.
    pub driver: DriverKind,
    /// The image it starts from.
    pub image: ImagePin,
    /// Its wall.
    pub wall: Wall,
    /// What it may use.
    pub resources: Resources,
    /// Ports published to the host.
    pub ports: Vec<Port>,
    /// The guest port of the browser's automation socket; never published.
    pub automation_port: Option<u16>,
    /// Host folders shown inside.
    pub mounts: Vec<Mount>,
    /// A sealed cell has nothing from the host.
    pub sealed: bool,
    /// The network it joins.
    pub network: Option<NetworkName>,
    /// The cell it is copied from.
    pub clone_from: Option<CellName>,
    /// Secrets it is given.
    pub secrets: Vec<SecretRef>,
}

/// Where a cell is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    /// Being made.
    Creating,
    /// Made and not running.
    Stopped,
    /// Running.
    Running,
    /// Being removed.
    Removing,
    /// Not in a state Adytum can use; the doctor says why.
    Broken,
}

/// A cell that exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    /// Its id.
    pub id: CellId,
    /// What it was made from.
    pub spec: CellSpec,
    /// Where it is now.
    pub state: CellState,
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../tests/golden/cell-spec.json");

    #[test]
    fn golden_spec_round_trips_byte_for_byte() {
        let spec: CellSpec = serde_json::from_str(GOLDEN).unwrap();
        let mut out = serde_json::to_string_pretty(&spec).unwrap();
        out.push('\n');
        assert_eq!(out, GOLDEN);
    }

    #[test]
    fn unknown_fields_are_refused() {
        let with_extra = GOLDEN.replacen("{\n", "{\n  \"extra\": 1,\n", 1);
        assert!(serde_json::from_str::<CellSpec>(&with_extra).is_err());
    }

    #[test]
    fn a_cell_name_outside_ours_is_refused_in_json() {
        let bad = GOLDEN.replacen("adytum-demo", "pane", 1);
        assert!(serde_json::from_str::<CellSpec>(&bad).is_err());
    }

    #[test]
    fn image_pin_checks_location_and_digest() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let ok = |l: &str, d: &str| ImagePin::new(l.into(), d.into(), Arch::X86_64);
        assert!(ok("https://example.test/a.img", &digest).is_ok());
        assert!(ok("/var/images/a.img", &digest).is_ok());
        assert!(ok("http://example.test/a.img", &digest).is_err());
        assert!(ok("a.img", &digest).is_err());
        assert!(ok("https://example.test/a b", &digest).is_err());
        assert!(ok("https://example.test/a.img", "sha256:ab").is_err());
        assert!(ok("https://example.test/a.img", &digest.to_uppercase()).is_err());
        assert!(ok("https://example.test/a.img", &format!("md5:{}", "ab".repeat(32))).is_err());
    }

    #[test]
    fn resources_default() {
        assert_eq!(Resources::default(), Resources { cpus: 2, memory_mib: 2048, disk_gib: 10 });
    }

    #[test]
    fn states_and_drivers_are_snake_case() {
        assert_eq!(serde_json::to_string(&CellState::Removing).unwrap(), "\"removing\"");
        assert_eq!(serde_json::to_string(&DriverKind::Podman).unwrap(), "\"podman\"");
        assert_eq!(serde_json::to_string(&Arch::X86_64).unwrap(), "\"x86_64\"");
    }
}
