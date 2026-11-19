//! The one host folder files pass through, and the rules for it.
//!
//! An upload names a host file; it must be inside the folder once symlinks
//! are resolved. A download is named by what the browser saved; the name
//! must be a plain file name. Both rules are plain functions so a test can
//! prove each refusal without a cell.

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// The folder under `files` where fetched downloads are put.
pub const DOWNLOADS_DIR: &str = "downloads";

fn refused(text: String) -> Error {
    Error::Refused(text)
}

/// The host file an upload may use: `requested` (absolute, or relative to
/// `files`) resolved through every symlink, which must be a regular file
/// inside `files`.
pub fn upload_source(files: &Path, requested: &str) -> Result<PathBuf> {
    let root = files.canonicalize().map_err(|e| Error::Io(format!("The files folder {} cannot be used: {e}. Create it first.", files.display())))?;
    let asked = Path::new(requested);
    let joined = if asked.is_absolute() { asked.to_path_buf() } else { root.join(asked) };
    let resolved = joined.canonicalize().map_err(|e| Error::Io(format!("The file {requested} cannot be read: {e}.")))?;
    if !resolved.starts_with(&root) {
        return Err(refused(format!("The file {requested} is outside the files folder {}. Put it in that folder first; only files there can be uploaded.", root.display())));
    }
    if !resolved.is_file() {
        return Err(refused(format!("{requested} is not a regular file. Upload one file at a time.")));
    }
    Ok(resolved)
}

/// The name an upload gets in the guest: the file's own name, which cannot
/// hold a `/`.
pub fn upload_name(resolved: &Path) -> Result<String> {
    let name = resolved.file_name().and_then(|n| n.to_str()).ok_or_else(|| refused("The file's name is not plain text. Rename it and try again.".to_owned()))?;
    check_name(name)?;
    Ok(name.to_owned())
}

/// A file name with no path in it.
pub fn check_name(name: &str) -> Result<()> {
    let bad = name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) || Path::new(name).components().any(|c| !matches!(c, Component::Normal(_)));
    if bad {
        return Err(refused(format!("{name:?} is not a plain file name. Give a name from list_downloads, without a folder or `..`.")));
    }
    Ok(())
}

/// The host folder fetched downloads go to, created if needed, and checked
/// to be inside `files` even if someone made it a symlink.
pub fn downloads_dir(files: &Path) -> Result<PathBuf> {
    let root = files.canonicalize().map_err(|e| Error::Io(format!("The files folder {} cannot be used: {e}. Create it first.", files.display())))?;
    let dir = root.join(DOWNLOADS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("The folder {} cannot be made: {e}.", dir.display())))?;
    let resolved = dir.canonicalize().map_err(|e| Error::Io(format!("The folder {} cannot be used: {e}.", dir.display())))?;
    if !resolved.starts_with(&root) {
        return Err(refused(format!("{} leads outside the files folder. Remove it and try again.", dir.display())));
    }
    Ok(resolved)
}

/// A path in `dir` for `name` that does not exist yet: `name`, then
/// `stem-1.ext`, `stem-2.ext`. A fetch never overwrites an earlier file.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if first.symlink_metadata().is_err() {
        return first;
    }
    let (stem, ext) = match name.rfind('.').filter(|&i| i > 0) {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    (1..=u32::MAX)
        .map(|n| dir.join(format!("{stem}-{n}{ext}")))
        .find(|p| p.symlink_metadata().is_err())
        // Four billion names taken: give up on a free one rather than panic.
        .unwrap_or(first)
}
