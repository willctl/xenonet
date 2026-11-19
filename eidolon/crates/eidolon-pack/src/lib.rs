//! A facet's files turned into chunks and a signed manifest, and back.
//!
//! - [`chunker`]: content-defined chunking (16 KiB minimum, 64 KiB average,
//!   256 KiB maximum), so an insert changes only the chunks around it.
//! - [`glob`]: the leave-out pattern matcher.
//! - [`manifest`]: [`Manifest`], the signed [`Head`] and how they are sealed
//!   and loaded.
//! - [`pack_folder`]: a host folder into sealed chunks in the vault.
//! - [`unpack_facet`]: a facet back into a folder, never writing outside it.
//! - [`diff`]: which chunks two manifests share.

pub mod chunker;
mod diff;
mod error;
pub mod glob;
pub mod manifest;
mod pack;
mod unpack;

pub use diff::{diff, ChunkDiff};
pub use error::{PackError, Result};
pub use manifest::{load, seal, Entry, FacetManifest, Head, Manifest, Node, SignedHead, FORMAT, MANIFEST_PIECE};
pub use pack::{pack_folder, Packed, Skipped};
pub use unpack::unpack_facet;
