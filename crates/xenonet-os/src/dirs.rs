//! Where XenoNet keeps its own files (core/dirs.js's appDir). No side
//! effects: whoever asks says the platform, the home folder and the
//! environment. Where the apps XenoNet reads keep theirs is the adapters'
//! (`dirs` in xenonet-adapter).

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
        if xenonet_text::text::is_space(c) {
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
