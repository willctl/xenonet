// xenonet identity hook: the pre-push hook it installs, run by a real git
// push into a local bare repository, against a made-up XenoNet that says
// what the project's rule and its check are. No network and no real
// XenoNet: the hook asks XenoNet which rule holds and reads the commits
// from git.

#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;

// A XenoNet that answers every request with `body` (the health check too,
// which only has to be JSON).
fn xenonet(body: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(mut s) = s else { continue };
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) == 0 || h.trim_end().is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut got = vec![0; length];
            let _ = r.read_exact(&mut got);
            let _ = write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}", body.len());
        }
    });
    port
}

fn answer(rule: &str, check: &str) -> String {
    format!(r#"{{"project":{{"id":"project:atlas","name":"atlas","path":"/x"}},"rule":{rule},"check":{check}}}"#)
}

struct Repo {
    base: PathBuf,
    work: PathBuf,
}

impl Repo {
    fn new(name: &str) -> Repo {
        let base = std::env::temp_dir().join(format!("xenonet-identity-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let (bare, work) = (base.join("origin.git"), base.join("work"));
        let repo = Repo { base, work };
        repo.git(&repo.base, &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
        repo.git(&repo.base, &["init", "-q", "-b", "main", repo.work.to_str().unwrap()]);
        repo.git(&repo.work, &["config", "user.name", "Nova"]);
        repo.git(&repo.work, &["config", "user.email", "nova@example.com"]);
        repo.git(&repo.work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        repo
    }

    fn git(&self, dir: &Path, args: &[&str]) -> Output {
        self.git_env(dir, args, &[])
    }

    fn git_env(&self, dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
        Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.base)
            .env_remove("XENONET_PORT")
            .envs(env.iter().copied())
            .output()
            .unwrap()
    }

    fn commit(&self, file: &str, env: &[(&str, &str)]) {
        std::fs::write(self.work.join(file), file).unwrap();
        self.git(&self.work, &["add", file]);
        let out = self.git_env(&self.work, &["commit", "-q", "-m", file], env);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    fn xenonet(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_xenonet")).current_dir(&self.work).args(args).env("NO_COLOR", "1").env_remove("XENONET_PORT").output().unwrap()
    }

    fn push(&self, port: u16, extra: &[&str]) -> (bool, String) {
        let mut args = vec!["push", "-q"];
        args.extend(extra);
        args.extend(["origin", "HEAD:main"]);
        let out = self.git_env(&self.work, &args, &[("XENONET_PORT", &port.to_string())]);
        (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
    }

    fn hook(&self) -> PathBuf {
        self.work.join(".git/hooks/pre-push")
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

const RULE: &str = r#"{"git":"nova@example.com","push":{"login":"nova-dev","host":"github.com"}}"#;
const OK: &str = r#"{"mismatch":false,"git":{"state":"ok"},"push":{"state":"ok"}}"#;

#[test]
fn the_hook_lets_matching_commits_through_and_refuses_the_rest() {
    let repo = Repo::new("push");
    let out = repo.xenonet(&["identity", "hook", "install"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let script = std::fs::read_to_string(repo.hook()).unwrap();
    assert!(script.contains("# xenonet-identity-guard"));
    let again = repo.xenonet(&["identity", "hook", "install"]);
    assert!(again.status.success(), "installing again replaces its own hook");

    // A new branch whose commits all carry the rule's email.
    let port = xenonet(answer(RULE, OK));
    repo.commit("a", &[]);
    let (ok, said) = repo.push(port, &[]);
    assert!(ok, "{said}");

    // Another author on the next commit stops the push, in words.
    repo.commit("b", &[("GIT_AUTHOR_EMAIL", "hello@orbit.example")]);
    let (ok, said) = repo.push(port, &[]);
    assert!(!ok);
    assert!(said.contains("refused") && said.contains("author hello@orbit.example") && said.contains("--no-verify"), "{said}");
    // Only that commit is named, not the pushed one before it.
    assert!(said.contains("1 of the commits"), "{said}");
    // The person's override.
    assert!(repo.push(port, &["--no-verify"]).0);

    // A committer other than the rule's stops it too.
    repo.commit("c", &[("GIT_COMMITTER_EMAIL", "ci@bot.example")]);
    let (ok, said) = repo.push(port, &[]);
    assert!(!ok && said.contains("committer ci@bot.example"), "{said}");
}

#[test]
fn a_push_account_that_is_not_the_rules_is_refused() {
    let repo = Repo::new("account");
    repo.xenonet(&["identity", "hook", "install"]);
    repo.commit("a", &[]);
    let wrong = r#"{"mismatch":true,"push":{"state":"differs","uses":"orbitworks/github.com"}}"#;
    let (ok, said) = repo.push(xenonet(answer(RULE, wrong)), &[]);
    assert!(!ok && said.contains("orbitworks/github.com") && said.contains("gh auth switch --user nova-dev"), "{said}");
    // What XenoNet couldn't read is not a reason.
    let unknown = r#"{"mismatch":false,"push":{"state":"unknown","uses":null}}"#;
    assert!(repo.push(xenonet(answer(RULE, unknown)), &[]).0);
}

#[test]
fn no_rule_lets_the_push_through_and_no_xenonet_refuses_it() {
    let repo = Repo::new("rule");
    repo.xenonet(&["identity", "hook", "install"]);
    repo.commit("a", &[("GIT_AUTHOR_EMAIL", "anyone@example.com")]);
    let none = r#"{"project":{"id":"project:atlas","name":"atlas","path":"/x"},"rule":null,"check":null}"#;
    assert!(repo.push(xenonet(none.to_string()), &[]).0, "no rule for the project");
    let outside = r#"{"project":null,"rule":null,"check":null}"#;
    assert!(repo.push(xenonet(outside.to_string()), &[]).0, "not in a project XenoNet knows");

    repo.commit("b", &[]);
    let free = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let (ok, said) = repo.push(free, &[]);
    assert!(!ok && said.contains("couldn't reach XenoNet") && said.contains("--no-verify"), "{said}");
}

#[test]
fn the_hook_only_ever_touches_its_own_file() {
    let repo = Repo::new("own");
    let theirs = "#!/bin/sh\necho mine\n";
    std::fs::create_dir_all(repo.hook().parent().unwrap()).unwrap();
    std::fs::write(repo.hook(), theirs).unwrap();
    let out = repo.xenonet(&["identity", "hook", "install"]);
    assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("isn't XenoNet's"), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(repo.hook()).unwrap(), theirs);
    let out = repo.xenonet(&["identity", "hook", "remove"]);
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(repo.hook()).unwrap(), theirs);

    std::fs::remove_file(repo.hook()).unwrap();
    assert!(repo.xenonet(&["identity", "hook", "install"]).status.success());
    assert!(repo.xenonet(&["identity", "hook", "remove"]).status.success());
    assert!(!repo.hook().exists());
    assert!(repo.xenonet(&["identity", "hook", "remove"]).status.success(), "nothing to remove is not a failure");
}

#[test]
fn the_hook_goes_where_gits_hooks_path_says_and_into_a_worktrees_repo() {
    let repo = Repo::new("path");
    repo.commit("a", &[]);
    repo.git(&repo.work, &["config", "core.hooksPath", "tools/hooks"]);
    assert!(repo.xenonet(&["identity", "hook", "install"]).status.success());
    assert!(repo.work.join("tools/hooks/pre-push").exists() && !repo.hook().exists());
    repo.git(&repo.work, &["config", "--unset", "core.hooksPath"]);

    let linked = repo.base.join("linked");
    let out = repo.git(&repo.work, &["worktree", "add", "-q", linked.to_str().unwrap(), "-b", "other"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new(env!("CARGO_BIN_EXE_xenonet")).current_dir(&linked).args(["identity", "hook", "install"]).env("NO_COLOR", "1").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(repo.hook().exists(), "a linked worktree shares the repo's hooks");
}
