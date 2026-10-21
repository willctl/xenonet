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
        bits = bits << 6 | u32::from(v);
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
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
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
