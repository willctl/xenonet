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
}
