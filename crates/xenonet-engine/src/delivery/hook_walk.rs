//! Looking through a project folder for what Claude Code would load from a
//! `.claude` folder and run outside its sandbox: hooks, plugins and servers
//! of an agent's own. Claude Code loads what is in `.claude` (the folder it
//! starts in, and any under it) whatever `--setting-sources` leaves out, and
//! a hook, or a skill that is a plugin, runs as Claude Code's own process,
//! outside the sandbox that wraps its commands.
//!
//! The walk guesses what Claude Code will load, so it fails closed: a name
//! is compared with its case ignored (the disk may ignore it), an entry or
//! a folder that can't be read is a refusal, not an empty folder, and a
//! front matter that can't be read as plain `key: value` lines is a
//! refusal, not a file without hooks.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// The deepest a folder is looked into, and the most entries looked at:
/// past either, the check says so and refuses.
const DEPTH: usize = 16;
const ENTRIES: usize = 50_000;

/// What the walk asks of the disk, so a test can make one entry vanish or
/// one folder fail.
trait Disk {
    /// The entries of a folder, every one of them: an entry that can't be
    /// read is an error, not left out.
    fn list(&self, d: &Path) -> io::Result<Vec<PathBuf>>;
    /// An entry itself, a link not followed.
    fn meta(&self, p: &Path) -> io::Result<std::fs::Metadata>;
    /// Where a path resolves, links followed.
    fn real(&self, p: &Path) -> io::Result<PathBuf>;
    fn read(&self, p: &Path) -> io::Result<Vec<u8>>;
}

struct Real;

impl Disk for Real {
    fn list(&self, d: &Path) -> io::Result<Vec<PathBuf>> {
        let mut v = std::fs::read_dir(d)?.map(|e| e.map(|e| e.path())).collect::<io::Result<Vec<_>>>()?;
        v.sort();
        Ok(v)
    }
    fn meta(&self, p: &Path) -> io::Result<std::fs::Metadata> {
        std::fs::symlink_metadata(p)
    }
    fn real(&self, p: &Path) -> io::Result<PathBuf> {
        p.canonicalize()
    }
    fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(p)
    }
}

/// Why a Claude Code run mustn't be sandboxed in `dir`, if a `.claude`
/// folder in it, at any depth, holds skills, agents, commands or plugins
/// that carry hooks or servers. A link to a folder is read where it points,
/// and one that points out of the folder is refused, as is a link inside a
/// `.claude` that does. Past what it will look at (too deep, or too many
/// entries) it refuses in words, and so it does for anything it can't read.
pub fn project_hooks(dir: &Path) -> Option<String> {
    project_hooks_within(dir, DEPTH, ENTRIES)
}

/// `project_hooks` with its limits given.
pub(super) fn project_hooks_within(dir: &Path, max_depth: usize, max_entries: usize) -> Option<String> {
    project_hooks_on(&Real, dir, max_depth, max_entries)
}

fn project_hooks_on(disk: &dyn Disk, dir: &Path, max_depth: usize, max_entries: usize) -> Option<String> {
    let root = disk.real(dir).unwrap_or_else(|_| dir.to_path_buf());
    Walk { disk, dir, root, max_depth, max_entries, seen: 0, outer: HashSet::new(), inner: HashSet::new() }.run().err()
}

/// What an entry is, with a link followed.
enum Kind {
    File(PathBuf),
    /// A folder, where it really is, and whether it was reached by a link.
    Dir(PathBuf, bool),
    /// A link to nothing: nothing for Claude Code to load either.
    Dangling,
}

struct Walk<'a> {
    disk: &'a dyn Disk,
    dir: &'a Path,
    root: PathBuf,
    max_depth: usize,
    max_entries: usize,
    seen: usize,
    /// The folders already looked through for `.claude`, and the ones
    /// already looked through as part of one, by where they really are.
    outer: HashSet<PathBuf>,
    inner: HashSet<PathBuf>,
}

impl Walk<'_> {
    fn shown(&self, p: &Path) -> String {
        p.strip_prefix(self.dir).unwrap_or(p).to_string_lossy().into_owned()
    }

    fn words(&self, p: &Path) -> String {
        format!(
            "{} carries hooks, servers of its own or is a plugin, which Claude Code runs outside the sandbox that holds its commands, so a sandboxed run isn't started in this folder: use another folder, or take them out",
            self.shown(p)
        )
    }

    fn linked(&self, p: &Path) -> String {
        format!(
            "{} is a link, in or as a .claude folder, which a command in the sandbox could write through to what it points at, so a sandboxed run isn't started in this folder: make it a plain folder or file, or take it out",
            self.shown(p)
        )
    }

    fn too_much(&self) -> String {
        format!(
            "this folder has too much to check for hooks in its .claude folders (more than {} entries, or folders more than {} deep), so a sandboxed run isn't started in it: use a smaller folder",
            self.max_entries, self.max_depth
        )
    }

    fn outside(&self, p: &Path) -> String {
        format!(
            "{} is a link that leads outside the folder, where it can't be checked for hooks, so a sandboxed run isn't started in this folder: take the link out",
            self.shown(p)
        )
    }

    fn unreadable(&self, p: &Path, why: &io::Error) -> String {
        format!(
            "{} couldn't be read ({why}), so it can't be checked for hooks and a sandboxed run isn't started in this folder: make it readable, take it out or use another folder",
            if p == self.dir { "the folder".to_string() } else { self.shown(p) }
        )
    }

    fn entry(&mut self) -> Result<(), String> {
        self.seen += 1;
        if self.seen > self.max_entries {
            return Err(self.too_much());
        }
        Ok(())
    }

    /// The entries of the folder read at `at`, shown as `shown_at`.
    fn list(&self, at: &Path, shown_at: &Path) -> Result<Vec<(PathBuf, String, PathBuf)>, String> {
        let entries = self.disk.list(at).map_err(|e| self.unreadable(shown_at, &e))?;
        let mut out = Vec::new();
        for f in entries {
            let Some(name) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            let shown_f = shown_at.join(&name);
            out.push((f, name, shown_f));
        }
        Ok(out)
    }

    /// Whether `f` is a link, dangling or not.
    fn is_link(&self, f: &Path, shown_f: &Path) -> Result<bool, String> {
        Ok(self.disk.meta(f).map_err(|e| self.unreadable(shown_f, &e))?.file_type().is_symlink())
    }

    /// What `f` is. A link that leaves the folder is a refusal when it
    /// leads to a folder, or to anything at all if `any` says so.
    fn kind(&self, f: &Path, shown_f: &Path, any: bool) -> Result<Kind, String> {
        let m = self.disk.meta(f).map_err(|e| self.unreadable(shown_f, &e))?;
        if !m.file_type().is_symlink() {
            return if m.is_dir() { Ok(Kind::Dir(self.disk.real(f).map_err(|e| self.unreadable(shown_f, &e))?, false)) } else { Ok(Kind::File(f.to_path_buf())) };
        }
        let t = match self.disk.real(f) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Kind::Dangling),
            Err(e) => return Err(self.unreadable(shown_f, &e)),
        };
        let dir = self.disk.meta(&t).map_err(|e| self.unreadable(shown_f, &e))?.is_dir();
        if !t.starts_with(&self.root) && (dir || any) {
            return Err(self.outside(shown_f));
        }
        Ok(if dir { Kind::Dir(t, true) } else { Kind::File(t) })
    }

    fn run(&mut self) -> Result<(), String> {
        // (read at, shown as, depth, how many levels of folders below it
        // are looked into, and whether it is a node_modules.) Every folder
        // is looked into for `.claude`, and node_modules only two levels
        // down (a package, or a scope's package), which is where a
        // package's own .claude is.
        let mut todo = vec![(self.dir.to_path_buf(), self.dir.to_path_buf(), 0usize, usize::MAX, false)];
        while let Some((d, shown_d, depth, below, in_modules)) = todo.pop() {
            let real = self.disk.real(&d).unwrap_or_else(|_| d.clone());
            if !self.outer.insert(real) {
                continue;
            }
            let entries = match self.list(&d, &shown_d) {
                // The folder named for the run isn't there: nothing to check.
                // Any other error on it (it can't be read) is a refusal.
                Err(_) if depth == 0 && matches!(self.disk.meta(&d), Err(e) if e.kind() == io::ErrorKind::NotFound) => return Ok(()),
                other => other?,
            };
            for (f, name, shown_f) in entries {
                self.entry()?;
                let lname = name.to_lowercase();
                if lname == ".claude" && self.is_link(&f, &shown_f)? {
                    return Err(self.linked(&shown_f));
                }
                let kind = self.kind(&f, &shown_f, false)?;
                if lname == ".claude" {
                    if let Kind::Dir(t, _) = kind {
                        self.claude(&t, &shown_f)?;
                    }
                    continue;
                }
                let Kind::Dir(t, linked) = kind else { continue };
                if !linked && lname == ".git" {
                    continue;
                }
                if depth + 1 > self.max_depth {
                    return Err(self.too_much());
                }
                // A link is read where it points, whatever it points into.
                let item = if linked {
                    (t, shown_f, depth + 1, usize::MAX, false)
                } else if lname == "node_modules" {
                    (t, shown_f, depth + 1, 1, true)
                } else if below == 0 {
                    continue;
                } else if below == usize::MAX || (in_modules && name.starts_with('@')) {
                    // A scope's packages are one level further down.
                    (t, shown_f, depth + 1, below, false)
                } else {
                    (t, shown_f, depth + 1, below - 1, false)
                };
                todo.push(item);
            }
        }
        Ok(())
    }

    /// What one `.claude` folder (read at `at`) holds.
    fn claude(&mut self, at: &Path, shown_at: &Path) -> Result<(), String> {
        let mut todo = vec![(at.to_path_buf(), shown_at.to_path_buf(), 0usize)];
        while let Some((d, shown_d, depth)) = todo.pop() {
            let real = self.disk.real(&d).map_err(|e| self.unreadable(&shown_d, &e))?;
            if !self.inner.insert(real) {
                continue;
            }
            for (f, name, shown_f) in self.list(&d, &shown_d)? {
                self.entry()?;
                let lname = name.to_lowercase();
                if self.is_link(&f, &shown_f)? {
                    return Err(self.linked(&shown_f));
                }
                let kind = self.kind(&f, &shown_f, true)?;
                if matches!(lname.as_str(), "hooks" | "hooks.json" | "plugin.json" | ".claude-plugin") {
                    return Err(self.words(&shown_f));
                }
                match kind {
                    Kind::Dir(t, _) => {
                        if depth + 1 > self.max_depth {
                            return Err(self.too_much());
                        }
                        todo.push((t, shown_f, depth + 1));
                    }
                    Kind::File(t) if lname.as_bytes().ends_with(b".md") => {
                        let bytes = self.disk.read(&t).map_err(|e| self.unreadable(&shown_f, &e))?;
                        match front_matter(&bytes) {
                            Front::Plain => {}
                            Front::Carries => return Err(self.words(&shown_f)),
                            Front::Unreadable => {
                                return Err(format!(
                                    "{} has front matter that can't be read as plain key: value lines, so it can't be checked for hooks and a sandboxed run isn't started in this folder: write it as plain lines, take it out or use another folder",
                                    self.shown(&shown_f)
                                ))
                            }
                        }
                    }
                    Kind::File(_) | Kind::Dangling => {}
                }
            }
        }
        Ok(())
    }
}

/// What a markdown file's front matter says.
#[derive(Debug, PartialEq)]
enum Front {
    /// None, or nothing in it that Claude Code runs.
    Plain,
    /// A `hooks`, `mcpServers` or `mcp_servers` key, anywhere in it.
    Carries,
    /// A line that isn't a plain `key: value` one (or a continuation of
    /// one), or no closing line: it can't be told what it holds.
    Unreadable,
}

/// The front matter of a file's bytes (read lossily, a byte order mark and
/// any lines before it left off): between a first `---` line and the next.
fn front_matter(bytes: &[u8]) -> Front {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    if lines.next().is_none_or(|l| l.trim() != "---") {
        return Front::Plain;
    }
    let mut body = Vec::new();
    let mut closed = false;
    for l in lines {
        if l.trim() == "---" {
            closed = true;
            break;
        }
        body.push(l);
    }
    let carries = crate::re!(r#"(?i)^[ \t]*(-[ \t]+)*['"]?(hooks|mcp_?servers)['"]?[ \t]*:"#);
    if body.iter().any(|l| carries.is_match(l)) {
        return Front::Carries;
    }
    let plain = crate::re!(r#"^['"]?[A-Za-z0-9_.\-]+['"]?[ \t]*:([ \t].*)?$"#);
    // An indented line or a list item is read only as part of a plain key
    // at column 0 before it: YAML also takes a whole mapping indented as
    // top level, and keys written as `?`, `{…}`, tags, anchors, aliases or
    // merges, none of which the key check above reads.
    let mut keyed = false;
    let fine = |l: &&str| {
        let t = l.trim();
        if t.is_empty() || l.starts_with('#') {
            return true;
        }
        if l.starts_with([' ', '\t', '-']) {
            return keyed && !t.starts_with(['{', '?', '!', '&', '*']) && !t.starts_with("<<");
        }
        keyed = plain.is_match(l);
        keyed
    };
    if closed && body.iter().all(fine) {
        Front::Plain
    } else {
        Front::Unreadable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(name: &str) -> PathBuf {
        let b = std::env::temp_dir().join(format!("xenonet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&b);
        std::fs::create_dir_all(&b).unwrap();
        b
    }

    fn write(base: &Path, name: &str, text: &str) {
        let f = base.join(name);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, text).unwrap();
    }

    const PLAIN: &str = "---\nname: x\n---\nx\n";
    const HOOKED: &str = "---\nname: x\nhooks:\n  Stop: []\n---\nx\n";

    #[test]
    fn a_project_with_skills_or_agents_that_carry_hooks_is_not_sandboxed_for_claude_code() {
        let base = base("hooks");
        // Skills and agents without hooks load beside the sandbox harmlessly.
        write(&base, ".claude/skills/review/SKILL.md", "---\nname: review\ndescription: Reads a diff\n---\nLook at the diff.\n");
        write(&base, ".claude/agents/helper.md", "---\nname: helper\n---\nHelp.\n");
        write(&base, "README.md", "hooks: are described here\n");
        assert_eq!(project_hooks(&base), None, "no hooks in them");
        assert_eq!(project_hooks(&base.join("nowhere")), None);
        for (name, text) in [
            (".claude/skills/fmt/SKILL.md", "---\nname: fmt\nhooks:\n  PostToolUse:\n    - command: ./fmt.sh\n---\nFormat.\n"),
            (".claude/agents/guard.md", "---\nname: guard\ntools: Read\nhooks:\n  Stop: []\n---\nGuard.\n"),
            (".claude/commands/ship.md", "---\nhooks:\n  Stop: []\n---\nShip.\n"),
            (".claude/skills/tool/hooks/hooks.json", "{}"),
            (".claude/skills/pack/.claude-plugin/plugin.json", "{\"name\":\"pack\"}"),
            (".claude/skills/pack2/plugin.json", "{\"name\":\"pack2\"}"),
        ] {
            let _ = std::fs::remove_dir_all(base.join(".claude"));
            write(&base, ".claude/skills/review/SKILL.md", "---\nname: review\n---\nok\n");
            write(&base, name, text);
            let why = project_hooks(&base).unwrap_or_default();
            assert!(why.starts_with(".claude/"), "{name}: {why}");
            assert!(why.contains("hooks") && why.contains("outside the sandbox"), "{name}: {why}");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    #[cfg(unix)]
    fn hooks_are_found_in_every_claude_folder_at_any_depth_and_in_a_linked_one() {
        let base = base("hooks-deep");
        // Plain skills, nested and not, run.
        write(&base, ".claude/skills/review/SKILL.md", PLAIN);
        write(&base, "pkg/web/.claude/skills/fmt/SKILL.md", PLAIN);
        write(&base, ".claude/skills/review/scripts/run.sh", "echo hi\n");
        assert_eq!(project_hooks(&base), None, "plain skills in nested folders run");
        for (name, text, shown) in [
            (".claude/commands/sub/deeper/x.md", HOOKED, ".claude/commands/sub/deeper/x.md"),
            ("pkg/web/.claude/skills/fmt/SKILL.md", HOOKED, "pkg/web/.claude/skills/fmt/SKILL.md"),
            ("pkg/web/.claude/agents/team/guard.md", HOOKED, "pkg/web/.claude/agents/team/guard.md"),
            (".claude/plugins/tools/plugin.json", "{}", ".claude/plugins/tools/plugin.json"),
            (".claude/skills/review/refs/hooks/on-stop.sh", "echo\n", ".claude/skills/review/refs/hooks"),
        ] {
            write(&base, name, text);
            let why = project_hooks(&base).unwrap_or_default();
            assert!(why.starts_with(shown) && why.contains("outside the sandbox"), "{name}: {why}");
            let _ = std::fs::remove_file(base.join(name));
            let _ = std::fs::remove_dir_all(base.join(".claude/skills/review/refs"));
            let _ = std::fs::remove_dir_all(base.join(".claude/plugins"));
            let _ = std::fs::remove_dir_all(base.join(".claude/commands"));
            let _ = std::fs::remove_dir_all(base.join("pkg/web/.claude/agents"));
            write(&base, "pkg/web/.claude/skills/fmt/SKILL.md", PLAIN);
            assert_eq!(project_hooks(&base), None, "{name}: back to plain");
        }
        // A `.claude` that is a link is a refusal wherever it leads: the
        // sandbox's denyWrite names `.claude` as written, and a command could
        // write through the link to what it points at.
        let outside = base.with_file_name(format!("xenonet-hooks-outside-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(outside.join("skills/ok")).unwrap();
        std::fs::write(outside.join("skills/ok/SKILL.md"), PLAIN).unwrap();
        std::fs::create_dir_all(base.join("linked")).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("linked/.claude")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.starts_with("linked/.claude") && why.contains("is a link"), "{why}");
        std::fs::remove_file(base.join("linked/.claude")).unwrap();
        write(&base, "shared/skills/ok/SKILL.md", PLAIN);
        std::os::unix::fs::symlink(base.join("shared"), base.join("linked/.claude")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.starts_with("linked/.claude") && why.contains("is a link"), "a link to a plain folder inside: {why}");
        std::fs::remove_file(base.join("linked/.claude")).unwrap();
        // A link inside a `.claude`, wherever it leads, is a refusal too.
        write(&base, "linked/.claude/skills/ok/SKILL.md", PLAIN);
        assert_eq!(project_hooks(&base), None, "a plain .claude folder runs");
        std::os::unix::fs::symlink(base.join("shared/skills/ok"), base.join("linked/.claude/skills/here")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.contains("skills/here") && why.contains("is a link"), "{why}");
        std::fs::remove_file(base.join("linked/.claude/skills/here")).unwrap();
        std::os::unix::fs::symlink(&outside, base.join("linked/.claude/skills/away")).unwrap();
        let why = project_hooks(&base).unwrap_or_default();
        assert!(why.contains("away") && why.contains("is a link"), "{why}");
        std::fs::remove_file(base.join("linked/.claude/skills/away")).unwrap();
        assert_eq!(project_hooks(&base), None);
        // Past what it will look at, it says so and refuses.
        write(&base, "a/b/c/d/e/f/g.txt", "x");
        let why = project_hooks_within(&base, 3, 100_000).unwrap_or_default();
        assert!(why.contains("too much to check"), "deeper than it looks: {why}");
        let why = project_hooks_within(&base, 16, 5).unwrap_or_default();
        assert!(why.contains("too much to check"), "more files than it looks at: {why}");
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn names_are_compared_with_case_ignored() {
        for (name, text) in [
            (".CLAUDE/skills/x/SKILL.md", HOOKED),
            (".Claude/agents/guard.MD", HOOKED),
            ("pkg/.CLAUDE/skills/x/SKILL.MD", HOOKED),
            (".claude/Hooks/on-stop.sh", "echo\n"),
            (".claude/skills/x/HOOKS.JSON", "{}"),
            (".claude/skills/x/PLUGIN.JSON", "{}"),
            (".claude/skills/x/.Claude-Plugin/plugin.txt", "{}"),
            ("node_modules/pkg/.CLAUDE/hooks.json", "{}"),
        ] {
            let b = base("case");
            write(&b, name, text);
            let why = project_hooks(&b).unwrap_or_default();
            assert!(why.contains("outside the sandbox"), "{name}: {why}");
            let _ = std::fs::remove_dir_all(&b);
        }
        // Folders skipped by name are skipped with case ignored too.
        let b = base("case-git");
        write(&b, ".GIT/modules/x/.claude/hooks.json", "{}");
        assert_eq!(project_hooks(&b), None, ".git is skipped, whatever its case");
        let _ = std::fs::remove_dir_all(&b);
    }

    /// A disk that fails where it is told to, otherwise the real one.
    struct Flaky {
        list: Option<&'static str>,
        meta: Option<&'static str>,
        read: Option<&'static str>,
        gone_after_list: Option<&'static str>,
    }

    fn hit(at: Option<&'static str>, p: &Path) -> bool {
        at.is_some_and(|n| p.file_name().is_some_and(|f| f == n))
    }

    impl Disk for Flaky {
        fn list(&self, d: &Path) -> io::Result<Vec<PathBuf>> {
            if hit(self.list, d) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Real.list(d)
        }
        fn meta(&self, p: &Path) -> io::Result<std::fs::Metadata> {
            if hit(self.meta, p) || hit(self.gone_after_list, p) {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            }
            Real.meta(p)
        }
        fn real(&self, p: &Path) -> io::Result<PathBuf> {
            Real.real(p)
        }
        fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
            if hit(self.read, p) {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Real.read(p)
        }
    }

    const NONE: Flaky = Flaky { list: None, meta: None, read: None, gone_after_list: None };

    #[test]
    fn an_error_reading_an_entry_or_a_folder_refuses_the_start() {
        let b = base("errors");
        write(&b, ".claude/skills/x/SKILL.md", PLAIN);
        write(&b, "pkg/src/a.txt", "x");
        assert_eq!(project_hooks_on(&NONE, &b, 16, 1000), None);
        for (disk, what) in [
            (Flaky { list: Some("skills"), ..NONE }, "a folder in a .claude that can't be listed"),
            (Flaky { list: Some("src"), ..NONE }, "a folder in the project that can't be listed"),
            (Flaky { list: Some(".claude"), ..NONE }, "the .claude folder itself"),
            (Flaky { gone_after_list: Some("pkg"), ..NONE }, "an entry that vanished after its folder was listed"),
            (Flaky { gone_after_list: Some("SKILL.md"), ..NONE }, "a file in .claude that vanished"),
            (Flaky { read: Some("SKILL.md"), ..NONE }, "a markdown file that can't be read"),
        ] {
            let why = project_hooks_on(&disk, &b, 16, 1000).unwrap_or_default();
            assert!(why.contains("couldn't be read") && why.contains("sandboxed run isn't started"), "{what}: {why}");
        }
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    #[cfg(unix)]
    fn an_unreadable_folder_is_a_refusal_not_an_empty_one() {
        use std::os::unix::fs::PermissionsExt;
        let b = base("chmod");
        write(&b, "pkg/locked/.claude/hooks.json", "{}");
        write(&b, ".claude/skills/x/SKILL.md", PLAIN);
        let locked = b.join("pkg/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let why = project_hooks(&b);
        let readable = std::fs::read_dir(&locked).is_ok();
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&b);
        if readable {
            return; // the test runs as root, which reads it anyway
        }
        let why = why.unwrap_or_default();
        assert!(why.starts_with("pkg/locked") && why.contains("couldn't be read"), "{why}");
    }

    #[test]
    #[cfg(unix)]
    fn a_link_to_a_folder_is_read_where_it_points_or_refused_if_that_is_outside() {
        let b = base("links");
        let outside = base("links-out");
        write(&outside, ".claude/hooks.json", "{}");
        // Out of the folder: refused, though nothing in it is looked at.
        std::os::unix::fs::symlink(&outside, b.join("vendor")).unwrap();
        let why = project_hooks(&b).unwrap_or_default();
        assert!(why.starts_with("vendor") && why.contains("outside the folder"), "{why}");
        std::fs::remove_file(b.join("vendor")).unwrap();
        // A link to a file out of the folder is no folder: left alone.
        write(&outside, "note.txt", "x");
        std::os::unix::fs::symlink(outside.join("note.txt"), b.join("note")).unwrap();
        assert_eq!(project_hooks(&b), None);
        // Into the folder, a link is walked, with its target once.
        write(&b, "real/.claude/skills/x/SKILL.md", HOOKED);
        std::os::unix::fs::symlink(b.join("real"), b.join("alias")).unwrap();
        let why = project_hooks(&b).unwrap_or_default();
        assert!(why.contains(".claude/skills/x/SKILL.md"), "{why}");
        std::fs::remove_dir_all(b.join("real")).unwrap();
        std::fs::remove_file(b.join("alias")).unwrap();
        // A loop is walked once.
        write(&b, "a/x.txt", "x");
        std::os::unix::fs::symlink(&b, b.join("a/up")).unwrap();
        assert_eq!(project_hooks(&b), None, "a link back up is looked at once");
        // A link into .git or into a deep node_modules is read where it points.
        write(&b, ".git/modules/m/.claude/hooks.json", "{}");
        std::os::unix::fs::symlink(b.join(".git/modules/m"), b.join("g")).unwrap();
        let why = project_hooks(&b).unwrap_or_default();
        assert!(why.contains("g/.claude/hooks.json") || why.contains("hooks.json"), "{why}");
        std::fs::remove_file(b.join("g")).unwrap();
        write(&b, "node_modules/a/b/c/.claude/hooks.json", "{}");
        assert_eq!(project_hooks(&b), None, "a deep node_modules is skipped by itself");
        std::os::unix::fs::symlink(b.join("node_modules/a/b/c"), b.join("n")).unwrap();
        let why = project_hooks(&b).unwrap_or_default();
        assert!(why.contains("n/.claude/hooks.json"), "{why}");
        let _ = std::fs::remove_dir_all(&b);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_dangling_link_holds_nothing_to_load() {
        #[cfg(unix)]
        {
            let b = base("dangling");
            std::os::unix::fs::symlink(b.join("gone"), b.join("x")).unwrap();
            write(&b, ".claude/skills/x/SKILL.md", PLAIN);
            assert_eq!(project_hooks(&b), None, "a dangling link outside .claude holds nothing to load");
            std::os::unix::fs::symlink(b.join("gone"), b.join(".claude/skills/y")).unwrap();
            let why = project_hooks(&b).unwrap_or_default();
            assert!(why.contains("skills/y") && why.contains("is a link"), "one inside .claude could be made to point at a folder later: {why}");
            let _ = std::fs::remove_dir_all(&b);
        }
    }

    #[test]
    fn a_folder_that_cant_be_looked_at_is_refused_and_only_a_missing_one_holds_nothing() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let b = base("root-unreadable");
            assert_eq!(project_hooks(&b.join("missing")), None, "a missing folder holds nothing to load");
            write(&b, "shut/x/a.txt", "x");
            std::fs::set_permissions(b.join("shut"), std::fs::Permissions::from_mode(0o000)).unwrap();
            let why = project_hooks(&b.join("shut/x"));
            std::fs::set_permissions(b.join("shut"), std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(why.is_some_and(|w| w.contains("couldn't be read")), "a folder under one that can't be read");
            let _ = std::fs::remove_dir_all(&b);
        }
    }

    #[test]
    fn a_claude_folder_that_is_a_link_is_refused_even_inside_or_dangling() {
        #[cfg(unix)]
        {
            let b = base("claude-link");
            write(&b, "y/agents/x.md", PLAIN);
            std::os::unix::fs::symlink(b.join("y"), b.join(".claude")).unwrap();
            let why = project_hooks(&b).unwrap_or_default();
            assert!(why.contains(".claude"), "a .claude linked inside the folder: {why}");
            std::fs::remove_file(b.join(".claude")).unwrap();
            std::os::unix::fs::symlink(b.join("gone"), b.join(".claude")).unwrap();
            let why = project_hooks(&b).unwrap_or_default();
            assert!(why.contains(".claude"), "a dangling .claude: {why}");
            std::fs::remove_file(b.join(".claude")).unwrap();
            write(&b, ".claude/agents/x.md", PLAIN);
            assert_eq!(project_hooks(&b), None, "a plain .claude folder holds nothing to refuse");
            let _ = std::fs::remove_dir_all(&b);
        }
    }

    #[test]
    fn front_matter_with_hooks_or_servers_is_found_in_every_form_and_what_cant_be_read_is_refused() {
        let carries = [
            "---\nname: x\nhooks:\n  Stop: []\n---\n",
            "---\nname: x\n  Hooks: {}\n---\n",
            "---\nname: x\n\"hooks\": {}\n---\n",
            "---\nname: x\n'hooks' : {}\n---\n",
            "---\nname: x\nmcpServers:\n  a: {}\n---\n",
            "---\nname: x\nMCPSERVERS: {}\n---\n",
            "---\nname: x\nmcp_servers:\n  a: {}\n---\n",
            "---\nname: x\n- hooks: {}\n---\n",
            "---\nname: x\ndescription: >\n  hooks: in prose, which is read as a key\n---\n",
            "\u{feff}---\nname: x\nhooks: {}\n---\n",
            "---\r\nname: x\r\nhooks:\r\n  Stop: []\r\n---\r\n",
            "\n\n---\nname: x\nhooks: {}\n---\n",
            "---  \nname: x\nhooks: {}\n---\t\n",
        ];
        for t in carries {
            assert_eq!(front_matter(t.as_bytes()), Front::Carries, "{t:?}");
        }
        // Not valid UTF-8: read lossily, the key is still there.
        let mut raw = b"---\nname: \xff\xfe\nhooks:\n  Stop: []\n---\n".to_vec();
        assert_eq!(front_matter(&raw), Front::Carries);
        raw.clear();
        for t in [
            "---\nname: x\n{hooks: {Stop: []}}\n---\n",
            // The whole mapping indented, which YAML reads as top level.
            "---\n  {hooks: {Stop: []}}\n---\n",
            "---\n  ? hooks\n  : {}\n---\n",
            "---\n  name: x\n---\n",
            "---\nname: x\n  ? hooks\n  : {}\n---\n",
            "---\nname: x\n  !!map {hooks: {}}\n---\n",
            "---\nname: x\n? hooks\n: {}\n---\n",
            "---\nname: x\nno colon here\n---\n",
            "---\nname: x\nhooks-not-closed: 1\n",
            "---\n[a, b]\n---\n",
        ] {
            assert_eq!(front_matter(t.as_bytes()), Front::Unreadable, "{t:?}");
        }
        for t in [
            "",
            "# Title\nhooks: are described here\n",
            "---\nname: x\ndescription: >\n  words\n  more\n---\nhooks:\n",
            "---\nname: x\ntools:\n- Read\n- Edit\n# a comment\n\n---\n",
            "---\nname: x\ndescription: a: b\n---\n",
            "---\nname: \"x\"\nallowed-tools: Read\n---\n",
        ] {
            assert_eq!(front_matter(t.as_bytes()), Front::Plain, "{t:?}");
        }
    }

    #[test]
    fn a_markdown_file_with_servers_or_an_unreadable_front_matter_refuses_the_folder() {
        for (name, text, word) in [
            (".claude/agents/db.md", "---\nname: db\nmcpServers:\n  pg: {command: x}\n---\n", "servers"),
            (".claude/agents/db2.md", "---\nname: db\nmcp_servers:\n  pg: {}\n---\n", "servers"),
            (".claude/agents/odd.md", "---\n{name: db}\n---\n", "front matter"),
            (".claude/commands/open.md", "---\nname: x\n", "front matter"),
        ] {
            let b = base("md");
            write(&b, name, text);
            let why = project_hooks(&b).unwrap_or_default();
            assert!(why.starts_with(name) && why.contains(word), "{name}: {why}");
            let _ = std::fs::remove_dir_all(&b);
        }
    }

    #[test]
    fn a_scoped_package_in_node_modules_is_looked_at() {
        for (name, found) in [
            ("node_modules/@acme/tool/.claude/hooks.json", true),
            ("node_modules/plain/.claude/hooks.json", true),
            ("node_modules/@acme/.claude/hooks.json", true),
            ("node_modules/@acme/tool/lib/.claude/hooks.json", false),
            ("node_modules/plain/lib/.claude/hooks.json", false),
            ("node_modules/a/b/c/.claude/hooks.json", false),
        ] {
            let b = base("scoped");
            write(&b, name, "{}");
            assert_eq!(project_hooks(&b).is_some(), found, "{name}");
            let _ = std::fs::remove_dir_all(&b);
        }
    }
}
