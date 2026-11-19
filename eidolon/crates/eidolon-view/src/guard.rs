use crate::error::{Result, ViewError};
use std::fs;
use std::path::{Path, PathBuf};

const MARKER: &str = "view.open";

/// A small file marker saying a sign-in window is open for an eidolon.
///
/// The CLI and the MCP tools check it before they drive the browser. It
/// holds the pid of the process that opened the window, so a marker left by
/// a process that is gone counts as closed.
#[derive(Debug, Clone)]
pub struct ViewGuard {
    path: PathBuf,
}

impl ViewGuard {
    /// A guard that keeps its marker in `state_dir`, the eidolon's state
    /// folder. Nothing is written until a window opens.
    pub fn new(state_dir: impl AsRef<Path>) -> ViewGuard {
        ViewGuard { path: state_dir.as_ref().join(MARKER) }
    }

    /// The marker's path.
    pub fn marker(&self) -> &Path {
        &self.path
    }

    /// Writes the marker with this process's pid.
    pub fn mark_open(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|source| ViewError::Io { what: "make the state folder", source })?;
        }
        fs::write(&self.path, format!("{}\n", std::process::id())).map_err(|source| ViewError::Io { what: "write the sign-in window's marker", source })
    }

    /// Removes the marker. A marker already gone is fine.
    pub fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(ViewError::Io { what: "remove the sign-in window's marker", source }),
        }
    }

    /// Whether a window is open: the marker exists and its process runs.
    /// A stale marker (process gone, or unreadable) counts as closed and is
    /// removed.
    pub fn is_open(&self) -> bool {
        let Ok(text) = fs::read_to_string(&self.path) else { return false };
        let alive = text.trim().parse::<u32>().is_ok_and(|pid| xenonet_os::process::running(pid, true));
        if !alive {
            // Best effort: a marker that cannot be removed is still read as closed.
            let _ = fs::remove_file(&self.path);
        }
        alive
    }

    /// `Err(ViewError::Open)` while a window is open. Automation tools call
    /// this first.
    pub fn refuse_if_open(&self) -> Result<()> {
        if self.is_open() {
            Err(ViewError::Open)
        } else {
            Ok(())
        }
    }
}
