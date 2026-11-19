//! A host folder into sealed chunks and a [`FacetManifest`].

use crate::chunker::chunk_reader;
use crate::error::{PackError, Result};
use crate::glob;
use crate::manifest::{Entry, FacetManifest, Node};
use eidolon_model::{FacetId, FacetKind};
use eidolon_vault::{ChunkName, Keys, Vault};
use std::fs;
use std::path::Path;

/// What was left out because it is not a file, folder or symlink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub why: &'static str,
}

/// The result of packing a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub manifest: FacetManifest,
    /// Sockets, pipes, devices and names that are not UTF-8.
    pub skipped: Vec<Skipped>,
}

/// Seals `plaintext` and keeps it in the vault unless it is already there.
pub(crate) fn store_chunk(plaintext: &[u8], keys: &Keys, vault: &Vault) -> Result<ChunkName> {
    let name = keys.name_of(plaintext);
    if !vault.has(&name) {
        let (name, sealed) = keys.seal(plaintext);
        vault.put(&name, &sealed)?;
    }
    Ok(name)
}

/// Packs `dir` as the facet `facet`. Symlinks are recorded, never followed.
/// Paths matching `leave_out` (see [`glob`](crate::glob)) are not carried.
pub fn pack_folder(facet: FacetId, kind: FacetKind, dir: &Path, leave_out: &[String], keys: &Keys, vault: &Vault) -> Result<Packed> {
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    walk(dir, "", leave_out, keys, vault, &mut entries, &mut skipped)?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(Packed { manifest: FacetManifest { facet, kind, entries }, skipped })
}

fn walk(dir: &Path, prefix: &str, leave_out: &[String], keys: &Keys, vault: &Vault, entries: &mut Vec<Entry>, skipped: &mut Vec<Skipped>) -> Result<()> {
    let mut children = Vec::new();
    for child in fs::read_dir(dir).map_err(PackError::io("read a folder"))? {
        children.push(child.map_err(PackError::io("read a folder"))?);
    }
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let Some(name) = child.file_name().to_str().map(str::to_owned) else {
            skipped.push(Skipped { path: format!("{prefix}{}", child.file_name().to_string_lossy()), why: "its name is not UTF-8" });
            continue;
        };
        let rel = format!("{prefix}{name}");
        if leave_out.iter().any(|p| glob::matches(p, &rel)) {
            continue;
        }
        let path = child.path();
        let meta = fs::symlink_metadata(&path).map_err(PackError::io("look at a file"))?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path).map_err(PackError::io("read a symlink"))?;
            match target.to_str() {
                Some(t) => entries.push(Entry { path: rel, node: Node::Symlink { target: t.to_owned() } }),
                None => skipped.push(Skipped { path: rel, why: "its link target is not UTF-8" }),
            }
        } else if kind.is_dir() {
            entries.push(Entry { path: rel.clone(), node: Node::Dir { mode: mode_of(&meta, true) } });
            walk(&path, &format!("{rel}/"), leave_out, keys, vault, entries, skipped)?;
        } else if kind.is_file() {
            let file = fs::File::open(&path).map_err(PackError::io("open a file"))?;
            let mut chunks = Vec::new();
            let size = chunk_reader(file, |piece| {
                chunks.push(store_chunk(piece, keys, vault)?);
                Ok(())
            })?;
            entries.push(Entry { path: rel, node: Node::File { mode: mode_of(&meta, false), size, chunks } });
        } else {
            skipped.push(Skipped { path: rel, why: "it is a socket, pipe or device" });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata, _dir: bool) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata, dir: bool) -> u32 {
    if dir {
        0o755
    } else {
        0o644
    }
}
