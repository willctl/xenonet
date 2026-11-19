//! Sealing and opening all of an eidolon's facets.

use crate::cell::CellAccess;
use crate::error::{FacetError, Result};
use crate::facet::Facet;
use eidolon_model::EidolonId;
use eidolon_pack::{load, pack_folder, seal, unpack_facet, FacetManifest, Manifest, SignedHead, FORMAT};
use eidolon_vault::{Keys, Vault};
use std::path::{Path, PathBuf};
use xenonet_id::codec::{hex, random};
use xenonet_id::sign::Signer;

/// A fresh, empty folder under `work_dir` for one facet's files.
fn fresh_dir(work_dir: &Path, what: &str, name: &str) -> Result<PathBuf> {
    let tag = hex(&random::<6>().map_err(|_| FacetError::Random)?);
    let dir = work_dir.join(format!("{what}-{name}-{tag}"));
    std::fs::create_dir_all(&dir).map_err(FacetError::io("make a staging folder"))?;
    Ok(dir)
}

/// Takes one facet out of the cell and packs it. The facet is resumed
/// whatever happens.
async fn seal_one(facet: &dyn Facet, cell: &dyn CellAccess, keys: &Keys, vault: &Vault, work_dir: &Path) -> Result<FacetManifest> {
    let leave_out = facet.leave_out();
    for pattern in &leave_out {
        eidolon_policy::leave_out_is_inside(pattern)?;
    }
    let staging = fresh_dir(work_dir, "take", &facet.spec().name)?;
    let quieted = facet.quiet(cell).await;
    let taken = match quieted {
        Ok(()) => facet.take(cell, &staging).await,
        Err(e) => Err(e),
    };
    let resumed = facet.resume(cell).await;
    let packed = taken.and_then(|()| Ok(pack_folder(facet.spec().id, facet.kind(), &staging, &leave_out, keys, vault)?));
    // Our own staging folder; the chunks are in the vault by now.
    let _ = std::fs::remove_dir_all(&staging);
    let packed = packed?;
    resumed?;
    Ok(packed.manifest)
}

/// For each facet in turn: quiet it, take its files, pack them into the
/// vault, resume it. Then seals one manifest of all facets and signs its
/// head.
#[expect(clippy::too_many_arguments, reason = "each argument is a distinct thing a seal needs; a struct would only rename them")]
pub async fn seal_facets(
    facets: &[Box<dyn Facet>],
    cell: &dyn CellAccess,
    keys: &Keys,
    vault: &Vault,
    signer: &dyn Signer,
    eidolon: EidolonId,
    generation: u64,
    work_dir: &Path,
) -> Result<SignedHead> {
    let mut parts = Vec::new();
    for facet in facets {
        parts.push(seal_one(facet.as_ref(), cell, keys, vault, work_dir).await?);
    }
    let manifest = Manifest { format: FORMAT, eidolon, generation, facets: parts };
    Ok(seal(&manifest, keys, vault, signer)?)
}

/// Loads the manifest `head` names, unpacks each facet into a staging
/// folder under `work_dir` and places it in the cell. This does not check
/// the head's signature: the caller verifies it against the seat's key
/// first.
pub async fn open_facets(head: &SignedHead, facets: &[Box<dyn Facet>], cell: &dyn CellAccess, keys: &Keys, vault: &Vault, work_dir: &Path) -> Result<()> {
    let manifest = load(head, keys, vault)?;
    // Find and unpack everything before the cell sees any of it.
    let mut staged = Vec::new();
    let mut result = Ok(());
    for facet in facets {
        let spec = facet.spec();
        let Some(part) = manifest.facets.iter().find(|f| f.facet == spec.id) else {
            result = Err(FacetError::MissingFacet(spec.id));
            break;
        };
        if part.kind != facet.kind() {
            result = Err(FacetError::WrongKind { expected: facet.kind() });
            break;
        }
        let parent = fresh_dir(work_dir, "place", &spec.name);
        // `unpack_facet` wants a folder that does not exist yet.
        let target = parent.map(|p| p.join("files"));
        match target {
            Ok(into) => {
                if let Err(e) = unpack_facet(part, keys, vault, &into) {
                    result = Err(e.into());
                    break;
                }
                staged.push((facet, into));
            }
            Err(e) => {
                result = Err(e);
                break;
            }
        }
    }
    if result.is_ok() {
        for (facet, into) in &staged {
            if let Err(e) = facet.place(cell, into).await {
                result = Err(e);
                break;
            }
        }
    }
    for (_, into) in &staged {
        if let Some(parent) = into.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }
    result
}
