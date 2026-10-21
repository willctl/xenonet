//! Node's `path` module, both of its flavors: `path.posix` and
//! `path.win32`, worked out as Node 22's lib/path.js does, so a path
//! XenoNet builds or compares here comes out as the JS made it. The JS
//! often picks a flavor by the platform it's told about (`platform ===
//! "win32" ? path.win32 : path.posix`), or by the path itself
//! (`pathStyle(p)` in core/paths.js, `Style::from(path_style(p))` here),
//! whatever machine it runs on, so both are here on every machine, and
//! `NATIVE` is this machine's.
//!
//! Paths are worked on as bytes: every character the rules look at (the
//! separators, dots, a drive's colon) is ASCII, so a cut never falls inside
//! a character.

use crate::paths::PathStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Posix,
    Win32,
}

/// The flavor Node's own `path` is on this machine.
pub const NATIVE: Style = if cfg!(windows) { Style::Win32 } else { Style::Posix };

/// pathStyle's flavor: a Windows or a POSIX path keeps its own, and a
/// relative one takes this machine's.
impl From<PathStyle> for Style {
    fn from(style: PathStyle) -> Style {
        match style {
            PathStyle::Windows => Style::Win32,
            PathStyle::Posix => Style::Posix,
            PathStyle::Native => NATIVE,
        }
    }
}

impl Style {
    /// The flavor JS picks with `platform === "win32" ? path.win32 : path.posix`.
    pub fn of(platform: &str) -> Style {
        if platform == "win32" {
            Style::Win32
        } else {
            Style::Posix
        }
    }

    pub fn sep(self) -> char {
        match self {
            Style::Posix => '/',
            Style::Win32 => '\\',
        }
    }

    pub fn delimiter(self) -> char {
        match self {
            Style::Posix => ':',
            Style::Win32 => ';',
        }
    }

    fn is_sep(self, b: u8) -> bool {
        match self {
            Style::Posix => b == b'/',
            Style::Win32 => b == b'/' || b == b'\\',
        }
    }

    pub fn join(self, parts: &[&str]) -> String {
        match self {
            Style::Posix => posix_join(parts),
            Style::Win32 => win32_join(parts),
        }
    }

    pub fn resolve(self, parts: &[&str]) -> String {
        match self {
            Style::Posix => posix_resolve(parts, &posix_cwd()),
            Style::Win32 => win32_resolve(parts, &cwd()),
        }
    }

    /// resolve with the working folder given, for tests and callers that know it.
    pub fn resolve_from(self, parts: &[&str], cwd: &str) -> String {
        match self {
            Style::Posix => posix_resolve(parts, cwd),
            Style::Win32 => win32_resolve(parts, cwd),
        }
    }

    pub fn normalize(self, p: &str) -> String {
        match self {
            Style::Posix => posix_normalize(p),
            Style::Win32 => win32_normalize(p),
        }
    }

    pub fn dirname(self, p: &str) -> String {
        match self {
            Style::Posix => posix_dirname(p),
            Style::Win32 => win32_dirname(p),
        }
    }

    pub fn basename(self, p: &str, suffix: Option<&str>) -> String {
        let start = match self {
            Style::Win32 if device_root(p.as_bytes()) => 2,
            _ => 0,
        };
        basename_from(self, p, suffix, start)
    }

    pub fn extname(self, p: &str) -> String {
        let start = match self {
            Style::Win32 if device_root(p.as_bytes()) => 2,
            _ => 0,
        };
        extname_from(self, p, start)
    }

    pub fn is_absolute(self, p: &str) -> bool {
        let b = p.as_bytes();
        match self {
            Style::Posix => b.first() == Some(&b'/'),
            Style::Win32 => !b.is_empty() && (self.is_sep(b[0]) || (b.len() > 2 && device_root(b) && self.is_sep(b[2]))),
        }
    }

    pub fn relative(self, from: &str, to: &str) -> String {
        match self {
            Style::Posix => posix_relative(from, to, &posix_cwd()),
            Style::Win32 => win32_relative(from, to, &cwd()),
        }
    }
}

/// process.cwd(), as Node gives it.
pub fn cwd() -> String {
    std::env::current_dir().map_or_else(|_| "/".into(), |d| d.to_string_lossy().into_owned())
}

// The working folder path.posix resolves against: on Windows, with forward
// slashes and without its drive, as Node's posixCwd has it.
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

/// pathStyle(p) (core/paths.js): the flavor the path keeps, this machine's
/// for a relative one.
pub fn style_of(p: &str) -> Style {
    Style::from(crate::paths::path_style(p))
}

/// path.join for this machine.
pub fn join(parts: &[&str]) -> String {
    NATIVE.join(parts)
}

/// path.resolve for this machine.
pub fn resolve(parts: &[&str]) -> String {
    NATIVE.resolve(parts)
}

/// path.dirname for this machine.
pub fn dirname(p: &str) -> String {
    NATIVE.dirname(p)
}

/// path.basename for this machine.
pub fn basename(p: &str) -> String {
    NATIVE.basename(p, None)
}

fn device_root(b: &[u8]) -> bool {
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

// Node's normalizeString: `.` and `..` worked out, separators collapsed.
fn normalize_string(path: &[u8], allow_above_root: bool, sep: u8, is_sep: impl Fn(u8) -> bool) -> Vec<u8> {
    let mut res: Vec<u8> = Vec::new();
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots: i32 = 0;
    let mut code = 0u8;
    let mut i = 0usize;
    while i <= path.len() {
        if i < path.len() {
            code = path[i];
        } else if is_sep(code) {
            break;
        } else {
            code = b'/';
        }
        if is_sep(code) {
            if last_slash == i as isize - 1 || dots == 1 {
                // nothing
            } else if dots == 2 {
                if res.len() < 2 || last_segment_length != 2 || res[res.len() - 1] != b'.' || res[res.len() - 2] != b'.' {
                    if res.len() > 2 {
                        match res.iter().rposition(|&c| c == sep) {
                            None => {
                                res.clear();
                                last_segment_length = 0;
                            }
                            Some(at) => {
                                res.truncate(at);
                                last_segment_length = (res.len() as isize - 1 - res.iter().rposition(|&c| c == sep).map_or(-1, |p| p as isize)) as usize;
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
                    res.extend_from_slice(b"..");
                    last_segment_length = 2;
                }
            } else {
                let seg = &path[(last_slash + 1) as usize..i];
                if !res.is_empty() {
                    res.push(sep);
                }
                res.extend_from_slice(seg);
                last_segment_length = (i as isize - last_slash - 1) as usize;
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

fn text(b: Vec<u8>) -> String {
    String::from_utf8(b).unwrap_or_default()
}

fn posix_normalize_string(p: &str, allow_above_root: bool) -> String {
    text(normalize_string(p.as_bytes(), allow_above_root, b'/', |c| c == b'/'))
}

fn win32_normalize_string(p: &str, allow_above_root: bool) -> String {
    text(normalize_string(p.as_bytes(), allow_above_root, b'\\', |c| c == b'/' || c == b'\\'))
}

fn posix_resolve(parts: &[&str], cwd: &str) -> String {
    // The working folder itself, as it is.
    if (parts.is_empty() || (parts.len() == 1 && (parts[0].is_empty() || parts[0] == "."))) && cwd.starts_with('/') {
        return cwd.to_string();
    }
    let mut resolved = String::new();
    let mut absolute = false;
    for p in parts.iter().rev().copied().chain(std::iter::once(cwd)) {
        if absolute {
            break;
        }
        if p.is_empty() {
            continue;
        }
        resolved = format!("{p}/{resolved}");
        absolute = p.starts_with('/');
    }
    let n = posix_normalize_string(&resolved, !absolute);
    if absolute {
        format!("/{n}")
    } else if n.is_empty() {
        ".".into()
    } else {
        n
    }
}

fn posix_normalize(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let absolute = p.starts_with('/');
    let trailing = p.ends_with('/');
    let mut n = posix_normalize_string(p, !absolute);
    if n.is_empty() {
        if absolute {
            return "/".into();
        }
        return if trailing { "./".into() } else { ".".into() };
    }
    if trailing {
        n.push('/');
    }
    if absolute {
        format!("/{n}")
    } else {
        n
    }
}

fn posix_join(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    posix_normalize(&parts.join("/"))
}

fn posix_dirname(p: &str) -> String {
    let b = p.as_bytes();
    if b.is_empty() {
        return ".".into();
    }
    let has_root = b[0] == b'/';
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = b.len() as isize - 1;
    while i >= 1 {
        if b[i as usize] == b'/' {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }
    if end == -1 {
        return if has_root { "/".into() } else { ".".into() };
    }
    if has_root && end == 1 {
        return "//".into();
    }
    p[..end as usize].to_string()
}

fn basename_from(style: Style, p: &str, suffix: Option<&str>, from: usize) -> String {
    let b = p.as_bytes();
    let mut start = from;
    let mut end: isize = -1;
    let mut matched_slash = true;
    if let Some(suffix) = suffix.filter(|s| !s.is_empty() && s.len() <= p.len()) {
        if suffix == p {
            return String::new();
        }
        let sb = suffix.as_bytes();
        let mut ext_idx = sb.len() as isize - 1;
        let mut first_non_slash_end: isize = -1;
        let mut i = b.len() as isize - 1;
        while i >= from as isize {
            let code = b[i as usize];
            if style.is_sep(code) {
                if !matched_slash {
                    start = i as usize + 1;
                    break;
                }
            } else {
                if first_non_slash_end == -1 {
                    matched_slash = false;
                    first_non_slash_end = i + 1;
                }
                if ext_idx >= 0 {
                    if code == sb[ext_idx as usize] {
                        ext_idx -= 1;
                        if ext_idx == -1 {
                            end = i;
                        }
                    } else {
                        ext_idx = -1;
                        end = first_non_slash_end;
                    }
                }
            }
            i -= 1;
        }
        if start as isize == end {
            end = first_non_slash_end;
        } else if end == -1 {
            end = b.len() as isize;
        }
        return slice(p, start, end.max(start as isize) as usize);
    }
    let mut i = b.len() as isize - 1;
    while i >= from as isize {
        if style.is_sep(b[i as usize]) {
            if !matched_slash {
                start = i as usize + 1;
                break;
            }
        } else if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        i -= 1;
    }
    if end == -1 {
        return String::new();
    }
    slice(p, start, end as usize)
}

// A byte range of a path, which JS's slice would cut at the same places.
fn slice(p: &str, start: usize, end: usize) -> String {
    let end = end.min(p.len());
    String::from_utf8_lossy(&p.as_bytes()[start.min(end)..end]).into_owned()
}

fn extname_from(style: Style, p: &str, from: usize) -> String {
    let b = p.as_bytes();
    let mut start_dot: isize = -1;
    let mut start_part = from as isize;
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut pre_dot_state = 0;
    let mut i = b.len() as isize - 1;
    while i >= from as isize {
        let code = b[i as usize];
        if style.is_sep(code) {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            i -= 1;
            continue;
        }
        if end == -1 {
            matched_slash = false;
            end = i + 1;
        }
        if code == b'.' {
            if start_dot == -1 {
                start_dot = i;
            } else if pre_dot_state != 1 {
                pre_dot_state = 1;
            }
        } else if start_dot != -1 {
            pre_dot_state = -1;
        }
        i -= 1;
    }
    if start_dot == -1 || end == -1 || pre_dot_state == 0 || (pre_dot_state == 1 && start_dot == end - 1 && start_dot == start_part + 1) {
        return String::new();
    }
    slice(p, start_dot as usize, end as usize)
}

fn posix_relative(from: &str, to: &str, cwd: &str) -> String {
    if from == to {
        return String::new();
    }
    let from = posix_resolve(&[from], cwd);
    let to = posix_resolve(&[to], cwd);
    if from == to {
        return String::new();
    }
    let (f, t) = (from.as_bytes(), to.as_bytes());
    let from_start = 1;
    let from_end = f.len();
    let from_len = from_end - from_start;
    let to_start = 1;
    let to_len = t.len() - to_start;
    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0;
    while i < length {
        let c = f[from_start + i];
        if c != t[to_start + i] {
            break;
        } else if c == b'/' {
            last_common_sep = i as isize;
        }
        i += 1;
    }
    if i == length {
        if to_len > length {
            if t[to_start + i] == b'/' {
                return slice(&to, to_start + i + 1, t.len());
            }
            if i == 0 {
                return slice(&to, to_start + i, t.len());
            }
        } else if from_len > length {
            if f[from_start + i] == b'/' {
                last_common_sep = i as isize;
            } else if i == 0 {
                last_common_sep = 0;
            }
        }
    }
    let mut out = String::new();
    let mut i = (from_start as isize + last_common_sep + 1) as usize;
    while i <= from_end {
        if i == from_end || f[i] == b'/' {
            out.push_str(if out.is_empty() { ".." } else { "/.." });
        }
        i += 1;
    }
    format!("{out}{}", slice(&to, (to_start as isize + last_common_sep) as usize, t.len()))
}

// The root of a win32 path: (where the root ends, its device, whether it's
// absolute), and for a UNC root that is the whole path, the path it
// normalizes to.
struct Root {
    end: usize,
    device: Option<String>,
    absolute: bool,
}

fn win32_root(p: &str, device_roots: bool) -> Root {
    let b = p.as_bytes();
    let len = b.len();
    let is_sep = |c: u8| c == b'/' || c == b'\\';
    let mut root = Root { end: 0, device: None, absolute: false };
    if len == 0 {
        return root;
    }
    let code = b[0];
    if len == 1 {
        if is_sep(code) {
            root.end = 1;
            root.absolute = true;
        }
        return root;
    }
    if is_sep(code) {
        root.absolute = true;
        if is_sep(b[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_sep(b[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first = &p[last..j];
                last = j;
                while j < len && is_sep(b[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_sep(b[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if device_roots && (first == "." || first == "?") {
                            root.device = Some(format!("\\\\{first}"));
                            root.end = 4;
                        } else {
                            root.device = Some(format!("\\\\{first}\\{}", &p[last..j]));
                            root.end = j;
                        }
                    }
                }
            }
        } else {
            root.end = 1;
        }
    } else if device_root(b) {
        root.device = Some(p[..2].to_string());
        root.end = 2;
        if len > 2 && is_sep(b[2]) {
            root.absolute = true;
            root.end = 3;
        }
    }
    root
}

fn win32_resolve(parts: &[&str], cwd: &str) -> String {
    let mut resolved_device = String::new();
    let mut resolved_tail = String::new();
    let mut resolved_absolute = false;
    let mut i = parts.len() as isize - 1;
    while i >= -1 {
        let path: String = if i >= 0 {
            let p = parts[i as usize];
            if p.is_empty() {
                i -= 1;
                continue;
            }
            p.to_string()
        } else if resolved_device.is_empty() {
            // The working folder itself, as it is (with backslashes, off
            // Windows).
            if parts.is_empty() || (parts.len() == 1 && (parts[0].is_empty() || parts[0] == ".") && cwd.starts_with(['/', '\\'])) {
                return if cfg!(windows) { cwd.to_string() } else { cwd.replace('/', "\\") };
            }
            cwd.to_string()
        } else {
            // Node looks at the device's own working folder (=C:) first,
            // which only Windows has.
            let p = std::env::var(format!("={resolved_device}")).unwrap_or_else(|_| cwd.to_string());
            let b = p.as_bytes();
            if p.get(..2).map(str::to_lowercase) != Some(resolved_device.to_lowercase()) && b.get(2) == Some(&b'\\') {
                format!("{resolved_device}\\")
            } else {
                p
            }
        };
        let root = win32_root(&path, true);
        if let Some(device) = &root.device {
            if !resolved_device.is_empty() {
                if device.to_lowercase() != resolved_device.to_lowercase() {
                    i -= 1;
                    continue;
                }
            } else {
                resolved_device.clone_from(device);
            }
        }
        if resolved_absolute {
            if !resolved_device.is_empty() {
                break;
            }
        } else {
            resolved_tail = format!("{}\\{resolved_tail}", &path[root.end..]);
            resolved_absolute = root.absolute;
            if root.absolute && !resolved_device.is_empty() {
                break;
            }
        }
        i -= 1;
    }
    let tail = win32_normalize_string(&resolved_tail, !resolved_absolute);
    if resolved_absolute {
        format!("{resolved_device}\\{tail}")
    } else {
        let out = format!("{resolved_device}{tail}");
        if out.is_empty() {
            ".".into()
        } else {
            out
        }
    }
}

fn win32_normalize(p: &str) -> String {
    let b = p.as_bytes();
    let len = b.len();
    if len == 0 {
        return ".".into();
    }
    let is_sep = |c: u8| c == b'/' || c == b'\\';
    if len == 1 {
        return if b[0] == b'/' { "\\".into() } else { p.to_string() };
    }
    let mut root_end = 0;
    let mut device: Option<String> = None;
    let mut absolute = false;
    if is_sep(b[0]) {
        absolute = true;
        if is_sep(b[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_sep(b[j]) {
                j += 1;
            }
            if j < len && j != last {
                let first = &p[last..j];
                last = j;
                while j < len && is_sep(b[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_sep(b[j]) {
                        j += 1;
                    }
                    if j == len || j != last {
                        if first == "." || first == "?" {
                            device = Some(format!("\\\\{first}"));
                            root_end = 4;
                            // \\?\COM1: and the like.
                            let possible = p.find(':').map_or("", |k| cut(p, 4, k + 1));
                            if !possible.is_empty() && reserved(&possible[..possible.len() - 1]) {
                                device = Some(format!("\\\\?\\{possible}"));
                                root_end = 4 + possible.len();
                            }
                        } else if j == len {
                            return format!("\\\\{first}\\{}\\", &p[last..]);
                        } else {
                            device = Some(format!("\\\\{first}\\{}", &p[last..j]));
                            root_end = j;
                        }
                    }
                }
            }
        } else {
            root_end = 1;
        }
    } else if let Some(k) = p.find(':').filter(|k| *k > 0) {
        if device_root(b) && k == 1 {
            device = Some(p[..2].to_string());
            root_end = 2;
            if len > 2 && is_sep(b[2]) {
                absolute = true;
                root_end = 3;
            }
        } else if reserved(&p[..k]) {
            device = Some(p[..=k].to_string());
            root_end = k + 1;
        }
    }
    let mut tail = if root_end < len { win32_normalize_string(&p[root_end..], !absolute) } else { String::new() };
    if tail.is_empty() && !absolute {
        tail = ".".into();
    }
    if !tail.is_empty() && is_sep(b[len - 1]) {
        tail.push('\\');
    }
    if !absolute && device.is_none() && p.contains(':') {
        // Nothing Windows would read as absolute (CVE-2024-36139).
        if device_root(tail.as_bytes()) {
            return format!(".\\{tail}");
        }
        for (k, _) in p.match_indices(':') {
            if k == len - 1 || is_sep(b[k + 1]) {
                return format!(".\\{tail}");
            }
        }
    }
    if reserved_at(p, p.find(':')) {
        return format!(".\\{}{tail}", device.unwrap_or_default());
    }
    match device {
        None if absolute => format!("\\{tail}"),
        None => tail,
        Some(d) if absolute => format!("{d}\\{tail}"),
        Some(d) => format!("{d}{tail}"),
    }
}

// Windows' reserved device names, which Node's win32 normalize and join
// keep from being read as a drive or a folder.
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
    RESERVED.contains(&name.to_uppercase().as_str())
}

// isWindowsReservedName(path, colonIndex): the part before the colon, or,
// with none (-1), all but the last unit (which, cut from a pair, is no
// reserved name).
fn reserved_at(p: &str, colon: Option<usize>) -> bool {
    match colon {
        Some(k) => reserved(&p[..k]),
        None => match p.chars().last() {
            None => false,
            Some(c) if c.len_utf16() == 2 => false,
            Some(c) => reserved(&p[..p.len() - c.len_utf8()]),
        },
    }
}

// A byte range, empty when the end is before the start, as String.slice is.
fn cut(s: &str, from: usize, to: usize) -> &str {
    let to = to.min(s.len());
    if from >= to {
        ""
    } else {
        &s[from..to]
    }
}

fn win32_join(parts: &[&str]) -> String {
    let parts: Vec<&str> = parts.iter().copied().filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return ".".into();
    }
    let first = parts[0].as_bytes();
    let mut joined = parts.join("\\");
    let is_sep = |c: u8| c == b'/' || c == b'\\';
    let mut needs_replace = true;
    let mut slash_count = 0;
    if is_sep(first[0]) {
        slash_count += 1;
        if first.len() > 1 && is_sep(first[1]) {
            slash_count += 1;
            if first.len() > 2 {
                if is_sep(first[2]) {
                    slash_count += 1;
                } else {
                    needs_replace = false;
                }
            }
        }
    }
    if needs_replace {
        let jb = joined.as_bytes();
        while slash_count < jb.len() && is_sep(jb[slash_count]) {
            slash_count += 1;
        }
        if slash_count >= 2 {
            joined = format!("\\{}", &joined[slash_count..]);
        }
    }
    // A part with a reserved device name isn't normalized.
    if joined.split('\\').filter(|p| !p.is_empty()).any(|p| p.find(':').is_some_and(|k| reserved(&p[..k]))) {
        return joined.replace('/', "\\");
    }
    win32_normalize(&joined)
}

fn win32_dirname(p: &str) -> String {
    let b = p.as_bytes();
    let len = b.len();
    if len == 0 {
        return ".".into();
    }
    let is_sep = |c: u8| c == b'/' || c == b'\\';
    if len == 1 {
        return if is_sep(b[0]) { p.to_string() } else { ".".into() };
    }
    let mut root_end: isize = -1;
    let mut offset = 0usize;
    if is_sep(b[0]) {
        root_end = 1;
        offset = 1;
        if is_sep(b[1]) {
            let mut j = 2;
            let mut last = j;
            while j < len && !is_sep(b[j]) {
                j += 1;
            }
            if j < len && j != last {
                last = j;
                while j < len && is_sep(b[j]) {
                    j += 1;
                }
                if j < len && j != last {
                    last = j;
                    while j < len && !is_sep(b[j]) {
                        j += 1;
                    }
                    if j == len {
                        return p.to_string();
                    }
                    if j != last {
                        root_end = j as isize + 1;
                        offset = j + 1;
                    }
                }
            }
        }
    } else if device_root(b) {
        root_end = if len > 2 && is_sep(b[2]) { 3 } else { 2 };
        offset = root_end as usize;
    }
    let mut end: isize = -1;
    let mut matched_slash = true;
    let mut i = len as isize - 1;
    while i >= offset as isize {
        if is_sep(b[i as usize]) {
            if !matched_slash {
                end = i;
                break;
            }
        } else {
            matched_slash = false;
        }
        i -= 1;
    }
    if end == -1 {
        if root_end == -1 {
            return ".".into();
        }
        end = root_end;
    }
    p[..end as usize].to_string()
}

fn win32_relative(from: &str, to: &str, cwd: &str) -> String {
    if from == to {
        return String::new();
    }
    let from_orig = win32_resolve(&[from], cwd);
    let to_orig = win32_resolve(&[to], cwd);
    if from_orig == to_orig {
        return String::new();
    }
    let from = from_orig.to_lowercase();
    let to = to_orig.to_lowercase();
    if from == to {
        return String::new();
    }
    // Lowercasing can change a path's length (İ is two characters
    // lowercased), and then the offsets below, found in the lowercased
    // paths, don't fit the originals: Node reads the paths part by part
    // instead.
    if from.len() != from_orig.len() || to.len() != to_orig.len() {
        return win32_relative_parts(&from_orig, &to_orig);
    }
    let (f, t) = (from.as_bytes(), to.as_bytes());
    let mut from_start = 0;
    while from_start < f.len() && f[from_start] == b'\\' {
        from_start += 1;
    }
    let mut from_end = f.len();
    while from_end - 1 > from_start && f[from_end - 1] == b'\\' {
        from_end -= 1;
    }
    let from_len = from_end - from_start;
    let mut to_start = 0;
    while to_start < t.len() && t[to_start] == b'\\' {
        to_start += 1;
    }
    let mut to_end = t.len();
    while to_end - 1 > to_start && t[to_end - 1] == b'\\' {
        to_end -= 1;
    }
    let to_len = to_end - to_start;
    let length = from_len.min(to_len);
    let mut last_common_sep: isize = -1;
    let mut i = 0;
    while i < length {
        let c = f[from_start + i];
        if c != t[to_start + i] {
            break;
        } else if c == b'\\' {
            last_common_sep = i as isize;
        }
        i += 1;
    }
    if i != length {
        if last_common_sep == -1 {
            return to_orig;
        }
    } else {
        if to_len > length {
            if t[to_start + i] == b'\\' {
                return slice(&to_orig, to_start + i + 1, to_orig.len());
            }
            if i == 2 {
                return slice(&to_orig, to_start + i, to_orig.len());
            }
        }
        if from_len > length {
            if f[from_start + i] == b'\\' {
                last_common_sep = i as isize;
            } else if i == 2 {
                last_common_sep = 3;
            }
        }
        if last_common_sep == -1 {
            last_common_sep = 0;
        }
    }
    let mut out = String::new();
    let mut i = (from_start as isize + last_common_sep + 1) as usize;
    while i <= from_end {
        if i == from_end || f[i] == b'\\' {
            out.push_str(if out.is_empty() { ".." } else { "\\.." });
        }
        i += 1;
    }
    let mut to_start = (to_start as isize + last_common_sep) as usize;
    if !out.is_empty() {
        return format!("{out}{}", slice(&to_orig, to_start, to_end));
    }
    if to_orig.as_bytes().get(to_start) == Some(&b'\\') {
        to_start += 1;
    }
    slice(&to_orig, to_start, to_end)
}

// Node's win32.relative for paths whose length lowercasing changes: the
// parts in common, compared lowercased, then a ".." for each part of
// `from` left and the rest of `to`.
fn win32_relative_parts(from_orig: &str, to_orig: &str) -> String {
    let mut from: Vec<&str> = from_orig.split('\\').collect();
    let mut to: Vec<&str> = to_orig.split('\\').collect();
    if from.last() == Some(&"") {
        from.pop();
    }
    if to.last() == Some(&"") {
        to.pop();
    }
    let length = from.len().min(to.len());
    let i = (0..length).find(|&i| from[i].to_lowercase() != to[i].to_lowercase()).unwrap_or(length);
    if i == 0 {
        return to_orig.to_string();
    }
    if i == length {
        if to.len() > length {
            return to[i..].join("\\");
        }
        if from.len() > length {
            return format!("{}..", "..\\".repeat(from.len() - 1 - i));
        }
        return String::new();
    }
    format!("{}{}", "..\\".repeat(from.len() - i), to[i..].join("\\"))
}

#[cfg(test)]
mod tests {
    use super::Style::*;

    #[test]
    fn posix_as_node_has_it() {
        assert_eq!(Posix.join(&["/a/b", "../c", "", "d/"]), "/a/c/d/");
        assert_eq!(Posix.join(&[]), ".");
        assert_eq!(Posix.join(&["", ""]), ".");
        assert_eq!(Posix.resolve_from(&["a", "../b"], "/x/y"), "/x/y/b");
        assert_eq!(Posix.resolve_from(&["/a/", "b/"], "/x"), "/a/b");
        assert_eq!(Posix.resolve_from(&[""], "/x/y"), "/x/y");
        assert_eq!(Posix.normalize("./a//b/../c/."), "a/c");
        assert_eq!(Posix.normalize("../../a"), "../../a");
        assert_eq!(Posix.normalize("/../a"), "/a");
        assert_eq!(Posix.normalize("./"), "./");
        assert_eq!(Posix.dirname("/a/b/"), "/a");
        assert_eq!(Posix.dirname("a"), ".");
        assert_eq!(Posix.dirname("/"), "/");
        assert_eq!(Posix.dirname("//a"), "//");
        assert_eq!(Posix.basename("/a/b.txt/", None), "b.txt");
        assert_eq!(Posix.basename("/a/b.txt", Some(".txt")), "b");
        assert_eq!(Posix.basename("b.txt", Some("b.txt")), "");
        assert_eq!(Posix.basename("C:\\x\\y", None), "C:\\x\\y");
        assert_eq!(Posix.extname("a/b.tar.gz"), ".gz");
        assert_eq!(Posix.extname(".bashrc"), "");
        assert_eq!(Posix.extname("a."), ".");
        assert_eq!(Posix.extname("a/.."), "");
        assert_eq!(Posix.relative("/a/b/c", "/a/d"), "../../d");
        assert_eq!(Posix.relative("/a/b", "/a/b/c/d"), "c/d");
        assert_eq!(Posix.relative("/a", "/a"), "");
        assert_eq!(Posix.relative("/p/x", "/q"), "../../q");
        assert_eq!(Posix.relative("/", "/q"), "q");
        assert_eq!(Posix.resolve_from(&["/p/a", "../b"], "/x"), "/p/b");
        assert_eq!(Posix.resolve_from(&["/w/atlas/../atlas/src/"], "/x"), "/w/atlas/src");
        assert_eq!(Posix.dirname("/a//b"), "/a/");
        assert_eq!(Posix.dirname(""), ".");
        assert!(Posix.is_absolute("/x") && !Posix.is_absolute("x"));
    }

    #[test]
    fn win32_as_node_has_it() {
        assert_eq!(Win32.join(&["C:\\Windows", "System32", "reg.exe"]), "C:\\Windows\\System32\\reg.exe");
        assert_eq!(Win32.join(&["C:/a/", "../b"]), "C:\\b");
        assert_eq!(Win32.join(&["//server", "share", "x"]), "\\\\server\\share\\x");
        assert_eq!(Win32.join(&["\\\\a\\b", "c"]), "\\\\a\\b\\c");
        assert_eq!(Win32.normalize("C:"), "C:.");
        assert_eq!(Win32.normalize("C:\\"), "C:\\");
        assert_eq!(Win32.normalize("C:\\a\\..\\..\\b"), "C:\\b");
        assert_eq!(Win32.normalize("\\\\srv\\share"), "\\\\srv\\share\\");
        assert_eq!(Win32.normalize("a/b/"), "a\\b\\");
        assert_eq!(Win32.normalize("/"), "\\");
        assert_eq!(Win32.resolve_from(&["C:\\Users\\Me\\"], "/cwd"), "C:\\Users\\Me");
        assert_eq!(Win32.resolve_from(&["c:\\a", "b"], "/cwd"), "c:\\a\\b");
        assert_eq!(Win32.resolve_from(&["D:\\x", "C:\\y", "z"], "/cwd"), "C:\\y\\z");
        assert_eq!(Win32.dirname("C:\\a\\b"), "C:\\a");
        assert_eq!(Win32.dirname("C:\\a"), "C:\\");
        assert_eq!(Win32.dirname("C:a"), "C:");
        assert_eq!(Win32.dirname("\\\\srv\\share\\x"), "\\\\srv\\share\\");
        assert_eq!(Win32.basename("C:\\a\\b.exe", Some(".exe")), "b");
        assert_eq!(Win32.basename("C:foo", None), "foo");
        assert_eq!(Win32.basename("a/b\\c", None), "c");
        assert_eq!(Win32.extname("C:\\a\\b.EXE"), ".EXE");
        assert!(Win32.is_absolute("C:\\x") && Win32.is_absolute("\\x") && !Win32.is_absolute("C:x"));
        assert_eq!(Win32.relative("C:\\a\\b", "C:\\a\\c\\d"), "..\\c\\d");
        assert_eq!(Win32.relative("C:\\A", "c:\\a\\x"), "x");
        assert_eq!(Win32.relative("C:\\a", "D:\\b"), "D:\\b");
        // Lowercased, İ is two characters, so these are read part by part,
        // as Node 22 reads them.
        assert_eq!(Win32.relative("C:\\a", "C:\\İİ"), "..\\İİ");
        assert_eq!(Win32.relative("C:\\İİ\\x", "C:\\İİ\\y"), "..\\y");
        assert_eq!(Win32.relative("C:\\a", "C:\\a\\İ"), "İ");
        assert_eq!(Win32.relative("C:\\İ\\a\\b", "C:\\i̇"), "..\\..");
    }

    #[test]
    fn win32_reserved_names_and_device_roots_as_node_22_has_them() {
        // What Node 22's path.win32 said of each: normalized, and joined with "x".
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
            assert_eq!(Win32.normalize(p), *normalized, "normalize {p}");
            assert_eq!(Win32.join(&[p, "x"]), *joined, "join {p}");
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
            assert_eq!(Win32.resolve_from(&["C:\\base", p], "/cwd"), *r, "resolve {p}");
        }
        assert_eq!(Win32.resolve_from(&["C:/w/atlas/"], "/cwd"), "C:\\w\\atlas");
        assert_eq!(Win32.resolve_from(&["C:\\w\\x\\..\\y\\"], "/cwd"), "C:\\w\\y");
        assert_eq!(Win32.basename("C:\\", None), "");
        assert!(Win32.is_absolute("/w") && !Win32.is_absolute("C:"));
    }

    #[test]
    fn the_working_folder_itself() {
        assert_eq!(Posix.resolve_from(&[], "/x/y"), "/x/y");
        assert_eq!(Posix.resolve_from(&["."], "/x/y"), "/x/y");
        assert_eq!(Win32.resolve_from(&[], "/x/y"), if cfg!(windows) { "/x/y" } else { "\\x\\y" });
        assert_eq!(Win32.resolve_from(&[""], "C:\\x"), "C:\\x");
    }

    #[test]
    fn a_path_keeps_its_own_flavor() {
        use crate::paths::path_style;
        assert_eq!(super::Style::from(path_style("C:\\x")), Win32);
        assert_eq!(super::Style::from(path_style("/x")), Posix);
        assert_eq!(super::Style::from(path_style("x")), super::NATIVE);
    }
}
