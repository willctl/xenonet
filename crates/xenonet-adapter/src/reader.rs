//! What a platform brings to the map: the readers of its sessions, which
//! the core drives without knowing whose they are.
//!
//! - `Tailed`: an app whose transcripts the live tailer follows. The
//!   adapter says which files are its sessions' (`classify`), how a line
//!   is sampled for the drift check (`version_of`, `kind_of`) and what each
//!   line means (`line`), through `Tail`, the generic operations the core's
//!   telemetry offers on a session (its tokens, calls, prompts, tools,
//!   subagents, the events it emits). The adapter keeps what is its own
//!   (its plan limits, what its lines carry between them).
//! - `Native`: an app with a reader of its own records (OpenCode's
//!   database, Gemini CLI's folder): where it reads from, and a reader,
//!   over the contract in `native`.
//! - `Lister`: an app whose sessions the map lists and whose transcripts
//!   it shows (Claude Code's transcripts, Codex's app-server, Grok Build's
//!   folders): its sessions, a page of a session, a rename, its own
//!   projects, and the block of a client it keeps running.
//!
//! The core asks the registry for these (`Platform::tailed`,
//! `Platform::native`, `Platform::lister`); the drivers in xenonet-sources
//! and xenonet-wire-map implement `Tail` and run the readers.

pub mod files;
pub mod native;

use serde_json::{Map, Value};
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_model::usage::TokenUsage;
use xenonet_schema::vendor::Sampling;
use xenonet_text::json::number;

/// What JS would throw reading a record: the error every reader of
/// transcripts gives where a line isn't shaped as its app writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeError(pub String);

/// An event's fields as JS has them, in order: a field that's there with
/// an undefined value is None (an event's `agentId` from Codex, say).
pub type Fields = Vec<(String, Option<Value>)>;

/// An event the telemetry emits, in the order they happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Emit {
    /// "activity": what a session did.
    Activity(Fields),
    /// "tokens": { sessionId, app, n, ts }.
    Tokens(Value),
    /// "refusal": a provider's refusal, as its line marks it.
    Refusal(Value),
}

impl Emit {
    /// A "tokens" event: a session of `app` processed `n` tokens at `ts`.
    pub fn tokens(session_id: &str, app: &str, n: f64, ts: f64) -> Emit {
        let mut m = Map::new();
        m.insert("sessionId".into(), Value::String(session_id.into()));
        m.insert("app".into(), Value::String(app.into()));
        m.insert("n".into(), number(n));
        m.insert("ts".into(), number(ts));
        Emit::Tokens(Value::Object(m))
    }
}

/// Where a line was read: its session, its subagent (null for none) and
/// its file.
#[derive(Clone, Debug)]
pub struct Where {
    pub session_id: String,
    pub agent_id: Value,
    pub file: Value,
}

/// A call's tokens and how it ran, for its price.
#[derive(Clone, Copy, Debug, Default)]
pub struct Call {
    pub tokens: TokenUsage,
}

/// A tool call, as the telemetry notes one. `agent` is None where no
/// agentId was given (undefined), else the agent's id or null.
#[derive(Clone, Debug, Default)]
pub struct ToolCall {
    pub name: Option<Value>,
    pub detail: String,
    pub full: Option<Value>,
    pub files: Vec<String>,
    pub ts: f64,
    pub agent: Option<Value>,
    pub server: Option<Value>,
    /// Fields the event takes on (`...extra`): the pages a wiki write names.
    pub extra: Option<Map<String, Value>>,
    pub ok: Value,
}

/// The record the stall check keeps of a tool call ({ key, label,
/// loopable, ok, ts }), with the id it's found by among the recent ones.
pub type ToolRecord = (u64, Map<String, Value>);

/// What the open run of chunks began, which its later chunks carry on:
/// the last prompt, or the last message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Prompt,
    Message,
}

/// What a transcript is: the run's own id, and its subagent's when the
/// file is a subagent's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Classified {
    pub id: String,
    pub agent: Option<String>,
}

/// The vendor-schema checks the map offers a reader: a record checked
/// whole, or one of every `every` sampled, each against its source's
/// schema, with what differs recorded as drift. Gives the record to use,
/// or None to drop it. A transcript's lines go through their app's
/// normalizer (`line`, `record`), and a reader says what it looked for
/// and didn't find (`pick`, `missing`); without the map's schemas these
/// take every line as it is and record nothing.
pub trait VendorCheck: Sync {
    fn check(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>) -> Option<Value>;
    fn sample(&self, source: &str, record: Value, version: Option<&str>, kind: Option<&str>, every: u64) -> Option<Value>;
    /// A transcript line of `app`'s through its normalizer: the record to
    /// read, or None for a line that isn't JSON or is dropped. `sampling`
    /// is the file's: which of its lines are checked whole.
    fn line(&self, _app: &str, text: &str, _sampling: &mut Sampling) -> Option<Value> {
        xenonet_text::json::parse(text).ok()
    }
    /// A record already parsed, through the normalizer, as the last line
    /// of a file without a newline is read.
    fn record(&self, _app: &str, raw: Value, _sampling: &mut Sampling) -> Option<Value> {
        Some(raw)
    }
    /// A version of a source none of its schemas has: drift.
    fn pick(&self, _source: &str, _version: Option<&str>) {}
    /// A field a reader looks for and didn't find.
    fn missing(&self, _source: &str, _path: &str, _version: Option<&str>) {}
}

/// Checks that take every record as it is, for a reader run without the
/// map's schemas (a test's).
pub struct Unchecked;

impl VendorCheck for Unchecked {
    fn check(&self, _: &str, record: Value, _: Option<&str>, _: Option<&str>) -> Option<Value> {
        Some(record)
    }
    fn sample(&self, _: &str, record: Value, _: Option<&str>, _: Option<&str>, _: u64) -> Option<Value> {
        Some(record)
    }
}

/// The telemetry's generic operations, as a tailed app's line handler
/// reaches them: the sessions by index (made when first named), each
/// session's counts and texts, the events emitted (`out`), and the
/// records every app keeps alike (subagents, calls in flight, messages
/// seen). `live` on an operation says the line is read as it's written,
/// not as history; `now` is the clock.
#[expect(clippy::too_many_arguments, reason = "a line's events and their time, handed down as the per-app code worked them out")]
pub trait Tail {
    // ----- sessions -----

    /// The session's index, made when it's new.
    fn session(&mut self, id: &str, app: &str) -> usize;
    fn find(&self, id: &str) -> Option<usize>;
    fn session_id(&self, i: usize) -> &str;
    /// The app the session is read as.
    fn session_app(&self, i: usize) -> &str;
    /// The home folder, for paths shown with ~.
    fn home(&self) -> Option<&str>;
    /// The MCP server the config names for the shared context.
    fn context_server(&self) -> Option<&str>;
    /// A line that may name an MCP server and what it is.
    fn observe_servers(&mut self, raw: &Value);

    // ----- events -----

    /// An activity event: `ev` as the app's code made it.
    fn activity(&mut self, i: usize, ev: Fields, live: bool, now: f64, out: &mut Vec<Emit>);
    /// A tool call: counted, remembered for the stall check (but a
    /// subagent's), and told. Gives the record kept of it.
    fn note_tool(&mut self, i: usize, t: ToolCall, live: bool, now: f64, out: &mut Vec<Emit>) -> Option<ToolRecord>;
    /// A provider's refusal, as a line marks it (`signal`).
    fn refusal(&self, app: &str, at: &Where, raw: &Value, signal: &Value, live: bool, out: &mut Vec<Emit>);
    /// What you asked: a turn begins.
    fn prompted(&mut self, i: usize, text: &str, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>);
    /// A chunk that carries on a prompt or a message (`of`: "prompt" or
    /// "text"), with the whole text so far.
    fn more(&mut self, i: usize, of: &str, whole: &str, meta: bool, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError>;
    /// You stopped the turn under way.
    fn interrupted(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>);
    /// How long the turn under way has taken at `ts`, or null.
    fn duration(&self, i: usize, ts: f64) -> Value;
    /// After a turn's end: it asked you something.
    fn ask_if_question(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>);

    // ----- a session's turn -----

    fn turn_active(&self, i: usize) -> bool;
    /// A turn begins at `ts`.
    fn begin_turn(&mut self, i: usize, ts: f64);
    /// A turn's end; `interrupted` when you stopped it.
    fn end_turn(&mut self, i: usize, ts: f64, interrupted: bool);
    fn turn_marks(&self, i: usize) -> f64;
    fn add_turn_mark(&mut self, i: usize);
    fn reset_stall(&mut self, i: usize);
    /// The session was heard from at `ts`.
    fn touch(&mut self, i: usize, ts: f64);
    /// What the open run of chunks began.
    fn set_run(&mut self, i: usize, run: Option<Run>);

    // ----- counts -----

    fn model(&self, i: usize) -> Value;
    fn set_model(&mut self, i: usize, model: Value);
    fn set_context_used(&mut self, i: usize, used: Value);
    fn set_context_window(&mut self, i: usize, window: Value);
    fn tokens_processed(&self, i: usize) -> f64;
    fn add_output_tokens(&mut self, i: usize, n: f64);
    fn set_output_tokens(&mut self, i: usize, n: f64);
    fn add_tokens(&mut self, i: usize, n: f64, ts: f64, live: bool);
    /// One call at API prices on its day.
    fn add_call(&mut self, i: usize, model: &Value, call: &Call, ts: f64);
    /// A call at what its app said it cost, or without a price (None).
    fn add_reported(&mut self, i: usize, model: &Value, call: &Call, ts: f64, cost: Option<f64>);
    fn provider(&self, i: usize) -> Value;
    fn set_provider(&mut self, i: usize, provider: Value);
    fn priority(&self, i: usize) -> bool;
    fn set_priority(&mut self, i: usize, priority: bool);

    // ----- texts, edits, results -----

    fn add_prompt(&mut self, i: usize, text: &str, ts: f64);
    /// The sequence number of the last prompt kept, which tells it from
    /// the next.
    fn last_prompt_seq(&self, i: usize) -> Option<u64>;
    fn add_message(&mut self, i: usize, text: Option<&Value>, ts: f64) -> Result<(), TypeError>;
    fn add_edits(&mut self, i: usize, edits: &[Value], ts: f64) -> Result<(), TypeError>;
    /// A tool's result; `tool`, the call's record when it had one.
    fn note_tool_result(&mut self, i: usize, ok: bool, ts: f64, tool: Option<&ToolRecord>);
    /// What the session asks you ({ callId, text, ts }), or null.
    fn set_question(&mut self, i: usize, question: Value);
    fn needs_you(&self, i: usize, now: f64) -> Value;

    // ----- records kept alike -----

    /// A subagent's record, by its id (None for one the line left out),
    /// when it has one.
    fn subagent(&mut self, i: usize, agent: Option<&Value>) -> Option<&mut Map<String, Value>>;
    fn add_subagent(&mut self, i: usize, agent: Option<&Value>, record: Map<String, Value>);
    /// A tool call in flight, by its call id (or none): its tag.
    fn inflight_add(&mut self, i: usize, id: Option<&Value>, name: Option<&Value>, ts: f64) -> String;
    /// The record the stall check keeps of a call in flight.
    fn inflight_set_tool(&mut self, i: usize, tag: &str, tool: Option<ToolRecord>);
    /// A call in flight ends; its record, when it had one.
    fn inflight_remove(&mut self, i: usize, id: Option<&Value>) -> Option<ToolRecord>;
    /// Whether a message's usage was counted already.
    fn was_seen(&self, i: usize, message_id: Option<&Value>) -> bool;
    fn mark_seen(&mut self, i: usize, message_id: Option<&Value>);
    /// A label waiting for a session's next subagent: (session, label, ts).
    fn pending_label(&self) -> Option<&(String, Value, f64)>;
    fn set_pending_label(&mut self, session: String, label: Value, ts: f64);
}

/// An app whose transcripts the live tailer follows: what its files are,
/// how its lines are sampled, and what each line means. One is made for
/// each telemetry (`Platform::tailed`), and keeps what is the app's own.
pub trait Tailed: Send + Sync {
    /// Its id in the registry.
    fn id(&self) -> &'static str;
    /// The vendor source its lines are checked against.
    fn source(&self) -> &'static str;
    /// Its normalizer's name as a block ("claude-lines").
    fn block(&self) -> &'static str;
    /// The version a line says its app is, or `current`, the file's last.
    fn version_of(&self, raw: &Value, current: Option<String>) -> Option<String>;
    /// The kind of line, for sampling one of each.
    fn kind_of(&self, raw: &Value) -> String;
    /// What a transcript is, by its path (with / for \), under the app's
    /// `root` when the file is in one of its folders: the session's own id
    /// (and its subagent's), or None for a file that isn't a session's.
    fn classify(&self, norm: &str, root: Option<&str>) -> Option<Classified>;
    /// A line of a session's transcript, as its events on the telemetry.
    #[expect(clippy::too_many_arguments, reason = "a line, where it was read, and how, as the telemetry takes them")]
    fn line(&mut self, t: &mut dyn Tail, vendor: &dyn VendorCheck, at: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError>;
    /// Its plan limits for the snapshot (`limits.<id>`), None for an app
    /// without.
    fn limits(&self, _now: f64) -> Option<Value> {
        None
    }
    /// Its limits' forecast: each window's projection, by name.
    fn forecast(&self, _now: f64) -> Vec<(String, Value)> {
        Vec::new()
    }
    /// Its provider was asked for plan usage and answered `r`.
    fn usage_reading(&mut self, _r: Option<&Value>, _now: f64) -> Result<(), TypeError> {
        Ok(())
    }
    /// Samples another source keeps of its plan usage, as its file has
    /// them now.
    fn usage_samples(&mut self, _samples: &Value) {}
    /// The readings settled: what the limits are now.
    fn settle_usage(&mut self) {}
}

/// Where a native reader reads from: the map's folders and the service's
/// environment.
pub struct Paths<'a> {
    pub home_dir: &'a str,
    pub state_dir: &'a str,
    pub env: &'a Map<String, Value>,
}

impl Paths<'_> {
    /// A variable of the service's environment, when it's set.
    pub fn var(&self, k: &str) -> Option<String> {
        self.env.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string)
    }
}

/// An app with a reader of its own records: what the reader is on the
/// map, where it reads, and a new reader.
pub trait Native: Send + Sync {
    /// Its name as a source of the map (the app's id).
    fn source(&self) -> &'static str;
    /// Its block's name.
    fn block(&self) -> &'static str;
    /// What it reads, in words, for its status.
    fn what(&self) -> &'static str;
    /// The vendor schema its records are checked against.
    fn schema(&self) -> &'static str;
    /// The file or folder it reads.
    fn at(&self, paths: &Paths<'_>) -> String;
    /// A new reader.
    fn make(&self) -> Box<dyn native::NativeReader>;
}

/// Where an app's lister reads: the map's folders, the service's
/// environment and its runtime.
pub struct ListerEnv<'a> {
    pub home_dir: &'a str,
    /// The app's own folder, as the settings have it: Claude Code's
    /// projects folder, Codex's home, Grok Build's home.
    pub root: &'a str,
    /// Claude desktop's folder, when it was found.
    pub desktop_dir: Option<&'a str>,
    pub state_dir: &'a str,
    /// The service's environment.
    pub env: &'a Map<String, Value>,
    /// XenoNet's version, for a client to say who it is.
    pub version: &'a str,
    /// The service's runtime, for a lister that keeps a client running.
    pub handle: tokio::runtime::Handle,
}

impl ListerEnv<'_> {
    /// A variable of the service's environment, when it's set.
    pub fn var(&self, k: &str) -> Option<String> {
        self.env.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string)
    }
}

/// A request of an app's own server that failed: its message, and the
/// server's error when it gave one.
#[derive(Clone, Debug, PartialEq)]
pub struct Failed {
    pub message: String,
    pub rpc: Option<Value>,
}

impl Failed {
    pub fn said(m: impl Into<String>) -> Failed {
        Failed { message: m.into(), rpc: None }
    }
}

/// Where a renamed session's name went: the places it was saved, as the
/// console names them, and, for an app whose desktop keeps the name too,
/// whether it was written there or why not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Renamed {
    pub saved: Vec<&'static str>,
    pub desktop: Option<Result<(), String>>,
}

/// A lister's block, for an app with a client XenoNet keeps running: its
/// name, its kind and its lifecycle now.
#[derive(Clone, Debug)]
pub struct BlockHealth {
    pub name: &'static str,
    pub kind: &'static str,
    pub life: Lifecycle,
}

/// An app whose sessions the map lists and whose transcripts it shows.
/// One is made for each map (`Platform::lister`), and keeps what is the
/// app's own: a client it runs, what its last listing read. Every method
/// blocks: the map calls them off its runtime's threads.
pub trait Lister: Send + Sync {
    /// Its sessions, as the map lists them (session/v1), checked through
    /// `vendor`.
    fn sessions(&self, vendor: &dyn VendorCheck) -> Result<Vec<Value>, String>;
    /// A page of a session's transcript ({ entries, cursor, total }),
    /// newest first; `cursor` continues further back. `files` finds the
    /// session's transcript files as the map knows them (its own first),
    /// for a reader that falls back to them.
    fn transcript(&self, vendor: &dyn VendorCheck, raw_id: &str, cursor: Option<&str>, limit: f64, files: &dyn Fn() -> Vec<String>) -> Result<Value, String>;
    /// Renames a session where its app keeps the name; `node` is the
    /// session as the map has it.
    fn rename(&self, _vendor: &dyn VendorCheck, _node: &Value, _title: &str) -> Result<Renamed, String> {
        Err("XenoNet can't rename this app's sessions".into())
    }
    /// The app's own projects (Codex desktop's), for an app that keeps
    /// some.
    fn projects(&self, _vendor: &dyn VendorCheck) -> Option<Vec<Value>> {
        None
    }
    /// Its block, for an app with a client XenoNet keeps running.
    fn block(&self) -> Option<BlockHealth> {
        None
    }
    /// A request of the app's own server, for an app with one.
    fn request(&self, _vendor: &dyn VendorCheck, _method: &str, _params: Value) -> Result<Value, Failed> {
        Err(Failed::said("XenoNet has no server of this app's to ask"))
    }
    /// Stops what it keeps running.
    fn stop(&self) {}
}
