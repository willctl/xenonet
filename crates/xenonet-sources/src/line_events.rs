//! core/events.js: transcript lines as one stream of events
//! (docs/design/vendor-layer.md, "One event stream"). Each app XenoNet
//! tails has one mapper from its normalized lines (lines.rs) to the events
//! below, and the readers read those: the live tailer (live/telemetry.rs),
//! a session's stats and search (xenonet-state), and the refusals
//! (refusals/detect.rs).
//!
//! `file` is what a reader keeps for a file (`LineFile`): a subagent's id,
//! and for Codex the prompt mode (`items`: whether the rollout has a
//! completed user message). Stats and search set the mode from the whole
//! file before mapping; the live tailer sets it as the lines come, and has
//! the mapper read every completed user message (`follow`), since the first
//! one changes the mode. What's kept per session stays the reader's.
//!
//! Every event has the line's time (`t`, NaN when it can't be read), its
//! subagent (`agent`, null for none) and its kind as the sampling hooks
//! name it (`origin`). An event carries what each reader's own rule needs,
//! as written (docs/design/blocks.md), and the rules stay in the readers.
//!
//! The mapper never fails. A field it can't read, where JS would throw
//! (lines.rs's `TypeError`), ends the line's events with `Broken`, saying
//! which, so a reader that would have read that field fails as it did when
//! it read the line itself, and one that wouldn't goes on. Two are read
//! apart from the rest of the line, so their `Broken` stands in their place
//! and the line's events go on: the refusal and a message's usage.

use crate::lines::{claude_kind_of, codex_kind_of, grok_kind_of, is_codex_prompt, opt_prop, prop, ClaudeLine, CodexLine, GrokLine, Js, TypeError, Usage};
use crate::refusals::detect::{claude_refusal, codex_refusal};
use crate::wiki::to_str;
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;
use xenonet_base::text::{join, js_number, js_trim, js_trim_start, truthy, type_of};
use xenonet_base::usage::{Figure, Speed, TokenUsage};

/// A field a line's events couldn't be read past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// The line itself (a record that isn't an object).
    Line,
    Refusal,
    Usage,
    /// An assistant's blocks that aren't a list, or one that isn't there
    /// (null).
    Blocks,
    /// A user line's content, with a block that isn't there.
    Content,
    /// A user line's text.
    Text,
    /// A Codex response item's type, as text.
    Type,
}

impl Field {
    pub fn name(self) -> &'static str {
        match self {
            Field::Line => "line",
            Field::Refusal => "refusal",
            Field::Usage => "usage",
            Field::Blocks => "blocks",
            Field::Content => "content",
            Field::Text => "text",
            Field::Type => "type",
        }
    }
}

/// Where a line broke, what reading it failed with, and what a reader
/// needs to know whether it would have read that far: for a Claude Code
/// user line, its flags, whether it carries tool results, and whether one
/// came before the block that isn't there (search reads a line only as far
/// as its first).
#[derive(Clone, Debug, PartialEq)]
pub struct Broken<'a> {
    pub at: Field,
    pub why: TypeError,
    pub after_result: bool,
    pub results: bool,
    pub meta: Js<'a>,
    pub sidechain: Js<'a>,
    pub compact_summary: Js<'a>,
}

impl Broken<'_> {
    fn at(at: Field, why: TypeError) -> Self {
        Broken { at, why, after_result: false, results: false, meta: None, sidechain: None, compact_summary: None }
    }

    /// What a reader that would have read the field fails with: what
    /// reading it failed with.
    pub fn error(&self) -> TypeError {
        self.why.clone()
    }
}

/// A Claude Code prompt's slash command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub args: Option<String>,
}

/// What a Claude Code prompt says of itself.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudePrompt<'a> {
    pub command: Option<Command>,
    pub meta: Js<'a>,
    pub sidechain: Js<'a>,
    pub compact_summary: Js<'a>,
    pub interruption: bool,
}

/// A prompt: its text blocks joined with spaces, the content as written,
/// and what's Claude Code's own (`claude`).
#[derive(Clone, Debug, PartialEq)]
pub struct Prompt<'a> {
    pub text: String,
    pub parts: Js<'a>,
    pub claude: Option<ClaudePrompt<'a>>,
}

/// A call's tokens: Claude Code's once per message id (`message_id`, with
/// its model and effort), Codex's when its running total grows (`total`
/// is Some, with the last call's usage as written in `last`). `usage` is
/// lines.rs's, or what a token_count's last call comes to.
#[derive(Clone, Debug, PartialEq)]
pub struct Call<'a> {
    pub message_id: Js<'a>,
    pub model: Js<'a>,
    pub effort: Js<'a>,
    pub usage: Option<Usage>,
    pub total: Option<Js<'a>>,
    pub last: Js<'a>,
    pub window: Js<'a>,
}

/// Why a turn ended.
#[derive(Clone, Debug, PartialEq)]
pub enum End<'a> {
    EndTurn,
    TaskComplete {
        duration: Js<'a>,
    },
    /// You stopped it: Claude Code's note, with its line's flags; Codex's
    /// turn_aborted, without.
    Interrupted {
        gate: Option<(Js<'a>, Js<'a>)>,
    },
    /// Claude Code's stop_reason "refusal", with the API's stop_details.
    Refusal {
        details: Js<'a>,
    },
    /// Codex's policy error, with its signal.
    Refused(Value),
}

/// What happened, as the line says it.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind<'a> {
    Refusal(Value),
    Prompt(Prompt<'a>),
    /// What the assistant wrote, as written.
    Said {
        parts: Js<'a>,
        sidechain: Js<'a>,
    },
    Call(Box<Call<'a>>),
    /// A tool call; `call` is a Codex call's type, as written.
    Tool {
        id: Js<'a>,
        name: Js<'a>,
        input: Js<'a>,
        call: Js<'a>,
    },
    /// A tool's result: Claude Code's says whether it failed; a Codex
    /// call's output has the call's type.
    Result {
        id: Js<'a>,
        error: Option<bool>,
        call: Js<'a>,
    },
    /// A Codex completed item, as written.
    Item(&'a Value),
    TurnStart {
        window: Js<'a>,
    },
    TurnEnd(End<'a>),
    /// The model, effort, tier and provider a line says; `tier` is None
    /// where it says none.
    Settings {
        model: Js<'a>,
        effort: Js<'a>,
        tier: Option<Js<'a>>,
        provider: Js<'a>,
    },
    Compact {
        meta: Js<'a>,
        sidechain: Js<'a>,
    },
    Context {
        used: Js<'a>,
        window: Js<'a>,
    },
    Limits(&'a Value),
    Broken(Broken<'a>),
}

/// An event of a line.
#[derive(Clone, Debug, PartialEq)]
pub struct LineEvent<'a> {
    pub t: f64,
    pub agent: Value,
    pub origin: Rc<str>,
    pub kind: Kind<'a>,
}

/// What a reader keeps for a file: whose it is, and for Codex whether its
/// prompts are its completed user messages (`items`), and whether to read
/// every completed user message (`follow`), for a reader finding that out
/// as the lines come; for Grok Build, what the line before was a chunk of
/// (`run`), which the mapper keeps as the lines come.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineFile {
    pub agent: Value,
    pub items: bool,
    pub follow: bool,
    pub run: Cell<Run>,
}

/// What a Grok Build line was a chunk of: a run of chunks of one kind is
/// one message.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Run {
    #[default]
    None,
    User,
    Assistant,
    Thought,
}

impl LineFile {
    pub fn new(agent: Option<Value>) -> Self {
        LineFile { agent: agent.unwrap_or(Value::Null), ..LineFile::default() }
    }
}

/// A call's usage, which a reader reads only for a call it counts: what
/// reading it failed with, when it couldn't be read.
pub fn usage_of(events: &[LineEvent<'_>], c: &Call<'_>) -> Result<Option<Usage>, TypeError> {
    if let Some(b) = broken_at(events, Field::Usage) {
        return Err(b.error());
    }
    Ok(c.usage.clone())
}

/// The `Broken` among a line's events at `field`, if there is one.
pub fn broken_at<'e, 'a>(events: &'e [LineEvent<'a>], field: Field) -> Option<&'e Broken<'a>> {
    events.iter().find_map(|e| match &e.kind {
        Kind::Broken(b) if b.at == field => Some(b),
        _ => None,
    })
}

struct Out<'a> {
    t: f64,
    agent: Value,
    origin: Rc<str>,
    events: Vec<LineEvent<'a>>,
}

impl<'a> Out<'a> {
    fn push(&mut self, kind: Kind<'a>) {
        self.events.push(LineEvent { t: self.t, agent: self.agent.clone(), origin: self.origin.clone(), kind });
    }
    fn broken(&mut self, b: Broken<'a>) {
        self.push(Kind::Broken(b));
    }
}

fn is_str(v: Js<'_>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

fn field<'a>(v: &'a Value, k: &str) -> Js<'a> {
    v.as_object().and_then(|m| m.get(k))
}

// The line's refusal, first: read apart from the rest of the line.
fn refusal(out: &mut Out<'_>, signal: Option<Value>) -> Option<Value> {
    if let Some(s) = &signal {
        out.push(Kind::Refusal(s.clone()));
    }
    signal
}

// ---------- Claude Code ----------

/// A prompt's slash command: `<command-name>` and its name first in the
/// trimmed text (`\/?([^<]+)</command-name>`, the slash taken first when
/// that leaves a name), and the first `<command-args>…</command-args>`.
pub fn command_of(text: &str) -> Option<Command> {
    let t = js_trim(text);
    let rest = t.strip_prefix("<command-name>")?;
    let name_at = |from: usize| -> Option<&str> {
        let run = rest[from..].find('<').map_or(rest.len(), |i| from + i);
        (run > from && rest[run..].starts_with("</command-name>")).then(|| &rest[from..run])
    };
    let name = if rest.starts_with('/') { name_at(1).or_else(|| name_at(0)) } else { name_at(0) }?;
    let args = t.find("<command-args>").and_then(|i| {
        let from = i + "<command-args>".len();
        t[from..].find("</command-args>").map(|j| t[from..from + j].to_string())
    });
    Some(Command { name: name.to_string(), args })
}

/// A Claude Code transcript's line, as events. A session's events are put
/// in time order, as its subagents' files are written beside its own.
pub fn claude<'a>(o: &'a ClaudeLine, file: &LineFile) -> Vec<LineEvent<'a>> {
    let origin: Rc<str> = Rc::from(claude_kind_of(field(&o.raw, "type")));
    let mut out = Out { t: f64::NAN, agent: file.agent.clone(), origin, events: Vec::new() };
    let (t, kind) = match (o.ts(), o.kind()) {
        (Ok(t), Ok(kind)) => (t, kind),
        (Err(why), _) | (_, Err(why)) => {
            out.broken(Broken::at(Field::Line, why));
            return out.events;
        }
    };
    out.t = t;
    refusal(&mut out, claude_refusal(Some(&o.raw)));
    if is_str(kind, "assistant") {
        claude_assistant(o, &mut out);
    } else if is_str(kind, "user") {
        claude_user(o, &mut out);
    }
    out.events
}

fn claude_assistant<'a>(o: &'a ClaudeLine, out: &mut Out<'a>) {
    let message = match o.message() {
        Ok(m) => m,
        Err(why) => return out.broken(Broken::at(Field::Line, why)),
    };
    let m = Some(message);
    // Each of a message's lines repeats its usage; the readers count it
    // once per id, so it's read apart.
    let usage = o.usage();
    out.push(Kind::Call(Box::new(Call {
        message_id: opt_prop(m, "id"),
        model: opt_prop(m, "model"),
        effort: nn(field(&o.raw, "effort")),
        usage: usage.as_ref().ok().cloned().flatten(),
        total: None,
        last: None,
        window: None,
    })));
    if let Err(why) = usage {
        out.broken(Broken::at(Field::Usage, why));
    }
    let blocks = nn(opt_prop(m, "content"));
    out.push(Kind::Said { parts: Some(blocks.unwrap_or(&EMPTY_ARRAY)), sidechain: field(&o.raw, "isSidechain") });
    // Its blocks as the readers go through them: a string's characters are
    // blocks without a type, and anything else that isn't a list, or a
    // block that isn't there, can't be read.
    let list: &[Value] = match blocks.unwrap_or(&EMPTY_ARRAY) {
        Value::Array(a) => a,
        Value::String(_) => &[],
        _ => return out.broken(Broken::at(Field::Blocks, TypeError("o.blocks is not iterable".into()))),
    };
    for b in list {
        let b = Some(b);
        match prop(b, "type") {
            Err(why) => return out.broken(Broken::at(Field::Blocks, why)),
            Ok(t) if is_str(t, "tool_use") => out.push(Kind::Tool { id: opt_prop(b, "id"), name: opt_prop(b, "name"), input: opt_prop(b, "input"), call: None }),
            Ok(_) => {}
        }
    }
    let reason = opt_prop(m, "stop_reason");
    if is_str(reason, "end_turn") {
        out.push(Kind::TurnEnd(End::EndTurn));
    } else if is_str(reason, "refusal") {
        out.push(Kind::TurnEnd(End::Refusal { details: opt_prop(m, "stop_details") }));
    }
}

static EMPTY_ARRAY: Value = Value::Array(Vec::new());

fn claude_user<'a>(o: &'a ClaudeLine, out: &mut Out<'a>) {
    let raw = &o.raw;
    let (meta, sidechain, compact_summary) = (field(raw, "isMeta"), field(raw, "isSidechain"), field(raw, "isCompactSummary"));
    let content = opt_prop(field(raw, "message"), "content");
    let mut results = 0;
    if let Some(Value::Array(c)) = content {
        // A block that isn't there can't be read; search reads a line only
        // as far as its first tool result.
        if let Err(why) = o.results() {
            let gap = c.iter().position(Value::is_null).unwrap_or(c.len());
            let after_result = c[..gap].iter().any(|x| is_str(field(x, "type"), "tool_result"));
            return out.broken(Broken { after_result, meta, ..Broken::at(Field::Content, why) });
        }
        for x in c {
            if is_str(field(x, "type"), "tool_result") {
                results += 1;
                out.push(Kind::Result { id: field(x, "tool_use_id"), error: Some(truthy(field(x, "is_error"))), call: None });
            }
        }
    }
    if results == 0 && truthy(compact_summary) {
        out.push(Kind::Compact { meta, sidechain });
    }
    let text = match o.text() {
        Ok(text) => text,
        Err(why) => return out.broken(Broken { meta, sidechain, compact_summary, results: results > 0, ..Broken::at(Field::Text, why) }),
    };
    let interruption = js_trim_start(&text).starts_with("[Request interrupted");
    if results == 0 {
        let claude = ClaudePrompt { command: command_of(&text), meta, sidechain, compact_summary, interruption };
        out.push(Kind::Prompt(Prompt { text, parts: content, claude: Some(claude) }));
    }
    if interruption {
        out.push(Kind::TurnEnd(End::Interrupted { gate: Some((meta, sidechain)) }));
    }
}

// ---------- Codex ----------

/// `Math.max(0, x)`, NaN kept.
fn max0(x: f64) -> f64 {
    if x.is_nan() || x > 0.0 {
        x
    } else {
        0.0
    }
}

// A figure as JS's arithmetic reads it: a value that can't be made a
// primitive (an object with its own toString) throws.
fn number(f: &Figure) -> Result<f64, TypeError> {
    match f {
        Figure::Worked(n) => Ok(*n),
        Figure::Written(v @ Some(Value::Array(_) | Value::Object(_))) => Ok(js_number(Some(&Value::String(to_str(v.as_ref())?)))),
        Figure::Written(v) => Ok(js_number(v.as_ref())),
    }
}

/// A call's tokens from a token_count's last_token_usage, as Claude Code's
/// usage has them: cached input and cache writes are part of input_tokens,
/// and each figure stays as written.
pub fn codex_usage(last: Js<'_>) -> Result<Usage, TypeError> {
    static EMPTY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(serde_json::Map::new()));
    let u = Some(nn(last).unwrap_or(&EMPTY));
    let input = match nn(prop(u, "input_tokens")?) {
        Some(v) => Figure::of(Some(v)),
        None => Figure::Worked(max0(number(&Figure::or0(prop(u, "total_tokens")?))? - number(&Figure::or0(prop(u, "output_tokens")?))?)),
    };
    let cached = Figure::or0(prop(u, "cached_input_tokens")?);
    let write = Figure::or0(prop(u, "cache_write_input_tokens")?);
    let fresh = max0(number(&input)? - number(&cached)? - number(&write)?);
    Ok(Usage {
        tokens: TokenUsage {
            input: Figure::Worked(fresh),
            cache_read: cached,
            cache_write_5m: write,
            cache_write_1h: Figure::Worked(0.0),
            output: Figure::or0(prop(u, "output_tokens")?),
            speed: Speed::STANDARD,
        },
        cache_write: Value::Null,
    })
}

/// What a completed user message says: its parts' text joined with spaces
/// (as CodexLine::user_message reads it).
pub fn user_text(it: &Value) -> Result<String, TypeError> {
    match nn(field(it, "content")) {
        None => Ok(String::new()),
        Some(Value::Array(parts)) => Ok(join(parts.iter().map(|c| opt_prop(Some(c), "text")), " ")),
        Some(_) => Err(TypeError("(it.content ?? []).map is not a function".into())),
    }
}

// A completed item's events: the item, and what a user or agent message
// says (`said`: the user message's text, as read from its line).
fn item_events<'a>(it: &'a Value, file: &LineFile, out: &mut Out<'a>, said: impl FnOnce() -> Result<String, TypeError>) {
    out.push(Kind::Item(it));
    let kind = field(it, "type");
    if is_str(kind, "UserMessage") {
        // A rollout's prompts are its completed user messages, or its
        // user_message events while it has none; live finds out as the
        // lines come, the first one counting from then on, so it reads
        // every one.
        if !file.follow && !file.items {
            return;
        }
        let text = match said() {
            Ok(text) => text,
            Err(why) => return out.broken(Broken::at(Field::Text, why)),
        };
        if is_codex_prompt(Some(&Value::String(text.clone()))) {
            out.push(Kind::Prompt(Prompt { text, parts: field(it, "content"), claude: None }));
        }
    } else if is_str(kind, "AgentMessage") {
        out.push(Kind::Said { parts: field(it, "content"), sidechain: None });
    }
}

const CALLS: [&str; 3] = ["function_call", "custom_tool_call", "local_shell_call"];
const OUTPUTS: [&str; 3] = ["function_call_output", "custom_tool_call_output", "local_shell_call_output"];

static EMPTY_OBJECT: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(serde_json::Map::new()));

fn codex_event<'a>(o: &'a CodexLine, p: &'a Value, signal: Option<&Value>, file: &LineFile, out: &mut Out<'a>) {
    let get = |k: &str| field(p, k);
    match get("type").and_then(Value::as_str) {
        Some("task_started") => out.push(Kind::TurnStart { window: get("model_context_window") }),
        Some("task_complete") => {
            if truthy(get("last_agent_message")) {
                out.push(Kind::Said { parts: get("last_agent_message"), sidechain: None });
            }
            out.push(Kind::TurnEnd(End::TaskComplete { duration: get("duration_ms") }));
        }
        Some("token_count") => {
            // Codex reports a call's tokens more than once: each reader
            // counts it when the total grows, by its own rule. Its usage is
            // read apart, as a reader reads it only for a call it counts.
            let info = get("info");
            if truthy(info) {
                let last = opt_prop(info, "last_token_usage");
                let usage = codex_usage(last);
                out.push(Kind::Call(Box::new(Call {
                    message_id: None,
                    model: None,
                    effort: None,
                    usage: usage.as_ref().ok().cloned(),
                    total: Some(opt_prop(info, "total_token_usage")),
                    last,
                    window: opt_prop(info, "model_context_window"),
                })));
                if let Err(why) = usage {
                    out.broken(Broken::at(Field::Usage, why));
                }
            }
            out.push(Kind::Context { used: opt_prop(opt_prop(info, "last_token_usage"), "total_tokens"), window: opt_prop(info, "model_context_window") });
            if let Some(rl) = get("rate_limits").filter(|r| truthy(Some(r))) {
                out.push(Kind::Limits(rl));
            }
        }
        Some("item_completed") => item_events(nn(get("item")).unwrap_or(&EMPTY_OBJECT), file, out, || o.user_message().map(Option::unwrap_or_default)),
        Some("user_message") => {
            let text = match get("message") {
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            if !file.items && is_codex_prompt(Some(&Value::String(text.clone()))) {
                out.push(Kind::Prompt(Prompt { text, parts: get("message"), claude: None }));
            }
        }
        // You stopped the turn (Esc, or Ctrl-C). A turn the next one
        // replaced, or a review ending, isn't you stopping it; older
        // rollouts give no reason.
        Some("turn_aborted") => {
            let reason = get("reason");
            if !truthy(reason) || is_str(reason, "interrupted") {
                out.push(Kind::TurnEnd(End::Interrupted { gate: None }));
            }
        }
        // A policy refusal ends the turn.
        Some("error") => {
            if let Some(s) = signal.filter(|s| is_str(field(s, "kind"), "refusal")) {
                out.push(Kind::TurnEnd(End::Refused(s.clone())));
            }
        }
        // All of the thread's settings: a tier not said is the default one.
        Some("thread_settings_applied") => out.push(Kind::Settings { model: None, effort: None, tier: Some(opt_prop(get("thread_settings"), "service_tier")), provider: None }),
        Some("agent_message") => out.push(Kind::Said { parts: get("message"), sidechain: None }),
        _ => {}
    }
}

// What a response item says: an assistant's message, or a call or its
// output.
fn response_item<'a>(p: &'a Value, out: &mut Out<'a>) {
    let get = |k: &str| field(p, k);
    if is_str(get("type"), "message") && is_str(get("role"), "assistant") {
        return out.push(Kind::Said { parts: get("content"), sidechain: None });
    }
    // The type as stats matches it, as text: one that isn't a string is
    // turned into one, which can fail.
    let t = match to_str(get("type")) {
        Ok(t) => t,
        Err(why) => return out.broken(Broken::at(Field::Type, why)),
    };
    let id = match nn(get("call_id")) {
        Some(v) => Some(v),
        None => get("id"),
    };
    if CALLS.contains(&t.as_str()) {
        out.push(Kind::Tool { id, name: get("name"), input: nn(get("input")).or(get("arguments")), call: get("type") });
    } else if OUTPUTS.contains(&t.as_str()) {
        out.push(Kind::Result { id, error: None, call: get("type") });
    }
}

/// A Codex rollout's line, as events.
pub fn codex<'a>(o: &'a CodexLine, file: &LineFile) -> Vec<LineEvent<'a>> {
    let payload = field(&o.raw, "payload");
    let origin: Rc<str> =
        Rc::from(codex_kind_of(field(&o.raw, "type"), payload.and_then(|p| field(p, "type")), payload.and_then(|p| field(p, "item")).and_then(|i| field(i, "type"))));
    let mut out = Out { t: f64::NAN, agent: file.agent.clone(), origin, events: Vec::new() };
    let (t, p, kind) = match (o.ts(), o.payload(), o.kind()) {
        (Ok(t), Ok(p), Ok(kind)) => (t, p, kind),
        (Err(why), _, _) | (_, Err(why), _) | (_, _, Err(why)) => {
            out.broken(Broken::at(Field::Line, why));
            return out.events;
        }
    };
    out.t = t;
    let signal = refusal(&mut out, codex_refusal(Some(&o.raw)));
    let get = |k: &str| field(p, k);
    if is_str(kind, "event_msg") {
        codex_event(o, p, signal.as_ref(), file, &mut out);
    } else if is_str(kind, "turn_context") {
        // Older rollouts say the tier with each turn; newer ones in the
        // thread's settings.
        let tier = get("service_tier").map(Some);
        out.push(Kind::Settings { model: get("model"), effort: get("effort"), tier, provider: None });
    } else if is_str(kind, "session_meta") {
        out.push(Kind::Settings { model: None, effort: None, tier: None, provider: get("model_provider") });
    } else if is_str(kind, "response_item") {
        response_item(p, &mut out);
    }
    out.events
}

/// A completed item's events, for a reader handed the item (live's
/// codex_item), at `t`.
pub fn codex_item<'a>(it: &'a Value, t: f64, file: &LineFile) -> Vec<LineEvent<'a>> {
    let kind = match field(it, "type") {
        Some(Value::String(s)) => s.clone(),
        v => type_of(v).to_string(),
    };
    let origin: Rc<str> = Rc::from(format!("event_msg:item_completed:{kind}"));
    let mut out = Out { t, agent: file.agent.clone(), origin, events: Vec::new() };
    item_events(it, file, &mut out, || user_text(it));
    out.events
}

// ---------- Grok Build ----------

/// Whether an app's lines say when a turn ends (events.js's mapper's
/// turnEnds): Grok Build's don't, and its hooks do.
pub fn lines_end_turns(app: &str) -> bool {
    app != "grok"
}

static TRUE: Value = Value::Bool(true);
static FALSE: Value = Value::Bool(false);
static NULL: Value = Value::Null;

/// A Grok Build session's line, as events (events.js's grok): Claude Code's
/// shapes, with its origins (user for what you said and a tool's result,
/// assistant for what the agent said and its tool calls), so the readers
/// read them as they read Claude Code's. A run of chunks of the same kind
/// is one message, which its first chunk speaks for: the chunks after it
/// make no events (`file.run`). Its lines don't say when a turn ends, nor
/// what a call cost.
pub fn grok<'a>(o: &'a GrokLine, file: &LineFile) -> Vec<LineEvent<'a>> {
    let mut out = Out { t: f64::NAN, agent: file.agent.clone(), origin: Rc::from("undefined"), events: Vec::new() };
    let (t, kind, own) = match (o.ts(), o.kind(), o.own()) {
        (Ok(t), Ok(kind), Ok(own)) => (t, kind, own),
        (Err(why), _, _) | (_, Err(why), _) | (_, _, Err(why)) => {
            out.broken(Broken::at(Field::Line, why));
            return out.events;
        }
    };
    out.t = t;
    out.origin = Rc::from(grok_kind_of(Some(&o.raw)));
    let kind = kind.and_then(Value::as_str);
    let run = match kind {
        Some("user_message_chunk") => Run::User,
        Some("agent_message_chunk") => Run::Assistant,
        Some("agent_thought_chunk") => Run::Thought,
        _ => Run::None,
    };
    let more = run != Run::None && run == file.run.get();
    file.run.set(run);
    if more || own {
        return out.events;
    }
    let u = Some(o.update());
    let done = |status: Js<'_>| is_str(status, "completed") || is_str(status, "failed");
    let call = || Kind::Call(Box::new(Call { message_id: Some(&NULL), model: Some(&NULL), effort: Some(&NULL), usage: None, total: None, last: None, window: None }));
    match kind {
        Some("user_message_chunk") => {
            out.origin = Rc::from("user");
            let meta = if o.not_asked() { &TRUE } else { &FALSE };
            let claude = ClaudePrompt { command: None, meta: Some(meta), sidechain: Some(&FALSE), compact_summary: Some(&FALSE), interruption: false };
            out.push(Kind::Prompt(Prompt { text: o.text(), parts: Some(o.text_value()), claude: Some(claude) }));
        }
        Some("agent_message_chunk") => {
            out.origin = Rc::from("assistant");
            out.push(call());
            out.push(Kind::Said { parts: Some(o.blocks()), sidechain: Some(&FALSE) });
        }
        Some("tool_call") => {
            out.origin = Rc::from("assistant");
            let b = o.blocks().get(0);
            out.push(call());
            out.push(Kind::Said { parts: Some(o.blocks()), sidechain: Some(&FALSE) });
            let id = opt_prop(b, "id");
            out.push(Kind::Tool { id, name: opt_prop(b, "name"), input: opt_prop(b, "input"), call: None });
            let status = opt_prop(u, "status");
            if done(status) {
                out.push(Kind::Result { id, error: Some(is_str(status, "failed")), call: None });
            }
        }
        Some("tool_call_update") => {
            let status = opt_prop(u, "status");
            if done(status) {
                out.origin = Rc::from("user");
                out.push(Kind::Result { id: opt_prop(u, "toolCallId"), error: Some(is_str(status, "failed")), call: None });
            }
        }
        Some("usage_update") => out.push(Kind::Context { used: opt_prop(u, "used"), window: opt_prop(u, "size") }),
        _ => {}
    }
    out.events
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kinds(evs: &[LineEvent<'_>]) -> Vec<String> {
        evs.iter()
            .map(|e| match &e.kind {
                Kind::Broken(b) => format!("broken:{}", b.at.name()),
                k => format!("{k:?}").split([' ', '(', '{']).next().unwrap_or_default().to_string(),
            })
            .collect()
    }

    #[test]
    fn claude_lines_as_events() {
        let ts = "2026-10-03T12:00:00.000Z";
        let f = LineFile::default();
        let o = ClaudeLine { raw: json!({ "type": "user", "timestamp": ts, "message": { "content": "<command-name>/goal</command-name><command-args>ship</command-args>" } }) };
        let evs = claude(&o, &f);
        let Kind::Prompt(p) = &evs[0].kind else { panic!("a prompt") };
        assert_eq!(p.claude.as_ref().and_then(|c| c.command.clone()), Some(Command { name: "goal".into(), args: Some("ship".into()) }));
        let o = ClaudeLine { raw: json!({ "type": "user", "timestamp": ts, "message": { "content": [{ "type": "tool_result", "tool_use_id": "a" }, null] } }) };
        let evs = claude(&o, &f);
        assert!(matches!(&evs[0].kind, Kind::Broken(b) if b.at == Field::Content && b.after_result));
        let o = ClaudeLine {
            raw: json!({ "type": "user", "timestamp": ts, "message": { "content": [{ "type": "tool_result", "tool_use_id": "a" }, { "type": "text", "text": "[Request interrupted by user]" }] } }),
        };
        assert_eq!(kinds(&claude(&o, &f)), ["Result", "TurnEnd"]);
        let o = ClaudeLine { raw: json!({ "type": "user", "timestamp": ts, "isCompactSummary": true, "message": { "content": 5 } }) };
        assert_eq!(kinds(&claude(&o, &f)), ["Compact", "broken:text"]);
        let o = ClaudeLine {
            raw: json!({ "type": "assistant", "timestamp": ts, "message": { "id": "m", "stop_reason": "refusal", "content": [{ "type": "text", "text": "no" }, { "type": "tool_use", "id": "t", "name": "Bash" }] } }),
        };
        assert_eq!(kinds(&claude(&o, &f)), ["Refusal", "Call", "Said", "Tool", "TurnEnd"]);
        let o = ClaudeLine { raw: json!({ "type": "assistant", "timestamp": ts, "message": { "content": 5 } }) };
        assert_eq!(kinds(&claude(&o, &f)), ["Call", "Said", "broken:blocks"]);
        let o = ClaudeLine { raw: Value::Null };
        assert_eq!(kinds(&claude(&o, &f)), ["broken:line"]);
    }

    #[test]
    fn grok_lines_as_events() {
        let up = |update: Value| GrokLine::new(json!({ "timestamp": 1791201600, "method": "session/update", "params": { "update": update } }));
        let text = |kind: &str, t: &str| up(json!({ "sessionUpdate": kind, "content": { "type": "text", "text": t } }));
        let lines = [
            text("user_message_chunk", "Add refunds "),
            text("user_message_chunk", "to the API."),
            text("agent_thought_chunk", "Hm."),
            text("agent_message_chunk", "On it."),
            text("agent_message_chunk", " Testing."),
            up(json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "kind": "execute", "status": "in_progress", "rawInput": { "command": "npm test" } })),
            up(json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed" })),
            up(json!({ "sessionUpdate": "tool_call", "toolCallId": "c2", "kind": "read", "status": "completed" })),
            up(json!({ "sessionUpdate": "usage_update", "used": 10, "size": 20 })),
            GrokLine::new(
                json!({ "method": "_x.ai/session/update", "params": { "update": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "x" } } } }),
            ),
            GrokLine::new(Value::Null),
        ];
        let file = LineFile::default();
        let evs: Vec<Vec<LineEvent<'_>>> = lines.iter().map(|o| grok(o, &file)).collect();
        assert_eq!(
            evs.iter().map(|e| kinds(e)).collect::<Vec<_>>(),
            [
                vec!["Prompt"],
                vec![],
                vec![],
                vec!["Call", "Said"],
                vec![],
                vec!["Call", "Said", "Tool"],
                vec!["Result"],
                vec!["Call", "Said", "Tool", "Result"],
                vec!["Context"],
                vec![],
                vec!["broken:line"],
            ]
            .map(|v| v.into_iter().map(String::from).collect::<Vec<_>>())
        );
        let Kind::Prompt(p) = &evs[0][0].kind else { panic!("a prompt") };
        assert_eq!((p.text.as_str(), &*evs[0][0].origin, evs[0][0].t), ("Add refunds ", "user", 1791201600000.0));
        assert!(matches!(&evs[6][0].kind, Kind::Result { error: Some(true), .. }) && &*evs[6][0].origin == "user");
        assert!(matches!(&evs[8][0].kind, Kind::Context { used: Some(u), .. } if **u == json!(10)) && &*evs[8][0].origin == "usage_update");
        assert!(!lines_end_turns("grok") && lines_end_turns("claude"));
    }

    #[test]
    fn codex_prompts_by_the_mode() {
        let line = |p: Value| CodexLine { raw: json!({ "type": "event_msg", "timestamp": "2026-10-03T12:00:00.000Z", "payload": p }) };
        let live = LineFile { follow: true, ..LineFile::default() };
        let said = line(json!({ "type": "user_message", "message": "hi" }));
        assert_eq!(kinds(&codex(&said, &live)), ["Prompt"]);
        let done = line(json!({ "type": "item_completed", "item": { "type": "UserMessage", "content": [{ "text": "hi" }] } }));
        let evs = codex(&done, &live);
        assert!(matches!(&evs[1].kind, Kind::Prompt(p) if p.text == "hi"), "live reads every completed user message");
        let items = LineFile { items: true, ..live };
        assert!(kinds(&codex(&said, &items)).is_empty(), "events aren't prompts once it has one");
        let whole = LineFile::default();
        assert_eq!(kinds(&codex(&done, &whole)), ["Item"], "a rollout read whole as having none");
        let count = line(
            json!({ "type": "token_count", "info": { "total_token_usage": { "total_tokens": 5 }, "last_token_usage": { "input_tokens": 10, "cached_input_tokens": 4 } }, "rate_limits": { "primary": {} } }),
        );
        let evs = codex(&count, &whole);
        assert_eq!(kinds(&evs), ["Call", "Context", "Limits"]);
        let Kind::Call(c) = &evs[0].kind else { panic!("a call") };
        assert_eq!(c.usage.as_ref().map(|u| (u.tokens.input.clone(), u.tokens.cache_read.clone())), Some((Figure::Worked(6.0), Figure::Written(Some(json!(4))))));
        let refused = line(json!({ "type": "error", "codex_error_info": "cyber_policy" }));
        assert_eq!(kinds(&codex(&refused, &whole)), ["Refusal", "TurnEnd"]);
        let odd = CodexLine { raw: json!({ "type": "response_item", "payload": { "type": { "toString": 1 } } }) };
        assert_eq!(kinds(&codex(&odd, &whole)), ["broken:type"]);
    }
}
