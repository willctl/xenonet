//! core/git.js's runtime: whether git is there, a repo's status through
//! git (xenonet-sources' git.rs says what to run and what each answer
//! means), and the watch that reads every repo's status, three at a time,
//! each a gatherer that keeps what it had when git can't answer.

use crate::blocks::{health_of, Block};
use crate::gatherer::{Gather, Options};
use crate::util::{lock, now_ms, num};
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
        let found = xenonet_base::paths::on_path("git", &path, std::env::var("PATHEXT").ok().as_deref(), cfg!(windows), &mut xenonet_base::paths::can_run);
        let mut ok = found.is_some();
        if git::needs_dev_tools(crate::config::platform(), found.as_deref()) {
            let tools = std::process::Command::new("/usr/bin/xcode-select")
                .arg("-p")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !tools.is_ok_and(|s| s.success()) {
                ok = false;
                crate::util::log("warn", "git.no-dev-tools", git::NO_DEV_TOOLS.trim_start_matches("[xenonet] "));
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

/// git with `args`, within `timeout`, in `env` added to this one's.
pub async fn run_git(args: &[String], env: &[(String, String)], clear: bool, timeout: Duration) -> Ran {
    #[cfg(test)]
    tests::RAN.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(args.join(" "));
    let mut cmd = Command::new("git");
    xenonet_base::process::Background::no_window(cmd.as_std_mut());
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
async fn git_in(dir: &str, args: Vec<String>) -> Ran {
    let env = [("GIT_NO_LAZY_FETCH".to_string(), "1".to_string())];
    let mut all = vec!["-C".to_string(), dir.to_string()];
    all.extend(args);
    run_git(&all, &env, false, Duration::from_secs(8)).await
}

/// The arguments a status in `dir` runs with: the filter drivers its
/// config lists turned off, so a status runs nothing of the repo's. None
/// when git can't list them, or one can't be turned off.
pub async fn status_args(dir: &str) -> Option<Vec<String>> {
    let strings = |a: Vec<&str>| a.into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut scoped = true;
    let mut drivers = git_in(dir, strings(git::drivers_args(true))).await;
    loop {
        let code = drivers.code.map(f64::from);
        match git::after_drivers(code, !drivers.ok, &drivers.stdout, scoped) {
            git::AfterDrivers::Unscoped => {
                scoped = false;
                drivers = git_in(dir, strings(git::drivers_args(false))).await;
            }
            git::AfterDrivers::Nothing => return None,
            git::AfterDrivers::Status(args) => return Some(args),
        }
    }
}

/// A repo's status with these arguments, or None when git can't give it.
async fn status_with(dir: &str, args: Vec<String>) -> Option<Value> {
    let out = git_in(dir, args).await;
    if !out.ok {
        return None;
    }
    let mut v = status_json(&git::parse_status(&out.stdout));
    v["checkedAt"] = num(now_ms());
    Some(v)
}

/// A repo's status, or None when git can't give it: the filter drivers its
/// config lists turned off first, so a status runs nothing of the repo's.
pub async fn git_status(dir: &str) -> Option<Value> {
    status_with(dir, status_args(dir).await?).await
}

/// What the watch keeps of a repo between passes, so it runs git only when
/// something may have changed (xenonet-sources' git.rs): when its status
/// was last read and its stamp then, and the arguments its drivers came to,
/// with the config stamp they were read under.
#[derive(Default)]
struct Seen {
    status: Option<(String, f64)>,
    drivers: Option<(String, f64, Vec<String>)>,
}

struct Inner {
    life: Lifecycle,
    /// project key → its status, as the last pass read it.
    status: Vec<(String, Value)>,
    /// project key → (the project's folder, its status's gatherer).
    sources: HashMap<String, (String, Arc<Gather>)>,
    /// project key → what's kept of it between passes.
    seen: HashMap<String, Seen>,
}

/// The watch: every git project's status, refreshed a few at a time.
pub struct RepoWatch {
    inner: Mutex<Inner>,
}

impl Default for RepoWatch {
    fn default() -> Self {
        RepoWatch { inner: Mutex::new(Inner { life: Lifecycle::new(now_ms()), status: Vec::new(), sources: HashMap::new(), seen: HashMap::new() }) }
    }
}

impl RepoWatch {
    /// Each repo's status, by project key, in the order they were read.
    pub fn status(&self) -> Vec<(String, Value)> {
        lock(&self.inner).status.clone()
    }

    pub fn status_of(&self, key: &str) -> Option<Value> {
        lock(&self.inner).status.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    /// One pass over the projects that are repos. A repo's status is read
    /// only when its stamps say it may have changed, or it's been idle for
    /// a while (git::status_due), and its drivers only when its config may
    /// have (git::drivers_fresh); otherwise what was read stands.
    pub async fn refresh(self: &Arc<Self>, projects: &[Value]) {
        if !git_available() {
            return;
        }
        let home = xenonet_service::config::home_dir(&|k| std::env::var(k).ok());
        let xdg = std::env::var("XDG_CONFIG_HOME").ok();
        self.refresh_with(projects, move |dir| git::repo_stamps(dir, &home, xdg.as_deref()), now_ms).await;
    }

    /// The pass, with the stamps and the clock given (a test's own).
    pub async fn refresh_with(self: &Arc<Self>, projects: &[Value], stamps: impl Fn(&str) -> Option<git::Stamps> + Send + Sync + 'static, now: fn() -> f64) {
        let stamps = Arc::new(stamps);
        let queue: Vec<(String, String)> = projects
            .iter()
            .filter(|p| xenonet_base::text::truthy(p.get("git")))
            .map(|p| (crate::util::s(p, "key").unwrap_or_default().to_string(), crate::util::s(p, "path").unwrap_or_default().to_string()))
            .collect();
        {
            let mut g = lock(&self.inner);
            g.status.retain(|(k, _)| queue.iter().any(|(q, _)| q == k));
            g.sources.retain(|k, _| queue.iter().any(|(q, _)| q == k));
            g.seen.retain(|k, _| queue.iter().any(|(q, _)| q == k));
            for (key, path) in &queue {
                let entry = g.sources.entry(key.clone()).or_insert_with(|| (path.clone(), Arc::new(Gather::new("git-status:repo", Options { keep: true, ..Default::default() }))));
                if entry.0 != *path {
                    // Another folder under the same key: nothing kept of the
                    // old one stands for it.
                    entry.0.clone_from(path);
                    g.seen.remove(key);
                }
            }
        }
        type Queue = Arc<Mutex<std::collections::VecDeque<(String, String)>>>;
        let queue: Queue = Arc::new(Mutex::new(std::collections::VecDeque::from(queue)));
        let failed = Arc::new(Mutex::new(Vec::<String>::new()));
        type Stamper = Arc<dyn Fn(&str) -> Option<git::Stamps> + Send + Sync>;
        let worker = |me: Arc<Self>, queue: Queue, failed: Arc<Mutex<Vec<String>>>, stamps: Stamper| async move {
            loop {
                let Some((key, path)) = lock(&queue).pop_front() else { break };
                let Some(source) = lock(&me.inner).sources.get(&key).map(|(_, s)| s.clone()) else { continue };
                let p = path.clone();
                let st = crate::util::blocking({
                    let stamps = stamps.clone();
                    let p = p.clone();
                    move || stamps(&p)
                })
                .await;
                let at = now();
                let (due, cached) = {
                    let g = lock(&me.inner);
                    let seen = g.seen.get(&key);
                    let last = seen.and_then(|s| s.status.as_ref()).map(|(stamp, t)| (stamp.as_str(), *t));
                    let due = !g.status.iter().any(|(k, _)| *k == key) || git::status_due(last, st.as_ref().map(|s| s.status.as_str()), at);
                    let drivers = seen.and_then(|s| s.drivers.as_ref());
                    let fresh = git::drivers_fresh(drivers.map(|(stamp, t, _)| (stamp.as_str(), *t)), st.as_ref().map(|s| s.config.as_str()), at);
                    (due, drivers.filter(|_| fresh).map(|(_, _, args)| args.clone()))
                };
                if !due {
                    continue;
                }
                let (me2, key2, config) = (me.clone(), key.clone(), st.as_ref().map(|s| s.config.clone()));
                let value = source
                    .run(false, None, || async move {
                        let args = match cached {
                            Some(args) => args,
                            None => {
                                let args = status_args(&p).await.ok_or_else(|| git::unreadable(&p))?;
                                if let Some(config) = config {
                                    lock(&me2.inner).seen.entry(key2).or_default().drivers = Some((config, at, args.clone()));
                                }
                                args
                            }
                        };
                        status_with(&p, args).await.ok_or_else(|| git::unreadable(&p))
                    })
                    .await;
                match source.last() {
                    Some(l) if l.ok => {
                        let mut g = lock(&me.inner);
                        if let Some(st) = &st {
                            g.seen.entry(key.clone()).or_default().status = Some((st.status.clone(), at));
                        }
                        match g.status.iter_mut().find(|(k, _)| *k == key) {
                            Some(slot) => slot.1 = value,
                            None => g.status.push((key, value)),
                        }
                    }
                    Some(l) => {
                        // Read it all again next pass.
                        lock(&me.inner).seen.remove(&key);
                        lock(&failed).push(l.error.unwrap_or_default());
                    }
                    None => {}
                }
            }
        };
        tokio::join!(
            worker(self.clone(), queue.clone(), failed.clone(), stamps.clone()),
            worker(self.clone(), queue.clone(), failed.clone(), stamps.clone()),
            worker(self.clone(), queue, failed.clone(), stamps)
        );
        let failed = lock(&failed).clone();
        let mut g = lock(&self.inner);
        g.life.count("passes", 1.0);
        match git::problem_of(&failed) {
            Some(p) => g.life.problem(&p, now_ms()),
            None => g.life.ok(now_ms()),
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
    static CLOCK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    fn clock() -> f64 {
        CLOCK.load(std::sync::atomic::Ordering::SeqCst) as f64
    }

    #[tokio::test]
    async fn git_runs_only_when_a_repo_may_have_changed() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("xenonet-wire-git-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        let git = |args: &[&str]| std::process::Command::new("git").args(xenonet_app::demo::GIT).args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);
        let home = dir.join("home").to_string_lossy().into_owned();
        let watch = Arc::new(RepoWatch::default());
        let projects = [json!({ "key": "/seen", "path": d, "git": { "origin": null } })];
        let mine = || RAN.lock().unwrap().iter().filter(|a| a.contains(d.as_str())).count();
        let pass = |ms: u64| {
            CLOCK.store(ms, std::sync::atomic::Ordering::SeqCst);
            let (watch, home, projects) = (watch.clone(), home.clone(), projects.clone());
            async move {
                let before = mine();
                watch.refresh_with(&projects, move |p| git::repo_stamps(p, &home, None), clock).await;
                mine() - before
            }
        };
        assert_eq!(pass(0).await, 2, "its drivers, then its status");
        assert_eq!(pass(20_000).await, 0, "nothing changed");
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        git(&["add", "b.txt"]);
        assert_eq!(pass(40_000).await, 1, "staged: its status again, its drivers as they were");
        assert_eq!(watch.status_of("/seen").unwrap()["staged"], 1);
        let later = std::time::SystemTime::now() + Duration::from_secs(5);
        std::fs::File::options().write(true).open(dir.join(".git").join("config")).unwrap().set_modified(later).unwrap();
        assert_eq!(pass(60_000).await, 2, "its config changed: both");
        assert_eq!(pass(60_000 + git::STATUS_IDLE_MS as u64).await, 1, "idle long enough: its status, for edits git alone sees");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_repo_and_its_uncommitted_work() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("xenonet-wire-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        let git = |args: &[&str]| std::process::Command::new("git").args(xenonet_app::demo::GIT).args(args).current_dir(&dir).output().unwrap();
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "first"]);
        std::fs::write(dir.join("a.txt"), "b").unwrap();
        let s = git_status(&d).await.unwrap();
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
