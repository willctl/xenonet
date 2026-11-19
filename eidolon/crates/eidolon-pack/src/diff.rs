use crate::manifest::{Manifest, Node};
use eidolon_vault::ChunkName;
use std::collections::{BTreeMap, BTreeSet};

/// How two manifests differ in chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDiff {
    /// In `new` but not in `old`.
    pub added: Vec<ChunkName>,
    /// In `old` but not in `new`.
    pub removed: Vec<ChunkName>,
    /// Bytes of `new` that `old` already has.
    pub shared_bytes: u64,
    /// Bytes of `new` that `old` lacks.
    pub new_bytes: u64,
}

/// Chunk names and approximate sizes of every file chunk in a manifest. A
/// manifest records a file's size and chunk names but not each chunk's
/// length, so a file's size is split evenly over its distinct chunks. That
/// is exact for a one-chunk file and close for the others.
fn sizes(m: &Manifest) -> BTreeMap<ChunkName, u64> {
    let mut out = BTreeMap::new();
    for entry in m.facets.iter().flat_map(|f| &f.entries) {
        if let Node::File { size, chunks, .. } = &entry.node {
            let distinct: BTreeSet<_> = chunks.iter().collect();
            if distinct.is_empty() {
                continue;
            }
            let each = size / distinct.len() as u64;
            for c in distinct {
                out.entry(*c).or_insert(each);
            }
        }
    }
    out
}

/// What moving from `old` to `new` adds and drops. Bytes are estimates for
/// files of several chunks (see above); the names are exact.
pub fn diff(old: &Manifest, new: &Manifest) -> ChunkDiff {
    let (old_sizes, new_sizes) = (sizes(old), sizes(new));
    let added: Vec<ChunkName> = new_sizes.keys().filter(|c| !old_sizes.contains_key(c)).copied().collect();
    let removed: Vec<ChunkName> = old_sizes.keys().filter(|c| !new_sizes.contains_key(c)).copied().collect();
    let new_bytes = added.iter().filter_map(|c| new_sizes.get(c)).sum();
    let shared_bytes = new_sizes.iter().filter(|(c, _)| old_sizes.contains_key(c)).map(|(_, s)| s).sum();
    ChunkDiff { added, removed, shared_bytes, new_bytes }
}
