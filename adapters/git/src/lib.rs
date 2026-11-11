//! Git's adapter: what XenoNet knows of a git checkout, read from git's own
//! files the way git lays them out, with the git command only where the
//! refs aren't files (a reftable repo).
//!
//! - `find`: the checkout a path is in (its working tree's top), and
//!   `open` for a folder known to be one.
//! - `Repo`: its git folder and the one its worktrees share, whether it's
//!   the main checkout, a linked worktree (and of which) or a submodule
//!   (and of which), its head and branch, its remotes, and its repo key.
//! - `repo_key`: a remote as a project's id is made from it
//!   (`xenonet_model::ids::normalize_remote`), so the ids stay what they
//!   are wherever the key is read.
//! - `Repo::watch`: what to watch for a checkout's head and refs to move:
//!   HEAD, the refs, packed-refs and the config, never the index, which
//!   git rewrites on every status.

use std::path::{Path, PathBuf};
use xenonet_adapter::watch::WatchSpec;

/// The spec tags `Repo::watch` gives.
pub const HEAD_TAG: &str = "git-head";
pub const COMMON_TAG: &str = "git-common";
pub const REFS_TAG: &str = "git-refs";

// A ref that names a ref names one at most this many times over.
const REF_DEPTH: usize = 5;
// The most read of a file git keeps small (HEAD, a .git file, a ref).
const SMALL: u64 = 4096;
// And of a config.
const CONFIG_MAX: u64 = 1 << 20;

/// What a checkout is to its repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The repo's main working tree.
    Main,
    /// A linked worktree (`git worktree add`): `main` is the main working
    /// tree's top, or the repo's git folder for a bare repo.
    Worktree { main: PathBuf },
    /// A submodule's working tree, inside its superproject's.
    Submodule { superproject: PathBuf },
}

/// Where HEAD points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Head {
    /// On a branch; `commit` None before the branch's first commit.
    Branch { name: String, commit: Option<String> },
    /// At a commit, on no branch.
    Detached { commit: String },
}

impl Head {
    /// The branch, when it's on one.
    pub fn branch(&self) -> Option<&str> {
        match self {
            Head::Branch { name, .. } => Some(name),
            Head::Detached { .. } => None,
        }
    }

    /// The commit, when there is one.
    pub fn commit(&self) -> Option<&str> {
        match self {
            Head::Branch { commit, .. } => commit.as_deref(),
            Head::Detached { commit } => Some(commit),
        }
    }

    /// As the map labels a checkout's branch: its name, or a detached
    /// head's first 12 characters (projects.rs's gitInfo).
    pub fn label(&self) -> String {
        match self {
            Head::Branch { name, .. } => name.clone(),
            Head::Detached { commit } => commit.chars().take(12).collect(),
        }
    }
}

/// A remote as the config names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

/// A checkout: its working tree, its own git folder and the one it shares
/// with the repo's other worktrees, and what it is to the repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    /// The working tree's top.
    pub top: PathBuf,
    /// Its own git folder: `.git`, or where a `.git` file points
    /// (`.git/worktrees/<name>`, `.git/modules/<name>`).
    pub git_dir: PathBuf,
    /// The folder its refs, objects and config are in: the main `.git` for
    /// a linked worktree, its own git folder otherwise.
    pub common_dir: PathBuf,
    pub kind: Kind,
}

fn small(path: &Path, max: u64) -> Option<String> {
    use std::io::Read as _;
    let mut out = Vec::new();
    std::fs::File::open(path).ok()?.take(max).read_to_end(&mut out).ok()?;
    Some(String::from_utf8_lossy(&out).into_owned())
}

// A path a git file names, from the folder that file is in.
fn from(base: &Path, named: &str) -> PathBuf {
    let p = Path::new(named);
    let joined = if p.is_absolute() { p.to_path_buf() } else { base.join(p) };
    tidy(&joined)
}

// `.` and `..` taken out, as path.resolve does, without touching the disk.
fn tidy(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The top of the checkout a path is in: the nearest folder at or above it
/// with a `.git` (a folder, or a worktree's or submodule's `.git` file).
pub fn root(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|d| d.join(".git").exists()).map(Path::to_path_buf)
}

/// The checkout a path is in.
pub fn find(path: &Path) -> Option<Repo> {
    open(&root(path)?)
}

/// The checkout whose top is `top`, or None when it has no `.git` git can
/// read.
pub fn open(top: &Path) -> Option<Repo> {
    let dot = top.join(".git");
    let meta = std::fs::metadata(&dot).ok()?;
    let git_dir = if meta.is_file() {
        // A worktree's or a submodule's `.git` file: `gitdir: <path>`.
        let text = small(&dot, SMALL)?;
        let to = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:"))?.trim().to_string();
        from(top, &to)
    } else {
        dot
    };
    if !git_dir.join("HEAD").is_file() {
        return None;
    }
    // A linked worktree's git folder names the shared one in commondir.
    let common = small(&git_dir.join("commondir"), SMALL).map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    let common_dir = common.as_ref().map_or_else(|| git_dir.clone(), |c| from(&git_dir, c));
    let kind = if common.is_some() {
        let main = if common_dir.file_name().is_some_and(|n| n == ".git") { common_dir.parent().map_or_else(|| common_dir.clone(), Path::to_path_buf) } else { common_dir.clone() };
        Kind::Worktree { main }
    } else {
        superproject(&git_dir).map_or(Kind::Main, |superproject| Kind::Submodule { superproject })
    };
    Some(Repo { top: top.to_path_buf(), git_dir, common_dir, kind })
}

// A submodule's git folder is `<superproject>/.git/modules/<name>` (nested
// for a submodule's own): the superproject is what's before the last
// `.git/modules`.
fn superproject(git_dir: &Path) -> Option<PathBuf> {
    let parts: Vec<_> = git_dir.components().collect();
    let at = (0..parts.len().saturating_sub(1)).rev().find(|&i| parts[i].as_os_str() == ".git" && parts[i + 1].as_os_str() == "modules")?;
    Some(parts[..at].iter().collect())
}

// A ref's commit, in the loose refs, then packed-refs; a ref that names
// another is followed.
fn resolve(common: &Path, name: &str, depth: usize) -> Option<String> {
    if depth > REF_DEPTH {
        return None;
    }
    if let Some(text) = small(&common.join(name), SMALL) {
        let t = text.trim();
        return match t.strip_prefix("ref:") {
            Some(next) => resolve(common, next.trim(), depth + 1),
            None => Some(t.to_string()).filter(|s| is_hash(s)),
        };
    }
    let packed = std::fs::read_to_string(common.join("packed-refs")).ok()?;
    packed.lines().filter(|l| !l.starts_with('#') && !l.starts_with('^')).find_map(|l| {
        let (hash, r) = l.split_once(' ')?;
        (r.trim() == name && is_hash(hash)).then(|| hash.to_string())
    })
}

fn is_hash(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Repo {
    /// Whether it's a linked worktree.
    pub fn is_worktree(&self) -> bool {
        matches!(self.kind, Kind::Worktree { .. })
    }

    /// The checkout it belongs to: a worktree's main working tree, a
    /// submodule's superproject; None for a main checkout.
    pub fn parent(&self) -> Option<&Path> {
        match &self.kind {
            Kind::Main => None,
            Kind::Worktree { main } => Some(main),
            Kind::Submodule { superproject } => Some(superproject),
        }
    }

    /// Whether its refs are in a reftable rather than files, which only
    /// the git command reads.
    fn reftable(&self) -> bool {
        self.common_dir.join("reftable").is_dir()
    }

    /// Where HEAD points now, or None when HEAD can't be read.
    pub fn head(&self) -> Option<Head> {
        if self.reftable() {
            return self.head_by_command();
        }
        let text = small(&self.git_dir.join("HEAD"), SMALL)?;
        let t = text.trim();
        match t.strip_prefix("ref:") {
            Some(r) => {
                let r = r.trim();
                let name = r.strip_prefix("refs/heads/").unwrap_or(r).to_string();
                Some(Head::Branch { name, commit: resolve(&self.common_dir, r, 0) })
            }
            None => is_hash(t).then(|| Head::Detached { commit: t.to_string() }),
        }
    }

    /// The branch it's on, None when detached.
    pub fn branch(&self) -> Option<String> {
        self.head().and_then(|h| h.branch().map(str::to_string))
    }

    /// The commit HEAD is at, None before the first.
    pub fn commit(&self) -> Option<String> {
        self.head().and_then(|h| h.commit().map(str::to_string))
    }

    // A reftable repo's HEAD, through git.
    fn head_by_command(&self) -> Option<Head> {
        let top = self.top.to_string_lossy();
        let name = git(&["-C", &top, "symbolic-ref", "--quiet", "--short", "HEAD"]);
        let commit = git(&["-C", &top, "rev-parse", "--verify", "--quiet", "HEAD"]).filter(|c| is_hash(c));
        match name {
            Some(name) => Some(Head::Branch { name, commit }),
            None => commit.map(|commit| Head::Detached { commit }),
        }
    }

    /// Its remotes, in the order its config lists them.
    pub fn remotes(&self) -> Vec<Remote> {
        small(&self.common_dir.join("config"), CONFIG_MAX).map(|c| remotes(&c)).unwrap_or_default()
    }

    /// origin's URL, as the map has read a repo's remote (projects.rs).
    pub fn origin(&self) -> Option<String> {
        self.remotes().into_iter().find(|r| r.name == "origin").map(|r| r.url)
    }

    /// origin's URL, else the first remote's, as the network names a repo
    /// (xenonet-engine's first_remote).
    pub fn remote(&self) -> Option<String> {
        let all = self.remotes();
        all.iter().find(|r| r.name == "origin").or(all.first()).map(|r| r.url.clone())
    }

    /// Its repo key: origin's URL normalized, which a project's id is made
    /// from. None with no origin.
    pub fn key(&self) -> Option<String> {
        repo_key(&self.origin()?)
    }

    /// What to watch for its head and refs to move: HEAD in its own git
    /// folder, the refs, packed-refs and config in the shared one. Never
    /// the index, which every `git status` rewrites.
    pub fn watch(&self) -> Vec<WatchSpec> {
        let mut out = Vec::new();
        if self.git_dir == self.common_dir {
            out.push(WatchSpec::flat(HEAD_TAG, &self.git_dir, &["HEAD", "packed-refs", "config"]));
        } else {
            out.push(WatchSpec::flat(HEAD_TAG, &self.git_dir, &["HEAD"]));
            out.push(WatchSpec::flat(COMMON_TAG, &self.common_dir, &["packed-refs", "config"]));
        }
        // A ref being written is <ref>.lock first, then renamed: the
        // rename's new name is the ref.
        out.push(WatchSpec { tag: REFS_TAG, root: self.common_dir.join("refs"), recursive: true, include: Vec::new() });
        out
    }
}

/// A remote's URL as its repo key: host and path, lowercased, without a
/// user, a scheme or `.git` (`github.com/willctl/xenonet`). None when that
/// leaves nothing.
pub fn repo_key(url: &str) -> Option<String> {
    let key = xenonet_model::ids::normalize_remote(url);
    (!key.is_empty()).then_some(key)
}

/// The remotes a git config lists: each `[remote "<name>"]` section's
/// first `url`, in order.
pub fn remotes(config: &str) -> Vec<Remote> {
    let mut out: Vec<Remote> = Vec::new();
    let mut section: Option<String> = None;
    for raw in config.lines() {
        let line = raw.trim();
        if let Some(head) = line.strip_prefix('[') {
            section = head.split_once(']').and_then(|(h, _)| remote_section(h));
            continue;
        }
        let Some(name) = &section else { continue };
        let Some((key, value)) = line.split_once('=') else { continue };
        if !key.trim().eq_ignore_ascii_case("url") || out.iter().any(|r| r.name == *name) {
            continue;
        }
        let url = value_of(value);
        if !url.is_empty() {
            out.push(Remote { name: name.clone(), url });
        }
    }
    out
}

// `remote "origin"` (or the old `remote.origin`): the remote's name.
fn remote_section(head: &str) -> Option<String> {
    let h = head.trim();
    if let Some(rest) = h.strip_prefix("remote") {
        let rest = rest.trim_start();
        if let Some(q) = rest.strip_prefix('"') {
            return q.strip_suffix('"').map(|n| n.replace("\\\"", "\"").replace("\\\\", "\\"));
        }
        if let Some(dot) = rest.strip_prefix('.') {
            return Some(dot.to_string()).filter(|n| !n.is_empty());
        }
    }
    None
}

// A config value: quotes taken off, a comment after it dropped.
fn value_of(v: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let mut chars = v.trim().chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(match n {
                        't' => '\t',
                        'n' => '\n',
                        other => other,
                    });
                }
            }
            '#' | ';' if !quoted => break,
            c => out.push(c),
        }
    }
    out.trim().to_string()
}

// git's answer, trimmed, or None when it failed or said nothing. Without a
// window, and with no lazy fetch, so asking never reaches a network.
fn git(args: &[&str]) -> Option<String> {
    use xenonet_os::process::Background as _;
    let out = std::process::Command::new("git")
        .no_window()
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests;
