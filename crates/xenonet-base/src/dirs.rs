//! Where XenoNet keeps its own files and where the desktop apps it reads
//! keep theirs (core/dirs.js). No side effects: whoever asks says the
//! platform, the home folder and the environment, and how to look at disk.

use crate::paths::join_all;

/// XenoNet's own folder of a kind ("state" or "cache"), following the XDG
/// base directories when they're set, otherwise the platform's standard
/// places; it never adds a dot dir to ~.
///
/// ```text
///   state (layout, dismissed alerts)          cache (search index, rebuildable)
///   Windows: %LOCALAPPDATA%\XenoNet           %LOCALAPPDATA%\XenoNet\Cache
///   macOS:   ~/Library/Application Support/XenoNet   ~/Library/Caches/XenoNet
///   Linux:   ~/.local/state/xenonet           ~/.cache/xenonet
/// ```
///
/// `xdg` is XDG_STATE_HOME for state and XDG_CACHE_HOME for the cache. A
/// copy running from source passes name "XenoNet Dev" for folders of its
/// own.
pub fn app_dir(kind: &str, xdg: Option<&str>, local_app_data: Option<&str>, platform: &str, home: &str, name: &str) -> String {
    let slug = slug_of(name);
    let state = kind == "state";
    if let Some(x) = xdg.filter(|x| !x.is_empty()) {
        return join_all(&[x, &slug]);
    }
    match platform {
        "win32" => {
            let local = local_app_data.map_or_else(|| join_all(&[home, "AppData", "Local"]), str::to_string);
            if state {
                join_all(&[&local, name])
            } else {
                join_all(&[&local, name, "Cache"])
            }
        }
        "darwin" => join_all(&[home, "Library", if state { "Application Support" } else { "Caches" }, name]),
        _ => join_all(&[home, if state { ".local/state" } else { ".cache" }, &slug]),
    }
}

// The name lowercased, each run of white space a dash.
fn slug_of(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut out = String::new();
    let mut space = false;
    for c in lower.chars() {
        if crate::text::is_js_space(c) {
            if !space {
                out.push('-');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out
}

/// A folder that couldn't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unreadable;

/// How a caller looks at disk: whether a path is there, a folder's names,
/// and a file's mtime.
pub trait Look {
    fn exists(&mut self, path: &str) -> bool;
    fn readdir(&mut self, path: &str) -> Result<Vec<String>, Unreadable>;
    fn mtime(&mut self, path: &str) -> f64;
}

/// The environment the desktop apps' folders depend on.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub claude_desktop_dir: Option<String>,
    pub local_app_data: Option<String>,
    pub app_data: Option<String>,
    pub xdg_config_home: Option<String>,
}

/// Claude desktop's data folder, or None.
///
/// ```text
///   Windows: the MSIX package redirects %APPDATA%\Claude into
///            %LOCALAPPDATA%\Packages\Claude_<publisher>\LocalCache\Roaming\Claude.
///   macOS:   ~/Library/Application Support/Claude
///   Linux:   $XDG_CONFIG_HOME/Claude, else ~/.config/Claude
/// ```
pub fn find_claude_desktop_dir(env: &Env, platform: &str, home: &str, look: &mut dyn Look) -> Option<String> {
    if let Some(d) = env.claude_desktop_dir.as_ref().filter(|d| !d.is_empty()) {
        return Some(d.clone());
    }
    let mut candidates = Vec::new();
    match platform {
        "win32" => {
            let local = env.local_app_data.clone().unwrap_or_else(|| join_all(&[home, "AppData", "Local"]));
            let packages = join_all(&[&local, "Packages"]);
            if let Ok(names) = look.readdir(&packages) {
                for n in names.iter().filter(|n| n.starts_with("Claude_")) {
                    candidates.push(join_all(&[&packages, n, "LocalCache", "Roaming", "Claude"]));
                }
            }
            let roaming = env.app_data.clone().unwrap_or_else(|| join_all(&[home, "AppData", "Roaming"]));
            candidates.push(join_all(&[&roaming, "Claude"]));
        }
        "darwin" => candidates.push(join_all(&[home, "Library", "Application Support", "Claude"])),
        _ => {
            let config = env.xdg_config_home.clone().unwrap_or_else(|| join_all(&[home, ".config"]));
            candidates.push(join_all(&[&config, "Claude"]));
        }
    }
    candidates.into_iter().find(|d| look.exists(d))
}

/// The codex binaries that ship inside the ChatGPT desktop app, newest
/// first, so the protocol matches whatever is writing ~/.codex.
///
/// ```text
///   Windows: %LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe
///   macOS:   ChatGPT.app/Contents/Resources/codex (or Codex.app's), in
///            /Applications or ~/Applications
/// ```
pub fn bundled_codex_binaries(env: &Env, platform: &str, home: &str, look: &mut dyn Look) -> Vec<String> {
    match platform {
        "win32" => {
            let local = env.local_app_data.clone().unwrap_or_else(|| join_all(&[home, "AppData", "Local"]));
            let bin = join_all(&[&local, "OpenAI", "Codex", "bin"]);
            let Ok(names) = look.readdir(&bin) else { return Vec::new() };
            let mut found: Vec<(String, f64)> = Vec::new();
            for f in names.iter().map(|d| join_all(&[&bin, d, "codex.exe"])) {
                if look.exists(&f) {
                    let m = look.mtime(&f);
                    found.push((f, m));
                }
            }
            found.sort_by(|a, b| b.1.total_cmp(&a.1));
            found.into_iter().map(|(f, _)| f).collect()
        }
        "darwin" => {
            let mine = join_all(&[home, "Applications"]);
            let mut out = Vec::new();
            for apps in ["/Applications", mine.as_str()] {
                for bundle in ["ChatGPT.app", "Codex.app"] {
                    out.push(join_all(&[apps, bundle, "Contents", "Resources", "codex"]));
                }
            }
            out.into_iter().filter(|f| look.exists(f)).collect()
        }
        _ => Vec::new(),
    }
}

/// Two version folders' names in order: part by part between the dots,
/// numbers as numbers, anything else as text, as the JS compares them.
fn version_order(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (x, y): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
    for i in 0..x.len().max(y.len()) {
        let (Some(p), Some(q)) = (x.get(i), y.get(i)) else { return if i >= x.len() { Ordering::Less } else { Ordering::Greater } };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        let o = if digits(p) && digits(q) { p.parse::<f64>().unwrap_or(0.0).total_cmp(&q.parse::<f64>().unwrap_or(0.0)) } else { p.cmp(q) };
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// The Claude Code that Claude desktop keeps a copy of, given Claude
/// desktop's folder (`find_claude_desktop_dir`): the newest version first,
/// and within one, the newest build. Only a build Claude desktop marked
/// `.verified` counts, so a download still under way, or one that failed
/// its check, is never run.
///
/// ```text
///   macOS:   <Claude>/claude-code/<version>/<build>/claude.app/Contents/MacOS/claude
///   Windows: <Claude>\claude-code\<version>\<build>\claude.exe, or in a
///            folder inside the build
/// ```
pub fn bundled_claude_code(desktop: &str, platform: &str, look: &mut dyn Look) -> Vec<String> {
    let root = join_all(&[desktop, "claude-code"]);
    let Ok(mut versions) = look.readdir(&root) else { return Vec::new() };
    versions.sort_by(|a, b| version_order(b, a));
    let mut out = Vec::new();
    for v in versions {
        let dir = join_all(&[&root, &v]);
        let Ok(builds) = look.readdir(&dir) else { continue };
        let mut found: Vec<(String, f64)> = Vec::new();
        for b in builds {
            let build = join_all(&[&dir, &b]);
            if !look.exists(&join_all(&[&build, ".verified"])) {
                continue;
            }
            let candidates = match platform {
                "darwin" => vec![join_all(&[&build, "claude.app", "Contents", "MacOS", "claude"])],
                "win32" => {
                    let inside = look.readdir(&build).unwrap_or_default();
                    std::iter::once(join_all(&[&build, "claude.exe"])).chain(inside.iter().map(|n| join_all(&[&build, n, "claude.exe"]))).collect()
                }
                _ => Vec::new(),
            };
            for f in candidates {
                if look.exists(&f) {
                    let m = look.mtime(&f);
                    found.push((f, m));
                }
            }
        }
        found.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.extend(found.into_iter().map(|(f, _)| f));
    }
    out
}

/// Where Claude Code's own installer puts it: ~/.local/bin/claude, or
/// claude.exe there on Windows. On Windows the installer leaves that
/// folder off PATH, and elsewhere a service started before the install
/// keeps the PATH it had.
pub fn installed_claude_code(platform: &str, home: &str) -> String {
    join_all(&[home, ".local", "bin", if platform == "win32" { "claude.exe" } else { "claude" }])
}

/// What the environment says of the CLIs XenoNet runs itself.
#[derive(Clone, Debug, Default)]
pub struct CliEnv {
    /// PATH, and PATHEXT, which Windows tries after each name.
    pub path: String,
    pub pathext: Option<String>,
    /// XENONET_CLAUDE_BIN and XENONET_CODEX_BIN.
    pub claude_bin: Option<String>,
    pub codex_bin: Option<String>,
    /// Where the desktop apps keep their folders.
    pub dirs: Env,
}

/// The program to run for an app's CLI, or None when it isn't here.
///
/// ```text
///   claude: on PATH (with PATHEXT on Windows, so an npm install's
///           claude.cmd), then where its installer puts it
///           (`installed_claude_code`), then the Claude Code that Claude
///           desktop keeps (`bundled_claude_code`), then XENONET_CLAUDE_BIN
///   codex:  XENONET_CODEX_BIN, then the one the ChatGPT desktop app ships
///           (`bundled_codex_binaries`), then on PATH
/// ```
///
/// `can_run` says whether a file on PATH may be run.
pub fn find_cli(app: &str, env: &CliEnv, platform: &str, home: &str, look: &mut dyn Look, can_run: &mut dyn FnMut(&str) -> bool) -> Option<String> {
    let windows = platform == "win32";
    let set = |v: &Option<String>| v.clone().filter(|v| !v.is_empty());
    match app {
        "claude" => crate::paths::on_path("claude", &env.path, env.pathext.as_deref(), windows, can_run)
            .or_else(|| Some(installed_claude_code(platform, home)).filter(|f| look.exists(f)))
            .or_else(|| find_claude_desktop_dir(&env.dirs, platform, home, look).and_then(|d| bundled_claude_code(&d, platform, look).into_iter().next()))
            .or_else(|| set(&env.claude_bin)),
        "codex" => set(&env.codex_bin)
            .or_else(|| bundled_codex_binaries(&env.dirs, platform, home, look).into_iter().next())
            .or_else(|| crate::paths::on_path("codex", &env.path, env.pathext.as_deref(), windows, can_run)),
        _ => None,
    }
}

// Spelled as on a unix host, where Node's path module is posix; on Windows
// the JS tests run against the Rust (test:rust) check the same. (Two
// attributes: clippy takes only a plain cfg(test) for test code.)
#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    #[test]
    fn app_dirs() {
        assert_eq!(app_dir("state", None, None, "darwin", "/Users/me", "XenoNet"), "/Users/me/Library/Application Support/XenoNet");
        assert_eq!(app_dir("cache", None, None, "linux", "/home/me", "XenoNet Dev"), "/home/me/.cache/xenonet-dev");
        assert_eq!(app_dir("state", Some("/x"), None, "darwin", "/Users/me", "XenoNet"), "/x/xenonet");
        assert_eq!(app_dir("cache", None, Some("L"), "win32", "/h", "XenoNet"), "L/XenoNet/Cache");
    }

    /// Made-up folders: the files there, with their mtimes.
    struct Disk(Vec<(&'static str, f64)>);

    impl Look for Disk {
        fn exists(&mut self, path: &str) -> bool {
            self.0.iter().any(|(f, _)| *f == path || f.starts_with(&format!("{path}/")))
        }
        fn readdir(&mut self, path: &str) -> Result<Vec<String>, Unreadable> {
            let mut names: Vec<String> = Vec::new();
            for (f, _) in &self.0 {
                if let Some(rest) = f.strip_prefix(&format!("{path}/")) {
                    let n = rest.split('/').next().unwrap_or("").to_string();
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
            }
            if names.is_empty() {
                Err(Unreadable)
            } else {
                Ok(names)
            }
        }
        fn mtime(&mut self, path: &str) -> f64 {
            self.0.iter().find(|(f, _)| *f == path).map_or(f64::NAN, |(_, m)| *m)
        }
    }

    #[test]
    fn claude_code_as_claude_desktop_keeps_it_newest_version_first() {
        let mac = "/Users/me/Library/Application Support/Claude/claude-code";
        let mut disk = Disk(vec![
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.99/aa/claude.app/Contents/MacOS/claude", 3.0),
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.99/aa/.verified", 3.0),
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.308/bb/claude.app/Contents/MacOS/claude", 1.0),
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.308/bb/.verified", 1.0),
            // Newer, but not verified: a download under way.
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.308/cc/claude.app/Contents/MacOS/claude", 2.0),
        ]);
        assert_eq!(
            bundled_claude_code("/Users/me/Library/Application Support/Claude", "darwin", &mut disk),
            [format!("{mac}/2.1.308/bb/claude.app/Contents/MacOS/claude"), format!("{mac}/2.1.99/aa/claude.app/Contents/MacOS/claude")]
        );
        // With none of the newest version's builds verified, the version
        // before it.
        let mut fresh = Disk(vec![
            ("/C/claude-code/2.0.0/new/claude.app/Contents/MacOS/claude", 2.0),
            ("/C/claude-code/1.9.0/old/claude.app/Contents/MacOS/claude", 1.0),
            ("/C/claude-code/1.9.0/old/.verified", 1.0),
        ]);
        assert_eq!(bundled_claude_code("/C", "darwin", &mut fresh), ["/C/claude-code/1.9.0/old/claude.app/Contents/MacOS/claude"]);
        let mut win = Disk(vec![
            ("/L/Claude/claude-code/1.0.0/x/claude.exe", 1.0),
            ("/L/Claude/claude-code/1.0.0/x/.verified", 1.0),
            ("/L/Claude/claude-code/1.0.0/y/bin/claude.exe", 2.0),
            ("/L/Claude/claude-code/1.0.0/y/.verified", 2.0),
            ("/L/Claude/claude-code/1.0.0/w/claude.exe", 5.0),
            ("/L/Claude/claude-code/0.9/z/claude.exe", 9.0),
            ("/L/Claude/claude-code/0.9/z/.verified", 9.0),
        ]);
        assert_eq!(
            bundled_claude_code("/L/Claude", "win32", &mut win),
            ["/L/Claude/claude-code/1.0.0/y/bin/claude.exe", "/L/Claude/claude-code/1.0.0/x/claude.exe", "/L/Claude/claude-code/0.9/z/claude.exe"]
        );
        assert_eq!(bundled_claude_code("/L/Claude", "linux", &mut win), Vec::<String>::new());
        assert_eq!(bundled_claude_code("/nowhere", "darwin", &mut win), Vec::<String>::new());
    }

    #[test]
    fn a_cli_is_found_on_path_then_where_the_desktop_apps_keep_it_then_by_its_variable() {
        let home = "/Users/me";
        let bundled = "/Users/me/Library/Application Support/Claude/claude-code/2.1.308/bb/claude.app/Contents/MacOS/claude";
        let mut disk = Disk(vec![
            (bundled, 1.0),
            ("/Users/me/Library/Application Support/Claude/claude-code/2.1.308/bb/.verified", 1.0),
            ("/Applications/ChatGPT.app/Contents/Resources/codex", 1.0),
        ]);
        let env = |path: &str, claude: Option<&str>, codex: Option<&str>| CliEnv {
            path: path.into(),
            pathext: None,
            claude_bin: claude.map(str::to_string),
            codex_bin: codex.map(str::to_string),
            dirs: Env::default(),
        };
        let on = |files: &'static [&'static str]| move |f: &str| files.contains(&f);
        let e = env("/usr/local/bin:/opt/bin", Some("/x/claude"), None);
        assert_eq!(find_cli("claude", &e, "darwin", home, &mut disk, &mut on(&["/opt/bin/claude"])).as_deref(), Some("/opt/bin/claude"), "PATH first");
        let mut installed =
            Disk(vec![("/Users/me/.local/bin/claude", 1.0), (bundled, 1.0), ("/Users/me/Library/Application Support/Claude/claude-code/2.1.308/bb/.verified", 1.0)]);
        assert_eq!(find_cli("claude", &e, "darwin", home, &mut installed, &mut on(&[])).as_deref(), Some("/Users/me/.local/bin/claude"), "then where its installer puts it");
        assert_eq!(find_cli("claude", &e, "darwin", home, &mut disk, &mut on(&[])).as_deref(), Some(bundled), "then Claude desktop's");
        let mut empty = Disk(vec![]);
        assert_eq!(find_cli("claude", &e, "darwin", home, &mut empty, &mut on(&[])).as_deref(), Some("/x/claude"), "then XENONET_CLAUDE_BIN");
        assert_eq!(find_cli("claude", &env("/usr/bin", Some(""), None), "darwin", home, &mut empty, &mut on(&[])), None);
        // Windows tries PATHEXT, so an npm install's claude.cmd is found.
        let w = CliEnv { pathext: Some(".EXE;.CMD".into()), ..env("/n", None, None) };
        assert_eq!(find_cli("claude", &w, "win32", home, &mut empty, &mut on(&["/n/claude.CMD"])).as_deref(), Some("/n/claude.CMD"));
        // Its installer's claude.exe, which it leaves off PATH.
        let mut native = Disk(vec![("/Users/me/.local/bin/claude.exe", 1.0)]);
        assert_eq!(find_cli("claude", &w, "win32", home, &mut native, &mut on(&[])).as_deref(), Some("/Users/me/.local/bin/claude.exe"));
        // Codex: its variable, then the ChatGPT app's, then PATH.
        assert_eq!(find_cli("codex", &env("/usr/bin", None, Some("/c/codex")), "darwin", home, &mut disk, &mut on(&["/usr/bin/codex"])).as_deref(), Some("/c/codex"));
        assert_eq!(
            find_cli("codex", &env("/usr/bin", None, None), "darwin", home, &mut disk, &mut on(&["/usr/bin/codex"])).as_deref(),
            Some("/Applications/ChatGPT.app/Contents/Resources/codex")
        );
        assert_eq!(find_cli("codex", &env("/usr/bin", None, None), "linux", home, &mut disk, &mut on(&["/usr/bin/codex"])).as_deref(), Some("/usr/bin/codex"));
        assert_eq!(find_cli("opencode", &env("/usr/bin", None, None), "linux", home, &mut disk, &mut on(&["/usr/bin/opencode"])), None);
    }
}
