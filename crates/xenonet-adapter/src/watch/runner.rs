//! The system's file events, for the specs their owners added: each event's
//! paths go to the owner of every spec that counts them, a 100 ms window at
//! a time. An owner is a key the caller picks (`app:claude`, a repo's
//! folder). When the system drops events (inotify's queue overflowed,
//! FSEvents says to scan a folder again) or a watch fails, the owners it
//! may concern hear a reconcile is needed, since a scan is then the only
//! way to know what changed.
//!
//! A spec's folder that isn't there yet is waited for: the nearest folder
//! above it (a few levels up at most) is watched until it appears, and its
//! owner hears a reconcile then, for the files written before the watch.
//!
//! A file its owner says is being written (`follow`) is also watched on
//! its own where the folder's watch is slow (`super::follow`: kqueue on
//! macOS). Its writes come both ways then; the folder's are left out while
//! the file's own watch stands, and any that arrive in another window
//! read nothing new, since the cursor reads on from where it read to.

use super::follow::{self, Follower};
use super::WatchSpec;
use notify::event::ModifyKind;
use notify::{EventKind, RecursiveMode, Watcher as _};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long events gather before their paths are handed on: the first
/// event of a window opens it, so a file written to without pause is still
/// read every 100 ms rather than once it stops.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

// How far above a missing folder the watcher looks for one to wait in.
// Further up is the home folder or the disk, whose events would all be
// looked at for one folder's sake.
const WAIT_LEVELS: usize = 3;

/// What an owner hears.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Batch {
    /// Paths of its specs that changed in the window, each once, in order.
    Changed { key: String, paths: Vec<PathBuf> },
    /// Events were lost or a watch failed: scan again. `key` None: any
    /// owner may be concerned.
    Reconcile { key: Option<String>, why: String },
}

// Where an event came from: a folder's watch, or a file's own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    Folder,
    File,
}

enum Msg {
    Event(Via, notify::Result<notify::Event>),
    Tell(Batch),
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Placed {
    // Its folder is watched.
    Root,
    // Its folder isn't there; this one above it is watched until it is.
    Waiting(PathBuf),
    // Its folder isn't there, and nothing near it is.
    Unwatched,
}

struct Entry {
    spec: WatchSpec,
    placed: Placed,
}

#[derive(Default)]
struct Count {
    tree: usize,
    flat: usize,
    mode: Option<RecursiveMode>,
}

struct Shared {
    notify: notify::RecommendedWatcher,
    // Each folder the system watches, by how many specs want it and how.
    // Two worktrees of one repo watch the same refs, and one letting go
    // mustn't take the other's watch.
    counts: HashMap<PathBuf, Count>,
    owners: BTreeMap<String, Vec<Entry>>,
    follower: Follower,
}

/// The running watcher. Dropping it stops it.
pub struct Watcher {
    shared: Arc<Mutex<Shared>>,
    tx: Sender<Msg>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn lock(m: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Watcher {
    /// Starts watching nothing yet; `on` hears every batch, on the
    /// watcher's own thread (it may block: file reads are fine there).
    pub fn start(on: impl FnMut(Batch) + Send + 'static) -> notify::Result<Watcher> {
        Watcher::start_with(follow::cap(), on)
    }

    // The same, with at most `cap` files watched on their own.
    pub(crate) fn start_with(cap: usize, on: impl FnMut(Batch) + Send + 'static) -> notify::Result<Watcher> {
        let (tx, rx) = channel::<Msg>();
        let events = tx.clone();
        let notify = notify::recommended_watcher(move |e| {
            let _ = events.send(Msg::Event(Via::Folder, e));
        })?;
        let events = tx.clone();
        let follower = if cap == 0 {
            Follower::default()
        } else {
            Follower::start(cap, move |e| {
                let _ = events.send(Msg::Event(Via::File, Ok(e)));
            })
        };
        let shared = Arc::new(Mutex::new(Shared { notify, counts: HashMap::new(), owners: BTreeMap::new(), follower }));
        let s = shared.clone();
        let mut on = on;
        let thread = std::thread::Builder::new()
            .name("xenonet-watch".into())
            .spawn(move || {
                let mut stopping = false;
                while !stopping {
                    let mut window = Vec::new();
                    match rx.recv() {
                        Ok(Msg::Event(from, e)) => window.push((from, e)),
                        Ok(Msg::Tell(b)) => {
                            on(b);
                            continue;
                        }
                        Ok(Msg::Stop) | Err(_) => break,
                    }
                    let until = Instant::now() + DEBOUNCE;
                    let mut told = Vec::new();
                    loop {
                        let left = until.saturating_duration_since(Instant::now());
                        match rx.recv_timeout(left) {
                            Ok(Msg::Event(from, e)) => window.push((from, e)),
                            Ok(Msg::Tell(b)) => told.push(b),
                            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                                stopping = true;
                                break;
                            }
                            Err(RecvTimeoutError::Timeout) => break,
                        }
                    }
                    // Routed under the lock, handed on without it, so `on`
                    // may add or remove specs.
                    let batches = lock(&s).route(window);
                    for b in told.into_iter().chain(batches) {
                        on(b);
                    }
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Watcher { shared, tx, thread: Some(thread) })
    }

    /// Watches an owner's specs, in place of any it had. A spec that can't
    /// be watched is told as a reconcile for its owner.
    pub fn add(&self, key: &str, specs: Vec<WatchSpec>) {
        let told = {
            let mut s = lock(&self.shared);
            s.drop_owner(key);
            let mut told = Vec::new();
            let entries: Vec<Entry> = specs
                .into_iter()
                .map(|spec| {
                    let spec = spec.canonical();
                    let placed = s.place(&spec, &mut told, key);
                    Entry { spec, placed }
                })
                .collect();
            s.owners.insert(key.to_string(), entries);
            told
        };
        for b in told {
            let _ = self.tx.send(Msg::Tell(b));
        }
    }

    /// Stops watching an owner's specs.
    pub fn remove(&self, key: &str) {
        lock(&self.shared).drop_owner(key);
    }

    /// The owners watched now.
    pub fn keys(&self) -> Vec<String> {
        lock(&self.shared).owners.keys().cloned().collect()
    }

    /// Watches a file of a spec's on its own as well as through its
    /// folder, because it's being written now (a live session's
    /// transcript): on macOS, where the folder's events can lag by
    /// seconds, its writes then come straight from the kernel. Whether it
    /// is: false past the cap, and anywhere but macOS, where the folder's
    /// watch is as quick.
    pub fn follow(&self, path: &Path) -> bool {
        self.follower().follow(path)
    }

    /// Stops watching a file on its own: it's gone, or no longer read.
    pub fn unfollow(&self, path: &Path) {
        self.follower().unfollow(path);
    }

    /// The files watched on their own now.
    pub fn followed(&self) -> Vec<PathBuf> {
        self.follower().followed()
    }

    /// What `follow` and `unfollow` go through, for a caller that must
    /// reach them from the watcher's own thread.
    pub fn follower(&self) -> Follower {
        lock(&self.shared).follower.clone()
    }

    /// An owner's specs, as the watcher names their folders.
    pub fn specs(&self, key: &str) -> Vec<WatchSpec> {
        lock(&self.shared).owners.get(key).map(|es| es.iter().map(|e| e.spec.clone()).collect()).unwrap_or_default()
    }

    /// Hands on paths as if a folder's watch had said they changed, through
    /// the same window and routing as the system's events. For tests:
    /// FSEvents on a Mac whose fseventsd is busy (a few builds at once)
    /// can lag by tens of seconds or drop a write, so a test of what the
    /// watcher does with an event gives it one rather than waiting on the
    /// system. The paths are named as the system names them (real paths).
    #[cfg(any(test, feature = "testing"))]
    pub fn inject(&self, paths: &[PathBuf]) {
        let ev = paths.iter().fold(notify::Event::new(EventKind::Any), |ev, p| ev.add_path(p.clone()));
        let _ = self.tx.send(Msg::Event(Via::Folder, Ok(ev)));
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        let follower = lock(&self.shared).follower.clone();
        follower.stop();
        if let Some(t) = self.thread.take() {
            // Dropped from its own thread (an `on` that let go of it), it
            // ends when its loop does.
            if t.thread().id() != std::thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

fn nearest(root: &Path) -> Option<PathBuf> {
    root.ancestors().skip(1).take(WAIT_LEVELS).find(|a| a.is_dir()).map(Path::to_path_buf)
}

impl Shared {
    // Watches a folder for one more spec; an error says why it can't.
    fn want(&mut self, dir: &Path, tree: bool) -> notify::Result<()> {
        let c = self.counts.entry(dir.to_path_buf()).or_default();
        if tree {
            c.tree += 1;
        } else {
            c.flat += 1;
        }
        let r = self.sync(dir);
        if r.is_err() {
            self.unwant(dir, tree);
        }
        r
    }

    fn unwant(&mut self, dir: &Path, tree: bool) {
        if let Some(c) = self.counts.get_mut(dir) {
            if tree {
                c.tree = c.tree.saturating_sub(1);
            } else {
                c.flat = c.flat.saturating_sub(1);
            }
        }
        let _ = self.sync(dir);
    }

    // Watches a folder as its specs want it now: under it too when any
    // wants that, not at all when none wants it.
    fn sync(&mut self, dir: &Path) -> notify::Result<()> {
        let Some(c) = self.counts.get(dir) else { return Ok(()) };
        let want = if c.tree > 0 {
            Some(RecursiveMode::Recursive)
        } else if c.flat > 0 {
            Some(RecursiveMode::NonRecursive)
        } else {
            None
        };
        let have = c.mode;
        if want == have {
            return Ok(());
        }
        if have.is_some() {
            let _ = self.notify.unwatch(dir);
        }
        let r = match want {
            Some(m) => self.notify.watch(dir, m),
            None => Ok(()),
        };
        let mode = if r.is_ok() { want } else { None };
        match self.counts.get_mut(dir) {
            Some(c) if mode.is_none() && c.tree == 0 && c.flat == 0 => {
                self.counts.remove(dir);
            }
            Some(c) => c.mode = mode,
            None => {}
        }
        r
    }

    fn place(&mut self, spec: &WatchSpec, told: &mut Vec<Batch>, key: &str) -> Placed {
        if spec.root.is_dir() {
            return match self.want(&spec.root, spec.recursive) {
                Ok(()) => Placed::Root,
                Err(e) => {
                    told.push(Batch::Reconcile { key: Some(key.to_string()), why: format!("can't watch {}: {e}", spec.root.display()) });
                    Placed::Unwatched
                }
            };
        }
        match nearest(&spec.root) {
            Some(above) if self.want(&above, false).is_ok() => Placed::Waiting(above),
            _ => {
                told.push(Batch::Reconcile { key: Some(key.to_string()), why: format!("{} isn't there to watch", spec.root.display()) });
                Placed::Unwatched
            }
        }
    }

    fn unplace(&mut self, spec: &WatchSpec, placed: &Placed) {
        match placed {
            Placed::Root => self.unwant(&spec.root, spec.recursive),
            Placed::Waiting(above) => self.unwant(above, false),
            Placed::Unwatched => {}
        }
    }

    fn drop_owner(&mut self, key: &str) {
        for e in self.owners.remove(key).unwrap_or_default() {
            self.unplace(&e.spec, &e.placed);
        }
    }

    // A window's events, as each owner hears them.
    fn route(&mut self, window: Vec<(Via, notify::Result<notify::Event>)>) -> Vec<Batch> {
        let mut out = Vec::new();
        let mut changed: BTreeMap<String, BTreeSet<PathBuf>> = BTreeMap::new();
        let mut touched: Vec<PathBuf> = Vec::new();
        for (from, e) in window {
            match e {
                Err(e) => {
                    let keys = self.keys_near(&e.paths);
                    let why = format!("the file watcher failed: {e}");
                    if keys.is_empty() {
                        out.push(Batch::Reconcile { key: None, why });
                    } else {
                        out.extend(keys.into_iter().map(|k| Batch::Reconcile { key: Some(k), why: why.clone() }));
                    }
                }
                Ok(ev) if ev.need_rescan() => {
                    let keys = self.keys_near(&ev.paths);
                    let why = "the system dropped file events".to_string();
                    if keys.is_empty() {
                        out.push(Batch::Reconcile { key: None, why });
                    } else {
                        out.extend(keys.into_iter().map(|k| Batch::Reconcile { key: Some(k), why: why.clone() }));
                    }
                }
                // Reads are no change, and XenoNet's own reads of a file
                // would otherwise come back to it.
                Ok(ev) if matches!(ev.kind, EventKind::Access(_)) => {}
                // A followed file's writes (and the times they set), which
                // its own watch tells sooner. What else the folder says of
                // it (created, renamed, removed) still counts.
                Ok(ev)
                    if from == Via::Folder
                        && matches!(ev.kind, EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)))
                        && !ev.paths.is_empty()
                        && ev.paths.iter().all(|p| self.follower.is_followed(p)) => {}
                Ok(ev) => {
                    for p in ev.paths {
                        for (key, entries) in &self.owners {
                            if entries.iter().any(|en| en.placed == Placed::Root && en.spec.matches(&p)) {
                                changed.entry(key.clone()).or_default().insert(p.clone());
                            }
                        }
                        touched.push(p);
                    }
                }
            }
        }
        out.extend(self.replace(&touched));
        out.extend(changed.into_iter().map(|(key, paths)| Batch::Changed { key, paths: paths.into_iter().collect() }));
        out
    }

    // The owners whose folders hold one of these paths, or hold them all
    // (an event with no paths).
    fn keys_near(&self, paths: &[PathBuf]) -> Vec<String> {
        self.owners
            .iter()
            .filter(|(_, es)| es.iter().any(|e| paths.is_empty() || paths.iter().any(|p| p.starts_with(&e.spec.root) || e.spec.root.starts_with(p))))
            .map(|(k, _)| k.clone())
            .collect()
    }

    // After events in or above a spec's folder: a folder waited for that
    // appeared is watched now (and its owner scans for what came before),
    // one watched that's gone is waited for again.
    fn replace(&mut self, touched: &[PathBuf]) -> Vec<Batch> {
        if touched.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut moves = Vec::new();
        for (key, es) in &self.owners {
            for (i, e) in es.iter().enumerate() {
                let near = touched.iter().any(|p| e.spec.root.starts_with(p) || p.parent().is_some_and(|d| e.spec.root.starts_with(d)));
                if !near {
                    continue;
                }
                let gone = e.placed == Placed::Root && !e.spec.root.is_dir();
                let waiting = matches!(&e.placed, Placed::Waiting(above) if e.spec.root.is_dir() || nearest(&e.spec.root).as_ref() != Some(above));
                if gone || waiting {
                    moves.push((key.clone(), i));
                }
            }
        }
        for (key, i) in moves {
            let Some(e) = self.owners.get_mut(&key).and_then(|es| es.get_mut(i)) else { continue };
            let (spec, was) = (e.spec.clone(), e.placed.clone());
            self.unplace(&spec, &was);
            let mut told = Vec::new();
            let now = self.place(&spec, &mut told, &key);
            if now == Placed::Root {
                told.push(Batch::Reconcile { key: Some(key.clone()), why: format!("{} appeared", spec.root.display()) });
            } else if was == Placed::Root {
                told.push(Batch::Reconcile { key: Some(key.clone()), why: format!("{} went away", spec.root.display()) });
            }
            if let Some(e) = self.owners.get_mut(&key).and_then(|es| es.get_mut(i)) {
                e.placed = now;
            }
            out.extend(told);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch::heard;
    use std::sync::mpsc::Receiver;

    // The tests hand the watcher its events (`Watcher::inject`) rather than
    // wait on the system's, which a busy Mac's fseventsd can hold back for
    // tens of seconds or drop. What the system watches is read from the
    // watcher's own table. One test, the last, waits on the system itself.

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-watcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        PathBuf::from(xenonet_os::fs::realpath_native(&d.to_string_lossy()).unwrap())
    }

    fn started() -> (Watcher, Receiver<Batch>) {
        let (tx, rx) = channel();
        let w = Watcher::start(move |b| {
            let _ = tx.send(b);
        })
        .unwrap();
        (w, rx)
    }

    fn changed_with(key: &str, file: &Path) -> impl Fn(&Batch) -> bool {
        let (key, file) = (key.to_string(), file.to_path_buf());
        move |b| matches!(b, Batch::Changed { key: k, paths } if *k == key && paths.contains(&file))
    }

    // The first batch `want` takes. What it waits for was injected, so it
    // comes a window later; the minute is only a backstop.
    fn next(rx: &Receiver<Batch>, want: impl FnMut(&Batch) -> bool) -> Option<Batch> {
        heard(rx, want, || {})
    }

    // How the system watches a folder now, if it does.
    fn mode(w: &Watcher, d: &Path) -> Option<RecursiveMode> {
        lock(&w.shared).counts.get(d).and_then(|c| c.mode)
    }

    #[test]
    fn hands_each_owner_its_own_paths() {
        let d = dir("owners");
        std::fs::create_dir_all(d.join("p/w")).unwrap();
        let (w, rx) = started();
        w.add("app:a", vec![WatchSpec::tree("t", d.join("p"), &["*/*.jsonl"])]);
        w.add("app:b", vec![WatchSpec::flat("s", &d, &["state.db"])]);
        assert_eq!((mode(&w, &d.join("p")), mode(&w, &d)), (Some(RecursiveMode::Recursive), Some(RecursiveMode::NonRecursive)));
        let (t, notes, db) = (d.join("p/w/s.jsonl"), d.join("p/w/notes.txt"), d.join("state.db"));
        std::fs::write(&t, "{}\n").unwrap();
        std::fs::write(&notes, "x").unwrap();
        std::fs::write(&db, "x").unwrap();
        w.inject(&[t.clone(), notes, db.clone()]);
        let a = next(&rx, |b| matches!(b, Batch::Changed { key, .. } if key == "app:a")).expect("the transcript's owner hears of it");
        assert_eq!(a, Batch::Changed { key: "app:a".into(), paths: vec![t] }, "not of the notes beside it");
        let b = next(&rx, |b| matches!(b, Batch::Changed { key, .. } if key == "app:b")).expect("the database's owner hears of it");
        assert_eq!(b, Batch::Changed { key: "app:b".into(), paths: vec![db] });
        w.remove("app:a");
        assert_eq!(w.keys(), ["app:b"]);
        assert_eq!(mode(&w, &d.join("p")), None, "its folder is let go");
        drop(w);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn waits_for_a_folder_that_isnt_there_yet() {
        let d = dir("waits");
        let (w, rx) = started();
        let root = d.join("sessions");
        w.add("app:c", vec![WatchSpec::tree("t", &root, &["**/*.jsonl"])]);
        assert_eq!((mode(&w, &d), mode(&w, &root)), (Some(RecursiveMode::NonRecursive), None), "the folder above it is watched meanwhile");
        std::fs::create_dir_all(root.join("2026")).unwrap();
        // The folder watched names the one made in it.
        w.inject(std::slice::from_ref(&root));
        assert!(next(&rx, |b| matches!(b, Batch::Reconcile { key: Some(k), why } if k == "app:c" && why.ends_with("appeared"))).is_some(), "a scan for what came before the watch");
        assert_eq!((mode(&w, &d), mode(&w, &root)), (None, Some(RecursiveMode::Recursive)), "then the folder itself");
        let f = root.join("2026/r.jsonl");
        std::fs::write(&f, "{}\n").unwrap();
        w.inject(std::slice::from_ref(&f));
        assert!(next(&rx, changed_with("app:c", &f)).is_some(), "then its files' events");
        drop(w);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_folder_two_owners_watch_stays_watched_for_the_one_left() {
        let d = dir("shared");
        let (w, rx) = started();
        w.add("git:a", vec![WatchSpec::tree("refs", &d, &[])]);
        w.add("git:b", vec![WatchSpec::tree("refs", &d, &[])]);
        w.remove("git:a");
        assert_eq!(mode(&w, &d), Some(RecursiveMode::Recursive), "still watched");
        let f = d.join("main");
        std::fs::write(&f, "x").unwrap();
        w.inject(std::slice::from_ref(&f));
        assert!(next(&rx, changed_with("git:b", &f)).is_some());
        w.remove("git:b");
        assert_eq!(mode(&w, &d), None, "let go with the last");
        drop(w);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn past_the_cap_a_file_is_heard_through_its_folder() {
        use std::io::Write as _;
        let d = dir("cap");
        let (a, b) = (d.join("a.jsonl"), d.join("b.jsonl"));
        let append = |p: &Path| std::fs::OpenOptions::new().append(true).open(p).unwrap().write_all(b"{}\n").unwrap();
        std::fs::write(&a, "{}\n").unwrap();
        std::fs::write(&b, "{}\n").unwrap();
        let (tx, rx) = channel();
        let w = Watcher::start_with(1, move |b| {
            let _ = tx.send(b);
        })
        .unwrap();
        w.add("app:a", vec![WatchSpec::flat("t", &d, &["*.jsonl"])]);
        let mac = cfg!(target_os = "macos");
        assert_eq!(w.follow(&a), mac, "followed on macOS alone");
        assert!(!w.follow(&b), "past the cap");
        assert_eq!(w.followed(), if mac { vec![a.clone()] } else { Vec::new() });
        append(&b);
        w.inject(std::slice::from_ref(&b));
        assert!(next(&rx, changed_with("app:a", &b)).is_some(), "the file past the cap, through its folder");
        // The followed file's own watch: kqueue on macOS, the kernel's
        // own events with nothing between to lag (elsewhere the folder's,
        // as quick), so this waits on the system.
        let write = || append(&a);
        write();
        assert!(heard(&rx, changed_with("app:a", &a), write).is_some(), "the followed one");
        w.unfollow(&a);
        assert_eq!(w.follow(&b), mac, "a slot let go is taken");
        drop(w);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_missing_folder_with_nothing_near_it_asks_for_a_scan() {
        let (w, rx) = started();
        let far = std::env::temp_dir().join(format!("xenonet-nowhere-{}", std::process::id())).join("a/b/c/d");
        w.add("app:d", vec![WatchSpec::tree("t", &far, &[])]);
        assert!(next(&rx, |b| matches!(b, Batch::Reconcile { key: Some(k), why } if k == "app:d" && why.ends_with("isn't there to watch"))).is_some());
    }

    // The system's own events (FSEvents, inotify, ReadDirectoryChangesW)
    // reach the owner of the file written. A busy Mac's can lag by tens
    // of seconds or drop a write, so the file is written again every few
    // seconds, for up to a minute.
    #[test]
    fn the_systems_own_events_reach_their_owner() {
        let d = dir("system");
        std::fs::create_dir_all(d.join("w")).unwrap();
        let (w, rx) = started();
        w.add("app:a", vec![WatchSpec::tree("t", &d, &["*/*.jsonl"])]);
        let t = d.join("w/s.jsonl");
        let write = || std::fs::write(&t, "{}\n").unwrap();
        write();
        assert!(heard(&rx, changed_with("app:a", &t), write).is_some(), "the system's event, within a minute");
        drop(w);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
