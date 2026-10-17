//! core/projects.js: the projects under your projects folder.
//!
//! The scan, as projects.js does it: top-down, a level at a time, through
//! the scanner block's breadth-first walk (whoever runs the walk hands each
//! folder read to `Scan::look` and asks `Scan::enter` about its
//! subfolders), then bottom-up from where sessions ran, then the groups.
//! Everything after the walk reads the disk itself, with std::fs: the
//! folders it looks at, a repo's HEAD and config, a package.json, a
//! README. And the relations between projects: satellites, worktrees and
//! clones of one remote.

pub(crate) mod jsre;

use jsre::{basename, dirname, line_starts_with, read_text, relative, resolve_from};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use xenonet_base::{ids, json as js_json, jsstr, paths, text};
use xenonet_sources::lines::{prop, TypeError};

const STACK_MARKERS: [(&str, &str); 12] = [
    ("package.json", "node"),
    ("tsconfig.json", "typescript"),
    ("pyproject.toml", "python"),
    ("requirements.txt", "python"),
    ("Cargo.toml", "rust"),
    ("go.mod", "go"),
    ("Gemfile", "ruby"),
    ("composer.json", "php"),
    ("docker-compose.yml", "docker"),
    ("compose.yaml", "docker"),
    ("Dockerfile", "docker"),
    ("supabase", "supabase"),
];

/// A folder holding one of these is a repo: a project of its own.
pub const VCS_MARKERS: [&str; 10] = [".git", ".hg", ".jj", ".sl", ".svn", ".bzr", "_darcs", ".pijul", ".fslckout", "_FOSSIL_"];

/// A folder holding one of these is a monorepo's root. package.json,
/// Cargo.toml and pyproject.toml count only when they declare a workspace.
pub const WORKSPACE_FILES: [&str; 9] = ["pnpm-workspace.yaml", "lerna.json", "nx.json", "turbo.json", "rush.json", "go.work", "MODULE.bazel", "WORKSPACE", "WORKSPACE.bazel"];

// Below the top level, the scan never looks inside dependencies, build
// output or caches.
const SKIP_DIRS: [&str; 14] =
    ["node_modules", "bower_components", "vendor", "dist", "build", "out", "target", "coverage", "__pycache__", "venv", "Pods", "DerivedData", "site-packages", "cdk.out"];

// What the Codex pass may know of a folder: which of these well-known
// files it has. Never other file names.
const KNOWN_FILES: &[&str] = &[
    ".gitmodules",
    "Makefile",
    "CMakeLists.txt",
    "Package.swift",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "setup.py",
    "deno.json",
    "mix.exs",
    "uv.lock",
    "poetry.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "Cargo.lock",
    "go.sum",
    "index.html",
    "AGENTS.md",
    "CLAUDE.md",
    "LICENSE",
];

// The well-known files: those above, the VCS markers, the workspace files
// and the stack markers.
fn known_file(name: &str) -> bool {
    KNOWN_FILES.contains(&name) || VCS_MARKERS.contains(&name) || WORKSPACE_FILES.contains(&name) || STACK_MARKERS.iter().any(|(f, _)| *f == name)
}

/// How far and how wide the scan goes: levels below the projects folder,
/// folders read per scan, and submodules per repo.
pub const SCAN_DEPTH: f64 = 3.0;
pub const SCAN_MAX_DIRS: f64 = 2000.0;
pub const SCAN_MAX_SUBMODULES: f64 = 10.0;

// The bottom-up pass reads the same few folders on every refresh, so it
// remembers what it saw for a while.
const FACTS_TTL_MS: f64 = 5.0 * 60e3;

/// What one readdir says about a folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts {
    pub vcs: Option<&'static str>,
    pub workspace: Option<&'static str>,
    /// Its entries' names, in the order read, once each.
    pub names: Vec<String>,
    pub subdirs: Vec<String>,
    set: HashSet<String>,
}

impl Facts {
    pub fn has(&self, name: &str) -> bool {
        self.set.contains(name)
    }

    /// The facts of a folder from its entries (name, is a folder), and
    /// for a workspace's root, the files that say so.
    pub fn of(dir: &str, ents: &[(String, bool)]) -> Facts {
        let mut names = Vec::new();
        let mut set = HashSet::new();
        for (n, _) in ents {
            if set.insert(n.clone()) {
                names.push(n.clone());
            }
        }
        let subdirs = ents.iter().filter(|(_, d)| *d).map(|(n, _)| n.clone()).collect();
        let mut f = Facts { vcs: None, workspace: None, names, subdirs, set };
        f.vcs = VCS_MARKERS.iter().copied().find(|m| f.has(m));
        if f.vcs.is_none() {
            f.workspace = workspace_of(dir, &f);
        }
        f
    }
}

fn parsed(file: &str) -> Option<Value> {
    js_json::parse(&read_text(file, 65536).unwrap_or_else(|| "{}".into())).ok()
}

// Which file makes this folder a monorepo root, if any.
fn workspace_of(dir: &str, f: &Facts) -> Option<&'static str> {
    if let Some(file) = WORKSPACE_FILES.iter().copied().find(|w| f.has(w)) {
        return Some(file);
    }
    if f.has("package.json") {
        // JSON.parse(...).workspaces: null throws and counts as none.
        if let Some(Value::Object(m)) = parsed(&paths::join(dir, "package.json")) {
            if text::truthy(m.get("workspaces")) {
                return Some("package.json");
            }
        }
    }
    let declares = |file: &str, line: &str| f.has(file) && line_starts_with(&read_text(&paths::join(dir, file), 65536).unwrap_or_default(), line);
    if declares("Cargo.toml", "[workspace]") {
        return Some("Cargo.toml");
    }
    if declares("pyproject.toml", "[tool.uv.workspace]") {
        return Some("pyproject.toml");
    }
    None
}

/// What one readdir says about a folder; None when it can't be read.
pub fn folder_facts(dir: &str) -> Option<Facts> {
    let ents = paths::read_dir_sorted(Path::new(dir)).ok()?;
    let ents: Vec<(String, bool)> = ents.iter().map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir()))).collect();
    Some(Facts::of(dir, &ents))
}

/// What the bottom-up pass saw of each folder, for a while.
#[derive(Default)]
pub struct FactsCache {
    seen: HashMap<String, (Option<Facts>, f64)>,
}

impl FactsCache {
    pub fn clear(&mut self) {
        self.seen.clear();
    }

    fn get(&mut self, dir: &str, now: f64) -> Option<Facts> {
        let key = paths::path_key(dir).unwrap_or_default();
        if let Some((facts, at)) = self.seen.get(&key) {
            if now - at < FACTS_TTL_MS {
                return facts.clone();
            }
        }
        let facts = folder_facts(dir);
        self.seen.insert(key, (facts.clone(), now));
        facts
    }
}

// ---------- a repo ----------

/// A repo's branch and origin, and whether it's a linked worktree or a
/// submodule, read from its git dir.
#[derive(Clone, Debug, PartialEq)]
pub struct GitInfo {
    pub branch: Option<String>,
    pub origin: Option<String>,
    pub worktree_of: Option<String>,
    pub submodule_of: Option<String>,
}

impl GitInfo {
    pub fn to_json(&self) -> Value {
        json!({ "branch": self.branch, "origin": self.origin, "worktreeOf": self.worktree_of, "submoduleOf": self.submodule_of })
    }
}

// /^(.*)[\\/]\.git[\\/]modules[\\/]/: what comes before a git dir's
// .git/modules/, the last on its first line.
fn superproject(git_dir: &str) -> Option<String> {
    let line: &str = git_dir.split(jsre::is_term).next().unwrap_or("");
    let b = line.as_bytes();
    let sep = |c: u8| c == b'/' || c == b'\\';
    (0..b.len())
        .rev()
        .find(|&i| b.len() >= i + 14 && sep(b[i]) && &b[i + 1..i + 5] == b".git" && sep(b[i + 5]) && &b[i + 6..i + 13] == b"modules" && sep(b[i + 13]))
        .map(|i| line[..i].to_string())
}

/// What a repo's git dir says (gitInfo): None when its .git can't be read.
pub fn git_info(dir: &str) -> Option<GitInfo> {
    let mut git_dir = paths::join(dir, ".git");
    let st = std::fs::metadata(&git_dir).ok()?;
    if st.is_file() {
        // A worktree or submodule: `.git` is a file pointing at the git dir.
        let to = jsre::rest_after(&read_text(&git_dir, 4096).unwrap_or_default(), "gitdir:")?;
        git_dir = resolve_from(dir, text::js_trim(&to));
    }
    // A linked worktree's git dir names the main repo's .git in commondir.
    let common = read_text(&paths::join(&git_dir, "commondir"), 4096).map(|c| text::js_trim(&c).to_string());
    let common = common.filter(|c| !c.is_empty());
    let common_dir = common.as_ref().map_or_else(|| git_dir.clone(), |c| resolve_from(&git_dir, c));
    let head = read_text(&paths::join(&git_dir, "HEAD"), 4096).unwrap_or_default();
    let branch = match jsre::rest_after_both(&head, "ref:", "refs/heads/") {
        Some(b) => Some(text::js_trim(&b).to_string()),
        None => Some(jsstr::head(text::js_trim(&head), 12)).filter(|b| !b.is_empty()),
    };
    let cfg = read_text(&paths::join(&common_dir, "config"), 16384).unwrap_or_default();
    Some(GitInfo {
        branch,
        origin: jsre::origin_url(&cfg),
        worktree_of: common.as_ref().filter(|_| basename(&common_dir) == ".git").map(|_| dirname(&common_dir)),
        submodule_of: if common.is_none() { superproject(&git_dir) } else { None },
    })
}

// ---------- what a folder is ----------

fn first_paragraph(md: Option<&str>) -> Option<String> {
    let md = md.filter(|m| !m.is_empty())?;
    for raw in md.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let line = text::js_trim(raw);
        if line.is_empty() || line.starts_with('#') || line.starts_with('<') || line.starts_with("![") || line.starts_with("[!") {
            continue;
        }
        let plain: String = line.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
        return Some(jsstr::head(&plain, 180));
    }
    None
}

// /^readme(\.md)?$/i, or with `any`, /^readme(\.\w+)?$/i.
fn is_readme(name: &str, any: bool) -> bool {
    let b = name.as_bytes();
    if b.len() < 6 || !b[..6].eq_ignore_ascii_case(b"readme") {
        return false;
    }
    let rest = &name[6..];
    if rest.is_empty() {
        return true;
    }
    if any {
        rest.len() > 1 && rest.starts_with('.') && rest[1..].bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    } else {
        rest.eq_ignore_ascii_case(".md")
    }
}

/// A folder's stack (from its marker files) and its description (its
/// package.json's, else its README's first paragraph).
pub fn describe(dir: &str, f: &Facts) -> (Vec<&'static str>, Value) {
    let mut stack: Vec<&'static str> = Vec::new();
    for (file, s) in STACK_MARKERS {
        if f.has(file) && !stack.contains(&s) {
            stack.push(s);
        }
    }
    let mut description = Value::Null;
    if f.has("package.json") {
        if let Some(Value::Object(m)) = parsed(&paths::join(dir, "package.json")) {
            description = m.get("description").filter(|d| !d.is_null()).cloned().unwrap_or(Value::Null);
        }
    }
    if !text::truthy(Some(&description)) {
        if let Some(readme) = f.names.iter().find(|n| is_readme(n, false)) {
            description = first_paragraph(read_text(&paths::join(dir, readme), 4096).as_deref()).map_or(Value::Null, Value::String);
        }
    }
    (stack, description)
}

// The folder's names and counts for the optional Codex pass: its
// well-known files, how many files and folders, its commonest file types.
fn outline(f: &Facts, out: &mut Map<String, Value>) {
    let subdirs: HashSet<&str> = f.subdirs.iter().map(String::as_str).collect();
    let files: Vec<&String> = f.names.iter().filter(|n| !subdirs.contains(n.as_str())).collect();
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for name in &files {
        if name.starts_with('.') {
            continue;
        }
        let Some(dot) = name.rfind('.') else { continue };
        let ext = &name[dot + 1..];
        if ext.is_empty() || ext.len() > 8 || !ext.bytes().all(|c| c.is_ascii_alphanumeric()) {
            continue;
        }
        let ext = ext.to_ascii_lowercase();
        match kinds.iter_mut().find(|(k, _)| *k == ext) {
            Some((_, n)) => *n += 1,
            None => kinds.push((ext, 1)),
        }
    }
    kinds.sort_by(|a, b| b.1.cmp(&a.1));
    let mut markers: Vec<String> = f.names.iter().filter(|n| known_file(n) || is_readme(n, true)).cloned().collect();
    markers.sort_by(|a, b| jsstr::cmp_units(a, b));
    out.insert("markers".into(), json!(markers));
    out.insert("files".into(), json!(files.len()));
    out.insert("folders".into(), json!(f.subdirs.iter().filter(|n| !n.starts_with('.')).count()));
    out.insert("kinds".into(), Value::Object(kinds.into_iter().take(5).map(|(k, n)| (k, json!(n))).collect()));
}

// ---------- the scan ----------

// A folder the scan has looked at. `why` is what makes it a project:
// "repo", "rule", "submodule", "workspace", "sessions" or "folder".
#[derive(Clone, Debug)]
struct Folder {
    path: String,
    key: String,
    depth: f64,
    facts: Facts,
    why: Option<&'static str>,
}

fn stops(f: &Folder) -> bool {
    matches!(f.why, Some("repo" | "rule" | "submodule"))
}

fn len16(s: &str) -> usize {
    jsstr::len(s)
}

// key.slice(0, key.lastIndexOf("/")).
fn parent_key(key: &str) -> String {
    match key.rfind('/') {
        Some(i) => key[..i].to_string(),
        None => jsstr::slice(key, 0, Some(-1)),
    }
}

fn null_length() -> TypeError {
    TypeError("Cannot read properties of null (reading 'length')".into())
}

/// The projects found, the folders looked at, and how many groups and
/// projects there are.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub projects: Vec<Value>,
    pub folders: Vec<Value>,
    pub groups: usize,
    pub count: usize,
}

/// One scan of a projects folder (see scanProjects in core/projects.js).
pub struct Scan {
    root: String,
    root_key: Option<String>,
    rules: Map<String, Value>,
    // The folders looked at, and by key, in the order first seen.
    folders: Vec<Folder>,
    order: Vec<String>,
    seen: HashMap<String, usize>,
    // folder → (its folder, whether it's inside a workspace), or None when
    // it's left out.
    walked: HashMap<String, Option<(usize, bool)>>,
    pub root_read: bool,
}

impl Scan {
    /// `rules`: path key → { path, as, by }.
    pub fn new(root: &str, rules: &Value) -> Scan {
        Scan {
            root: root.to_string(),
            root_key: paths::path_key(root),
            rules: rules.as_object().cloned().unwrap_or_default(),
            folders: Vec::new(),
            order: Vec::new(),
            seen: HashMap::new(),
            walked: HashMap::new(),
            root_read: false,
        }
    }

    // rules[key]?.as ?? null
    fn rule(&self, key: &str) -> Option<&Value> {
        match self.rules.get(key) {
            Some(Value::Object(r)) => r.get("as").filter(|v| !v.is_null()),
            _ => None,
        }
    }

    fn rule_is(&self, key: &str, what: &str) -> bool {
        self.rule(key).and_then(Value::as_str) == Some(what)
    }

    fn get(&self, key: &str) -> Option<&Folder> {
        self.seen.get(key).map(|&i| &self.folders[i])
    }

    fn set(&mut self, f: Folder) {
        let key = f.key.clone();
        self.folders.push(f);
        let i = self.folders.len() - 1;
        if self.seen.insert(key.clone(), i).is_none() {
            self.order.push(key);
        }
    }

    fn values(&self) -> Vec<usize> {
        self.order.iter().map(|k| self.seen[k]).collect()
    }

    /// A folder the walk read, with its entries (name, is a folder), at
    /// `level` below the root. `aborted`: the scan is no longer wanted.
    pub fn look(&mut self, dir: &str, ents: &[(String, bool)], level: f64, aborted: bool) {
        if level == 0.0 {
            self.root_read = true;
            return;
        }
        if aborted {
            return;
        }
        let key = paths::path_key(dir).unwrap_or_default();
        if self.rule_is(&key, "ignore") {
            self.walked.insert(dir.to_string(), None);
            return;
        }
        let facts = Facts::of(dir, ents);
        // A workspace inside another one is one of its packages.
        let in_workspace = matches!(self.walked.get(&dirname(dir)), Some(Some((_, true))));
        let why = if self.rule_is(&key, "project") {
            Some("rule")
        } else if self.rule_is(&key, "group") {
            None
        } else if facts.vcs.is_some() {
            Some("repo")
        } else if facts.workspace.is_some() && !in_workspace {
            Some("workspace")
        } else {
            None
        };
        self.set(Folder { path: dir.to_string(), key, depth: level, facts, why });
        let i = self.folders.len() - 1;
        self.walked.insert(dir.to_string(), Some((i, in_workspace || why == Some("workspace"))));
    }

    /// Whether the walk goes into a subfolder `name` of `dir`, at `level`.
    /// At the top level only dot folders and node_modules are passed over,
    /// below it dependencies, build output and caches too, and nothing
    /// inside a repo, a folder made one project, or one left out.
    pub fn enter(&self, name: &str, dir: &str, level: f64) -> bool {
        if name.starts_with('.') {
            return false;
        }
        if level == 1.0 {
            return name != "node_modules";
        }
        match self.walked.get(dir) {
            Some(Some((i, _))) => !stops(&self.folders[*i]) && !SKIP_DIRS.contains(&name),
            _ => false,
        }
    }

    fn root_key(&self) -> Result<&str, TypeError> {
        self.root_key.as_deref().ok_or_else(null_length)
    }

    fn ignored(&self, key: &str) -> Result<bool, TypeError> {
        let rl = len16(self.root_key()?);
        let mut k = key.to_string();
        while len16(&k) > rl {
            if self.rule_is(&k, "ignore") {
                return Ok(true);
            }
            k = parent_key(&k);
        }
        Ok(false)
    }

    fn depth_of(&self, key: &str) -> f64 {
        let rl = self.root_key.as_deref().map_or(0, len16);
        jsstr::slice(key, rl as i64, None).matches('/').count() as f64
    }

    // The nearest folder at or above `key` (or above it only) that `test`
    // picks.
    fn nearest(&self, key: &str, own: bool, test: &dyn Fn(&Folder) -> bool) -> Option<String> {
        let rl = self.root_key.as_deref().map_or(0, len16);
        let mut k = if own { key.to_string() } else { parent_key(key) };
        while len16(&k) > rl {
            if let Some(f) = self.get(&k) {
                if test(f) {
                    return Some(k);
                }
            }
            k = parent_key(&k);
        }
        None
    }

    fn add(&mut self, dir: Option<&str>, why: &'static str, facts: Option<Facts>) -> Result<(), TypeError> {
        let Some(key) = dir.and_then(paths::path_key) else {
            return Err(null_length());
        };
        let dir = dir.unwrap_or_default();
        if self.get(&key).is_some_and(stops) || self.ignored(&key)? {
            return Ok(());
        }
        let Some(facts) = facts.or_else(|| folder_facts(dir)) else { return Ok(()) };
        let depth = self.depth_of(&key);
        self.set(Folder { path: dir.to_string(), key, depth, facts, why: Some(why) });
        Ok(())
    }

    /// Everything after the walk: folders pinned as projects, submodules,
    /// the repos sessions ran in, the groups. `sessions`: the folders
    /// sessions ran in; `max_submodules` as the limits give it; `now` for
    /// the facts cache.
    pub fn finish(&mut self, sessions: &[Value], max_submodules: Option<&Value>, cache: &mut FactsCache, now: f64) -> Result<Found, TypeError> {
        let root_key = self.root_key()?.to_string();
        let rl = len16(&root_key);

        // Folders pinned as projects that the walk didn't reach. Codex's
        // picks inside a folder you made one project don't count.
        let rules: Vec<(String, Value)> = self.rules.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (key, r) in &rules {
            let as_ = prop(Some(r), "as")?;
            if as_.and_then(Value::as_str) != Some("project") || !paths::is_under(key, &root_key) || *key == root_key || self.get(key).is_some_and(|f| f.why.is_some()) {
                continue;
            }
            let by_codex = prop(Some(r), "by")?.and_then(Value::as_str) == Some("codex");
            if by_codex && self.yours_above(key) {
                continue;
            }
            let path = prop(Some(r), "path")?.filter(|p| text::truthy(Some(p))).map(text::js_string);
            self.add(path.as_deref(), "rule", None)?;
        }

        // Submodules are repos of their own, listed in .gitmodules.
        for i in self.values() {
            let f = self.folders[i].clone();
            if f.facts.vcs != Some(".git") || !f.facts.has(".gitmodules") || self.rule_is(&f.key, "project") {
                continue;
            }
            let listed = jsre::assignments(&read_text(&paths::join(&f.path, ".gitmodules"), 65536).unwrap_or_default(), "path");
            let n = slice_end(listed.len(), max_submodules);
            for rel in &listed[..n] {
                let dir = paths::join(&f.path, rel);
                if let Some(facts) = folder_facts(&dir).filter(|x| x.vcs.is_some()) {
                    self.add(Some(&dir), "submodule", Some(facts))?;
                }
            }
        }

        // ----- bottom-up, from where sessions ran -----
        let mut session_keys: Vec<(String, String)> = Vec::new(); // key → folder
        let mut session_count: Vec<(String, usize)> = Vec::new();
        for d in sessions {
            if !text::truthy(Some(d)) {
                continue;
            }
            let dir = text::js_string(d);
            let Some(key) = paths::path_key(&dir) else { continue };
            if !paths::is_under(&key, &root_key) || key == root_key || self.ignored(&key)? {
                continue;
            }
            match session_keys.iter_mut().find(|(k, _)| *k == key) {
                Some((_, at)) => *at = dir,
                None => session_keys.push((key.clone(), dir)),
            }
            match session_count.iter_mut().find(|(k, _)| *k == key) {
                Some((_, n)) => *n += 1,
                None => session_count.push((key, 1)),
            }
        }
        for (key, dir) in &session_keys {
            let repo = self.nearest(key, true, &stops);
            // A folder you made one project keeps everything in it.
            if repo.as_deref().is_some_and(|r| self.rule_is(r, "project")) {
                continue;
            }
            let inside = repo.clone().or_else(|| self.nearest(key, true, &|f| f.why == Some("workspace")));
            let (mut d, mut k) = (dir.clone(), key.clone());
            while len16(&k) > rl && Some(&k) != repo.as_ref() {
                let next = |d: &str, k: &str| (dirname(d), parent_key(k));
                if self.rule_is(&k, "group") || Some(&k) == inside.as_ref() {
                    (d, k) = next(&d, &k);
                    continue;
                }
                let facts = match self.get(&k) {
                    Some(f) => Some(f.facts.clone()),
                    None => cache.get(&d, now),
                };
                // Inside a repo or workspace only another repo counts.
                let counts = facts.as_ref().is_some_and(|f| f.vcs.is_some() || (inside.is_none() && f.workspace.is_some()));
                if !counts {
                    (d, k) = next(&d, &k);
                    continue;
                }
                let facts = facts.unwrap();
                let git = if facts.vcs == Some(".git") { git_info(&d) } else { None };
                if let Some(of) = git.as_ref().and_then(|g| g.worktree_of.as_deref()) {
                    if paths::path_key(of).is_some_and(|wk| paths::is_under(&k, &wk)) {
                        break;
                    }
                }
                let why = if facts.vcs.is_some() { "repo" } else { "workspace" };
                self.add(Some(&d), why, Some(facts))?;
                break;
            }
        }

        // ----- groups, and folders made projects by their sessions -----
        let mut groups = self.find_groups();
        for (key, dir) in &session_keys {
            let Some(home) = self.home_of(key, &groups) else { continue };
            if !groups.contains(&home) || home == *key {
                continue;
            }
            // home + key.slice(home.length).match(/^\/[^/]+/)[0]
            let below = &key[home.len()..];
            let seg = below.strip_prefix('/').map(|b| b.split('/').next().unwrap_or("")).filter(|s| !s.is_empty());
            let Some(seg) = seg else { return Err(TypeError("Cannot read properties of null (reading '0')".into())) };
            let child = format!("{home}/{seg}");
            if let Some(&i) = self.seen.get(&child) {
                self.folders[i].why = Some("sessions");
            } else {
                let at = spelled(dir, &child);
                self.add(at.as_deref(), "sessions", None)?;
            }
        }
        groups = self.find_groups();
        // Plain top-level folders stay projects.
        for i in self.values() {
            let f = &self.folders[i];
            if f.depth == 1.0 && f.why.is_none() && !groups.contains(&f.key) {
                self.folders[i].why = Some("folder");
            }
        }

        // A group that is only a step on the path to one thing is left out.
        let kept_order: Vec<String> = self.values().into_iter().map(|i| &self.folders[i]).filter(|f| f.why.is_some() || groups.contains(&f.key)).map(|f| f.key.clone()).collect();
        let mut kept: HashSet<String> = kept_order.iter().cloned().collect();
        let mut children: HashMap<String, usize> = HashMap::new();
        for key in &kept_order {
            if let Some(up) = self.nearest(key, false, &|f| kept.contains(&f.key)) {
                *children.entry(up).or_default() += 1;
            }
        }
        let own_sessions: HashSet<String> = session_keys.iter().filter_map(|(k, _)| self.home_of(k, &groups)).collect();
        let mut by_length: Vec<&String> = groups.iter().collect();
        by_length.sort_by_key(|g| std::cmp::Reverse(len16(g)));
        for key in by_length {
            let deep = self.get(key).is_some_and(|f| f.depth > 1.0);
            if deep && self.rule(key).is_none_or(|r| !text::truthy(Some(r))) && children.get(key) == Some(&1) && !own_sessions.contains(key) {
                kept.remove(key);
            }
        }

        let mut projects: Vec<(String, Value)> = Vec::new();
        for key in kept_order.iter().filter(|k| kept.contains(*k)) {
            let f = self.get(key).unwrap().clone();
            let group = groups.contains(key);
            let git = if f.facts.vcs == Some(".git") { git_info(&f.path).map(|g| g.to_json()) } else { None };
            let (stack, description) = describe(&f.path, &f.facts);
            let modified = std::fs::metadata(&f.path).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok());
            let detected =
                if group { Value::String((if self.rule_is(key, "group") { "rule" } else { "contains" }).into()) } else { f.why.map_or(Value::Null, |w| Value::String(w.into())) };
            let parent = self.nearest(key, false, &|x| kept.contains(&x.key));
            let mut p = Map::new();
            p.insert("name".into(), Value::String(basename(&f.path)));
            p.insert("path".into(), Value::String(f.path.clone()));
            p.insert("key".into(), Value::String(key.clone()));
            p.insert("rel".into(), Value::String(rel_of(&self.root, &f.path)));
            p.insert("role".into(), Value::String((if group { "group" } else { "project" }).into()));
            p.insert("detected".into(), detected);
            p.insert("marker".into(), f.facts.vcs.or(f.facts.workspace).map_or(Value::Null, |m| Value::String(m.into())));
            p.insert("parent".into(), parent.map_or(Value::Null, Value::String));
            p.insert("git".into(), git.unwrap_or(Value::Null));
            p.insert("stack".into(), json!(stack));
            p.insert("description".into(), description);
            p.insert("modifiedAt".into(), modified.map_or(Value::Null, js_json::number));
            projects.push((key.clone(), Value::Object(p)));
        }
        projects.sort_by(|a, b| jsstr::locale_compare(&a.0, &b.0));
        let groups_count = projects.iter().filter(|(_, p)| p["role"] == "group").count();
        let count = projects.len() - groups_count;

        // Every folder the scan looked at, as names and counts only, for
        // the optional Codex pass (see classify.js) to judge.
        let mut all: Vec<&Folder> = self.values().into_iter().map(|i| &self.folders[i]).collect();
        all.sort_by(|a, b| jsstr::locale_compare(&a.key, &b.key));
        let folders = all
            .into_iter()
            .map(|f| {
                let group = groups.contains(&f.key);
                let mut o = Map::new();
                o.insert("rel".into(), Value::String(rel_of(&self.root, &f.path)));
                o.insert("key".into(), Value::String(f.key.clone()));
                o.insert("path".into(), Value::String(f.path.clone()));
                o.insert("repo".into(), Value::Bool(f.facts.vcs.is_some()));
                o.insert(
                    "call".into(),
                    Value::String(
                        (if group {
                            "group"
                        } else if f.why.is_some() {
                            "project"
                        } else {
                            "folder"
                        })
                        .into(),
                    ),
                );
                o.insert("why".into(), if group { Value::String("contains".into()) } else { f.why.map_or(Value::Null, |w| Value::String(w.into())) });
                outline(&f.facts, &mut o);
                let sessions: usize = session_count.iter().filter(|(k, _)| paths::is_under(k, &f.key)).map(|(_, c)| c).sum();
                o.insert("sessions".into(), json!(sessions));
                Value::Object(o)
            })
            .collect();
        Ok(Found { projects: projects.into_iter().map(|(_, p)| p).collect(), folders, groups: groups_count, count })
    }

    fn yours_above(&self, key: &str) -> bool {
        let rl = self.root_key.as_deref().map_or(0, len16);
        let mut k = parent_key(key);
        while len16(&k) > rl {
            if let Some(Value::Object(r)) = self.rules.get(&k) {
                if r.get("as").and_then(Value::as_str) == Some("project") && r.get("by").and_then(Value::as_str) != Some("codex") {
                    return true;
                }
            }
            k = parent_key(&k);
        }
        false
    }

    // Every folder that holds a project (or that a rule makes a group), in
    // the order seen; never one that stops the walk.
    fn find_groups(&self) -> Vec<String> {
        let keys: Vec<&str> = self.values().into_iter().map(|i| &self.folders[i]).filter(|f| f.why.is_some()).map(|f| f.key.as_str()).collect();
        self.values()
            .into_iter()
            .map(|i| &self.folders[i])
            .filter(|f| !stops(f) && (self.rule_is(&f.key, "group") || keys.iter().any(|k| *k != f.key && paths::is_under(k, &f.key))))
            .map(|f| f.key.clone())
            .collect()
    }

    fn home_of(&self, key: &str, groups: &[String]) -> Option<String> {
        self.nearest(key, true, &|f| f.why.is_some() || groups.contains(&f.key))
    }
}

// The folder `key` names, spelled as on disk, found from a path inside it.
fn spelled(dir: &str, key: &str) -> Option<String> {
    let mut d = paths::resolve(dir);
    loop {
        if paths::path_key(&d).as_deref() == Some(key) {
            return Some(d);
        }
        let up = dirname(&d);
        if up == d {
            return None;
        }
        d = up;
    }
}

// path.relative(root, p).split(path.sep).join("/").
fn rel_of(root: &str, p: &str) -> String {
    relative(root, p)
}

// `xs.slice(0, end)`'s end, as JS reads it from a value.
fn slice_end(len: usize, end: Option<&Value>) -> usize {
    let Some(v) = end else { return len };
    let n = text::js_number(Some(v));
    if n.is_nan() {
        return 0;
    }
    let n = n.trunc();
    if n < 0.0 {
        (len as f64 + n).max(0.0) as usize
    } else {
        n.min(len as f64) as usize
    }
}

// ---------- relations ----------

/// Pairs of project keys.
pub type KeyPairs = Vec<(String, String)>;

/// Satellite projects hang off a parent: `ledger-local-supabase` orbits
/// `ledger` when both sit in the same folder. A linked worktree orbits its
/// repo. Clones of the same remote are linked as siblings. Returns
/// (project key → parent key, in order) and the pairs of keys that share
/// a remote.
pub fn relations(projects: &[Value]) -> Result<(KeyPairs, KeyPairs), TypeError> {
    let real: Vec<&Value> = projects.iter().filter(|p| p.get("role").and_then(Value::as_str) != Some("group")).collect();
    let key_of = |p: &Value| -> String { p.get("key").map(text::js_string).unwrap_or_else(|| "undefined".into()) };
    let parent_of = |p: &Value| -> String {
        match p.get("parent") {
            None | Some(Value::Null) => String::new(),
            Some(v) => text::js_string(v),
        }
    };
    let name_of = |p: &Value| -> Result<String, TypeError> {
        match p.get("name") {
            Some(Value::String(s)) => Ok(s.to_lowercase()),
            None => Err(TypeError("Cannot read properties of undefined (reading 'toLowerCase')".into())),
            Some(Value::Null) => Err(TypeError("Cannot read properties of null (reading 'toLowerCase')".into())),
            Some(_) => Err(TypeError("p.name.toLowerCase is not a function".into())),
        }
    };
    // Later projects take the place of earlier ones of the same name, as a
    // Map built from them does.
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut by_key: HashMap<String, usize> = HashMap::new();
    for (i, p) in real.iter().enumerate() {
        by_name.insert(format!("{}|{}", parent_of(p), name_of(p)?), i);
        by_key.insert(key_of(p), i);
    }
    let mut parents: Vec<(String, String)> = Vec::new();
    let mut set = |k: String, v: String| match parents.iter_mut().find(|(a, _)| *a == k) {
        Some((_, at)) => *at = v,
        None => parents.push((k, v)),
    };
    for (i, p) in real.iter().enumerate() {
        let wt = p.get("git").and_then(|g| g.get("worktreeOf")).filter(|w| text::truthy(Some(w)));
        let main = wt.and_then(|w| {
            let s = text::js_string(w);
            paths::path_key(&s).and_then(|k| by_key.get(&k).copied())
        });
        if let Some(m) = main.filter(|&m| m != i) {
            set(key_of(p), key_of(real[m]));
            continue;
        }
        let name = name_of(p)?;
        let parts: Vec<&str> = name.split(['-', '_', '.']).collect();
        for n in (1..parts.len()).rev() {
            let at = |sep: &str| by_name.get(&format!("{}|{}", parent_of(p), parts[..n].join(sep))).copied();
            if let Some(c) = at("-").or_else(|| at("_")).filter(|&c| c != i) {
                set(key_of(p), key_of(real[c]));
                break;
            }
        }
    }
    let mut same_remote = Vec::new();
    let mut by_remote: HashMap<String, usize> = HashMap::new();
    for (i, p) in real.iter().enumerate() {
        let origin = p.get("git").and_then(|g| g.get("origin"));
        let r = match origin {
            Some(o) if text::truthy(Some(o)) => match o {
                Value::String(s) => ids::normalize_remote(s),
                _ => return Err(TypeError("url.toLowerCase is not a function".into())),
            },
            _ => continue,
        };
        if r.is_empty() {
            continue;
        }
        match by_remote.get(&r) {
            Some(&first) => same_remote.push((key_of(real[first]), key_of(p))),
            None => {
                by_remote.insert(r, i);
            }
        }
    }
    Ok((parents, same_remote))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> String {
        let d = std::env::temp_dir().join(format!("xenonet-projects-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.to_string_lossy().into_owned()
    }

    fn repo(dir: &str) {
        fs::create_dir_all(format!("{dir}/.git")).unwrap();
        fs::write(format!("{dir}/.git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(format!("{dir}/.git/config"), "[remote \"origin\"]\n\turl = git@example.com:me/app.git\n").unwrap();
    }

    #[test]
    fn reads_a_repo() {
        let root = tmp("repo");
        repo(&format!("{root}/app"));
        let g = git_info(&format!("{root}/app")).unwrap();
        assert_eq!(g.branch.as_deref(), Some("main"));
        assert_eq!(g.origin.as_deref(), Some("git@example.com:me/app.git"));
        assert_eq!(g.worktree_of, None);
        assert_eq!(superproject("/p/game/.git/modules/art"), Some("/p/game".into()));
        assert_eq!(superproject("/p/game/.git/worktrees/art"), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn says_what_a_folder_is() {
        let root = tmp("facts");
        fs::write(format!("{root}/Cargo.toml"), "[package]\n\n[workspace]\n").unwrap();
        fs::write(format!("{root}/README.md"), "# Title\n\n![badge](x)\nThe *first* line.\n").unwrap();
        let f = folder_facts(&root).unwrap();
        assert_eq!(f.workspace, Some("Cargo.toml"));
        let (stack, desc) = describe(&root, &f);
        assert_eq!(stack, ["rust"]);
        assert_eq!(desc, "The first line.");
        let mut o = Map::new();
        outline(&f, &mut o);
        assert_eq!(o["markers"], json!(["Cargo.toml", "README.md"]));
        assert_eq!(o["kinds"], json!({ "toml": 1, "md": 1 }));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn walks_and_finds_groups() {
        let root = tmp("scan");
        repo(&format!("{root}/studio/app"));
        repo(&format!("{root}/ledger"));
        fs::write(format!("{root}/studio/NOTES.md"), "notes").unwrap();
        let mut scan = Scan::new(&root, &json!({}));
        scan.look(&root, &[("ledger".into(), true), ("studio".into(), true)], 0.0, false);
        let mut level = vec![format!("{root}/ledger"), format!("{root}/studio")];
        let mut depth = 1.0;
        while !level.is_empty() {
            let mut next = Vec::new();
            for dir in &level {
                let ents: Vec<(String, bool)> =
                    paths::read_dir_sorted(Path::new(dir)).unwrap().iter().map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().unwrap().is_dir())).collect();
                scan.look(dir, &ents, depth, false);
                for (n, d) in &ents {
                    if *d && scan.enter(n, dir, depth) {
                        next.push(paths::join(dir, n));
                    }
                }
            }
            level = next;
            depth += 1.0;
        }
        let found = scan.finish(&[], None, &mut FactsCache::default(), 0.0).unwrap();
        let shape: Vec<String> =
            found.projects.iter().map(|p| format!("{} {} {}", p["rel"].as_str().unwrap(), p["role"].as_str().unwrap(), p["detected"].as_str().unwrap())).collect();
        assert_eq!(shape, ["ledger project repo", "studio group contains", "studio/app project repo"]);
        assert_eq!((found.groups, found.count), (1, 2));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn relates_projects() {
        let p = |key: &str, name: &str, parent: Value, origin: Value| json!({ "key": key, "name": name, "parent": parent, "role": "project", "git": { "origin": origin } });
        let ps = [
            p("/p/ledger", "ledger", Value::Null, json!("git@github.com:a/l.git")),
            p("/p/ledger-db", "ledger-db", Value::Null, json!("https://github.com/a/l")),
            p("/p/misc/ledger-cli", "ledger-cli", json!("/p/misc"), Value::Null),
        ];
        let (parents, same) = relations(&ps).unwrap();
        assert_eq!(parents, [("/p/ledger-db".to_string(), "/p/ledger".to_string())]);
        assert_eq!(same, [("/p/ledger".to_string(), "/p/ledger-db".to_string())]);
    }
}
