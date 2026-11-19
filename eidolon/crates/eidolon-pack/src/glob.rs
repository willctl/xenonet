//! The small glob the leave-out lists use.
//!
//! - `*` matches any run of characters except `/`.
//! - `?` matches one character except `/`.
//! - `**` matches any run of characters including `/`; `**/` also matches
//!   nothing, so `a/**/b` matches `a/b`.
//! - `dir/**` also matches `dir` itself, so an excluded folder is not
//!   recreated empty.
//! - A pattern with no `/` matches a name at any depth (`*.lock` leaves out
//!   `a/b/x.lock` too). A pattern with a `/` is matched from the facet root.

/// Whether the relative `path` (`/`-separated) is left out by `pattern`.
pub fn matches(pattern: &str, path: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    if !pattern.contains('/') {
        // Any component, so a matching folder takes its contents with it.
        return path.split('/').any(|part| {
            let t: Vec<char> = part.chars().collect();
            m(&p, &t)
        });
    }
    let t: Vec<char> = path.chars().collect();
    if m(&p, &t) {
        return true;
    }
    // `dir/**` leaves out `dir` too.
    pattern.strip_suffix("/**").is_some_and(|base| {
        let b: Vec<char> = base.chars().collect();
        m(&b, &t)
    })
}

fn m(p: &[char], t: &[char]) -> bool {
    match p {
        [] => t.is_empty(),
        ['*', '*', '/', rest @ ..] => {
            if m(rest, t) {
                return true;
            }
            (1..=t.len()).any(|k| t[k - 1] == '/' && m(rest, &t[k..]))
        }
        ['*', '*', rest @ ..] => (0..=t.len()).any(|k| m(rest, &t[k..])),
        ['*', rest @ ..] => {
            let span = t.iter().position(|c| *c == '/').unwrap_or(t.len());
            (0..=span).any(|k| m(rest, &t[k..]))
        }
        ['?', rest @ ..] => matches!(t.first(), Some(c) if *c != '/') && m(rest, &t[1..]),
        [c, rest @ ..] => t.first() == Some(c) && m(rest, &t[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn the_cases_the_leave_out_lists_use() {
        assert!(matches("cache2/**", "cache2/entries/a"));
        assert!(matches("cache2/**", "cache2"));
        assert!(!matches("cache2/**", "cache22/a"));
        assert!(!matches("cache2/**", "x/cache2/a"), "a pattern with a slash starts at the root");
        assert!(matches("*.lock", "a.lock"));
        assert!(matches("*.lock", "deep/er/a.lock"));
        assert!(!matches("*.lock", "a.lock.txt"));
        assert!(matches("a/**/b", "a/b"));
        assert!(matches("a/**/b", "a/x/y/b"));
        assert!(!matches("a/*/b", "a/x/y/b"));
        assert!(matches("a/*/b", "a/x/b"));
        assert!(matches("file?.txt", "file1.txt"));
        assert!(!matches("file?.txt", "file/.txt"));
        assert!(matches("naïve*", "naïve-é"));
        assert!(matches("**/tmp", "a/b/tmp"));
    }
}
