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

impl FsError {
    /// The error as Node words it: "ENOTDIR: not a directory, mkdir '/x'".
    pub fn of(e: io::Error, syscall: &str, path: &str) -> Self {
        let code = code_of(&e);
        let message = match code {
            Some(c) => format!("{c}: {}, {syscall} '{path}'", describe(c).map_or_else(|| e.to_string(), str::to_string)),
            None => e.to_string(),
        };
        FsError { code: code.map(Into::into), message }
    }
}

// libuv's words for the errors the blocks meet.
fn describe(code: &str) -> Option<&'static str> {
    Some(match code {
        "ENOENT" => "no such file or directory",
        "ENOTDIR" => "not a directory",
        "EISDIR" => "illegal operation on a directory",
        "EACCES" => "permission denied",
        "EPERM" => "operation not permitted",
        "EEXIST" => "file already exists",
        "ENOSPC" => "no space left on device",
        "EROFS" => "read-only file system",
        "EBUSY" => "resource busy or locked",
        "EMFILE" => "too many open files",
        "ENOTEMPTY" => "directory not empty",
        "EXDEV" => "cross-device link not permitted",
        _ => return None,
    })
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

/// fs.realpathSync: `p` absolute, with its links followed. On unix that's
/// canonicalize. On Windows it's Node's own walk, which checks each part
/// for a link and follows it, and leaves every other part as written: a
/// short name such as RUNNER~1 isn't spelled out and no `\\?\` goes in
/// front, as canonicalize would do there.
pub fn realpath(p: &str) -> io::Result<String> {
    #[cfg(windows)]
    {
        realpath_windows(p)
    }
    #[cfg(not(windows))]
    {
        std::fs::canonicalize(p).map(|r| r.to_string_lossy().into_owned())
    }
}

/// fs.realpathSync.native: `p` with its links followed and, on Windows, its
/// short names spelled out, without the `\\?\` that canonicalize puts in
/// front there, which Node never shows.
pub fn realpath_native(p: &str) -> io::Result<String> {
    std::fs::canonicalize(p).map(|r| plain(r.to_string_lossy().into_owned()))
}

/// The running program's file, with its links followed (realpath_native):
/// the path a firewall lets a program in by.
pub fn running_exe() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    realpath_native(&exe.to_string_lossy()).ok()
}

// A path without the \\?\ (or \\?\UNC\) that marks a verbatim Windows
// path, as libuv gives one back.
fn plain(p: String) -> String {
    if let Some(unc) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    p.strip_prefix(r"\\?\").map_or_else(|| p.clone(), str::to_string)
}

#[cfg(windows)]
fn realpath_windows(p: &str) -> io::Result<String> {
    use crate::nodepath::Style::Win32;
    // ERROR_CANT_RESOLVE_FILENAME, which Node reports as ELOOP.
    const TOO_MANY_LINKS: i32 = 1921;
    let mut p = Win32.resolve(&[p]);
    for _ in 0..64 {
        let mut pos = win32_root_len(&p);
        std::fs::symlink_metadata(&p[..pos])?;
        let mut link = None;
        while pos < p.len() {
            let end = p[pos..].find('\\').map_or(p.len(), |i| pos + i);
            let next = (end + 1).min(p.len());
            if std::fs::symlink_metadata(&p[..end])?.file_type().is_symlink() {
                link = Some((pos, end, next));
                break;
            }
            pos = next;
        }
        let Some((at, end, next)) = link else { return Ok(p) };
        // Node stats the link first, so one that leads nowhere fails here.
        std::fs::metadata(&p[..end])?;
        // libuv undoes the \\?\ that Windows writes into an absolute link.
        let target = plain(std::fs::read_link(&p[..end])?.to_string_lossy().into_owned());
        let resolved = Win32.resolve(&[&p[..at], &target]);
        p = Win32.resolve(&[&resolved, &p[next..]]);
    }
    Err(io::Error::from_raw_os_error(TOO_MANY_LINKS))
}

/// How much of a resolved Windows path is its root, as Node's splitRoot
/// finds it: a drive or a `\\server\share`, and the separators after it.
#[cfg(windows)]
fn win32_root_len(p: &str) -> usize {
    let b = p.as_bytes();
    let sep = |c: u8| c == b'\\' || c == b'/';
    let mut i = 0;
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        i = 2;
    } else if b.len() >= 2 && sep(b[0]) && sep(b[1]) {
        let server = 2 + b[2..].iter().take_while(|&&c| !sep(c)).count();
        if server > 2 && server < b.len() {
            let share = server + 1 + b[server + 1..].iter().take_while(|&&c| !sep(c)).count();
            if share > server + 1 {
                i = share;
            }
        }
    }
    i + b[i..].iter().take_while(|&&c| sep(c)).count()
}

/// A Windows file's index on its volume, which Node's fs.stat gives as
/// `ino` there: the one thing that tells a file from another put under the
/// same name, since NTFS hands a file created under a name just freed the
/// old file's creation time. std keeps it behind an unstable API.
#[cfg(windows)]
#[expect(unsafe_code, reason = "GetFileInformationByHandle, whose file index std doesn't give")]
pub fn file_index(path: &str) -> Option<u64> {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)]
    #[derive(Default)]
    #[expect(non_snake_case, reason = "the fields keep the names BY_HANDLE_FILE_INFORMATION has in Windows's headers")]
    struct ByHandleFileInformation {
        dwFileAttributes: u32,
        ftCreationTime: [u32; 2],
        ftLastAccessTime: [u32; 2],
        ftLastWriteTime: [u32; 2],
        dwVolumeSerialNumber: u32,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        nNumberOfLinks: u32,
        nFileIndexHigh: u32,
        nFileIndexLow: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: *mut std::ffi::c_void, info: *mut ByHandleFileInformation) -> i32;
    }
    // std opens it sharing read, write and delete, so it stands in no
    // writer's way.
    let f = std::fs::File::open(path).ok()?;
    let mut info = ByHandleFileInformation::default();
    // SAFETY: the handle is the open file's, which outlives the call, and
    // `info` is laid out as BY_HANDLE_FILE_INFORMATION.
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle(), &raw mut info) } != 0;
    ok.then(|| (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
}

#[cfg(test)]
mod tests {
    use super::{plain, realpath, realpath_native};
    use std::path::{Path, PathBuf};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xenonet-realpath-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a")).expect("a folder");
        dir
    }

    fn text(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn realpath_works_out_dots_and_names_a_missing_file() {
        let dir = temp("dots");
        let a = realpath(&text(&dir.join("a"))).expect("a");
        assert_eq!(realpath(&text(&dir.join("a").join("..").join("a"))).expect("a again"), a);
        assert!(a.ends_with('a') && !a.starts_with(r"\\?\"), "{a}");
        assert_eq!(realpath(&text(&dir.join("missing"))).map_err(|e| e.kind()), Err(std::io::ErrorKind::NotFound));
        let native = realpath_native(&text(&dir.join("a").join("..").join("a"))).expect("a, natively");
        assert!(native.ends_with('a') && !native.starts_with(r"\\?\"), "{native}");
        assert_eq!(plain(r"\\?\UNC\host\share".into()), r"\\host\share");
        assert_eq!(plain(r"\\?\C:\x".into()), r"C:\x");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn realpath_follows_a_link() {
        let dir = temp("link");
        std::os::unix::fs::symlink(dir.join("a"), dir.join("l")).expect("a link");
        assert_eq!(realpath(&text(&dir.join("l"))).expect("l"), realpath(&text(&dir.join("a"))).expect("a"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
