//! Against real repos, made in a temp folder with the git command, which
//! reads none of this machine's git config (no signing, no hooks).

use super::*;
use std::process::{Command, Stdio};

struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(name: &str) -> Temp {
    let d = std::env::temp_dir().join(format!("xenonet-git-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // The real path, as git writes it into a worktree's files.
    Temp(PathBuf::from(xenonet_os::fs::realpath_native(&d.to_string_lossy()).unwrap()))
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=XenoNet Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "protocol.file.allow=always",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo_with_commit(dir: &Path) -> String {
    std::fs::create_dir_all(dir).unwrap();
    git_in(dir, &["init", "-q"]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git_in(dir, &["add", "a.txt"]);
    git_in(dir, &["commit", "-q", "-m", "first"]);
    git_in(dir, &["rev-parse", "HEAD"])
}

#[test]
fn finds_the_checkout_a_path_is_in() {
    let t = temp("find");
    let app = t.0.join("app");
    let head = repo_with_commit(&app);
    std::fs::create_dir_all(app.join("src/deep")).unwrap();
    assert_eq!(root(&app.join("src/deep")), Some(app.clone()));
    assert_eq!(root(&app.join("src/deep/not-yet.rs")), Some(app.clone()), "a file that isn't there yet is in it too");
    let r = find(&app.join("src")).unwrap();
    assert_eq!((r.top.as_path(), r.git_dir.as_path(), r.common_dir.as_path(), &r.kind), (app.as_path(), app.join(".git").as_path(), app.join(".git").as_path(), &Kind::Main));
    assert_eq!(r.head(), Some(Head::Branch { name: "main".into(), commit: Some(head.clone()) }));
    assert_eq!((r.branch().as_deref(), r.commit()), (Some("main"), Some(head)));
    assert!(!r.is_worktree() && r.parent().is_none());
    assert_eq!(find(&t.0), None, "outside any repo");
}

#[test]
fn reads_branches_detached_heads_and_packed_refs() {
    let t = temp("head");
    let app = t.0.join("app");
    let first = repo_with_commit(&app);
    git_in(&app, &["checkout", "-q", "-b", "feat/refunds"]);
    let r = open(&app).unwrap();
    assert_eq!(r.branch().as_deref(), Some("feat/refunds"));
    git_in(&app, &["checkout", "-q", "--detach"]);
    let h = r.head().unwrap();
    assert_eq!(h, Head::Detached { commit: first.clone() });
    assert_eq!((h.branch(), h.label()), (None, first[..12].to_string()));
    git_in(&app, &["checkout", "-q", "main"]);
    git_in(&app, &["pack-refs", "--all"]);
    assert!(!app.join(".git/refs/heads/main").exists(), "the ref is packed now");
    assert_eq!(r.commit(), Some(first));
    // A branch before its first commit.
    git_in(&app, &["checkout", "-q", "--orphan", "fresh"]);
    assert_eq!(r.head(), Some(Head::Branch { name: "fresh".into(), commit: None }));
}

#[test]
fn keys_a_repo_by_its_origin_as_project_ids_are_made() {
    let t = temp("key");
    let app = t.0.join("app");
    repo_with_commit(&app);
    let r = open(&app).unwrap();
    assert_eq!((r.origin(), r.key()), (None, None));
    git_in(&app, &["remote", "add", "upstream", "https://github.com/other/app.git"]);
    assert_eq!((r.origin(), r.key()), (None, None), "only origin names the repo");
    git_in(&app, &["remote", "add", "origin", "git@github.com:Willctl/XenoNet.git"]);
    assert_eq!(r.remotes().len(), 2);
    assert_eq!(r.key().as_deref(), Some("github.com/willctl/xenonet"));
    assert_eq!(r.key(), Some(xenonet_model::ids::normalize_remote("git@github.com:Willctl/XenoNet.git")), "the normalization project ids are made with");
    assert_eq!(repo_key("https://user@github.com/willctl/xenonet"), Some("github.com/willctl/xenonet".into()));
    assert_eq!(repo_key(""), None);
}

#[test]
fn reads_a_config_as_git_writes_it() {
    let cfg = "[core]\n\tbare = false\n[remote \"origin\"]\n\tpushurl = git@example.com:me/push.git\n\turl = \"git@example.com:me/app.git\" # mine\n\turl = git@mirror:me/app.git\n[remote.old]\n\turl=https://old.example/app\n[branch \"main\"]\n\tremote = origin\n";
    assert_eq!(remotes(cfg), [Remote { name: "origin".into(), url: "git@example.com:me/app.git".into() }, Remote { name: "old".into(), url: "https://old.example/app".into() }]);
}

#[test]
fn tells_a_worktree_from_its_main_checkout() {
    let t = temp("worktree");
    let app = t.0.join("app");
    let head = repo_with_commit(&app);
    let wt = t.0.join("app-wt");
    git_in(&app, &["worktree", "add", "-q", "-b", "side", &wt.to_string_lossy()]);
    let r = find(&wt).unwrap();
    assert_eq!(r.kind, Kind::Worktree { main: app.clone() });
    assert!(r.is_worktree());
    assert_eq!(r.parent(), Some(app.as_path()));
    assert_eq!(r.common_dir, app.join(".git"));
    assert_eq!(r.git_dir, app.join(".git/worktrees/app-wt"));
    assert_eq!(r.head(), Some(Head::Branch { name: "side".into(), commit: Some(head) }));
    // The worktree reads its repo's remotes.
    git_in(&app, &["remote", "add", "origin", "https://github.com/me/app"]);
    assert_eq!(r.key().as_deref(), Some("github.com/me/app"));
    assert_eq!(open(&app).unwrap().kind, Kind::Main);
}

#[test]
fn tells_a_submodule_from_its_superproject() {
    let t = temp("submodule");
    let lib = t.0.join("lib");
    repo_with_commit(&lib);
    let game = t.0.join("game");
    repo_with_commit(&game);
    git_in(&game, &["submodule", "add", "-q", &lib.to_string_lossy(), "vendor/lib"]);
    let r = find(&game.join("vendor/lib")).unwrap();
    assert_eq!(r.top, game.join("vendor/lib"));
    assert_eq!(r.kind, Kind::Submodule { superproject: game.clone() });
    assert_eq!(r.git_dir, game.join(".git/modules/vendor/lib"));
    assert_eq!(r.parent(), Some(game.as_path()));
}

#[test]
fn watches_head_and_refs_never_the_index() {
    let t = temp("specs");
    let app = t.0.join("app");
    repo_with_commit(&app);
    let wt = t.0.join("app-wt");
    git_in(&app, &["worktree", "add", "-q", "-b", "side", &wt.to_string_lossy()]);
    let main = open(&app).unwrap().watch();
    let dot = app.join(".git");
    let counted = |specs: &[WatchSpec], p: &Path| specs.iter().any(|s| s.matches(p));
    for p in [dot.join("HEAD"), dot.join("packed-refs"), dot.join("config"), dot.join("refs/heads/main"), dot.join("refs/remotes/origin/main")] {
        assert!(counted(&main, &p), "{} is watched", p.display());
    }
    for p in [dot.join("index"), dot.join("index.lock"), dot.join("objects/ab/cdef"), dot.join("logs/HEAD"), dot.join("FETCH_HEAD")] {
        assert!(!counted(&main, &p), "{} isn't", p.display());
    }
    let side = open(&wt).unwrap().watch();
    let own = dot.join("worktrees/app-wt");
    assert!(counted(&side, &own.join("HEAD")) && !counted(&side, &own.join("index")));
    assert!(counted(&side, &dot.join("refs/heads/side")) && counted(&side, &dot.join("packed-refs")));
    assert!(!counted(&side, &dot.join("HEAD")), "the main checkout's HEAD is the main checkout's");
}

type Files = std::collections::BTreeMap<PathBuf, (std::time::SystemTime, Vec<u8>)>;

// Each file under a folder, with when it was last written and what it
// holds: two of these say which files git wrote between them.
fn files_under(dir: &Path) -> Files {
    let mut out = Files::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                todo.push(p);
            } else if let (Ok(at), Ok(bytes)) = (m.modified(), std::fs::read(&p)) {
                out.insert(p, (at, bytes));
            }
        }
    }
    out
}

fn written_since(dir: &Path, before: &Files) -> Vec<PathBuf> {
    files_under(dir).into_iter().filter(|(p, now)| before.get(p) != Some(now)).map(|(p, _)| p).collect()
}

// What git writes for a commit and a checkout reaches the repo's owner:
// its ref and HEAD, never its index. The files are found by looking at
// .git before and after, and handed to the watcher as the system would
// name them, rather than waited for from the system, whose events a busy
// Mac's fseventsd can hold back for tens of seconds (xenonet-adapter's
// tests wait on the system's own).
#[test]
fn a_commit_and_a_checkout_are_pushed() {
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};
    use xenonet_adapter::watch::{Batch, Watcher};
    let t = temp("pushed");
    let app = t.0.join("app");
    repo_with_commit(&app);
    let r = open(&app).unwrap();
    let (tx, rx) = channel();
    let w = Watcher::start(move |b| {
        let _ = tx.send(b);
    })
    .unwrap();
    w.add("git:app", r.watch());
    // The paths of the repo's batches, until one names `file`. They were
    // handed on, so they come a window later; the half minute is only a
    // backstop.
    let heard = |file: &Path| {
        let end = Instant::now() + Duration::from_secs(30);
        let mut all = Vec::new();
        while let Ok(b) = rx.recv_timeout(end.saturating_duration_since(Instant::now())) {
            if let Batch::Changed { key, paths } = b {
                assert_eq!(key, "git:app");
                all.extend(paths);
                if all.iter().any(|p| p == file) {
                    return all;
                }
            }
        }
        panic!("no batch named {}, only {all:?}", file.display());
    };
    let git = PathBuf::from(xenonet_os::fs::realpath_native(&app.join(".git").to_string_lossy()).unwrap());
    let before = files_under(&git);
    std::fs::write(app.join("b.txt"), "b\n").unwrap();
    git_in(&app, &["add", "b.txt"]);
    git_in(&app, &["commit", "-q", "-m", "second"]);
    let wrote = written_since(&git, &before);
    assert!(wrote.contains(&git.join("index")), "git wrote its index: {wrote:?}");
    w.inject(&wrote);
    let seen = heard(&git.join("refs/heads/main"));
    assert!(!seen.contains(&git.join("index")), "the index is never watched");
    let before = files_under(&git);
    git_in(&app, &["checkout", "-q", "-b", "next"]);
    w.inject(&written_since(&git, &before));
    heard(&git.join("HEAD"));
    assert_eq!(r.branch().as_deref(), Some("next"));
}

#[test]
fn reads_a_reftable_repos_head_through_git() {
    let t = temp("reftable");
    let app = t.0.join("app");
    std::fs::create_dir_all(&app).unwrap();
    // Reftable came in git 2.45; an older git (Ubuntu 24.04's) has none to
    // read.
    let made = Command::new("git")
        .args(["-c", "init.defaultBranch=main", "init", "-q", "--ref-format=reftable"])
        .current_dir(&app)
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return;
    }
    std::fs::write(app.join("a.txt"), "a\n").unwrap();
    git_in(&app, &["add", "a.txt"]);
    git_in(&app, &["commit", "-q", "-m", "first"]);
    let head = git_in(&app, &["rev-parse", "HEAD"]);
    let r = open(&app).unwrap();
    assert!(r.reftable());
    assert_eq!(r.head(), Some(Head::Branch { name: "main".into(), commit: Some(head.clone()) }));
    git_in(&app, &["checkout", "-q", "--detach"]);
    assert_eq!(r.head(), Some(Head::Detached { commit: head }));
}
