//! Regular expressions as JS reads them, for the patterns the JS modules
//! use (none with lookaround): `\d`, `\w` and `\b` are ASCII's, `\s` is JS's
//! white space and line terminators, and `.` stops at any line terminator,
//! not only `\n`. Rust's own classes are Unicode's, which would match more.

use regex::Regex;
use std::fmt::Write as _;
use xenonet_base::text::{JS_DOT, JS_SPACE};

/// A JS pattern (without its slashes) in Rust's syntax.
pub fn translate(js: &str) -> String {
    let mut out = String::with_capacity(js.len() + 16);
    let mut class = false;
    let mut chars = js.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let Some(e) = chars.next() else {
                    out.push_str(r"\\");
                    break;
                };
                match (e, class) {
                    ('d', false) => out.push_str("[0-9]"),
                    ('D', false) => out.push_str("[^0-9]"),
                    ('w', false) => out.push_str("[0-9A-Za-z_]"),
                    ('W', false) => out.push_str("[^0-9A-Za-z_]"),
                    ('s', false) => {
                        let _ = write!(out, "[{JS_SPACE}]");
                    }
                    ('S', false) => {
                        let _ = write!(out, "[^{JS_SPACE}]");
                    }
                    ('d', true) => out.push_str("0-9"),
                    ('w', true) => out.push_str("0-9A-Za-z_"),
                    ('s', true) => out.push_str(JS_SPACE),
                    ('b', false) => out.push_str(r"(?-u:\b)"),
                    ('B', false) => out.push_str(r"(?-u:\B)"),
                    ('/', _) => out.push('/'),
                    ('u', _) if chars.peek().is_some_and(char::is_ascii_hexdigit) => {
                        let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                        let _ = write!(out, r"\x{{{hex}}}");
                    }
                    (e, _) => {
                        out.push('\\');
                        out.push(e);
                    }
                }
            }
            '[' if !class => {
                class = true;
                out.push('[');
                // A class that starts with ] or ^] keeps it as a character.
                if let Some(c) = chars.next_if_eq(&'^') {
                    out.push(c);
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    out.push_str(r"\]");
                }
            }
            '[' => out.push_str(r"\["),
            ']' if class => {
                class = false;
                out.push(']');
            }
            '.' if !class => out.push_str(JS_DOT),
            // Rust reads these as set operations inside a class.
            '&' | '~' | '-' if class && chars.peek() == Some(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// A JS pattern compiled, with the `i` flag when `ignore_case`.
pub fn js(pattern: &str, ignore_case: bool) -> Regex {
    let p = translate(pattern);
    Regex::new(&if ignore_case { format!("(?i){p}") } else { p }).expect("a fixed pattern: callers pass them from the source")
}

/// A pattern compiled once, the first time it's used.
#[macro_export]
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| $crate::jsre::js($p, false))
    }};
    ($p:expr, i) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| $crate::jsre::js($p, true))
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_are_ascii_and_js_space() {
        assert!(js(r"^\d+$", false).is_match("123"));
        assert!(!js(r"^\d+$", false).is_match("١٢٣"));
        assert!(js(r"^\s$", false).is_match("\u{feff}"));
        assert!(!js(r"^\s$", false).is_match("\u{85}"));
        assert!(js(r"\bab\b", false).is_match("x ab y"));
        assert!(js(r"\bab\b", false).is_match("éab"), "é isn't a word character in JS");
        assert!(!js(r"^.$", false).is_match("\r"));
        assert!(js(r"^[\w.-]+$", false).is_match("a.b-c_d"));
        assert!(js(r"^[^/]+\/x$", false).is_match("ab/x"));
        assert!(js(r"^\u00e9$", false).is_match("é"));
    }
}
