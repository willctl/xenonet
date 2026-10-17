//! core/hosts/frames.js: the relay's frames, one JSON object a line. Each
//! says what it is in `t`; neither side acts on a frame it doesn't know.

use serde_json::Value;

pub const PROTOCOL: i64 = 1;
/// From any host: bigger frames are dropped as they arrive, unread.
pub const MAX_FRAME: usize = 16 << 20;
/// From a console, which only ever asks for small things.
pub const MAX_REQUEST: usize = 64 << 10;
pub const FROM_RELAY: [&str; 7] = ["hello", "snapshot", "event", "res", "time", "ping", "bye"];
pub const FROM_CONSOLE: [&str; 2] = ["get", "time"];
/// How deep a frame may nest: JSON.parse takes any depth, but what reads
/// it after doesn't.
pub const MAX_DEPTH: usize = 64;

/// Splits a stream of bytes into lines, however its chunks fall. A line
/// over `max` bytes is dropped as it comes, without being held, and said
/// once; a carriage return before the newline goes too.
pub struct LineSplitter {
    max: usize,
    parts: Vec<u8>,
    dropping: bool,
}

/// What a chunk held: its whole lines, and how many were dropped.
#[derive(Debug, Default, PartialEq)]
pub struct Split {
    pub lines: Vec<String>,
    pub dropped: usize,
    /// The order lines and drops came in: true for a line, false for a drop.
    pub order: Vec<bool>,
}

impl LineSplitter {
    pub fn new(max: usize) -> Self {
        LineSplitter { max, parts: Vec::new(), dropping: false }
    }

    pub fn push(&mut self, buf: &[u8]) -> Split {
        let mut out = Split::default();
        let mut start = 0;
        loop {
            let nl = buf[start..].iter().position(|&b| b == b'\n').map(|i| start + i);
            let piece = &buf[start..nl.unwrap_or(buf.len())];
            if !self.dropping {
                if self.parts.len() + piece.len() > self.max {
                    self.dropping = true;
                    self.parts.clear();
                    out.dropped += 1;
                    out.order.push(false);
                } else {
                    self.parts.extend_from_slice(piece);
                }
            }
            let Some(nl) = nl else { return out };
            if self.dropping {
                self.dropping = false;
            } else if !self.parts.is_empty() {
                let mut text = String::from_utf8_lossy(&self.parts).into_owned();
                if text.ends_with('\r') {
                    text.pop();
                }
                self.parts.clear();
                if !text.is_empty() {
                    out.lines.push(text);
                    out.order.push(true);
                }
            }
            start = nl + 1;
        }
    }
}

/// A line as a frame, or why it isn't one: a JSON object with a string
/// `t`, of a kind in `known` when that's given.
pub fn parse_frame(line: &str, known: Option<&[&str]>) -> Result<Value, String> {
    // JSON.parse takes any depth; this parser stops past 512 levels, far
    // deeper than any frame (MAX_DEPTH), so such a line is one too deep.
    let frame = match xenonet_base::json::parse(line) {
        Ok(f) => f,
        Err(e) if e.message == "Nested too deeply" => return Err("a frame nested too deep".into()),
        Err(_) => return Err("not JSON".into()),
    };
    let Some(t) = frame.as_object().and_then(|o| o.get("t")).and_then(Value::as_str) else { return Err("not a frame".into()) };
    if let Some(k) = known {
        if !k.contains(&t) {
            return Err(format!("a kind of frame this XenoNet doesn't know ({})", xenonet_app::jstext::slice16(t, 32)));
        }
    }
    Ok(frame)
}

pub fn encode(frame: &Value) -> String {
    format!("{}\n", xenonet_base::json::stringify(frame, 0))
}

/// Whether a value nests no deeper than `max` arrays and objects, checked
/// without recursion.
pub fn shallow(value: &Value, max: usize) -> bool {
    let mut stack = vec![(value, 0usize)];
    while let Some((v, depth)) = stack.pop() {
        if depth > max {
            return false;
        }
        let children: Box<dyn Iterator<Item = &Value>> = match v {
            Value::Array(a) => Box::new(a.iter()),
            Value::Object(o) => Box::new(o.values()),
            _ => continue,
        };
        for x in children {
            if x.is_array() || x.is_object() {
                stack.push((x, depth + 1));
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_however_the_bytes_fall() {
        let mut s = LineSplitter::new(MAX_FRAME);
        let text = "{\"t\":\"ping\",\"now\":1}\r\n\n\nCafé ☕\n";
        let mut lines = Vec::new();
        for b in text.as_bytes() {
            lines.extend(s.push(&[*b]).lines);
        }
        assert_eq!(lines, ["{\"t\":\"ping\",\"now\":1}", "Café ☕"]);
        let mut small = LineSplitter::new(16);
        let got = small.push(b"0123456789abcdef\n0123456789abcdefX\nok\n");
        assert_eq!((got.lines, got.dropped), (vec!["0123456789abcdef".to_string(), "ok".to_string()], 1));
    }

    #[test]
    fn frames_and_depth() {
        assert!(parse_frame("[1,2]", None).is_err());
        assert!(parse_frame("{\"t\":\"exec\"}", Some(&FROM_RELAY)).unwrap_err().contains("doesn't know (exec)"));
        let deep = xenonet_base::json::parse(&format!("{}{}", "[".repeat(100), "]".repeat(100))).unwrap();
        assert!(!shallow(&deep, MAX_DEPTH));
    }
}
