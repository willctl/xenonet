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
    let mut cmd = Command::new("git");
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

/// A repo's status, or None when git can't give it: the filter drivers its
/// config lists turned off first, so a status runs nothing of the repo's.
pub async fn git_status(dir: &str) -> Option<Value> {
    let env = [("GIT_NO_LAZY_FETCH".to_string(), "1".to_string())];
    let run = |args: Vec<String>| {
        let mut all = vec!["-C".to_string(), dir.to_string()];
        all.extend(args);
        let env = env.clone();
        async move { run_git(&all, &env, false, Duration::from_secs(8)).await }
    };
    let strings = |a: Vec<&str>| a.into_iter().map(str::to_string).collect::<Vec<_>>();
    let mut scoped = true;
    let mut drivers = run(strings(git::drivers_args(true))).await;
    loop {
        let code = drivers.code.map(f64::from);
        match git::after_drivers(code, !drivers.ok, &drivers.stdout, scoped) {
            git::AfterDrivers::Unscoped => {
                scoped = false;
                drivers = run(strings(git::drivers_args(false))).await;
            }
            git::AfterDrivers::Nothing => return None,
            git::AfterDrivers::Status(args) => {
                let out = run(args).await;
                if !out.ok {
                    return None;
                }
                let mut v = status_json(&git::parse_status(&out.stdout));
                v["checkedAt"] = num(now_ms());
                return Some(v);
            }
        }
    }
}

struct Inner {
    life: Lifecycle,
    /// project key → its status, as the last pass read it.
    status: Vec<(String, Value)>,
    /// project key → (the project's folder, its status's gatherer).
    sources: HashMap<String, (String, Arc<Gather>)>,
}

/// The watch: every git project's status, refreshed a few at a time.
pub struct RepoWatch {
    inner: Mutex<Inner>,
}

impl Default for RepoWatch {
    fn default() -> Self {
        RepoWatch { inner: Mutex::new(Inner { life: Lifecycle::new(now_ms()), status: Vec::new(), sources: HashMap::new() }) }
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

    /// One pass over the projects that are repos.
    pub async fn refresh(self: &Arc<Self>, projects: &[Value]) {
        if !git_available() {
            return;
        }
        let queue: Vec<(String, String)> = projects
            .iter()
            .filter(|p| xenonet_base::text::truthy(p.get("git")))
            .map(|p| (crate::util::s(p, "key").unwrap_or_default().to_string(), crate::util::s(p, "path").unwrap_or_default().to_string()))
            .collect();
        {
            let mut g = lock(&self.inner);
            g.status.retain(|(k, _)| queue.iter().any(|(q, _)| q == k));
            g.sources.retain(|k, _| queue.iter().any(|(q, _)| q == k));
            for (key, path) in &queue {
                let entry = g.sources.entry(key.clone()).or_insert_with(|| (path.clone(), Arc::new(Gather::new("git-status:repo", Options { keep: true, ..Default::default() }))));
                entry.0 = path.clone();
            }
        }
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(queue)));
        let failed = Arc::new(Mutex::new(Vec::<String>::new()));
        let worker = |me: Arc<Self>, queue: Arc<Mutex<std::collections::VecDeque<(String, String)>>>, failed: Arc<Mutex<Vec<String>>>| async move {
            loop {
                let Some((key, path)) = lock(&queue).pop_front() else { break };
                let Some(source) = lock(&me.inner).sources.get(&key).map(|(_, s)| s.clone()) else { continue };
                let p = path.clone();
                let value = source.run(false, None, || async move { git_status(&p).await.ok_or_else(|| git::unreadable(&p)) }).await;
                match source.last() {
                    Some(l) if l.ok => {
                        let mut g = lock(&me.inner);
                        match g.status.iter_mut().find(|(k, _)| *k == key) {
                            Some(slot) => slot.1 = value,
                            None => g.status.push((key, value)),
                        }
                    }
                    Some(l) => lock(&failed).push(l.error.unwrap_or_default()),
                    None => {}
                }
            }
        };
        tokio::join!(worker(self.clone(), queue.clone(), failed.clone()), worker(self.clone(), queue.clone(), failed.clone()), worker(self.clone(), queue, failed.clone()));
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
