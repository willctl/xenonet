//! Files watched each on their own, beside their folder's watch: the
//! transcripts being written, and the pages Xenolith syncs (goal 4: a
//! message in well under a second).
//!
//! On macOS a folder's events come through fseventsd, which a busy Mac
//! keeps behind: a line written to a transcript has been seen to take 5 to
//! 25 seconds to be heard. So a file a session is writing gets a kqueue
//! watch of its own (EVFILT_VNODE: written, extended, deleted, renamed),
//! which the kernel delivers straight to us. The folder's FSEvents watch
//! stays, for the files and sessions that are new, and for any file past
//! the cap. notify can't run its kqueue backend beside its FSEvents one
//! (its `macos_kqueue` feature turns FSEvents off), so this calls kqueue
//! itself.
//!
//! A watch holds a file descriptor, so there are at most `CAP` of them,
//! and fewer when the process may open few files (a launchd job's soft
//! limit is 256). Past the cap a file is heard through its folder only. A
//! watch is let go when its file is gone or moved, and once it hasn't been
//! written to for `IDLE` (its session ended, or waits): its next write is
//! heard through the folder again, and that follows it again.
//!
//! Elsewhere the folder's watch is already direct (inotify,
//! ReadDirectoryChangesW), and this does nothing.

#[cfg(not(target_os = "macos"))]
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The most files watched on their own.
pub const CAP: usize = 256;

/// How long a file goes unwritten before its own watch is let go.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

#[cfg(target_os = "macos")]
pub use kq::Follower;

/// Where nothing is watched on its own: every call does nothing.
#[cfg(not(target_os = "macos"))]
#[derive(Clone, Default)]
pub struct Follower(());

#[cfg(not(target_os = "macos"))]
impl Follower {
    pub fn start(_cap: usize, _on: impl Fn(notify::Event) + Send + 'static) -> Follower {
        Follower(())
    }
    pub fn follow(&self, _path: &Path) -> bool {
        false
    }
    pub fn unfollow(&self, _path: &Path) {}
    pub fn is_followed(&self, _path: &Path) -> bool {
        false
    }
    pub fn followed(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    pub fn stop(&self) {}
}

/// The cap for this process: `CAP`, or a quarter of the files it may
/// open when that's less, so the watches never starve its other files.
pub fn cap() -> usize {
    #[cfg(target_os = "macos")]
    {
        kq::open_files_limit().map_or(CAP, |n| CAP.min(n / 4))
    }
    #[cfg(not(target_os = "macos"))]
    {
        CAP
    }
}

#[cfg(target_os = "macos")]
#[expect(unsafe_code, reason = "kqueue(2), kevent(2), open(2) with O_EVTONLY and getrlimit(2), which std doesn't wrap")]
mod kq {
    use super::IDLE;
    use notify::event::{DataChange, EventKind, ModifyKind, RemoveKind, RenameMode};
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    // What a watch hears of its file.
    const FFLAGS: u32 = libc::NOTE_WRITE | libc::NOTE_EXTEND | libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE;
    // The user event that stops the thread.
    const STOP: usize = 0;
    // How often the thread wakes to let idle watches go.
    const SWEEP: Duration = Duration::from_secs(60);

    struct Watch {
        fd: std::fs::File,
        // The file's inode, so one replaced at its path is watched anew.
        ino: u64,
        written: Instant,
    }

    #[derive(Default)]
    struct Files {
        by_path: HashMap<PathBuf, Watch>,
        by_fd: HashMap<RawFd, PathBuf>,
    }

    impl Files {
        fn remove(&mut self, path: &Path) {
            if let Some(w) = self.by_path.remove(path) {
                self.by_fd.remove(&w.fd.as_raw_fd());
                // Closing the descriptor takes its event off the queue.
            }
        }
    }

    struct Inner {
        kq: OwnedFd,
        cap: usize,
        idle: Duration,
        sweep: Duration,
        files: Mutex<Files>,
        thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    /// The files watched on their own, shared by the watcher and whoever
    /// says which files those are. None when kqueue couldn't be had: then
    /// every call does nothing, and the folders' watches are all there is.
    #[derive(Clone, Default)]
    pub struct Follower(Option<Arc<Inner>>);

    fn lock(m: &Mutex<Files>) -> std::sync::MutexGuard<'_, Files> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn change(kq: RawFd, ev: &libc::kevent) -> std::io::Result<()> {
        // SAFETY: one change from a valid kevent, no events asked back.
        let r = unsafe { libc::kevent(kq, ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        if r < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn kevent(ident: usize, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
        libc::kevent { ident, filter, flags, fflags, data: 0, udata: std::ptr::null_mut() }
    }

    /// The soft limit on the files this process may open.
    pub fn open_files_limit() -> Option<usize> {
        let mut l = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: getrlimit writes the struct it's given.
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut l) } != 0 {
            return None;
        }
        usize::try_from(l.rlim_cur).ok()
    }

    impl Follower {
        /// Starts the thread that hears the watches, which tells `on` each
        /// change to a file as a file event names it. At most `cap` files
        /// are watched.
        pub fn start(cap: usize, on: impl Fn(notify::Event) + Send + 'static) -> Follower {
            Follower::start_with(cap, IDLE, SWEEP, on)
        }

        // The same, letting a watch go once idle for `idle`, looked for
        // every `sweep`.
        fn start_with(cap: usize, idle: Duration, sweep: Duration, on: impl Fn(notify::Event) + Send + 'static) -> Follower {
            // SAFETY: kqueue takes no arguments; a negative result is an error.
            let raw = unsafe { libc::kqueue() };
            if raw < 0 {
                return Follower(None);
            }
            // SAFETY: a new descriptor this owns alone.
            let kq = unsafe { OwnedFd::from_raw_fd(raw) };
            if change(kq.as_raw_fd(), &kevent(STOP, libc::EVFILT_USER, libc::EV_ADD | libc::EV_CLEAR, 0)).is_err() {
                return Follower(None);
            }
            let inner = Arc::new(Inner { kq, cap, idle, sweep, files: Mutex::new(Files::default()), thread: Mutex::new(None) });
            let me = inner.clone();
            let Ok(t) = std::thread::Builder::new().name("xenonet-follow".into()).spawn(move || run(&me, &on)) else { return Follower(None) };
            *inner.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(t);
            Follower(Some(inner))
        }

        /// Watches a file on its own, or keeps watching it, as written to
        /// now. False when it isn't: past the cap, not there, or not on
        /// macOS.
        pub fn follow(&self, path: &Path) -> bool {
            let Some(inner) = &self.0 else { return false };
            let Ok(meta) = std::fs::metadata(path) else { return false };
            if !meta.is_file() {
                return false;
            }
            let mut files = lock(&inner.files);
            if let Some(w) = files.by_path.get_mut(path) {
                if w.ino == meta.ino() {
                    w.written = Instant::now();
                    return true;
                }
                // Another file at its path now: the watch is of the old one.
                files.remove(path);
            }
            if files.by_path.len() >= inner.cap {
                return false;
            }
            let Ok(c) = CString::new(path.as_os_str().as_bytes()) else { return false };
            // SAFETY: a NUL-terminated path; O_EVTONLY opens it for events
            // alone, so it doesn't hold its volume from being unmounted.
            let raw = unsafe { libc::open(c.as_ptr(), libc::O_EVTONLY | libc::O_CLOEXEC) };
            if raw < 0 {
                return false;
            }
            // SAFETY: a new descriptor this owns alone.
            let fd = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(raw) });
            // The inode of what was opened, which may be newer than the stat.
            let ino = fd.metadata().map_or(meta.ino(), |m| m.ino());
            let ev = kevent(fd.as_raw_fd() as usize, libc::EVFILT_VNODE, libc::EV_ADD | libc::EV_CLEAR, FFLAGS);
            if change(inner.kq.as_raw_fd(), &ev).is_err() {
                return false;
            }
            files.by_fd.insert(fd.as_raw_fd(), path.to_path_buf());
            files.by_path.insert(path.to_path_buf(), Watch { fd, ino, written: Instant::now() });
            true
        }

        /// Stops watching a file on its own: it's gone, or no longer read.
        pub fn unfollow(&self, path: &Path) {
            if let Some(inner) = &self.0 {
                lock(&inner.files).remove(path);
            }
        }

        /// Whether a file is watched on its own now.
        pub fn is_followed(&self, path: &Path) -> bool {
            self.0.as_ref().is_some_and(|i| lock(&i.files).by_path.contains_key(path))
        }

        /// The files watched on their own, in order.
        pub fn followed(&self) -> Vec<PathBuf> {
            let mut out: Vec<PathBuf> = self.0.as_ref().map(|i| lock(&i.files).by_path.keys().cloned().collect()).unwrap_or_default();
            out.sort();
            out
        }

        /// Stops the thread and lets every watch go.
        pub fn stop(&self) {
            let Some(inner) = &self.0 else { return };
            let _ = change(inner.kq.as_raw_fd(), &kevent(STOP, libc::EVFILT_USER, 0, libc::NOTE_TRIGGER));
            let t = inner.thread.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            if let Some(t) = t {
                if t.thread().id() != std::thread::current().id() {
                    let _ = t.join();
                }
            }
            *lock(&inner.files) = Files::default();
        }
    }

    // What the thread does until it's stopped: each change heard told as
    // a file event, and every minute the idle watches let go.
    fn run(inner: &Inner, on: &impl Fn(notify::Event)) {
        let empty = kevent(0, 0, 0, 0);
        let mut got = [empty; 64];
        let mut swept = Instant::now();
        let wait = libc::timespec { tv_sec: inner.sweep.as_secs() as libc::time_t, tv_nsec: libc::c_long::from(inner.sweep.subsec_nanos()) };
        loop {
            // SAFETY: the event list is `got`, as long as said; no changes.
            let n = unsafe { libc::kevent(inner.kq.as_raw_fd(), std::ptr::null(), 0, got.as_mut_ptr(), got.len() as libc::c_int, &raw const wait) };
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return;
            }
            let mut told = Vec::new();
            {
                let mut files = lock(&inner.files);
                for ev in &got[..n as usize] {
                    if ev.filter == libc::EVFILT_USER {
                        return;
                    }
                    let fd = ev.ident as RawFd;
                    let Some(path) = files.by_fd.get(&fd).cloned() else { continue };
                    if ev.fflags & (libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE) != 0 {
                        // Gone or moved: what's at its path now, if anything,
                        // is found through its folder's watch.
                        files.remove(&path);
                        let kind = if ev.fflags & libc::NOTE_RENAME != 0 { EventKind::Modify(ModifyKind::Name(RenameMode::From)) } else { EventKind::Remove(RemoveKind::File) };
                        told.push(notify::Event::new(kind).add_path(path));
                    } else if ev.fflags & (libc::NOTE_WRITE | libc::NOTE_EXTEND) != 0 {
                        if let Some(w) = files.by_path.get_mut(&path) {
                            w.written = Instant::now();
                        }
                        told.push(notify::Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content))).add_path(path));
                    }
                }
                if swept.elapsed() >= inner.sweep {
                    swept = Instant::now();
                    let idle: Vec<PathBuf> = files.by_path.iter().filter(|(_, w)| w.written.elapsed() >= inner.idle).map(|(p, _)| p.clone()).collect();
                    for p in idle {
                        files.remove(&p);
                    }
                }
            }
            for e in told {
                on(e);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write as _;
        use std::sync::mpsc::{channel, Receiver};

        fn dir(name: &str) -> PathBuf {
            let d = std::env::temp_dir().join(format!("xenonet-follow-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d.canonicalize().unwrap()
        }

        fn append(p: &Path, text: &str) {
            std::fs::OpenOptions::new().create(true).append(true).open(p).unwrap().write_all(text.as_bytes()).unwrap();
        }

        fn started(cap: usize, idle: Duration, sweep: Duration) -> (Follower, Receiver<notify::Event>) {
            let (tx, rx) = channel();
            let f = Follower::start_with(cap, idle, sweep, move |e| {
                let _ = tx.send(e);
            });
            (f, rx)
        }

        // The next event naming `path`. kqueue's are the kernel's own, with
        // no fseventsd between to lag, so they come at once; the half
        // minute is for a machine too busy to run the follower's thread.
        fn next_of(rx: &Receiver<notify::Event>, path: &Path) -> Option<notify::Event> {
            let end = Instant::now() + Duration::from_secs(30);
            while let Ok(e) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
                if e.paths.iter().any(|p| p == path) {
                    return Some(e);
                }
            }
            None
        }

        #[test]
        fn a_followed_files_writes_and_its_going_are_heard() {
            let d = dir("heard");
            let (f, rx) = started(8, IDLE, SWEEP);
            let a = d.join("a.jsonl");
            append(&a, "{}\n");
            assert!(f.follow(&a) && f.follow(&a), "followed, and again as it's written");
            assert_eq!(f.followed(), std::slice::from_ref(&a));
            append(&a, "{\"n\":1}\n");
            let e = next_of(&rx, &a).expect("its write");
            assert_eq!(e.kind, EventKind::Modify(ModifyKind::Data(DataChange::Content)));
            let moved = d.join("moved.jsonl");
            std::fs::rename(&a, &moved).unwrap();
            let e = next_of(&rx, &a).expect("its move");
            assert_eq!(e.kind, EventKind::Modify(ModifyKind::Name(RenameMode::From)));
            assert!(f.followed().is_empty(), "a file moved away is let go");
            assert!(!f.follow(&a), "nothing at its path to follow");
            f.stop();
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn a_file_replaced_at_its_path_is_followed_anew() {
            let d = dir("replaced");
            let (f, rx) = started(8, IDLE, SWEEP);
            let a = d.join("a.jsonl");
            append(&a, "{}\n");
            assert!(f.follow(&a));
            let new = d.join("new.jsonl");
            append(&new, "{}\n{}\n");
            std::fs::rename(&new, &a).unwrap();
            // The old file's watch hears it unlinked, or the next follow
            // sees another inode: either way the new one is watched next.
            assert!(f.follow(&a));
            while rx.try_recv().is_ok() {}
            assert!(f.follow(&a), "the new file is followed");
            append(&a, "{}\n");
            assert!(next_of(&rx, &a).is_some(), "the new file's write");
            f.stop();
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn past_the_cap_nothing_more_is_followed_until_a_watch_is_let_go() {
            let d = dir("cap");
            let (f, _rx) = started(2, IDLE, SWEEP);
            let [a, b, c] = ["a", "b", "c"].map(|n| d.join(format!("{n}.jsonl")));
            for p in [&a, &b, &c] {
                append(p, "{}\n");
            }
            assert!(f.follow(&a) && f.follow(&b));
            assert!(!f.follow(&c), "past the cap");
            assert_eq!(f.followed(), [a.clone(), b.clone()]);
            f.unfollow(&a);
            assert!(f.follow(&c), "a slot let go is taken");
            assert_eq!(f.followed(), [b, c]);
            f.stop();
            assert!(f.followed().is_empty(), "stopping lets every watch go");
            std::fs::remove_dir_all(&d).unwrap();
        }

        #[test]
        fn an_idle_file_is_let_go() {
            let d = dir("idle");
            let (f, _rx) = started(8, Duration::from_millis(200), Duration::from_millis(50));
            let a = d.join("a.jsonl");
            append(&a, "{}\n");
            assert!(f.follow(&a));
            let end = Instant::now() + Duration::from_secs(30);
            while !f.followed().is_empty() && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(50));
            }
            assert!(f.followed().is_empty(), "unwritten for its idle time, its watch is let go");
            f.stop();
            std::fs::remove_dir_all(&d).unwrap();
        }
    }
}
