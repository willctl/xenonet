//! The manifest, the head that signs it, and how both are stored.
//!
//! The manifest lists file names, so it is private: it is serialized, split
//! into pieces of at most [`MANIFEST_PIECE`] bytes and sealed as chunks like
//! any file. What stays in clear is the [`Head`]: the eidolon, the
//! generation and the names of the manifest's chunks. It is signed.
//!
//! Canonical bytes of a head (what the signature covers): JSON with no
//! whitespace and keys in this order, `format`, `eidolon`, `generation`,
//! `manifest`; the eidolon as its id text, the generation as a decimal
//! number, the manifest as an array of lowercase hex chunk names. Example:
//! `{"format":1,"eidolon":"eid_...","generation":3,"manifest":["ab..."]}`.
//! The signature is over those bytes under `Purpose::EIDOLON_MANIFEST` and
//! is written as base64url.

use crate::error::{PackError, Result};
use eidolon_model::{EidolonId, FacetId, FacetKind, HolderKey};
use eidolon_vault::{ChunkName, Keys, Vault};
use serde::{Deserialize, Serialize};
use xenonet_id::codec::{b64, unb64};
use xenonet_id::sign::{Purpose, Signer};

/// The only manifest and head format there is.
pub const FORMAT: u32 = 1;

/// The most bytes of serialized manifest in one chunk.
pub const MANIFEST_PIECE: usize = 256 * 1024;

/// What one path in a facet is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    Dir { mode: u32 },
    File { mode: u32, size: u64, chunks: Vec<ChunkName> },
    Symlink { target: String },
}

/// One path (relative, `/`-separated) and what it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub node: Node,
}

/// All of one facet's files, sorted by path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetManifest {
    pub facet: FacetId,
    pub kind: FacetKind,
    pub entries: Vec<Entry>,
}

/// Everything an eidolon carries at one generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub eidolon: EidolonId,
    pub generation: u64,
    pub facets: Vec<FacetManifest>,
}

/// The clear, signed part: where the sealed manifest is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Head {
    pub format: u32,
    pub eidolon: EidolonId,
    pub generation: u64,
    pub manifest: Vec<ChunkName>,
}

impl Head {
    /// The bytes the signature covers (see the module docs).
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let names: Vec<String> = self.manifest.iter().map(|n| format!("\"{}\"", n.to_hex())).collect();
        format!("{{\"format\":{},\"eidolon\":\"{}\",\"generation\":{},\"manifest\":[{}]}}", self.format, self.eidolon, self.generation, names.join(",")).into_bytes()
    }
}

/// A head and its signature (base64url), made by the machine that holds
/// the seat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedHead {
    pub head: Head,
    pub signature: String,
}

impl SignedHead {
    /// Whether `key` signed exactly this head.
    pub fn verify(&self, key: &HolderKey) -> bool {
        unb64(&self.signature).is_some_and(|sig| key.verifying_key().verify(Purpose::EIDOLON_MANIFEST, &self.head.canonical_bytes(), &sig))
    }
}

/// Seals `manifest` into the vault and signs a head that names its chunks.
pub fn seal(manifest: &Manifest, keys: &Keys, vault: &Vault, signer: &dyn Signer) -> Result<SignedHead> {
    let bytes = serde_json::to_vec(manifest).map_err(|e| PackError::BadManifest(e.to_string()))?;
    let mut names = Vec::new();
    for piece in bytes.chunks(MANIFEST_PIECE) {
        names.push(crate::pack::store_chunk(piece, keys, vault)?);
    }
    let head = Head { format: FORMAT, eidolon: manifest.eidolon, generation: manifest.generation, manifest: names };
    let sig = signer.sign(Purpose::EIDOLON_MANIFEST, &head.canonical_bytes()).map_err(|e| PackError::Sign(e.message))?;
    Ok(SignedHead { head, signature: b64(&sig) })
}

/// Reads the manifest a head names. This does not check the signature:
/// whoever trusts the head calls [`SignedHead::verify`] with the seat's key
/// first. A changed chunk fails here; a manifest that disagrees with its
/// head is refused.
pub fn load(head: &SignedHead, keys: &Keys, vault: &Vault) -> Result<Manifest> {
    if head.head.format != FORMAT {
        return Err(PackError::BadManifest(format!("format {} is not known", head.head.format)));
    }
    let mut bytes = Vec::new();
    for name in &head.head.manifest {
        bytes.extend_from_slice(&keys.open(name, &vault.get(name)?)?);
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| PackError::BadManifest(e.to_string()))?;
    if manifest.format != FORMAT || manifest.eidolon != head.head.eidolon || manifest.generation != head.head.generation {
        return Err(PackError::BadManifest("it does not match its head".to_owned()));
    }
    Ok(manifest)
}
