//! The files the native readers, numbat's included, read, as Node's fs
//! calls give them: a folder's entries, a file's stats, its text, or a
//! range of its bytes. Behind a trait, so a twin given another `files`
//! (a test's) reads through that.

use std::io;
use xenonet_base::fs::{code_of, mtime_ms_of, FsError};

/// A folder's entry, as a Dirent tells it: neither follows a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
    pub is_file: bool,
}

/// What statSync tells of a file (following a link).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    pub mtime_ms: f64,
    pub size: f64,
    /// None where it isn't known (a stand-in fs, or Windows here).
    pub ino: Option<f64>,
}

pub trait Files {
    /// readdirSync(dir, { withFileTypes: true }), in Node's order.
    fn read_dir(&mut self, dir: &str) -> Result<Vec<Entry>, FsError>;
    /// statSync(file).
    fn stat(&mut self, file: &str) -> Result<Stat, FsError>;
    /// readFileSync(file, "utf8").
    fn read_text(&mut self, file: &str) -> Result<String, FsError>;
    /// `len` bytes from `offset`, as openSync, one readSync into a zeroed
    /// buffer of that length, and closeSync give them.
    fn read_at(&mut self, file: &str, offset: f64, len: usize) -> Result<Vec<u8>, FsError>;
    /// existsSync(file).
    fn exists(&mut self, file: &str) -> bool;
}

/// The disk.
pub struct StdFiles;

// An error as Node words a read's: "EISDIR: illegal operation on a
// directory, read" (no path).
fn read_error(e: io::Error) -> FsError {
    let FsError { code, message } = FsError::of(e, "read", "");
    FsError { code, message: message.strip_suffix(" ''").map_or(message.clone(), str::to_string) }
}

impl Files for StdFiles {
    fn read_dir(&mut self, dir: &str) -> Result<Vec<Entry>, FsError> {
        let ents = xenonet_base::paths::read_dir_sorted(std::path::Path::new(dir)).map_err(|e| FsError::of(e, "scandir", dir))?;
        Ok(ents
            .into_iter()
            .map(|e| {
                let t = e.file_type().ok();
                Entry { name: e.file_name().to_string_lossy().into_owned(), is_dir: t.is_some_and(|t| t.is_dir()), is_file: t.is_some_and(|t| t.is_file()) }
            })
            .collect())
    }

    fn stat(&mut self, file: &str) -> Result<Stat, FsError> {
        let m = std::fs::metadata(file).map_err(|e| FsError::of(e, "stat", file))?;
        #[cfg(unix)]
        let ino = {
            use std::os::unix::fs::MetadataExt;
            Some(m.ino() as f64)
        };
        #[cfg(not(unix))]
        let ino = None;
        Ok(Stat { mtime_ms: mtime_ms_of(&m).map_err(|e| FsError::of(e, "stat", file))?, size: m.len() as f64, ino })
    }

    fn read_text(&mut self, file: &str) -> Result<String, FsError> {
        let bytes = std::fs::read(file).map_err(|e| {
            // Node opens, then reads: a folder opens and fails its read.
            if code_of(&e) == Some("EISDIR") {
                read_error(e)
            } else {
                FsError::of(e, "open", file)
            }
        })?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn read_at(&mut self, file: &str, offset: f64, len: usize) -> Result<Vec<u8>, FsError> {
        let f = std::fs::File::open(file).map_err(|e| FsError::of(e, "open", file))?;
        let mut buf = vec![0u8; len];
        let at = if offset.is_finite() && offset > 0.0 { offset as u64 } else { 0 };
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileExt;
            f.read_at(&mut buf, at).map_err(read_error)?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::FileExt;
            f.seek_read(&mut buf, at).map_err(read_error)?;
        }
        Ok(buf)
    }

    fn exists(&mut self, file: &str) -> bool {
        std::fs::metadata(file).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_as_node_does() {
        let dir = std::env::temp_dir().join(format!("xenonet-native-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        let d = dir.to_string_lossy().into_owned();
        let mut f = StdFiles;
        let names: Vec<_> = f.read_dir(&d).unwrap().into_iter().map(|e| (e.name, e.is_dir, e.is_file)).collect();
        assert_eq!(names, [("a.txt".to_string(), false, true), ("b".to_string(), true, false)]);
        let a = format!("{d}/a.txt");
        assert_eq!(f.stat(&a).unwrap().size, 5.0);
        assert_eq!(f.read_text(&a).unwrap(), "hello");
        assert_eq!(f.read_at(&a, 1.0, 6).unwrap(), b"ello\0\0");
        assert!(f.exists(&a) && !f.exists(&format!("{d}/none")));
        let gone = f.read_dir(&format!("{d}/none")).unwrap_err();
        assert!(gone.is("ENOENT"), "{gone:?}");
        assert!(f.read_text(&format!("{d}/b")).unwrap_err().is("EISDIR"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
