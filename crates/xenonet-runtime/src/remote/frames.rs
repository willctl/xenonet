//! The relay's frames, each one JSON object, as a line. Each says what it
//! is in `t`; neither side acts on a frame it doesn't know.

use serde_json::Value;

pub const PROTOCOL: i64 = 1;
pub const FROM_RELAY: [&str; 7] = ["hello", "snapshot", "event", "res", "time", "ping", "bye"];
pub const FROM_CONSOLE: [&str; 2] = ["get", "time"];
/// How deep a frame may nest: JSON.parse takes any depth, but what reads
/// it after doesn't.
pub const MAX_DEPTH: usize = 64;

/// A line as a frame, or why it isn't one: a JSON object with a string
/// `t`, of a kind in `known` when that's given.
pub fn parse_frame(line: &str, known: Option<&[&str]>) -> Result<Value, String> {
    // JSON.parse takes any depth; this parser stops past 512 levels, far
    // deeper than any frame (MAX_DEPTH), so such a line is one too deep.
    let frame = match xenonet_text::json::parse(line) {
        Ok(f) => f,
        Err(e) if e.message == "Nested too deeply" => return Err("a frame nested too deep".into()),
        Err(_) => return Err("not JSON".into()),
    };
    let Some(t) = frame.as_object().and_then(|o| o.get("t")).and_then(Value::as_str) else { return Err("not a frame".into()) };
    if let Some(k) = known {
        if !k.contains(&t) {
            return Err(format!("a kind of frame this XenoNet doesn't know ({})", xenonet_text::utf16::head(t, 32)));
        }
    }
    Ok(frame)
}

pub fn encode(frame: &Value) -> String {
    format!("{}\n", xenonet_text::json::stringify(frame, 0))
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
    fn frames_and_depth() {
        assert!(parse_frame("[1,2]", None).is_err());
        assert!(parse_frame("{\"t\":\"exec\"}", Some(&FROM_RELAY)).unwrap_err().contains("doesn't know (exec)"));
        let deep = xenonet_text::json::parse(&format!("{}{}", "[".repeat(100), "]".repeat(100))).unwrap();
        assert!(!shallow(&deep, MAX_DEPTH));
    }
}
