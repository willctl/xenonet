//! xenonet identity: a project's rule for the git email and GitHub push
//! account it uses (set through the local service, which is yours to do and
//! refused from a session's shell), the check of that rule, and an opt-in
//! pre-push hook that refuses a push under the wrong identity, whichever
//! program pushes.

use crate::args::Flags;
use crate::js::{encode, str, t};
use crate::out::{c, eprint, json, print, Done, Fail};
use crate::reach::need;
use crate::sys;
use serde_json::{json, Map, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use xenonet_api::client::Server;

pub const HELP: &str = include_str!("help/identity.txt");

/// The line that marks a hook as this command's own.
const MARK: &str = "# xenonet-identity-guard";
const OVERRIDE: &str = "To push anyway, run git push --no-verify. That override is yours to use.";

fn call(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    s.request(method, route, body, &[]).map_err(|r| Fail::new(r.message, 1))
}

fn project_of(s: &Server, folder: &str) -> Result<Value, Fail> {
    call(s, "GET", &format!("/api/identities/project?path={}", encode(folder)), None)
}

fn folder_arg(arg: Option<&String>) -> String {
    arg.filter(|a| !a.is_empty()).map_or_else(sys::cwd, |a| crate::commands::root::folder(a))
}

fn rule_words(rule: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(git) = str(rule, "git") {
        parts.push(format!("commits as {git}"));
    }
    if let Some(p) = rule.get("push") {
        parts.push(format!("pushes as {} on {}", t(p, "login"), p.get("host").and_then(Value::as_str).unwrap_or("github.com")));
    }
    if let Some(a) = str(rule, "claude") {
        parts.push(format!("Claude Code as {a}"));
    }
    if let Some(a) = str(rule, "chatgpt") {
        parts.push(format!("Codex as {a}"));
    }
    if str(rule, "mode") == Some("refuse") {
        parts.push("refuses a start it can't place".into());
    }
    parts.join(", ")
}

/// What one part of a check says, with the mark for it.
fn part_line(check: &Value, key: &str, label: &str, wants: String) -> Option<String> {
    let part = check.get(key)?;
    let uses = str(part, "uses");
    Some(match str(part, "state") {
        Some("ok") => format!("  {} {label} {}", c::green("✓"), c::dim(&wants)),
        Some("differs") => format!("  {} {label} {wants}, but {}", c::red("✗"), uses.map_or_else(|| "XenoNet reads another".to_string(), |u| format!("here it is {u}"))),
        _ => format!("  {} {label} {wants}, which XenoNet couldn't read here", c::yellow("?")),
    })
}

fn show(found: &Value) {
    let Some(project) = found.get("project").filter(|p| !p.is_null()) else {
        print(format!("{} This folder isn't in a project XenoNet knows.", c::dim("·")));
        return;
    };
    let name = t(project, "name");
    let rule = found.get("rule").filter(|r| r.is_object());
    let Some(rule) = rule else {
        print(format!("{} {name} has no rule. xenonet identity rule --git <email> --push <login> sets one.", c::dim("·")));
        return;
    };
    let check = found.get("check").unwrap_or(&Value::Null);
    let mark = if check.get("mismatch") == Some(&Value::Bool(true)) { c::red("✗") } else { c::green("✓") };
    print(format!("{mark} {} {}", c::bold(&name), c::dim(format!("rule: {}", rule_words(rule)))));
    let git = str(rule, "git").map(|g| format!("commits as {g}"));
    let push = rule.get("push").map(|p| format!("pushes as {} on {}", t(p, "login"), p.get("host").and_then(Value::as_str).unwrap_or("github.com")));
    let claude = str(rule, "claude").map(|a| format!("Claude Code runs as {a}"));
    let chatgpt = str(rule, "chatgpt").map(|a| format!("Codex runs as {a}"));
    let lines = [
        git.and_then(|w| part_line(check, "git", "git ", w)),
        push.and_then(|w| part_line(check, "push", "push", w)),
        claude.and_then(|w| part_line(check, "claude", "claude", w)),
        chatgpt.and_then(|w| part_line(check, "chatgpt", "codex ", w)),
    ];
    for line in lines.into_iter().flatten() {
        print(line);
    }
}

/// The rule as asked for: what the project has, with `--git` and `--push`
/// laid over it.
fn wanted(current: Option<&Value>, flags: &Flags) -> Result<Map<String, Value>, Fail> {
    let mut body = Map::new();
    if let Some(Value::Object(old)) = current {
        if let Some(g) = old.get("git") {
            body.insert("git".into(), g.clone());
        }
        if let Some(p) = old.get("push") {
            body.insert("push".into(), p.clone());
        }
        for k in ["claude", "chatgpt", "mode"] {
            if let Some(v) = old.get(k) {
                body.insert(k.into(), v.clone());
            }
        }
    }
    for (key, what) in [("claude", "Claude Code"), ("chatgpt", "Codex")] {
        if flags.has(key) {
            match flags.str(key).filter(|g| g.contains('@')) {
                Some(a) => body.insert(key.into(), json!(a)),
                None => return Err(Fail::new(format!("--{key} takes the email of the {what} account: xenonet identity rule --{key} nova@example.com"), 2)),
            };
        }
    }
    if flags.has("mode") {
        match flags.str("mode").filter(|m| matches!(*m, "warn" | "refuse")) {
            Some(m) => body.insert("mode".into(), json!(m)),
            None => return Err(Fail::new("--mode is warn or refuse: warn only says, refuse stops a start XenoNet can't place under the rule's accounts", 2)),
        };
    }
    if flags.has("git") {
        match flags.str("git").filter(|g| g.contains('@')) {
            Some(g) => body.insert("git".into(), json!(g)),
            None => return Err(Fail::new("--git takes an email address: xenonet identity rule --git nova@example.com", 2)),
        };
    }
    if flags.has("push") {
        let Some(p) = flags.str("push").filter(|p| !p.is_empty()) else { return Err(Fail::new("--push takes an account: xenonet identity rule --push <login>[@host]", 2)) };
        let (login, host) = p.split_once('@').map_or((p, "github.com"), |(l, h)| (l, h));
        if login.is_empty() || host.is_empty() {
            return Err(Fail::new("--push takes an account: xenonet identity rule --push <login>[@host]", 2));
        }
        body.insert("push".into(), json!({ "login": login, "host": host }));
    }
    Ok(body)
}

fn rule(args: &[String], flags: &Flags) -> Done {
    let folder = folder_arg(args.get(1));
    let s = need(flags)?;
    let found = project_of(&s, &folder)?;
    let Some(id) = found.get("project").and_then(|p| str(p, "id")).map(str::to_string) else {
        if flags.on("json") {
            json(&found);
        } else {
            show(&found);
        }
        return Ok(1);
    };
    let sets = ["git", "push", "claude", "chatgpt", "mode"].iter().any(|k| flags.has(k));
    if flags.on("clear") && sets {
        return Err(Fail::new("--clear takes the rule away; give it alone", 2));
    }
    if flags.on("clear") || sets {
        let mut body = if flags.on("clear") { Map::new() } else { wanted(found.get("rule"), flags)? };
        body.insert("nodeId".into(), json!(id));
        call(&s, "POST", "/api/identities/rule", Some(&Value::Object(body)))?;
        let after = project_of(&s, &folder)?;
        if flags.on("json") {
            json(&after);
        } else {
            if flags.on("clear") {
                print(format!("{} {} has no rule now.", c::green("✓"), t(&after["project"], "name")));
            }
            show(&after);
        }
        return Ok(0);
    }
    if flags.on("json") {
        json(&found);
    } else {
        show(&found);
    }
    Ok(0)
}

fn check(args: &[String], flags: &Flags) -> Done {
    let folder = folder_arg(args.get(1));
    let s = need(flags)?;
    let found = project_of(&s, &folder)?;
    if flags.on("json") {
        json(&found);
    } else {
        show(&found);
    }
    Ok(i32::from(found.get("check").and_then(|c| c.get("mismatch")) == Some(&Value::Bool(true))))
}

// ---------- the pre-push hook's decision ----------

/// One line git gives a pre-push hook: `<local ref> <local sha> <remote ref>
/// <remote sha>`.
#[derive(Debug, PartialEq, Eq)]
pub struct Update {
    pub local_ref: String,
    pub local_sha: String,
    pub remote_ref: String,
    pub remote_sha: String,
}

/// An all-zero sha is git's "no commit": a branch that is new on the
/// remote, or one being deleted.
fn is_zero(sha: &str) -> bool {
    !sha.is_empty() && sha.chars().all(|c| c == '0')
}

pub fn updates(stdin: &str) -> Vec<Update> {
    stdin
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            (f.len() >= 4).then(|| Update { local_ref: f[0].into(), local_sha: f[1].into(), remote_ref: f[2].into(), remote_sha: f[3].into() })
        })
        .collect()
}

/// A commit being pushed, with the emails it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub author: String,
    pub committer: String,
}

/// Why a push has to stop, in words, given the project's rule and the
/// service's check of it; empty when it may go. A part the service
/// couldn't read ("unknown") is not a reason.
pub fn problems(project: &str, rule: &Value, check: &Value, commits: &[Commit]) -> Vec<String> {
    let mut out = Vec::new();
    if let (Some(push), Some(part)) = (rule.get("push"), check.get("push").filter(|p| str(p, "state") == Some("differs"))) {
        let (login, host) = (t(push, "login"), push.get("host").and_then(Value::as_str).unwrap_or("github.com").to_string());
        out.push(format!(
            "A push from {project} would sign in as {}, and its rule says {login} on {host}. Switch to that account (for GitHub's gh: gh auth switch --user {login}), or change the rule with xenonet identity rule --push <login>[@host].",
            str(part, "uses").unwrap_or("another account")
        ));
    }
    if let Some(want) = str(rule, "git") {
        let wrong: Vec<&Commit> = commits.iter().filter(|c| !c.author.eq_ignore_ascii_case(want) || !c.committer.eq_ignore_ascii_case(want)).collect();
        if !wrong.is_empty() {
            let shown: Vec<String> = wrong
                .iter()
                .take(5)
                .map(|c| {
                    let who = if !c.author.eq_ignore_ascii_case(want) && !c.committer.eq_ignore_ascii_case(want) && c.author != c.committer {
                        format!("author {}, committer {}", c.author, c.committer)
                    } else if !c.author.eq_ignore_ascii_case(want) {
                        format!("author {}", c.author)
                    } else {
                        format!("committer {}", c.committer)
                    };
                    format!("  {} {who}", c.sha.chars().take(8).collect::<String>())
                })
                .collect();
            let more = if wrong.len() > 5 { format!("\n  and {} more", wrong.len() - 5) } else { String::new() };
            out.push(format!(
                "{} of the commits in this push {} an email other than {want}, the one {project}'s rule names:\n{}{more}\nSet git config user.email {want} and rewrite them (git rebase -r --exec 'git commit --amend --no-edit --reset-author' <the commit before them>), or change the rule with xenonet identity rule --git <email>.",
                wrong.len(),
                if wrong.len() == 1 { "carries" } else { "carry" },
                shown.join("\n")
            ));
        }
    }
    out
}

fn git(dir: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args);
    xenonet_os::process::Background::no_window(&mut cmd);
    let out = cmd.output().map_err(|e| format!("couldn't run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// The commits these updates would add to `remote`: those reachable from
/// each pushed commit and from none that the remote's branches here, or the
/// remote's old commit, already have.
fn commits_in(dir: &str, remote: &str, updates: &[Update]) -> Result<Vec<Commit>, String> {
    let known = git(dir, &["remote"]).unwrap_or_default().lines().any(|r| r == remote);
    let remotes = if known { format!("--remotes={remote}") } else { "--remotes".to_string() };
    let mut seen: Vec<Commit> = Vec::new();
    for u in updates {
        if is_zero(&u.local_sha) {
            continue;
        }
        let mut args = vec!["log", "--format=%H%x09%ae%x09%ce", &u.local_sha, "--not", &remotes];
        let have = !is_zero(&u.remote_sha) && git(dir, &["cat-file", "-e", &format!("{}^{{commit}}", u.remote_sha)]).is_ok();
        if have {
            args.push(&u.remote_sha);
        }
        let listed = git(dir, &args).map_err(|e| format!("git couldn't list what {} adds: {e}", u.local_ref))?;
        for line in listed.lines() {
            let mut f = line.split('\t');
            if let (Some(sha), Some(author), Some(committer)) = (f.next(), f.next(), f.next()) {
                if !seen.iter().any(|c| c.sha == sha) {
                    seen.push(Commit { sha: sha.into(), author: author.into(), committer: committer.into() });
                }
            }
        }
    }
    Ok(seen)
}

fn refuse(lines: &[String]) -> i32 {
    eprint(c::red("xenonet identity guard: this push is refused."));
    for l in lines {
        eprint(l);
    }
    eprint(OVERRIDE);
    1
}

/// `xenonet identity check --push <remote> <url>`, as the hook runs it.
fn guard(args: &[String], flags: &Flags) -> i32 {
    let remote = args.get(1).map_or("", String::as_str);
    let dir = sys::cwd();
    let s = match need(flags) {
        Ok(s) => s,
        Err(f) => return refuse(&[format!("It couldn't reach XenoNet to check ({}), and the guard was installed to protect this repo, so it stops here.", f.message)]),
    };
    let found = match project_of(&s, &dir) {
        Ok(f) => f,
        Err(f) => return refuse(&[format!("XenoNet didn't answer the check ({}), and the guard was installed to protect this repo, so it stops here.", f.message)]),
    };
    // No rule, nothing to hold the push to.
    let Some(rule) = found.get("rule").filter(|r| r.is_object()) else { return 0 };
    let project = found.get("project").map_or_else(|| "this project".to_string(), |p| t(p, "name"));
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let commits = match commits_in(&dir, remote, &updates(&input)) {
        Ok(c) => c,
        Err(e) => return refuse(&[e]),
    };
    let why = problems(&project, rule, found.get("check").unwrap_or(&Value::Null), &commits);
    if why.is_empty() {
        return 0;
    }
    refuse(&why)
}

// ---------- the hook's file ----------

fn script() -> String {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().replace('\'', "'\\''")).unwrap_or_default();
    format!(
        "#!/bin/sh\n{MARK}\n# Installed by `xenonet identity hook install`; `xenonet identity hook remove` takes it out.\n# `git push --no-verify` skips it.\nbin='{exe}'\n[ -x \"$bin\" ] || bin=xenonet\nif ! command -v \"$bin\" >/dev/null 2>&1; then\n  echo \"xenonet identity guard: the xenonet command isn't there, so this push is refused. Install it, or run xenonet identity hook remove, or push with --no-verify.\" >&2\n  exit 1\nfi\nexec \"$bin\" identity check --push \"$@\"\n"
    )
}

/// Where the repo in `folder` keeps its pre-push hook.
fn hook_path(folder: &str) -> Result<PathBuf, Fail> {
    let out = git(folder, &["rev-parse", "--git-path", "hooks"]).map_err(|e| Fail::new(format!("{folder} isn't in a git repository ({e})"), 1))?;
    let dir = Path::new(out.trim());
    Ok(if dir.is_absolute() { dir.to_path_buf() } else { Path::new(folder).join(dir) }.join("pre-push"))
}

fn hook(args: &[String], flags: &Flags) -> Done {
    let what = args.get(1).map_or("", String::as_str);
    if !matches!(what, "install" | "remove") {
        return Err(Fail::new("xenonet identity hook install or remove", 2));
    }
    let folder = folder_arg(args.get(2));
    let file = hook_path(&folder)?;
    let ours = std::fs::read_to_string(&file).map(|t| t.lines().any(|l| l == MARK));
    let shown = xenonet_os::paths::tilde(&file.to_string_lossy(), &sys::home());
    let done = |text: String| -> Done {
        if flags.on("json") {
            json(&json!({ "hook": file.to_string_lossy(), "done": what }));
        } else {
            print(text);
        }
        Ok(0)
    };
    match (what, ours) {
        ("install", Ok(false)) => Err(Fail::new(format!("There's a pre-push hook at {shown} that isn't XenoNet's, so it's left alone. Add a line to it that runs `xenonet identity check --push \"$@\"`, or move it aside and install again."), 1)),
        ("install", _) => {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Fail::new(format!("couldn't make {}: {e}", dir.display()), 1))?;
            }
            std::fs::write(&file, script()).map_err(|e| Fail::new(format!("couldn't write {shown}: {e}"), 1))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
            }
            done(format!("{} The identity guard is in {shown}. It checks a push only in a project that has a rule (xenonet identity rule), and git push --no-verify skips it.", c::green("✓")))
        }
        ("remove", Ok(true)) => {
            std::fs::remove_file(&file).map_err(|e| Fail::new(format!("couldn't remove {shown}: {e}"), 1))?;
            done(format!("{} The identity guard is out of {shown}.", c::green("✓")))
        }
        ("remove", Ok(false)) => Err(Fail::new(format!("The pre-push hook at {shown} isn't XenoNet's, so it's left alone."), 1)),
        _ => done(format!("{} There's no identity guard in {shown}.", c::dim("·"))),
    }
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    match args.first().map_or("", String::as_str) {
        "rule" => rule(args, flags),
        "check" if flags.on("push") => Ok(guard(args, flags)),
        "check" => check(args, flags),
        "hook" => hook(args, flags),
        "" => Err(Fail::new("xenonet identity rule, check or hook; xenonet identity --help says how", 2)),
        w => Err(Fail::new(format!("xenonet identity doesn't know {w}: rule, check or hook"), 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: &str = "0000000000000000000000000000000000000000";

    fn commit(sha: &str, author: &str, committer: &str) -> Commit {
        Commit { sha: sha.into(), author: author.into(), committer: committer.into() }
    }

    fn rule() -> Value {
        json!({ "git": "nova@example.com", "push": { "login": "nova-dev", "host": "github.com" } })
    }

    #[test]
    fn reads_the_ranges_git_gives_the_hook() {
        let text = format!("refs/heads/main abc123 refs/heads/main def456\nrefs/heads/new 1111 refs/heads/new {ZERO}\n(delete) {ZERO} refs/heads/old 9999\n\nnot a range\n");
        let u = updates(&text);
        assert_eq!(u.len(), 3);
        assert_eq!((u[0].local_sha.as_str(), u[0].remote_sha.as_str()), ("abc123", "def456"));
        assert!(is_zero(&u[1].remote_sha) && !is_zero(&u[1].local_sha), "a new branch has no remote commit");
        assert!(is_zero(&u[2].local_sha), "a deletion has no local one");
        assert!(!is_zero(""), "no sha is not the zero sha");
    }

    #[test]
    fn lets_a_push_through_when_everything_matches() {
        let ok = json!({ "push": { "state": "ok" }, "git": { "state": "ok" } });
        assert_eq!(problems("atlas", &rule(), &ok, &[commit("aaaaaaaa11", "nova@example.com", "Nova@Example.com")]), Vec::<String>::new());
        assert!(problems("atlas", &rule(), &ok, &[]).is_empty(), "nothing new to check");
    }

    #[test]
    fn what_the_service_could_not_read_is_not_a_reason() {
        let unknown = json!({ "push": { "state": "unknown" } });
        assert_eq!(problems("atlas", &rule(), &unknown, &[]), Vec::<String>::new());
    }

    #[test]
    fn refuses_a_push_account_that_is_not_the_rules() {
        let differs = json!({ "push": { "state": "differs", "uses": "orbitworks/github.com" } });
        let why = problems("atlas", &rule(), &differs, &[]);
        assert_eq!(why.len(), 1);
        assert!(why[0].contains("orbitworks/github.com") && why[0].contains("nova-dev on github.com") && why[0].contains("gh auth switch --user nova-dev"), "{}", why[0]);
    }

    #[test]
    fn refuses_a_commit_whose_author_or_committer_is_another() {
        let ok = json!({});
        let by_author = problems("atlas", &rule(), &ok, &[commit("aaaaaaaa11", "hello@orbit.example", "nova@example.com")]);
        assert!(by_author[0].contains("aaaaaaaa author hello@orbit.example") && !by_author[0].contains("committer"), "{}", by_author[0]);
        let by_committer = problems("atlas", &rule(), &ok, &[commit("bbbbbbbb22", "nova@example.com", "ci@bot.example")]);
        assert!(by_committer[0].contains("bbbbbbbb committer ci@bot.example"), "{}", by_committer[0]);
        let both = problems("atlas", &rule(), &ok, &[commit("cccccccc33", "a@x.example", "b@x.example")]);
        assert!(both[0].contains("author a@x.example, committer b@x.example"), "{}", both[0]);
        // Only the first few are named.
        let many: Vec<Commit> = (0..8).map(|i| commit(&format!("{i}000000000"), "a@x.example", "a@x.example")).collect();
        assert!(problems("atlas", &rule(), &ok, &many)[0].contains("and 3 more"));
    }

    #[test]
    fn both_reasons_are_said_together() {
        let differs = json!({ "push": { "state": "differs" } });
        assert_eq!(problems("atlas", &rule(), &differs, &[commit("aaaaaaaa11", "a@x.example", "a@x.example")]).len(), 2);
    }

    #[test]
    fn a_rule_naming_only_the_push_account_leaves_commits_alone() {
        let only_push = json!({ "push": { "login": "nova-dev", "host": "github.com" } });
        assert_eq!(problems("atlas", &only_push, &json!({ "push": { "state": "ok" } }), &[commit("aaaaaaaa11", "a@x.example", "a@x.example")]), Vec::<String>::new());
    }

    #[test]
    fn the_hook_script_is_marked_and_passes_gits_arguments_on() {
        let text = script();
        assert!(text.lines().any(|l| l == MARK) && text.starts_with("#!/bin/sh\n"));
        assert!(text.contains("exec \"$bin\" identity check --push \"$@\""));
    }

    #[test]
    fn an_account_with_a_host_is_split() {
        let flags = |v: Value| Flags(v.as_object().unwrap().clone());
        let body = wanted(None, &flags(json!({ "push": "nova@ghe.acme.example", "git": "nova@example.com" }))).unwrap();
        assert_eq!(body["push"], json!({ "login": "nova", "host": "ghe.acme.example" }));
        let kept = wanted(Some(&rule()), &flags(json!({ "git": "ada@example.com" }))).unwrap();
        assert_eq!((kept["git"].as_str(), kept["push"]["login"].as_str()), (Some("ada@example.com"), Some("nova-dev")), "a part left out stays");
        assert!(wanted(None, &flags(json!({ "git": "nova" }))).is_err());
        assert!(wanted(None, &flags(json!({ "push": "@host" }))).is_err());
    }
}
