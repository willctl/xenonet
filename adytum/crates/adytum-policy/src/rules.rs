use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};

use adytum_model::{CellName, CellSpec, Refusal, RefusalCode};

use crate::facts::{CellFacts, World};

/// Every rule, in a fixed order; the first refusal wins.
pub fn check_create(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    ports_on_localhost(spec)?;
    automation_port_kept_inside(spec)?;
    sealed_has_no_mounts(spec)?;
    name_is_ours(spec.name.as_str())?;
    name_free(spec, world)?;
    walls_share_no_network(spec, world)?;
    walls_share_no_mount(spec, world)?;
    walls_share_no_clone_source(spec, world)?;
    walls_share_no_secret(spec, world)?;
    no_clone_of_open_eidolon(spec, world)
}

fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// A port is published on a loopback address only (127.0.0.0/8 or ::1).
pub fn ports_on_localhost(spec: &CellSpec) -> Result<(), Refusal> {
    match spec.ports.iter().find(|p| !is_loopback(p.bind)) {
        None => Ok(()),
        Some(p) => Err(Refusal::new(
            RefusalCode::PortNotLocalhost,
            format!("Port {} would be published on {}, which other machines can reach.", p.guest, p.bind),
            "Publish it on 127.0.0.1.",
        )),
    }
}

/// The automation port is never published, not even on localhost.
pub fn automation_port_kept_inside(spec: &CellSpec) -> Result<(), Refusal> {
    match spec.automation_port {
        Some(auto) if spec.ports.iter().any(|p| p.guest == auto) => Err(Refusal::new(
            RefusalCode::AutomationPortPublished,
            format!("The automation port {auto} would be published to the host."),
            "Remove it from the ports; the host reaches it through the exec channel.",
        )),
        _ => Ok(()),
    }
}

/// A sealed cell has nothing mounted from the host.
pub fn sealed_has_no_mounts(spec: &CellSpec) -> Result<(), Refusal> {
    if spec.sealed && !spec.mounts.is_empty() {
        return Err(Refusal::new(RefusalCode::SealedCellMount, "A sealed cell cannot have mounts from the host.", "Remove the mounts, or make the cell not sealed."));
    }
    Ok(())
}

/// Adytum touches only cells it named. Drivers call this on any name before
/// they touch an instance.
pub fn name_is_ours(name: &str) -> Result<(), Refusal> {
    name.parse::<CellName>().map(|_| ()).map_err(|_| {
        Refusal::new(RefusalCode::NameNotOurs, format!("{name:?} is not a cell Adytum made."), "Adytum only touches cells named adytum-<lowercase letters, digits, dashes>.")
    })
}

/// No existing cell has this name.
pub fn name_free(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    if world.find(&spec.name).is_some() {
        return Err(Refusal::new(RefusalCode::NameTaken, format!("A cell named {} already exists.", spec.name), "Pick another name, or remove that cell first."));
    }
    Ok(())
}

fn others<'a>(spec: &'a CellSpec, world: &'a World) -> impl Iterator<Item = &'a CellFacts> {
    world.cells.iter().filter(move |c| c.wall != spec.wall)
}

/// Different walls never share a network.
pub fn walls_share_no_network(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    let Some(net) = &spec.network else { return Ok(()) };
    match others(spec, world).find(|c| c.network.as_ref() == Some(net)) {
        None => Ok(()),
        Some(c) => Err(Refusal::new(
            RefusalCode::WallNetwork,
            format!("Network {net} is already used by {}, which is behind another wall.", c.name),
            "Use a network of its own for this wall.",
        )),
    }
}

/// Resolves `.` and `..` by looking at the path alone, never the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(part);
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn overlaps(a: &Path, b: &Path) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    a.starts_with(&b) || b.starts_with(&a)
}

/// Different walls never share a mount source; a folder and anything inside
/// it count as shared.
pub fn walls_share_no_mount(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    for m in &spec.mounts {
        for c in others(spec, world) {
            if let Some(theirs) = c.mount_sources.iter().find(|s| overlaps(&m.source, s)) {
                return Err(Refusal::new(
                    RefusalCode::WallMount,
                    format!("{} overlaps {}, which {} behind another wall already mounts.", m.source.display(), theirs.display(), c.name),
                    "Mount a folder that no other wall uses.",
                ));
            }
        }
    }
    Ok(())
}

/// A cell is never cloned from a cell behind another wall.
pub fn walls_share_no_clone_source(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    let Some(src) = &spec.clone_from else { return Ok(()) };
    match others(spec, world).find(|c| &c.name == src) {
        None => Ok(()),
        Some(c) => Err(Refusal::new(
            RefusalCode::WallClone,
            format!("{} is behind another wall and cannot be cloned.", c.name),
            "Clone a cell behind the same wall, or start from the image.",
        )),
    }
}

/// Different walls never share a secret.
pub fn walls_share_no_secret(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    for s in &spec.secrets {
        if let Some(c) = others(spec, world).find(|c| c.secrets.contains(&s.name)) {
            return Err(Refusal::new(
                RefusalCode::WallSecret,
                format!("Secret {} is already given to {}, which is behind another wall.", s.name, c.name),
                "Give this wall a secret of its own.",
            ));
        }
    }
    Ok(())
}

/// A cell that holds an open eidolon is not cloned.
pub fn no_clone_of_open_eidolon(spec: &CellSpec, world: &World) -> Result<(), Refusal> {
    match spec.clone_from.as_ref().and_then(|n| world.find(n)) {
        Some(c) if c.holds_open_eidolon => Err(Refusal::new(
            RefusalCode::CloneOpenEidolon,
            format!("{} holds an open eidolon; a clone would be a second live copy.", c.name),
            "Seal the eidolon first, or clone another cell.",
        )),
        _ => Ok(()),
    }
}
