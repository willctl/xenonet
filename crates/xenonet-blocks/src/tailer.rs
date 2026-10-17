//! A tailer reads files that grow, such as transcripts, a line at a time as
//! lines are added (core/blocks/tailer.js). It keeps an offset in each file
//! and reads from there; it hands on whole lines only, keeping a
//! half-written last line (as bytes, since a read can end inside a
//! character) for the next read; and it starts a file over when the file
//! got shorter, as when it's rewritten.
//!
//! At start it reads every file written within `prime_window_ms` as
//! history, and leaves older ones dormant: one that's written to again
//! later has what it held then read as history too, before its new lines.
//! A file it can't read at all waits the same way. Started again after a
//! stop, a file it followed before is caught up by the sweep, so what was
//! written meanwhile reads as live, not as history (the fix on
//! fix/scanner-racy-mtime, which the scanner's reading a folder again
//! needs).
//!
//! This is what the tailer knows and decides. Whoever runs it watches the
//! folders, keeps the timers, reads the files (one read of a file at a
//! time), and asks here at each step: `listed_at_start` and `at_start` for
//! what the start's walk lists, `in_sweep` for each file it follows,
//! `may_discover` and `discovered` for what a later walk lists,
//! `add_file` and `take_held` for a file it reads for the first time, then
//! `plan_take` and `took` for each read.

use crate::lifecycle::Lifecycle;
use xenonet_base::ordered::{Cursor, OrderedMap};
use xenonet_base::text::is_blank;

/// Where a file it follows is: how far it's read, and the half line after.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Followed {
    pub offset: u64,
    pub rest: Vec<u8>,
}

/// What to do with a file a walk listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    /// Leave it: known already, gone, or not one to read.
    Skip,
    /// Too old: it waits, dormant, until it's written to.
    Dormant,
    /// Read it.
    Track,
}

/// What a sweep does with a file it follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swept {
    /// It's gone: forgotten.
    Drop,
    /// It changed size: read what's new.
    Read,
    Nothing,
}

pub struct Tailer {
    pub name: String,
    pub prime_window_ms: f64,
    /// file → where it's read to, in the order they were first read.
    pub files: OrderedMap<Followed>,
    /// file too old to read at start → its size then.
    pub dormant: OrderedMap<u64>,
    pub life: Lifecycle,
}

impl Tailer {
    pub fn new(name: &str, prime_window_ms: f64, now: f64) -> Self {
        Tailer { name: name.into(), prime_window_ms, files: OrderedMap::new(), dormant: OrderedMap::new(), life: Lifecycle::new(now) }
    }

    fn old(&self, now: f64, mtime_ms: f64) -> bool {
        now - mtime_ms > self.prime_window_ms
    }

    /// Whether the start's walk should look at a file it listed: not one it
    /// followed before a stop, which the sweep catches up as live.
    pub fn listed_at_start(&self, file: &str) -> bool {
        !self.files.contains(file)
    }

    /// A file the start's walk listed, stat'ed (None when it couldn't be):
    /// history to read, or dormant.
    pub fn at_start(&mut self, file: &str, stat: Option<(f64, u64)>, now: f64) -> Listed {
        let Some((mtime, size)) = stat else { return Listed::Skip };
        if self.old(now, mtime) {
            self.dormant.insert(file, size);
            return Listed::Dormant;
        }
        Listed::Track
    }

    /// A file nothing could be read of waits like one too old to read at
    /// start: no sweep tries it again, and once it's written to and can be
    /// read, what it held up to `size` is read as history. One partly read
    /// stays as it is. Says whether it was let go.
    pub fn park(&mut self, file: &str, size: u64) -> bool {
        if self.files.get(file).is_some_and(|f| f.offset > 0) {
            return false;
        }
        self.files.remove(file);
        self.dormant.insert(file, size);
        true
    }

    /// The next file a sweep looks at, after `cursor`: like a for…of over
    /// the files, it sees files added since it began.
    pub fn next_followed(&self, cursor: Cursor) -> Option<(Cursor, String)> {
        self.files.next_after(cursor).map(|(c, k)| (c, k.to_string()))
    }

    /// A file the sweep follows, with its size now (None when it's gone).
    pub fn in_sweep(&mut self, file: &str, size: Option<u64>) -> Swept {
        match (size, self.files.get(file)) {
            (None, _) => {
                self.files.remove(file);
                Swept::Drop
            }
            (Some(size), Some(f)) if size != f.offset => Swept::Read,
            _ => Swept::Nothing,
        }
    }

    /// Whether a later walk should look at a file it listed: one it neither
    /// follows nor keeps dormant.
    pub fn may_discover(&self, file: &str) -> bool {
        !self.files.contains(file) && !self.dormant.contains(file)
    }

    /// A file a later walk listed, stat'ed: new, and read from its first
    /// line, as live; or old (copied in, say), and dormant. `now` is read
    /// only for a file that could be stat'ed.
    pub fn discovered(&mut self, file: &str, stat: Option<(f64, u64)>, now: impl FnOnce() -> f64) -> Listed {
        let Some((mtime, size)) = stat else { return Listed::Skip };
        if self.old(now(), mtime) {
            self.dormant.insert(file, size);
            return Listed::Dormant;
        }
        Listed::Track
    }

    /// A file read for the first time: followed from its start.
    pub fn add_file(&mut self, file: &str) {
        self.files.insert(file, Followed::default());
    }

    /// What a file held when it went dormant, which is read as history
    /// before its new lines; None when it wasn't dormant, or was empty.
    pub fn take_held(&mut self, file: &str) -> Option<u64> {
        self.dormant.remove(file).filter(|&n| n > 0)
    }

    /// The bytes to read of a file now `size` long, up to `until`: None
    /// when there's nothing new. A file that got shorter was written again,
    /// and starts over, without the half line from before.
    pub fn plan_take(&mut self, file: &str, size: u64, until: Option<u64>) -> Option<(u64, u64)> {
        let f = self.files.get_mut(file)?;
        if size < f.offset {
            *f = Followed::default();
        }
        let end = until.map_or(size, |u| size.min(u));
        (end > f.offset).then_some((f.offset, end))
    }

    /// The bytes read, up to `end`: the whole lines they finish, not blank,
    /// in order. Only whole lines are decoded, since a read can end inside
    /// a character; the unfinished line waits, as bytes, for the next read.
    pub fn took(&mut self, file: &str, data: &[u8], end: u64) -> Vec<String> {
        let Some(f) = self.files.get_mut(file) else { return Vec::new() };
        f.offset = end;
        let mut all = std::mem::take(&mut f.rest);
        all.extend_from_slice(data);
        let cut = all.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        f.rest = all[cut..].to_vec();
        String::from_utf8_lossy(&all[..cut]).split('\n').filter(|l| !is_blank(l)).map(str::to_string).collect()
    }

    /// Whether a read that worked makes the tailer fine again: not while a
    /// root has no watch, since new lines then wait for the sweep.
    pub fn ok_after_read(&self, watchers: usize, roots: usize) -> bool {
        self.life.state != crate::lifecycle::State::Running && !(self.life.started && watchers < roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_half_lines_and_files_written_again() {
        let mut t = Tailer::new("t", 1000.0, 0.0);
        t.add_file("a");
        assert_eq!(t.plan_take("a", 10, None), Some((0, 10)));
        assert_eq!(t.took("a", b"{\"a\":1}\n{\"b\"", 10), ["{\"a\":1}"]);
        assert_eq!(t.plan_take("a", 10, None), None, "nothing new");
        assert_eq!(t.took("a", b":2}\n \n", 16), ["{\"b\":2}"]);
        assert_eq!(t.plan_take("a", 4, None), Some((0, 4)), "shorter: written again");
        assert!(t.files.get("a").unwrap().rest.is_empty());
    }

    #[test]
    fn what_a_walk_finds() {
        let mut t = Tailer::new("t", 1000.0, 0.0);
        assert_eq!(t.at_start("old", Some((0.0, 5)), 5000.0), Listed::Dormant);
        assert_eq!(t.at_start("new", Some((4500.0, 5)), 5000.0), Listed::Track);
        t.add_file("new");
        assert!(!t.listed_at_start("new"), "followed before a stop: the sweep catches it up");
        assert!(!t.may_discover("old"));
        assert_eq!(t.take_held("old"), Some(5));
        assert_eq!(t.in_sweep("new", None), Swept::Drop);
        assert!(t.park("x", 3) && t.dormant.contains("x"));
    }
}
