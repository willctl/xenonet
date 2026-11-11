//! Git's runtime: whether git is there, a repo's status through
//! git (xenonet-sources' git.rs says what to run and what each answer
//! means), and the watch that reads every repo's status (all of them three
//! at a time in a pass, one when its watch says it changed), each a
//! gatherer that keeps what it had when git can't answer. Git runs
//! through a `GitHost`: this machine's git, or what a test answers in its
//! place.

use crate::runtime::blocks::{health_of, Block};
use crate::runtime::gatherer::{Gather, Options};
use crate::runtime::util::{lock, num, Clock};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::process::Command;
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_sources::git;

/// Whether git can run: on PATH, and on a Mac not the stub that asks to
/// install the Command Line Tools.
pub fn git_available() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| {
        let path = std::env::var("PATH").unwrap_or_default();
        let found = xenonet_os::paths::on_path("git", &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut xenonet_os::paths::can_run);
        let mut ok = found.is_some();
        if git::needs_dev_tools(crate::runtime::config::platform(), found.as_deref()) {
            let tools = std::process::Command::new("/usr/bin/xcode-select")
                .arg("-p")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !tools.is_ok_and(|s| s.success()) {
                ok = false;
                crate::runtime::util::log("warn", "git.no-dev-tools", git::NO_DEV_TOOLS.trim_start_matches("[xenonet] "));
            }
        }
        ok
    })
}

/// What a run of git gave: its exit code (None when it couldn't start or
/// was killed), and its output.
pub struct Ran {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
}

/// Where git runs: this machine's git (`SystemGit`), or a stand-in that
/// answers as git would, for a test whose repos are only their HEAD and
/// config.
#[async_trait::async_trait]
pub trait GitHost: Send + Sync {
    /// Whether git can run.
    fn available(&self) -> bool;
    /// git with `args`, within `timeout`, in `env` added to this one's (or
    /// in `env` alone, with `clear`).
    async fn run(&self, args: &[String], env: &[(String, String)], clear: bool, timeout: Duration) -> Ran;
}

/// This machine's git.
pub struct SystemGit;

#[async_trait::async_trait]
impl GitHost for SystemGit {
    fn available(&self) -> bool {
        git_available()
    }
    async fn run(&self, args: &[String], env: &[(String, String)], clear: bool, timeout: Duration) -> Ran {
        run_git(args, env, clear, timeout).await
    }
}

/// The git the map's runtime runs: this machine's unless a test gives
/// another.
#[derive(Clone)]
pub struct Git(pub Arc<dyn GitHost>);

impl Git {
    pub fn new(host: impl GitHost + 'static) -> Git {
        Git(Arc::new(host))
    }
}

impl Default for Git {
    fn default() -> Self {
        Git::new(SystemGit)
    }
}

impl std::fmt::Debug for Git {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Git")
    }
}

/// git with `args`, within `timeout`, in `env` added to this one's.
pub async fn run_git(args: &[String], env: &[(String, String)], clear: bool, timeout: Duration) -> Ran {
    #[cfg(test)]
    tests::RAN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(args.join(" "));
    let mut cmd = Command::new("git");
    xenonet_os::process::Background::no_window(cmd.as_std_mut());
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    if clear {
        cmd.env_clear();
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let Ok(child) = cmd.spawn() else { return Ran { ok: false, code: None, stdout: String::new() } };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ran { ok: out.status.success(), code: out.status.code(), stdout: String::from_utf8_lossy(&out.stdout).into_owned() },
        _ => Ran { ok: false, code: None, stdout: String::new() },
    }
}

fn status_json(s: &git::Status) -> Value {
    json!({ "branch": s.branch, "ahead": num(s.ahead), "behind": num(s.behind), "staged": num(s.staged), "modified": num(s.modified), "untracked": num(s.untracked), "dirty": num(s.dirty) })
}

// git in a repo, with what a status needs: no lazy fetches.
async fn git_in(git: &dyn GitHost, dir: &str, args: Vec<String>) -> Ran {
    let env = [("GIT_NO_LAZY_FETCH".to_string(), "1".to_string())];
    let mut all = vec!["-C".to_string(), dir.to_string()];
    all.extend(args);
    git.run(&all, &env, false, Duration::from_secs(8)).await
}

/// The arguments a status in `dir` runs with: the filter drivers its
/// config lists turned off, so a status runs nothing of the repo's. None
/// when git can't list them, or one can't be turned off.
pub async fn status_args(git: &dyn GitHost, dir: &str) -> Option<Vec<String>> {
    let strings = |a: Vec<&str>| a.into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut scoped = true;
    let mut drivers = git_in(git, dir, strings(git::drivers_args(true))).await;
    loop {
        let code = drivers.code.map(f64::from);
        match git::after_drivers(code, !drivers.ok, &drivers.stdout, scoped) {
            git::AfterDrivers::Unscoped => {
                scoped = false;
                drivers = git_in(git, dir, strings(git::drivers_args(false))).await;
            }
            git::AfterDrivers::Nothing => return None,
            git::AfterDrivers::Status(args) => return Some(args),
        }
    }
}

/// A repo's status with these arguments, read at `now`, or None when git
/// can't give it.
async fn status_with(git: &dyn GitHost, dir: &str, args: Vec<String>, now: f64) -> Option<Value> {
    let out = git_in(git, dir, args).await;
    if !out.ok {
        return None;
    }
    let mut v = status_json(&git::parse_status(&out.stdout));
    v["checkedAt"] = num(now);
    Some(v)
}

/// A repo's status, or None when git can't give it: the filter drivers its
/// config lists turned off first, so a status runs nothing of the repo's.
pub async fn git_status(git: &dyn GitHost, dir: &str, clock: &Clock) -> Option<Value> {
    let args = status_args(git, dir).await?;
    status_with(git, dir, args, clock.now()).await
}

struct Inner {
    life: Lifecycle,
    /// project key → its status, as it was last read.
    status: Vec<(String, Value)>,
    /// project key → (the project's folder, its status's gatherer).
    sources: HashMap<String, (String, Arc<Gather>)>,
    /// project key → the arguments its drivers came to, which stand until
    /// its config changes or the next pass.
    drivers: HashMap<String, Vec<String>>,
}

/// Every git project's status: all of them in a pass (at start, and the
/// reconcile's), and one when its watch says it may have changed
/// (`changed`).
pub struct RepoWatch {
    inner: Mutex<Inner>,
    clock: Clock,
    git: Git,
}

impl Default for RepoWatch {
    fn default() -> Self {
        RepoWatch::new(Clock::system(), Git::default())
    }
}

impl RepoWatch {
    /// A watch that reads `clock` and runs `git`.
    pub fn new(clock: Clock, git: Git) -> RepoWatch {
        let life = Lifecycle::new(clock.now());
        RepoWatch { inner: Mutex::new(Inner { life, status: Vec::new(), sources: HashMap::new(), drivers: HashMap::new() }), clock, git }
    }

    /// Each repo's status, by project key, in the order they were read.
    pub fn status(&self) -> Vec<(String, Value)> {
        lock(&self.inner).status.clone()
    }

    pub fn status_of(&self, key: &str) -> Option<Value> {
        lock(&self.inner).status.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// Whether a project is a repo the watch reads.
    pub fn knows(&self, key: &str) -> bool {
        lock(&self.inner).sources.contains_key(key)
    }

    /// One pass over the projects that are repos, three at a time: each
    /// one's drivers listed again, then its status read.
    pub async fn refresh(self: &Arc<Self>, projects: &[Value]) {
        if !self.git.0.available() {
            return;
        }
        let queue: Vec<(String, String)> = projects
            .iter()
            .filter(|p| xenonet_text::text::truthy(p.get("git")))
            .map(|p| (crate::runtime::util::s(p, "key").unwrap_or_default().to_string(), crate::runtime::util::s(p, "path").unwrap_or_default().to_string()))
            .collect();
        {
            let mut g = lock(&self.inner);
            g.status.retain(|(k, _)| queue.iter().any(|(q, _)| q == k));
            g.sources.retain(|k, _| queue.iter().any(|(q, _)| q == k));
            g.drivers.clear();
            for (key, path) in &queue {
                let clock = self.clock.clone();
                let entry =
                    g.sources.entry(key.clone()).or_insert_with(|| (path.clone(), Arc::new(Gather::new("git-status:repo", Options { keep: true, clock, ..Default::default() }))));
                entry.0.clone_from(path);
            }
        }
        type Queue = Arc<Mutex<std::collections::VecDeque<String>>>;
        let queue: Queue = Arc::new(Mutex::new(queue.into_iter().map(|(k, _)| k).collect()));
        let failed = Arc::new(Mutex::new(Vec::<String>::new()));
        let worker = |me: Arc<Self>, queue: Queue, failed: Arc<Mutex<Vec<String>>>| async move {
            loop {
                let Some(key) = lock(&queue).pop_front() else { break };
                if let Err(e) = me.read(&key).await {
                    lock(&failed).push(e);
                }
            }
        };
        tokio::join!(worker(self.clone(), queue.clone(), failed.clone()), worker(self.clone(), queue.clone(), failed.clone()), worker(self.clone(), queue, failed.clone()));
        let failed = lock(&failed).clone();
        let mut g = lock(&self.inner);
        g.life.count("passes", 1.0);
        let now = self.clock.now();
        match git::problem_of(&failed) {
            Some(p) => g.life.problem(&p, now),
            None => g.life.ok(now),
        }
    }

    /// One repo read again, as its watch says it may have changed:
    /// `config` when its config did, which lists its drivers again. Says
    /// whether its status changed; false for a project the last pass
    /// didn't read as a repo.
    pub async fn changed(self: &Arc<Self>, key: &str, config: bool) -> bool {
        if !self.git.0.available() || !self.knows(key) {
            return false;
        }
        if config {
            lock(&self.inner).drivers.remove(key);
        }
        let before = self.status_of(key);
        if let Err(e) = self.read(key).await {
            let mut g = lock(&self.inner);
            let now = self.clock.now();
            g.life.problem(&e, now);
        }
        self.status_of(key) != before
    }

    // A repo's status, with the drivers its config lists turned off (listed
    // again when they aren't kept). What was read stands when git can't
    // answer; the error says why.
    async fn read(self: &Arc<Self>, key: &str) -> Result<(), String> {
        let Some((path, source)) = lock(&self.inner).sources.get(key).cloned() else { return Ok(()) };
        let cached = lock(&self.inner).drivers.get(key).cloned();
        let (me, key2, p, git) = (self.clone(), key.to_string(), path.clone(), self.git.clone());
        let value = source
            .run(false, None, || async move {
                let args = match cached {
                    Some(args) => args,
                    None => {
                        let args = status_args(&*git.0, &p).await.ok_or_else(|| git::unreadable(&p))?;
                        lock(&me.inner).drivers.insert(key2, args.clone());
                        args
                    }
                };
                let now = me.clock.now();
                status_with(&*git.0, &p, args, now).await.ok_or_else(|| git::unreadable(&p))
            })
            .await;
        match source.last() {
            Some(l) if l.ok => {
                let mut g = lock(&self.inner);
                match g.status.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = value,
                    None => g.status.push((key.to_string(), value)),
                }
                Ok(())
            }
            Some(l) => {
                lock(&self.inner).drivers.remove(key);
                Err(l.error.unwrap_or_default())
            }
            None => Ok(()),
        }
    }
}

impl Block for RepoWatch {
    fn name(&self) -> String {
        "git-status".into()
    }
    fn health(&self) -> Value {
        let g = lock(&self.inner);
        let reads: f64 = g.sources.values().map(|(_, s)| lock(&s.state).runs()).sum();
        health_of("git-status", "gatherer", &g.life, &[("repos", g.status.len() as f64), ("reads", reads)], Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every git run, as its arguments.
    pub static RAN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    fn clock() -> f64 {
        0.0
    }

    #[tokio::test]
    async fn git_runs_for_a_pass_and_for_a_repo_that_changed() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("xenonet-state-git-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        let git = |args: &[&str]| std::process::Command::new("git").args(xenonet_demo::GIT).args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);
        let watch = Arc::new(RepoWatch::new(Clock::new(clock), Git::default()));
        let projects = [json!({ "key": "/seen", "path": d, "git": { "origin": null } })];
        let mine = || RAN.lock().unwrap().iter().filter(|a| a.contains(d.as_str())).count();
        let before = mine();
        watch.refresh(&projects).await;
        assert_eq!(mine() - before, 2, "its drivers, then its status");
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        git(&["add", "b.txt"]);
        let before = mine();
        assert!(watch.changed("/seen", false).await, "staged: its status changed");
        assert_eq!(mine() - before, 1, "its status, its drivers as they were");
        assert_eq!(watch.status_of("/seen").unwrap()["staged"], 1);
        let before = mine();
        assert!(!watch.changed("/seen", true).await, "nothing new");
        assert_eq!(mine() - before, 2, "its config changed: both");
        assert!(!watch.changed("/elsewhere", false).await, "not a repo it read");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_repo_and_its_uncommitted_work() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("xenonet-state-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        let git = |args: &[&str]| std::process::Command::new("git").args(xenonet_demo::GIT).args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "b").unwrap();
        let s = git_status(&SystemGit, &d, &Clock::system()).await.unwrap();
        assert_eq!((s["branch"].as_str(), s["dirty"].as_f64()), (Some("main"), Some(1.0)));
        let watch = Arc::new(RepoWatch::default());
        watch.refresh(&[json!({ "key": "/r", "path": d, "git": { "origin": null } }), json!({ "key": "/x", "path": "/nowhere/at/all", "git": {} })]).await;
        assert_eq!(watch.status_of("/r").unwrap()["dirty"], 1);
        let h = watch.health();
        assert_eq!(h["state"], "degraded", "one repo it couldn't read");
        assert!(h["lastError"]["message"].as_str().unwrap().contains("/nowhere/at/all"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
