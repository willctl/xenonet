//! The filesystem as XenoNet's blocks see it: the few calls they make, with
//! errors named as Node names them (`ENOENT`, `EACCES`), behind a trait, so
//! a test (or a twin given a fake `fs`) can stand in for the disk.

use std::io;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// An error as Node reports one: its code when the system gave one, and
/// its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
    pub code: Option<String>,
    pub message: String,
}

impl FsError {
    pub fn is(&self, code: &str) -> bool {
        self.code.as_deref() == Some(code)
    }

    /// The code, or the message when there's none, as the blocks quote it.
    pub fn code_or_message(&self) -> &str {
        self.code.as_deref().unwrap_or(&self.message)
    }
}

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> Self {
        FsError { code: code_of(&e).map(Into::into), message: e.to_string() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
}

pub trait Fs {
    /// A path's mtime in ms, following links, as Stats.mtimeMs gives it.
    fn mtime_ms(&mut self, path: &str) -> Result<f64, FsError>;
    /// A folder's entries, in Node's order (see read_dir_sorted).
    fn read_dir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError>;
}

/// The disk.
pub struct StdFs;

/// Stats.mtimeMs: seconds and nanoseconds, as one double.
pub fn mtime_ms_of(m: &std::fs::Metadata) -> io::Result<f64> {
    let t = m.modified()?;
    Ok(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6)
        }
    })
}

impl Fs for StdFs {
    fn mtime_ms(&mut self, path: &str) -> Result<f64, FsError> {
        Ok(mtime_ms_of(&std::fs::metadata(path)?)?)
    }

    fn read_dir(&mut self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let ents = crate::paths::read_dir_sorted(Path::new(path))?;
        Ok(ents
            .into_iter()
            .map(|e| {
                let t = e.file_type().ok();
                DirEntry { name: e.file_name().to_string_lossy().into_owned(), is_dir: t.is_some_and(|t| t.is_dir()), is_file: t.is_some_and(|t| t.is_file()) }
            })
            .collect())
    }
}

/// The code Node gives an error: the errno's name on unix, libuv's name for
/// a Windows error, or None.
pub fn code_of(e: &io::Error) -> Option<&'static str> {
    let n = e.raw_os_error()?;
    #[cfg(target_os = "macos")]
    let table: &[(i32, &str)] = &[
        (1, "EPERM"),
        (2, "ENOENT"),
        (4, "EINTR"),
        (5, "EIO"),
        (9, "EBADF"),
        (12, "ENOMEM"),
        (13, "EACCES"),
        (16, "EBUSY"),
        (17, "EEXIST"),
        (18, "EXDEV"),
        (20, "ENOTDIR"),
        (21, "EISDIR"),
        (22, "EINVAL"),
        (23, "ENFILE"),
        (24, "EMFILE"),
        (28, "ENOSPC"),
        (30, "EROFS"),
        (35, "EAGAIN"),
        (62, "ELOOP"),
        (63, "ENAMETOOLONG"),
        (66, "ENOTEMPTY"),
        (60, "ETIMEDOUT"),
        (61, "ECONNREFUSED"),
    ];
    #[cfg(all(unix, not(target_os = "macos")))]
    let table: &[(i32, &str)] = &[
        (1, "EPERM"),
        (2, "ENOENT"),
        (4, "EINTR"),
        (5, "EIO"),
        (9, "EBADF"),
        (11, "EAGAIN"),
        (12, "ENOMEM"),
        (13, "EACCES"),
        (16, "EBUSY"),
        (17, "EEXIST"),
        (18, "EXDEV"),
        (20, "ENOTDIR"),
        (21, "EISDIR"),
        (22, "EINVAL"),
        (23, "ENFILE"),
        (24, "EMFILE"),
        (28, "ENOSPC"),
        (30, "EROFS"),
        (36, "ENAMETOOLONG"),
        (39, "ENOTEMPTY"),
        (40, "ELOOP"),
        (110, "ETIMEDOUT"),
        (111, "ECONNREFUSED"),
    ];
    // Windows errors as libuv names them (uv_translate_sys_error).
    #[cfg(windows)]
    let table: &[(i32, &str)] = &[
        (2, "ENOENT"),
        (3, "ENOENT"),
        (5, "EPERM"),
        (6, "EBADF"),
        (8, "ENOMEM"),
        (32, "EBUSY"),
        (33, "EBUSY"),
        (80, "EEXIST"),
        (87, "EINVAL"),
        (112, "ENOSPC"),
        (123, "ENOENT"),
        (145, "ENOTEMPTY"),
        (161, "ENOENT"),
        (183, "EEXIST"),
        (206, "ENAMETOOLONG"),
        (267, "ENOTDIR"),
        (1921, "ELOOP"),
    ];
    table.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
}
