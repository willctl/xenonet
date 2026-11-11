//! Where each file an adapter watches was read up to, so a change reads
//! only what was added: whole lines, from the last line end read to the
//! last one written. A file that shrank or was replaced is read again from
//! its start.

use super::{Read, WatchSpec};
use std::collections::{BTreeSet, HashMap};
use std::io::{Read as _, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The most read of a file in one go. A transcript met for the first time
/// may be long; the rest is read next (`take_more`), so no read holds a
/// whole one.
pub const READ_MAX: u64 = 8 << 20;

#[derive(Clone, Copy, Debug)]
struct Mark {
    at: u64,
    // The file's identity (its inode on unix), so a file replaced by
    // another as long or longer is still read again from its start.
    id: Option<u64>,
}

/// An adapter's specs, and how far it read each file under them.
#[derive(Clone, Debug, Default)]
pub struct Cursor {
    specs: Vec<WatchSpec>,
    files: HashMap<PathBuf, Mark>,
    more: BTreeSet<PathBuf>,
}

/// What a read gave: whole lines from `from` to `to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Appended {
    pub from: u64,
    pub to: u64,
    pub read: Read,
    pub lines: Vec<String>,
}

#[cfg(unix)]
#[expect(clippy::unnecessary_wraps, reason = "None on the systems without an inode")]
fn file_id(m: &std::fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(m.ino())
}

#[cfg(not(unix))]
fn file_id(_: &std::fs::Metadata) -> Option<u64> {
    // Windows's file index needs an open handle; a replaced file shows by
    // its length there, or by the reconcile.
    None
}

impl Cursor {
    /// A cursor over these specs (as the watcher names them:
    /// `WatchSpec::canonical`), with nothing read yet.
    pub fn new(specs: Vec<WatchSpec>) -> Cursor {
        Cursor { specs, ..Cursor::default() }
    }

    pub fn specs(&self) -> &[WatchSpec] {
        &self.specs
    }

    /// The first spec that counts this path, which says what it is.
    pub fn spec_of(&self, path: &Path) -> Option<&WatchSpec> {
        self.specs.iter().find(|s| s.matches(path))
    }

    /// How far a file was read, when it was.
    pub fn at(&self, path: &Path) -> Option<u64> {
        self.files.get(path).map(|m| m.at)
    }

    /// Says a file was read up to `at` already (by the scan at start, or a
    /// reconcile), so its next read starts there.
    pub fn seed(&mut self, path: &Path, at: u64) {
        let id = std::fs::metadata(path).ok().as_ref().and_then(file_id);
        self.files.insert(path.to_path_buf(), Mark { at, id });
    }

    /// Forgets a file: it's gone, or no longer followed. Whether it was
    /// known.
    pub fn forget(&mut self, path: &Path) -> bool {
        self.more.remove(path);
        self.files.remove(path).is_some()
    }

    /// The files a read left more of to read, which their adapter's
    /// `changed` is given again.
    pub fn take_more(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.more).into_iter().collect()
    }

    /// What was added to a file since it was last read, as whole lines:
    /// None when nothing whole was. An error for a file that can't be read
    /// (NotFound for one that's gone); nothing is marked read then.
    pub fn read(&mut self, path: &Path) -> std::io::Result<Option<Appended>> {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            return Ok(None);
        }
        let (len, id) = (meta.len(), file_id(&meta));
        let (from, read) = match self.files.get(path) {
            None => (0, Read::New),
            Some(m) if len < m.at || (m.id.is_some() && id.is_some() && m.id != id) => (0, Read::Rewritten),
            Some(m) if len == m.at => return Ok(None),
            Some(m) => (m.at, Read::Appended),
        };
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(from))?;
        // Up to READ_MAX, and on past it to the end of the line it stops
        // in, so a line longer than that is still read whole.
        let mut buf = Vec::new();
        let mut want = (len - from).min(READ_MAX);
        loop {
            let start = buf.len();
            (&mut f).take(want).read_to_end(&mut buf)?;
            let got = buf.len() - start;
            let done = got < want as usize || from + buf.len() as u64 >= len;
            if done || buf[start..].contains(&b'\n') {
                break;
            }
            want = (len - from - buf.len() as u64).min(READ_MAX);
        }
        let Some(last) = buf.iter().rposition(|&b| b == b'\n') else { return Ok(None) };
        let to = from + last as u64 + 1;
        let lines = buf[..last]
            .split(|&b| b == b'\n')
            .map(|l| l.strip_suffix(b"\r").unwrap_or(l))
            .filter(|l| !l.iter().all(u8::is_ascii_whitespace))
            .map(|l| String::from_utf8_lossy(l).into_owned())
            .collect();
        self.files.insert(path.to_path_buf(), Mark { at: to, id });
        // Whole lines may wait past what this read took.
        if to < len && buf.len() as u64 >= READ_MAX {
            self.more.insert(path.to_path_buf());
        }
        Ok(Some(Appended { from, to, read, lines }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-cursor-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn append(p: &Path, text: &str) {
        std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap().write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn reads_whole_lines_as_they_are_added() {
        let d = dir("lines");
        let f = d.join("a.jsonl");
        let mut c = Cursor::default();
        append(&f, "{\"a\":1}\n{\"b\"");
        let a = c.read(&f).unwrap().unwrap();
        assert_eq!((a.from, a.to, a.read, a.lines.as_slice()), (0, 8, Read::New, ["{\"a\":1}".to_string()].as_slice()));
        assert_eq!(c.read(&f).unwrap(), None, "half a line waits");
        append(&f, ":2}\r\n\n{\"c\":3}\n");
        let a = c.read(&f).unwrap().unwrap();
        assert_eq!((a.from, a.read, a.lines.as_slice()), (8, Read::Appended, ["{\"b\":2}".to_string(), "{\"c\":3}".to_string()].as_slice()));
        assert_eq!(c.read(&f).unwrap(), None);
        // Shrunk: read again from the start.
        std::fs::write(&f, "{\"z\":0}\n").unwrap();
        let a = c.read(&f).unwrap().unwrap();
        assert_eq!((a.from, a.read, a.lines.len()), (0, Read::Rewritten, 1));
        std::fs::remove_file(&f).unwrap();
        assert_eq!(c.read(&f).unwrap_err().kind(), std::io::ErrorKind::NotFound);
        assert!(c.forget(&f) && !c.forget(&f));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_seeded_file_reads_on_from_its_seed() {
        let d = dir("seed");
        let f = d.join("a.jsonl");
        append(&f, "old\n");
        let mut c = Cursor::default();
        c.seed(&f, 4);
        assert_eq!(c.read(&f).unwrap(), None);
        append(&f, "new\n");
        let a = c.read(&f).unwrap().unwrap();
        assert_eq!((a.from, a.read, a.lines.as_slice()), (4, Read::Appended, ["new".to_string()].as_slice()));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_replaced_file_is_read_from_its_start() {
        let d = dir("replaced");
        let f = d.join("a.jsonl");
        append(&f, "one\n");
        let mut c = Cursor::default();
        c.read(&f).unwrap();
        let other = d.join("b.jsonl");
        append(&other, "two\nthree\n");
        std::fs::rename(&other, &f).unwrap();
        let a = c.read(&f).unwrap().unwrap();
        assert_eq!((a.read, a.lines.len()), (Read::Rewritten, 2));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_long_file_is_read_a_piece_at_a_time() {
        let d = dir("long");
        let f = d.join("a.jsonl");
        let line = format!("{}\n", "x".repeat(1023));
        let n = (READ_MAX / 1024) as usize + 10;
        std::fs::write(&f, line.repeat(n)).unwrap();
        let mut c = Cursor::default();
        let first = c.read(&f).unwrap().unwrap();
        assert_eq!(first.lines.len(), (READ_MAX / 1024) as usize);
        assert_eq!(c.take_more(), vec![f.clone()]);
        let rest = c.read(&f).unwrap().unwrap();
        assert_eq!((rest.read, rest.lines.len()), (Read::Appended, 10));
        assert_eq!(c.take_more(), Vec::<PathBuf>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
