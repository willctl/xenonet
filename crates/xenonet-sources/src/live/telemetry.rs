//! core/live.js's LiveTelemetry: each session's state from its transcripts'
//! lines, and what's made of it (the snapshot, the signals the UI acts on,
//! forecasts, costs, collisions, the history).
//!
//! Reading the files is the tailer's (xenonet-blocks), and the events go
//! out through whoever runs this: each line goes in, and the events it
//! makes come back, in order (`Emit`). Time is an argument (`now`) wherever
//! live.js asked Date.now().
//!
//! Records are read as JS read them: a field JS would read as undefined is
//! missing, a Map's keys are told apart as SameValueZero tells them
//! (`key_tag`), and where JS would throw on an odd record this gives the
//! TypeError lines.rs gives, after making the changes JS made before it
//! threw. Arithmetic on a record's counts is a number's (Number() of what
//! the record says), where JS's `+` would have joined strings.

use super::{
    categorize, claude_edits, clip, clip_value, codex_edits, command_text, day_key, file_key, files_of, looks_like_question, mcp_server_of, median, patch_files, slope, string_of,
    tail, tool_detail, tool_key, wiki_pages_of,
};
use crate::lines::{is_codex_prompt, opt_prop, prop, ClaudeLine, CodexLine, TypeError};
use crate::refusals::detect::{claude_signal, codex_signal};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use xenonet_base::json::{number, number_string};
use xenonet_base::ordered::OrderedMap;
use xenonet_base::text::{join, js_number, js_string, js_trim, truthy};
use xenonet_base::usage::{Speed, TokenUsage};
use xenonet_base::{apps, jsstr, paths, pricing};

pub const PRIME_WINDOW_MS: f64 = 24.0 * 3600e3;
const RATE_WINDOW_S: f64 = 300.0;
const FILE_MEMORY_MS: f64 = 20.0 * 60e3;
const COLLISION_WINDOW_MS: f64 = 10.0 * 60e3;
const SUBAGENT_IDLE_MS: f64 = 45e3;
const TURN_STALE_MS: f64 = 10.0 * 60e3;
const SILENT_MS: f64 = 4.0 * 60e3;
const QUESTION_TTL_MS: f64 = 12.0 * 3600e3;
const REVIEW_TTL_MS: f64 = 45.0 * 60e3;
const HOOK_NEED_MS: f64 = 30.0 * 60e3;
pub const HISTORY_MAX: usize = 20000;
// Claude plan usage is asked of Anthropic every 5 minutes (claude_usage);
// a reading this old says nothing about the windows now.
const CLAUDE_LIMITS_FRESH_MS: f64 = 45.0 * 60e3;

// ---------- JS's ways ----------

/// An object's fields as JS has them, in order: a field that's there with
/// an undefined value is None (an event's `agentId` from Codex, say).
pub type Fields = Vec<(String, Option<Value>)>;

static OBJECT_KEYS: AtomicU64 = AtomicU64::new(0);

/// A JS Map's key, as SameValueZero tells keys apart: by value for what
/// isn't an object, and an object only as itself, so one read from a line
/// matches no other.
pub fn key_tag(k: Option<&Value>) -> String {
    match k {
        None => "u".into(),
        Some(Value::Null) => "z".into(),
        Some(Value::Bool(true)) => "t".into(),
        Some(Value::Bool(false)) => "f".into(),
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            format!("n{}", number_string(if f == 0.0 { 0.0 } else { f }))
        }
        Some(Value::String(s)) => format!("s{s}"),
        Some(_) => format!("o{}", OBJECT_KEYS.fetch_add(1, AtomicOrdering::Relaxed)),
    }
}

fn str_tag(s: &str) -> String {
    format!("s{s}")
}

/// `Math.max(a, b)`: NaN when either is.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `v ?? fallback`: the value unless it's null or undefined.
fn nn(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

/// A value in a template literal: `${v}`.
fn tpl(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => js_string(v),
    }
}

fn is_str(v: Option<&Value>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

/// `a === b`, for what a record holds: an object or a list is only itself,
/// which two values read apart never are.
fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// `x?.[0]`: a list's first item, a string's first unit, an object's "0".
fn index0(v: Option<&Value>) -> Option<Value> {
    match v {
        Some(Value::Array(a)) => a.first().cloned(),
        Some(Value::String(s)) => s.encode_utf16().next().map(|u| Value::String(String::from_utf16_lossy(&[u]))),
        Some(Value::Object(m)) => m.get("0").cloned(),
        _ => None,
    }
}

/// `Object.keys(v ?? {})`.
fn object_keys(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Object(m)) => jsstr::js_key_order(m.clone()).keys().cloned().collect(),
        Some(Value::Array(a)) => (0..a.len()).map(|i| i.to_string()).collect(),
        Some(Value::String(s)) => (0..jsstr::len(s)).map(|i| i.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn obj(fields: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in fields {
        m.insert(k.into(), v);
    }
    Value::Object(m)
}

/// A JS object built in order, with `Some` for each field that's there.
fn obj_opt(fields: Vec<(&str, Option<Value>)>) -> Value {
    Value::Object(map_opt(fields))
}

/// obj_opt's fields, as a map.
fn map_opt(fields: Vec<(&str, Option<Value>)>) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in fields {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    }
    m
}

fn field<'a>(f: &'a Fields, key: &str) -> Option<&'a Value> {
    f.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.as_ref())
}

/// A field set as an object literal sets it: in its place when it's
/// there, else last.
fn put(f: &mut Fields, key: &str, v: Option<Value>) {
    match f.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = v,
        None => f.push((key.to_string(), v)),
    }
}

/// `pages ? { pages } : undefined`.
fn pages_extra(pages: Option<Vec<String>>) -> Option<Map<String, Value>> {
    pages.map(|p| {
        let mut m = Map::new();
        m.insert("pages".into(), Value::Array(p.into_iter().map(Value::String).collect()));
        m
    })
}

fn ev(fields: Vec<(&str, Option<Value>)>) -> Fields {
    fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

#[expect(clippy::unnecessary_wraps, reason = "a field of obj_opt or ev, which takes each as an Option, that's always there")]
fn sv(v: &str) -> Option<Value> {
    Some(Value::String(v.into()))
}

#[expect(clippy::unnecessary_wraps, reason = "a field of obj_opt or ev, which takes each as an Option, that's always there")]
fn nv(v: f64) -> Option<Value> {
    Some(number(v))
}

/// A stable sort by a comparator that gives a number, as JS's sort takes
/// one: negative puts `a` first, and NaN counts as 0. (Rust's own sort may
/// panic on an order that isn't total.)
fn sort_by_num<T: Clone>(v: &mut Vec<T>, cmp: impl Fn(&T, &T) -> f64) {
    merge_sort(v, &cmp);
}

fn merge_sort<T: Clone>(v: &mut Vec<T>, cmp: &dyn Fn(&T, &T) -> f64) {
    if v.len() < 2 {
        return;
    }
    let mid = v.len() / 2;
    let mut right = v.split_off(mid);
    merge_sort(v, cmp);
    merge_sort(&mut right, cmp);
    let left = std::mem::take(v);
    let (mut i, mut j) = (0, 0);
    while i < left.len() && j < right.len() {
        // The right one goes first only when it's strictly before.
        if cmp(&left[i], &right[j]) > 0.0 {
            v.push(right[j].clone());
            j += 1;
        } else {
            v.push(left[i].clone());
            i += 1;
        }
    }
    v.extend_from_slice(&left[i..]);
    v.extend_from_slice(&right[j..]);
}

fn num_of(v: Option<&Value>, key: &str) -> f64 {
    js_number(opt_prop(v, key))
}

/// `(v ?? 0)` as a number.
fn or0(v: Option<&Value>) -> f64 {
    nn(v).map_or(0.0, |v| js_number(Some(v)))
}

// ---------- a session's state ----------

/// A JS Map: by key tag, with the key itself.
type KMap<V> = OrderedMap<(Option<Value>, V)>;

fn kset<V>(m: &mut KMap<V>, key: Option<&Value>, v: V) -> String {
    let tag = key_tag(key);
    m.insert(&tag, (key.cloned(), v));
    tag
}

fn sset<V>(m: &mut KMap<V>, key: &str, v: V) {
    m.insert(&str_tag(key), (Some(Value::String(key.into())), v));
}

/// Takes out the oldest key, as JS's `m.delete(m.keys().next().value)`.
fn drop_first<V>(m: &mut KMap<V>) {
    let first = m.keys().next().map(String::from);
    if let Some(first) = first {
        m.remove(&first);
    }
}

#[derive(Clone, Debug, Default)]
struct Spend {
    cost: f64,
    tokens: f64,
    unpriced: Vec<String>,
}

/// A call in flight: its fields (name, ts) and the tool record of it the
/// stall check keeps, when there is one (`None`: no `tool` field; `Some(None)`:
/// null), with a copy of that record for when it's no longer among the
/// recent ones.
#[derive(Clone, Debug)]
struct Inflight {
    fields: Map<String, Value>,
    #[expect(clippy::option_option, reason = "the JS field it stands for can be missing, null or a record, which the snapshot keeps apart")]
    tool: Option<Option<(u64, Map<String, Value>)>>,
}

#[derive(Clone, Debug)]
struct Prompt {
    seq: u64,
    text: Value,
    ts: Value,
}

impl Prompt {
    fn value(&self) -> Value {
        obj(vec![("text", self.text.clone()), ("ts", self.ts.clone())])
    }
}

#[derive(Clone, Debug)]
struct Message {
    text: Value,
    ts: Value,
}

/// A recent tool call, for the stall check: { key, label, loopable, ok, ts }.
#[derive(Clone, Debug)]
struct ToolRec {
    uid: u64,
    rec: Map<String, Value>,
}

/// A call's tokens and how it ran, for its price.
#[derive(Clone, Copy, Debug, Default)]
pub struct Call {
    pub tokens: TokenUsage,
}

/// A tool call, as noteTool takes it. `agent` is None where JS passed no
/// agentId (undefined), else the agent's id or null.
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

/// One session's state, as the tailer has seen it.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub app: String,
    tokens_processed: f64,
    tokens_output: f64,
    daily: KMap<f64>,
    spend: KMap<Spend>,
    provider: Value,
    priority: bool,
    context_used: Value,
    context_window: Value,
    turn_contexts: Vec<f64>,
    model: Value,
    turn_active: bool,
    turn_started_at: Option<f64>,
    inflight: KMap<Inflight>,
    tool_counts: Map<String, Value>,
    last_tool: Option<Value>,
    subagents: KMap<Map<String, Value>>,
    files: KMap<Value>,
    edits: KMap<Vec<Value>>,
    mcp: KMap<Value>,
    seen_messages: KMap<()>,
    rate: Vec<(f64, f64)>,
    pub last_event_at: f64,
    prompts: Vec<Prompt>,
    first_prompt: Option<Prompt>,
    messages: Vec<Message>,
    last_prompt_at: f64,
    last_turn_end_at: f64,
    interrupted_at: f64,
    codex_items: bool,
    event_prompt: Option<(Prompt, f64)>,
    turn_marks: f64,
    question: Value,
    hook_need: Value,
    recent_tools: Vec<ToolRec>,
    error_streak: f64,
    // Undefined (None) until the first reset.
    error_since: Option<f64>,
    last_error_at: Option<f64>,
    last_result_at: Option<f64>,
    seq: u64,
    // Bumped when its prompts or messages change, for the twin's copies.
    texts: u64,
}

impl Session {
    pub fn new(id: &str, app: &str) -> Self {
        Session {
            id: id.into(),
            app: app.into(),
            tokens_processed: 0.0,
            tokens_output: 0.0,
            daily: KMap::new(),
            spend: KMap::new(),
            provider: Value::Null,
            priority: false,
            context_used: number(0.0),
            // The app's context window until a line says otherwise; any app
            // but Codex has Claude Code's.
            context_window: number(apps::app(apps::tailed_app(app, "claude")).and_then(|a| a.context).unwrap_or_default()),
            turn_contexts: Vec::new(),
            model: Value::Null,
            turn_active: false,
            turn_started_at: None,
            inflight: KMap::new(),
            tool_counts: Map::new(),
            last_tool: None,
            subagents: KMap::new(),
            files: KMap::new(),
            edits: KMap::new(),
            mcp: KMap::new(),
            seen_messages: KMap::new(),
            rate: Vec::new(),
            last_event_at: 0.0,
            prompts: Vec::new(),
            first_prompt: None,
            messages: Vec::new(),
            last_prompt_at: 0.0,
            last_turn_end_at: 0.0,
            interrupted_at: 0.0,
            codex_items: false,
            event_prompt: None,
            turn_marks: 0.0,
            question: Value::Null,
            hook_need: Value::Null,
            recent_tools: Vec::new(),
            error_streak: 0.0,
            error_since: None,
            last_error_at: None,
            last_result_at: None,
            seq: 0,
            texts: 0,
        }
    }

    pub fn add_tokens(&mut self, n: f64, ts: f64, live: bool) {
        if n.is_nan() || n <= 0.0 {
            return;
        }
        self.tokens_processed += n;
        let day = day_key(ts);
        let had = self.daily.get(&str_tag(&day)).map_or(0.0, |(_, v)| *v);
        sset(&mut self.daily, &day, had + n);
        if live {
            let sec = (ts / 1000.0).floor();
            match self.rate.last_mut() {
                Some(last) if last.0 == sec => last.1 += n,
                _ => self.rate.push((sec, n)),
            }
            let cutoff = sec - RATE_WINDOW_S;
            while self.rate.first().is_some_and(|r| r.0 < cutoff) {
                self.rate.remove(0);
            }
        }
    }

    /// One call at API prices on its day, as the stats panel prices it.
    /// Two days are kept.
    pub fn add_call(&mut self, model: &Value, call: &Call, ts: f64) {
        let day = day_key(ts);
        let tag = str_tag(&day);
        if !self.spend.contains(&tag) {
            sset(&mut self.spend, &day, Spend::default());
            while self.spend.len() > 2 {
                drop_first(&mut self.spend);
            }
        }
        let name = string_of(Some(model));
        let parts = pricing::cost_parts(&name, Some(ts), &call.tokens);
        let d = &mut self.spend.get_mut(&tag).expect("added above when missing, and the newest stays").1;
        match parts {
            None => {
                let plain = pricing::plain_model(&name);
                let unpriced = if !plain.is_empty() {
                    plain
                } else if truthy(Some(model)) {
                    js_string(model)
                } else {
                    "unknown".into()
                };
                if !d.unpriced.contains(&unpriced) {
                    d.unpriced.push(unpriced);
                }
            }
            Some(c) => {
                d.cost += c.total;
                d.tokens += call.tokens.total();
            }
        }
    }

    pub fn add_prompt(&mut self, text: &str, ts: f64) {
        self.reset_stall();
        self.seq += 1;
        let p = Prompt { seq: self.seq, text: Value::String(tail(text, 1200)), ts: number(ts) };
        if self.first_prompt.is_none() {
            self.first_prompt = Some(p.clone());
        }
        self.prompts.push(p);
        self.texts += 1;
        if self.prompts.len() > 4 {
            self.prompts.remove(0);
        }
        self.last_prompt_at = ts;
        self.interrupted_at = 0.0;
        self.question = Value::Null;
        self.hook_need = Value::Null;
    }

    /// A reply's words, unless blank; what isn't text (a number, say) is
    /// the TypeError JS's `text?.trim()` throws.
    pub fn add_message(&mut self, text: Option<&Value>, ts: f64) -> Result<(), TypeError> {
        let t = match text {
            None | Some(Value::Null) => return Ok(()),
            Some(Value::String(t)) => t,
            Some(_) => return Err(TypeError("text?.trim is not a function".into())),
        };
        if js_trim(t).is_empty() {
            return Ok(());
        }
        self.messages.push(Message { text: Value::String(tail(t, 1500)), ts: number(ts) });
        self.texts += 1;
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
        Ok(())
    }

    /// A turn's end; `interrupted` when you stopped it.
    pub fn end_turn(&mut self, ts: f64, interrupted: bool) {
        self.reset_stall();
        self.turn_marks += 1.0;
        self.turn_active = false;
        self.inflight.clear();
        if ts >= self.last_turn_end_at {
            self.interrupted_at = if interrupted { ts } else { 0.0 };
        }
        self.last_turn_end_at = js_max(self.last_turn_end_at, ts);
        if truthy(Some(&self.context_used)) {
            self.turn_contexts.push(js_number(Some(&self.context_used)));
            if self.turn_contexts.len() > 12 {
                self.turn_contexts.remove(0);
            }
        }
    }

    /// Edits as small diffs, by file: the last 3 of each of the last 40
    /// files.
    pub fn add_edits(&mut self, edits: &[Value], ts: f64) -> Result<(), TypeError> {
        for e in edits {
            let path = prop(Some(e), "path")?;
            let k = file_key(&tpl(path));
            let tag = str_tag(&k);
            let mut list = self.edits.remove(&tag).map(|(_, l)| l).unwrap_or_default();
            let mut m = match e {
                Value::Object(m) => m.clone(),
                _ => Map::new(),
            };
            m.insert("ts".into(), number(ts));
            list.push(Value::Object(m));
            if list.len() > 3 {
                list.remove(0);
            }
            sset(&mut self.edits, &k, list);
            if self.edits.len() > 40 {
                drop_first(&mut self.edits);
            }
        }
        Ok(())
    }

    pub fn reset_stall(&mut self) {
        self.recent_tools.clear();
        self.error_streak = 0.0;
        self.error_since = Some(0.0);
        self.last_error_at = Some(0.0);
        self.last_result_at = Some(0.0);
    }

    /// A tool's result. `tool` is the call's record when the call had one:
    /// a duplicate result isn't another failure.
    fn note_tool_result(&mut self, ok: bool, ts: f64, tool: Option<&(u64, Map<String, Value>)>) {
        if let Some((uid, copy)) = tool {
            let recent = self.recent_tools.iter_mut().find(|t| t.uid == *uid);
            let had = match &recent {
                Some(t) => t.rec.get("ok").cloned(),
                None => copy.get("ok").cloned(),
            };
            if !matches!(had, Some(Value::Null)) {
                return;
            }
            if let Some(t) = recent {
                t.rec.insert("ok".into(), Value::Bool(ok));
            }
        }
        if ts < self.last_result_at.unwrap_or(0.0) {
            return;
        }
        self.last_result_at = Some(ts);
        if !ok && (self.error_streak == 0.0 || self.error_streak.is_nan()) {
            self.error_since = Some(ts);
        }
        self.error_streak = if ok { 0.0 } else { self.error_streak + 1.0 };
        if ok {
            self.error_since = Some(0.0);
        }
        if !ok {
            self.last_error_at = Some(ts);
        }
    }

    /// A tool's result with no record of its call (as Codex's are).
    pub fn note_result(&mut self, ok: bool, ts: f64) {
        self.note_tool_result(ok, ts, None);
    }

    pub fn tokens_per_minute(&self, now: f64) -> f64 {
        let cutoff = (now / 1000.0).floor() - 60.0;
        self.rate.iter().fold(0.0, |sum, (sec, n)| if *sec >= cutoff { sum + n } else { sum })
    }

    pub fn status(&self, now: f64) -> &'static str {
        if !self.inflight.is_empty() && now - self.last_event_at < TURN_STALE_MS {
            return "tool";
        }
        if self.turn_active && now - self.last_event_at < TURN_STALE_MS {
            return "thinking";
        }
        "idle"
    }

    fn question_pending(&self) -> bool {
        truthy(Some(&self.question)) && self.inflight.contains(&key_tag(opt_prop(Some(&self.question), "callId")))
    }

    fn hook_waiting(&self, now: f64) -> bool {
        truthy(Some(&self.hook_need)) && now - num_of(Some(&self.hook_need), "ts") < HOOK_NEED_MS
    }

    /// What it waits on you for: a permission, a question, or a turn to
    /// review; null for nothing.
    pub fn needs_you(&self, now: f64) -> Value {
        if self.hook_waiting(now) {
            let h = Some(&self.hook_need);
            return obj_opt(vec![("kind", sv("permission")), ("text", opt_prop(h, "text").cloned()), ("since", opt_prop(h, "ts").cloned())]);
        }
        if self.question_pending() {
            let q = Some(&self.question);
            return obj_opt(vec![("kind", sv("question")), ("text", opt_prop(q, "text").cloned()), ("since", opt_prop(q, "ts").cloned())]);
        }
        let ended = self.last_turn_end_at != 0.0 && !self.last_turn_end_at.is_nan();
        if !self.turn_active && ended && self.last_turn_end_at >= self.last_prompt_at {
            // A turn you stopped waits on you, but you know: there's
            // nothing in it to review, and what it said last asks you
            // nothing. The snapshot's interruptedAt says so instead.
            if self.interrupted_at != 0.0 && !self.interrupted_at.is_nan() {
                return Value::Null;
            }
            let last = self.messages.last().map_or_else(String::new, |m| string_of(Some(&m.text)));
            let age = now - self.last_turn_end_at;
            if looks_like_question(&last) && age < QUESTION_TTL_MS {
                return obj(vec![("kind", "question".into()), ("text", Value::String(tail(&last, 320))), ("since", number(self.last_turn_end_at))]);
            }
            if age < REVIEW_TTL_MS {
                return obj(vec![("kind", "review".into()), ("text", Value::String(tail(&last, 320))), ("since", number(self.last_turn_end_at))]);
            }
        }
        Value::Null
    }

    /// Why it looks stuck ({ reasons }), or null.
    pub fn stuck(&self, now: f64) -> Value {
        if !self.turn_active || self.question_pending() || self.hook_waiting(now) {
            return Value::Null;
        }
        let mut reasons: Vec<Value> = Vec::new();
        // Repetition alone is not a stall: searches, pagination and polling
        // often repeat successfully. A loop needs repeated completed
        // failures.
        let last = self.recent_tools.last();
        let mut run = 0;
        if let Some(lt) = last {
            for t in self.recent_tools.iter().rev() {
                let same = t.uid == lt.uid || strict_eq(t.rec.get("key"), lt.rec.get("key"));
                if !same || t.rec.get("ok") != Some(&Value::Bool(false)) || now - js_number(t.rec.get("ts")) > 10.0 * 60e3 {
                    break;
                }
                run += 1;
            }
        }
        let looped = last.filter(|lt| run >= 3 && truthy(lt.rec.get("loopable")) && self.error_streak >= 3.0);
        if let Some(lt) = looped {
            reasons.push(Value::String(format!("ran {} {run} times in a row", tpl(lt.rec.get("label")))));
        }
        let failed_loop = looped.is_some();
        // A burst of failed parallel calls can be one recoverable attempt.
        // Different calls need sustained failure before we ask for
        // attention.
        let last_error = self.last_error_at.unwrap_or(f64::NAN);
        let since = self.error_since.unwrap_or(f64::NAN);
        if self.error_streak >= 3.0 && now - last_error < 10.0 * 60e3 && (failed_loop || self.error_streak >= 5.0 && last_error - since >= 60e3) {
            reasons.push(Value::String(format!("{} failed tool calls in a row", number_string(self.error_streak))));
        }
        // Long builds and test runs are quiet by nature: a tool in flight
        // gets 15 minutes before silence counts, plain thinking gets 4.
        let quiet = now - self.last_event_at;
        let limit = if self.inflight.is_empty() { SILENT_MS } else { 15.0 * 60e3 };
        if self.turn_active && quiet > limit && quiet < TURN_STALE_MS.max(limit + 10.0 * 60e3) {
            reasons.push(Value::String(format!("silent for {} min", number_string(js_round(quiet / 60e3)))));
        }
        if reasons.is_empty() {
            Value::Null
        } else {
            obj(vec![("reasons", Value::Array(reasons))])
        }
    }

    /// The context's fill, its growth a turn, and the turns left before
    /// it's 90% full.
    pub fn forecast(&self) -> Value {
        let window = if truthy(Some(&self.context_window)) { js_number(Some(&self.context_window)) } else { 1.0 };
        let growths: Vec<f64> = self.turn_contexts.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
        let growth = median(&growths);
        let threshold = window * 0.9;
        let used = js_number(Some(&self.context_used));
        let turns_left = if growth > 0.0 { number(js_max(0.0, ((threshold - used) / growth).floor())) } else { Value::Null };
        let growth_value = if growth != 0.0 && !growth.is_nan() { number(growth) } else { Value::Null };
        obj(vec![("pct", number(used / window)), ("growthPerTurn", growth_value), ("turnsLeft", turns_left)])
    }

    fn tokens_value(&self) -> Value {
        obj(vec![("processed", number(self.tokens_processed)), ("output", number(self.tokens_output))])
    }

    fn context_value(&self) -> Value {
        obj(vec![("used", self.context_used.clone()), ("window", self.context_window.clone())])
    }

    /// What the snapshot says of it at `now`; files it touched over 20
    /// minutes ago are forgotten.
    pub fn snapshot(&mut self, now: f64) -> Value {
        self.files.retain(|_, (_, ts)| {
            let age = now - js_number(Some(ts));
            age.is_nan() || age <= FILE_MEMORY_MS
        });
        let mut subs: Vec<Value> = self
            .subagents
            .iter()
            .map(|(_, (_, a))| {
                let mut m: Map<String, Value> = a.iter().filter(|(k, _)| k.as_str() != "announced").map(|(k, v)| (k.clone(), v.clone())).collect();
                let active = !truthy(a.get("done")) && now - js_number(a.get("lastAt")) < SUBAGENT_IDLE_MS;
                m.insert("active".into(), Value::Bool(active));
                Value::Object(m)
            })
            .collect();
        sort_by_num(&mut subs, |a, b| js_number(b.get("lastAt")) - js_number(a.get("lastAt")));
        subs.truncate(12);
        let mut files: Vec<(Value, Value)> = self.files.iter().map(|(_, (k, ts))| (k.clone().unwrap_or(Value::Null), ts.clone())).collect();
        sort_by_num(&mut files, |a, b| js_number(Some(&b.1)) - js_number(Some(&a.1)));
        files.truncate(10);
        let files: Vec<Value> = files.into_iter().map(|(path, ts)| obj(vec![("path", path), ("ts", ts)])).collect();
        let mcp: Vec<Value> = self.mcp.iter().map(|(_, (k, ts))| obj(vec![("server", k.clone().unwrap_or(Value::Null)), ("ts", ts.clone())])).collect();
        let today = self.daily.get(&str_tag(&day_key(now))).map_or(0.0, |(_, v)| *v);
        let last_message = self.messages.last().map_or(Value::Null, |m| Value::String(tail(&string_of(Some(&m.text)), 280)));
        let or_null = |v: f64| if v != 0.0 && !v.is_nan() { number(v) } else { Value::Null };
        obj(vec![
            ("id", Value::String(self.id.clone())),
            ("app", Value::String(self.app.clone())),
            ("status", self.status(now).into()),
            ("tokens", self.tokens_value()),
            ("tokensToday", number(today)),
            ("tpm", number(self.tokens_per_minute(now))),
            ("context", self.context_value()),
            ("forecast", self.forecast()),
            ("model", self.model.clone()),
            ("turnStartedAt", if self.turn_active { self.turn_started_at.map_or(Value::Null, number) } else { Value::Null }),
            ("lastTool", self.last_tool.clone().unwrap_or(Value::Null)),
            ("toolCounts", Value::Object(self.tool_counts.clone())),
            ("subagents", Value::Array(subs)),
            ("files", Value::Array(files)),
            ("mcp", Value::Array(mcp)),
            ("needsYou", self.needs_you(now)),
            ("stuck", self.stuck(now)),
            ("lastMessage", last_message),
            ("lastTurnEndAt", or_null(self.last_turn_end_at)),
            ("interruptedAt", or_null(self.interrupted_at)),
            ("lastEventAt", number(self.last_event_at)),
        ])
    }

    fn new_uid(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}

// ---------- what it emits ----------

/// An event for the twin to emit, in the order they happened.
#[derive(Clone, Debug, PartialEq)]
pub enum Emit {
    /// "activity": what a session did.
    Activity(Fields),
    /// "tokens": { sessionId, app, n, ts }.
    Tokens(Value),
    /// "refusal": a provider's refusal, as its line marks it.
    Refusal(Value),
}

/// An event in the history, with its time.
#[derive(Clone, Debug)]
pub struct Event {
    pub ts: f64,
    pub fields: Fields,
}

/// Where a line was read: its session, its subagent (null for none) and
/// its file.
#[derive(Clone, Debug)]
pub struct Where {
    pub session_id: String,
    pub agent_id: Value,
    pub file: Value,
}

// ---------- the telemetry ----------

#[derive(Debug)]
pub struct Telemetry {
    pub home: Option<String>,
    roots: Vec<(String, String)>,
    sessions: Vec<Session>,
    index: HashMap<String, usize>,
    limits_codex: Value,
    limits_claude: Value,
    samples_codex: Vec<Value>,
    samples_claude: Vec<Value>,
    claude_reason: Value,
    anthropic_samples: Vec<Value>,
    desktop_samples: Vec<Value>,
    history: Vec<Event>,
    pending_label: Option<(String, Value, f64)>,
}

impl Telemetry {
    /// `roots`: [(dir, app)], the folders it reads, as real paths.
    pub fn new(home: Option<String>, roots: Vec<(String, String)>) -> Self {
        Telemetry {
            home,
            roots,
            sessions: Vec::new(),
            index: HashMap::new(),
            limits_codex: Value::Null,
            limits_claude: Value::Null,
            samples_codex: Vec::new(),
            samples_claude: Vec::new(),
            claude_reason: Value::Null,
            anthropic_samples: Vec::new(),
            desktop_samples: Vec::new(),
            history: Vec::new(),
            pending_label: None,
        }
    }

    // ----- sessions -----

    /// The session's index, made when it's new.
    pub fn session(&mut self, id: &str, app: &str) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        self.sessions.push(Session::new(id, app));
        self.index.insert(id.into(), self.sessions.len() - 1);
        self.sessions.len() - 1
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn ids(&self) -> Vec<String> {
        self.sessions.iter().map(|s| s.id.clone()).collect()
    }

    pub fn at(&self, i: usize) -> &Session {
        &self.sessions[i]
    }

    pub fn at_mut(&mut self, i: usize) -> &mut Session {
        &mut self.sessions[i]
    }

    // ----- discovery -----

    /// What a transcript is: { app, sessionId } (with agentId for a Claude
    /// Code subagent's), or None to leave it alone.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
    pub fn classify(&self, file: &str) -> Option<Value> {
        let norm = file.replace('\\', "/");
        if !norm.ends_with(".jsonl") {
            return None;
        }
        let codex_root = self.roots.iter().any(|(dir, app)| app == "codex" && norm.starts_with(&dir.replace('\\', "/")));
        if norm.contains("/.codex/") || codex_root {
            let id = uuid_before(&norm)?;
            return norm.contains("rollout-").then(|| obj(vec![("app", "codex".into()), ("sessionId", Value::String(format!("codex:{id}")))]));
        }
        if let Some((session, agent)) = subagent_file(&norm) {
            return Some(obj(vec![("app", "claude".into()), ("sessionId", Value::String(format!("claude:{session}"))), ("agentId", Value::String(agent))]));
        }
        let rest = norm.strip_suffix(".jsonl")?;
        let slash = rest.rfind('/')?;
        let id = &rest[slash + 1..];
        (id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
            .then(|| obj(vec![("app", "claude".into()), ("sessionId", Value::String(format!("claude:{id}")))]))
    }

    // ----- a line -----

    /// A line the tailer read, through its app's normalizer: its refusal
    /// (kept whether read as history or live, told to you only when live)
    /// and what it says. What it emitted before an error is in `out`.
    pub fn line(&mut self, codex: bool, f: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let sig = if codex { codex_signal(Some(&raw)) } else { claude_signal(Some(&raw)) };
        if let Some(signal) = sig {
            let model = self.find(&f.session_id).map_or(Value::Null, |i| self.sessions[i].model.clone());
            let cwd = opt_prop(Some(&raw), "cwd").filter(|c| c.is_string()).cloned().unwrap_or(Value::Null);
            out.push(Emit::Refusal(obj(vec![
                ("signal", signal),
                ("sessionId", Value::String(f.session_id.clone())),
                ("app", if codex { "codex" } else { "claude" }.into()),
                ("agentId", f.agent_id.clone()),
                ("file", f.file.clone()),
                ("cwd", cwd),
                ("model", model),
                ("live", Value::Bool(live)),
            ])));
        }
        if codex {
            self.apply_codex(f, raw, live, now, out)
        } else {
            self.apply_claude(f, raw, live, now, out)
        }
    }

    fn activity(&mut self, i: usize, ev: Fields, live: bool, now: f64, out: &mut Vec<Emit>) {
        let ts = field(&ev, "ts").map_or(f64::NAN, |v| js_number(Some(v)));
        let s = &mut self.sessions[i];
        s.last_event_at = js_max(s.last_event_at, ts);
        let mut full: Fields = vec![("sessionId".into(), Some(Value::String(s.id.clone()))), ("app".into(), Some(Value::String(s.app.clone())))];
        for (k, v) in ev {
            put(&mut full, &k, v);
        }
        if ts > now - PRIME_WINDOW_MS {
            // The history is in time order, which history_since and
            // trim_history rely on. At start it's sorted once, after reading
            // everything; after that an event that arrives late goes where
            // it belongs.
            if live {
                let at = self.index_of(ts, true);
                self.history.insert(at, Event { ts, fields: full.clone() });
            } else {
                self.history.push(Event { ts, fields: full.clone() });
            }
            if live && self.history.len() > HISTORY_MAX + 500 {
                self.trim_history(now);
            }
        }
        if live {
            out.push(Emit::Activity(full));
        }
    }

    /// An activity event a caller made: `ev` as JS gave it.
    pub fn note_activity(&mut self, i: usize, ev: Fields, live: bool, now: f64, out: &mut Vec<Emit>) {
        self.activity(i, ev, live, now, out);
    }

    /// How many events in the history come before `ts` (with `at`, before
    /// or at it).
    fn index_of(&self, ts: f64, at: bool) -> usize {
        let before = |e: &Event| e.ts < ts || (at && e.ts == ts);
        let h = &self.history;
        if h.is_empty() || before(&h[h.len() - 1]) {
            return h.len();
        }
        let (mut lo, mut hi) = (0, h.len());
        while lo < hi {
            let mid = usize::midpoint(lo, hi);
            if before(&h[mid]) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    pub fn history(&self) -> &[Event] {
        &self.history
    }

    pub fn history_since(&self, since: f64) -> &[Event] {
        &self.history[self.index_of(since, false)..]
    }

    /// History is read file by file, so it's put in time order once, at the
    /// end of the start.
    pub fn primed(&mut self, now: f64) {
        sort_by_num(&mut self.history, |a, b| a.ts - b.ts);
        self.trim_history(now);
    }

    pub fn trim_history(&mut self, now: f64) {
        let cutoff = now - PRIME_WINDOW_MS;
        let i = self.history.iter().take_while(|e| e.ts < cutoff).count();
        if i > 0 {
            self.history.drain(..i);
        }
        if self.history.len() > HISTORY_MAX {
            let extra = self.history.len() - HISTORY_MAX;
            self.history.drain(..extra);
        }
    }

    /// A tool call: counted, remembered for the stall check (but a
    /// subagent's), and told. Gives the record kept of it.
    pub fn note_tool(&mut self, i: usize, t: ToolCall, live: bool, now: f64, out: &mut Vec<Emit>) -> Option<(u64, Map<String, Value>)> {
        let name_s = string_of(t.name.as_ref());
        let category = categorize(&name_s);
        let s = &mut self.sessions[i];
        let count = s.tool_counts.get(category).map_or(0.0, |v| or0(Some(v)));
        s.tool_counts.insert(category.into(), number(count + 1.0));
        s.last_tool = Some(obj_opt(vec![("name", t.name.clone()), ("category", sv(category)), ("detail", Some(Value::String(t.detail.clone()))), ("ts", nv(t.ts))]));
        for f in &t.files {
            sset(&mut s.files, f, number(t.ts));
        }
        let srv = match nn(t.server.as_ref()) {
            Some(v) => v.clone(),
            None => mcp_server_of(&name_s).map_or(Value::Null, Value::String),
        };
        if truthy(Some(&srv)) {
            kset(&mut s.mcp, Some(&srv), number(t.ts));
        }
        let tool = if !truthy(t.agent.as_ref()) {
            let input = nn(t.full.as_ref()).cloned().unwrap_or_else(|| Value::String(t.detail.clone()));
            let label = if t.detail.is_empty() { clip_value(t.name.as_ref(), 48) } else { clip(&t.detail, 48) };
            let rec: Map<String, Value> = [
                ("key", Value::String(tool_key(&tpl(t.name.as_ref()), Some(&input)))),
                ("label", Value::String(label)),
                ("loopable", Value::Bool(matches!(category, "shell" | "mcp" | "web"))),
                ("ok", t.ok.clone()),
                ("ts", number(t.ts)),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
            let uid = s.new_uid();
            s.recent_tools.push(ToolRec { uid, rec: rec.clone() });
            Some((uid, rec))
        } else {
            None
        };
        if s.recent_tools.len() > 20 {
            s.recent_tools.remove(0);
        }
        let mut e = ev(vec![
            ("kind", sv("tool")),
            ("tool", t.name.clone()),
            ("category", sv(category)),
            ("detail", Some(Value::String(t.detail.clone()))),
            ("files", Some(Value::Array(t.files.iter().map(|f| Value::String(f.clone())).collect()))),
            ("server", Some(srv)),
            ("agentId", t.agent.clone()),
        ]);
        for (k, v) in t.extra.iter().flatten() {
            put(&mut e, k, Some(v.clone()));
        }
        put(&mut e, "ts", nv(t.ts));
        self.activity(i, e, live, now, out);
        tool
    }

    /// Claude Code's Notification hooks carry permission prompts the
    /// transcript doesn't.
    pub fn note_hook(&mut self, id: &str, body: Option<&Value>, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let i = self.session(id, "claude");
        let name = prop(body, "hook_event_name")?.cloned();
        let s = &mut self.sessions[i];
        if is_str(name.as_ref(), "Notification") {
            let text = nn(opt_prop(body, "message")).cloned().unwrap_or_else(|| "Claude needs your attention".into());
            s.hook_need = obj(vec![("text", Value::String(clip_value(Some(&text), 200))), ("ts", number(now))]);
        } else if name.as_ref().and_then(Value::as_str).is_some_and(|n| ["UserPromptSubmit", "Stop", "SessionEnd", "PostToolUse"].contains(&n)) {
            s.hook_need = Value::Null;
        }
        let detail = nn(name.as_ref()).cloned().unwrap_or_else(|| "hook".into());
        self.activity(i, ev(vec![("kind", sv("hook")), ("detail", Some(detail)), ("ts", nv(now))]), true, now, out);
        Ok(())
    }

    // ----- Claude Code transcript lines -----

    /// A Claude Code transcript's line, as the transcript has it.
    pub fn apply_claude(&mut self, f: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let o = ClaudeLine { raw };
        let i = self.session(&f.session_id, "claude");
        // A line that doesn't say when it was written was written when it's
        // read.
        let t = o.ts()?;
        let ts = if t != 0.0 && !t.is_nan() { t } else { now };
        let agent = f.agent_id.clone();
        let mut sub = None;
        if truthy(Some(&agent)) {
            let tag = key_tag(Some(&agent));
            let s = &mut self.sessions[i];
            let a = &mut s
                .subagents
                .get_or_insert_with(&tag, || {
                    let m: Map<String, Value> = [
                        ("id", agent.clone()),
                        ("label", "subagent".into()),
                        ("startedAt", number(ts)),
                        ("lastAt", number(ts)),
                        ("tools", number(0.0)),
                        ("done", Value::Bool(false)),
                        ("announced", Value::Bool(false)),
                    ]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect();
                    (Some(agent.clone()), m)
                })
                .1;
            let last = js_max(js_number(a.get("lastAt")), ts);
            a.insert("lastAt".into(), number(last));
            sub = Some(tag);
        }
        self.claude_line(i, sub.as_deref(), &agent, &o, ts, live, now, out)?;
        // Announce a subagent once its first line has given it a label.
        if let Some(tag) = sub {
            let a = &mut self.sessions[i].subagents.get_mut(&tag).expect("a subagent is added when the first line that names it is read").1;
            if !truthy(a.get("announced")) {
                a.insert("announced".into(), Value::Bool(true));
                let label = a.get("label").cloned();
                self.activity(i, ev(vec![("kind", sv("subagent-start")), ("agentId", Some(agent)), ("label", label), ("ts", nv(ts))]), live, now, out);
            }
        }
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "a line and what claude_lines worked out about it (session, subagent, time), handed down as the JS's closures saw them")]
    fn claude_line(&mut self, i: usize, sub: Option<&str>, agent: &Value, o: &ClaudeLine, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let kind = o.kind()?;
        if is_str(kind, "assistant") {
            self.claude_assistant(i, sub, agent, o, ts, live, now, out)
        } else if is_str(kind, "user") {
            self.claude_user(i, sub, agent, o, ts, live, now, out)
        } else {
            Ok(())
        }
    }

    // A reply: its tokens and context fill, its tool calls and text, and
    // the end of a turn (or of a subagent's work).
    #[expect(clippy::too_many_arguments, reason = "a line and what claude_lines worked out about it (session, subagent, time), handed down as the JS's closures saw them")]
    fn claude_assistant(&mut self, i: usize, sub: Option<&str>, agent: &Value, o: &ClaudeLine, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let has_agent = truthy(Some(agent));
        let model = o.model()?.cloned();
        if !has_agent {
            if let Some(m) = model.as_ref().filter(|m| truthy(Some(m))) {
                self.sessions[i].model = m.clone();
            }
        }
        // Each of a message's lines repeats its usage: counted once.
        let id = o.message_id()?.cloned();
        let seen = truthy(id.as_ref()) && self.sessions[i].seen_messages.contains(&key_tag(id.as_ref()));
        let u = if truthy(id.as_ref()) && !seen { o.usage()? } else { None };
        if let Some(u) = u {
            let s = &mut self.sessions[i];
            kset(&mut s.seen_messages, id.as_ref(), ());
            if s.seen_messages.len() > 400 {
                drop_first(&mut s.seen_messages);
            }
            let t = &u.tokens;
            let ctx = t.input.number() + t.cache_read.number() + js_number(Some(&u.cache_write));
            let output = t.output.number();
            s.tokens_output += output;
            s.add_tokens(ctx + output, ts, live);
            let call = Call { tokens: u.tokens.map(|f| f.priced()) };
            let call_model = nn(model.as_ref()).cloned().or_else(|| nn(Some(&s.model)).cloned()).unwrap_or_else(|| "unknown".into());
            s.add_call(&call_model, &call, ts);
            if !has_agent {
                s.context_used = number(ctx + output);
                let one_m = string_of(model.as_ref()).to_ascii_lowercase().contains("[1m]");
                if ctx + output > 200000.0 || one_m {
                    s.context_window = number(1_000_000.0);
                }
            }
            if live {
                out.push(Emit::Tokens(obj(vec![("sessionId", Value::String(s.id.clone())), ("app", "claude".into()), ("n", number(ctx + output)), ("ts", number(ts))])));
            }
        }
        let blocks = o.blocks()?.clone();
        let list: &[Value] = match &blocks {
            Value::Array(a) => a,
            // A string's characters have no type.
            Value::String(_) => &[],
            _ => return Err(TypeError("o.blocks is not iterable".into())),
        };
        for b in list {
            let t = prop(Some(b), "type")?;
            if is_str(t, "tool_use") {
                self.claude_tool_use(i, sub, agent, b, ts, live, now, out)?;
            } else if is_str(t, "text") {
                let text = prop(Some(b), "text")?;
                let said = match text {
                    None | Some(Value::Null) => false,
                    Some(Value::String(x)) => !js_trim(x).is_empty(),
                    Some(_) => return Err(TypeError("b.text?.trim is not a function".into())),
                };
                if said && !has_agent {
                    self.sessions[i].add_message(text, ts)?;
                    self.activity(i, ev(vec![("kind", sv("message")), ("detail", Some(Value::String(clip_value(text, 72)))), ("ts", nv(ts))]), live, now, out);
                }
            }
        }
        // A refusal ends the turn too: the provider said no, and nothing more
        // comes until you write again.
        let message = opt_prop(Some(&o.raw), "message");
        let refused = is_str(opt_prop(message, "stop_reason"), "refusal");
        if !o.end_turn()? && !refused {
            return Ok(());
        }
        if refused && !has_agent {
            let details = opt_prop(message, "stop_details");
            let why = nn(opt_prop(details, "explanation")).or(nn(opt_prop(details, "category"))).cloned().unwrap_or_else(|| "the provider refused".into());
            self.activity(i, ev(vec![("kind", sv("refusal")), ("detail", Some(Value::String(clip_value(Some(&why), 160)))), ("ts", nv(ts))]), live, now, out);
        }
        if let Some(tag) = sub {
            let a = &mut self.sessions[i].subagents.get_mut(tag).expect("a subagent is added when the first line that names it is read").1;
            a.insert("done".into(), Value::Bool(true));
            let label = a.get("label").cloned();
            self.activity(i, ev(vec![("kind", sv("subagent-stop")), ("agentId", Some(agent.clone())), ("label", label), ("ts", nv(ts))]), live, now, out);
        } else if self.sessions[i].turn_active {
            // A turn that began with a line read as written when it was read
            // can seem to end before it began: how long it took isn't known
            // then.
            let duration = self.duration(i, ts);
            self.sessions[i].end_turn(ts, false);
            self.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("ts", nv(ts))]), live, now, out);
            self.ask_if_question(i, ts, live, now, out);
        }
        Ok(())
    }

    fn duration(&self, i: usize, ts: f64) -> Value {
        match self.sessions[i].turn_started_at {
            Some(at) if at != 0.0 && !at.is_nan() && ts >= at => number(ts - at),
            _ => Value::Null,
        }
    }

    // After a turn's end: it asked you something.
    fn ask_if_question(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        let need = self.sessions[i].needs_you(now);
        if is_str(opt_prop(Some(&need), "kind"), "question") {
            let detail = clip_value(opt_prop(Some(&need), "text"), 120);
            self.activity(i, ev(vec![("kind", sv("needs-you")), ("detail", sv(&detail)), ("ts", nv(ts))]), live, now, out);
        }
    }

    #[expect(clippy::too_many_arguments, reason = "a line and what claude_lines worked out about it (session, subagent, time), handed down as the JS's closures saw them")]
    fn claude_tool_use(&mut self, i: usize, sub: Option<&str>, agent: &Value, b: &Value, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let bid = opt_prop(Some(b), "id").cloned();
        let name = opt_prop(Some(b), "name").cloned();
        let input = opt_prop(Some(b), "input").cloned();
        let s = &mut self.sessions[i];
        let mut fields = Map::new();
        if let Some(n) = &name {
            fields.insert("name".into(), n.clone());
        }
        fields.insert("ts".into(), number(ts));
        let itag = kset(&mut s.inflight, bid.as_ref(), Inflight { fields, tool: None });
        if let Some(tag) = sub {
            let a = &mut s.subagents.get_mut(tag).expect("a subagent is added when the first line that names it is read").1;
            let tools = js_number(a.get("tools")) + 1.0;
            a.insert("tools".into(), number(tools));
        }
        let name_s = string_of(name.as_ref());
        let server = mcp_server_of(&name_s);
        let pages = wiki_pages_of(server.as_deref(), input.as_ref());
        let full = match opt_prop(input.as_ref(), "command") {
            Some(c @ Value::String(_)) => c.clone(),
            _ => nn(input.as_ref()).cloned().unwrap_or_else(|| Value::Object(Map::new())),
        };
        let call = ToolCall {
            name: name.clone(),
            detail: tool_detail(input.as_ref(), self.home.as_deref()),
            full: Some(full),
            files: files_of(input.as_ref()),
            ts,
            agent: Some(agent.clone()),
            server: None,
            extra: pages_extra(pages),
            ok: Value::Null,
        };
        let tool = self.note_tool(i, call, live, now, out);
        let s = &mut self.sessions[i];
        if let Some((_, e)) = s.inflight.get_mut(&itag) {
            e.tool = Some(tool);
        }
        let edit_name = name.as_ref().and_then(Value::as_str).unwrap_or_default();
        let edits = claude_edits(edit_name, input.as_ref())?;
        if !edits.is_empty() {
            s.add_edits(&edits, ts)?;
        }
        let description = opt_prop(input.as_ref(), "description");
        if categorize(&name_s) == "agent" && truthy(description) {
            self.pending_label = Some((s.id.clone(), Value::String(clip_value(description, 40)), ts));
        }
        let asks = name.as_ref().and_then(Value::as_str).filter(|n| *n == "AskUserQuestion" || *n == "ExitPlanMode");
        if !truthy(Some(agent)) && asks.is_some() {
            let q = if asks == Some("ExitPlanMode") {
                Value::String("Plan ready for your approval".into())
            } else {
                let first = index0(opt_prop(input.as_ref(), "questions"));
                nn(opt_prop(first.as_ref(), "question")).or(nn(opt_prop(input.as_ref(), "question"))).cloned().unwrap_or_else(|| "Claude asked a question".into())
            };
            s.question = obj_opt(vec![("callId", bid), ("text", Some(Value::String(clip_value(Some(&q), 300)))), ("ts", nv(ts))]);
            self.activity(i, ev(vec![("kind", sv("needs-you")), ("detail", Some(Value::String(clip_value(Some(&q), 120)))), ("ts", nv(ts))]), live, now, out);
        }
        Ok(())
    }

    // Tool results, a prompt (in a subagent's file, its task), or your
    // interruption.
    #[expect(clippy::too_many_arguments, reason = "a line and what claude_lines worked out about it (session, subagent, time), handed down as the JS's closures saw them")]
    fn claude_user(&mut self, i: usize, sub: Option<&str>, agent: &Value, o: &ClaudeLine, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let has_agent = truthy(Some(agent));
        let results = o.results()?;
        if let Some(rs) = &results {
            for r in rs {
                let s = &mut self.sessions[i];
                let tag = key_tag(r.id.as_ref());
                let tool = s.inflight.remove(&tag).and_then(|(_, e)| e.tool).flatten();
                if !has_agent {
                    s.note_tool_result(!r.error, ts, tool.as_ref());
                }
                if r.error {
                    self.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts)), ("agentId", Some(agent.clone()))]), live, now, out);
                }
            }
            let s = &mut self.sessions[i];
            s.last_event_at = js_max(s.last_event_at, ts);
        }
        if truthy(o.meta()?) || truthy(o.sidechain()?) != has_agent {
            return Ok(());
        }
        // A compacted summary and an interruption aren't prompts; an
        // interruption ends the turn you stopped.
        if o.interruption()? {
            if !has_agent {
                self.interrupted(i, ts, live, now, out);
            }
            return Ok(());
        }
        if results.is_some() || truthy(o.compact_summary()?) {
            return Ok(());
        }
        let text = o.text()?;
        if js_trim(&text).is_empty() || text.starts_with('<') {
            return Ok(());
        }
        if let Some(tag) = sub {
            // A subagent's first prompt is its task; the Agent tool's
            // description is a better label.
            let id = self.sessions[i].id.clone();
            let pending = self.pending_label.clone();
            let a = &mut self.sessions[i].subagents.get_mut(tag).expect("a subagent is added when the first line that names it is read").1;
            if is_str(a.get("label"), "subagent") {
                let label = match pending {
                    Some((session, label, at)) if session == id && (ts - at).abs() < 60000.0 => label,
                    _ => Value::String(clip(&text, 40)),
                };
                a.insert("label".into(), label);
            }
            return Ok(());
        }
        let s = &mut self.sessions[i];
        s.turn_active = true;
        s.turn_started_at = Some(ts);
        s.add_prompt(&text, ts);
        self.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(&text, 90)))), ("ts", nv(ts))]), live, now, out);
        Ok(())
    }

    // You stopped the turn under way: the agent waits for you, so the turn
    // ends, and says it was stopped.
    fn interrupted(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        if !self.sessions[i].turn_active {
            return;
        }
        let duration = self.duration(i, ts);
        self.sessions[i].end_turn(ts, true);
        self.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("interrupted", Some(Value::Bool(true))), ("ts", nv(ts))]), live, now, out);
    }

    // ----- Codex rollout lines -----

    /// A Codex rollout's line, as the rollout has it.
    pub fn apply_codex(&mut self, f: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let o = CodexLine { raw };
        let i = self.session(&f.session_id, "codex");
        let t = o.ts()?;
        let ts = if t != 0.0 && !t.is_nan() { t } else { now };
        let p = o.payload()?.clone();
        let kind = o.kind()?.cloned();
        let pp = Some(&p);
        let get = |k: &str| opt_prop(pp, k);
        if is_str(kind.as_ref(), "event_msg") {
            match get("type").and_then(Value::as_str) {
                Some("task_started") => {
                    let s = &mut self.sessions[i];
                    s.reset_stall();
                    s.turn_marks += 1.0;
                    s.turn_active = true;
                    s.turn_started_at = Some(ts);
                    if let Some(w) = get("model_context_window").filter(|w| truthy(Some(w))) {
                        s.context_window = w.clone();
                    }
                    self.activity(i, ev(vec![("kind", sv("turn-start")), ("ts", nv(ts))]), live, now, out);
                }
                Some("task_complete") => {
                    let said = get("last_agent_message");
                    if truthy(said) {
                        self.sessions[i].add_message(said, ts)?;
                    }
                    self.sessions[i].end_turn(ts, false);
                    let duration = nn(get("duration_ms")).cloned().unwrap_or(Value::Null);
                    self.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("ts", nv(ts))]), live, now, out);
                    self.ask_if_question(i, ts, live, now, out);
                }
                Some("token_count") => self.codex_tokens(i, &p, ts, live, out),
                Some("item_completed") => {
                    let it = nn(get("item")).cloned().unwrap_or_else(|| Value::Object(Map::new()));
                    self.codex_item(i, &it, ts, live, now, out)?;
                }
                Some("user_message") => {
                    let text = o.user_event()?;
                    self.codex_user_event(i, text, ts, live, now, out);
                }
                // You stopped the turn (Esc, or Ctrl-C): Codex writes no
                // task_complete for it. A turn the next one replaced, or a
                // review ending, isn't you stopping it; older rollouts give
                // no reason.
                Some("turn_aborted") => {
                    let reason = get("reason");
                    if !truthy(reason) || is_str(reason, "interrupted") {
                        self.interrupted(i, ts, live, now, out);
                    }
                }
                // A policy refusal: the turn ends with it.
                Some("error") => {
                    let sig = codex_signal(Some(&o.raw));
                    let Some(sig) = sig.filter(|g| is_str(g.get("kind"), "refusal")) else { return Ok(()) };
                    let g = |k: &str| nn(sig.get(k));
                    let why = g("explanation").or(g("text")).or(sig.get("category")).cloned();
                    self.activity(i, ev(vec![("kind", sv("refusal")), ("detail", Some(Value::String(clip_value(why.as_ref(), 160)))), ("ts", nv(ts))]), live, now, out);
                    if self.sessions[i].turn_active {
                        self.sessions[i].end_turn(ts, false);
                    }
                }
                // All of the thread's settings: a tier not said is the
                // default.
                Some("thread_settings_applied") => {
                    let tier = opt_prop(get("thread_settings"), "service_tier");
                    self.sessions[i].priority = pricing::priority_tier(&string_of(tier));
                }
                _ => {}
            }
            return Ok(());
        }
        let s = &mut self.sessions[i];
        if is_str(kind.as_ref(), "turn_context") {
            if let Some(m) = get("model").filter(|m| truthy(Some(m))) {
                s.model = m.clone();
            }
        }
        // What prices its calls: the provider, and the tier, said with a
        // turn in older rollouts and in the thread's settings in newer ones.
        if is_str(kind.as_ref(), "session_meta") {
            if let Some(v) = nn(get("model_provider")) {
                s.provider = v.clone();
            }
        }
        if is_str(kind.as_ref(), "turn_context") && get("service_tier").is_some() {
            s.priority = pricing::priority_tier(&string_of(get("service_tier")));
        }
        if is_str(kind.as_ref(), "response_item") {
            let call_id = nn(get("call_id")).or(get("id")).cloned();
            match get("type").and_then(Value::as_str) {
                Some("function_call" | "custom_tool_call" | "local_shell_call") => {
                    let mut fields = Map::new();
                    if let Some(n) = get("name") {
                        fields.insert("name".into(), n.clone());
                    }
                    fields.insert("ts".into(), number(ts));
                    kset(&mut s.inflight, call_id.as_ref(), Inflight { fields, tool: None });
                    s.last_event_at = js_max(s.last_event_at, ts);
                    if is_str(get("name"), "apply_patch") {
                        let files = patch_files(nn(get("input")).or(get("arguments")));
                        self.note_tool(i, ToolCall { name: sv("apply_patch"), files, ts, ..ToolCall::default() }, live, now, out);
                    }
                }
                // Its output ends a call, a shell call's too, matched as
                // stats match them: by call_id, else id.
                Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output") => {
                    s.inflight.remove(&key_tag(call_id.as_ref()));
                    s.last_event_at = js_max(s.last_event_at, ts);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn codex_tokens(&mut self, i: usize, p: &Value, ts: f64, live: bool, out: &mut Vec<Emit>) {
        let info = opt_prop(Some(p), "info");
        let total_usage = opt_prop(info, "total_token_usage");
        let s = &mut self.sessions[i];
        if truthy(total_usage) {
            let total = or0(opt_prop(total_usage, "total_tokens"));
            let delta = total - s.tokens_processed;
            if delta > 0.0 {
                s.add_tokens(delta, ts, live);
                // The call's own tokens, as stats read them: cached input
                // and cache writes are part of input_tokens.
                let empty = Value::Object(Map::new());
                let last = Some(nn(opt_prop(info, "last_token_usage")).unwrap_or(&empty));
                let input = match nn(opt_prop(last, "input_tokens")) {
                    Some(v) => js_number(Some(v)),
                    None => js_max(0.0, or0(opt_prop(last, "total_tokens")) - or0(opt_prop(last, "output_tokens"))),
                };
                let cached = or0(opt_prop(last, "cached_input_tokens"));
                let write = or0(opt_prop(last, "cache_write_input_tokens"));
                let model = nn(Some(&s.model)).cloned().unwrap_or_else(|| "unknown".into());
                let call_model = if truthy(Some(&s.provider)) && !is_str(Some(&s.provider), "openai") {
                    Value::String(format!("{}/{}", js_string(&s.provider), js_string(&model)))
                } else {
                    model
                };
                let call = Call {
                    tokens: TokenUsage {
                        input: Some(js_max(0.0, input - cached - write)),
                        cache_read: Some(cached),
                        cache_write_5m: Some(write),
                        cache_write_1h: Some(0.0),
                        output: Some(or0(opt_prop(last, "output_tokens"))),
                        speed: Speed { fast: false, priority: s.priority },
                    },
                };
                s.add_call(&call_model, &call, ts);
                if live {
                    out.push(Emit::Tokens(obj(vec![("sessionId", Value::String(s.id.clone())), ("app", "codex".into()), ("n", number(delta)), ("ts", number(ts))])));
                }
            }
            s.tokens_output = or0(opt_prop(total_usage, "output_tokens")) + or0(opt_prop(total_usage, "reasoning_output_tokens"));
        }
        let last = opt_prop(info, "last_token_usage");
        if truthy(last) {
            if let Some(v) = nn(opt_prop(last, "total_tokens")) {
                s.context_used = v.clone();
            }
        }
        if let Some(w) = opt_prop(info, "model_context_window").filter(|w| truthy(Some(w))) {
            s.context_window = w.clone();
        }
        let rl = opt_prop(Some(p), "rate_limits").cloned();
        if let Some(rl) = rl.as_ref().filter(|rl| truthy(Some(rl))) {
            self.note_codex_limits(rl, ts);
        }
        let s = &mut self.sessions[i];
        s.last_event_at = js_max(s.last_event_at, ts);
    }

    // A user_message event: a prompt while the rollout has no completed
    // user message (`codex exec` writes none, nor did Codex before 0.130),
    // passed by once it has.
    fn codex_user_event(&mut self, i: usize, text: Option<String>, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        let s = &mut self.sessions[i];
        let text = text.map(Value::String);
        if s.codex_items || !is_codex_prompt(text.as_ref()) {
            return;
        }
        let text = string_of(text.as_ref());
        s.add_prompt(&text, ts);
        s.event_prompt = Some((s.prompts.last().expect("add_prompt just added one").clone(), s.turn_marks));
        self.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(&text, 90)))), ("ts", nv(ts))]), live, now, out);
    }

    /// Codex's rate limits in XenoNet's own shape (the snapshot's
    /// limits.codex): each window's use in percent, its length and when it
    /// resets, in ms.
    pub fn note_codex_limits(&mut self, rl: &Value, ts: f64) {
        let window = |w: Option<&Value>| -> Value {
            if !(truthy(w) && matches!(w, Some(Value::Object(_) | Value::Array(_)))) {
                return Value::Null;
            }
            let numeric = |k: &str| opt_prop(w, k).filter(|v| v.is_number()).cloned().unwrap_or(Value::Null);
            let resets = opt_prop(w, "resets_at").and_then(Value::as_f64).filter(|r| *r > 0.0).map_or(Value::Null, |r| number(r * 1000.0));
            obj(vec![("pct", numeric("used_percent")), ("windowMinutes", numeric("window_minutes")), ("resetsAt", resets)])
        };
        let primary = window(opt_prop(Some(rl), "primary"));
        let secondary = window(opt_prop(Some(rl), "secondary"));
        let pick = |w: &Value, k: &str| nn(opt_prop(Some(w), k)).cloned().unwrap_or(Value::Null);
        let sample = obj(vec![("ts", number(ts)), ("primary", pick(&primary, "pct")), ("secondary", pick(&secondary, "pct")), ("resets", pick(&primary, "resetsAt"))]);
        self.limits_codex = obj(vec![("primary", primary), ("secondary", secondary), ("observedAt", number(ts))]);
        if let Some(last) = self.samples_codex.last().filter(|l| truthy(Some(l))) {
            let same = strict_eq(opt_prop(Some(last), "primary"), sample.get("primary")) && strict_eq(opt_prop(Some(last), "secondary"), sample.get("secondary"));
            if same && ts - num_of(Some(last), "ts") < 10.0 * 60e3 {
                return;
            }
        }
        self.samples_codex.push(sample);
        if self.samples_codex.len() > 2000 {
            self.samples_codex.remove(0);
        }
    }

    /// A completed item of a Codex rollout.
    pub fn codex_item(&mut self, i: usize, it: &Value, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let item = Some(it);
        let get = |k: &str| opt_prop(item, k);
        let words = |strict: bool| -> Result<String, TypeError> {
            let content = nn(get("content"));
            let list: &[Value] = match content {
                None => &[],
                Some(Value::Array(a)) => a,
                Some(_) => return Err(TypeError("(it.content ?? []).map is not a function".into())),
            };
            let mut parts = Vec::new();
            for c in list {
                // UserMessage reads c?.text, AgentMessage c.text.
                parts.push(if strict { prop(Some(c), "text")? } else { opt_prop(Some(c), "text") });
            }
            Ok(join(parts.into_iter(), " "))
        };
        match get("type").and_then(Value::as_str) {
            Some("UserMessage") => {
                let text = words(false)?;
                // The rollout's first: from here on its prompts are these.
                // Codex may have written it as a user_message event just
                // before, in the same turn, which was taken for this prompt.
                let s = &mut self.sessions[i];
                let event = if s.codex_items { None } else { s.event_prompt.clone() };
                s.codex_items = true;
                s.event_prompt = None;
                if let Some((p, marks)) = event {
                    if s.prompts.last().is_some_and(|l| l.seq == p.seq) && marks == s.turn_marks {
                        return Ok(());
                    }
                }
                // A blank one (an image alone), or one that starts with "<",
                // isn't a prompt, as in stats and search.
                if !is_codex_prompt(Some(&Value::String(text.clone()))) {
                    return Ok(());
                }
                s.add_prompt(&text, ts);
                self.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(&text, 90)))), ("ts", nv(ts))]), live, now, out);
            }
            Some("AgentMessage") => {
                let text = words(true)?;
                if !js_trim(&text).is_empty() {
                    self.sessions[i].add_message(Some(&Value::String(text.clone())), ts)?;
                    self.activity(i, ev(vec![("kind", sv("message")), ("detail", Some(Value::String(clip(&text, 72)))), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("CommandExecution") => {
                let code = get("exit_code");
                let ok = nn(code).is_none() || js_number(code) == 0.0;
                let done = ok && !is_str(get("status"), "failed");
                self.sessions[i].note_tool_result(done, ts, None);
                let cmd = command_text(get("command"));
                let detail = clip(&paths::tilde(&cmd, self.home.as_deref().unwrap_or("")), 72);
                self.note_tool(i, ToolCall { name: sv("shell"), detail, full: Some(Value::String(cmd)), ok: Value::Bool(done), ts, ..ToolCall::default() }, live, now, out);
                if !ok {
                    self.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("McpToolCall") => {
                let ok = is_str(get("status"), "completed") && !truthy(get("error"));
                self.sessions[i].note_tool_result(ok, ts, None);
                let (server, tool) = (get("server"), get("tool"));
                let pages = wiki_pages_of(server.and_then(Value::as_str), get("arguments"));
                let call = ToolCall {
                    name: Some(Value::String(format!("mcp__{}__{}", tpl(server), tpl(tool)))),
                    server: server.cloned(),
                    detail: format!("{}.{}", tpl(server), tpl(tool)),
                    full: Some(nn(get("arguments")).cloned().unwrap_or_else(|| Value::Object(Map::new()))),
                    ok: Value::Bool(ok),
                    ts,
                    extra: pages_extra(pages),
                    ..ToolCall::default()
                };
                self.note_tool(i, call, live, now, out);
                if !ok {
                    self.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("FileChange") => {
                let changes = get("changes");
                let files = object_keys(nn(changes));
                self.sessions[i].add_edits(&codex_edits(changes), ts)?;
                let done = !is_str(get("status"), "failed");
                self.sessions[i].note_tool_result(done, ts, None);
                let detail = files.iter().map(|f| paths::base_name(f)).collect::<Vec<_>>().join(", ");
                self.note_tool(i, ToolCall { name: sv("apply_patch"), detail, files, ok: Value::Bool(done), ts, ..ToolCall::default() }, live, now, out);
            }
            // An empty query is no query: the action's says what was
            // searched, as a thread shown from its rollout reads it.
            Some("WebSearch") => {
                let q = [get("query"), opt_prop(get("action"), "query")].into_iter().find(|v| truthy(*v)).flatten();
                self.note_tool(i, ToolCall { name: sv("web_search"), detail: clip_value(q, 72), ts, ..ToolCall::default() }, live, now, out);
            }
            Some("ImageView") => {
                let detail = paths::base_name(&string_of(get("path")));
                self.note_tool(i, ToolCall { name: sv("view_image"), detail, ts, ..ToolCall::default() }, live, now, out);
            }
            Some(kind @ ("CollabAgentToolCall" | "SubAgentActivity")) => {
                // Rollouts name these fields in snake_case (the app-server's
                // API says them in camelCase, which is read too).
                let id = [get("agent_thread_id").cloned(), get("agentThreadId").cloned(), index0(get("receiver_thread_ids")), index0(get("receiverThreadIds")), get("id").cloned()]
                    .into_iter()
                    .find(|v| v.as_ref().is_some_and(|x| !x.is_null()))
                    .flatten();
                let tag = key_tag(id.as_ref());
                let s = &mut self.sessions[i];
                if !s.subagents.contains(&tag) {
                    let label = nn(get("prompt")).or(nn(get("agent_path"))).or(nn(get("agentPath"))).cloned().unwrap_or_else(|| "subagent".into());
                    let label = Value::String(clip_value(Some(&label), 40));
                    let m = map_opt(vec![
                        ("id", id.clone()),
                        ("label", Some(label.clone())),
                        ("startedAt", nv(ts)),
                        ("lastAt", nv(ts)),
                        ("tools", nv(0.0)),
                        ("done", Some(Value::Bool(false))),
                        ("announced", Some(Value::Bool(true))),
                    ]);
                    s.subagents.insert(&tag, (id.clone(), m));
                    self.activity(i, ev(vec![("kind", sv("subagent-start")), ("agentId", id.clone()), ("label", Some(label)), ("ts", nv(ts))]), live, now, out);
                }
                let a = &mut self.sessions[i].subagents.get_mut(&tag).expect("added above when missing").1;
                a.insert("lastAt".into(), number(ts));
                // A subagent's activity says what happened to it: started,
                // interacted (given more to do), interrupted or completed.
                // It's done once it completes or is stopped, until it's
                // given more.
                if kind != "SubAgentActivity" {
                    return Ok(());
                }
                let what = get("kind");
                if is_str(what, "completed") || is_str(what, "interrupted") {
                    let was_done = truthy(a.get("done"));
                    a.insert("done".into(), Value::Bool(true));
                    let label = a.get("label").cloned();
                    if !was_done {
                        self.activity(i, ev(vec![("kind", sv("subagent-stop")), ("agentId", id), ("label", label), ("ts", nv(ts))]), live, now, out);
                    }
                } else if is_str(what, "started") || is_str(what, "interacted") {
                    a.insert("done".into(), Value::Bool(false));
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ----- Claude plan usage -----

    /// Anthropic was asked, and answered `r` ({ usage } or { reason }): a
    /// reading joins the samples (8 days are kept), or the reason there's
    /// none is kept.
    pub fn claude_asked(&mut self, r: Option<&Value>, now: f64) -> Result<(), TypeError> {
        let usage = prop(r, "usage")?;
        if truthy(usage) {
            self.claude_reason = Value::Null;
            self.anthropic_samples.retain(|x| now - num_of(Some(x), "ts") < 8.0 * 86400e3);
            let u = |k: &str| opt_prop(usage, k).cloned();
            self.anthropic_samples.push(obj_opt(vec![
                ("ts", u("observedAt")),
                ("fiveHour", u("fiveHour")),
                ("sevenDay", u("sevenDay")),
                ("resets", u("resets")),
                ("source", sv("anthropic")),
            ]));
        } else {
            self.claude_reason = nn(prop(r, "reason")?).cloned().unwrap_or(Value::Null);
        }
        Ok(())
    }

    /// Claude desktop's samples, as its file has them now.
    pub fn desktop_read(&mut self, samples: &Value) {
        if let Value::Array(a) = samples {
            self.desktop_samples.clone_from(a);
        }
    }

    /// Both sources' samples, in time order; the newer reading wins.
    pub fn settle_claude(&mut self) {
        let mut samples: Vec<Value> = self.anthropic_samples.iter().chain(&self.desktop_samples).cloned().collect();
        sort_by_num(&mut samples, |a, b| num_of(Some(a), "ts") - num_of(Some(b), "ts"));
        if let Some(last) = samples.last().filter(|l| truthy(Some(l))) {
            let l = |k: &str| opt_prop(Some(last), k).cloned();
            let resets = nn(opt_prop(Some(last), "resets")).cloned().unwrap_or_else(|| obj(vec![("fiveHour", Value::Null), ("sevenDay", Value::Null)]));
            self.limits_claude =
                obj_opt(vec![("fiveHour", l("fiveHour")), ("sevenDay", l("sevenDay")), ("resets", Some(resets)), ("observedAt", l("ts")), ("source", l("source"))]);
        }
        self.samples_claude = samples;
    }

    /// An old Claude reading says nothing about the windows now, so it's
    /// reported as missing, with when it was taken and why there's no newer
    /// one.
    pub fn claude_limits(&self, now: f64) -> Value {
        let c = &self.limits_claude;
        if truthy(Some(c)) && now - num_of(Some(c), "observedAt") < CLAUDE_LIMITS_FRESH_MS {
            return c.clone();
        }
        if !truthy(Some(c)) && !truthy(Some(&self.claude_reason)) {
            return Value::Null;
        }
        let observed = nn(opt_prop(Some(c), "observedAt")).cloned().unwrap_or(Value::Null);
        obj(vec![("fiveHour", Value::Null), ("sevenDay", Value::Null), ("observedAt", observed), ("stale", Value::Bool(true)), ("reason", self.claude_reason.clone())])
    }

    /// Straight-line projection of usage over the recent samples.
    pub fn limit_forecast(&self, now: f64) -> Value {
        // `horizon` stands in for an unknown reset time: the window's length.
        let project = |points: Vec<(f64, f64)>, pct: Option<&Value>, resets_at: Option<Value>, horizon: f64| -> Value {
            let per_hour = slope(&points.iter().map(|(t, v)| (t / 3600e3, *v)).collect::<Vec<_>>());
            if nn(pct).is_none() {
                return Value::Null;
            }
            let p = js_number(pct);
            let eta = (per_hour > 0.05).then(|| now + ((100.0 - p) / per_hour) * 3600e3);
            let limit = match nn(resets_at.as_ref()) {
                Some(r) => js_number(Some(r)),
                None => now + horizon,
            };
            obj_opt(vec![
                ("pct", pct.cloned()),
                ("perHour", nv(per_hour)),
                ("eta", Some(eta.map_or(Value::Null, number))),
                ("resetsAt", resets_at),
                ("hitsBeforeReset", Some(Value::Bool(eta.is_some_and(|e| e < limit)))),
            ])
        };
        let mut out = Map::new();
        let primary = opt_prop(Some(&self.limits_codex), "primary");
        if truthy(primary) {
            let (pct, minutes, resets_at) = (opt_prop(primary, "pct"), opt_prop(primary, "windowMinutes"), opt_prop(primary, "resetsAt"));
            let points = self
                .samples_codex
                .iter()
                .filter(|x| nn(opt_prop(Some(x), "primary")).is_some() && strict_eq(opt_prop(Some(x), "resets"), resets_at) && now - num_of(Some(x), "ts") < 24.0 * 3600e3)
                .map(|x| (num_of(Some(x), "ts"), num_of(Some(x), "primary")))
                .collect();
            let horizon = nn(minutes).map_or(300.0, |m| js_number(Some(m))) * 60e3;
            let mut m = match project(points, pct, resets_at.cloned(), horizon) {
                Value::Object(m) => m,
                _ => Map::new(),
            };
            if let Some(w) = minutes {
                m.insert("windowMinutes".into(), w.clone());
            }
            out.insert("codex".into(), Value::Object(m));
        }
        let c = self.claude_limits(now);
        if truthy(Some(&c)) && !truthy(c.get("stale")) {
            let resets = c.get("resets");
            // Only samples from the current window, when its reset is known.
            let since = |resets_at: Option<&Value>, span: f64, lookback: f64| js_max(now - lookback, if truthy(resets_at) { js_number(resets_at) - span } else { 0.0 });
            let points = |key: &str, from: f64| -> Vec<(f64, f64)> {
                self.samples_claude
                    .iter()
                    .filter(|x| nn(opt_prop(Some(x), key)).is_some() && num_of(Some(x), "ts") >= from)
                    .map(|x| (num_of(Some(x), "ts"), num_of(Some(x), key)))
                    .collect()
            };
            for (name, key, span, lookback) in [("claudeFiveHour", "fiveHour", 5.0 * 3600e3, 3.0 * 3600e3), ("claudeSevenDay", "sevenDay", 7.0 * 86400e3, 48.0 * 3600e3)] {
                let resets_at = opt_prop(resets, key);
                let pts = points(key, since(resets_at, span, lookback));
                out.insert(name.into(), project(pts, c.get(key), Some(nn(resets_at).cloned().unwrap_or(Value::Null)), span));
            }
        }
        Value::Object(out)
    }

    // ----- outputs -----

    /// Sessions that touched (read or edited) the same file recently, from
    /// their snapshots.
    pub fn collab(snapshots: &[Value]) -> Value {
        let mut by_file: OrderedMap<Vec<(String, Value)>> = OrderedMap::new();
        for s in snapshots {
            let id = opt_prop(Some(s), "id");
            let files: &[Value] = match opt_prop(Some(s), "files") {
                Some(Value::Array(a)) => a,
                _ => &[],
            };
            for f in files {
                let key = file_key(&tpl(opt_prop(Some(f), "path")));
                let ids = by_file.get_or_insert_with(&key, Vec::new);
                let tag = key_tag(id);
                if !ids.iter().any(|(t, _)| *t == tag) {
                    ids.push((tag, id.cloned().unwrap_or(Value::Null)));
                }
            }
        }
        let mut edges: OrderedMap<Map<String, Value>> = OrderedMap::new();
        for (file, ids) in by_file.iter() {
            if ids.len() < 2 {
                continue;
            }
            for i in 0..ids.len() {
                for j in i + 1..ids.len() {
                    let (a, b) = (js_string(&ids[i].1), js_string(&ids[j].1));
                    let mut pair = [a, b];
                    pair.sort_by(|x, y| jsstr::cmp_units(x, y));
                    let k = pair.join("|");
                    let e = edges.get_or_insert_with(&k, || {
                        [("a", ids[i].1.clone()), ("b", ids[j].1.clone()), ("kind", "file".into()), ("files", Value::Array(Vec::new()))]
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect()
                    });
                    if let Some(Value::Array(files)) = e.get_mut("files") {
                        files.push(Value::String(paths::base_name(file)));
                    }
                }
            }
        }
        Value::Array(edges.iter().map(|(_, e)| Value::Object(e.clone())).collect())
    }

    /// Sessions that both edited the same file within ten minutes.
    pub fn collisions(&self, now: f64) -> Value {
        // file key → (the key, [(session, when, path)]).
        type Edited = (String, Vec<(String, f64, Value)>);
        let mut by_file: OrderedMap<Edited> = OrderedMap::new();
        for s in &self.sessions {
            for (tag, (k, list)) in s.edits.iter() {
                let Some(last) = list.last() else { continue };
                let ts = num_of(Some(last), "ts");
                if now - ts > COLLISION_WINDOW_MS {
                    continue;
                }
                by_file.get_or_insert_with(tag, || (string_of(k.as_ref()), Vec::new())).1.push((s.id.clone(), ts, opt_prop(Some(last), "path").cloned().unwrap_or(Value::Null)));
            }
        }
        let mut out: OrderedMap<Map<String, Value>> = OrderedMap::new();
        for (_, (k, list)) in by_file.iter() {
            if list.len() < 2 {
                continue;
            }
            for i in 0..list.len() {
                for j in i + 1..list.len() {
                    let (a, b) = if jsstr::cmp_units(&list[j].0, &list[i].0) == Ordering::Less { (&list[j], &list[i]) } else { (&list[i], &list[j]) };
                    let key = format!("{}|{}", a.0, b.0);
                    let c = out.get_or_insert_with(&key, || {
                        [
                            ("a", Value::String(a.0.clone())),
                            ("b", Value::String(b.0.clone())),
                            ("files", Value::Array(Vec::new())),
                            ("paths", Value::Array(Vec::new())),
                            ("ts", number(0.0)),
                        ]
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect()
                    });
                    if let Some(Value::Array(files)) = c.get_mut("files") {
                        files.push(Value::String(paths::base_name(&string_of(Some(&a.2)))));
                    }
                    if let Some(Value::Array(paths)) = c.get_mut("paths") {
                        paths.push(Value::String(k.clone()));
                    }
                    let ts = js_max(js_max(js_number(c.get("ts")), a.1), b.1);
                    c.insert("ts".into(), number(ts));
                }
            }
        }
        Value::Array(out.iter().map(|(_, c)| Value::Object(c.clone())).collect())
    }

    /// Both sessions' edits of the files both edited.
    pub fn collision_diffs(&self, a: &str, b: &str) -> Value {
        let (Some(sa), Some(sb)) = (self.find(a).map(|i| &self.sessions[i]), self.find(b).map(|i| &self.sessions[i])) else { return Value::Array(Vec::new()) };
        Value::Array(
            sa.edits
                .iter()
                .filter_map(|(tag, (k, list))| {
                    let (_, theirs) = sb.edits.get(tag)?;
                    Some(obj(vec![("file", k.clone().unwrap_or(Value::Null)), ("a", Value::Array(list.clone())), ("b", Value::Array(theirs.clone()))]))
                })
                .collect(),
        )
    }

    /// Context for a handoff brief, from what the tailer has seen.
    pub fn session_notes(&self, id: &str, now: f64) -> Value {
        let Some(i) = self.find(id) else { return Value::Null };
        let s = &self.sessions[i];
        let edits: Vec<Value> = s.edits.iter().flat_map(|(_, (_, list))| list.iter().map(|e| opt_prop(Some(e), "path").cloned().unwrap_or(Value::Null))).collect();
        obj(vec![
            ("firstPrompt", s.first_prompt.as_ref().map_or(Value::Null, Prompt::value)),
            ("prompts", Value::Array(s.prompts.iter().map(Prompt::value).collect())),
            ("messages", Value::Array(s.messages.iter().map(|m| obj(vec![("text", m.text.clone()), ("ts", m.ts.clone())])).collect())),
            ("files", Value::Array(s.files.iter().map(|(_, (k, _))| k.clone().unwrap_or(Value::Null)).collect())),
            ("edits", Value::Array(edits)),
            ("context", s.context_value()),
            ("needsYou", s.needs_you(now)),
        ])
    }

    /// What today's calls would cost at API prices, by app, and at this
    /// minute's pace: each app's tokens a minute (from `sessions`, the
    /// snapshot's) at what its tokens cost on average today. Models without
    /// a price are named, not guessed.
    pub fn costs(&self, now: f64, sessions: &[Value]) -> Value {
        let day = str_tag(&day_key(now));
        let mut by_app: Vec<(String, f64, f64)> = vec![("claude".into(), 0.0, 0.0), ("codex".into(), 0.0, 0.0)];
        let mut unpriced: Vec<String> = Vec::new();
        for s in &self.sessions {
            let Some((_, d)) = s.spend.get(&day) else { continue };
            match by_app.iter_mut().find(|(app, _, _)| *app == s.app) {
                Some(slot) => {
                    slot.1 += d.cost;
                    slot.2 += d.tokens;
                }
                // undefined + a number, as JS adds it.
                None => by_app.push((s.app.clone(), f64::NAN, f64::NAN)),
            }
            for m in &d.unpriced {
                if !unpriced.contains(m) {
                    unpriced.push(m.clone());
                }
            }
        }
        let mut per_hour = 0.0;
        for (app, cost, tokens) in by_app.iter().take(2) {
            let tpm = sessions.iter().filter(|x| is_str(opt_prop(Some(x), "app"), app)).fold(0.0, |a, x| a + num_of(Some(x), "tpm"));
            if *tokens > 0.0 {
                per_hour += tpm * 60.0 * (cost / tokens);
            }
        }
        unpriced.sort_by(|a, b| jsstr::cmp_units(a, b));
        let by_app_value: Map<String, Value> = by_app.iter().map(|(app, cost, _)| (app.clone(), number(*cost))).collect();
        obj(vec![
            ("today", number(by_app[0].1 + by_app[1].1)),
            ("byApp", Value::Object(by_app_value)),
            ("perHour", number(per_hour)),
            ("unpriced", Value::Array(unpriced.into_iter().map(Value::String).collect())),
            ("checked", pricing::PRICES_CHECKED.into()),
        ])
    }

    /// What each session's calls today would cost at API prices: [{ id,
    /// app, cost, tokens, unpriced }] for each session with a call today.
    pub fn session_costs(&self, now: f64) -> Value {
        let day = str_tag(&day_key(now));
        Value::Array(
            self.sessions
                .iter()
                .filter_map(|s| {
                    let (_, d) = s.spend.get(&day)?;
                    let mut unpriced = d.unpriced.clone();
                    unpriced.sort_by(|a, b| jsstr::cmp_units(a, b));
                    Some(obj(vec![
                        ("id", Value::String(s.id.clone())),
                        ("app", Value::String(s.app.clone())),
                        ("cost", number(d.cost)),
                        ("tokens", number(d.tokens)),
                        ("unpriced", Value::Array(unpriced.into_iter().map(Value::String).collect())),
                    ]))
                })
                .collect(),
        )
    }

    /// Everything the UI shows, at `now`: the sessions of the last day, the
    /// token rate's last five minutes by app, plan limits and their
    /// forecast, costs, the sessions on the same files, the MCP servers in
    /// use. (The tailer's status is the caller's to add.)
    pub fn snapshot(&mut self, now: f64) -> Value {
        let mut sessions = Vec::new();
        for s in &mut self.sessions {
            if now - s.last_event_at < PRIME_WINDOW_MS {
                sessions.push(s.snapshot(now));
            }
        }
        let mut series = [vec![0.0; RATE_WINDOW_S as usize], vec![0.0; RATE_WINDOW_S as usize]];
        let now_s = (now / 1000.0).floor();
        for s in &self.sessions {
            let which = match s.app.as_str() {
                "claude" => 0,
                "codex" => 1,
                _ => continue,
            };
            for (sec, n) in &s.rate {
                let i = RATE_WINDOW_S - 1.0 - (now_s - sec);
                if (0.0..RATE_WINDOW_S).contains(&i) {
                    series[which][i as usize] += n;
                }
            }
        }
        let mut resources: OrderedMap<(Value, f64)> = OrderedMap::new();
        for s in &sessions {
            if let Some(Value::Array(list)) = s.get("mcp") {
                for m in list {
                    let server = m.get("server").cloned().unwrap_or(Value::Null);
                    let tag = key_tag(Some(&server));
                    let had = resources.get(&tag).map_or(0.0, |(_, t)| *t);
                    resources.insert(&tag, (server, js_max(had, num_of(Some(m), "ts"))));
                }
            }
        }
        let mut resources: Vec<(Value, f64)> = resources.iter().map(|(_, r)| r.clone()).collect();
        sort_by_num(&mut resources, |a, b| b.1 - a.1);
        resources.truncate(8);
        let sum = |key: &str| sessions.iter().fold(0.0, |a, s| a + num_of(Some(s), key));
        let total = sessions.iter().fold(0.0, |a, s| a + num_of(s.get("tokens"), "processed"));
        let series_value =
            obj(vec![("claude", Value::Array(series[0].iter().map(|x| number(*x)).collect())), ("codex", Value::Array(series[1].iter().map(|x| number(*x)).collect()))]);
        let (tpm, today) = (sum("tpm"), sum("tokensToday"));
        let limits = obj(vec![("codex", self.limits_codex.clone()), ("claude", self.claude_limits(now))]);
        let forecast = self.limit_forecast(now);
        let costs = self.costs(now, &sessions);
        let collab = Self::collab(&sessions);
        let collisions = self.collisions(now);
        let resources = Value::Array(resources.into_iter().map(|(server, ts)| obj(vec![("server", server), ("ts", number(ts))])).collect());
        obj(vec![
            ("at", number(now)),
            ("sessions", Value::Array(sessions)),
            ("series", series_value),
            ("tpm", number(tpm)),
            ("total24h", number(total)),
            ("today", number(today)),
            ("limits", limits),
            ("limitForecast", forecast),
            ("costs", costs),
            ("collab", collab),
            ("collisions", collisions),
            ("resources", resources),
        ])
    }

    // ----- what the twin reads and sets -----

    pub fn limits(&self, which: &str) -> Value {
        match which {
            "codex" => self.limits_codex.clone(),
            _ => self.limits_claude.clone(),
        }
    }

    pub fn set_limits(&mut self, which: &str, v: Value) {
        match which {
            "codex" => self.limits_codex = v,
            _ => self.limits_claude = v,
        }
    }

    pub fn limit_samples(&self, which: &str) -> Value {
        Value::Array(match which {
            "codex" => self.samples_codex.clone(),
            _ => self.samples_claude.clone(),
        })
    }

    /// Samples set from outside: a list, as JS keeps whatever it's given
    /// (anything else is kept as no samples).
    pub fn set_limit_samples(&mut self, which: &str, v: Value) {
        let list = match v {
            Value::Array(a) => a,
            _ => Vec::new(),
        };
        match which {
            "codex" => self.samples_codex = list,
            _ => self.samples_claude = list,
        }
    }

    pub fn claude_reason(&self) -> Value {
        self.claude_reason.clone()
    }

    pub fn set_claude_reason(&mut self, v: Value) {
        self.claude_reason = v;
    }

    pub fn claude_samples(&self) -> Value {
        obj(vec![("anthropic", Value::Array(self.anthropic_samples.clone())), ("desktop", Value::Array(self.desktop_samples.clone()))])
    }

    pub fn pending_label(&self) -> Value {
        match &self.pending_label {
            None => Value::Null,
            Some((session, label, ts)) => obj(vec![("session", Value::String(session.clone())), ("label", label.clone()), ("ts", number(*ts))]),
        }
    }
}

/// The session id a Codex rollout's name ends with: the uuid before
/// ".jsonl" (/([0-9a-f]{8}-…-[0-9a-f]{12})\.jsonl$/i).
fn uuid_before(norm: &str) -> Option<String> {
    let rest = norm.strip_suffix(".jsonl")?;
    if rest.len() < 36 || !rest.is_char_boundary(rest.len() - 36) {
        return None;
    }
    let id = &rest[rest.len() - 36..];
    let ok = id.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() });
    ok.then(|| id.to_string())
}

/// A Claude Code subagent's transcript
/// (/\/([0-9a-f-]{36})\/subagents\/agent-([^/]+)\.jsonl$/i): its session
/// and its agent.
fn subagent_file(norm: &str) -> Option<(String, String)> {
    let rest = norm.strip_suffix(".jsonl")?;
    let slash = rest.rfind('/')?;
    let name = &rest[slash + 1..];
    let agent = name.strip_prefix("agent-").filter(|a| !a.is_empty())?;
    let dir = rest[..slash].strip_suffix("/subagents")?;
    let at = dir.rfind('/')?;
    let session = &dir[at + 1..];
    (session.len() == 36 && session.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).then(|| (session.to_string(), agent.to_string()))
}

// ---------- the faces the twin gives JS ----------

impl Session {
    /// A session's field, for the twin's view of it: Some(None) for
    /// undefined, and None for a field it doesn't have.
    pub fn get(&self, name: &str) -> Option<Option<Value>> {
        let f = |v: f64| Some(number(v));
        Some(match name {
            "id" => Some(Value::String(self.id.clone())),
            "app" => Some(Value::String(self.app.clone())),
            "tokens" => Some(self.tokens_value()),
            "provider" => Some(self.provider.clone()),
            "priority" => Some(Value::Bool(self.priority)),
            "context" => Some(self.context_value()),
            "turnContexts" => Some(Value::Array(self.turn_contexts.iter().map(|x| number(*x)).collect())),
            "model" => Some(self.model.clone()),
            "turn" => Some(obj(vec![("active", Value::Bool(self.turn_active)), ("startedAt", self.turn_started_at.map_or(Value::Null, number))])),
            "toolCounts" => Some(Value::Object(self.tool_counts.clone())),
            "lastTool" => Some(self.last_tool.clone().unwrap_or(Value::Null)),
            "rate" => Some(Value::Array(self.rate.iter().map(|(s, n)| Value::Array(vec![number(*s), number(*n)])).collect())),
            "lastEventAt" => f(self.last_event_at),
            "prompts" => Some(Value::Array(self.prompts.iter().map(Prompt::value).collect())),
            "firstPrompt" => Some(self.first_prompt.as_ref().map_or(Value::Null, Prompt::value)),
            "messages" => Some(Value::Array(self.messages.iter().map(|m| obj(vec![("text", m.text.clone()), ("ts", m.ts.clone())])).collect())),
            "lastPromptAt" => f(self.last_prompt_at),
            "lastTurnEndAt" => f(self.last_turn_end_at),
            "interruptedAt" => f(self.interrupted_at),
            "codexItems" => Some(Value::Bool(self.codex_items)),
            "eventPrompt" => Some(self.event_prompt.as_ref().map_or(Value::Null, |(p, marks)| obj(vec![("prompt", p.value()), ("marks", number(*marks))]))),
            "turnMarks" => f(self.turn_marks),
            "question" => Some(self.question.clone()),
            "hookNeed" => Some(self.hook_need.clone()),
            "recentTools" => Some(Value::Array(self.recent_tools.iter().map(|t| Value::Object(t.rec.clone())).collect())),
            "errorStreak" => f(self.error_streak),
            "errorSince" => self.error_since.map(number),
            "lastErrorAt" => self.last_error_at.map(number),
            "lastResultAt" => self.last_result_at.map(number),
            _ => return None,
        })
    }

    /// Sets a field from JS (None: undefined). Says whether it has one by
    /// that name.
    pub fn set(&mut self, name: &str, v: Option<Value>) -> bool {
        let num = |v: &Option<Value>| js_number(v.as_ref());
        let opt_num = |v: &Option<Value>| v.as_ref().filter(|x| !x.is_null()).map(|x| js_number(Some(x)));
        let or_null = |v: Option<Value>| v.unwrap_or(Value::Null);
        match name {
            "tokens" => {
                self.tokens_processed = num_of(v.as_ref(), "processed");
                self.tokens_output = num_of(v.as_ref(), "output");
            }
            "provider" => self.provider = or_null(v),
            "priority" => self.priority = truthy(v.as_ref()),
            "context" => {
                self.context_used = or_null(opt_prop(v.as_ref(), "used").cloned());
                self.context_window = or_null(opt_prop(v.as_ref(), "window").cloned());
            }
            "turnContexts" => self.turn_contexts = list_of(v.as_ref()).iter().map(|x| js_number(Some(x))).collect(),
            "model" => self.model = or_null(v),
            "turn" => {
                self.turn_active = truthy(opt_prop(v.as_ref(), "active"));
                self.turn_started_at = opt_prop(v.as_ref(), "startedAt").filter(|x| !x.is_null()).map(|x| js_number(Some(x)));
            }
            "toolCounts" => {
                self.tool_counts = match v {
                    Some(Value::Object(m)) => m,
                    _ => Map::new(),
                }
            }
            "lastTool" => self.last_tool = v.filter(|x| !x.is_null()),
            "rate" => self.rate = list_of(v.as_ref()).iter().map(|r| (js_number(index0(Some(r)).as_ref()), js_number(list_of(Some(r)).get(1)))).collect(),
            "lastEventAt" => self.last_event_at = num(&v),
            "prompts" => {
                let list = list_of(v.as_ref());
                self.prompts = list.iter().map(|p| self.prompt_of(p)).collect();
                self.texts += 1;
            }
            "firstPrompt" => self.first_prompt = v.filter(|x| !x.is_null()).map(|p| self.prompt_of(&p)),
            "messages" => {
                self.messages =
                    list_of(v.as_ref()).iter().map(|m| Message { text: or_null(opt_prop(Some(m), "text").cloned()), ts: or_null(opt_prop(Some(m), "ts").cloned()) }).collect();
                self.texts += 1;
            }
            "lastPromptAt" => self.last_prompt_at = num(&v),
            "lastTurnEndAt" => self.last_turn_end_at = num(&v),
            "interruptedAt" => self.interrupted_at = num(&v),
            "codexItems" => self.codex_items = truthy(v.as_ref()),
            "eventPrompt" => {
                self.event_prompt = v.filter(|x| truthy(Some(x))).map(|e| {
                    let p = opt_prop(Some(&e), "prompt").cloned().unwrap_or(Value::Null);
                    // The prompt it names, when it's one of the prompts.
                    let same = self.prompts.iter().find(|q| q.value() == p).cloned();
                    (same.unwrap_or_else(|| self.prompt_of(&p)), num_of(Some(&e), "marks"))
                });
            }
            "turnMarks" => self.turn_marks = num(&v),
            "question" => self.question = or_null(v),
            "hookNeed" => self.hook_need = or_null(v),
            "recentTools" => {
                let list = list_of(v.as_ref());
                self.recent_tools.clear();
                for t in list {
                    self.push_tool(&t);
                }
            }
            "errorStreak" => self.error_streak = num(&v),
            "errorSince" => self.error_since = opt_num(&v).or(v.as_ref().map(|_| 0.0)),
            "lastErrorAt" => self.last_error_at = opt_num(&v).or(v.as_ref().map(|_| 0.0)),
            "lastResultAt" => self.last_result_at = opt_num(&v),
            _ => return false,
        }
        true
    }

    /// What JS holds on to and sees change, as live.js's snapshot and
    /// notes hand out the session's own objects: { tokens, context,
    /// toolCounts, texts }, with its prompts and messages when `texts`
    /// (the twin's last count of their changes) is out of date.
    pub fn mirror(&self, texts: f64) -> Value {
        let mut m = Map::new();
        m.insert("tokens".into(), self.tokens_value());
        m.insert("context".into(), self.context_value());
        m.insert("toolCounts".into(), Value::Object(self.tool_counts.clone()));
        m.insert("texts".into(), number(self.texts as f64));
        if texts != self.texts as f64 {
            m.insert("prompts".into(), Value::Array(self.prompts.iter().map(Prompt::value).collect()));
            m.insert("messages".into(), Value::Array(self.messages.iter().map(|x| obj(vec![("text", x.text.clone()), ("ts", x.ts.clone())])).collect()));
        }
        Value::Object(m)
    }

    fn prompt_of(&mut self, p: &Value) -> Prompt {
        self.seq += 1;
        Prompt { seq: self.seq, text: opt_prop(Some(p), "text").cloned().unwrap_or(Value::Null), ts: opt_prop(Some(p), "ts").cloned().unwrap_or(Value::Null) }
    }

    /// A recent tool call JS pushed: kept as it was given.
    pub fn push_tool(&mut self, t: &Value) {
        let rec = match t {
            Value::Object(m) => m.clone(),
            _ => Map::new(),
        };
        let uid = self.new_uid();
        self.recent_tools.push(ToolRec { uid, rec });
    }
}

fn list_of(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

/// What JS does to one of a session's Maps (or its Set of messages).
pub enum MapOp {
    Get(Option<Value>),
    Has(Option<Value>),
    Set(Option<Value>, Option<Value>),
    Delete(Option<Value>),
    Clear,
    Size,
    Entries,
}

/// The answer: a value (None: undefined).
pub type MapAnswer = Option<Value>;

macro_rules! map_op {
    ($map:expr, $op:expr, $to:expr, $from:expr) => {{
        let m = $map;
        match $op {
            MapOp::Get(k) => m.get(&key_tag(k.as_ref())).map(|(_, v)| $to(v)),
            MapOp::Has(k) => Some(Value::Bool(m.contains(&key_tag(k.as_ref())))),
            MapOp::Set(k, v) => {
                kset(m, k.as_ref(), $from(v));
                Some(Value::Null)
            }
            MapOp::Delete(k) => Some(Value::Bool(m.remove(&key_tag(k.as_ref())).is_some())),
            MapOp::Clear => {
                m.clear();
                Some(Value::Null)
            }
            MapOp::Size => Some(number(m.len() as f64)),
            MapOp::Entries => Some(Value::Array(m.iter().map(|(_, (k, v))| Value::Array(vec![k.clone().unwrap_or(Value::Null), $to(v)])).collect())),
        }
    }};
}

impl Session {
    /// One of its Maps, as JS uses it: inflight, files, edits, mcp,
    /// subagents, daily, spend, seenMessages. None: no such map.
    pub fn map_op(&mut self, map: &str, op: MapOp) -> Option<MapAnswer> {
        let recent = self.recent_tools.clone();
        let inflight_value = move |e: &Inflight| -> Value {
            let mut m = e.fields.clone();
            if let Some(tool) = &e.tool {
                let t = tool.as_ref().map_or(Value::Null, |(uid, copy)| Value::Object(recent.iter().find(|r| r.uid == *uid).map_or_else(|| copy.clone(), |r| r.rec.clone())));
                m.insert("tool".into(), t);
            }
            Value::Object(m)
        };
        let inflight_of = |v: Option<Value>| -> Inflight {
            let mut fields = match v {
                Some(Value::Object(m)) => m,
                _ => Map::new(),
            };
            // A tool record JS gives here is its own copy, which the stall
            // check doesn't follow.
            let tool = fields.shift_remove("tool").map(|_| None);
            Inflight { fields, tool }
        };
        let value = |v: &Value| v.clone();
        let value_of = |v: Option<Value>| v.unwrap_or(Value::Null);
        let num = |v: &f64| number(*v);
        let num_of_v = |v: Option<Value>| js_number(v.as_ref());
        let list = |v: &Vec<Value>| Value::Array(v.clone());
        let list_of_v = |v: Option<Value>| list_of(v.as_ref());
        let sub = |v: &Map<String, Value>| Value::Object(v.clone());
        let sub_of = |v: Option<Value>| match v {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let spend =
            |d: &Spend| obj(vec![("cost", number(d.cost)), ("tokens", number(d.tokens)), ("unpriced", Value::Array(d.unpriced.iter().cloned().map(Value::String).collect()))]);
        let spend_of = |v: Option<Value>| Spend {
            cost: num_of(v.as_ref(), "cost"),
            tokens: num_of(v.as_ref(), "tokens"),
            unpriced: list_of(opt_prop(v.as_ref(), "unpriced")).iter().map(js_string).collect(),
        };
        let seen = |(): &()| Value::Bool(true);
        let seen_of = |_: Option<Value>| ();
        Some(match map {
            "inflight" => map_op!(&mut self.inflight, op, inflight_value, inflight_of),
            "files" => map_op!(&mut self.files, op, value, value_of),
            "mcp" => map_op!(&mut self.mcp, op, value, value_of),
            "edits" => map_op!(&mut self.edits, op, list, list_of_v),
            "subagents" => map_op!(&mut self.subagents, op, sub, sub_of),
            "daily" => map_op!(&mut self.daily, op, num, num_of_v),
            "spend" => map_op!(&mut self.spend, op, spend, spend_of),
            "seenMessages" => map_op!(&mut self.seen_messages, op, seen, seen_of),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: f64 = 1_791_000_000_000.0;

    fn at(ms: f64) -> Value {
        Value::String(xenonet_base::time::iso(ms))
    }

    fn claude(t: &mut Telemetry, raw: Value) -> Vec<Emit> {
        let mut out = Vec::new();
        let f = Where { session_id: "claude:a".into(), agent_id: Value::Null, file: Value::Null };
        t.apply_claude(&f, raw, true, NOW, &mut out).unwrap();
        out
    }

    fn codex(t: &mut Telemetry, raw: Value) -> Vec<Emit> {
        let mut out = Vec::new();
        let f = Where { session_id: "codex:b".into(), agent_id: Value::Null, file: Value::Null };
        t.apply_codex(&f, raw, true, NOW, &mut out).unwrap();
        out
    }

    fn kinds(out: &[Emit]) -> Vec<String> {
        out.iter()
            .filter_map(|e| match e {
                Emit::Activity(f) => field(f, "kind").and_then(Value::as_str).map(String::from),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn files_say_what_they_are() {
        let t = Telemetry::new(None, vec![("/x/.codex/sessions".into(), "codex".into())]);
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(t.classify(&format!("/h/.claude/projects/p/{id}.jsonl")).unwrap()["sessionId"], format!("claude:{id}"));
        let sub = t.classify(&format!("/h/.claude/projects/p/{id}/subagents/agent-abc.jsonl")).unwrap();
        assert_eq!((sub["sessionId"].as_str(), sub["agentId"].as_str()), (Some(format!("claude:{id}").as_str()), Some("abc")));
        assert_eq!(t.classify(&format!("/x/.codex/sessions/2026/rollout-2026-T-{id}.jsonl")).unwrap()["sessionId"], format!("codex:{id}"));
        assert_eq!(t.classify(&format!("/x/.codex/sessions/2026/other-{id}.jsonl")), None);
        assert_eq!(t.classify("/h/notes.txt"), None);
        assert_eq!(t.classify("C:\\h\\.claude\\projects\\p\\11111111-2222-3333-4444-55555555555A.jsonl").unwrap()["app"], "claude");
    }

    #[test]
    fn a_claude_turn() {
        let mut t = Telemetry::new(None, Vec::new());
        assert_eq!(kinds(&claude(&mut t, json!({ "type": "user", "timestamp": at(NOW - 5000.0), "message": { "content": "fix it" } }))), ["prompt"]);
        let out = claude(
            &mut t,
            json!({ "type": "assistant", "timestamp": at(NOW - 4000.0), "message": { "id": "m1", "model": "claude-x", "usage": { "input_tokens": 10, "cache_read_input_tokens": 1000, "output_tokens": 5 }, "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "npm test" } }] } }),
        );
        assert_eq!(kinds(&out), ["tool"]);
        assert!(matches!(&out[0], Emit::Tokens(v) if v["n"] == 1015));
        let i = t.find("claude:a").unwrap();
        assert_eq!(t.at(i).status(NOW), "tool");
        claude(&mut t, json!({ "type": "user", "timestamp": at(NOW - 3000.0), "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": true }] } }));
        assert_eq!(t.at(i).status(NOW), "thinking");
        assert_eq!(t.at(i).error_streak, 1.0);
        let out = claude(
            &mut t,
            json!({ "type": "assistant", "timestamp": at(NOW - 2000.0), "message": { "id": "m2", "stop_reason": "end_turn", "content": [{ "type": "text", "text": "Done. Should I push?" }] } }),
        );
        assert_eq!(kinds(&out), ["message", "turn-end", "needs-you"]);
        let snap = t.at_mut(i).snapshot(NOW);
        assert_eq!(snap["needsYou"]["kind"], "question");
        assert_eq!(snap["tokens"]["processed"], 1015);
        assert_eq!(snap["context"]["used"], 1015);
        assert_eq!(t.history_since(0.0).len(), 6);
        // A line JS would throw on: its text is a number.
        let mut out = Vec::new();
        let f = Where { session_id: "claude:a".into(), agent_id: Value::Null, file: Value::Null };
        assert!(t.apply_claude(&f, json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": 5 }] } }), true, NOW, &mut out).is_err());
    }

    #[test]
    fn a_codex_turn_and_its_limits() {
        let mut t = Telemetry::new(None, Vec::new());
        codex(&mut t, json!({ "type": "event_msg", "timestamp": at(NOW - 9000.0), "payload": { "type": "task_started" } }));
        codex(
            &mut t,
            json!({ "type": "event_msg", "timestamp": at(NOW - 8000.0), "payload": { "type": "token_count", "info": { "total_token_usage": { "total_tokens": 5000, "output_tokens": 300 }, "last_token_usage": { "total_tokens": 4000 } }, "rate_limits": { "primary": { "used_percent": 12, "window_minutes": 300 } } } }),
        );
        let i = t.find("codex:b").unwrap();
        let snap = t.at_mut(i).snapshot(NOW);
        assert_eq!((snap["tokens"]["processed"].as_f64(), snap["context"]["used"].as_f64()), (Some(5000.0), Some(4000.0)));
        assert_eq!(t.limits("codex")["primary"], json!({ "pct": 12, "windowMinutes": 300, "resetsAt": null }));
        for k in 0..3 {
            codex(
                &mut t,
                json!({ "type": "event_msg", "timestamp": at(NOW - 7000.0 + f64::from(k)), "payload": { "type": "item_completed", "item": { "type": "McpToolCall", "server": "s", "tool": "x", "arguments": { "a": 1 }, "status": "failed" } } }),
            );
        }
        assert!(t.at(i).stuck(NOW)["reasons"][0].as_str().unwrap().contains("3 times in a row"));
        codex(&mut t, json!({ "type": "event_msg", "timestamp": at(NOW - 1000.0), "payload": { "type": "task_complete", "last_agent_message": "Deployed." } }));
        assert_eq!(t.at(i).needs_you(NOW)["kind"], "review");
        let full = t.snapshot(NOW);
        assert_eq!(full.as_object().unwrap().keys().take(2).collect::<Vec<_>>(), ["at", "sessions"]);
        assert_eq!(full["resources"][0]["server"], "s");
    }

    #[test]
    fn collisions_and_costs() {
        let mut t = Telemetry::new(None, Vec::new());
        claude(
            &mut t,
            json!({ "type": "assistant", "timestamp": at(NOW - 60e3), "message": { "id": "e1", "model": "claude-made-up", "usage": { "input_tokens": 1, "output_tokens": 1 }, "content": [{ "type": "tool_use", "id": "x", "name": "Edit", "input": { "file_path": "C:\\repo\\app.js", "old_string": "a", "new_string": "b" } }] } }),
        );
        codex(
            &mut t,
            json!({ "type": "event_msg", "timestamp": at(NOW - 50e3), "payload": { "type": "item_completed", "item": { "type": "FileChange", "changes": { "C:/repo/app.js": { "type": "update", "unified_diff": "-a\n+c" } } } } }),
        );
        let c = t.collisions(NOW);
        assert_eq!(c[0]["files"], json!(["app.js"]));
        assert_eq!(t.collision_diffs("claude:a", "codex:b")[0]["file"], "c:/repo/app.js");
        assert_eq!(t.costs(NOW, &[])["unpriced"], json!(["claude-made-up"]));
    }

    #[test]
    fn history_in_order() {
        let mut t = Telemetry::new(None, Vec::new());
        let i = t.session("claude:a", "claude");
        let mut out = Vec::new();
        for ago in [5.0, 4.0, 40.0, 3.0] {
            t.note_activity(i, ev(vec![("kind", sv("tool")), ("ts", nv(NOW - ago * 1000.0))]), true, NOW, &mut out);
        }
        assert_eq!(t.history_since(NOW - 10e3).len(), 3);
        assert_eq!(t.history_since(0.0)[0].ts, NOW - 40e3);
    }

    #[test]
    fn claude_limits_go_stale() {
        let mut t = Telemetry::new(None, Vec::new());
        t.set_limits("claude", json!({ "fiveHour": 9, "sevenDay": 19, "observedAt": NOW - 10.0 * 60e3 }));
        t.set_limit_samples("claude", json!([{ "ts": NOW - 40.0 * 60e3, "fiveHour": 4, "sevenDay": 18 }, { "ts": NOW - 10.0 * 60e3, "fiveHour": 9, "sevenDay": 19 }]));
        assert_eq!(t.claude_limits(NOW)["fiveHour"], 9);
        assert_eq!(t.limit_forecast(NOW)["claudeFiveHour"]["pct"], 9);
        t.set_limits("claude", json!({ "fiveHour": 0, "sevenDay": 0, "observedAt": NOW - 2.0 * 86400e3 }));
        assert_eq!(t.claude_limits(NOW)["stale"], true);
        assert_eq!(t.limit_forecast(NOW).get("claudeFiveHour"), None);
        assert!(t.claude_asked(None, NOW).is_err());
        t.claude_asked(Some(&json!({ "reason": "no sign-in" })), NOW).unwrap();
        assert_eq!(t.claude_limits(NOW)["reason"], "no sign-in");
    }

    #[test]
    fn sorts_as_js_does() {
        let mut v = vec![(3.0, 'a'), (1.0, 'b'), (f64::NAN, 'c'), (1.0, 'd'), (2.0, 'e')];
        sort_by_num(&mut v, |a, b| a.0 - b.0);
        assert_eq!(v.iter().map(|x| x.1).collect::<String>().len(), 5);
        let mut w = vec![(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd')];
        sort_by_num(&mut w, |a, b| f64::from(a.0 - b.0));
        assert_eq!(w.iter().map(|x| x.1).collect::<String>(), "bdac");
    }
}
