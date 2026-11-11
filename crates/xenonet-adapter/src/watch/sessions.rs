//! The registry's platforms on one watcher: each platform's changed paths
//! read into session events through its own `changed` and cursor, and the
//! paths of any other owner (a repo, added under a key of its own) passed
//! on as they are, so one set of the system's watches serves all of them.

use super::runner::{Batch, Watcher};
use super::{Cursor, SessionEvent, WatchSpec};
use crate::Platform;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
        let apps: Apps = Arc::new(Mutex::new(HashMap::new()));
        let them = apps.clone();
        let mut on = on;
        let watcher = Watcher::start(move |b| {
            for r in handle(&them, b) {
                on(r);
            }
        })?;
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
        if let Some((_, c)) = lock(&self.apps).get_mut(&app_key(app)) {
            c.seed(file, at);
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

fn handle(apps: &Apps, b: Batch) -> Vec<Report> {
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
        std::thread::sleep(Duration::from_millis(300));
        let f = d.join("s1.jsonl");
        let write = || std::fs::write(&f, "{\"n\":1}\n{\"n\":2}\n").unwrap();
        write();
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
            write,
        );
        assert_eq!(lines, ["{\"n\":1}", "{\"n\":2}"]);
        drop(s);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
