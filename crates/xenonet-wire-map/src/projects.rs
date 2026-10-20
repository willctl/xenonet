//! core/projects.js's scan, run: the scanner block's breadth-first walk
//! of the projects folder (xenonet-blocks' scanner.rs), each folder it
//! reads handed to xenonet-state's projects.rs, which decides what each is
//! and which subfolders to enter, then finds the rest from where sessions
//! ran. The walk reads the disk off the runtime's threads.

use crate::blocks::{health_of, Block};
use crate::util::{blocking, lock, now_ms, num};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use xenonet_base::fs::{DirEntry, Fs, StdFs};
use xenonet_blocks::scanner::{Limits, Rules, Scanner};
use xenonet_blocks::Halt;
use xenonet_state::projects::{FactsCache, Scan, SCAN_DEPTH, SCAN_MAX_DIRS, SCAN_MAX_SUBMODULES};

/// What a scan found: the projects, the scan's status, and the folders
/// it looked at (for Codex's outline).
#[derive(Clone, Debug, Default)]
pub struct Scanned {
    pub projects: Vec<Value>,
    pub scan: Value,
    pub folders: Vec<Value>,
}

struct Enter<'a>(&'a Scan);

impl Rules for Enter<'_> {
    fn pick(&mut self, _: &str, _: &str) -> Result<bool, Halt> {
        Ok(false)
    }
    fn enter(&mut self, name: &str, dir: &str, level: u32) -> Result<bool, Halt> {
        Ok(self.0.enter(name, dir, f64::from(level)))
    }
}

/// The scans' shared parts: the "projects" block's walk, and what the
/// bottom-up pass remembers of folders for a while.
pub struct Projects {
    scanner: Mutex<Scanner>,
    facts: Mutex<FactsCache>,
}

impl Default for Projects {
    fn default() -> Self {
        Projects { scanner: Mutex::new(Scanner::new("projects", "", Limits::default(), false, now_ms())), facts: Mutex::new(FactsCache::default()) }
    }
}

fn limit(limits: &Value, k: &str, fallback: f64) -> f64 {
    limits.get(k).and_then(Value::as_f64).unwrap_or(fallback)
}

impl Projects {
    /// "Detect projects again": nothing the bottom-up pass kept.
    pub fn forget(&self) {
        lock(&self.facts).clear();
    }

    /// Finds the projects under `root`, top-down then bottom-up.
    /// `own_walk`: a walk of its own rather than the map's block (a
    /// check of another folder, Codex's outline). `stop`: set when the scan
    /// is no longer wanted.
    pub async fn scan(
        self: &Arc<Self>,
        root: &str,
        session_dirs: Vec<Value>,
        rules: Value,
        limits: Value,
        own_walk: bool,
        stop: Option<Arc<AtomicBool>>,
    ) -> Result<Scanned, String> {
        let me = self.clone();
        let root = root.to_string();
        blocking(move || me.scan_now(&root, &session_dirs, &rules, &limits, own_walk, stop.as_deref())).await
    }

    fn scan_now(&self, root: &str, session_dirs: &[Value], rules: &Value, limits: &Value, own_walk: bool, stop: Option<&AtomicBool>) -> Result<Scanned, String> {
        let depth = limit(limits, "depth", SCAN_DEPTH);
        let max_dirs = limit(limits, "maxDirs", SCAN_MAX_DIRS);
        let max_submodules = limits.get("maxSubmodules").cloned().unwrap_or(json!(SCAN_MAX_SUBMODULES));
        let started = now_ms();
        let mut scan = Scan::new(root, rules);
        let mut own;
        let mut shared;
        let scanner: &mut Scanner = if own_walk {
            own = Scanner::new("projects", root, Limits::default(), false, started);
            &mut own
        } else {
            shared = lock(&self.scanner);
            &mut shared
        };
        let aborted = || stop.is_some_and(|s| s.load(Ordering::SeqCst));
        let mut now = now_ms;
        let mut w = scanner.begin_breadth(root, Limits { depth, folders: max_dirs, ms: f64::INFINITY }, &mut now);
        while w.more() {
            let level = w.n;
            let reads = scanner.plan(&mut w, None);
            for read in reads {
                let Some(at) = scanner.begin_read(&mut w, aborted(), &mut now) else {
                    scanner.skip(&mut w, read.slot);
                    continue;
                };
                let ents: Vec<DirEntry> = match StdFs.read_dir(&read.dir) {
                    Ok(e) => e,
                    Err(e) => {
                        if !scanner.read_failed(&mut w, &read.dir, &e) {
                            scanner.skip(&mut w, read.slot);
                            continue;
                        }
                        Vec::new()
                    }
                };
                let names: Vec<(String, bool)> = ents.iter().map(|e| (e.name.clone(), e.is_dir)).collect();
                scan.look(&read.dir, &names, f64::from(level), aborted());
                let _ = scanner.finish_read(&mut w, &read, at, &ents, &mut Enter(&scan));
            }
            scanner.end_level(&mut w);
        }
        scanner.finish_breadth(w, &mut now);
        if aborted() {
            return Err("This operation was aborted".into());
        }
        if !scan.root_read {
            return Ok(Scanned { projects: Vec::new(), scan: json!({ "ok": false, "ms": num(now_ms() - started) }), folders: Vec::new() });
        }
        let last = scanner.last.clone();
        let (dirs_read, truncated) = last.map_or((0, false), |l| (l.read, l.truncated.is_some()));
        let found = scan.finish(session_dirs, Some(&max_submodules), &mut lock(&self.facts), now_ms()).map_err(|e| e.0)?;
        let at = now_ms();
        Ok(Scanned {
            projects: found.projects,
            scan: json!({ "ok": true, "at": num(at), "ms": num(at - started), "dirsRead": dirs_read, "truncated": truncated, "depth": num(depth), "maxDirs": num(max_dirs), "groups": found.groups, "projects": found.count }),
            folders: found.folders,
        })
    }
}

impl Block for Projects {
    fn name(&self) -> String {
        "projects".into()
    }
    fn health(&self) -> Value {
        let s = lock(&self.scanner);
        let b = s.budget;
        health_of("projects", "scanner", &s.life, &[], json!({ "depth": num(b.depth), "folders": num(b.folders), "ms": num(b.ms) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn repos_groups_and_a_folder_that_isnt_there() {
        let root = std::env::temp_dir().join(format!("xenonet-wire-projects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for f in ["api/.git/HEAD", "api/package.json", "labs/geo/.git/HEAD", "labs/sim/.git/HEAD", "notes/todo.md"] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "ref: refs/heads/main\n").unwrap();
        }
        let p = Arc::new(Projects::default());
        let r = root.to_string_lossy().into_owned();
        let found = p.scan(&r, vec![], json!({}), json!({ "depth": 3, "maxDirs": 2000 }), false, None).await.unwrap();
        assert_eq!(found.scan["ok"], true);
        let names: Vec<&str> = found.projects.iter().filter_map(|p| p["name"].as_str()).collect();
        assert!(names.contains(&"api") && names.contains(&"geo"), "{names:?}");
        assert!(found.scan["groups"].as_u64().unwrap() >= 1);
        assert_eq!(p.health()["state"], "running");
        let gone = p.scan("/nowhere/at/all", vec![], json!({}), json!({}), true, None).await.unwrap();
        assert_eq!(gone.scan["ok"], false);
        let stop = Arc::new(AtomicBool::new(true));
        assert!(p.scan(&r, vec![], json!({}), json!({}), true, Some(stop)).await.is_err(), "no longer wanted");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
