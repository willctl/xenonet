//! The doctor's look for actors' tokens left in the repos XenoNet knows
//! (findTokenFiles and createTokenScan in core/actors/token-file.js), one
//! for the service's life: a look is kept for a day, and a run you ask for
//! yourself looks again. git grep answers with each token it finds and the
//! file it's in: what's tracked, what's untracked but not ignored, and the
//! ignored files too (a .env, say), outside the folders of dependencies and
//! builds. Only a token XenoNet issued counts, by its SHA-256 against the
//! engine store's, so a made-up one in a test fixture isn't taken for a
//! leak; when the store can't be read, every token counts and the look
//! says it couldn't check them. A name that holds a token itself is shown
//! without it.

use crate::exec::{self, Spec};
use crate::js::{now_ms, number, Locked};
use crate::world::World;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use xenonet_actors::token_file::HEAVY_FOLDERS;
use xenonet_actors::tokens::{hash_token, TOKEN_GREP};

pub const TOKEN_SCAN_MS: f64 = 24.0 * 3600e3;
const BUDGET_MS: f64 = 5000.0;
const EACH_MS: f64 = 3000.0;
const TOGETHER: usize = 4;

/// One git grep: its exit code (None for a timeout or one that couldn't
/// run) and what it found.
fn grep(repo: &str, more: &[String], ms: f64) -> (Option<i32>, String) {
    let mut args: Vec<String> =
        ["--no-optional-locks", "-C", repo, "-c", "core.fsmonitor=false", "grep", "-o", "-z", "-I", "--untracked"].iter().map(ToString::to_string).collect();
    args.extend(more.iter().cloned());
    match exec::run(&Spec::new("git", &args, ms.max(1.0) as u64)) {
        Ok(out) => (Some(0), out),
        Err(f) if f.killed => (None, String::new()),
        Err(f) => (f.code.as_ref().and_then(Value::as_i64).map(|c| c as i32), f.stdout),
    }
}

/// 0: found some; 1: none; anything else: not looked in.
fn answered(code: Option<i32>) -> bool {
    matches!(code, Some(0 | 1))
}

/// git grep -o -z's answer: a file's name (as it is, never quoted), a NUL,
/// and a token found in it, a line each. A name may hold a newline; a
/// token never does.
fn matches(out: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = out;
    while let Some(nul) = rest.find('\0') {
        let (name, after) = (&rest[..nul], &rest[nul + 1..]);
        let end = after.find('\n').unwrap_or(after.len());
        found.push((name, &after[..end]));
        rest = after.get(end + 1..).unwrap_or("");
    }
    found
}

/// The files in these working trees that hold an actor's token XenoNet
/// issued (`issued`: the hashes of the tokens that work now), or any
/// actor's token when it can't tell (None): how many repos were looked in,
/// and the files, absolute and sorted, and `unchecked` when it couldn't
/// tell.
pub fn find_token_files(repos: &[String], issued: Option<&HashSet<String>>) -> Value {
    let started = now_ms();
    let queue = Mutex::new(repos.iter().cloned().collect::<std::collections::VecDeque<_>>());
    let found = Mutex::new((0usize, Vec::<String>::new()));
    let anywhere = regex::Regex::new(TOKEN_GREP).ok();
    // Whether a token counts, hashed once however often it turns up.
    let verdicts = Mutex::new(HashMap::<String, bool>::new());
    let counts = |token: &str| match issued {
        None => true,
        Some(known) => *verdicts.locked().entry(token.to_string()).or_insert_with(|| known.contains(&hash_token(token))),
    };
    let next = || loop {
        if now_ms() - started >= BUDGET_MS {
            return;
        }
        let Some(repo) = queue.locked().pop_front() else { return };
        let t0 = now_ms();
        let seen = grep(&repo, &["-E".into(), "-e".into(), TOKEN_GREP.into()], EACH_MS);
        let left = EACH_MS - (now_ms() - t0);
        let ignored = if !answered(seen.0) {
            seen.clone()
        } else if left <= 0.0 {
            (None, String::new())
        } else {
            let mut more: Vec<String> = ["--no-exclude-standard", "-E", "-e", TOKEN_GREP, "--", "."].iter().map(ToString::to_string).collect();
            more.extend(HEAVY_FOLDERS.iter().map(|d| format!(":(exclude,glob)**/{d}/**")));
            grep(&repo, &more, left)
        };
        if !answered(seen.0) || !answered(ignored.0) {
            continue;
        }
        let mut names: Vec<String> = Vec::new();
        let mut named = HashSet::new();
        for (code, out) in [&seen, &ignored] {
            if *code == Some(0) {
                for (n, token) in matches(out) {
                    if !named.contains(n) && counts(token) {
                        named.insert(n);
                        names.push(n.to_string());
                    }
                }
            }
        }
        let mut f = found.locked();
        f.0 += 1;
        for n in names {
            let full = xenonet_base::paths::join(&repo, &n);
            let shown = match &anywhere {
                Some(re) => re.replace_all(&full, |m: &regex::Captures<'_>| {
                    let t = &m[0];
                    format!("{}…", &t[..t.rfind('_').map_or(0, |i| i + 1)])
                }),
                None => full.as_str().into(),
            };
            f.1.push(shown.into_owned());
        }
    };
    std::thread::scope(|s| {
        for _ in 0..TOGETHER {
            s.spawn(next);
        }
    });
    let (checked, mut files) = found.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
    files.sort_by(|a, b| xenonet_engine::js::utf16_cmp(a, b));
    let mut out = json!({ "checked": checked, "files": files });
    if issued.is_none() {
        out["unchecked"] = json!(true);
    }
    out
}

/// The look, kept for a day; looks don't overlap, so one asked for during
/// another gets that one.
#[derive(Default)]
pub struct TokenScan {
    last: Mutex<Option<Value>>,
    looking: Mutex<bool>,
    done: Condvar,
}

impl TokenScan {
    /// { at, repos, checked, files }.
    pub fn look(&self, w: &World, fresh: bool) -> Value {
        {
            let mut busy = self.looking.locked();
            if *busy {
                while *busy {
                    busy = self.done.wait(busy).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                return self.last.locked().clone().unwrap_or(Value::Null);
            }
            if !fresh {
                if let Some(l) = self.last.locked().clone() {
                    if now_ms() - l["at"].as_f64().unwrap_or(0.0) < TOKEN_SCAN_MS {
                        return l;
                    }
                }
            }
            *busy = true;
        }
        let repos: Vec<String> = w
            .map
            .state
            .nodes()
            .unwrap_or_default()
            .iter()
            .filter(|n| n["kind"] == "project" && crate::js::truthy_json(&n["git"]) && n["path"].as_str().is_some_and(|p| !p.is_empty()))
            .filter_map(|n| n["path"].as_str().map(str::to_string))
            .collect();
        // The tokens that work now, read for each look: None when the
        // store can't be read, and then every token counts.
        let issued: Option<HashSet<String>> = w.actors.as_ref().and_then(|a| a.store.with(|s| s.issued_token_hashes()).ok()).map(|h| h.into_iter().collect());
        let found = find_token_files(&repos, issued.as_ref());
        let mut out = json!({ "at": number(now_ms()), "repos": repos.len(), "checked": found["checked"], "files": found["files"] });
        if found.get("unchecked").is_some() {
            out["unchecked"] = json!(true);
        }
        *self.last.locked() = Some(out.clone());
        *self.looking.locked() = false;
        self.done.notify_all();
        out
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_token_left_in_a_repo_is_found_and_shown_without_it() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-tokens-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.to_string_lossy().into_owned();
        if exec::run(&Spec::new("git", &["-C".into(), d.clone(), "init".into(), "-q".into()], 5000)).is_err() {
            return;
        }
        let token = format!("xenonet_actor_{}", "a".repeat(64));
        std::fs::write(dir.join(".env"), format!("TOKEN={token}\n")).unwrap();
        std::fs::write(dir.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(dir.join("clean.txt"), "nothing\n").unwrap();
        let issued: HashSet<String> = [hash_token(&token)].into();
        let found = find_token_files(std::slice::from_ref(&d), Some(&issued));
        assert_eq!(found, json!({ "checked": 1, "files": [format!("{d}/.env")] }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_token_xenonet_issued_counts_unless_it_cant_tell() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-issued-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fixtures")).unwrap();
        let d = dir.to_string_lossy().into_owned();
        if exec::run(&Spec::new("git", &["-C".into(), d.clone(), "init".into(), "-q".into()], 5000)).is_err() {
            return;
        }
        // A test fixture's made-up token, and a gateway token XenoNet
        // issued, left in an ignored .env beside a made-up one.
        let made_up = format!("xenonet_gateway_{}", "a".repeat(64));
        let real = format!("xenonet_gateway_{}", "0123456789abcdef".repeat(4));
        std::fs::write(dir.join("fixtures").join("gateway.json"), format!("{{\"token\":\"{made_up}\"}}\n{made_up}\n")).unwrap();
        std::fs::write(dir.join(".env"), format!("A={made_up}\nB={real}\n")).unwrap();
        std::fs::write(dir.join(".gitignore"), ".env\n").unwrap();
        let repos = std::slice::from_ref(&d);
        let issued: HashSet<String> = [hash_token(&real), hash_token("xenonet_actor_elsewhere")].into();
        assert_eq!(find_token_files(repos, Some(&issued)), json!({ "checked": 1, "files": [format!("{d}/.env")] }), "the fixture's token isn't one XenoNet issued");
        assert_eq!(find_token_files(repos, Some(&HashSet::new())), json!({ "checked": 1, "files": [] }), "none issued, none found");
        // Without the store, every token counts, and the look says it
        // couldn't check them.
        assert_eq!(find_token_files(repos, None), json!({ "checked": 1, "files": [format!("{d}/.env"), format!("{d}/fixtures/gateway.json")], "unchecked": true }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grep_answers_are_read_name_by_name() {
        let t = format!("xenonet_actor_{}", "b".repeat(64));
        let out = format!("a.txt\0{t}\nwith\nnewline.txt\0{t}\nlast\0{t}");
        assert_eq!(matches(&out), [("a.txt", t.as_str()), ("with\nnewline.txt", t.as_str()), ("last", t.as_str())]);
        assert_eq!([matches("").len(), matches("no nul here\n").len()], [0, 0]);
    }
}
