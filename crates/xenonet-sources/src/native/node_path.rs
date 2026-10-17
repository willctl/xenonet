//! Node's path.win32 and path.posix (Node 22's), for what core/native's JS
//! called through pathStyle (core/paths.js): isAbsolute, resolve, join and
//! basename, so a path keeps the syntax of the machine that wrote it.
//!
//! Positions are bytes where JS counts UTF-16 units: every place the rules
//! look at is a separator, a dot or a colon, all ASCII, so they agree. The
//! one difference: a drive-relative Windows path (C:foo) resolves against
//! the process's own folder, where Node asks for the drive's (=C:).

use xenonet_base::paths::PathStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    Win32,
    Posix,
}

fn flavor(style: PathStyle) -> Flavor {
    match style {
        PathStyle::Windows => Flavor::Win32,
        PathStyle::Posix => Flavor::Posix,
        PathStyle::Native if cfg!(windows) => Flavor::Win32,
        PathStyle::Native => Flavor::Posix,
    }
}

fn is_win_sep(c: u8) -> bool {
    c == b'/' || c == b'\\'
}

fn is_posix_sep(c: u8) -> bool {
    c == b'/'
}

fn is_device_root(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

fn cwd() -> String {
    std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()
}

// Byte slice, empty when the end is before the start, as String.slice is.
fn cut(s: &str, from: usize, to: usize) -> &str {
    let to = to.min(s.len());
    if from >= to {
        ""
    } else {
        &s[from..to]
    }
}

pub(crate) fn is_absolute(style: PathStyle, p: &str) -> bool {
    let b = p.as_bytes();
    match flavor(style) {
        Flavor::Posix => b.first() == Some(&b'/'),
        Flavor::Win32 => !b.is_empty() && (is_win_sep(b[0]) || (b.len() > 2 && is_device_root(b[0]) && b[1] == b':' && is_win_sep(b[2]))),
    }
}

pub(crate) fn resolve(style: PathStyle, parts: &[&str]) -> String {
    match flavor(style) {
        Flavor::Posix => posix_resolve(parts),
        Flavor::Win32 => win32_resolve(parts),
    }
}

pub(crate) fn join(style: PathStyle, parts: &[&str]) -> String {
    match flavor(style) {
        Flavor::Posix => posix_join(parts),
        Flavor::Win32 => win32_join(parts),
    }
}

pub(crate) fn basename(style: PathStyle, p: &str) -> String {
    let win = flavor(style) == Flavor::Win32;
    let b = p.as_bytes();
    let sep = |c: u8| if win { is_win_sep(c) } else { is_posix_sep(c) };
    let mut start = if win && b.len() >= 2 && is_device_root(b[0]) && b[1] == b':' { 2 } else { 0 };
    let mut end = None;
    for i in (start..b.len()).rev() {
        if sep(b[i]) {
            if end.is_some() {
                start = i + 1;
                break;
            }
        } else if end.is_none() {
            end = Some(i + 1);
        }
    }
    end.map_or_else(String::new, |e| p[start..e].to_string())
}

// Node's normalizeString: `.` and `..` worked out, separators collapsed,
// none at either end.
fn normalize_string(path: &str, allow_above_root: bool, sep: char, is_sep: fn(u8) -> bool) -> String {
    let b = path.as_bytes();
    let mut res = String::new();
    let mut last_segment_length: isize = 0;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code: u8 = 0;
    let mut i = 0usize;
    while i <= b.len() {
        if i < b.len() {
            code = b[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/';
        }
        if is_sep(code) {
            if last_slash == i as isize - 1 || dots == 1 {
                // Nothing: an empty segment, or ".".
            } else if dots == 2 {
                let rb = res.as_bytes();
                let dotted = rb.len() >= 2 && last_segment_length == 2 && rb[rb.len() - 1] == b'.' && rb[rb.len() - 2] == b'.';
                if !dotted {
                    if res.len() > 2 {
                        match res.rfind(sep) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(k) => {
                                res.truncate(k);
                                last_segment_length = res.len() as isize - 1 - res.rfind(sep).map_or(-1, |x| x as isize);
                            }
                        }
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    } else if !res.is_empty() {
                        res.clear();
                        last_segment_length = 0;
                        last_slash = i as isize;
                        dots = 0;
                        i += 1;
                        continue;
                    }
                }
                if allow_above_root {
                    if !res.is_empty() {
                        res.push(sep);
                    }
                    res.push_str("..");
                    last_segment_length = 2;
                }
            } else {
                let seg = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(sep);
                }
                res.push_str(seg);
                last_segment_length = i as isize - last_slash - 1;
            }
            last_slash = i as isize;
            dots = 0;
        } else if code == b'.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
        i += 1;
    }
    res
}

// ---------- posix ----------

fn posix_cwd() -> String {
    let c = cwd();
    if cfg!(windows) {
        let c = c.replace('\\', "/");
        let at = c.find('/').unwrap_or(c.len());
        c[at..].to_string()
    } else {
        c
    }
}

fn posix_resolve(parts: &[&str]) -> String {
    if parts.is_empty() || (parts.len() == 1 && (parts[0].is_empty() || parts[0] == ".")) {
        let c = posix_cwd();
        if c.starts_with('/') {
            return c;
        }
    }
    let mut resolved = String::new();
    let mut absolute = false;
    for p in parts.iter().rev() {
        if absolute {
            break;
        }
        if p.is_empty() {
            continue;
        }
        resolved = format!("{p}/{resolved}");
        absolute = p.starts_with('/');
    }
    if !absolute {
        let c = posix_cwd();
        resolved = format!("{c}/{resolved}");
        absolute = c.starts_with('/');
    }
    let resolved = normalize_string(&resolved, !absolute, '/', is_posix_sep);
    if absolute {
        format!("/{resolved}")
    } else if resolved.is_empty() {
        ".".into()
    } else {
        resolved
    }
}

fn posix_normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut p = normalize_string(path, !absolute, '/', is_posix_sep);
    if p.is_empty() {
        return if absolute {
            "/".into()
        } else if trailing {
            "./".into()
        } else {
            ".".into()
        };
    }
    if trailing {
        p.push('/');
    }
    if absolute {
        format!("/{p}")
    } else {
        p
    }
}

fn posix_join(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    posix_normalize(&parts.join("/"))
}

// ---------- win32 ----------

const RESERVED: [&str; 28] = [
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

fn reserved(name: &str) -> bool {
    let up = name.to_uppercase();
    RESERVED.contains(&up.as_str())
}

// isWindowsReservedName(path, colonIndex): the part before the colon, or,
// with none (-1), all but the last unit (which, cut from a pair, is no
// reserved name).
fn reserved_at(path: &str, colon: Option<usize>) -> bool {
    match colon {
        Some(k) => reserved(&path[..k]),
        None => match path.chars().last() {
            None => false,
            Some(c) if c.len_utf16() == 2 => false,
            Some(c) => reserved(&path[..path.len() - c.len_utf8()]),
        },
    }
}

// The root a Windows path begins with: (where it ends, its device, whether
// it's absolute), as resolve reads it.
fn win32_root(path: &str) -> (usize, String, bool) {
    let b = path.as_bytes();
    let len = b.len();
    let mut root_end = 0;
    let mut device = String::new();
    let mut absolute = false;
    if len == 0 {
        return (0, device, false);
    }
    let code = b[0];
    if len == 1 {
        if is_win_sep(code) {
            root_end = 1;
            absolute = true;
        }
    } else if is_win_sep(code) {
        absolute = true;
        if is_win_sep(b[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_win_sep(b[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first = &path[last..j];
                last = j;
                while j < len && is_win_sep(b[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_win_sep(b[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if first != "." && first != "?" {
                            device = format!("\\\\{first}\\{}", &path[last..j]);
                            root_end = j;
                        } else {
                            device = format!("\\\\{first}");
                            root_end = 4;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else if is_device_root(code) && b[1] == b':' {
        device = path[..2].to_string();
        root_end = 2;
        if len > 2 && is_win_sep(b[2]) {
            absolute = true;
            root_end = 3;
        }
    }
    (root_end, device, absolute)
}

fn win32_resolve(parts: &[&str]) -> String {
    let mut resolved_device = String::new();
    let mut resolved_tail = String::new();
    let mut resolved_absolute = false;
    let mut i = parts.len() as isize - 1;
    while i >= -1 {
        let path = if i >= 0 {
            let p = parts[i as usize];
            if p.is_empty() {
                i -= 1;
                continue;
            }
            p.to_string()
        } else if resolved_device.is_empty() {
            let c = cwd();
            let fast = parts.is_empty() || (parts.len() == 1 && (parts[0].is_empty() || parts[0] == ".") && c.as_bytes().first().is_some_and(|x| is_win_sep(*x)));
            if fast {
                return if cfg!(windows) { c } else { c.replace('/', "\\") };
            }
            c
        } else {
            // The drive's own folder would be =C: in the environment; the
            // process's is used (see the module's note).
            let c = cwd();
            let units: Vec<u16> = c.encode_utf16().collect();
            let head = String::from_utf16_lossy(&units[..units.len().min(2)]);
            if head.to_lowercase() != resolved_device.to_lowercase() && units.get(2) == Some(&u16::from(b'\\')) {
                format!("{resolved_device}\\")
            } else {
                c
            }
        };
        let (root_end, device, absolute) = win32_root(&path);
        if !device.is_empty() {
            if !resolved_device.is_empty() {
                if device.to_lowercase() != resolved_device.to_lowercase() {
                    i -= 1;
                    continue;
                }
            } else {
                resolved_device = device;
            }
        }
        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            resolved_tail = format!("{}\\{resolved_tail}", cut(&path, root_end, path.len()));
            resolved_absolute = absolute;
            if absolute && !resolved_device.is_empty() {
                break;
            }
        }
        i -= 1;
    }
    let tail = normalize_string(&resolved_tail, !resolved_absolute, '\\', is_win_sep);
    if resolved_absolute {
        format!("{resolved_device}\\{tail}")
    } else {
        let s = format!("{resolved_device}{tail}");
        if s.is_empty() {
            ".".into()
        } else {
            s
        }
    }
}

fn win32_normalize(path: &str) -> String {
    let b = path.as_bytes();
    let len = b.len();
    if len == 0 {
        return ".".into();
    }
    let mut root_end = 0;
    let mut device: Option<String> = None;
    let mut absolute = false;
    let code = b[0];
    if len == 1 {
        return if is_posix_sep(code) { "\\".into() } else { path.to_string() };
    }
    if is_win_sep(code) {
        absolute = true;
        if is_win_sep(b[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_win_sep(b[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first = &path[last..j];
                last = j;
                while j < len && is_win_sep(b[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_win_sep(b[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if first == "." || first == "?" {
                            device = Some(format!("\\\\{first}"));
                            root_end = 4;
                            // \\?\COM1: and the like.
                            let possible = path.find(':').map_or("", |k| cut(path, 4, k + 1));
                            if !possible.is_empty() && reserved(&possible[..possible.len() - 1]) {
                                device = Some(format!("\\\\?\\{possible}"));
                                root_end = 4 + possible.len();
                            }
                        } else if j == len {
                            return format!("\\\\{first}\\{}\\", &path[last..]);
                        } else {
                            device = Some(format!("\\\\{first}\\{}", &path[last..j]));
                            root_end = j;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else if let Some(k) = path.find(':').filter(|k| *k > 0) {
        if is_device_root(code) && k == 1 {
            device = Some(path[..2].to_string());
            root_end = 2;
            if len > 2 && is_win_sep(b[2]) {
                absolute = true;
                root_end = 3;
            }
        } else if reserved(&path[..k]) {
            device = Some(path[..=k].to_string());
            root_end = k + 1;
        }
    }
    let mut tail = if root_end < len { normalize_string(&path[root_end..], !absolute, '\\', is_win_sep) } else { String::new() };
    if tail.is_empty() && !absolute {
        tail = ".".into();
    }
    if !tail.is_empty() && is_win_sep(b[len - 1]) {
        tail.push('\\');
    }
    if !absolute && device.is_none() && path.contains(':') {
        // Not something Windows would read as absolute (CVE-2024-36139).
        let tb = tail.as_bytes();
        if tb.len() >= 2 && is_device_root(tb[0]) && tb[1] == b':' {
            return format!(".\\{tail}");
        }
        for (k, _) in path.match_indices(':') {
            if k == len - 1 || is_win_sep(b[k + 1]) {
                return format!(".\\{tail}");
            }
        }
    }
    if reserved_at(path, path.find(':')) {
        return format!(".\\{}{tail}", device.unwrap_or_default());
    }
    match device {
        None if absolute => format!("\\{tail}"),
        None => tail,
        Some(d) if absolute => format!("{d}\\{tail}"),
        Some(d) => format!("{d}{tail}"),
    }
}

fn win32_join(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    let first = parts[0].as_bytes();
    let mut joined = parts.join("\\");
    // Not two separators at the start, which normalize would take for a
    // UNC root, unless the first part is one.
    let mut needs_replace = true;
    let mut slashes = 0;
    if is_win_sep(first[0]) {
        slashes += 1;
        if first.len() > 1 && is_win_sep(first[1]) {
            slashes += 1;
            if first.len() > 2 {
                if is_win_sep(first[2]) {
                    slashes += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }
    if needs_replace {
        let jb = joined.as_bytes();
        while slashes < jb.len() && is_win_sep(jb[slashes]) {
            slashes += 1;
        }
        if slashes >= 2 {
            joined = format!("\\{}", &joined[slashes..]);
        }
    }
    // A part with a reserved device name isn't normalized.
    if joined.split('\\').filter(|p| !p.is_empty()).any(|p| p.find(':').is_some_and(|k| reserved(&p[..k]))) {
        return joined.replace('/', "\\");
    }
    win32_normalize(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: PathStyle = PathStyle::Windows;
    const P: PathStyle = PathStyle::Posix;

    #[test]
    fn as_node_writes_them() {
        // What Node 22's path.win32 said of each.
        let cases: &[(&str, &str, &str)] = &[
            ("CONx", ".\\CONx", "CONx\\x"),
            ("CON:", ".\\CON:.", "CON:\\x"),
            ("con:foo", ".\\con:foo", "con:foo\\x"),
            ("a\\CON:x", "a\\CON:x", "a\\CON:x\\x"),
            ("C:\\a\\..\\b\\", "C:\\b\\", "C:\\b\\x"),
            ("C:", "C:.", "C:\\x"),
            ("\\\\?\\COM1:\\x", "\\\\?\\COM1:\\x", "\\\\?\\COM1:\\x\\x"),
            ("COM\u{b9}:", ".\\COM\u{b9}:.", "COM\u{b9}:\\x"),
            ("foo:bar", "foo:bar", "foo:bar\\x"),
            ("..\\..\\x", "..\\..\\x", "..\\..\\x\\x"),
            ("\\\\srv\\share", "\\\\srv\\share\\", "\\\\srv\\share\\x"),
            ("\\\\srv\\share\\a\\..\\..", "\\\\srv\\share\\", "\\\\srv\\share\\x"),
            ("//a", "\\a", "\\\\a\\x\\"),
            ("x/CON:", ".\\x\\CON:", ".\\x\\CON:\\x"),
            ("NUL", "NUL", "NUL\\x"),
            ("\\\\.\\COM1:", "\\\\?\\COM1:\\", "\\\\.\\COM1:\\x"),
            ("\\\\?\\c:\\x\\..\\y", "\\\\?\\c:\\y", "\\\\?\\c:\\y\\x"),
            ("C:..\\x", "C:..\\x", "C:..\\x\\x"),
            ("a:b", "a:b", "a:b\\x"),
        ];
        for (p, normalized, joined) in cases {
            assert_eq!(win32_normalize(p), *normalized, "normalize {p}");
            assert_eq!(join(W, &[p, "x"]), *joined, "join {p}");
        }
        let resolved: &[(&str, &str)] = &[
            ("C:\\a\\..\\b\\", "C:\\b"),
            ("\\\\srv\\share", "\\\\srv\\share\\"),
            ("\\\\?\\COM1:\\x", "\\\\?\\COM1:\\x"),
            ("//a", "C:\\a"),
            ("..\\..\\x", "C:\\x"),
            ("CON:", "C:\\base\\CON:"),
        ];
        for (p, r) in resolved {
            assert_eq!(resolve(W, &["C:\\base", p]), *r, "resolve {p}");
        }
        assert_eq!(resolve(W, &["C:\\w\\atlas", "src\\a.js"]), "C:\\w\\atlas\\src\\a.js");
        assert_eq!(resolve(W, &["C:/w/atlas/"]), "C:\\w\\atlas");
        assert_eq!(join(W, &["C:\\w\\atlas", "src\\a.js"]), "C:\\w\\atlas\\src\\a.js");
        assert_eq!(resolve(P, &["/w/atlas", "../x/./y/"]), "/w/x/y");
        assert_eq!(resolve(P, &["/w/atlas", "/elsewhere"]), "/elsewhere");
        assert_eq!(join(P, &["/w/atlas", "src/a.js"]), "/w/atlas/src/a.js");
        assert_eq!(join(P, &["", ""]), ".");
        assert_eq!(basename(P, "/w/atlas/"), "atlas");
        assert_eq!(basename(W, "C:\\w\\atlas"), "atlas");
        assert_eq!(basename(W, "C:\\"), "");
        assert_eq!(basename(P, "/"), "");
        assert!(is_absolute(W, "/w") && is_absolute(W, "C:\\") && !is_absolute(W, "C:") && is_absolute(P, "/") && !is_absolute(P, "C:\\"));
    }
}
