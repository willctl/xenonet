use crate::error::{FacetError, Result};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

/// What a command in the cell printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// How a facet reaches the files and commands of its cell. The CLI
/// implements this with Adytum's driver; [`LocalCell`] stands in for tests.
#[async_trait]
pub trait CellAccess: Send + Sync {
    /// Copies the contents of the cell folder `guest_path` into the host
    /// folder `host_dir`.
    async fn copy_out(&self, guest_path: &str, host_dir: &Path) -> Result<()>;
    /// Copies the contents of the host folder `host_dir` into the cell
    /// folder `guest_path`.
    async fn copy_in(&self, host_dir: &Path, guest_path: &str) -> Result<()>;
    /// Runs a command in the cell.
    async fn run(&self, argv: &[&str]) -> Result<RunOutput>;
}

/// A cell that is a host folder: the guest root. Copies are plain recursive
/// copies under that root. `run` runs nothing and reports success.
#[derive(Debug, Clone)]
pub struct LocalCell {
    root: PathBuf,
}

impl LocalCell {
    pub fn new(root: impl Into<PathBuf>) -> LocalCell {
        LocalCell { root: root.into() }
    }

    /// Where a guest path lives on the host.
    pub fn host_path(&self, guest_path: &str) -> Result<PathBuf> {
        if !guest_path.starts_with('/') || guest_path.contains('\0') || guest_path.split('/').any(|p| p == "..") {
            return Err(FacetError::BadGuestPath(guest_path.to_owned()));
        }
        Ok(self.root.join(guest_path.trim_start_matches('/')))
    }
}

#[async_trait]
impl CellAccess for LocalCell {
    async fn copy_out(&self, guest_path: &str, host_dir: &Path) -> Result<()> {
        copy_dir(&self.host_path(guest_path)?, host_dir)
    }

    async fn copy_in(&self, host_dir: &Path, guest_path: &str) -> Result<()> {
        copy_dir(host_dir, &self.host_path(guest_path)?)
    }

    async fn run(&self, _argv: &[&str]) -> Result<RunOutput> {
        Ok(RunOutput { status: 0, stdout: String::new(), stderr: String::new() })
    }
}

/// Copies what is inside `from` into `to`, keeping modes and symlinks.
fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).map_err(FacetError::io("make a folder"))?;
    for child in std::fs::read_dir(from).map_err(FacetError::io("read a folder"))? {
        let child = child.map_err(FacetError::io("read a folder"))?;
        let (src, dst) = (child.path(), to.join(child.file_name()));
        let kind = child.file_type().map_err(FacetError::io("look at a file"))?;
        if kind.is_symlink() {
            copy_link(&src, &dst)?;
        } else if kind.is_dir() {
            copy_dir(&src, &dst)?;
            let perms = std::fs::metadata(&src).map_err(FacetError::io("look at a folder"))?.permissions();
            std::fs::set_permissions(&dst, perms).map_err(FacetError::io("set a folder's mode"))?;
        } else if kind.is_file() {
            // Keeps the permission bits.
            std::fs::copy(&src, &dst).map_err(FacetError::io("copy a file"))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_link(src: &Path, dst: &Path) -> Result<()> {
    let target = std::fs::read_link(src).map_err(FacetError::io("read a symlink"))?;
    std::os::unix::fs::symlink(target, dst).map_err(FacetError::io("make a symlink"))
}

#[cfg(not(unix))]
fn copy_link(_src: &Path, _dst: &Path) -> Result<()> {
    Err(FacetError::Cell("symlinks are not supported on this system".to_owned()))
}
