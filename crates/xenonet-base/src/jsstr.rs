//! Strings as JS indexes, cuts, compares and dates them, for the readers
//! and the map, whose output the tests compare to the JS's character for
//! character.
//!
//! - JS counts a string in UTF-16 units: `length`, `slice` and
//!   `padStart` here do too. A cut through a pair of units (half an emoji)
//!   leaves U+FFFD where JS keeps the half, which prints as U+FFFD.
//! - `localeCompare` is ICU's root collation in JS. Here it's that order
//!   for ASCII, Latin-1 and the Latin letters with accents (the base letter
//!   first, then accents, then lowercase before uppercase); any other
//!   character sorts after them, by code point.
//! - A day is the local one, as `toLocaleDateString("en-CA")` gives it:
//!   the system's zone (TZ included) on unix, read through libc. Elsewhere
//!   it's UTC's for now.

use crate::time;
use std::cmp::Ordering;

/// `s.length`.
pub fn len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// A JS index (`slice`'s): negative counts from the end, and both are
/// held to the string.
fn index(i: i64, n: usize) -> usize {
    if i < 0 {
        n.saturating_sub(i.unsigned_abs() as usize)
    } else {
        (i as usize).min(n)
    }
}

/// The UTF-16 units from `from` to `to`, as a string: a unit of a pair
/// cut off from its other half is U+FFFD.
fn units(s: &str, from: usize, to: usize) -> String {
    if from >= to {
        return String::new();
    }
    let mut out = String::new();
    let mut at = 0;
    for c in s.chars() {
        let w = c.len_utf16();
        let end = at + w;
        if end <= from {
            at = end;
            continue;
        }
        if at >= to {
            break;
        }
        if at >= from && end <= to {
            out.push(c);
        } else {
            out.push('\u{FFFD}');
        }
        at = end;
    }
    out
}

/// `s.slice(start, end)`.
pub fn slice(s: &str, start: i64, end: Option<i64>) -> String {
    if s.is_ascii() {
        let n = s.len();
        let (a, b) = (index(start, n), end.map_or(n, |e| index(e, n)));
        return if a < b { s[a..b].to_string() } else { String::new() };
    }
    let n = len(s);
    units(s, index(start, n), end.map_or(n, |e| index(e, n)))
}

/// `s.slice(0, n)`: at most the first n units.
pub fn head(s: &str, n: usize) -> String {
    if s.is_ascii() {
        return s[..n.min(s.len())].to_string();
    }
    units(s, 0, n)
}

/// `s.slice(-n)`: at most the last n units (all of it for 0, as JS's
/// `slice(-0)` is).
pub fn last(s: &str, n: usize) -> String {
    if n == 0 {
        return s.to_string();
    }
    slice(s, -(n as i64), None)
}

/// `s.substring(start, end)`: negative is 0, and the two swap when start
/// is after end.
pub fn substring(s: &str, start: i64, end: Option<i64>) -> String {
    let n = len(s);
    let clamp = |i: i64| i.clamp(0, n as i64) as usize;
    let (a, b) = (clamp(start), end.map_or(n, clamp));
    units(s, a.min(b), a.max(b))
}

/// `s.padStart(n, fill)`.
pub fn pad_start(s: &str, n: usize, fill: &str) -> String {
    let have = len(s);
    if have >= n || fill.is_empty() {
        return s.to_string();
    }
    let want = n - have;
    let mut pad = String::new();
    while len(&pad) < want {
        pad.push_str(fill);
    }
    format!("{}{s}", head(&pad, want))
}

/// The default order of JS's `sort()` for strings: by UTF-16 units.
pub fn cmp_units(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `s.replace(/\s+/g, " ")`: each run of JS white space (and line breaks)
/// as one space.
pub fn collapse_space(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if crate::text::is_js_space(c) {
            space = true;
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    if space {
        out.push(' ');
    }
    out
}

// ---------- object keys ----------

/// Whether a key is an array index, which a JS object lists before its
/// other keys, in numeric order: "0", "7", "4294967294", not "07" or "-1".
pub fn is_array_index(key: &str) -> bool {
    !key.is_empty() && (key == "0" || !key.starts_with('0')) && key.len() <= 10 && key.bytes().all(|b| b.is_ascii_digit()) && key.parse::<u64>().is_ok_and(|n| n < u32::MAX as u64)
}

/// An object's keys in the order a JS object made from them in this order
/// would list them: array indexes first, by number, then the rest as
/// given.
pub fn js_key_order(map: serde_json::Map<String, serde_json::Value>) -> serde_json::Map<String, serde_json::Value> {
    if !map.keys().any(|k| is_array_index(k)) {
        return map;
    }
    let (mut idx, rest): (Vec<_>, Vec<_>) = map.into_iter().partition(|(k, _)| is_array_index(k));
    idx.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(0));
    idx.into_iter().chain(rest).collect()
}

// ---------- localeCompare ----------

// ASCII's punctuation and symbols in ICU's root order, before the digits
// and the letters. Control characters other than these are ignored, as
// ICU ignores them.
const PUNCT: &str = "\t\n\u{b}\u{c}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// A character's collation elements in the root order, as ICU's (DUCET's)
/// are made: (primary, secondary, tertiary), with 0 for a level an element
/// has no weight at. A letter is its base letter's primary, and an accent
/// an element of its own that weighs only from the second level on, so an
/// accent counts only between strings with the same letters. Lowercase
/// comes before uppercase at the third level. Nothing for a character the
/// order ignores.
fn weights(c: char, out: &mut Vec<(u32, u32, u32)>) {
    const COMMON: u32 = 1;
    const LOWER: u32 = 2;
    const UPPER: u32 = 8;
    if let Some(i) = PUNCT.find(c) {
        return out.push((1 + i as u32, COMMON, LOWER));
    }
    // Other punctuation from Latin-1 and General Punctuation that's common
    // in text: placed beside the ASCII it reads like.
    let near = |ascii: char, extra: u32| (1 + PUNCT.find(ascii).unwrap() as u32, COMMON, LOWER + extra);
    match c {
        '\u{a0}' => return out.push(near(' ', 1)),
        '\u{2010}'..='\u{2015}' => return out.push(near('-', 1 + (c as u32 - 0x2010))),
        '\u{2026}' => return out.push(near('.', 1)),
        '\u{2018}' | '\u{2019}' => return out.push(near('\'', 1)),
        '\u{201c}' | '\u{201d}' => return out.push(near('"', 1)),
        _ => {}
    }
    if c.is_ascii_digit() {
        return out.push((100 + (c as u32 - '0' as u32), COMMON, LOWER));
    }
    let primary = |base: char| 200 + 2 * (base.to_ascii_lowercase() as u32 - 'a' as u32);
    let case = |upper: bool| if upper { UPPER } else { LOWER };
    if c.is_ascii_alphabetic() {
        return out.push((primary(c), COMMON, case(c.is_ascii_uppercase())));
    }
    if let Some((base, accent)) = latin(c) {
        let upper = c.is_uppercase();
        return match base {
            // ß, æ and œ are two letters each (ss, ae, oe), with a mark
            // between them that weighs at the second level.
            "ss" | "ae" | "oe" => {
                let mut two = base.chars();
                out.push((primary(two.next().unwrap()), COMMON, case(upper) + 2));
                out.push((0, 100, 4));
                out.push((primary(two.next().unwrap()), COMMON, case(upper) + 2));
            }
            // þ comes after z.
            "th" => out.push((primary('z') + 2, COMMON, case(upper))),
            _ => {
                out.push((primary(base.chars().next().unwrap()), COMMON, case(upper)));
                out.push((0, 2 + accent, LOWER));
            }
        };
    }
    if c.is_control() {
        return;
    }
    out.push((0x10000 + c as u32, COMMON, LOWER));
}

/// Latin letters with accents: the base letter, and which accent (any
/// number other than 0 that keeps accented letters apart).
fn latin(c: char) -> Option<(&'static str, u32)> {
    const TABLE: &[(char, &str, u32)] = &[
        ('à', "a", 2), ('á', "a", 1), ('â', "a", 4), ('ã', "a", 6), ('ä', "a", 5), ('å', "a", 7), ('æ', "ae", 0),
        ('ç', "c", 8), ('è', "e", 2), ('é', "e", 1), ('ê', "e", 4), ('ë', "e", 5), ('ì', "i", 2), ('í', "i", 1),
        ('î', "i", 4), ('ï', "i", 5), ('ð', "d", 9), ('ñ', "n", 6), ('ò', "o", 2), ('ó', "o", 1), ('ô', "o", 4),
        ('õ', "o", 6), ('ö', "o", 5), ('ø', "o", 15), ('ù', "u", 2), ('ú', "u", 1), ('û', "u", 4), ('ü', "u", 5),
        ('ý', "y", 1), ('ÿ', "y", 5), ('þ', "th", 0), ('ß', "ss", 0), ('ā', "a", 10), ('ă', "a", 3), ('ą', "a", 8),
        ('ć', "c", 1), ('č', "c", 11), ('ď', "d", 11), ('ē', "e", 10), ('ė', "e", 12), ('ę', "e", 8), ('ě', "e", 11),
        ('ğ', "g", 3), ('ī', "i", 10), ('į', "i", 8), ('ı', "i", 13), ('ł', "l", 9), ('ľ', "l", 11), ('ń', "n", 1),
        ('ň', "n", 11), ('ō', "o", 10), ('ő', "o", 14), ('œ', "oe", 0), ('ř', "r", 11), ('ś', "s", 1), ('š', "s", 11),
        ('ş', "s", 8), ('ť', "t", 11), ('ū', "u", 10), ('ů', "u", 7), ('ű', "u", 14), ('ź', "z", 1), ('ż', "z", 12),
        ('ž', "z", 11),
    ];
    let lower = c.to_lowercase().next()?;
    TABLE.iter().find(|(l, _, _)| *l == lower).map(|(_, b, a)| (*b, *a))
}

fn collation(s: &str) -> Vec<(u32, u32, u32)> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        weights(c, &mut out);
    }
    out
}

/// `a.localeCompare(b)`, as an order (see the module's note).
pub fn locale_compare(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (x, y) = (collation(a), collation(b));
    // Each level compares the weights the elements have at it, leaving out
    // those that have none there.
    let level = |f: fn(&(u32, u32, u32)) -> u32| x.iter().map(f).filter(|&w| w != 0).cmp(y.iter().map(f).filter(|&w| w != 0));
    level(|w| w.0).then_with(|| level(|w| w.1)).then_with(|| level(|w| w.2))
}

/// `a.localeCompare(b)` as JS returns it: -1, 0 or 1.
pub fn locale_compare_num(a: &str, b: &str) -> f64 {
    match locale_compare(a, b) {
        Ordering::Less => -1.0,
        Ordering::Equal => 0.0,
        Ordering::Greater => 1.0,
    }
}

// ---------- local days ----------

#[cfg(unix)]
extern "C" {
    // In every unix C library, though the libc crate leaves it out on some.
    fn tzset();
}

/// How far local time is ahead of UTC at a time, in ms.
#[cfg(unix)]
pub fn local_offset_ms(ms: f64) -> f64 {
    if !ms.is_finite() {
        return 0.0;
    }
    let t = (ms / 1000.0).floor() as libc::time_t;
    // SAFETY: tzset reads TZ, which JS may have changed (process.env.TZ
    // sets the C environment too); localtime_r writes only `tm`.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        tzset();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0.0;
        }
        tm.tm_gmtoff as f64 * 1000.0
    }
}

#[cfg(not(unix))]
pub fn local_offset_ms(_ms: f64) -> f64 {
    0.0
}

/// The local day a time falls on, as `new Date(ms).toLocaleDateString("en-CA")`
/// writes it (YYYY-MM-DD), and "Invalid Date" for a time JS can't hold.
pub fn local_day(ms: f64) -> String {
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return "Invalid Date".into();
    }
    let local = ms + local_offset_ms(ms);
    let (y, m, d) = time::civil_from_days((local / 86_400_000.0).floor() as i64);
    if (0..=9999).contains(&y) {
        format!("{y:04}-{m:02}-{d:02}")
    } else {
        // Outside four digits, en-CA writes the year as it is.
        format!("{y}-{m:02}-{d:02}")
    }
}

/// When the local day a time falls on starts and ends, as
/// `new Date(y, m, d).getTime()` and the next day's give them.
pub fn local_day_bounds(ms: f64) -> (f64, f64) {
    let local = ms + local_offset_ms(ms);
    let day = (local / 86_400_000.0).floor();
    let start = |d: f64| {
        let guess = d * 86_400_000.0;
        let first = guess - local_offset_ms(guess);
        guess - local_offset_ms(first)
    };
    (start(day), start(day + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_count_utf16_units() {
        assert_eq!(len("a😀b"), 4);
        assert_eq!(slice("a😀b", 0, Some(2)), "a\u{FFFD}");
        assert_eq!(slice("a😀b", 1, Some(3)), "😀");
        assert_eq!(slice("hello", -3, None), "llo");
        assert_eq!(slice("hello", 2, Some(-1)), "ll");
        assert_eq!(slice("hello", 4, Some(2)), "");
        assert_eq!(head("héllo", 2), "hé");
        assert_eq!(last("héllo", 0), "héllo");
        assert_eq!(substring("hello", 4, Some(1)), "ell");
        assert_eq!(pad_start("7", 3, "0"), "007");
        assert_eq!(pad_start("abc", 2, "0"), "abc");
        assert_eq!(collapse_space("  a \n\t b  "), " a b ");
    }

    #[test]
    fn array_indexes_go_first() {
        let m: serde_json::Map<String, serde_json::Value> = serde_json::from_str(r#"{"b":1,"10":2,"2":3,"02":4,"-1":5}"#).unwrap();
        assert_eq!(js_key_order(m).keys().collect::<Vec<_>>(), ["2", "10", "b", "02", "-1"]);
        assert!(is_array_index("4294967294") && !is_array_index("4294967295"));
    }

    // Pairs and what Node's localeCompare said of them (Node 24, ICU 77).
    const PAIRS: &[(&str, &str, i8)] = &[
        ("a", "A", -1),
        ("A", "b", -1),
        ("a-b", "ab", -1),
        ("co-op", "coop", -1),
        ("", "a", -1),
        ("a b", "a_b", -1),
        ("a_b", "a-b", -1),
        ("a1", "a10", -1),
        ("a10", "a2", -1),
        ("aa", "aA", -1),
        ("aA", "Aa", -1),
        ("Aa", "ab", -1),
        ("á", "Á", -1),
        ("a", "á", -1),
        ("Á", "aa", -1),
        ("e", "é", -1),
        ("é", "f", -1),
        ("ss", "ß", -1),
        ("ß", "st", -1),
        ("æ", "b", -1),
        ("a", "æ", -1),
        ("$", "0", -1),
        ("~", "$", -1),
        ("9", "a", -1),
        ("z", "α", -1),
        ("Zebra", "apple", 1),
        ("claude-opus", "claude-sonnet", -1),
        ("gpt-example", "claude", 1),
        ("openai", "anthropic", 1),
        ("same", "same", 0),
    ];

    #[test]
    fn locale_compare_is_icus_root_order_for_latin_text() {
        for &(a, b, want) in PAIRS {
            assert_eq!(locale_compare_num(a, b) as i8, want, "{a:?} against {b:?}");
            assert_eq!(locale_compare_num(b, a) as i8, -want, "{b:?} against {a:?}");
        }
        let mut chars: Vec<String> = (0x20u8..0x7f).map(|c| (c as char).to_string()).collect();
        chars.sort_by(|a, b| locale_compare(a, b));
        assert_eq!(chars.concat(), " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ");
    }

    #[test]
    fn local_days_follow_the_zone() {
        assert_eq!(local_day(f64::NAN), "Invalid Date");
        let ms = time::parse_iso("2026-10-03T12:00:00Z").unwrap();
        let (from, to) = local_day_bounds(ms);
        assert!(from <= ms && ms < to);
        assert_eq!(local_day(from), local_day(ms));
        assert_eq!(local_day(to - 1.0), local_day(ms));
        assert_ne!(local_day(to), local_day(ms));
    }
}
