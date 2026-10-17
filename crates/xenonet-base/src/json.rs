//! JSON as JS reads and writes it: `JSON.parse` and `JSON.stringify`, over
//! serde_json's `Value` (with its keys in order).
//!
//! serde_json alone reads some lines JS reads, and writes some numbers
//! differently, so XenoNet's readers and stores go through here:
//!
//! - A number is the double nearest to what's written, as in JS, however
//!   many digits it has. Whole numbers JS holds exactly (up to 2^53) are
//!   kept as integers; the rest as doubles. A number too big for a double
//!   reads as null here, where JS makes it Infinity.
//! - A lone surrogate escape (`"\ud83d"`, half an emoji, which JS writes
//!   when a string was cut in the middle of one) reads as U+FFFD. JS keeps
//!   the half character; a Rust string can't.
//! - A key written twice keeps its first place and its last value.
//! - Nesting deeper than 512 levels is refused, to keep the stack safe on
//!   any thread (JS has no such limit; transcripts nest a few dozen deep).
//! - Writing, a double is written as JS writes a number (`1e+21`, `5e-7`,
//!   `0.1`), and one that isn't finite as null.

use serde_json::{Map, Number, Value};

const MAX_DEPTH: usize = 512;

/// Why a text isn't JSON, and where (in bytes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub at: usize,
}

impl std::fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at position {}", self.message, self.at)
    }
}

impl std::error::Error for SyntaxError {}

/// A JS number as a JSON value: an integer when it's whole and JS holds it
/// exactly, else a double, and null when it isn't finite (as
/// JSON.stringify writes it).
pub fn number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_992.0 && !(n == 0.0 && n.is_sign_negative()) {
        Value::Number(Number::from(n as i64))
    } else {
        Number::from_f64(n).map_or(Value::Null, Value::Number)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    depth: usize,
}

/// Reads a text as `JSON.parse` does (see the module's note).
pub fn parse(text: &str) -> Result<Value, SyntaxError> {
    let mut p = Parser { s: text.as_bytes(), text, i: 0, depth: 0 };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("Unexpected non-whitespace character after JSON"));
    }
    Ok(v)
}

impl Parser<'_> {
    fn err(&self, message: &str) -> SyntaxError {
        SyntaxError { message: message.to_string(), at: self.i }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Result<Value, SyntaxError> {
        match self.s.get(self.i) {
            None => Err(self.err("Unexpected end of JSON input")),
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("Unexpected token")),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Value, SyntaxError>) -> Result<Value, SyntaxError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("Nested too deeply"));
        }
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn literal(&mut self, word: &str, v: Value) -> Result<Value, SyntaxError> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("Unexpected token"))
        }
    }

    fn object(&mut self) -> Result<Value, SyntaxError> {
        self.i += 1;
        let mut m = Map::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Object(m));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.err(if self.i == self.s.len() { "Unexpected end of JSON input" } else { "Expected property name" }));
            }
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return Err(self.err("Expected ':' after property name"));
            }
            self.i += 1;
            self.ws();
            let v = self.value()?;
            // A key written twice keeps its first place, as in JS.
            m.insert(k, v);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(m));
                }
                None => return Err(self.err("Unexpected end of JSON input")),
                Some(_) => return Err(self.err("Expected ',' or '}' after property value")),
            }
        }
    }

    fn array(&mut self) -> Result<Value, SyntaxError> {
        self.i += 1;
        let mut a = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::Array(a));
        }
        loop {
            self.ws();
            a.push(self.value()?);
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(a));
                }
                None => return Err(self.err("Unexpected end of JSON input")),
                Some(_) => return Err(self.err("Expected ',' or ']' after array element")),
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, SyntaxError> {
        let h = self.s.get(self.i..self.i + 4).filter(|h| h.iter().all(u8::is_ascii_hexdigit)).ok_or_else(|| self.err("Bad Unicode escape"))?;
        self.i += 4;
        Ok(u32::from_str_radix(std::str::from_utf8(h).unwrap(), 16).unwrap())
    }

    fn string(&mut self) -> Result<String, SyntaxError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            // The run of plain characters up to the next quote, escape or
            // control character, copied at once.
            let start = self.i;
            while let Some(&c) = self.s.get(self.i) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.text[start..self.i]);
            match self.s.get(self.i) {
                None => return Err(self.err("Unterminated string in JSON")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = *self.s.get(self.i).ok_or_else(|| self.err("Unterminated string in JSON"))?;
                    self.i += 1;
                    match c {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) && self.s[self.i..].starts_with(b"\\u") {
                                let back = self.i;
                                self.i += 2;
                                let lo = self.hex4()?;
                                if (0xDC00..0xE000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                } else {
                                    // Half a pair, then something else: the half alone.
                                    self.i = back;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(ch.unwrap_or('\u{FFFD}'));
                        }
                        _ => {
                            self.i -= 1;
                            return Err(self.err("Bad escaped character in JSON"));
                        }
                    }
                }
                Some(_) => return Err(self.err("Bad control character in string literal in JSON")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, SyntaxError> {
        let start = self.i;
        let digits = |p: &mut Self| {
            let from = p.i;
            while p.s.get(p.i).is_some_and(u8::is_ascii_digit) {
                p.i += 1;
            }
            p.i - from
        };
        if self.s.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.s.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return Err(self.err("No number after minus sign in JSON")),
        }
        let mut whole = true;
        if self.s.get(self.i) == Some(&b'.') {
            self.i += 1;
            whole = false;
            if digits(self) == 0 {
                return Err(self.err("Unterminated fractional number in JSON"));
            }
        }
        if let Some(b'e' | b'E') = self.s.get(self.i) {
            self.i += 1;
            whole = false;
            if let Some(b'+' | b'-') = self.s.get(self.i) {
                self.i += 1;
            }
            if digits(self) == 0 {
                return Err(self.err("Exponent part is missing a number in JSON"));
            }
        }
        let lit = &self.text[start..self.i];
        if whole && lit != "-0" {
            if let Ok(n) = lit.parse::<i64>() {
                if n.unsigned_abs() <= 1 << 53 {
                    return Ok(Value::Number(Number::from(n)));
                }
            }
        }
        // Rust's parse is correctly rounded, as V8's is.
        let f: f64 = lit.parse().map_err(|_| self.err("Bad number"))?;
        Ok(Number::from_f64(f).map_or(Value::Null, Value::Number))
    }
}

/// A number as JS writes it (Number.prototype.toString): the shortest
/// digits that read back as the same double, in fixed notation from 1e-7
/// up to 1e21 and in exponent notation outside that.
pub fn number_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n == 0.0 {
        return "0".into();
    }
    // Rust's {:e} gives the shortest round-trip digits: d.ddddde<exp>.
    let e = format!("{:e}", n.abs());
    let (mantissa, exp) = e.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let point = exp + 1; // where the decimal point goes, counted from the first digit
    let sign = if n < 0.0 { "-" } else { "" };
    let body = if k <= point && point <= 21 {
        format!("{digits}{}", "0".repeat((point - k) as usize))
    } else if 0 < point && point <= 21 {
        format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
    } else if -6 < point && point <= 0 {
        format!("0.{}{digits}", "0".repeat((-point) as usize))
    } else {
        let e = point - 1;
        let es = if e >= 0 { format!("e+{e}") } else { format!("e-{}", -e) };
        if k == 1 {
            format!("{digits}{es}")
        } else {
            format!("{}.{}{es}", &digits[..1], &digits[1..])
        }
    };
    format!("{sign}{body}")
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, v: &Value, indent: &str, at: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => out.push_str(&i.to_string()),
            (None, Some(f)) if f.is_finite() => out.push_str(&number_string(f)),
            _ => out.push_str(&n.to_string()),
        },
        Value::String(s) => write_string(out, s),
        Value::Array(a) => {
            if a.is_empty() {
                return out.push_str("[]");
            }
            out.push('[');
            let outer = at.clone();
            at.push_str(indent);
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !indent.is_empty() {
                    out.push('\n');
                    out.push_str(at);
                }
                write_value(out, x, indent, at);
            }
            *at = outer;
            if !indent.is_empty() {
                out.push('\n');
                out.push_str(at);
            }
            out.push(']');
        }
        Value::Object(m) => {
            if m.is_empty() {
                return out.push_str("{}");
            }
            out.push('{');
            let outer = at.clone();
            at.push_str(indent);
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !indent.is_empty() {
                    out.push('\n');
                    out.push_str(at);
                }
                write_string(out, k);
                out.push(':');
                if !indent.is_empty() {
                    out.push(' ');
                }
                write_value(out, x, indent, at);
            }
            *at = outer;
            if !indent.is_empty() {
                out.push('\n');
                out.push_str(at);
            }
            out.push('}');
        }
    }
}

/// `JSON.stringify(value, null, indent)`, indent being 0 for none.
pub fn stringify(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_value(&mut out, v, &" ".repeat(indent.min(10)), &mut String::new());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_as_json_parse_does() {
        let v = parse(r#" {"a":1,"b":[true,false,null],"a":3,"s":"xé😀\n","n":-0.5,"big":12345678901234567890} "#).unwrap();
        assert_eq!(stringify(&v, 0), r#"{"a":3,"b":[true,false,null],"s":"xé😀\n","n":-0.5,"big":12345678901234567000}"#);
        assert_eq!(parse(r#""\ud83d""#).unwrap(), Value::String("\u{FFFD}".into()));
        assert_eq!(parse(r#""\ud83dx""#).unwrap(), Value::String("\u{FFFD}x".into()));
        assert_eq!(parse("1e400").unwrap(), Value::Null);
        assert_eq!(parse("9007199254740993").unwrap().as_f64(), Some(9007199254740992.0));
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "01", "1.", "-", "\"\t\"", "nul", "{} x", "\u{feff}{}", "'a'", "[1] [2]"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(513) + &"]".repeat(513);
        assert!(parse(&("[".repeat(512) + &"]".repeat(512))).is_ok());
        assert!(parse(&deep).is_err());
    }

    #[test]
    fn writes_numbers_as_js_does() {
        for (n, s) in [
            (1.0, "1"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (1.5e-7, "1.5e-7"),
            (1e-6, "0.000001"),
            (-2.5, "-2.5"),
            (123.456, "123.456"),
            (5e-324, "5e-324"),
            (1.7976931348623157e308, "1.7976931348623157e+308"),
            (0.000001234, "0.000001234"),
        ] {
            assert_eq!(number_string(n), s, "{n}");
        }
        assert_eq!(stringify(&parse(r#"{"a":[1,{"b":2}],"c":{}}"#).unwrap(), 2), "{\n  \"a\": [\n    1,\n    {\n      \"b\": 2\n    }\n  ],\n  \"c\": {}\n}");
    }
}
