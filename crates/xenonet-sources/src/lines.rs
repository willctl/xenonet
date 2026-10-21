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
use xenonet_base::usage::{Figure, Speed, TokenUsage};

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

/// A message's usage as an assistant line says it: the call's tokens, each
/// figure as the line wrote it (`?? 0`) except a five-minute cache write it
/// doesn't give, worked out from the others with JS's arithmetic (NaN
/// included); and all its cache writes, which live counts in the context.
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub tokens: TokenUsage<Figure>,
    pub cache_write: Value,
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
    #[expect(clippy::unnecessary_wraps, reason = "the readers' helpers take a Js, an Option, so this hands them the line as one")]
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
        let cache_write_5m = match nn(opt_prop(cc, "ephemeral_5m_input_tokens")) {
            Some(v) => Figure::of(Some(v)),
            // Math.max(0, written - write1h), with JS's arithmetic.
            None => {
                let d = js_number(Some(&or0(prop(u, "cache_creation_input_tokens")?))) - js_number(Some(&write1h));
                Figure::Worked(if d.is_nan() { f64::NAN } else { d.max(0.0) })
            }
        };
        let written = |v: Value| Figure::Written(Some(v));
        Ok(Some(Usage {
            tokens: TokenUsage {
                input: written(or0(prop(u, "input_tokens")?)),
                cache_read: written(or0(prop(u, "cache_read_input_tokens")?)),
                cache_write_5m,
                cache_write_1h: written(write1h),
                output: written(or0(prop(u, "output_tokens")?)),
                speed: if is_str(prop(u, "speed")?, "fast") { Speed::FAST } else { Speed::STANDARD },
            },
            cache_write: or0(prop(u, "cache_creation_input_tokens")?),
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
    #[expect(clippy::unnecessary_wraps, reason = "the readers' helpers take a Js, an Option, so this hands them the line as one")]
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

/// A line of a Grok Build session's updates.jsonl
/// (`~/.grok/sessions/<encoded cwd>/<session id>/updates.jsonl`): a session update, in the envelope Grok
/// writes ({ timestamp, method, params: { sessionId, update } }), or the
/// params alone, as older lines are. An update's kind is its sessionUpdate.
///
/// What the readers read of it in Claude Code's shapes (line_events.rs) is
/// made once, when the line is: a chunk's text, and a reply's blocks (its
/// text, or a tool call).
#[derive(Clone, Debug, PartialEq)]
pub struct GrokLine {
    pub raw: Value,
    text: Value,
    blocks: Value,
}

// An MCP tool's name as Grok writes it: <server>__<tool>.
fn mcp_tool(name: &str) -> bool {
    let word = |s: &str, more: &[u8]| s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric) && s.bytes().all(|b| b.is_ascii_alphanumeric() || more.contains(&b));
    // Split at any "__", as the pattern can: both halves are ASCII there.
    name.as_bytes().windows(2).enumerate().any(|(i, w)| w == b"__" && word(&name[..i], b"_-") && word(&name[i + 2..], b"_.-"))
}

impl GrokLine {
    pub fn new(raw: Value) -> Self {
        let mut line = GrokLine { raw, text: Value::Null, blocks: Value::Array(Vec::new()) };
        let Ok(kind) = line.kind().map(|k| k.and_then(Value::as_str).map(str::to_string)) else { return line };
        let text = line.text();
        line.blocks = match kind.as_deref() {
            Some("agent_message_chunk") => serde_json::json!([{ "type": "text", "text": text }]),
            Some("tool_call") => {
                let u = line.update();
                let mut b = Map::new();
                b.insert("type".into(), "tool_use".into());
                if let Some(id) = opt_prop(Some(u), "toolCallId") {
                    b.insert("id".into(), id.clone());
                }
                b.insert("name".into(), Value::String(line.tool_name()));
                b.insert("input".into(), line.tool_input());
                Value::Array(vec![Value::Object(b)])
            }
            _ => Value::Array(Vec::new()),
        };
        line.text = Value::String(text);
        line
    }

    #[expect(clippy::unnecessary_wraps, reason = "the readers' helpers take a Js, an Option, so this hands them the line as one")]
    fn r(&self) -> Js<'_> {
        Some(&self.raw)
    }

    // The envelope's params, or the line itself for one without an
    // envelope.
    fn params(&self) -> Result<Js<'_>, TypeError> {
        let (method, params) = (prop(self.r(), "method")?, prop(self.r(), "params")?);
        Ok(if method.is_some() || params.is_some() { params } else { self.r() })
    }

    /// The update, or {} when the line has none.
    pub fn update(&self) -> &Value {
        match self.params().map(|p| opt_prop(p, "update")) {
            Ok(Some(u @ Value::Object(_))) => u,
            _ => &EMPTY_OBJECT,
        }
    }
    /// The update's kind: "user_message_chunk", "tool_call", …
    pub fn kind(&self) -> Result<Js<'_>, TypeError> {
        self.params()?;
        Ok(self.update().get("sessionUpdate"))
    }
    /// Whether it's one of Grok's own updates rather than ACP's.
    pub fn own(&self) -> Result<bool, TypeError> {
        Ok(is_str(prop(self.r(), "method")?, "_x.ai/session/update"))
    }
    /// When it was written: the envelope's time is Unix seconds. NaN when
    /// it doesn't say.
    pub fn ts(&self) -> Result<f64, TypeError> {
        Ok(match prop(self.r(), "timestamp")? {
            Some(Value::Number(n)) => n.as_f64().filter(|t| *t > 0.0).map_or(f64::NAN, |t| t * 1000.0),
            t => time_of(t),
        })
    }
    /// A chunk's text: its text block's; "" for another kind of block.
    pub fn text(&self) -> String {
        let c = self.update().get("content");
        match (opt_prop(c, "type"), opt_prop(c, "text")) {
            (Some(Value::String(t)), Some(Value::String(s))) if t == "text" => s.clone(),
            _ => String::new(),
        }
    }
    /// The chunk's text, as a value the events can hold.
    pub fn text_value(&self) -> &Value {
        &self.text
    }
    /// A reply's blocks, in Claude Code's shapes: a chunk's text, or a tool
    /// call as a tool_use.
    pub fn blocks(&self) -> &Value {
        &self.blocks
    }
    /// Whether a chunk of yours isn't something you asked: a turn Grok's
    /// host started, or a shell command you ran from its prompt.
    pub fn not_asked(&self) -> bool {
        let u = Some(self.update());
        opt_prop(opt_prop(u, "_meta"), "hostTurn") == Some(&Value::Bool(true)) || nn(opt_prop(opt_prop(opt_prop(u, "content"), "_meta"), "bash_command")).is_some()
    }
    /// A tool call's name: the one Grok gives its own tools (_meta's
    /// x.ai/tool), or an MCP tool's, which Grok calls by its title
    /// (`<server>__<tool>`), as XenoNet names them (`mcp__<server>__<tool>`);
    /// else the call's kind, as ACP has it.
    pub fn tool_name(&self) -> String {
        let u = Some(self.update());
        if let Some(Value::String(n)) = opt_prop(opt_prop(opt_prop(u, "_meta"), "x.ai/tool"), "name") {
            if !n.is_empty() {
                return n.clone();
            }
        }
        if let Some(Value::String(t)) = opt_prop(u, "title") {
            if mcp_tool(t) {
                return format!("mcp__{t}");
            }
        }
        match opt_prop(u, "kind") {
            Some(Value::String(k)) if !k.is_empty() => k.clone(),
            _ => "tool".into(),
        }
    }
    /// What a tool call was given: its input, with the files it works on
    /// (its locations) as file_path where the input doesn't name one.
    pub fn tool_input(&self) -> Value {
        let u = Some(self.update());
        let input = match opt_prop(u, "rawInput") {
            Some(Value::Object(m)) => m.clone(),
            _ => Map::new(),
        };
        let at = match opt_prop(u, "locations") {
            Some(Value::Array(list)) => list.iter().find_map(|l| opt_prop(Some(l), "path").filter(|p| p.is_string())),
            _ => None,
        };
        let path = nn(input.get("file_path")).or(nn(input.get("path"))).or(nn(input.get("target_file"))).or(at).cloned();
        let mut out = input;
        if let (Some(p @ Value::String(_)), false) = (path, out.contains_key("file_path")) {
            out.insert("file_path".into(), p);
        }
        Value::Object(out)
    }
}

/// Grok Build's lines are sampled by their update's kind.
pub fn grok_kind_of(raw: Js<'_>) -> String {
    let (method, params) = (opt_prop(raw, "method"), opt_prop(raw, "params"));
    let p = if method.is_some() || params.is_some() { params } else { raw };
    kind_part(opt_prop(opt_prop(p, "update"), "sessionUpdate"))
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
        let t = usage.tokens;
        assert_eq!((t.cache_write_5m, t.speed, t.output, usage.cache_write), (Figure::Worked(200.0), Speed::FAST, Figure::Written(Some(json!(0))), json!(300)));
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
    fn grok_lines() {
        let up = |update: Value| GrokLine::new(json!({ "timestamp": 1791201600, "method": "session/update", "params": { "sessionId": "s", "update": update } }));
        let said = up(json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "Add refunds" } }));
        assert_eq!(
            (said.kind().unwrap(), said.ts().unwrap(), said.text(), said.not_asked()),
            (Some(&json!("user_message_chunk")), 1791201600000.0, "Add refunds".to_string(), false)
        );
        let bare = GrokLine::new(json!({ "sessionId": "s", "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hi" } } }));
        assert_eq!((bare.text(), bare.ts().unwrap().is_nan(), bare.blocks()), ("Hi".to_string(), true, &json!([{ "type": "text", "text": "Hi" }])));
        let call = up(
            json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "kind": "edit", "locations": [{ "path": "/a/b.ts" }], "rawInput": { "old_string": "x" }, "_meta": { "x.ai/tool": { "name": "search_replace" } } }),
        );
        assert_eq!(call.blocks(), &json!([{ "type": "tool_use", "id": "c1", "name": "search_replace", "input": { "old_string": "x", "file_path": "/a/b.ts" } }]));
        assert_eq!(up(json!({ "sessionUpdate": "tool_call", "title": "linear__save_issue", "kind": "other" })).tool_name(), "mcp__linear__save_issue");
        assert_eq!(up(json!({ "sessionUpdate": "tool_call", "title": "Run `ls`" })).tool_name(), "tool");
        assert!(mcp_tool("a___b") && mcp_tool("my-server__get.thing") && !mcp_tool("__x") && !mcp_tool("x__") && !mcp_tool("run tests"));
        assert!(up(json!({ "sessionUpdate": "user_message_chunk", "_meta": { "hostTurn": true } })).not_asked());
        let own = GrokLine::new(json!({ "method": "_x.ai/session/update", "params": { "update": { "sessionUpdate": "auto_compact_started" } } }));
        assert!(own.own().unwrap());
        assert!(GrokLine::new(Value::Null).kind().is_err(), "a line that isn't an object has nothing to read");
        assert_eq!(grok_kind_of(Some(&json!({ "params": { "update": { "sessionUpdate": "plan" } } }))), "plan");
        assert_eq!(grok_kind_of(Some(&json!({ "update": [1] }))), "undefined");
    }

    #[test]
    fn half_lines_wait() {
        let t = take(b"{\"a\":1}\n\n{\"b\":2}");
        assert_eq!((t.lines, t.last, t.rest.len()), (vec!["{\"a\":1}".to_string()], Some(json!({ "b": 2 })), 0));
        let t = take(b"{\"a\":1}\n{\"b\"");
        assert_eq!(t.rest, b"{\"b\"");
    }
}
