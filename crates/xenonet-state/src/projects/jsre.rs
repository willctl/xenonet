//! The few regular expressions projects.js and identities.js use, matched
//! by hand as JS matches them: JS's `\s` (its white space and line
//! terminators), `.` that stops at any line terminator, and `^`/`$` at
//! each line with the m flag.

use std::fs::File;
use std::io::Read;
use xenonet_text::text::{is_line_terminator, is_space};

fn at(chars: &[char], i: usize, lit: &str) -> bool {
    lit.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c))
}

fn skip_space(chars: &[char], mut i: usize) -> usize {
    while i < chars.len() && is_space(chars[i]) {
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
    (q..run).rev().find(|&r| is_line_terminator(chars[r]))
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
            if q >= n || is_line_terminator(chars[q]) {
                return None;
            }
            q += 1;
        }
    }
    // White space to the end of the text: the greedy \s* gives back its
    // last character that isn't a line terminator, which is the value.
    let s = (s0..n).rev().find(|&s| !is_line_terminator(chars[s]))?;
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
        if p == 0 || is_line_terminator(chars[p - 1]) {
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
    let line_end = |from: usize| (from..n).find(|&e| is_line_terminator(chars[e])).unwrap_or(n);
    for start in 0..n {
        if !at(&chars, start, prefix) {
            continue;
        }
        let i = start + prefix.chars().count();
        let j = skip_space(&chars, i);
        if j < n {
            return Some(text_of(&chars, j, line_end(j)));
        }
        if let Some(s) = (i..n).rev().find(|&s| !is_line_terminator(chars[s])) {
            return Some(text_of(&chars, s, line_end(s)));
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
            if is_line_terminator(c) {
                return true;
            }
            if !is_space(c) {
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
    }

    #[test]
    fn finds_a_line_start() {
        assert!(line_starts_with("[package]\n  [workspace]\n", "[workspace]"));
        assert!(!line_starts_with("x [workspace]", "[workspace]"));
        assert!(line_starts_with("[workspace]", "[workspace]"));
    }
}
