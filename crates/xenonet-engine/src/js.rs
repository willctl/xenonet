//! Values as JS reads them, for the modules that take what a caller hands
//! them as it comes: a template literal's text, truthiness, whole numbers.

use serde_json::Value;
use xenonet_base::text::js_string;

/// A value as a template literal writes it (`${v}`), undefined for a
/// field that isn't there.
pub fn template(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => js_string(v),
    }
}

/// JS's truthiness.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// Number.isInteger.
pub fn is_integer(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|f| f.is_finite() && f.trunc() == *f)
}

/// Two texts in JS's order (`a < b`): by UTF-16 code units, which differs
/// from Rust's (by code points) only above U+FFFF.
pub fn utf16_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

// Where a character sorts under ICU's root collation, which
// String.prototype.localeCompare uses: white space, then punctuation and
// symbols in CLDR's order, then digits, then letters with case aside; the
// rest after, by code point. Control characters count for nothing. This is
// exact for ASCII; elsewhere it's an approximation of ICU's tables.
fn collation(c: char) -> Option<(u32, u8)> {
    const PUNCT: &str = "\t\n\u{b}\u{c}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";
    if let Some(i) = PUNCT.find(c) {
        return Some((PUNCT[..i].chars().count() as u32 + 1, 0));
    }
    if c.is_ascii_digit() {
        return Some((100 + c as u32 - '0' as u32, 0));
    }
    if c.is_ascii_alphabetic() {
        return Some((200 + c.to_ascii_lowercase() as u32 - 'a' as u32, u8::from(c.is_ascii_uppercase())));
    }
    if c.is_control() {
        return None;
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    Some((1000 + lower as u32, u8::from(lower != c)))
}

/// `a.localeCompare(b)` as ICU's root collation orders text: letters
/// without case first, then a small letter before its capital.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let ka: Vec<(u32, u8)> = a.chars().filter_map(collation).collect();
    let kb: Vec<(u32, u8)> = b.chars().filter_map(collation).collect();
    let primary = ka.iter().map(|k| k.0).cmp(kb.iter().map(|k| k.0));
    primary.then_with(|| ka.iter().map(|k| k.1).cmp(kb.iter().map(|k| k.1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering::*;

    #[test]
    fn orders() {
        assert_eq!(locale_compare("a", "B"), Less);
        assert_eq!(locale_compare("a", "A"), Less);
        assert_eq!(locale_compare("Wren", "hermes"), Greater);
        assert_eq!(locale_compare("a-b", "ab"), Less);
        assert_eq!(locale_compare("x", "x"), Equal);
        assert_eq!(utf16_cmp("\u{ffff}", "\u{10000}"), Greater);
    }
}

/// `pathStyle(base).join(base, ...parts)`: joined in the syntax `base` keeps
/// (core/paths.js), and normalized as Node's path module normalizes.
pub fn join_style(base: &str, parts: &[&str]) -> String {
    join_in(base, base, parts)
}

/// `pathStyle(style_of).join(base, ...parts)`.
pub fn join_in(style_of: &str, base: &str, parts: &[&str]) -> String {
    use xenonet_base::paths::{normalize_posix, path_style, PathStyle};
    let all: Vec<&str> = std::iter::once(base).chain(parts.iter().copied()).filter(|p| !p.is_empty()).collect();
    match path_style(style_of) {
        PathStyle::Posix => normalize_posix(&all.join("/")),
        PathStyle::Windows => normalize_win32(&all.join("\\")),
        PathStyle::Native => {
            if cfg!(windows) {
                normalize_win32(&all.join("\\"))
            } else {
                normalize_posix(&all.join("/"))
            }
        }
    }
}

/// `pathStyle(p).resolve(p)` for an absolute path: normalized in the
/// syntax it keeps, without a separator at the end (but a root's).
pub fn resolve_style(p: &str) -> String {
    use xenonet_base::paths::{normalize_posix, path_style, resolve, PathStyle};
    match path_style(p) {
        PathStyle::Posix => {
            let n = normalize_posix(p);
            if n.len() > 1 {
                n.trim_end_matches('/').to_string()
            } else {
                n
            }
        }
        PathStyle::Windows => {
            let n = normalize_win32(p);
            // A UNC root is \\server\share\, a drive's C:\.
            let root = if n.starts_with("\\\\") {
                n.match_indices('\\').nth(3).map_or(n.len(), |(i, _)| i + 1)
            } else if n.as_bytes().get(1) == Some(&b':') {
                3
            } else {
                1
            };
            if n.len() > root {
                n.trim_end_matches('\\').to_string()
            } else {
                n
            }
        }
        PathStyle::Native => resolve(p),
    }
}

// path.win32.normalize, for the paths the modules join: a drive or UNC
// root kept, separators made backslashes and collapsed, . and .. worked out.
fn normalize_win32(p: &str) -> String {
    let s = p.replace('/', "\\");
    let (root, rest) = if s.len() >= 2 && s.as_bytes()[1] == b':' {
        let drive = &s[..2];
        let rest = &s[2..];
        if rest.starts_with('\\') {
            (format!("{drive}\\"), rest.trim_start_matches('\\'))
        } else {
            (drive.to_string(), rest)
        }
    } else if s.starts_with("\\\\") {
        ("\\\\".to_string(), s.trim_start_matches('\\'))
    } else if s.starts_with('\\') {
        ("\\".to_string(), s.trim_start_matches('\\'))
    } else {
        (String::new(), s.as_str())
    };
    let trailing = rest.ends_with('\\');
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|l| *l != "..") {
                    parts.pop();
                } else if root.is_empty() {
                    parts.push("..");
                }
            }
            seg => parts.push(seg),
        }
    }
    let mut out = format!("{root}{}", parts.join("\\"));
    if out.is_empty() {
        out = ".".into();
    }
    if trailing && !parts.is_empty() {
        out.push('\\');
    }
    out
}

/// What an object literal reads under `name` that it doesn't have itself:
/// Object.prototype's member of that name, as String() writes it, or None.
pub fn proto_member(name: &str) -> Option<String> {
    match name {
        "constructor" => Some("function Object() { [native code] }".into()),
        "__proto__" => Some("[object Object]".into()),
        "__defineGetter__"
        | "__defineSetter__"
        | "hasOwnProperty"
        | "__lookupGetter__"
        | "__lookupSetter__"
        | "isPrototypeOf"
        | "propertyIsEnumerable"
        | "toString"
        | "valueOf"
        | "toLocaleString" => Some(format!("function {name}() {{ [native code] }}")),
        _ => None,
    }
}

/// Node's Buffer.from(text, "base64"): either alphabet, anything else
/// skipped, and nothing after the first "=".
pub fn base64_bytes(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        bits = bits << 6 | v as u32;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    out
}

pub fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Buffer.from(text, "utf16le").
pub fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// buffer.toString("utf16le"): a byte left over is dropped.
pub fn from_utf16le(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
    String::from_utf16_lossy(&units)
}

// ---------- paths, as Node's path module works them out ----------

/// path.isAbsolute on this machine.
pub fn is_absolute_host(p: &str) -> bool {
    if cfg!(windows) {
        let b = p.as_bytes();
        p.starts_with('/') || p.starts_with('\\') || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'/' || b[2] == b'\\'))
    } else {
        p.starts_with('/')
    }
}

/// path.basename on this machine.
pub fn basename_host(p: &str) -> String {
    let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
    let s = p.trim_end_matches(seps);
    s.rsplit(seps).next().unwrap_or("").to_string()
}

/// path.posix.dirname (path.win32's with `win32`).
pub fn dirname_of(p: &str, win32: bool) -> String {
    let sep = |c: u8| c == b'/' || (win32 && c == b'\\');
    let b = p.as_bytes();
    if b.is_empty() {
        return ".".into();
    }
    // A drive's root is kept whole.
    let start = if win32 && b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' { 2 } else { 0 };
    let root = b.len() > start && sep(b[start]);
    let mut end = None;
    let mut matched_slash = true;
    for i in (start + 1..b.len()).rev() {
        if sep(b[i]) {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None if root => p[..start + 1].to_string(),
        None if start > 0 => p[..start].to_string(),
        None => ".".into(),
        Some(1) if root && start == 0 && !win32 => p[..2].to_string(),
        Some(e) => p[..e].to_string(),
    }
}

/// `pathStyle(p).dirname(p)`.
pub fn dirname_style(p: &str) -> String {
    use xenonet_base::paths::{path_style, PathStyle};
    match path_style(p) {
        PathStyle::Windows => dirname_of(p, true),
        PathStyle::Posix => dirname_of(p, false),
        PathStyle::Native => dirname_of(p, cfg!(windows)),
    }
}

fn posix_resolve(base: &str, p: &str) -> String {
    let full = if p.starts_with('/') {
        p.to_string()
    } else if base.starts_with('/') {
        format!("{base}/{p}")
    } else {
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into());
        format!("{cwd}/{base}/{p}")
    };
    let n = xenonet_base::paths::normalize_posix(&full);
    if n.len() > 1 {
        n.trim_end_matches('/').to_string()
    } else {
        n
    }
}

fn win32_resolve(base: &str, p: &str) -> String {
    let drive = |s: &str| {
        let b = s.as_bytes();
        (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':').then(|| s[..2].to_string())
    };
    let rooted = |s: &str| s.starts_with('\\') || s.starts_with('/');
    let full = if drive(p).is_some() && p.len() > 2 && rooted(&p[2..]) {
        p.to_string()
    } else if p.starts_with("\\\\") || p.starts_with("//") {
        p.to_string()
    } else if rooted(p) {
        format!("{}{p}", drive(base).unwrap_or_default())
    } else {
        format!("{base}\\{p}")
    };
    let n = normalize_win32(&full);
    let root = if n.starts_with("\\\\") {
        n.match_indices('\\').nth(3).map_or(n.len(), |(i, _)| i + 1)
    } else if n.as_bytes().get(1) == Some(&b':') {
        3
    } else {
        1
    };
    if n.len() > root {
        n.trim_end_matches('\\').to_string()
    } else {
        n
    }
}

/// `pathStyle(styleOf).resolve(base, p)`.
pub fn resolve_in(style_of: &str, base: &str, p: &str) -> String {
    use xenonet_base::paths::{path_style, PathStyle};
    match path_style(style_of) {
        PathStyle::Windows => win32_resolve(base, p),
        PathStyle::Posix => posix_resolve(base, p),
        PathStyle::Native => {
            if cfg!(windows) {
                win32_resolve(base, p)
            } else {
                posix_resolve(base, p)
            }
        }
    }
}

/// path.relative on this machine (POSIX's).
pub fn relative_host(from: &str, to: &str) -> String {
    let a = posix_resolve("/", from);
    let b = posix_resolve("/", to);
    if a == b {
        return String::new();
    }
    let a: Vec<&str> = a.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = b.split('/').filter(|s| !s.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out: Vec<&str> = vec![".."; a.len() - common];
    out.extend(&b[common..]);
    out.join("/")
}
