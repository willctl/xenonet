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
pub(crate) fn split_lines(s: &str) -> Vec<&str> {
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
