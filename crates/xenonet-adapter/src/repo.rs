//! What the core knows of a checkout, as the adapter of its version control
//! reads it (git's: adapters/git): its top, branch, origin and repo key,
//! what it belongs to (a worktree's main checkout, a submodule's
//! superproject), and what to watch for its head and refs to move.
//!
//! The core may not depend on an adapter crate, so the binary installs the
//! reader it was built with (`install`), as it does the platforms, and the
//! core asks `checkouts()`. Until then no folder is a checkout.

use crate::watch::WatchSpec;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// A checkout as the core sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checkout {
    /// The working tree's top.
    pub top: PathBuf,
    /// Its version control's marker folder (`.git`).
    pub vcs: &'static str,
    /// The branch it's on, or a detached head's first 12 characters.
    pub branch: Option<String>,
    /// origin's URL, as its config writes it (url, never pushurl).
    pub origin: Option<String>,
    /// origin's URL normalized (`xenonet_model::ids::normalize_remote`).
    pub key: Option<String>,
    /// A linked worktree's main checkout.
    pub worktree_of: Option<PathBuf>,
    /// A submodule's superproject.
    pub submodule_of: Option<PathBuf>,
    /// What to watch for its head, refs and config to move.
    pub watch: Vec<WatchSpec>,
    /// Its config file, as the watcher names it: a change to it may change
    /// its remotes, its identity and what a status runs.
    pub config: PathBuf,
}

/// The reader of checkouts the binary installs.
pub trait Checkouts: Send + Sync {
    /// The checkout whose top is `top`, or None when it isn't one.
    fn open(&self, top: &Path) -> Option<Checkout>;

    /// The checkout a path is in: the nearest top at or above it.
    fn find(&self, path: &Path) -> Option<Checkout>;
}

/// No checkouts: what the core reads until one is installed.
struct Nothing;

impl Checkouts for Nothing {
    fn open(&self, _top: &Path) -> Option<Checkout> {
        None
    }
    fn find(&self, _path: &Path) -> Option<Checkout> {
        None
    }
}

static INSTALLED: OnceLock<Arc<dyn Checkouts>> = OnceLock::new();

/// Puts the reader of checkouts in place, once: the composition root does,
/// with the platforms. A second call changes nothing.
pub fn install(c: Arc<dyn Checkouts>) {
    let _ = INSTALLED.set(c);
}

/// The reader in place: one that finds nothing, until `install`.
pub fn checkouts() -> &'static dyn Checkouts {
    static NOTHING: Nothing = Nothing;
    INSTALLED.get().map_or(&NOTHING as &dyn Checkouts, |c| &**c)
}
