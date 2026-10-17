//! Transcript lines as XenoNet reads them (core/lines.js): a Claude Code
//! transcript's lines and a Codex rollout's, as views of the vendor's record
//! with XenoNet's names for what the readers read alike.
//!
//! A view reads the record when asked, the way the JS readers did, so a line
//! that broke a reader before breaks it at the same point now: where JS
//! would throw reading a field (a property of null, `.filter` on a number),
//! this gives a `TypeError`. A field JS would read as undefined is None, and
//! one it would read as null is `Some(Value::Null)`.

use serde_json::{Map, Value};
use xenonet_base::json;
use xenonet_base::text::{is_blank, join, js_number, js_trim, js_trim_start, truthy, type_of};
use xenonet_base::time::parse_iso;

/// What JS would throw reading a record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError(pub String);

/// A value as JS would see it: missing (undefined), or there.
pub type Js<'a> = Option<&'a Value>;

/// `v.key`: an object's property; nothing on another value; a TypeError on
/// null or undefined.
pub fn prop<'a>(v: Js<'a>, key: &str) -> Result<Js<'a>, TypeError> {
    match v {
        None => Err(TypeError(format!("Cannot read properties of undefined (reading '{key}')"))),
        Some(Value::Null) => Err(TypeError(format!("Cannot read properties of null (reading '{key}')"))),
        Some(Value::Object(m)) => Ok(m.get(key)),
        Some(_) => Ok(None),
    }
}

/// `v?.key`.
pub fn opt_prop<'a>(v: Js<'a>, key: &str) -> Js<'a> {
    match v {
        None | Some(Value::Null) => None,
        v => prop(v, key).unwrap_or(None),
    }
}

/// `v ?? fallback`, as the value or None for nullish.
fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

fn is_str(v: Js<'_>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

// When a line was written, from its ISO 8601 timestamp: NaN when it says
// none, or something that isn't a time. A number isn't one: both formats
// write a string.
fn time_of(t: Js<'_>) -> f64 {
    match t {
        Some(Value::String(s)) => parse_iso(s).unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

// `xs.filter(f)` (or `.map`) on a value: an array's items, or the
// TypeError JS throws for anything else.
fn items<'a>(v: Js<'a>, what: &str) -> Result<&'a [Value], TypeError> {
    match v {
        Some(Value::Array(a)) => Ok(a),
        _ => Err(TypeError(format!("{what} is not a function"))),
    }
}

static EMPTY_OBJECT: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(Map::new()));
static EMPTY_ARRAY: Value = Value::Array(Vec::new());

/// A call's tokens as XenoNet counts them. `cache_write5m` is the record's
/// own figure, or worked out from the others, so it can be a number JS
/// would get by arithmetic (NaN included).
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub input: Value,
    pub cache_read: Value,
    pub cache_write: Value,
    pub cache_write5m: Result<Value, f64>,
    pub cache_write1h: Value,
    pub output: Value,
    pub fast: bool,
}

/// A tool result a user line carries.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub id: Option<Value>,
    pub error: bool,
}

/// A line of a Claude Code transcript (~/.claude/projects/**/*.jsonl).
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeLine {
    pub raw: Value,
}

impl ClaudeLine {
    fn r(&self) -> Js<'_> {
        Some(&self.raw)
    }

    /// "user", "assistant", or another kind the readers pass over.
    pub fn kind(&self) -> Result<Js<'_>, TypeError> {
        prop(self.r(), "type")
    }
    /// When it was written; NaN when it doesn't say or can't be read.
    pub fn ts(&self) -> Result<f64, TypeError> {
        Ok(time_of(prop(self.r(), "timestamp")?))
    }
    pub fn meta(&self) -> Result<Js<'_>, TypeError> {
        prop(self.r(), "isMeta")
    }
    pub fn sidechain(&self) -> Result<Js<'_>, TypeError> {
        prop(self.r(), "isSidechain")
    }
    /// A compacted context's summary, which carries on from the
    /// conversation before it: no prompt.
    pub fn compact_summary(&self) -> Result<Js<'_>, TypeError> {
        prop(self.r(), "isCompactSummary")
    }
    /// Its effort, or null.
    pub fn effort(&self) -> Result<Option<&Value>, TypeError> {
        Ok(nn(prop(self.r(), "effort")?))
    }

    // ----- a user line -----

    /// What it holds: text, or a list of blocks.
    pub fn content(&self) -> Result<Js<'_>, TypeError> {
        Ok(opt_prop(prop(self.r(), "message")?, "content"))
    }
    /// Whether it carries tool results (and so is no prompt).
    pub fn has_results(&self) -> Result<bool, TypeError> {
        let Some(Value::Array(c)) = self.content()? else { return Ok(false) };
        for x in c {
            if is_str(prop(Some(x), "type")?, "tool_result") {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// The tool results it carries, or None when it carries none.
    pub fn results(&self) -> Result<Option<Vec<ToolResult>>, TypeError> {
        if !self.has_results()? {
            return Ok(None);
        }
        let mut out = Vec::new();
        for x in items(self.content()?, "this.content.filter")? {
            if is_str(prop(Some(x), "type")?, "tool_result") {
                out.push(ToolResult { id: prop(Some(x), "tool_use_id")?.cloned(), error: truthy(prop(Some(x), "is_error")?) });
            }
        }
        Ok(Some(out))
    }
    /// What it says: its text, or its text blocks joined with spaces.
    pub fn text(&self) -> Result<String, TypeError> {
        let c = self.content()?;
        if let Some(Value::String(s)) = c {
            return Ok(s.clone());
        }
        let list = items(Some(nn(c).unwrap_or(&EMPTY_ARRAY)), "(c ?? []).filter")?;
        let mut texts = Vec::new();
        for x in list {
            if is_str(prop(Some(x), "type")?, "text") {
                texts.push(prop(Some(x), "text")?);
            }
        }
        Ok(join(texts.into_iter(), " "))
    }
    /// Whether it's Claude Code's note that you stopped it: no prompt either.
    pub fn interruption(&self) -> Result<bool, TypeError> {
        Ok(js_trim_start(&self.text()?).starts_with("[Request interrupted"))
    }

    // ----- an assistant line -----

    pub fn message(&self) -> Result<&Value, TypeError> {
        Ok(nn(prop(self.r(), "message")?).unwrap_or(&EMPTY_OBJECT))
    }
    /// The message it's part of: Claude Code writes one line per block, each
    /// with the message's id and usage.
    pub fn message_id(&self) -> Result<Js<'_>, TypeError> {
        prop(Some(self.message()?), "id")
    }
    pub fn model(&self) -> Result<Js<'_>, TypeError> {
        prop(Some(self.message()?), "model")
    }
    /// The message's tokens, or None when it doesn't say.
    pub fn usage(&self) -> Result<Option<Usage>, TypeError> {
        let u = prop(Some(self.message()?), "usage")?;
        if !truthy(u) {
            return Ok(None);
        }
        let or0 = |v: Js<'_>| nn(v).cloned().unwrap_or_else(|| Value::from(0));
        let cc = prop(u, "cache_creation")?;
        let write1h = or0(opt_prop(cc, "ephemeral_1h_input_tokens"));
        let cache_write5m = match nn(opt_prop(cc, "ephemeral_5m_input_tokens")) {
            Some(v) => Ok(v.clone()),
            // Math.max(0, written - write1h), with JS's arithmetic.
            None => {
                let d = js_number(Some(&or0(prop(u, "cache_creation_input_tokens")?))) - js_number(Some(&write1h));
                Err(if d.is_nan() { f64::NAN } else { d.max(0.0) })
            }
        };
        Ok(Some(Usage {
            input: or0(prop(u, "input_tokens")?),
            cache_read: or0(prop(u, "cache_read_input_tokens")?),
            cache_write: or0(prop(u, "cache_creation_input_tokens")?),
            cache_write5m,
            cache_write1h: write1h,
            output: or0(prop(u, "output_tokens")?),
            fast: is_str(prop(u, "speed")?, "fast"),
        }))
    }
    /// Its content blocks, as the vendor wrote them.
    pub fn blocks(&self) -> Result<&Value, TypeError> {
        Ok(nn(prop(Some(self.message()?), "content")?).unwrap_or(&EMPTY_ARRAY))
    }
    /// Whether the turn (or a subagent's work) ends with it.
    pub fn end_turn(&self) -> Result<bool, TypeError> {
        Ok(is_str(prop(Some(self.message()?), "stop_reason")?, "end_turn"))
    }
}

/// A line of a Codex rollout (~/.codex/sessions/**/rollout-*.jsonl): an
/// envelope, and its payload as Codex wrote it.
#[derive(Clone, Debug, PartialEq)]
pub struct CodexLine {
    pub raw: Value,
}

impl CodexLine {
    fn r(&self) -> Js<'_> {
        Some(&self.raw)
    }

    /// "event_msg", "response_item", "turn_context", "session_meta", …
    pub fn kind(&self) -> Result<Js<'_>, TypeError> {
        prop(self.r(), "type")
    }
    pub fn ts(&self) -> Result<f64, TypeError> {
        Ok(time_of(prop(self.r(), "timestamp")?))
    }
    pub fn payload(&self) -> Result<&Value, TypeError> {
        Ok(nn(prop(self.r(), "payload")?).unwrap_or(&EMPTY_OBJECT))
    }
    /// What you said, when the line is a completed user message (an
    /// item_completed UserMessage), its parts joined with spaces; else None.
    pub fn user_message(&self) -> Result<Option<String>, TypeError> {
        let p = Some(self.payload()?);
        if !is_str(self.kind()?, "event_msg") || !is_str(prop(p, "type")?, "item_completed") || !is_str(opt_prop(prop(p, "item")?, "type"), "UserMessage") {
            return Ok(None);
        }
        let content = nn(prop(prop(p, "item")?, "content")?).unwrap_or(&EMPTY_ARRAY);
        let parts = items(Some(content), "(p.item.content ?? []).map")?;
        Ok(Some(join(parts.iter().map(|c| opt_prop(Some(c), "text")), " ")))
    }
    /// What you said, when the line is a user_message event; else None.
    pub fn user_event(&self) -> Result<Option<String>, TypeError> {
        let p = Some(self.payload()?);
        if !is_str(self.kind()?, "event_msg") || !is_str(prop(p, "type")?, "user_message") {
            return Ok(None);
        }
        Ok(Some(prop(p, "message")?.and_then(Value::as_str).unwrap_or_default().to_string()))
    }
}

/// Whether a rollout's lines have a completed user message: when they have
/// none at all, its prompts are its user_message events.
pub fn has_user_messages(lines: &[CodexLine]) -> Result<bool, TypeError> {
    for l in lines {
        if l.user_message()?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether what you said in a rollout is a prompt: not blank (an image
/// alone), and not starting with "<" (a delegation envelope, or a note
/// Codex wrote for itself).
pub fn is_codex_prompt(text: Js<'_>) -> bool {
    matches!(text, Some(Value::String(s)) if !js_trim(s).is_empty() && !s.starts_with('<'))
}

// ----- the drift hooks' sorting -----

// A type that isn't a string counts by what it is ("object").
fn kind_part(v: Js<'_>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        v => type_of(v).to_string(),
    }
}

/// Claude Code's lines are sampled by type.
pub fn claude_kind_of(kind: Js<'_>) -> String {
    kind_part(kind)
}

/// Claude Code writes its version on each line.
pub fn claude_version_of(version: Js<'_>, current: Option<String>) -> Option<String> {
    version.and_then(Value::as_str).map(str::to_string).or(current)
}

/// Codex's lines by type and payload type (and an item's type).
pub fn codex_kind_of(kind: Js<'_>, payload_type: Js<'_>, item_type: Js<'_>) -> String {
    let t = kind_part(kind);
    if t != "event_msg" && t != "response_item" {
        return t;
    }
    let p = kind_part(payload_type);
    if p == "item_completed" {
        format!("{t}:{p}:{}", kind_part(item_type))
    } else {
        format!("{t}:{p}")
    }
}

/// A Codex rollout says its version once, in its first line (session_meta).
pub fn codex_version_of(kind: Js<'_>, cli_version: Js<'_>, current: Option<String>) -> Option<String> {
    match (kind, cli_version) {
        (Some(Value::String(k)), Some(Value::String(v))) if k == "session_meta" => Some(v.clone()),
        _ => current,
    }
}

// ----- reading -----

/// The lines in `data`, and what's after the last newline: the whole
/// lines, not blank, in order; `last`, the record after them when it's
/// whole; and `rest`, the bytes to keep for the next read when it isn't (a
/// read can end inside a character, so it's kept as bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct Taken {
    pub lines: Vec<String>,
    pub last: Option<Value>,
    pub rest: Vec<u8>,
}

pub fn take(data: &[u8]) -> Taken {
    let cut = data.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    let lines = String::from_utf8_lossy(&data[..cut]).split('\n').filter(|l| !is_blank(l)).map(str::to_string).collect();
    let rest = &data[cut..];
    let text = String::from_utf8_lossy(rest);
    if is_blank(&text) {
        return Taken { lines, last: None, rest: rest.to_vec() };
    }
    match json::parse(&text) {
        Ok(v) => Taken { lines, last: Some(v), rest: Vec::new() },
        Err(_) => Taken { lines, last: None, rest: rest.to_vec() },
    }
}

/// Where a read of a growing transcript starts: a file shorter than what
/// was read of it was written again, and is read from its start.
pub fn read_from(size: u64, offset: u64) -> (bool, u64) {
    if size < offset {
        (true, 0)
    } else {
        (false, offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_lines() {
        let user = ClaudeLine {
            raw: json!({ "type": "user", "timestamp": "2026-10-03T12:00:00.000Z", "message": { "content": [{ "type": "text", "text": "Fix" }, { "type": "image" }, { "type": "text", "text": "the parser" }] } }),
        };
        assert_eq!((user.text().unwrap(), user.ts().unwrap()), ("Fix the parser".to_string(), 1791028800000.0));
        assert_eq!(user.effort().unwrap(), None);
        let usage = ClaudeLine { raw: json!({ "message": { "usage": { "input_tokens": 10, "cache_creation_input_tokens": 300, "cache_creation": { "ephemeral_1h_input_tokens": 100 }, "speed": "fast" } } }) }.usage().unwrap().unwrap();
        assert_eq!((usage.cache_write5m, usage.fast, usage.output), (Err(200.0), true, json!(0)));
        assert!(ClaudeLine { raw: json!({ "message": { "content": [null] } }) }.has_results().is_err(), "x.type of null throws, as in JS");
        assert!(ClaudeLine { raw: json!({ "message": { "content": { "text": "an object" } } }) }.text().is_err());
        assert!(ClaudeLine { raw: Value::Null }.kind().is_err());
    }

    #[test]
    fn codex_lines_and_kinds() {
        let said = CodexLine {
            raw: json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "type": "UserMessage", "content": [{ "text": "Why is" }, { "path": "/x.png" }, { "text": "it slow?" }] } } }),
        };
        assert_eq!(said.user_message().unwrap().as_deref(), Some("Why is  it slow?"));
        assert_eq!(codex_kind_of(Some(&json!("event_msg")), Some(&json!({ "toString": 1 })), None), "event_msg:object");
        assert_eq!(codex_kind_of(Some(&json!("event_msg")), Some(&json!("item_completed")), None), "event_msg:item_completed:undefined");
        assert!(is_codex_prompt(Some(&json!("Why?"))) && !is_codex_prompt(Some(&json!("  "))) && !is_codex_prompt(Some(&json!("<x>"))));
    }

    #[test]
    fn half_lines_wait() {
        let t = take(b"{\"a\":1}\n\n{\"b\":2}");
        assert_eq!((t.lines, t.last, t.rest.len()), (vec!["{\"a\":1}".to_string()], Some(json!({ "b": 2 })), 0));
        let t = take(b"{\"a\":1}\n{\"b\"");
        assert_eq!(t.rest, b"{\"b\"");
    }
}
