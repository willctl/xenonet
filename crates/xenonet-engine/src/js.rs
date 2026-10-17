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
