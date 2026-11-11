//! The registry's platforms on one watcher: each platform's changed paths
//! read into session events through its own `changed` and cursor, and the
//! paths of any other owner (a repo, added under a key of its own) passed
//! on as they are, so one set of the system's watches serves all of them.
//!
//! A transcript an app's lines are read from is being written, so it's
//! followed (`Watcher::follow`: its own watch, where its folder's is
//! slow), until it's gone or goes idle. One the scan read that was written
//! lately is followed as it's seeded.

use super::follow::{Follower, IDLE};
use super::runner::{Batch, Watcher};
use super::{Cursor, SessionEvent, WatchSpec};
use crate::Platform;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// What the owner of the watches hears.
#[derive(Clone, Debug, PartialEq)]
pub enum Report {
    /// What changed in an app's sessions.
    Events { app: String, events: Vec<SessionEvent> },
    /// Scan again: `app`'s sessions, or every app's for None. A key that
    /// isn't an app's (a repo's) comes as `Other`.
    Reconcile { app: Option<String>, why: String },
    /// Another owner's changed paths, or that it should scan again
    /// (`paths` empty, with why).
    Other { key: String, paths: Vec<PathBuf>, reconcile: Option<String> },
}

/// The key an app's watches are added under.
pub fn app_key(id: &str) -> String {
    format!("app:{id}")
}

type Apps = Arc<Mutex<HashMap<String, (Arc<dyn Platform>, Cursor)>>>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The platforms' watches, running.
pub struct Sessions {
    apps: Apps,
    watcher: Watcher,
}

impl Sessions {
    /// Watches each platform's specs (`Platform::watch`, as
    /// `Registry::watches` gives them; a platform with none is left out),
    /// and tells `on` what changed.
    pub fn start(platforms: Vec<(Arc<dyn Platform>, Vec<WatchSpec>)>, on: impl FnMut(Report) + Send + 'static) -> notify::Result<Sessions> {
        Sessions::start_with(super::follow::cap(), platforms, on)
    }

    // The same, with at most `cap` transcripts followed.
    pub(crate) fn start_with(cap: usize, platforms: Vec<(Arc<dyn Platform>, Vec<WatchSpec>)>, on: impl FnMut(Report) + Send + 'static) -> notify::Result<Sessions> {
        let apps: Apps = Arc::new(Mutex::new(HashMap::new()));
        let them = apps.clone();
        // The watcher's own thread follows what it reads, through the
        // follower it hands back once it's up.
        let follower: Arc<OnceLock<Follower>> = Arc::new(OnceLock::new());
        let f = follower.clone();
        let mut on = on;
        let watcher = Watcher::start_with(cap, move |b| {
            for r in handle(&them, b, f.get()) {
                on(r);
            }
        })?;
        let _ = follower.set(watcher.follower());
        let s = Sessions { apps, watcher };
        for (p, specs) in platforms {
            s.add(p, specs);
        }
        Ok(s)
    }

    /// Watches a platform's specs, in place of any it had, with a cursor
    /// that has read nothing.
    pub fn add(&self, platform: Arc<dyn Platform>, specs: Vec<WatchSpec>) {
        if specs.is_empty() {
            return;
        }
        let key = app_key(platform.id());
        let specs: Vec<WatchSpec> = specs.iter().map(WatchSpec::canonical).collect();
        lock(&self.apps).insert(key.clone(), (platform, Cursor::new(specs.clone())));
        self.watcher.add(&key, specs);
    }

    /// Says an app's file was read up to `at` already (by the scan at
    /// start, or a reconcile), so its next event reads on from there.
    pub fn seed(&self, app: &str, file: &Path, at: u64) {
        match lock(&self.apps).get_mut(&app_key(app)) {
            Some((_, c)) => c.seed(file, at),
            None => return,
        }
        // Written lately: its session is likely live, so followed now
        // rather than after its next write is heard the slow way.
        let lately = std::fs::metadata(file).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < IDLE);
        if lately {
            self.watcher.follow(file);
        }
    }

    /// The ids of the apps watched.
    pub fn apps(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock(&self.apps).values().map(|(p, _)| p.id().to_string()).collect();
        ids.sort();
        ids
    }

    /// The watcher, for other owners' specs (`Watcher::add` under a key
    /// that doesn't start with `app:`).
    pub fn watcher(&self) -> &Watcher {
        &self.watcher
    }
}

fn handle(apps: &Apps, b: Batch, follower: Option<&Follower>) -> Vec<Report> {
    match b {
        Batch::Reconcile { key: None, why } => vec![Report::Reconcile { app: None, why }],
        Batch::Reconcile { key: Some(key), why } => match key.strip_prefix("app:") {
            Some(app) => vec![Report::Reconcile { app: Some(app.to_string()), why }],
            None => vec![Report::Other { key, paths: Vec::new(), reconcile: Some(why) }],
        },
        Batch::Changed { key, paths } => {
            let mut g = lock(apps);
            let Some((p, cursor)) = g.get_mut(&key) else { return vec![Report::Other { key, paths, reconcile: None }] };
            let mut events = Vec::new();
            let mut paths = paths;
            // A long file is read a piece at a time: the rest of it is
            // given to `changed` again until there's no more.
            while !paths.is_empty() {
                events.extend(p.changed(&paths, cursor));
                paths = cursor.take_more();
            }
            if events.is_empty() {
                return Vec::new();
            }
            if let Some(f) = follower {
                for e in &events {
                    match e {
                        SessionEvent::Lines(l) => {
                            f.follow(&l.file);
                        }
                        SessionEvent::Gone { file, .. } => f.unfollow(file),
                        SessionEvent::Changed { .. } => {}
                    }
                }
            }
            vec![Report::Events { app: p.id().to_string(), events }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{app, App};
    use crate::watch::{heard, transcript, Read};
    use std::sync::mpsc::channel;
    use std::time::Duration;

    // A platform whose transcripts are every .jsonl under its folder, the
    // file's name its session.
    struct Lines(App);
    impl Platform for Lines {
        fn app(&self) -> &App {
            &self.0
        }
        fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
            let mut out = Vec::new();
            for p in paths {
                let id = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                transcript(cursor, p, &id, None, &mut out);
            }
            out
        }
    }

    #[test]
    fn a_platforms_lines_come_as_they_are_written() {
        let d = std::env::temp_dir().join(format!("xenonet-sessions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let d = PathBuf::from(xenonet_os::fs::realpath_native(&d.to_string_lossy()).unwrap());
        let (tx, rx) = channel();
        let p: Arc<dyn Platform> = Arc::new(Lines(app("claude").unwrap().clone()));
        let s = Sessions::start(vec![(p, vec![WatchSpec::tree("t", &d, &["*.jsonl"])])], move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        assert_eq!(s.apps(), ["claude"]);
        let f = d.join("s1.jsonl");
        std::fs::write(&f, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        // Handed to the watcher as the folder's watch would say it, not
        // waited for from the system (runner's tests say why).
        s.watcher().inject(std::slice::from_ref(&f));
        let mut lines = Vec::new();
        heard(
            &rx,
            |r| {
                if let Report::Events { app, events } = r {
                    assert_eq!(app, "claude");
                    for e in events {
                        if let SessionEvent::Lines(l) = e {
                            assert_eq!((l.session.as_str(), l.read == Read::New || l.read == Read::Appended), ("s1", true));
                            lines.extend(l.lines.iter().cloned());
                        }
                    }
                }
                lines.len() >= 2
            },
            || {},
        );
        assert_eq!(lines, ["{\"n\":1}", "{\"n\":2}"]);
        drop(s);
        std::fs::remove_dir_all(&d).unwrap();
    }

    // How long each of `rounds` lines appended to a transcript took to come
    // as lines, with at most `cap` transcripts followed (0: none, the
    // folder's watch alone). None for a line not heard within a minute.
    // Every line is heard once at most.
    fn rounds(name: &str, cap: usize, rounds: usize) -> Vec<Option<Duration>> {
        use std::io::Write as _;
        use std::time::Instant;
        let d = std::env::temp_dir().join(format!("xenonet-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let d = PathBuf::from(xenonet_os::fs::realpath_native(&d.to_string_lossy()).unwrap());
        let (tx, rx) = channel();
        let p: Arc<dyn Platform> = Arc::new(Lines(app("claude").unwrap().clone()));
        let s = Sessions::start_with(cap, vec![(p, vec![WatchSpec::tree("t", &d, &["*.jsonl"])])], move |r| {
            let _ = tx.send(r);
        })
        .unwrap();
        let f = d.join("live.jsonl");
        let append = |text: &str| std::fs::OpenOptions::new().create(true).append(true).open(&f).unwrap().write_all(text.as_bytes()).unwrap();
        let mut seen: Vec<String> = Vec::new();
        let take = |r: Report, seen: &mut Vec<String>| {
            if let Report::Events { events, .. } = r {
                for e in events {
                    if let SessionEvent::Lines(l) = e {
                        seen.extend(l.lines);
                    }
                }
            }
        };
        // The first line is heard through the folder, and follows the file.
        // It's handed to the watcher rather than waited for from FSEvents,
        // which a busy Mac can hold back; the lines after it come through
        // the file's own watch (kqueue), or inotify, as they're timed.
        append("{\"r\":\"first\"}\n");
        s.watcher().inject(std::slice::from_ref(&f));
        heard(&rx, |r| matches!(r, Report::Events { .. }), || {}).expect("the first line");
        assert_eq!(s.watcher().followed().contains(&f), cap > 0 && cfg!(target_os = "macos"), "a transcript written to is followed where that's quicker");
        let mut took = Vec::new();
        for i in 0..rounds {
            let line = format!("{{\"r\":{i}}}");
            let start = Instant::now();
            append(&format!("{line}\n"));
            let end = start + Duration::from_secs(60);
            let mut got = None;
            while got.is_none() {
                match rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
                    Ok(r) => {
                        take(r, &mut seen);
                        if seen.contains(&line) {
                            got = Some(start.elapsed());
                        }
                    }
                    Err(_) => break,
                }
            }
            took.push(got);
        }
        // Late events from the other watch read nothing again.
        std::thread::sleep(Duration::from_millis(500));
        while let Ok(r) = rx.try_recv() {
            take(r, &mut seen);
        }
        for i in 0..rounds {
            let line = format!("{{\"r\":{i}}}");
            assert!(seen.iter().filter(|l| **l == line).count() <= 1, "{line} read twice");
        }
        drop(s);
        std::fs::remove_dir_all(&d).unwrap();
        took
    }

    // The median, the 95th percentile and the slowest, in ms, and how many
    // weren't heard.
    fn summary(took: &[Option<Duration>]) -> String {
        let mut ms: Vec<u128> = took.iter().flatten().map(Duration::as_millis).collect();
        ms.sort_unstable();
        let at = |q: f64| ms.get(((ms.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0);
        format!("{} rounds: median {} ms, p95 {} ms, max {} ms, missed {}", took.len(), at(0.5), at(0.95), ms.last().copied().unwrap_or(0), took.len() - ms.len())
    }

    #[test]
    fn a_followed_transcripts_lines_come_quickly() {
        let took = rounds("quick", crate::watch::follow::cap(), 20);
        // Under load the time varies, so it's told rather than held to a
        // bound; every line still has to come.
        eprintln!("followed transcript, {}", summary(&took));
        assert!(took.iter().all(Option::is_some), "every line comes");
    }

    // The time a line takes with the transcript followed against with its
    // folder's watch alone: `cargo test -p xenonet-adapter -- --ignored
    // followed_against --nocapture`.
    #[test]
    #[ignore = "a measurement, minutes long where FSEvents lags"]
    fn followed_against_the_folder_alone() {
        let n = 30;
        eprintln!("folder alone: {}", summary(&rounds("folder", 0, n)));
        eprintln!("followed:     {}", summary(&rounds("followed", crate::watch::follow::cap(), n)));
    }
}
