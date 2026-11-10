//! core/live.js's LiveTelemetry: each session's state from its transcripts'
//! lines, and what's made of it (the snapshot, the signals the UI acts on,
//! forecasts, costs, collisions, the history).
//!
//! Reading the files is the tailer's (xenonet-blocks); what each app's
//! lines mean is its adapter's (`Tailed`, from the registry), which works
//! on a session through the `Tail` operations this implements; and the
//! events go out through whoever runs this: each line goes in, and the
//! events it makes come back, in order (`Emit`). Time is an argument
//! (`now`) wherever live.js asked Date.now().
//!
//! Records are read as JS read them: a field JS would read as undefined is
//! missing, a Map's keys are told apart as SameValueZero tells them
//! (`key_tag`), and where JS would throw on an odd record this gives the
//! TypeError lines.rs gives, after making the changes JS made before it
//! threw. Arithmetic on a record's counts is a number's (Number() of what
//! the record says), where JS's `+` would have joined strings.

use super::js::{ev, field, index0, is_str, js_max, js_round, key_tag, nn, num_of, nv, obj, obj_opt, or0, put, sort_by_num, str_tag, strict_eq, sv, tpl};
use super::names::ServerNames;
use super::{categorize, clip, clip_value, day_key, file_key, looks_like_question, mcp_server_of, median, slope, string_of, tail, tool_key};
use crate::line_events::{self, Kind, LineEvent};
use crate::lines::{opt_prop, prop};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::HashMap;
use xenonet_adapter::apps;
pub use xenonet_adapter::reader::{Call, Classified, Emit, Fields, Run, Tail, Tailed, ToolCall, ToolRecord, TypeError, VendorCheck, Where};
use xenonet_model::usage::pricing;
use xenonet_os::paths;
use xenonet_text::json::{number, number_string};
use xenonet_text::ordered::OrderedMap;
use xenonet_text::text::{display, to_number, trim, truthy};
use xenonet_text::utf16;

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
// An app whose lines don't say when a turn ends (Grok Build's): its hooks
// do, and without them a turn this quiet, with no tool call in flight, has
// ended. A session whose hooks spoke this lately waits for them.
const QUIET_TURN_MS: f64 = 60e3;
const HOOKS_HEARD_MS: f64 = 3600e3;

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

/// A day's calls: what they'd cost, their priced tokens, the models without
/// a price, and whether any call had one.
#[derive(Clone, Debug, Default)]
struct Spend {
    cost: f64,
    tokens: f64,
    unpriced: Vec<String>,
    priced: bool,
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
    turn_marks: f64,
    question: Value,
    hook_need: Value,
    // When its hooks last spoke, for an app whose lines don't say when a
    // turn ends.
    hook_at: f64,
    run_entry: Option<Run>,
    // How it asks before it acts (plan, bypassPermissions…), as its hooks
    // last said.
    permission_mode: Option<String>,
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
            turn_marks: 0.0,
            question: Value::Null,
            hook_need: Value::Null,
            hook_at: 0.0,
            run_entry: None,
            permission_mode: None,
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
        let parts = pricing::cost_parts(&string_of(Some(model)), Some(ts), &call.tokens);
        self.priced(model, call, ts, parts.map(|c| c.total));
    }

    /// A call at what its app said it cost (Grok Build's turns), or, when
    /// it didn't say (`cost` None), one without a price.
    pub fn add_reported(&mut self, model: &Value, call: &Call, ts: f64, cost: Option<f64>) {
        self.priced(model, call, ts, cost);
    }

    fn priced(&mut self, model: &Value, call: &Call, ts: f64, cost: Option<f64>) {
        let day = day_key(ts);
        let tag = str_tag(&day);
        if !self.spend.contains(&tag) {
            sset(&mut self.spend, &day, Spend::default());
            while self.spend.len() > 2 {
                drop_first(&mut self.spend);
            }
        }
        let name = string_of(Some(model));
        let d = &mut self.spend.get_mut(&tag).expect("added above when missing, and the newest stays").1;
        match cost {
            None => {
                let plain = pricing::plain_model(&name);
                let unpriced = if !plain.is_empty() {
                    plain
                } else if truthy(Some(model)) {
                    display(model)
                } else {
                    "unknown".into()
                };
                if !d.unpriced.contains(&unpriced) {
                    d.unpriced.push(unpriced);
                }
            }
            Some(c) => {
                d.cost += c;
                d.priced = true;
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
        if trim(t).is_empty() {
            return Ok(());
        }
        self.messages.push(Message { text: Value::String(tail(t, 1500)), ts: number(ts) });
        self.texts += 1;
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
        Ok(())
    }

    /// The last prompt, now with the whole text its later chunks brought
    /// (Grok Build's runs); the first prompt too, when it's that one.
    pub fn extend_prompt(&mut self, text: &str) {
        let Some(p) = self.prompts.last_mut() else { return };
        p.text = Value::String(tail(text, 1200));
        if let Some(first) = self.first_prompt.as_mut().filter(|f| f.seq == p.seq) {
            first.text = p.text.clone();
        }
        self.texts += 1;
    }

    /// The last message, now with the whole text its later chunks brought.
    pub fn extend_message(&mut self, text: &str) {
        let Some(m) = self.messages.last_mut() else { return };
        m.text = Value::String(tail(text, 1500));
        self.texts += 1;
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
            self.turn_contexts.push(to_number(Some(&self.context_used)));
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

    /// What it said last, its last 280 characters, or null.
    pub fn last_message(&self) -> Value {
        self.messages.last().map_or(Value::Null, |m| Value::String(tail(&string_of(Some(&m.text)), 280)))
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
                if !same || t.rec.get("ok") != Some(&Value::Bool(false)) || now - to_number(t.rec.get("ts")) > 10.0 * 60e3 {
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
        let window = if truthy(Some(&self.context_window)) { to_number(Some(&self.context_window)) } else { 1.0 };
        let growths: Vec<f64> = self.turn_contexts.windows(2).map(|w| w[1] - w[0]).filter(|d| *d > 0.0).collect();
        let growth = median(&growths);
        let threshold = window * 0.9;
        let used = to_number(Some(&self.context_used));
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
            let age = now - to_number(Some(ts));
            age.is_nan() || age <= FILE_MEMORY_MS
        });
        let mut subs: Vec<Value> = self
            .subagents
            .iter()
            .map(|(_, (_, a))| {
                let mut m: Map<String, Value> = a.iter().filter(|(k, _)| k.as_str() != "announced").map(|(k, v)| (k.clone(), v.clone())).collect();
                let active = !truthy(a.get("done")) && now - to_number(a.get("lastAt")) < SUBAGENT_IDLE_MS;
                m.insert("active".into(), Value::Bool(active));
                Value::Object(m)
            })
            .collect();
        sort_by_num(&mut subs, |a, b| to_number(b.get("lastAt")) - to_number(a.get("lastAt")));
        subs.truncate(12);
        let mut files: Vec<(Value, Value)> = self.files.iter().map(|(_, (k, ts))| (k.clone().unwrap_or(Value::Null), ts.clone())).collect();
        sort_by_num(&mut files, |a, b| to_number(Some(&b.1)) - to_number(Some(&a.1)));
        files.truncate(10);
        let files: Vec<Value> = files.into_iter().map(|(path, ts)| obj(vec![("path", path), ("ts", ts)])).collect();
        let mcp: Vec<Value> = self.mcp.iter().map(|(_, (k, ts))| obj(vec![("server", k.clone().unwrap_or(Value::Null)), ("ts", ts.clone())])).collect();
        let today = self.daily.get(&str_tag(&day_key(now))).map_or(0.0, |(_, v)| *v);
        let last_message = self.last_message();
        let or_null = |v: f64| if v != 0.0 && !v.is_nan() { number(v) } else { Value::Null };
        let mut snap = obj(vec![
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
        ]);
        // How it asks before it acts, once its hooks have said.
        if let (Some(mode), Value::Object(m)) = (&self.permission_mode, &mut snap) {
            m.insert("permissionMode".into(), Value::String(mode.clone()));
        }
        snap
    }

    fn new_uid(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }
}

// ---------- what it emits ----------

/// An event in the history, with its time.
#[derive(Clone, Debug)]
pub struct Event {
    pub ts: f64,
    pub fields: Fields,
}

// ---------- the telemetry ----------

pub struct Telemetry {
    pub home: Option<String>,
    /// The MCP server the config names for the shared context
    /// (wikiServer, the one the folder's marker names when it has one):
    /// Xenolith's tools under it write there, whatever it's called.
    pub context_server: Option<String>,
    roots: Vec<(String, String)>,
    sessions: Vec<Session>,
    index: HashMap<String, usize>,
    history: Vec<Event>,
    pending_label: Option<(String, Value, f64)>,
    // What each uuid-named MCP server says it is (names.rs).
    server_names: ServerNames,
    // The tailed apps' readers, from the adapters in place, in the
    // registry's order; one is out of its place while it reads a line.
    apps: Vec<Option<Box<dyn Tailed>>>,
}

impl Telemetry {
    /// `roots`: [(dir, app)], the folders it reads, as real paths.
    pub fn new(home: Option<String>, roots: Vec<(String, String)>) -> Self {
        Telemetry {
            home,
            context_server: None,
            roots,
            sessions: Vec::new(),
            index: HashMap::new(),
            history: Vec::new(),
            pending_label: None,
            server_names: ServerNames::new(),
            apps: xenonet_adapter::platforms().tailed().into_iter().map(Some).collect(),
        }
    }

    // ----- the apps -----

    // The reader of `app`'s transcripts: any app that isn't tailed is read
    // as Claude Code.
    fn app_index(&self, app: &str) -> Option<usize> {
        let id = apps::tailed_app(app, "claude");
        self.apps.iter().position(|a| a.as_ref().is_some_and(|a| a.id() == id))
    }

    fn app_mut(&mut self, app: &str) -> Option<&mut Box<dyn Tailed>> {
        let k = self.app_index(app)?;
        self.apps[k].as_mut()
    }

    fn apps(&self) -> impl Iterator<Item = &dyn Tailed> {
        self.apps.iter().filter_map(|a| a.as_deref())
    }

    /// An app's provider was asked for plan usage and answered `r`.
    pub fn usage_reading(&mut self, app: &str, r: Option<&Value>, now: f64) -> Result<(), TypeError> {
        match self.app_mut(app) {
            Some(a) => a.usage_reading(r, now),
            None => Ok(()),
        }
    }

    /// Samples another source keeps of an app's plan usage.
    pub fn usage_samples(&mut self, app: &str, samples: &Value) {
        if let Some(a) = self.app_mut(app) {
            a.usage_samples(samples);
        }
    }

    /// An app's plan usage readings settled.
    pub fn settle_usage(&mut self, app: &str) {
        if let Some(a) = self.app_mut(app) {
            a.settle_usage();
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

    /// What a transcript is: { app, sessionId } (with agentId for a
    /// subagent's), or None to leave it alone. The app whose folder holds
    /// the file answers for it; elsewhere, the first app that knows the
    /// file by its name.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
    pub fn classify(&self, file: &str) -> Option<Value> {
        let norm = file.replace('\\', "/");
        if !norm.ends_with(".jsonl") {
            return None;
        }
        let found = |app: &dyn Tailed, c: Classified| {
            obj_opt(vec![("app", sv(app.id())), ("sessionId", Some(Value::String(format!("{}:{}", app.id(), c.id)))), ("agentId", c.agent.map(Value::String))])
        };
        for (dir, app) in &self.roots {
            let dir = dir.replace('\\', "/");
            if !norm.starts_with(&format!("{dir}/")) {
                continue;
            }
            let reader = self.app_index(app).and_then(|k| self.apps[k].as_deref())?;
            return reader.classify(&norm, Some(&dir)).map(|c| found(reader, c));
        }
        self.apps().find_map(|a| a.classify(&norm, None).map(|c| found(a, c)))
    }

    // ----- a line -----

    /// A line the tailer read, through its app's normalizer, as its events
    /// (the app's reader says what it means): its refusal (kept whether
    /// read as history or live, told to you only when live) and what it
    /// says. What it emitted before an error is in `out`. `app` is the
    /// transcript's: any app that isn't tailed is read as Claude Code.
    #[expect(clippy::too_many_arguments, reason = "a line, where it was read, and how, as the readers take them")]
    pub fn line(&mut self, app: &str, vendor: &dyn VendorCheck, f: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let Some(mut reader) = self.app_index(app).and_then(|k| self.apps[k].take()) else {
            return Err(TypeError(format!("XenoNet has no reader for {app}'s transcripts")));
        };
        let read = reader.line(self, vendor, f, raw, live, now, out);
        let k = self.apps.iter().position(Option::is_none).expect("the reader's place is empty while it reads");
        self.apps[k] = Some(reader);
        read
    }

    fn activity(&mut self, i: usize, ev: Fields, live: bool, now: f64, out: &mut Vec<Emit>) {
        let ts = field(&ev, "ts").map_or(f64::NAN, |v| to_number(Some(v)));
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
                if self.history.len() > HISTORY_MAX + 500 {
                    self.trim_history(now);
                }
                out.push(Emit::Activity(full));
            } else {
                self.history.push(Event { ts, fields: full });
                // Reading a pile of busy transcripts would keep every event
                // of the day until the sort at the end; the sort and trim
                // leave the same ones as doing it once.
                if self.history.len() > 4 * HISTORY_MAX {
                    self.primed(now);
                }
            }
        } else if live {
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
        let i = self.session(id, apps::tailed_app(id.split(':').next().unwrap_or_default(), "claude"));
        let name = prop(body, "hook_event_name")?.cloned();
        let s = &mut self.sessions[i];
        if is_str(name.as_ref(), "Notification") {
            let text = nn(opt_prop(body, "message")).cloned().unwrap_or_else(|| "Claude needs your attention".into());
            s.hook_need = obj(vec![("text", Value::String(clip_value(Some(&text), 200))), ("ts", number(now))]);
        } else if name.as_ref().and_then(Value::as_str).is_some_and(|n| ["UserPromptSubmit", "Stop", "SessionEnd", "PostToolUse"].contains(&n)) {
            s.hook_need = Value::Null;
        }
        // How it asks before it acts, as every hook says it: Grok Build's
        // permissionMode, Claude Code's permission_mode. A subagent's isn't
        // its session's.
        if !truthy(opt_prop(body, "subagentType")) {
            if let Some(Value::String(mode)) = opt_prop(body, "permissionMode").filter(|m| m.is_string()).or(opt_prop(body, "permission_mode")) {
                if !mode.is_empty() {
                    s.permission_mode = Some(mode.clone());
                }
            }
        }
        // An app whose lines don't say when a turn ends (Grok Build's) says
        // it in its hooks: Stop, and StopCancelled (you stopped it) and
        // StopFailure, where Stop doesn't run. A subagent's end isn't the
        // turn's.
        if !line_events::lines_end_turns(&self.sessions[i].app) && !truthy(opt_prop(body, "subagentType")) {
            self.sessions[i].hook_at = now;
            let event = name.as_ref().and_then(Value::as_str).unwrap_or_default();
            if ["Stop", "StopCancelled", "StopFailure"].contains(&event) && self.sessions[i].turn_active {
                if event == "StopCancelled" && is_str(opt_prop(body, "reason"), "user_interrupt") {
                    self.interrupted(i, now, true, now, out);
                } else {
                    let duration = self.duration(i, now);
                    self.sessions[i].end_turn(now, false);
                    self.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("ts", nv(now))]), true, now, out);
                }
            }
        }
        let detail = nn(name.as_ref()).cloned().unwrap_or_else(|| "hook".into());
        self.activity(i, ev(vec![("kind", sv("hook")), ("detail", Some(detail)), ("ts", nv(now))]), true, now, out);
        Ok(())
    }

    /// Turns gone quiet in an app whose lines don't say when a turn ends,
    /// where its hooks haven't spoken lately: with no tool call in flight,
    /// each ended when its last line was written.
    pub fn settle(&mut self, now: f64) {
        for s in &mut self.sessions {
            if line_events::lines_end_turns(&s.app) || !s.turn_active || !s.inflight.is_empty() || now - s.hook_at < HOOKS_HEARD_MS || now - s.last_event_at < QUIET_TURN_MS {
                continue;
            }
            let at = s.last_event_at;
            s.end_turn(at, false);
        }
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

    // What you asked: a turn begins.
    fn prompted(&mut self, i: usize, text: &str, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        let s = &mut self.sessions[i];
        s.turn_active = true;
        s.turn_started_at = Some(ts);
        s.add_prompt(text, ts);
        s.run_entry = Some(Run::Prompt);
        self.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(text, 90)))), ("ts", nv(ts))]), live, now, out);
    }

    // A chunk that carries on a prompt or a message (Grok Build's runs):
    // the whole text so far takes the place of what was kept of it, or is
    // kept now, once there's something in it, when its first chunks had
    // nothing to say. The activity names it once, by its text when it's
    // first kept. A thought carries on nothing the tailer keeps.
    #[expect(clippy::too_many_arguments, reason = "a chunk's event and what claude_line worked out about it (session, time), handed down as the JS's closures saw them")]
    fn more(&mut self, i: usize, of: &str, whole: &str, meta: bool, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let s = &mut self.sessions[i];
        s.last_event_at = js_max(s.last_event_at, ts);
        match of {
            "text" => {
                if s.run_entry == Some(Run::Message) {
                    s.extend_message(whole);
                    return Ok(());
                }
                if trim(whole).is_empty() {
                    return Ok(());
                }
                let text = Value::String(whole.to_string());
                s.add_message(Some(&text), ts)?;
                s.run_entry = Some(Run::Message);
                self.activity(i, ev(vec![("kind", sv("message")), ("detail", Some(Value::String(clip_value(Some(&text), 72)))), ("ts", nv(ts))]), live, now, out);
            }
            "prompt" => {
                if s.run_entry == Some(Run::Prompt) {
                    s.extend_prompt(whole);
                    return Ok(());
                }
                if meta || trim(whole).is_empty() || whole.starts_with('<') {
                    return Ok(());
                }
                self.prompted(i, whole, ts, live, now, out);
            }
            _ => {}
        }
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
                    let (a, b) = (display(&ids[i].1), display(&ids[j].1));
                    let mut pair = [a, b];
                    pair.sort_by(|x, y| utf16::cmp_units(x, y));
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
                    let (a, b) = if utf16::cmp_units(&list[j].0, &list[i].0) == Ordering::Less { (&list[j], &list[i]) } else { (&list[i], &list[j]) };
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
                    let ts = js_max(js_max(to_number(c.get("ts")), a.1), b.1);
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
        // The tailed apps, then any other a session is of.
        let mut by_app: Vec<(String, f64, f64)> = apps::with("lines").map(|a| (a.id.clone(), 0.0, 0.0)).collect();
        let tailed = by_app.len();
        let mut unpriced: Vec<String> = Vec::new();
        // The apps with a call today, and those with a price for one: an app
        // with calls and no price costs what isn't known (null, never 0), and
        // so does the day when no call today has one.
        let (mut called, mut priced): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        for s in &self.sessions {
            let Some((_, d)) = s.spend.get(&day) else { continue };
            if !called.contains(&s.app.as_str()) {
                called.push(&s.app);
            }
            if d.priced && !priced.contains(&s.app.as_str()) {
                priced.push(&s.app);
            }
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
        for (app, cost, tokens) in by_app.iter().take(tailed) {
            let tpm = sessions.iter().filter(|x| is_str(opt_prop(Some(x), "app"), app)).fold(0.0, |a, x| a + num_of(Some(x), "tpm"));
            if *tokens > 0.0 {
                per_hour += tpm * 60.0 * (cost / tokens);
            }
        }
        unpriced.sort_by(|a, b| utf16::cmp_units(a, b));
        let unknown = |i: usize, app: &str| i < tailed && called.contains(&app) && !priced.contains(&app);
        let by_app_value: Map<String, Value> =
            by_app.iter().enumerate().map(|(i, (app, cost, _))| (app.clone(), if unknown(i, app) { Value::Null } else { number(*cost) })).collect();
        let today = if !called.is_empty() && priced.is_empty() {
            Value::Null
        } else {
            number(by_app.iter().enumerate().take(tailed).fold(0.0, |a, (i, x)| a + if unknown(i, &x.0) { 0.0 } else { x.1 }))
        };
        obj(vec![
            ("today", today),
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
                    unpriced.sort_by(|a, b| utf16::cmp_units(a, b));
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
        self.settle(now);
        let mut sessions = Vec::new();
        for s in &mut self.sessions {
            if now - s.last_event_at < PRIME_WINDOW_MS {
                sessions.push(s.snapshot(now));
            }
        }
        let tailed: Vec<&str> = apps::with("lines").map(|a| a.id.as_str()).collect();
        let mut series = vec![vec![0.0; RATE_WINDOW_S as usize]; tailed.len()];
        let now_s = (now / 1000.0).floor();
        for s in &self.sessions {
            let Some(which) = tailed.iter().position(|a| *a == s.app) else { continue };
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
        let series_value = Value::Object(tailed.iter().zip(&series).map(|(app, xs)| ((*app).to_string(), Value::Array(xs.iter().map(|x| number(*x)).collect()))).collect());
        let (tpm, today) = (sum("tpm"), sum("tokensToday"));
        // The apps' plan limits, and their forecasts, from their readers.
        let limits = obj(self.apps().filter_map(|a| a.limits(now).map(|v| (a.id(), v))).collect());
        let forecast = Value::Object(self.apps().flat_map(|a| a.forecast(now)).collect());
        let costs = self.costs(now, &sessions);
        let collab = Self::collab(&sessions);
        let collisions = self.collisions(now);
        let resources = Value::Array(
            resources
                .into_iter()
                .map(|(server, ts)| {
                    let found = server.as_str().and_then(|id| self.server_names.name(id));
                    let mut fields = vec![("server", server), ("ts", number(ts))];
                    if let Some(found) = found {
                        fields.push(("name", Value::String(found.name)));
                        fields.push(("nameSource", Value::String(found.source.into())));
                    }
                    obj(fields)
                })
                .collect(),
        );
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
}

/// The telemetry's operations on a session, as an app's reader works on
/// one.
impl Tail for Telemetry {
    fn session(&mut self, id: &str, app: &str) -> usize {
        Telemetry::session(self, id, app)
    }
    fn find(&self, id: &str) -> Option<usize> {
        Telemetry::find(self, id)
    }
    fn session_id(&self, i: usize) -> &str {
        &self.sessions[i].id
    }
    fn session_app(&self, i: usize) -> &str {
        &self.sessions[i].app
    }
    fn home(&self) -> Option<&str> {
        self.home.as_deref()
    }
    fn context_server(&self) -> Option<&str> {
        self.context_server.as_deref()
    }
    fn observe_servers(&mut self, raw: &Value) {
        self.server_names.observe(raw, mcp_server_of);
    }

    fn activity(&mut self, i: usize, ev: Fields, live: bool, now: f64, out: &mut Vec<Emit>) {
        Telemetry::activity(self, i, ev, live, now, out);
    }
    fn note_tool(&mut self, i: usize, t: ToolCall, live: bool, now: f64, out: &mut Vec<Emit>) -> Option<ToolRecord> {
        Telemetry::note_tool(self, i, t, live, now, out)
    }
    fn refusal(&self, app: &str, at: &Where, raw: &Value, signal: &Value, live: bool, out: &mut Vec<Emit>) {
        let model = self.find(&at.session_id).map_or(Value::Null, |i| self.sessions[i].model.clone());
        let cwd = opt_prop(Some(raw), "cwd").filter(|c| c.is_string()).cloned().unwrap_or(Value::Null);
        out.push(Emit::Refusal(obj(vec![
            ("signal", signal.clone()),
            ("sessionId", Value::String(at.session_id.clone())),
            ("app", app.into()),
            ("agentId", at.agent_id.clone()),
            ("file", at.file.clone()),
            ("cwd", cwd),
            ("model", model),
            ("live", Value::Bool(live)),
        ])));
    }
    fn prompted(&mut self, i: usize, text: &str, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        Telemetry::prompted(self, i, text, ts, live, now, out);
    }
    fn more(&mut self, i: usize, of: &str, whole: &str, meta: bool, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        Telemetry::more(self, i, of, whole, meta, ts, live, now, out)
    }
    fn interrupted(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        Telemetry::interrupted(self, i, ts, live, now, out);
    }
    fn duration(&self, i: usize, ts: f64) -> Value {
        Telemetry::duration(self, i, ts)
    }
    fn ask_if_question(&mut self, i: usize, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        Telemetry::ask_if_question(self, i, ts, live, now, out);
    }

    fn turn_active(&self, i: usize) -> bool {
        self.sessions[i].turn_active
    }
    fn begin_turn(&mut self, i: usize, ts: f64) {
        let s = &mut self.sessions[i];
        s.turn_active = true;
        s.turn_started_at = Some(ts);
    }
    fn end_turn(&mut self, i: usize, ts: f64, interrupted: bool) {
        self.sessions[i].end_turn(ts, interrupted);
    }
    fn turn_marks(&self, i: usize) -> f64 {
        self.sessions[i].turn_marks
    }
    fn add_turn_mark(&mut self, i: usize) {
        self.sessions[i].turn_marks += 1.0;
    }
    fn reset_stall(&mut self, i: usize) {
        self.sessions[i].reset_stall();
    }
    fn touch(&mut self, i: usize, ts: f64) {
        let s = &mut self.sessions[i];
        s.last_event_at = js_max(s.last_event_at, ts);
    }
    fn set_run(&mut self, i: usize, run: Option<Run>) {
        self.sessions[i].run_entry = run;
    }

    fn model(&self, i: usize) -> Value {
        self.sessions[i].model.clone()
    }
    fn set_model(&mut self, i: usize, model: Value) {
        self.sessions[i].model = model;
    }
    fn set_context_used(&mut self, i: usize, used: Value) {
        self.sessions[i].context_used = used;
    }
    fn set_context_window(&mut self, i: usize, window: Value) {
        self.sessions[i].context_window = window;
    }
    fn tokens_processed(&self, i: usize) -> f64 {
        self.sessions[i].tokens_processed
    }
    fn add_output_tokens(&mut self, i: usize, n: f64) {
        self.sessions[i].tokens_output += n;
    }
    fn set_output_tokens(&mut self, i: usize, n: f64) {
        self.sessions[i].tokens_output = n;
    }
    fn add_tokens(&mut self, i: usize, n: f64, ts: f64, live: bool) {
        self.sessions[i].add_tokens(n, ts, live);
    }
    fn add_call(&mut self, i: usize, model: &Value, call: &Call, ts: f64) {
        self.sessions[i].add_call(model, call, ts);
    }
    fn add_reported(&mut self, i: usize, model: &Value, call: &Call, ts: f64, cost: Option<f64>) {
        self.sessions[i].add_reported(model, call, ts, cost);
    }
    fn provider(&self, i: usize) -> Value {
        self.sessions[i].provider.clone()
    }
    fn set_provider(&mut self, i: usize, provider: Value) {
        self.sessions[i].provider = provider;
    }
    fn priority(&self, i: usize) -> bool {
        self.sessions[i].priority
    }
    fn set_priority(&mut self, i: usize, priority: bool) {
        self.sessions[i].priority = priority;
    }

    fn add_prompt(&mut self, i: usize, text: &str, ts: f64) {
        self.sessions[i].add_prompt(text, ts);
    }
    fn last_prompt_seq(&self, i: usize) -> Option<u64> {
        self.sessions[i].prompts.last().map(|p| p.seq)
    }
    fn add_message(&mut self, i: usize, text: Option<&Value>, ts: f64) -> Result<(), TypeError> {
        self.sessions[i].add_message(text, ts)
    }
    fn add_edits(&mut self, i: usize, edits: &[Value], ts: f64) -> Result<(), TypeError> {
        self.sessions[i].add_edits(edits, ts)
    }
    fn note_tool_result(&mut self, i: usize, ok: bool, ts: f64, tool: Option<&ToolRecord>) {
        self.sessions[i].note_tool_result(ok, ts, tool);
    }
    fn set_question(&mut self, i: usize, question: Value) {
        self.sessions[i].question = question;
    }
    fn needs_you(&self, i: usize, now: f64) -> Value {
        self.sessions[i].needs_you(now)
    }

    fn subagent(&mut self, i: usize, agent: Option<&Value>) -> Option<&mut Map<String, Value>> {
        self.sessions[i].subagents.get_mut(&key_tag(agent)).map(|(_, a)| a)
    }
    fn add_subagent(&mut self, i: usize, agent: Option<&Value>, record: Map<String, Value>) {
        kset(&mut self.sessions[i].subagents, agent, record);
    }
    fn inflight_add(&mut self, i: usize, id: Option<&Value>, name: Option<&Value>, ts: f64) -> String {
        let mut fields = Map::new();
        if let Some(n) = name {
            fields.insert("name".into(), n.clone());
        }
        fields.insert("ts".into(), number(ts));
        kset(&mut self.sessions[i].inflight, id, Inflight { fields, tool: None })
    }
    fn inflight_set_tool(&mut self, i: usize, tag: &str, tool: Option<ToolRecord>) {
        if let Some((_, e)) = self.sessions[i].inflight.get_mut(tag) {
            e.tool = Some(tool);
        }
    }
    fn inflight_remove(&mut self, i: usize, id: Option<&Value>) -> Option<ToolRecord> {
        self.sessions[i].inflight.remove(&key_tag(id)).and_then(|(_, e)| e.tool).flatten()
    }
    fn was_seen(&self, i: usize, message_id: Option<&Value>) -> bool {
        self.sessions[i].seen_messages.contains(&key_tag(message_id))
    }
    fn mark_seen(&mut self, i: usize, message_id: Option<&Value>) {
        let s = &mut self.sessions[i];
        kset(&mut s.seen_messages, message_id, ());
        if s.seen_messages.len() > 400 {
            drop_first(&mut s.seen_messages);
        }
    }
    fn pending_label(&self) -> Option<&(String, Value, f64)> {
        self.pending_label.as_ref()
    }
    fn set_pending_label(&mut self, session: String, label: Value, ts: f64) {
        self.pending_label = Some((session, label, ts));
    }
}

/// A provider's refusals, as a line's events mark them, read first.
pub fn refusals(t: &dyn Tail, app: &str, at: &Where, raw: &Value, events: &[LineEvent<'_>], live: bool, out: &mut Vec<Emit>) {
    for e in events {
        let Kind::Refusal(signal) = &e.kind else { continue };
        t.refusal(app, at, raw, signal, live, out);
    }
}

/// Straight-line projection of a plan window's use at `now`, over
/// `points` (when, percent): the percent an hour, when it's used up
/// (`eta`, when the use grows), and whether that comes before the reset
/// (`resets_at`, or `horizon` on from now when the reset isn't known: the
/// window's length).
pub fn project_limit(now: f64, points: &[(f64, f64)], pct: Option<&Value>, resets_at: Option<Value>, horizon: f64) -> Value {
    let per_hour = slope(&points.iter().map(|(t, v)| (t / 3600e3, *v)).collect::<Vec<_>>());
    if nn(pct).is_none() {
        return Value::Null;
    }
    let p = to_number(pct);
    let eta = (per_hour > 0.05).then(|| now + ((100.0 - p) / per_hour) * 3600e3);
    let limit = match nn(resets_at.as_ref()) {
        Some(r) => to_number(Some(r)),
        None => now + horizon,
    };
    obj_opt(vec![
        ("pct", pct.cloned()),
        ("perHour", nv(per_hour)),
        ("eta", Some(eta.map_or(Value::Null, number))),
        ("resetsAt", resets_at),
        ("hitsBeforeReset", Some(Value::Bool(eta.is_some_and(|e| e < limit)))),
    ])
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
        let num = |v: &Option<Value>| to_number(v.as_ref());
        let opt_num = |v: &Option<Value>| v.as_ref().filter(|x| !x.is_null()).map(|x| to_number(Some(x)));
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
            "turnContexts" => self.turn_contexts = list_of(v.as_ref()).iter().map(|x| to_number(Some(x))).collect(),
            "model" => self.model = or_null(v),
            "turn" => {
                self.turn_active = truthy(opt_prop(v.as_ref(), "active"));
                self.turn_started_at = opt_prop(v.as_ref(), "startedAt").filter(|x| !x.is_null()).map(|x| to_number(Some(x)));
            }
            "toolCounts" => {
                self.tool_counts = match v {
                    Some(Value::Object(m)) => m,
                    _ => Map::new(),
                }
            }
            "lastTool" => self.last_tool = v.filter(|x| !x.is_null()),
            "rate" => self.rate = list_of(v.as_ref()).iter().map(|r| (to_number(index0(Some(r)).as_ref()), to_number(list_of(Some(r)).get(1)))).collect(),
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
        let num_of_v = |v: Option<Value>| to_number(v.as_ref());
        let list = |v: &Vec<Value>| Value::Array(v.clone());
        let list_of_v = |v: Option<Value>| list_of(v.as_ref());
        let sub = |v: &Map<String, Value>| Value::Object(v.clone());
        let sub_of = |v: Option<Value>| match v {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        let spend = |d: &Spend| {
            obj(vec![
                ("cost", number(d.cost)),
                ("tokens", number(d.tokens)),
                ("unpriced", Value::Array(d.unpriced.iter().cloned().map(Value::String).collect())),
                ("priced", Value::Bool(d.priced)),
            ])
        };
        let spend_of = |v: Option<Value>| Spend {
            cost: num_of(v.as_ref(), "cost"),
            tokens: num_of(v.as_ref(), "tokens"),
            unpriced: list_of(opt_prop(v.as_ref(), "unpriced")).iter().map(display).collect(),
            priced: truthy(opt_prop(v.as_ref(), "priced")),
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
    use xenonet_adapter::reader::Unchecked;

    const NOW: f64 = 1_791_000_000_000.0;

    fn at(ms: f64) -> Value {
        Value::String(xenonet_text::time::iso(ms))
    }

    // A telemetry with the adapters' readers in place.
    fn telemetry(roots: Vec<(String, String)>) -> Telemetry {
        xenonet_platforms::install();
        Telemetry::new(None, roots)
    }

    fn read(t: &mut Telemetry, app: &str, raw: Value) -> Result<Vec<Emit>, TypeError> {
        let mut out = Vec::new();
        let f = Where { session_id: format!("{app}:{}", if app == "claude" { "a" } else { "b" }), agent_id: Value::Null, file: Value::Null };
        t.line(app, &Unchecked, &f, raw, true, NOW, &mut out)?;
        Ok(out)
    }

    fn claude(t: &mut Telemetry, raw: Value) -> Vec<Emit> {
        read(t, "claude", raw).unwrap()
    }

    fn codex(t: &mut Telemetry, raw: Value) -> Vec<Emit> {
        read(t, "codex", raw).unwrap()
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
        let t = telemetry(vec![("/x/.codex/sessions".into(), "codex".into()), ("/g/.grok/sessions".into(), "grok".into())]);
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(t.classify(&format!("/h/.claude/projects/p/{id}.jsonl")).unwrap()["sessionId"], format!("claude:{id}"));
        let sub = t.classify(&format!("/h/.claude/projects/p/{id}/subagents/agent-abc.jsonl")).unwrap();
        assert_eq!((sub["sessionId"].as_str(), sub["agentId"].as_str()), (Some(format!("claude:{id}").as_str()), Some("abc")));
        assert_eq!(t.classify(&format!("/x/.codex/sessions/2026/rollout-2026-T-{id}.jsonl")).unwrap()["sessionId"], format!("codex:{id}"));
        assert_eq!(t.classify(&format!("/x/.codex/sessions/2026/other-{id}.jsonl")), None);
        assert_eq!(t.classify("/g/.grok/sessions/%2Fw/s-1234567/updates.jsonl").unwrap()["sessionId"], "grok:s-1234567");
        assert_eq!(t.classify("/g/.grok/sessions/%2Fw/s-1234567/history.jsonl"), None);
        assert_eq!(t.classify("/h/notes.txt"), None);
        assert_eq!(t.classify("C:\\h\\.claude\\projects\\p\\11111111-2222-3333-4444-55555555555A.jsonl").unwrap()["app"], "claude");
    }

    #[test]
    fn a_claude_turn() {
        let mut t = telemetry(Vec::new());
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
        assert!(read(&mut t, "claude", json!({ "type": "assistant", "message": { "content": [{ "type": "text", "text": 5 }] } })).is_err());
    }

    #[test]
    fn a_codex_turn_and_its_limits() {
        let mut t = telemetry(Vec::new());
        codex(&mut t, json!({ "type": "event_msg", "timestamp": at(NOW - 9000.0), "payload": { "type": "task_started" } }));
        codex(
            &mut t,
            json!({ "type": "event_msg", "timestamp": at(NOW - 8000.0), "payload": { "type": "token_count", "info": { "total_token_usage": { "total_tokens": 5000, "output_tokens": 300 }, "last_token_usage": { "total_tokens": 4000 } }, "rate_limits": { "primary": { "used_percent": 12, "window_minutes": 300 } } } }),
        );
        let i = t.find("codex:b").unwrap();
        let snap = t.at_mut(i).snapshot(NOW);
        assert_eq!((snap["tokens"]["processed"].as_f64(), snap["context"]["used"].as_f64()), (Some(5000.0), Some(4000.0)));
        assert_eq!(t.snapshot(NOW)["limits"]["codex"]["primary"], json!({ "pct": 12, "windowMinutes": 300, "resetsAt": null }));
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
        assert_eq!(full["limitForecast"]["codex"]["pct"], 12);
    }

    #[test]
    fn a_codex_extensions_web_search_is_a_web_search() {
        let mut t = telemetry(Vec::new());
        let done = |k: f64, item: Value| json!({ "type": "event_msg", "timestamp": at(NOW - 5000.0 + k), "payload": { "type": "item_completed", "item": item } });
        let search = codex(&mut t, done(0.0, json!({ "type": "Extension", "kind": "web.search", "query": "", "action": { "type": "search", "query": "zod v4 coerce" } })));
        assert_eq!(kinds(&search), ["tool"]);
        for (k, other) in [(1.0, "image_gen.generation"), (2.0, "clock.sleep")] {
            assert_eq!(kinds(&codex(&mut t, done(k, json!({ "type": "Extension", "kind": other })))), Vec::<String>::new(), "{other} isn't read");
        }
        let details: Vec<_> = t.history_since(0.0).iter().filter_map(|e| field(&e.fields, "detail").cloned()).collect();
        assert_eq!(details, [json!("zod v4 coerce")]);
    }

    #[test]
    fn collisions_and_costs() {
        let mut t = telemetry(Vec::new());
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
        let mut t = telemetry(Vec::new());
        let i = t.session("claude:a", "claude");
        let mut out = Vec::new();
        for ago in [5.0, 4.0, 40.0, 3.0] {
            t.note_activity(i, ev(vec![("kind", sv("tool")), ("ts", nv(NOW - ago * 1000.0))]), true, NOW, &mut out);
        }
        assert_eq!(t.history_since(NOW - 10e3).len(), 3);
        assert_eq!(t.history_since(0.0)[0].ts, NOW - 40e3);
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
