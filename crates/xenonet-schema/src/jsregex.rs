//! Regular expressions as JS runs them without flags, for the patterns in
//! XenoNet's schemas (a JSON Schema `pattern` is the source of a JS RegExp,
//! and zod tests it with `RegExp.prototype.test`).
//!
//! JS without the u flag matches UTF-16 code units, `.` stops at line
//! terminators, `\d`, `\w` and `\b` are ASCII's, `\s` is JS's own white
//! space, and Annex B reads a `{` that isn't a quantifier, and an escape
//! that means nothing, as the character itself. The `regex` crate differs
//! on most of those, so this is a small backtracking matcher of its own.
//! Backreferences aren't supported (no schema uses one): compiling one is
//! an error rather than a quiet mismatch.

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    node: Node,
}

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Unit(u16),
    Set(Set),
    Start,
    End,
    WordBoundary(bool),
    Look { ahead: bool, negate: bool, node: Box<Node> },
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32>, greedy: bool },
}

/// A set of code units: ranges, inclusive, or their complement.
#[derive(Clone, Debug)]
struct Set {
    ranges: Vec<(u16, u16)>,
    negate: bool,
}

impl Set {
    fn has(&self, u: u16) -> bool {
        self.ranges.iter().any(|&(a, b)| a <= u && u <= b) != self.negate
    }
}

const DIGIT: &[(u16, u16)] = &[(0x30, 0x39)];
const WORD: &[(u16, u16)] = &[(0x30, 0x39), (0x41, 0x5a), (0x5f, 0x5f), (0x61, 0x7a)];
// WhiteSpace and LineTerminator, as JS's \s has them.
const SPACE: &[(u16, u16)] =
    &[(0x09, 0x0d), (0x20, 0x20), (0xa0, 0xa0), (0x1680, 0x1680), (0x2000, 0x200a), (0x2028, 0x2029), (0x202f, 0x202f), (0x205f, 0x205f), (0x3000, 0x3000), (0xfeff, 0xfeff)];
// What `.` doesn't match: \n, \r, U+2028 and U+2029.
const LINE_ENDS: &[(u16, u16)] = &[(0x0a, 0x0a), (0x0d, 0x0d), (0x2028, 0x2029)];

fn complement(ranges: &[(u16, u16)]) -> Vec<(u16, u16)> {
    let mut sorted = ranges.to_vec();
    sorted.sort_unstable();
    let mut out = Vec::new();
    let mut next: u32 = 0;
    for (a, b) in sorted {
        if u32::from(a) > next {
            out.push((next as u16, a - 1));
        }
        next = next.max(u32::from(b) + 1);
    }
    if next <= 0xffff {
        out.push((next as u16, 0xffff));
    }
    out
}

fn is_word(u: u16) -> bool {
    WORD.iter().any(|&(a, b)| a <= u && u <= b)
}

/// Compiles a pattern's source (what RegExp.prototype.source gives), or
/// says why it can't.
pub fn compile(source: &str) -> Result<Regex, String> {
    let units: Vec<u16> = source.encode_utf16().collect();
    let mut p = Parser { s: &units, i: 0, groups: count_groups(&units) };
    let node = p.alternation()?;
    if p.i < p.s.len() {
        return Err(format!("unmatched ) in /{source}/"));
    }
    Ok(Regex { node })
}

impl Regex {
    /// Whether the pattern matches anywhere in `text`, as `test` says.
    pub fn test(&self, text: &str) -> bool {
        let s: Vec<u16> = text.encode_utf16().collect();
        (0..=s.len()).any(|start| run(&self.node, &s, start, &mut |_| true))
    }
}

// Capturing groups, for telling a backreference from an octal escape.
fn count_groups(s: &[u16]) -> usize {
    let mut n = 0;
    let mut i = 0;
    let mut in_class = false;
    while i < s.len() {
        match s[i] {
            0x5c => i += 1,
            0x5b => in_class = true,
            0x5d => in_class = false,
            0x28 if !in_class => {
                let named = s.get(i + 1) == Some(&0x3f) && s.get(i + 2) == Some(&0x3c) && !matches!(s.get(i + 3), Some(&0x3d) | Some(&0x21));
                if s.get(i + 1) != Some(&0x3f) || named {
                    n += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    n
}

struct Parser<'a> {
    s: &'a [u16],
    i: usize,
    groups: usize,
}

fn hex_value(units: &[u16]) -> Option<u32> {
    let mut v = 0u32;
    for &u in units {
        let d = char::from_u32(u32::from(u))?.to_digit(16)?;
        v = v * 16 + d;
    }
    Some(v)
}

impl Parser<'_> {
    fn peek(&self) -> Option<u16> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, u: u16) -> bool {
        if self.peek() == Some(u) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn alternation(&mut self) -> Result<Node, String> {
        let mut options = vec![self.sequence()?];
        while self.eat(b'|' as u16) {
            options.push(self.sequence()?);
        }
        Ok(if options.len() == 1 { options.pop().unwrap_or(Node::Empty) } else { Node::Alt(options) })
    }

    fn sequence(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(u) = self.peek() {
            if u == b'|' as u16 || u == b')' as u16 {
                break;
            }
            let atom = self.term()?;
            items.push(atom);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap_or(Node::Empty),
            _ => Node::Concat(items),
        })
    }

    fn term(&mut self) -> Result<Node, String> {
        let u = self.peek().unwrap_or(0);
        let atom = match u {
            0x5e => {
                self.i += 1;
                return Ok(Node::Start);
            }
            0x24 => {
                self.i += 1;
                return Ok(Node::End);
            }
            0x5c if matches!(self.s.get(self.i + 1), Some(&0x62) | Some(&0x42)) => {
                let yes = self.s[self.i + 1] == 0x62;
                self.i += 2;
                return Ok(Node::WordBoundary(yes));
            }
            0x28 => self.group()?,
            0x2e => {
                self.i += 1;
                Node::Set(Set { ranges: LINE_ENDS.to_vec(), negate: true })
            }
            0x5b => self.class()?,
            0x5c => self.escape()?,
            0x2a | 0x2b | 0x3f => return Err("nothing to repeat".into()),
            0x7b => {
                if self.quantifier_at(self.i).is_some() {
                    return Err("nothing to repeat".into());
                }
                self.i += 1;
                Node::Unit(u)
            }
            _ => {
                self.i += 1;
                Node::Unit(u)
            }
        };
        self.quantified(atom)
    }

    // {n}, {n,} or {n,m} at `at`, with where it ends.
    fn quantifier_at(&self, at: usize) -> Option<(u32, Option<u32>, usize)> {
        let s = self.s;
        if s.get(at) != Some(&0x7b) {
            return None;
        }
        let digits = |mut j: usize| {
            let start = j;
            let mut v: u64 = 0;
            while let Some(&d) = s.get(j) {
                if !(0x30..=0x39).contains(&d) {
                    break;
                }
                v = (v * 10 + u64::from(d - 0x30)).min(u64::from(u32::MAX));
                j += 1;
            }
            (j > start).then_some((v as u32, j))
        };
        let (min, mut j) = digits(at + 1)?;
        let max = if s.get(j) == Some(&0x2c) {
            j += 1;
            match digits(j) {
                Some((m, k)) => {
                    j = k;
                    Some(m)
                }
                None => None,
            }
        } else {
            Some(min)
        };
        if s.get(j) != Some(&0x7d) {
            return None;
        }
        Some((min, max, j + 1))
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some(0x2a) => {
                self.i += 1;
                (0, None)
            }
            Some(0x2b) => {
                self.i += 1;
                (1, None)
            }
            Some(0x3f) => {
                self.i += 1;
                (0, Some(1))
            }
            Some(0x7b) => match self.quantifier_at(self.i) {
                Some((min, max, end)) => {
                    self.i = end;
                    if max.is_some_and(|m| m < min) {
                        return Err("numbers out of order in {} quantifier".into());
                    }
                    (min, max)
                }
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        if matches!(atom, Node::Start | Node::End | Node::WordBoundary(_)) {
            return Err("nothing to repeat".into());
        }
        let greedy = !self.eat(0x3f);
        Ok(Node::Repeat { node: Box::new(atom), min, max, greedy })
    }

    fn group(&mut self) -> Result<Node, String> {
        self.i += 1;
        let mut look: Option<(bool, bool)> = None;
        if self.eat(0x3f) {
            match self.peek() {
                Some(0x3a) => self.i += 1,
                Some(0x3d) => {
                    self.i += 1;
                    look = Some((true, false));
                }
                Some(0x21) => {
                    self.i += 1;
                    look = Some((true, true));
                }
                Some(0x3c) => {
                    self.i += 1;
                    match self.peek() {
                        Some(0x3d) => {
                            self.i += 1;
                            look = Some((false, false));
                        }
                        Some(0x21) => {
                            self.i += 1;
                            look = Some((false, true));
                        }
                        _ => {
                            // A named group: its name, then the group.
                            while let Some(u) = self.peek() {
                                self.i += 1;
                                if u == 0x3e {
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => return Err("invalid group".into()),
            }
        }
        let inner = self.alternation()?;
        if !self.eat(0x29) {
            return Err("unterminated group".into());
        }
        Ok(match look {
            Some((ahead, negate)) => Node::Look { ahead, negate, node: Box::new(inner) },
            None => inner,
        })
    }

    // An escape outside a class: a set (\d, \w, \s and their capitals) or
    // a code unit.
    fn escape(&mut self) -> Result<Node, String> {
        match self.class_escape(false)? {
            Esc::Unit(u) => Ok(Node::Unit(u)),
            Esc::Set(ranges, negate) => Ok(Node::Set(Set { ranges, negate })),
        }
    }

    fn class_escape(&mut self, in_class: bool) -> Result<Esc, String> {
        self.i += 1;
        let Some(c) = self.peek() else { return Ok(Esc::Unit(0x5c)) };
        self.i += 1;
        let unit = |u: u16| Ok(Esc::Unit(u));
        match c {
            0x64 => Ok(Esc::Set(DIGIT.to_vec(), false)),
            0x44 => Ok(Esc::Set(DIGIT.to_vec(), true)),
            0x77 => Ok(Esc::Set(WORD.to_vec(), false)),
            0x57 => Ok(Esc::Set(WORD.to_vec(), true)),
            0x73 => Ok(Esc::Set(SPACE.to_vec(), false)),
            0x53 => Ok(Esc::Set(SPACE.to_vec(), true)),
            0x74 => unit(0x09),
            0x6e => unit(0x0a),
            0x76 => unit(0x0b),
            0x66 => unit(0x0c),
            0x72 => unit(0x0d),
            0x62 if in_class => unit(0x08),
            0x63 => match self.peek() {
                Some(l) if (0x41..=0x5a).contains(&l) || (0x61..=0x7a).contains(&l) => {
                    self.i += 1;
                    unit(l % 32)
                }
                // \c without a letter is a backslash and a c.
                _ => {
                    self.i -= 1;
                    unit(0x5c)
                }
            },
            0x78 => match self.s.get(self.i..self.i + 2).and_then(hex_value) {
                Some(v) => {
                    self.i += 2;
                    unit(v as u16)
                }
                None => unit(0x78),
            },
            0x75 => match self.s.get(self.i..self.i + 4).and_then(hex_value) {
                Some(v) => {
                    self.i += 4;
                    unit(v as u16)
                }
                None => unit(0x75),
            },
            0x30..=0x39 => {
                if c != 0x30 && !in_class && self.groups > 0 {
                    return Err("backreferences aren't supported".into());
                }
                if c >= 0x38 {
                    return unit(c);
                }
                // A legacy octal escape: up to three digits, at most \377.
                let mut v = u32::from(c - 0x30);
                for _ in 0..2 {
                    match self.peek() {
                        Some(d) if (0x30..=0x37).contains(&d) && v * 8 + u32::from(d - 0x30) <= 0o377 => {
                            v = v * 8 + u32::from(d - 0x30);
                            self.i += 1;
                        }
                        _ => break,
                    }
                }
                unit(v as u16)
            }
            0x6b if self.groups > 0 => Err("backreferences aren't supported".into()),
            other => unit(other),
        }
    }

    fn class(&mut self) -> Result<Node, String> {
        self.i += 1;
        let negate = self.eat(0x5e);
        let mut ranges: Vec<(u16, u16)> = Vec::new();
        loop {
            let Some(u) = self.peek() else { return Err("unterminated character class".into()) };
            if u == 0x5d {
                self.i += 1;
                break;
            }
            let from = self.class_atom()?;
            // A range, unless either end is a set (Annex B reads the - as itself).
            if self.peek() == Some(0x2d) && self.s.get(self.i + 1).is_some_and(|&n| n != 0x5d) {
                let save = self.i;
                self.i += 1;
                let to = self.class_atom()?;
                match (&from, &to) {
                    (Esc::Unit(a), Esc::Unit(b)) => {
                        if a > b {
                            return Err("range out of order in character class".into());
                        }
                        ranges.push((*a, *b));
                        continue;
                    }
                    _ => {
                        self.i = save;
                    }
                }
            }
            match from {
                Esc::Unit(a) => ranges.push((a, a)),
                Esc::Set(r, false) => ranges.extend(r),
                Esc::Set(r, true) => ranges.extend(complement(&r)),
            }
        }
        Ok(Node::Set(Set { ranges, negate }))
    }

    fn class_atom(&mut self) -> Result<Esc, String> {
        let u = self.peek().unwrap_or(0);
        if u == 0x5c {
            return self.class_escape(true);
        }
        self.i += 1;
        Ok(Esc::Unit(u))
    }
}

enum Esc {
    Unit(u16),
    Set(Vec<(u16, u16)>, bool),
}

// Matches `node` at `i`, calling `k` with where each way of matching it
// ends, until `k` says yes. Backtracking is `k` saying no.
fn run(node: &Node, s: &[u16], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    match node {
        Node::Empty => k(i),
        Node::Unit(u) => i < s.len() && s[i] == *u && k(i + 1),
        Node::Set(set) => i < s.len() && set.has(s[i]) && k(i + 1),
        Node::Start => i == 0 && k(i),
        Node::End => i == s.len() && k(i),
        Node::WordBoundary(yes) => {
            let before = i > 0 && is_word(s[i - 1]);
            let after = i < s.len() && is_word(s[i]);
            ((before != after) == *yes) && k(i)
        }
        Node::Look { ahead: true, negate, node } => {
            let found = run(node, s, i, &mut |_| true);
            found != *negate && k(i)
        }
        Node::Look { ahead: false, negate, node } => {
            let found = (0..=i).rev().any(|j| run(node, s, j, &mut |end| end == i));
            found != *negate && k(i)
        }
        Node::Concat(items) => seq(items, s, i, k),
        Node::Alt(options) => options.iter().any(|o| run(o, s, i, k)),
        Node::Repeat { node, min, max, greedy } => match node.as_ref() {
            Node::Unit(_) | Node::Set(_) => repeat_one(node, *min, *max, *greedy, s, i, k),
            _ => repeat(node, *min, *max, *greedy, 0, s, i, k),
        },
    }
}

fn seq(items: &[Node], s: &[u16], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    match items.split_first() {
        None => k(i),
        Some((first, rest)) => run(first, s, i, &mut |j| seq(rest, s, j, k)),
    }
}

// A repeat of one code unit, without recursion: as many as match, then
// fewer (or the other way round when lazy).
fn repeat_one(node: &Node, min: u32, max: Option<u32>, greedy: bool, s: &[u16], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    let one = |u: u16| match node {
        Node::Unit(x) => *x == u,
        Node::Set(set) => set.has(u),
        _ => false,
    };
    let limit = max.map_or(usize::MAX, |m| m as usize);
    let mut n = 0;
    while n < limit && i + n < s.len() && one(s[i + n]) {
        n += 1;
    }
    let min = min as usize;
    if n < min {
        return false;
    }
    if greedy {
        (min..=n).rev().any(|c| k(i + c))
    } else {
        (min..=n).any(|c| k(i + c))
    }
}

#[allow(clippy::too_many_arguments)]
fn repeat(node: &Node, min: u32, max: Option<u32>, greedy: bool, done: u32, s: &[u16], i: usize, k: &mut dyn FnMut(usize) -> bool) -> bool {
    if max.is_some_and(|m| done >= m) {
        return k(i);
    }
    // Past the minimum, an iteration that matches nothing ends the loop.
    let more = |k: &mut dyn FnMut(usize) -> bool| run(node, s, i, &mut |j| (done < min || j != i) && repeat(node, min, max, greedy, done + 1, s, j, k));
    if done < min {
        return more(k);
    }
    if greedy {
        more(k) || k(i)
    } else {
        k(i) || more(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(p: &str, s: &str) -> bool {
        compile(p).unwrap_or_else(|e| panic!("{p}: {e}")).test(s)
    }

    #[test]
    fn matches_as_js_does() {
        assert!(t("^#[0-9a-fA-F]{6}$", "#00ffAA"));
        assert!(!t("^#[0-9a-fA-F]{6}$", "#00ffAA0"));
        assert!(t("^(?:@[a-z][a-z0-9-]{1,31}:)?project:", "@lab:project:/x"));
        assert!(!t("^(?:@[a-z][a-z0-9-]{1,31}:)?project:", "x project:"));
        assert!(t(r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"#, "~/.ssh/id"));
        assert!(!t(r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"#, "~/a\"b"));
        assert!(!t(r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"#, "/a\nb"));
        assert!(t(r"^\d{1,4}[mhd]$", "15m"));
        assert!(!t(r"^\d{1,4}[mhd]$", "15s"));
        assert!(t(r"^[A-Za-z0-9_-]{1,900}\.[A-Za-z0-9_-]{43}$", &format!("abc.{}", "x".repeat(43))));
        assert!(t("^(claude|codex):.", "codex:1"));
        assert!(!t("^(claude|codex):.", "codex:\n"));
        assert!(t("a{", "a{"), "a {{ that is not a quantifier is itself");
        assert!(t(r"\bfoo\b", "a foo."));
        assert!(!t(r"\bfoo\b", "afoo"));
        assert!(t(r"^\s$", "\u{a0}"));
        assert!(!t(r"^\w$", "é"), "\\w is ASCII's");
        assert!(t(r"^.$", "\u{e9}"));
        assert!(!t(r"^.$", "😀"), ". is one code unit without the u flag");
        assert!(t(r"^..$", "😀"));
        assert!(t(r"^(?=a)\w+$", "abc"));
        assert!(!t(r"^(?!a)\w+$", "abc"));
        assert!(t(r"(?<=@)x", "@x"));
        assert!(!t(r"(?<!@)x", "@x"));
        assert!(t(r"^(?:ab)*$", "ababab"));
        assert!(t(r"^(?:a|ab)(?:c|bcd)(?:d*)$", "abcd"));
        assert!(t(r"^[\d-x]$", "-"), "a range with a set at one end is three atoms");
        assert!(t(r"^\x41\u0042\103$", "ABC"));
        assert!(t(r"^[^]$", "\n"));
        assert!(!t(r"^[]$", ""));
        assert!(t(r"^a+?$", "aaa"));
        assert!(t(r"^(?:\d\d[2468][048]|\d\d0[48])-02-29$", "2024-02-29"));
    }

    #[test]
    fn refuses_what_it_cant_run() {
        assert!(compile("(a)\\1").is_err());
        assert!(compile("*a").is_err());
        assert!(compile("(a").is_err());
        assert!(compile("a)").is_err());
        assert!(compile("[b-a]").is_err());
    }
}
