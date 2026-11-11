//! Watching an app's files instead of polling them (docs/design/goals.md,
//! goals 4 and 5: push, not poll; a scan only as the backstop).
//!
//! - A platform says where its sessions and transcripts live
//!   (`Platform::watch`, a `WatchSpec` each), and what a set of changed
//!   paths means (`Platform::changed`, `SessionEvent`s), reading only what
//!   was added since its `Cursor`.
//! - `runner::Watcher` runs the system's file events (`notify`: FSEvents,
//!   inotify, ReadDirectoryChangesW) with a 100 ms debounce and hands each
//!   owner the paths of its own specs that changed. When the system drops
//!   events (a queue overflow) or a watch fails, it says a reconcile is
//!   needed: the scan is what catches up then.
//! - `follow`: on macOS, where FSEvents can lag by seconds, a transcript
//!   being written is watched on its own too (kqueue), beside its folder.
//! - `sessions::Sessions` drives the registry's platforms on one watcher:
//!   their paths go through `changed`, other owners' (a repo's) pass
//!   through as they are. A transcript it reads lines of is followed.
//!
//! An adapter whose sources can't be watched gives no specs, and its scan
//! stays the reconcile path.

pub mod cursor;
pub mod follow;
pub mod runner;
pub mod sessions;

pub use cursor::{Appended, Cursor};
pub use runner::{Batch, Watcher, DEBOUNCE};
pub use sessions::{Report, Sessions};

use std::path::{Path, PathBuf};

/// A folder an adapter's files live under, and which of its files count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WatchSpec {
    /// What the folder is to its adapter ("transcripts", "desktop",
    /// "state"), so `changed` knows what a path under it means.
    pub tag: &'static str,
    /// The folder. It need not be there yet: the watcher watches the
    /// nearest folder above it until it appears.
    pub root: PathBuf,
    /// Whether the folders under it are watched too.
    pub recursive: bool,
    /// Which files count, as globs over the path relative to `root`, with /
    /// between its parts: `*` and `?` within a part, `**` for any number of
    /// parts. Empty for every file.
    pub include: Vec<String>,
}

impl WatchSpec {
    /// A folder and everything under it.
    pub fn tree(tag: &'static str, root: impl Into<PathBuf>, include: &[&str]) -> WatchSpec {
        WatchSpec { tag, root: root.into(), recursive: true, include: include.iter().map(|s| (*s).to_string()).collect() }
    }

    /// A folder's own files, none below it.
    pub fn flat(tag: &'static str, root: impl Into<PathBuf>, include: &[&str]) -> WatchSpec {
        WatchSpec { tag, root: root.into(), recursive: false, include: include.iter().map(|s| (*s).to_string()).collect() }
    }

    /// One file, watched through its folder, and the files beside it
    /// named for it with one of `beside` after (a database's `-wal`). A
    /// name with * or ? in it matches loosely: they stay wildcards, which
    /// still match the name itself.
    pub fn file(tag: &'static str, path: impl AsRef<Path>, beside: &[&str]) -> Option<WatchSpec> {
        let path = path.as_ref();
        let name = path.file_name()?.to_string_lossy().into_owned();
        let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
        let include = std::iter::once(name.clone()).chain(beside.iter().map(|b| format!("{name}{b}"))).collect();
        Some(WatchSpec { tag, root: parent.to_path_buf(), recursive: false, include })
    }

    /// The same spec with its folder as the system names it (links
    /// followed, Windows's long form), which is how file events name the
    /// paths under it. A folder that isn't there yet keeps the part that
    /// isn't, under the real path of the part that is.
    #[must_use]
    pub fn canonical(&self) -> WatchSpec {
        WatchSpec { root: canonical(&self.root), ..self.clone() }
    }

    /// A path's place under the folder, with / between its parts, or None
    /// for a path outside it (or below it, for a flat spec).
    pub fn relative(&self, path: &Path) -> Option<String> {
        let rest = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        if parts.is_empty() || (!self.recursive && parts.len() > 1) {
            return None;
        }
        Some(parts.join("/"))
    }

    /// Whether a path is one of the files this spec counts.
    pub fn matches(&self, path: &Path) -> bool {
        self.relative(path).is_some_and(|rel| self.include.is_empty() || self.include.iter().any(|g| glob(g, &rel)))
    }
}

/// A folder as the system names it: the real path of the part that's there,
/// and the rest as it was written.
pub fn canonical(path: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = xenonet_os::fs::realpath_native(&at.to_string_lossy()) {
            let mut out = PathBuf::from(real);
            out.extend(missing.iter().rev());
            return out;
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name.to_os_string());
                at = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// A path with / between its parts, as an adapter's `classify` reads one.
pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Whether `text` (parts joined by /) matches `pattern`: `*` and `?` within
/// a part, `**` for any number of whole parts.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<&str> = pattern.split('/').collect();
    let t: Vec<&str> = text.split('/').collect();
    parts(&p, &t)
}

fn parts(p: &[&str], t: &[&str]) -> bool {
    match p.split_first() {
        None => t.is_empty(),
        Some((&"**", rest)) => (0..=t.len()).any(|skip| parts(rest, &t[skip..])),
        Some((first, rest)) => t.split_first().is_some_and(|(head, tail)| part(first.as_bytes(), head.as_bytes()) && parts(rest, tail)),
    }
}

// One part against a pattern of *, ? and plain bytes, backtracking to the
// last * on a mismatch.
fn part(p: &[u8], t: &[u8]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == b'*')
}

/// What changed in an app's sessions, as its adapter reads a set of
/// changed paths.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionEvent {
    /// Whole lines added to a session's transcript.
    Lines(Lines),
    /// A session's record changed where it's read whole (a sidecar, a
    /// desktop record, a summary): read it again. `session` None: the
    /// app's source as a whole (a database), which names no one session.
    Changed { session: Option<String>, file: PathBuf },
    /// A session's transcript was removed or moved away.
    Gone { session: String, agent: Option<String>, file: PathBuf },
}

/// Lines read from a session's transcript, from `from` to `to` (bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct Lines {
    pub session: String,
    /// The subagent the file is of, when it's a subagent's.
    pub agent: Option<String>,
    pub file: PathBuf,
    pub from: u64,
    pub to: u64,
    pub read: Read,
    /// Each line, without its line end; blank lines left out.
    pub lines: Vec<String>,
}

/// How lines were read from a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The first lines the cursor read of the file: from its start, or
    /// from where it was seeded.
    New,
    /// What was added since the last read.
    Appended,
    /// The file shrank or was replaced: read again from its start, and
    /// what was read of it before no longer stands.
    Rewritten,
}

/// A transcript's new lines, or that it's gone, as an adapter's `changed`
/// reads a path it classified.
pub fn transcript(cursor: &mut Cursor, path: &Path, session: &str, agent: Option<&str>, out: &mut Vec<SessionEvent>) {
    match cursor.read(path) {
        Ok(Some(a)) => out.push(SessionEvent::Lines(Lines {
            session: session.to_string(),
            agent: agent.map(str::to_string),
            file: path.to_path_buf(),
            from: a.from,
            to: a.to,
            read: a.read,
            lines: a.lines,
        })),
        Ok(None) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            cursor.forget(path);
            out.push(SessionEvent::Gone { session: session.to_string(), agent: agent.map(str::to_string), file: path.to_path_buf() });
        }
        // Another error (a file held open elsewhere on Windows) is read
        // again with the next event, or by the reconcile.
        Err(_) => {}
    }
}

/// A record read whole changed.
pub fn changed(path: &Path, session: Option<String>, out: &mut Vec<SessionEvent>) {
    out.push(SessionEvent::Changed { session, file: path.to_path_buf() });
}

/// What an adapter of a source read whole (a database, a folder of
/// records) gives for its paths: one `Changed` naming no session, for the
/// first path that's one of its specs'.
pub fn whole_source(paths: &[PathBuf], cursor: &Cursor) -> Vec<SessionEvent> {
    paths.iter().find(|p| cursor.spec_of(p).is_some()).map(|p| vec![SessionEvent::Changed { session: None, file: p.clone() }]).unwrap_or_default()
}

/// The first message `want` takes, within a minute, with `poke` (a file
/// written again) every few seconds until then. FSEvents on a Mac whose
/// fseventsd is behind (a busy build) has been seen to take tens of
/// seconds, or to miss a write, so a test of the watcher gives the
/// system another event to deliver rather than failing on its lag.
#[cfg(test)]
pub(crate) fn heard<T>(rx: &std::sync::mpsc::Receiver<T>, mut want: impl FnMut(&T) -> bool, mut poke: impl FnMut()) -> Option<T> {
    use std::time::{Duration, Instant};
    let end = Instant::now() + Duration::from_secs(60);
    let mut next = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        match rx.recv_timeout(next.min(end).saturating_duration_since(Instant::now())) {
            Ok(m) if want(&m) => return Some(m),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                poke();
                next = Instant::now() + Duration::from_secs(3);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_match_parts() {
        assert!(glob("*/*.jsonl", "-w-atlas/abc.jsonl"));
        assert!(!glob("*/*.jsonl", "-w-atlas/abc/subagents/agent-1.jsonl"));
        assert!(glob("*/*/subagents/*.jsonl", "-w-atlas/abc/subagents/agent-1.jsonl"));
        assert!(glob("**/rollout-*.jsonl", "2026/11/11/rollout-x.jsonl"));
        assert!(glob("**/rollout-*.jsonl", "rollout-x.jsonl"), "** takes no parts too");
        assert!(!glob("**/rollout-*.jsonl", "2026/x.jsonl"));
        assert!(glob("state_*.sqlite*", "state_5.sqlite-wal"));
        assert!(glob("a?c", "abc") && !glob("a?c", "abbc"));
        assert!(glob("*a*b", "xxaxxb") && !glob("*a*b", "xxaxxbx"));
    }

    #[test]
    fn specs_say_which_paths_are_theirs() {
        let tree = WatchSpec::tree("t", "/h/p", &["*/*.jsonl"]);
        assert!(tree.matches(Path::new("/h/p/w/a.jsonl")));
        assert!(!tree.matches(Path::new("/h/p/w/a.json")));
        assert!(!tree.matches(Path::new("/h/q/w/a.jsonl")));
        let flat = WatchSpec::flat("s", "/h/c", &["state_*.sqlite"]);
        assert!(flat.matches(Path::new("/h/c/state_5.sqlite")));
        assert!(!flat.matches(Path::new("/h/c/sub/state_5.sqlite")), "a flat spec has no folders below");
        let db = WatchSpec::file("db", "/h/d/opencode.db", &["-wal"]).unwrap();
        assert_eq!((db.root.as_path(), db.include.as_slice()), (Path::new("/h/d"), ["opencode.db".to_string(), "opencode.db-wal".to_string()].as_slice()));
        assert!(db.matches(Path::new("/h/d/opencode.db-wal")) && !db.matches(Path::new("/h/d/other.db")));
        assert!(WatchSpec::tree("all", "/h", &[]).matches(Path::new("/h/x/y")));
    }

    #[test]
    fn a_missing_folder_keeps_the_part_that_isnt_there() {
        let dir = std::env::temp_dir().join(format!("xenonet-watch-canonical-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = PathBuf::from(xenonet_os::fs::realpath_native(&dir.to_string_lossy()).unwrap());
        assert_eq!(canonical(&dir.join("not").join("yet")), real.join("not").join("yet"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
