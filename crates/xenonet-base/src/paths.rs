//! Paths as XenoNet compares and shows them (`core/paths.js`): keys that
//! match however an app wrote a folder, home written as `~`, the throwaway
//! folders grouped into zones, and the walks that list files.

use std::fs;
use std::path::{Path, PathBuf};

use crate::nodepath::Style;

/// A path's key: compared case-insensitively with forward slashes, because
/// the apps report the same Windows folder as `C:\x`, `c:/x` or `\\?\C:\x`.
/// On macOS, /tmp, /var and /etc are links into /private, and tools report
/// either form. None for an empty path.
pub fn path_key(p: &str) -> Option<String> {
    if p.is_empty() {
        return None;
    }
    let s = p.strip_prefix(r"\\?\").unwrap_or(p).replace('\\', "/");
    let s = match s.strip_prefix("/private/") {
        Some(rest) if ["var", "tmp", "etc"].iter().any(|d| rest.strip_prefix(d).is_some_and(|t| t.is_empty() || t.starts_with('/'))) => format!("/{rest}"),
        _ => s,
    };
    Some(s.trim_end_matches('/').to_lowercase())
}

/// Whether a key is the other or under it.
pub fn is_under(child: &str, parent: &str) -> bool {
    if child.is_empty() || parent.is_empty() {
        return false;
    }
    child == parent || (child.starts_with(parent) && child.as_bytes().get(parent.len()) == Some(&b'/'))
}

// JS's \w without the u flag, and the two others tilde's lookahead names.
fn word_like(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-'
}

// As a JS regex with the i flag compares two characters.
fn same_char(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

// Every match of `form` in `text`, compared without case and not followed
// by a word character, a dot or a dash, replaced with "~": as
// text.replace(/form(?![\w.-])/gi, "~").
fn replace_form(text: &str, form: &[char]) -> String {
    replace_unless(text, form, word_like)
}

/// Every match of `form` in `text`, compared without case and not followed
/// by a character `stops` says, replaced with "~": as
/// text.replace(/form(?!stops)/gi, "~").
pub fn replace_unless(text: &str, form: &[char], stops: fn(char) -> bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i <= chars.len() {
        let end = i + form.len();
        let here = end <= chars.len() && chars[i..end].iter().zip(form).all(|(a, b)| same_char(*a, *b)) && chars.get(end).is_none_or(|c| !stops(*c));
        if here {
            out.push('~');
            if form.is_empty() {
                // An empty match moves on by one character, as JS's does.
                if let Some(c) = chars.get(i) {
                    out.push(*c);
                }
                i += 1;
            } else {
                i = end;
            }
        } else {
            if let Some(c) = chars.get(i) {
                out.push(*c);
            }
            i += 1;
        }
    }
    out
}

/// Writes paths under the home folder as "~", so activity details,
/// summaries and screenshots don't carry the user name.
pub fn tilde(text: &str, home: &str) -> String {
    if text.is_empty() || home.is_empty() {
        return text.to_string();
    }
    let h = home.trim_end_matches(['\\', '/']);
    let mut forms: Vec<String> = Vec::new();
    for f in [h.to_string(), h.replace('\\', "/"), h.replace('/', "\\")] {
        if !forms.contains(&f) {
            forms.push(f);
        }
    }
    forms.iter().fold(text.to_string(), |out, f| replace_form(&out, &f.chars().collect::<Vec<_>>()))
}

/// A path's last part, with either slash; the path itself when it has none.
pub fn base_name(p: &str) -> String {
    let s = p.replace('\\', "/");
    let s = s.trim_end_matches('/');
    match s.rsplit('/').next() {
        Some(last) if !last.is_empty() => last.to_string(),
        _ => p.to_string(),
    }
}

/// A zone that groups the throwaway folders an app makes per session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub id: &'static str,
    pub name: &'static str,
    pub app: Option<&'static str>,
}

pub const SCRATCH_ZONES: [Zone; 3] = [
    Zone { id: "zone:claude-scratch", name: "Claude scratch", app: Some("claude") },
    Zone { id: "zone:codex-projectless", name: "Codex without a project", app: Some("codex") },
    Zone { id: "zone:temp", name: "Temp", app: None },
];

// /documents/codex/\d{4}-\d{2}-\d{2}(\/|$)
fn codex_day_folder(k: &str) -> bool {
    const AT: &str = "/documents/codex/";
    k.match_indices(AT).any(|(i, _)| {
        let d = &k.as_bytes()[i + AT.len()..];
        let digits = |r: std::ops::Range<usize>| d.len() >= r.end && d[r].iter().all(u8::is_ascii_digit);
        digits(0..4) && d.get(4) == Some(&b'-') && digits(5..7) && d.get(7) == Some(&b'-') && digits(8..10) && matches!(d.get(10), None | Some(b'/'))
    })
}

// ^/var/folders/[^/]+/[^/]+/t(/|$): macOS's $TMPDIR.
fn mac_tmpdir(k: &str) -> bool {
    let Some(rest) = k.strip_prefix("/var/folders/") else { return false };
    let mut parts = rest.splitn(4, '/');
    let (a, b, t) = (parts.next(), parts.next(), parts.next());
    matches!((a, b, t), (Some(a), Some(b), Some("t")) if !a.is_empty() && !b.is_empty())
}

/// Whether a key falls in zone `i` of SCRATCH_ZONES, whatever the zones
/// before it say.
pub fn in_scratch_zone(i: usize, key: &str) -> bool {
    match i {
        0 => key.contains("/claude/scratch-workspaces/"),
        1 => codex_day_folder(key),
        // %TEMP% on Windows, $TMPDIR (/var/folders/xx/…/T) on macOS, /tmp anywhere.
        2 => {
            let slashed = format!("{key}/");
            slashed.contains("/appdata/local/temp/") || slashed.starts_with("/tmp/") || mac_tmpdir(key)
        }
        _ => false,
    }
}

/// The zone a folder's key belongs to, for both desktop apps' throwaway
/// folders and the temp folders, or None.
pub fn scratch_zone(key: &str) -> Option<&'static Zone> {
    if key.is_empty() {
        return None;
    }
    (0..SCRATCH_ZONES.len()).find(|&i| in_scratch_zone(i, key)).map(|i| &SCRATCH_ZONES[i])
}

/// Which syntax a path keeps. Paths read from another machine keep that
/// machine's, and inferring it from the path also keeps fixture data
/// independent of the test machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStyle {
    Windows,
    Posix,
    /// This machine's own: a relative path.
    Native,
}

pub fn path_style(p: &str) -> PathStyle {
    let b = p.as_bytes();
    let drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    if drive || p.starts_with(r"\\") || p.contains('\\') {
        PathStyle::Windows
    } else if p.starts_with('/') {
        PathStyle::Posix
    } else {
        PathStyle::Native
    }
}

/// Joins any number of parts and normalizes, as Node's path.join does for
/// this platform: empty parts are left out, and nothing at all is ".".
pub fn join_all(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    if cfg!(windows) {
        return Style::Win32.join(&parts);
    }
    normalize_posix(&parts.join("/"))
}

/// Joins and normalizes as Node's path.join does for this platform.
pub fn join(dir: &str, name: &str) -> String {
    if cfg!(windows) {
        return Style::Win32.join(&[dir, name]);
    }
    normalize_posix(&format!("{dir}/{name}"))
}

/// path.resolve on this machine: absolute, `.` and `..` worked out, and no
/// slash at the end.
pub fn resolve(p: &str) -> String {
    #[cfg(windows)]
    {
        let full = std::path::absolute(if p.is_empty() { "." } else { p }).map_or_else(|_| p.to_string(), |a| a.to_string_lossy().into_owned());
        if full.len() > 3 {
            full.trim_end_matches(['\\', '/']).to_string()
        } else {
            full
        }
    }
    #[cfg(not(windows))]
    {
        let full = if p.starts_with('/') { p.to_string() } else { format!("{}/{p}", std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()) };
        let n = normalize_posix(&full);
        if n.len() > 1 {
            n.trim_end_matches('/').to_string()
        } else {
            n
        }
    }
}

/// path.posix.normalize: `.` and `..` worked out, slashes collapsed, a
/// trailing slash kept.
pub fn normalize_posix(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let absolute = p.starts_with('/');
    let trailing = p.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    let mut out = parts.join("/");
    if out.is_empty() && !absolute {
        out = ".".into();
    }
    if trailing && !out.is_empty() && out != "." {
        out.push('/');
    }
    if absolute {
        format!("/{out}")
    } else {
        out
    }
}

/// The first `cmd` on a PATH that `can_run` accepts, or None. `pathext` is
/// Windows' PATHEXT (only used when `windows`).
pub fn on_path(cmd: &str, path_var: &str, pathext: Option<&str>, windows: bool, can_run: &mut dyn FnMut(&str) -> bool) -> Option<String> {
    let exts: Vec<&str> = if windows { pathext.unwrap_or(".EXE;.CMD;.BAT").split(';').collect() } else { vec![""] };
    let delimiter = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(delimiter).filter(|d| !d.is_empty()) {
        for ext in &exts {
            let file = join(dir, &format!("{cmd}{ext}"));
            if can_run(&file) {
                return Some(file);
            }
        }
    }
    None
}

/// Whether this process may run a file, as fs.accessSync(f, X_OK) says.
pub fn can_run(file: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Ok(c) = std::ffi::CString::new(Path::new(file).as_os_str().as_bytes()) else { return false };
        may_run(&c)
    }
    #[cfg(not(unix))]
    {
        Path::new(file).exists()
    }
}

#[cfg(unix)]
#[expect(unsafe_code, reason = "access(2), which std doesn't wrap")]
fn may_run(path: &std::ffi::CStr) -> bool {
    // SAFETY: `path` is a NUL-terminated string that outlives the call, and
    // access only reads it.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

/// A folder's entries as Node's readdir lists them: by name, byte for byte,
/// on unix (libuv sorts what scandir finds), in the filesystem's order
/// elsewhere. None when the folder can't be read.
pub fn read_dir_sorted(dir: &Path) -> std::io::Result<Vec<fs::DirEntry>> {
    let mut ents: Vec<fs::DirEntry> = fs::read_dir(dir)?.filter_map(Result::ok).collect();
    if cfg!(unix) {
        ents.sort_by(|a, b| {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                a.file_name().as_bytes().cmp(b.file_name().as_bytes())
            }
            #[cfg(not(unix))]
            {
                a.file_name().cmp(&b.file_name())
            }
        });
    }
    Ok(ents)
}

/// The files under `dir` whose names `pick` picks, in folders down to
/// `depth` levels below it. A folder that can't be read is passed over.
pub fn list_files(dir: &str, pick: &mut dyn FnMut(&str) -> bool, depth: u32) -> Vec<String> {
    fn walk(d: &str, level: u32, depth: u32, pick: &mut dyn FnMut(&str) -> bool, out: &mut Vec<String>) {
        let Ok(ents) = read_dir_sorted(Path::new(d)) else { return };
        for e in ents {
            let name = e.file_name().to_string_lossy().into_owned();
            let full = join(d, &name);
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if level < depth {
                    walk(&full, level + 1, depth, pick, out);
                }
            } else if t.is_file() && pick(&name) {
                out.push(full);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, depth, pick, &mut out);
    out
}

/// fs.realpathSync.native: `p` with its links followed and, on Windows, its
/// short names spelled out, without the `\\?\` prefix canonicalize puts on
/// it there, which Node never shows and no other spelling of the path
/// matches. `p` as it is when it can't be resolved.
pub fn real_path(p: &Path) -> PathBuf {
    crate::fs::realpath_native(&p.to_string_lossy()).map_or_else(|_| p.to_path_buf(), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(path_key(r"\\?\C:\Users\Me\X\").as_deref(), Some("c:/users/me/x"));
        assert_eq!(path_key("/private/tmp/scratch/").as_deref(), Some("/tmp/scratch"));
        assert_eq!(path_key("/private/tmpfiles").as_deref(), Some("/private/tmpfiles"));
        assert_eq!(path_key(""), None);
        assert!(is_under("/a/b", "/a") && is_under("/a", "/a") && !is_under("/ab", "/a"));
    }

    #[test]
    fn tilde_writes_home_as_a_tilde() {
        assert_eq!(tilde("/Users/me/x and /users/ME/y, /Users/me2", "/Users/me/"), "~/x and ~/y, /Users/me2");
        assert_eq!(tilde(r"C:\Users\me\x C:/Users/me/y", r"C:\Users\me"), r"~\x ~/y");
        assert_eq!((tilde("ab", "/"), tilde("a.b c", "/")), ("ab~".into(), "a.b~ c~".into()), "as JS does with an empty home");
    }

    #[test]
    fn zones_and_names() {
        assert_eq!(scratch_zone("/private/var/folders/ab/xyz/t/x").map(|z| z.id), None, "keys come through path_key first");
        assert_eq!(scratch_zone("/var/folders/ab/xyz/t/claude-run/x").map(|z| z.id), Some("zone:temp"));
        assert_eq!(scratch_zone("/users/me/documents/codex/2026-10-02/regex").map(|z| z.id), Some("zone:codex-projectless"));
        assert_eq!(base_name("/a/b/"), "b");
        assert_eq!(base_name("/"), "/");
        assert_eq!(normalize_posix("/a/../b//c/"), "/b/c/");
    }
}
