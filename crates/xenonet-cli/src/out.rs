//! Output for the xenonet command (cli/lib/out.js): color when a person is
//! reading (a terminal, without NO_COLOR), plain text otherwise, and JSON
//! for scripts (--json).

use serde_json::Value;
use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use xenonet_base::client::Refused;
use xenonet_base::json::{self, number_string};
use xenonet_base::units::js_round;

pub use xenonet_base::units::plural as plural_with;

/// A failure the command explains itself: printed without a stack trace,
/// and with its own exit code. `expected` is a JS Fail's: one that says how
/// the command was used wrong. Anything else (a plain Error in the JS) is
/// what XenoNet didn't expect, which goes in its error log too, with exit
/// code 1. `status` is an HTTP answer's, for the commands that look at it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fail {
    pub message: String,
    pub code: i32,
    pub expected: bool,
    pub status: Option<u16>,
}

impl Fail {
    /// `new Fail(message, code)`.
    pub fn new(message: impl Into<String>, code: i32) -> Self {
        Fail { message: message.into(), code, expected: true, status: None }
    }

    /// `new Error(message)`.
    pub fn error(message: impl Into<String>) -> Self {
        Fail { message: message.into(), code: 1, expected: false, status: None }
    }
}

impl From<Refused> for Fail {
    fn from(r: Refused) -> Self {
        Fail { message: r.message, code: 1, expected: false, status: r.status }
    }
}

/// What a command comes to: its exit code, or why it failed.
pub type Done = Result<i32, Fail>;

static COLOR: AtomicBool = AtomicBool::new(false);
static WROTE_BADLY: AtomicBool = AtomicBool::new(false);

fn tty() -> bool {
    static TTY: OnceLock<bool> = OnceLock::new();
    *TTY.get_or_init(|| std::io::stdout().is_terminal() && std::env::var("NO_COLOR").map_or(true, |v| v.is_empty()) && std::env::var("TERM").map_or(true, |t| t != "dumb"))
}

/// Color on, as far as the terminal allows (setColor).
pub fn set_color(on: bool) {
    COLOR.store(on && tty(), Ordering::Relaxed);
}

pub fn color() -> bool {
    COLOR.load(Ordering::Relaxed)
}

/// Whether writing to stdout went wrong some other way than its reader
/// going away: the command then exits 1.
pub fn wrote_badly() -> bool {
    WROTE_BADLY.load(Ordering::Relaxed)
}

/// Text in an SGR color, when color's on.
pub fn paint(code: &str, s: impl std::fmt::Display) -> String {
    if color() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// The colors out.js's `c` has.
pub mod c {
    use super::paint;
    use std::fmt::Display;

    pub fn bold(s: impl Display) -> String {
        paint("1", s)
    }
    pub fn dim(s: impl Display) -> String {
        paint("2", s)
    }
    pub fn red(s: impl Display) -> String {
        paint("31", s)
    }
    pub fn green(s: impl Display) -> String {
        paint("32", s)
    }
    pub fn yellow(s: impl Display) -> String {
        paint("33", s)
    }
    pub fn blue(s: impl Display) -> String {
        paint("34", s)
    }
    pub fn magenta(s: impl Display) -> String {
        paint("35", s)
    }
    pub fn cyan(s: impl Display) -> String {
        paint("36", s)
    }
    pub fn gray(s: impl Display) -> String {
        paint("90", s)
    }

    /// `c[name](s)`, for a color named by data.
    pub fn named(name: &str, s: impl Display) -> String {
        match name {
            "bold" => bold(s),
            "dim" => dim(s),
            "red" => red(s),
            "green" => green(s),
            "yellow" => yellow(s),
            "blue" => blue(s),
            "magenta" => magenta(s),
            "cyan" => cyan(s),
            "gray" => gray(s),
            _ => s.to_string(),
        }
    }
}

/// The terminal's columns, as process.stdout.columns has them: only when
/// stdout is a terminal.
fn columns() -> Option<usize> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    #[cfg(unix)]
    {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: TIOCGWINSZ fills the winsize given, on the terminal stdout is.
        let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0;
        (ok && ws.ws_col > 0).then_some(usize::from(ws.ws_col))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

pub fn width() -> usize {
    columns().unwrap_or(100).clamp(60, 160)
}

/// Text without its colors: characters as they show on screen.
pub fn visible(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\x1b[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let n = after.bytes().take_while(|b| b.is_ascii_digit() || *b == b';').count();
        if after.as_bytes().get(n) == Some(&b'm') {
            rest = &after[n + 1..];
        } else {
            out.push_str("\x1b[");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn len(s: &str) -> usize {
    s.chars().count()
}

/// Padded with spaces to `n` characters on screen.
pub fn pad(s: &str, n: usize) -> String {
    format!("{s}{}", " ".repeat(n.saturating_sub(len(&visible(s)))))
}

/// Cut to `n` characters, the last an ellipsis when it's cut.
pub fn clip(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let mut out: String = chars[..n.saturating_sub(1)].iter().collect();
        out.push('…');
        out
    } else {
        s.to_string()
    }
}

/// How a table lays out: the gap between columns, how far it's shifted
/// right, and whether the last column wraps under itself.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub gap: usize,
    pub indent: usize,
    pub wrap: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout { gap: 2, indent: 0, wrap: false }
    }
}

impl Layout {
    pub fn indent(indent: usize) -> Self {
        Layout { indent, ..Default::default() }
    }
    pub fn wrapped(indent: usize) -> Self {
        Layout { indent, wrap: true, ..Default::default() }
    }
}

/// Rows of cells as aligned columns. The last column takes what's left of
/// the terminal: cut to fit, or with `wrap`, wrapped onto more lines under
/// itself. A row shorter than the others pads only the cells it has.
pub fn table(rows: &[Vec<String>], layout: Layout) -> String {
    table_at(rows, layout, width())
}

pub fn table_at(rows: &[Vec<String>], Layout { gap, indent, wrap }: Layout, width: usize) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..cols.saturating_sub(1)).map(|i| rows.iter().map(|r| r.get(i).map_or(0, |c| len(&visible(c)))).max().unwrap_or(0)).collect();
    let used = indent + widths.iter().map(|w| w + gap).sum::<usize>();
    let room = width.saturating_sub(used).max(16);
    let lead = " ".repeat(indent);
    rows.iter()
        .map(|r| {
            let head: String = r.iter().take(cols.saturating_sub(1)).enumerate().map(|(i, cell)| pad(cell, widths[i] + gap)).collect();
            let last = r.get(cols.wrapping_sub(1)).map_or("", String::as_str);
            if !wrap {
                return format!("{lead}{head}{}", if visible(last) == last { clip(last, room) } else { last.to_string() });
            }
            let lines = wrap_text(last, room);
            let mut out = vec![format!("{lead}{head}{}", lines[0])];
            out.extend(lines[1..].iter().map(|l| format!("{lead}{}{l}", " ".repeat(used - indent))));
            out.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// /^(\x1b\[[0-9;]*m)([^\x1b]*)\x1b\[0m$/: a cell painted in one color.
fn one_color(s: &str) -> Option<(&str, &str)> {
    let after = s.strip_prefix("\x1b[")?;
    let n = after.bytes().take_while(|b| b.is_ascii_digit() || *b == b';').count();
    if after.as_bytes().get(n) != Some(&b'm') {
        return None;
    }
    let open = &s[..n + 3];
    let text = after[n + 1..].strip_suffix("\x1b[0m")?;
    (!text.contains('\x1b')).then_some((open, text))
}

/// Text broken into lines of at most `n` characters, at spaces where it
/// can. A cell painted in one color keeps it on every line; one with other
/// colors in it stays as it is.
pub fn wrap_text(s: &str, n: usize) -> Vec<String> {
    let found = one_color(s);
    if found.is_none() && s.contains('\x1b') {
        return vec![s.to_string()];
    }
    let (open, text) = found.unwrap_or(("", s));
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            if line.is_empty() {
                line = word.to_string();
            } else if len(&line) + 1 + len(word) <= n {
                line.push(' ');
                line.push_str(word);
            } else {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            }
            while len(&line) > n {
                let chars: Vec<char> = line.chars().collect();
                lines.push(chars[..n].iter().collect());
                line = chars[n..].iter().collect();
            }
        }
        lines.push(line);
    }
    if open.is_empty() {
        lines
    } else {
        lines.into_iter().map(|l| format!("{open}{l}\x1b[0m")).collect()
    }
}

/// Text written to stdout as it is. Output piped into something that
/// stopped reading (xenonet agents | head) ends the command quietly.
pub fn write(s: &str) {
    write_bytes(s.as_bytes());
}

pub fn write_bytes(b: &[u8]) {
    let mut out = std::io::stdout().lock();
    if let Err(e) = out.write_all(b).and_then(|()| out.flush()) {
        if e.kind() == std::io::ErrorKind::BrokenPipe {
            std::process::exit(0);
        }
        WROTE_BADLY.store(true, Ordering::Relaxed);
    }
}

/// A line.
pub fn print(s: impl AsRef<str>) {
    write(&format!("{}\n", s.as_ref()));
}

/// A value as JSON, two spaces deep.
pub fn json(v: &Value) {
    write(&format!("{}\n", json::stringify(v, 2)));
}

/// A field of an answer as JSON, or "undefined" when it isn't there, as
/// JSON.stringify leaves it.
pub fn json_at(v: Option<&Value>) {
    match v {
        Some(v) => json(v),
        None => print("undefined"),
    }
}

/// A line on stderr.
pub fn eprint(s: impl AsRef<str>) {
    let _ = std::io::stderr().lock().write_all(format!("{}\n", s.as_ref()).as_bytes());
}

/// "1 session", "1,234 sessions".
pub fn plural(n: f64, word: &str) -> String {
    plural_with(n, word, None)
}

/// When something was, as a person says it: just now, 5 min ago, 2 h ago,
/// 3 d ago. A time that isn't one is NaN days ago, as in JS.
pub fn ago(ts: f64, now: f64) -> String {
    let s = js_round((now - ts) / 1000.0);
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{} min ago", number_string(js_round(s / 60.0)))
    } else if s < 86400.0 {
        format!("{} h ago", number_string(js_round(s / 3600.0)))
    } else {
        format!("{} d ago", number_string(js_round(s / 86400.0)))
    }
}

/// Date.now().
pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(r: &[&[&str]]) -> Vec<Vec<String>> {
        r.iter().map(|row| row.iter().map(|s| s.to_string()).collect()).collect()
    }

    // The same cases as test/cli.test.js's.
    #[test]
    fn columns_line_up_and_the_last_one_wraps_under_itself() {
        let out = table_at(&rows(&[&["✓", "short", "one"], &["!", "a longer title", "two words"]]), Layout::indent(2), 100);
        assert_eq!(out, "  ✓  short           one\n  !  a longer title  two words");
        assert_eq!(wrap_text("the quick brown fox jumps", 10), ["the quick", "brown fox", "jumps"]);
        assert_eq!(wrap_text("\x1b[2mdim words here\x1b[0m", 9), ["\x1b[2mdim words\x1b[0m", "\x1b[2mhere\x1b[0m"]);
        assert_eq!(wrap_text("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap_text("a \x1b[1mb\x1b[0m c", 2), ["a \x1b[1mb\x1b[0m c"], "two colors stay as they are");
        assert_eq!(wrap_text("one\ntwo  three", 20), ["one", "two  three"]);
        assert_eq!(plural(1.0, "check"), "1 check");
        assert_eq!(plural(2400.0, "thread"), "2,400 threads");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("abc", 4), "abc");
        assert_eq!(clip("abc", 0), "…");
    }

    #[test]
    fn a_table_cuts_or_wraps_its_last_column_to_the_room_left() {
        let long = "word ".repeat(20);
        let cut = table_at(&rows(&[&["a", long.trim()]]), Layout::default(), 60);
        assert_eq!(cut.chars().count(), 3 + 57);
        assert!(cut.ends_with('…'));
        let wrapped = table_at(&rows(&[&["a", "one two three"]]), Layout { gap: 2, indent: 1, wrap: true }, 20);
        // The room is never less than 16.
        assert_eq!(wrapped, " a  one two three");
        let wrapped = table_at(&rows(&[&["a", &"x ".repeat(12)]]), Layout { gap: 2, indent: 1, wrap: true }, 20);
        assert_eq!(wrapped, " a  x x x x x x x x\n    x x x x ");
        // A short row pads the cells it has.
        assert_eq!(table_at(&rows(&[&["ab", "c", "d"], &["e"]]), Layout::default(), 100), "ab  c  d\ne   ");
        // Painted text counts as it shows, and isn't cut.
        assert_eq!(pad("\x1b[2mab\x1b[0m", 4), "\x1b[2mab\x1b[0m  ");
        assert_eq!(visible("\x1b[1;31mx\x1b[0m\x1b[y"), "x\x1b[y");
        assert_eq!(table_at(&[], Layout::default(), 100), "");
    }

    #[test]
    fn times_as_a_person_says_them() {
        let now = 1_800_000_000_000.0;
        assert_eq!(ago(now - 59_000.0, now), "just now");
        assert_eq!(ago(now - 120_000.0, now), "2 min ago");
        assert_eq!(ago(now - 3.0 * 3_600_000.0, now), "3 h ago");
        assert_eq!(ago(now - 2.0 * 86_400_000.0, now), "2 d ago");
        assert_eq!(ago(f64::NAN, now), "NaN d ago");
    }
}
