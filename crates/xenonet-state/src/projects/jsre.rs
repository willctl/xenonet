//! The few regular expressions and path functions projects.js and
//! identities.js use, matched by hand as JS matches them: JS's `\s` (its
//! white space and line terminators), `.` that stops at any line
//! terminator, and `^`/`$` at each line with the m flag. Node's path
//! functions as they treat a string, for the paths these modules build.

use std::fs::File;
use std::io::Read;
use xenonet_base::paths;
use xenonet_base::text::is_js_space;

/// A line terminator, where JS's `.` stops and its `^`/`$` (with m) match.
pub(crate) fn is_term(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn at(chars: &[char], i: usize, lit: &str) -> bool {
    let mut j = i;
    for c in lit.chars() {
        if chars.get(j) != Some(&c) {
            return false;
        }
        j += 1;
    }
    true
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && is_js_space(chars[i]) {
        i += 1;
    }
    i
}

fn text_of(chars: &[char], from: usize, to: usize) -> String {
    chars[from..to].iter().collect()
}

// Where `\s*$` (with m) ends when it begins at q, as the greedy `\s*`
// leaves it: the end of the text when the white space reaches it, else
// the last line terminator in it. None when there's no line end in it.
fn space_to_line_end(chars: &[char], q: usize) -> Option<usize> {
    let run = skip_space(chars, q);
    if run == chars.len() {
        return Some(run);
    }
    (q..run).rev().find(|&r| is_term(chars[r]))
}

// /^\s*KEY\s*=\s*(.+?)\s*$/m at a line start p: the value and where the
// match ends.
fn assignment_at(chars: &[char], p: usize, key: &str) -> Option<(String, usize)> {
    let n = chars.len();
    let mut i = skip_space(chars, p);
    if !at(chars, i, key) {
        return None;
    }
    i = skip_space(chars, i + key.chars().count());
    if chars.get(i) != Some(&'=') {
        return None;
    }
    let s0 = i + 1;
    let j = skip_space(chars, s0);
    if j < n {
        // The value starts at the first character that isn't white space,
        // and is the least that leaves only white space to the line's end.
        let mut q = j + 1;
        loop {
            if let Some(end) = space_to_line_end(chars, q) {
                return Some((text_of(chars, j, q), end));
            }
            if q >= n || is_term(chars[q]) {
                return None;
            }
            q += 1;
        }
    }
    // White space to the end of the text: the greedy \s* gives back its
    // last character that isn't a line terminator, which is the value.
    let s = (s0..n).rev().find(|&s| !is_term(chars[s]))?;
    Some((text_of(chars, s, s + 1), n))
}

/// Every value of /^\s*KEY\s*=\s*(.+?)\s*$/gm in a text: a .gitmodules'
/// paths, a git config's include paths.
pub(crate) fn assignments(text: &str, key: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    // Each match is searched for from where the last one ended.
    let mut p = 0;
    while p <= n {
        if p == 0 || is_term(chars[p - 1]) {
            if let Some((value, end)) = assignment_at(&chars, p, key) {
                out.push(value);
                p = if end == p { end + 1 } else { end };
                continue;
            }
        }
        p += 1;
    }
    out
}

/// /PREFIX\s*(.+)/.exec(text)?.[1]: what follows the first prefix that has
/// something after it, to the end of its line.
pub(crate) fn rest_after(text: &str, prefix: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let line_end = |from: usize| (from..n).find(|&e| is_term(chars[e])).unwrap_or(n);
    for start in 0..n {
        if !at(&chars, start, prefix) {
            continue;
        }
        let i = start + prefix.chars().count();
        let j = skip_space(&chars, i);
        if j < n {
            return Some(text_of(&chars, j, line_end(j)));
        }
        if let Some(s) = (i..n).rev().find(|&s| !is_term(chars[s])) {
            return Some(text_of(&chars, s, line_end(s)));
        }
    }
    None
}

/// /PREFIX\s*LITERAL(.+)/.exec(text)?.[1], for a literal that starts with
/// something other than white space.
pub(crate) fn rest_after_both(text: &str, prefix: &str, literal: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    for start in 0..n {
        if !at(&chars, start, prefix) {
            continue;
        }
        let i = skip_space(&chars, start + prefix.chars().count());
        if !at(&chars, i, literal) {
            continue;
        }
        let k = i + literal.chars().count();
        let e = (k..n).find(|&e| is_term(chars[e])).unwrap_or(n);
        if e > k {
            return Some(text_of(&chars, k, e));
        }
    }
    None
}

/// /\[remote "origin"\][^[]*?url\s*=\s*(\S+)/.exec(text)?.[1]: the origin's
/// url in a git config.
pub(crate) fn origin_url(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    const HEAD: &str = "[remote \"origin\"]";
    for h in 0..n {
        if !at(&chars, h, HEAD) {
            continue;
        }
        let mut i = h + HEAD.chars().count();
        while i < n && chars[i] != '[' {
            if at(&chars, i, "url") {
                let j = skip_space(&chars, i + 3);
                if chars.get(j) == Some(&'=') {
                    let k = skip_space(&chars, j + 1);
                    let mut e = k;
                    while e < n && !is_js_space(chars[e]) {
                        e += 1;
                    }
                    if e > k {
                        return Some(text_of(&chars, k, e));
                    }
                }
            }
            i += 1;
        }
    }
    None
}

/// Whether some line of a text holds `literal` after nothing but white
/// space: /^\s*LITERAL/m.
pub(crate) fn line_starts_with(text: &str, literal: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    for i in 0..chars.len() {
        if !at(&chars, i, literal) {
            continue;
        }
        let mut j = i;
        loop {
            if j == 0 {
                return true;
            }
            let c = chars[j - 1];
            if is_term(c) {
                return true;
            }
            if !is_js_space(c) {
                break;
            }
            j -= 1;
        }
    }
    false
}

/// The first `max` bytes of a file as UTF-8 (what can't be read as it,
/// U+FFFD), or None when it can't be read.
pub(crate) fn read_text(file: &str, max: u64) -> Option<String> {
    let f = File::open(file).ok()?;
    let mut buf = Vec::new();
    f.take(max).read_to_end(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

// ---------- Node's path functions ----------

/// path.dirname.
pub(crate) fn dirname(p: &str) -> String {
    if cfg!(windows) {
        return std::path::Path::new(p).parent().map_or_else(|| p.to_string(), |d| d.to_string_lossy().into_owned());
    }
    let b = p.as_bytes();
    if b.is_empty() {
        return ".".into();
    }
    let root = b[0] == b'/';
    let mut end = None;
    let mut matched_slash = true;
    for i in (1..b.len()).rev() {
        if b[i] == b'/' {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None => (if root { "/" } else { "." }).into(),
        Some(1) if root => "//".into(),
        Some(e) => p[..e].to_string(),
    }
}

/// path.basename.
pub(crate) fn basename(p: &str) -> String {
    if cfg!(windows) {
        return std::path::Path::new(p).file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    }
    p.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string()
}

/// path.resolve(base, p).
pub(crate) fn resolve_from(base: &str, p: &str) -> String {
    if std::path::Path::new(p).is_absolute() {
        paths::resolve(p)
    } else {
        paths::resolve(&paths::join(base, p))
    }
}

/// path.relative(from, to), with forward slashes.
pub(crate) fn relative(from: &str, to: &str) -> String {
    let (from, to) = (paths::resolve(from), paths::resolve(to));
    if cfg!(windows) {
        let f: Vec<String> = from.split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_string).collect();
        let t: Vec<String> = to.split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_string).collect();
        let same = f.iter().zip(&t).take_while(|(a, b)| a.to_lowercase() == b.to_lowercase()).count();
        let mut parts: Vec<String> = vec!["..".into(); f.len() - same];
        parts.extend(t[same..].iter().cloned());
        return parts.join("/");
    }
    if from == to {
        return String::new();
    }
    let (f, t) = (from.as_bytes(), to.as_bytes());
    let (from_start, from_end, to_start) = (1, f.len(), 1);
    let from_len = from_end - from_start;
    let to_len = t.len() - to_start;
    let length = from_len.min(to_len);
    let mut last_sep: i64 = -1;
    let mut i = 0;
    while i < length {
        let c = f[from_start + i];
        if c != t[to_start + i] {
            break;
        } else if c == b'/' {
            last_sep = i as i64;
        }
        i += 1;
    }
    if i == length {
        if to_len > length {
            if t[to_start + i] == b'/' {
                return to[to_start + i + 1..].to_string();
            }
            if i == 0 {
                return to[to_start + i..].to_string();
            }
        } else if from_len > length {
            if f[from_start + i] == b'/' {
                last_sep = i as i64;
            } else if i == 0 {
                last_sep = 0;
            }
        }
    }
    let mut out = String::new();
    let mut k = (from_start as i64 + last_sep + 1) as usize;
    while k <= from_end {
        if k == from_end || f[k] == b'/' {
            out.push_str(if out.is_empty() { ".." } else { "/.." });
        }
        k += 1;
    }
    let rest_at = (to_start as i64 + last_sep) as usize;
    format!("{out}{}", &to[rest_at.min(to.len())..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_assignments_as_js_does() {
        let gm = "[submodule \"a\"]\n\tpath = libs/engine\n[submodule \"b\"]\n\tpath = art  \n";
        assert_eq!(assignments(gm, "path"), ["libs/engine", "art"]);
        assert_eq!(assignments("path =\n  next", "path"), ["next"]);
        assert_eq!(assignments("x\npath = ", "path"), [" "]);
        assert_eq!(assignments("path =\n", "path"), Vec::<String>::new());
        assert_eq!(assignments("  path=a b \r\npath = c", "path"), ["a b", "c"]);
    }

    #[test]
    fn reads_the_rest_of_a_line() {
        assert_eq!(rest_after("gitdir: /x/.git/worktrees/a\n", "gitdir:").as_deref(), Some("/x/.git/worktrees/a"));
        assert_eq!(rest_after("nothing", "gitdir:"), None);
        assert_eq!(rest_after("gitdir:", "gitdir:"), None);
        assert_eq!(rest_after("gitdir:  ", "gitdir:").as_deref(), Some(" "));
        assert_eq!(rest_after_both("ref: refs/heads/main\n", "ref:", "refs/heads/").as_deref(), Some("main"));
        assert_eq!(rest_after_both("ref: refs/heads/\nref:refs/heads/b", "ref:", "refs/heads/").as_deref(), Some("b"));
    }

    #[test]
    fn finds_the_origin() {
        let cfg = "[core]\n\turl = no\n[remote \"origin\"]\n\tfetch = x\n\turl = git@example.com:me/app.git\n";
        assert_eq!(origin_url(cfg).as_deref(), Some("git@example.com:me/app.git"));
        assert_eq!(origin_url("[remote \"origin\"]\n[remote \"x\"]\nurl = a"), None);
        assert_eq!(origin_url("[remote \"origin\"]\npushurl = a\nurl = b").as_deref(), Some("a"));
    }

    #[test]
    fn finds_a_line_start() {
        assert!(line_starts_with("[package]\n  [workspace]\n", "[workspace]"));
        assert!(!line_starts_with("x [workspace]", "[workspace]"));
        assert!(line_starts_with("[workspace]", "[workspace]"));
    }

    #[cfg(unix)]
    #[test]
    fn paths_as_node_has_them() {
        assert_eq!(dirname("/a/b/c"), "/a/b");
        assert_eq!(dirname("/a/b/"), "/a");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("a"), ".");
        assert_eq!(basename("/a/b/"), "b");
        assert_eq!(relative("/p", "/p/a/b"), "a/b");
        assert_eq!(relative("/p/a", "/p/b"), "../b");
        assert_eq!(relative("/p", "/p"), "");
        assert_eq!(relative("/p/x", "/q"), "../../q");
        assert_eq!(relative("/", "/q"), "q");
        assert_eq!(resolve_from("/p/a", "../b"), "/p/b");
        assert_eq!(resolve_from("/p/a", "/x/y/"), "/x/y");
    }
}
