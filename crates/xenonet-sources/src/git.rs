//! core/git.js: a repo's status, and whether git is there.
//!
//! What `git status --porcelain -b` says, the guards and filter overrides a
//! status runs with, what each run's answer means for the next, and the
//! repo watch's decisions. Running git stays with the caller (the twin, or
//! whatever stands in for git in a test).

/// What `git status --porcelain=v1 -b` says of a repo.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub branch: Option<String>,
    pub ahead: f64,
    pub behind: f64,
    pub staged: f64,
    pub modified: f64,
    pub untracked: f64,
    pub dirty: f64,
}

// "ahead 12" in a branch line: the digits after the first "<word> ", as
// /ahead (\d+)/ reads them, or 0.
fn count_after(head: &str, word: &str) -> f64 {
    let pat = format!("{word} ");
    let mut from = 0;
    while let Some(i) = head[from..].find(&pat) {
        let at = from + i + pat.len();
        let digits: String = head[at..].chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            return digits.parse::<f64>().unwrap_or(0.0);
        }
        from = from + i + 1;
    }
    0.0
}

/// A text split as `split(/\r?\n/)` splits it: at each newline, with a
/// carriage return before one taken too.
pub fn split_lines(s: &str) -> Vec<&str> {
    let parts: Vec<&str> = s.split('\n').collect();
    let last = parts.len() - 1;
    parts.into_iter().enumerate().map(|(i, l)| if i < last { l.strip_suffix('\r').unwrap_or(l) } else { l }).collect()
}

// A text's lines as /\r?\n/ splits it, without the empty ones.
fn lines_of(out: &str) -> Vec<&str> {
    split_lines(out).into_iter().filter(|l| !l.is_empty()).collect()
}

/// `git status --porcelain=v1 -b`'s output as counts: the branch (and how
/// far ahead and behind), staged, modified and untracked files, and every
/// line that's a file.
pub fn parse_status(out: &str) -> Status {
    let mut lines = lines_of(out);
    let head = if lines.first().is_some_and(|l| l.starts_with("## ")) { lines.remove(0)[3..].to_string() } else { String::new() };
    let branch = match head.strip_prefix("No commits yet on ") {
        Some(rest) => rest.to_string(),
        None => {
            let b = head.split("...").next().unwrap_or("");
            if b == "HEAD (no branch)" {
                "detached".into()
            } else {
                b.to_string()
            }
        }
    };
    let (mut staged, mut modified, mut untracked) = (0.0, 0.0, 0.0);
    for l in &lines {
        if l.starts_with("??") {
            untracked += 1.0;
        } else {
            // l[0] and l[1], as UTF-16 units: a space or not.
            let mut units = l.encode_utf16();
            if units.next() != Some(0x20) {
                staged += 1.0;
            }
            if units.next() != Some(0x20) {
                modified += 1.0;
            }
        }
    }
    Status {
        branch: (!branch.is_empty()).then_some(branch),
        ahead: count_after(&head, "ahead"),
        behind: count_after(&head, "behind"),
        staged,
        modified,
        untracked,
        dirty: lines.len() as f64,
    }
}

// XenoNet runs git status in every repo in the projects folder every 20
// seconds, ones you cloned or unpacked included, and a repo's own config
// can name programs for git status to run (see core/git.js). So status
// runs without fsmonitor, hooks and optional locks, with the repo's own
// filter drivers emptied (but git-lfs's), without looking inside
// submodules, and without fetching what a partial clone left out.
pub const GUARDS: [&str; 5] = ["--no-optional-locks", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"];
pub const STATUS_ARGS: [&str; 5] = ["status", "--porcelain=v1", "-b", "--untracked-files=normal", "--ignore-submodules=dirty"];
pub const DRIVERS_ARGS: [&str; 5] = ["config", "--show-scope", "--get-regexp", "-z", "^(filter|lfs)\\."];

/// The arguments that list the filter drivers, with their scopes or (for
/// a git before 2.26) without.
pub fn drivers_args(scoped: bool) -> Vec<&'static str> {
    DRIVERS_ARGS.iter().copied().filter(|a| scoped || *a != "--show-scope").collect()
}

// git-lfs's commands, as git lfs install writes them (with --skip-smudge
// or not, and with --local in the repo's own config).
fn lfs_commands(variable: &str) -> Option<&'static [&'static str]> {
    match variable {
        "clean" => Some(&["git-lfs clean -- %f"]),
        "smudge" => Some(&["git-lfs smudge -- %f", "git-lfs smudge --skip -- %f"]),
        "process" => Some(&["git-lfs filter-process", "git-lfs filter-process --skip"]),
        _ => None,
    }
}

// The scopes git config says are yours: your system and global config, and
// a -c of XenoNet's own. Any other is the repo's.
fn yours(scope: &str) -> bool {
    matches!(scope, "system" | "global" | "command")
}

/// The -c settings that empty the drivers the repo's own config gives
/// commands, but for one whose every command there is git-lfs's, or None
/// when one can't be emptied (a name with an = in it). `listing` is what
/// DRIVERS_ARGS prints: scope, NUL, key, newline, value, NUL; without
/// `scoped`, key, newline, value, NUL, all of it counted as the repo's.
/// Git-lfs extensions in the repo's config empty every driver.
pub fn filter_overrides(listing: &str, scoped: bool) -> Option<Vec<String>> {
    let mut drivers: Vec<String> = Vec::new();
    // driver name → whether the commands the repo gives it are all git-lfs's
    let mut lfs_only: Vec<(String, bool)> = Vec::new();
    let mut extensions = false;
    let fields: Vec<&str> = listing.split('\0').collect();
    let mut i = 0;
    while i < fields.len() {
        let scope = if scoped {
            let s = fields[i];
            i += 1;
            s
        } else {
            "local"
        };
        let entry = fields.get(i).copied().unwrap_or("");
        i += 1;
        if entry.is_empty() {
            continue;
        }
        let repo = !yours(scope);
        let (key, value) = match entry.find('\n') {
            Some(nl) => (&entry[..nl], Some(&entry[nl + 1..])),
            None => (entry, None),
        };
        if key.len() >= 14 && key.as_bytes()[..14].eq_ignore_ascii_case(b"lfs.extension.") {
            extensions = extensions || repo;
            continue;
        }
        // filter.<name>.<variable>; a name can hold dots and spaces.
        let dot = key.rfind('.');
        let Some(dot) = dot.filter(|&d| key.starts_with("filter.") && d >= 7) else { continue };
        let name = &key[7..dot];
        let variable = key[dot + 1..].to_lowercase();
        if !drivers.iter().any(|d| d == name) {
            drivers.push(name.to_string());
        }
        let Some(commands) = lfs_commands(&variable).filter(|_| repo) else { continue };
        let lfs = value.is_some_and(|v| commands.contains(&v));
        match lfs_only.iter_mut().find(|(n, _)| n == name) {
            Some((_, was)) => *was = *was && lfs,
            None => lfs_only.push((name.to_string(), lfs)),
        }
    }
    let emptied: Vec<String> = if extensions { drivers } else { lfs_only.into_iter().filter(|(_, lfs)| !lfs).map(|(n, _)| n).collect() };
    let mut args = Vec::new();
    for name in emptied {
        if name.contains('=') {
            return None;
        }
        for setting in ["clean=", "smudge=", "process=", "required=false"] {
            args.push("-c".to_string());
            args.push(format!("filter.{name}.{setting}"));
        }
    }
    Some(args)
}

/// What a run of git config listing the drivers means for the status.
#[derive(Clone, Debug, PartialEq)]
pub enum AfterDrivers {
    /// git before 2.26 doesn't know --show-scope: list them again without.
    Unscoped,
    /// No status: git couldn't list them, or one can't be emptied.
    Nothing,
    /// Run status with these arguments (after `-C dir`).
    Status(Vec<String>),
}

/// The next step once git config has listed the drivers: its exit code
/// (None when it succeeded), its output, and whether it listed scopes.
/// Exit 129 is an option git doesn't know, exit 1 no key matched.
pub fn after_drivers(code: Option<f64>, err: bool, stdout: &str, scoped: bool) -> AfterDrivers {
    if err && scoped && code == Some(129.0) {
        return AfterDrivers::Unscoped;
    }
    if err && code != Some(1.0) {
        return AfterDrivers::Nothing;
    }
    match filter_overrides(if err { "" } else { stdout }, scoped) {
        None => AfterDrivers::Nothing,
        Some(filters) => AfterDrivers::Status(GUARDS.iter().map(ToString::to_string).chain(filters).chain(STATUS_ARGS.iter().map(ToString::to_string)).collect()),
    }
}

// The watch reads a repo's status only when something says it may have
// changed, and its filter drivers only when its config may have: with a
// dozen repos, reading both every pass ran git more than once a second.

/// A repo whose index, HEAD, reflog and fetches haven't changed is read
/// again after this long anyway, for edits to its files, which git status
/// alone sees.
pub const STATUS_IDLE_MS: f64 = 120_000.0;
/// What a repo's config said of its filter drivers stands until its config
/// files or yours change, and this long at most, for files they include.
pub const CONFIG_TTL_MS: f64 = 600_000.0;

/// A repo's git folder, and the one it shares with its other worktrees
/// (the same for a plain repo): `.git`, or where a `.git` file's gitdir
/// points, and its commondir. None when there's neither.
pub fn git_dirs(dir: &str) -> Option<(String, String)> {
    use xenonet_base::nodepath::NATIVE;
    let mut git_dir = NATIVE.join(&[dir, ".git"]);
    let meta = std::fs::metadata(&git_dir).ok()?;
    if meta.is_file() {
        let text = String::from_utf8_lossy(&std::fs::read(&git_dir).ok()?).into_owned();
        let to = text.split_once("gitdir:")?.1.lines().next()?.trim().to_string();
        git_dir = xenonet_base::nodepath::resolve(&[dir, &to]);
    }
    let common = std::fs::read(NATIVE.join(&[&git_dir, "commondir"])).ok().map(|b| String::from_utf8_lossy(&b).trim().to_string()).filter(|c| !c.is_empty());
    let common = common.map_or_else(|| git_dir.clone(), |c| xenonet_base::nodepath::resolve(&[&git_dir, &c]));
    Some((git_dir, common))
}

/// What says a repo may have changed, as its files' mtimes: `status`, its
/// index, HEAD, HEAD's reflog and ORIG_HEAD (staging, commits, checkouts,
/// resets, merges), its FETCH_HEAD and packed refs, its remote-tracking
/// refs and their reflogs, newest of each (fetches, `--no-write-fetch-head`
/// ones too, and pushes, which move a remote-tracking ref and nothing
/// else), and its config (an upstream set, say); `config`, its config files
/// and yours (~/.gitconfig, and the XDG one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stamps {
    pub status: String,
    pub config: String,
}

fn mtime_of(p: &std::path::Path) -> Option<f64> {
    std::fs::symlink_metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok())
}

/// The newest mtime of a folder and everything in it (a few levels and a
/// few thousand entries at most), or "" when it isn't there: a ref moved
/// within it rewrites its file and its folder, and its reflog grows.
fn tree_mtime(dir: &str) -> String {
    fn walk(p: &std::path::Path, depth: u8, left: &mut u32, newest: &mut Option<f64>) {
        let Some(at) = mtime_of(p) else { return };
        *newest = Some(newest.map_or(at, |n| n.max(at)));
        if depth == 0 || !p.is_dir() {
            return;
        }
        for e in std::fs::read_dir(p).into_iter().flatten().flatten() {
            if *left == 0 {
                return;
            }
            *left -= 1;
            walk(&e.path(), depth - 1, left, newest);
        }
    }
    let mut newest = None;
    walk(std::path::Path::new(dir), 8, &mut 4096, &mut newest);
    newest.map_or_else(String::new, xenonet_base::json::number_string)
}

fn mtimes(files: &[String]) -> String {
    files
        .iter()
        .map(|f| std::fs::metadata(f).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).map_or_else(String::new, xenonet_base::json::number_string))
        .collect::<Vec<_>>()
        .join("|")
}

/// A repo's stamps, or None when its git folder can't be found: then its
/// status and drivers are read every time.
pub fn repo_stamps(dir: &str, home: &str, xdg_config_home: Option<&str>) -> Option<Stamps> {
    use xenonet_base::nodepath::NATIVE;
    let (git, common) = git_dirs(dir)?;
    let at = |base: &str, parts: &[&str]| NATIVE.join(&std::iter::once(base).chain(parts.iter().copied()).collect::<Vec<_>>());
    let status = [at(&git, &["index"]), at(&git, &["HEAD"]), at(&git, &["logs", "HEAD"]), at(&git, &["ORIG_HEAD"]), at(&common, &["FETCH_HEAD"]), at(&common, &["packed-refs"])];
    let remotes = format!("{}|{}", tree_mtime(&at(&common, &["refs", "remotes"])), tree_mtime(&at(&common, &["logs", "refs", "remotes"])));
    let xdg = xdg_config_home.filter(|x| !x.is_empty()).map_or_else(|| at(home, &[".config"]), str::to_string);
    let config = [at(&common, &["config"]), at(&git, &["config.worktree"]), at(home, &[".gitconfig"]), at(&xdg, &["git", "config"])];
    let config = mtimes(&config);
    Some(Stamps { status: format!("{}|{remotes}|{config}", mtimes(&status)), config })
}

/// Whether to read a repo's status now, from when it was last read
/// (`last`: its status stamp then, and when) and its stamp now: never
/// read, a stamp that can't be told or has changed, or idle for
/// STATUS_IDLE_MS.
pub fn status_due(last: Option<(&str, f64)>, stamp: Option<&str>, now: f64) -> bool {
    match (last, stamp) {
        (Some((was, at)), Some(is)) => was != is || now - at >= STATUS_IDLE_MS,
        _ => true,
    }
}

/// Whether the drivers a repo's config listed before (`cached`: its config
/// stamp then, and when) still stand: the same stamp, within CONFIG_TTL_MS.
pub fn drivers_fresh(cached: Option<(&str, f64)>, stamp: Option<&str>, now: f64) -> bool {
    match (cached, stamp) {
        (Some((was, at)), Some(is)) => was == is && now - at < CONFIG_TTL_MS,
        _ => false,
    }
}

/// Whether git on the path needs the Command Line Tools checked: on a Mac,
/// /usr/bin/git is a stub until they're installed.
pub fn needs_dev_tools(platform: &str, git: Option<&str>) -> bool {
    platform == "darwin" && git == Some("/usr/bin/git")
}

pub const NO_DEV_TOOLS: &str = "[xenonet] git needs the Command Line Tools (xcode-select --install); repo status is off until they're installed";

/// Why a repo's status is missing.
pub fn unreadable(path: &str) -> String {
    format!("git couldn't read the status of {path}")
}

/// What the watch says when some repos' status couldn't be read: the
/// first one's error, and how many more.
pub fn problem_of(failed: &[String]) -> Option<String> {
    match failed {
        [] => None,
        [one] => Some(one.clone()),
        [first, rest @ ..] => Some(format!("{first}, and {} more", rest.len())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repos_status_is_read_when_it_may_have_changed_and_its_drivers_when_its_config_may_have() {
        assert!(status_due(None, Some("1|2"), 0.0), "never read");
        assert!(status_due(Some(("1|2", 0.0)), None, 1.0), "a stamp that can't be told");
        assert!(!status_due(Some(("1|2", 0.0)), Some("1|2"), STATUS_IDLE_MS - 1.0));
        assert!(status_due(Some(("1|2", 0.0)), Some("1|3"), 1.0), "its index or HEAD changed");
        assert!(status_due(Some(("1|2", 0.0)), Some("1|2"), STATUS_IDLE_MS), "idle long enough");
        assert!(!drivers_fresh(None, Some("c"), 0.0));
        assert!(!drivers_fresh(Some(("c", 0.0)), None, 0.0));
        assert!(drivers_fresh(Some(("c", 0.0)), Some("c"), CONFIG_TTL_MS - 1.0));
        assert!(!drivers_fresh(Some(("c", 0.0)), Some("d"), 1.0), "its config changed");
        assert!(!drivers_fresh(Some(("c", 0.0)), Some("c"), CONFIG_TTL_MS));
    }

    #[test]
    fn a_push_changes_the_status_stamp() {
        let root = std::env::temp_dir().join(format!("xenonet-stamps-push-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let git = |dir: &std::path::Path, args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath="])
                .args(args)
                .current_dir(dir)
                .output()
        };
        if !git(&root, &["--version"]).is_ok_and(|o| o.status.success()) {
            return;
        }
        let (remote, repo) = (root.join("remote.git"), root.join("repo"));
        let ok = |o: std::io::Result<std::process::Output>| assert!(o.unwrap().status.success());
        ok(git(&root, &["init", "-q", "--bare", &remote.to_string_lossy()]));
        ok(git(&root, &["init", "-q", "-b", "main", &repo.to_string_lossy()]));
        std::fs::write(repo.join("a.txt"), "a").unwrap();
        ok(git(&repo, &["add", "-A"]));
        ok(git(&repo, &["commit", "-q", "-m", "one"]));
        ok(git(&repo, &["remote", "add", "origin", &remote.to_string_lossy()]));
        ok(git(&repo, &["push", "-q", "-u", "origin", "main"]));
        std::fs::write(repo.join("a.txt"), "b").unwrap();
        ok(git(&repo, &["commit", "-q", "-am", "two"]));
        let (path, home) = (repo.to_string_lossy().into_owned(), root.join("home").to_string_lossy().into_owned());
        let ahead = repo_stamps(&path, &home, None).unwrap();
        assert_eq!(repo_stamps(&path, &home, None).unwrap(), ahead);
        // A push moves the remote-tracking ref and its reflog, and nothing
        // else a status reads: "ahead 1" goes.
        ok(git(&repo, &["push", "-q"]));
        let pushed = repo_stamps(&path, &home, None).unwrap();
        assert_ne!(pushed.status, ahead.status);
        assert_eq!(pushed.config, ahead.config);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_stamps_follow_a_repos_own_files_and_a_worktrees_shared_ones() {
        let root = std::env::temp_dir().join(format!("xenonet-stamps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        let git = repo.join(".git");
        std::fs::create_dir_all(git.join("logs")).unwrap();
        std::fs::create_dir_all(git.join("worktrees").join("wt")).unwrap();
        for f in ["index", "HEAD", "config", "logs/HEAD"] {
            std::fs::write(git.join(f), "x").unwrap();
        }
        let home = root.join("home").to_string_lossy().into_owned();
        let path = repo.to_string_lossy().into_owned();
        assert_eq!(git_dirs(&path), Some((git.to_string_lossy().into_owned(), git.to_string_lossy().into_owned())));
        let first = repo_stamps(&path, &home, None).unwrap();
        assert_eq!(repo_stamps(&path, &home, None).unwrap(), first, "nothing changed");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(git.join("index")).unwrap().set_modified(later).unwrap();
        let staged = repo_stamps(&path, &home, None).unwrap();
        assert!(staged.status != first.status && staged.config == first.config);
        // Its config changing changes both: an upstream set changes ahead
        // and behind.
        std::fs::File::options().write(true).open(git.join("config")).unwrap().set_modified(later).unwrap();
        assert_ne!(repo_stamps(&path, &home, None).unwrap().config, staged.config);
        // A worktree: a .git file that points at its folder, which shares
        // the repo's config.
        let wt = root.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", git.join("worktrees").join("wt").display())).unwrap();
        std::fs::write(git.join("worktrees").join("wt").join("commondir"), "../..\n").unwrap();
        let (own, shared) = git_dirs(&wt.to_string_lossy()).unwrap();
        assert!(own.ends_with("wt") && std::path::Path::new(&shared) == git.as_path(), "{own} {shared}");
        assert_eq!(repo_stamps(&root.join("plain").to_string_lossy(), &home, None), None, "not a repo: read every time");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reads_a_status() {
        let s = parse_status("## main...origin/main [ahead 2, behind 13]\n M src/a.ts\n?? notes.md\nMM both\r\n");
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!([s.ahead, s.behind, s.staged, s.modified, s.untracked, s.dirty], [2.0, 13.0, 1.0, 2.0, 1.0, 3.0]);
        assert_eq!(parse_status("## No commits yet on dev\n").branch.as_deref(), Some("dev"));
        assert_eq!(parse_status("## HEAD (no branch)\n").branch.as_deref(), Some("detached"));
        assert_eq!(parse_status("").branch, None);
        assert_eq!(parse_status(" M a\n").dirty, 1.0);
        // A line of one character: its second is missing, so not a space.
        assert_eq!(parse_status("M\n").modified, 1.0);
        assert_eq!(count_after("ahead x, ahead 4", "ahead"), 4.0);
    }

    #[test]
    fn empties_the_repos_drivers() {
        let e = |s: &str, k: &str, v: Option<&str>| format!("{s}\0{}\0", v.map_or_else(|| k.to_string(), |v| format!("{k}\n{v}")));
        let listing = [
            e("global", "filter.lfs.clean", Some("git-lfs clean -- %f")),
            e("local", "filter.marker.clean", Some("cat")),
            e("local", "filter.lfs.smudge", Some("git-lfs smudge -- %f")),
        ]
        .concat();
        let args = filter_overrides(&listing, true).unwrap();
        assert_eq!(args, ["-c", "filter.marker.clean=", "-c", "filter.marker.smudge=", "-c", "filter.marker.process=", "-c", "filter.marker.required=false"]);
        assert_eq!(filter_overrides("", true), Some(vec![]));
        assert_eq!(filter_overrides(&e("local", "filter.x=y.clean", Some("cat")), true), None);
        assert_eq!(filter_overrides(&e("local", "lfs.Extension.a.clean", Some("x")), true), Some(vec![]));
        let ext = [e("global", "filter.crypt.clean", Some("x")), e("local", "LFS.extension.a.clean", Some("x"))].concat();
        assert_eq!(filter_overrides(&ext, true).unwrap().len(), 8);
    }

    #[test]
    fn says_what_comes_next() {
        assert_eq!(after_drivers(Some(129.0), true, "", true), AfterDrivers::Unscoped);
        assert_eq!(after_drivers(Some(129.0), true, "", false), AfterDrivers::Nothing);
        assert_eq!(after_drivers(Some(128.0), true, "", true), AfterDrivers::Nothing);
        let AfterDrivers::Status(args) = after_drivers(Some(1.0), true, "junk", true) else { panic!() };
        assert_eq!(args.len(), 10);
        assert_eq!(problem_of(&["a".into(), "b".into(), "c".into()]).as_deref(), Some("a, and 2 more"));
    }
}
