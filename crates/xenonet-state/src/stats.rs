//! core/stats.js: a session's stats, from its transcript: the models it used
//! and when, its tokens and what they'd cost at API prices on the day
//! (pricing.rs), its turns and how long they took, its tools, its
//! subagents, and how full its context got.
//!
//! Claude Code writes one line per content block, repeating the message's
//! usage, so calls are counted once per message id; subagents write files of
//! their own. Codex writes a token_count event after each call, with that
//! call's tokens in last_token_usage (cached and written tokens are part of
//! input_tokens), the model in the turn_context before it, and the service
//! tier (the priority tier costs more) in the thread's settings.
//!
//! The figures are added up as JS adds them: a token count a transcript
//! writes as a string is joined on as a string, and what JS would throw on
//! (a value that can't be made a primitive, a line view's TypeError) is a
//! TypeError here, which the twin throws.

use regex::Regex;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use xenonet_base::text::{js_number, js_trim, truthy};
use xenonet_base::{fs as xfs, json, jsstr, paths, pricing};
use xenonet_sources::lines::{has_user_messages, is_codex_prompt, opt_prop, prop, ClaudeLine, CodexLine, Js, TypeError};
use xenonet_sources::live::{categorize, clip, mcp_server_of};
use xenonet_sources::wiki::{to_str, to_str_or_empty, Out};

const CONTEXT_POINTS: usize = 400;

// Object.prototype's properties, which every plain object has already.
const INHERITED: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

// ---------- JS's values, for adding up ----------

/// A primitive, as JS's `+` and `<` see a value.
#[derive(Clone, Debug, PartialEq)]
enum Prim {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
}

impl Prim {
    // ToPrimitive: an array or an object is its string.
    fn of(v: Js<'_>) -> Result<Prim, TypeError> {
        Ok(match v {
            None => Prim::Undef,
            Some(Value::Null) => Prim::Null,
            Some(Value::Bool(b)) => Prim::Bool(*b),
            Some(Value::Number(n)) => Prim::Num(n.as_f64().unwrap_or(f64::NAN)),
            Some(Value::String(s)) => Prim::Str(s.clone()),
            Some(v) => Prim::Str(to_str(Some(v))?),
        })
    }

    fn num(&self) -> f64 {
        match self {
            Prim::Undef => f64::NAN,
            Prim::Null => 0.0,
            Prim::Bool(b) => f64::from(u8::from(*b)),
            Prim::Num(n) => *n,
            Prim::Str(s) => js_number(Some(&Value::String(s.clone()))),
        }
    }

    fn string(&self) -> String {
        match self {
            Prim::Undef => "undefined".into(),
            Prim::Null => "null".into(),
            Prim::Bool(b) => b.to_string(),
            Prim::Num(n) => json::number_string(*n),
            Prim::Str(s) => s.clone(),
        }
    }

    // a + b.
    fn add(&self, b: &Prim) -> Prim {
        if matches!(self, Prim::Str(_)) || matches!(b, Prim::Str(_)) {
            Prim::Str(self.string() + &b.string())
        } else {
            Prim::Num(self.num() + b.num())
        }
    }

    fn out(&self) -> Out {
        match self {
            Prim::Undef => Out::Undef,
            Prim::Null => Out::Null,
            Prim::Bool(b) => Out::Bool(*b),
            Prim::Num(n) => Out::Num(*n),
            Prim::Str(s) => Out::Str(s.clone()),
        }
    }
}

// a < b: None where JS's comparison is undefined (a NaN).
fn less(a: &Prim, b: &Prim) -> Option<bool> {
    if let (Prim::Str(x), Prim::Str(y)) = (a, b) {
        return Some(jsstr::cmp_units(x, y) == Ordering::Less);
    }
    let (x, y) = (a.num(), b.num());
    if x.is_nan() || y.is_nan() {
        None
    } else {
        Some(x < y)
    }
}

fn greater(a: &Prim, b: &Prim) -> bool {
    less(b, a) == Some(true)
}

fn at_most(a: &Prim, b: &Prim) -> bool {
    less(b, a) == Some(false)
}

// Math.max(0, x), and Math.max(a, b).
fn max0(x: f64) -> f64 {
    if x.is_nan() || x > 0.0 {
        x
    } else {
        0.0
    }
}

fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

// A sort's comparator result, as Array.prototype.sort reads it.
fn order(r: f64) -> Ordering {
    if r < 0.0 {
        Ordering::Less
    } else if r > 0.0 {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// How a JS Map or Set tells keys apart: by value, but an object only
/// equals itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Undef,
    Null,
    Bool(bool),
    Num(u64),
    Str(String),
    Obj(u64),
}

/// A value read from a record, and which read made it (two objects are
/// the same only when one read made them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Val {
    pub v: Option<Value>,
    pub id: u64,
}

impl Val {
    fn key(&self) -> Key {
        match &self.v {
            None => Key::Undef,
            Some(Value::Null) => Key::Null,
            Some(Value::Bool(b)) => Key::Bool(*b),
            Some(Value::Number(n)) => {
                let f = n.as_f64().unwrap_or(f64::NAN);
                Key::Num(if f == 0.0 {
                    0f64.to_bits()
                } else if f.is_nan() {
                    f64::NAN.to_bits()
                } else {
                    f.to_bits()
                })
            }
            Some(Value::String(s)) => Key::Str(s.clone()),
            Some(_) => Key::Obj(self.id),
        }
    }

    fn out(&self) -> Out {
        Out::of(self.v.as_ref())
    }

    fn js(&self) -> Js<'_> {
        self.v.as_ref()
    }
}

#[derive(Default)]
struct Ids(u64);

impl Ids {
    fn val(&mut self, v: Js<'_>) -> Val {
        self.0 += 1;
        Val { v: v.cloned(), id: self.0 }
    }
}

// A JS Map's entries, in the order they were first set.
struct Ordered<T> {
    at: HashMap<Key, usize>,
    items: Vec<T>,
}

impl<T> Ordered<T> {
    fn new() -> Self {
        Ordered { at: HashMap::new(), items: Vec::new() }
    }
    fn has(&self, k: &Key) -> bool {
        self.at.contains_key(k)
    }
    fn get_mut(&mut self, k: &Key) -> Option<&mut T> {
        self.at.get(k).map(|&i| &mut self.items[i])
    }
    /// The item at `k`, added last with `make` when there's none.
    fn get_or_insert_with(&mut self, k: Key, make: impl FnOnce() -> T) -> &mut T {
        let i = match self.at.get(&k) {
            Some(&i) => i,
            None => {
                self.at.insert(k, self.items.len());
                self.items.push(make());
                self.items.len() - 1
            }
        };
        &mut self.items[i]
    }
    fn set(&mut self, k: Key, v: T) {
        match self.at.get(&k) {
            Some(&i) => self.items[i] = v,
            None => {
                self.at.insert(k, self.items.len());
                self.items.push(v);
            }
        }
    }
}

// ---------- events ----------

/// What an event is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Prompt,
    Task,
    Spawn,
    End,
    Compact,
    Call,
    Tool,
    Result,
    Other,
}

/// A call's token figure: as the record wrote it, or worked out.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    V(Option<Value>),
    N(f64),
}

impl Default for Tok {
    fn default() -> Self {
        Tok::V(None)
    }
}

impl Tok {
    fn of(v: Js<'_>) -> Tok {
        Tok::V(v.cloned())
    }
    // `v ?? 0`.
    fn or0(v: Js<'_>) -> Tok {
        match v {
            None | Some(Value::Null) => Tok::N(0.0),
            v => Tok::V(v.cloned()),
        }
    }
    fn prim(&self) -> Result<Prim, TypeError> {
        match self {
            Tok::V(v) => Prim::of(v.as_ref()),
            Tok::N(n) => Ok(Prim::Num(*n)),
        }
    }
    // As the pricing twin reads a figure: nothing for null, else Number(v).
    fn number(&self) -> Result<Option<f64>, TypeError> {
        match self {
            Tok::V(None | Some(Value::Null)) => Ok(None),
            t => Ok(Some(t.prim()?.num())),
        }
    }
}

/// Calls, prompts, turn ends and tool calls, as stats reads them from a
/// transcript.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub t: f64,
    pub agent: Val,
    pub model: Val,
    pub effort: Val,
    /// input, cacheRead, cacheWrite5m, cacheWrite1h, output.
    pub tokens: [Tok; 5],
    pub fast: Option<Value>,
    pub priority: Option<Value>,
    pub window: Option<Value>,
    pub id: Val,
    pub name: Val,
    pub text: Option<Value>,
    pub error: Option<Value>,
}

impl Event {
    fn new(kind: Kind, t: f64) -> Event {
        Event {
            kind,
            t,
            agent: Val::default(),
            model: Val::default(),
            effort: Val::default(),
            tokens: Default::default(),
            fast: None,
            priority: None,
            window: None,
            id: Val::default(),
            name: Val::default(),
            text: None,
            error: None,
        }
    }

    /// An event as JS hands one to summarize: { kind, t, agentId, model,
    /// effort, tokens, fast, priority, window, id, name, text, error }.
    pub fn from_js(e: &Value, ids: &mut EventIds) -> Result<Event, TypeError> {
        let e = Some(e);
        let kind = match prop(e, "kind")?.and_then(Value::as_str) {
            Some("prompt") => Kind::Prompt,
            Some("task") => Kind::Task,
            Some("spawn") => Kind::Spawn,
            Some("end") => Kind::End,
            Some("compact") => Kind::Compact,
            Some("call") => Kind::Call,
            Some("tool") => Kind::Tool,
            Some("result") => Kind::Result,
            _ => Kind::Other,
        };
        let mut ev = Event::new(kind, js_number(prop(e, "t")?));
        let ids = &mut ids.0;
        ev.agent = ids.val(prop(e, "agentId")?);
        ev.model = ids.val(prop(e, "model")?);
        ev.effort = ids.val(prop(e, "effort")?);
        let k = prop(e, "tokens")?;
        for (i, name) in ["input", "cacheRead", "cacheWrite5m", "cacheWrite1h", "output"].iter().enumerate() {
            ev.tokens[i] = Tok::of(opt_prop(k, name));
        }
        ev.fast = prop(e, "fast")?.cloned();
        ev.priority = prop(e, "priority")?.cloned();
        ev.window = prop(e, "window")?.cloned();
        ev.id = ids.val(prop(e, "id")?);
        ev.name = ids.val(prop(e, "name")?);
        ev.text = prop(e, "text")?.cloned();
        ev.error = prop(e, "error")?.cloned();
        Ok(ev)
    }
}

/// Where each value an event carries came from, so the same object
/// counts once.
#[derive(Default)]
pub struct EventIds(Ids);

fn is_str(v: Js<'_>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

// `v ?? fallback`.
fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

/// A transcript's file, read: whose it is (null for the session's own
/// file, a subagent's id for its own), and its lines' records.
pub struct File {
    pub agent: Option<Value>,
    pub lines: Vec<Value>,
}

static COMMAND: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\A<command-name>").expect("a fixed pattern"));

/// What someone typed: a slash command as /name and its arguments; nothing
/// for Claude Code's own notes (command output and the like).
pub fn prompt_text(text: &str) -> Option<String> {
    let t = js_trim(text);
    if let Some(m) = COMMAND.find(t) {
        // \/?([^<]+)<\/command-name>, with the slash taken first when that
        // leaves a name.
        let rest = &t[m.end()..];
        let name_at = |from: usize| -> Option<&str> {
            let run = rest[from..].find('<').map_or(rest.len(), |i| from + i);
            (run > from && rest[run..].starts_with("</command-name>")).then(|| &rest[from..run])
        };
        let name = if rest.starts_with('/') { name_at(1).or_else(|| name_at(0)) } else { name_at(0) };
        if let Some(name) = name {
            let args = t.find("<command-args>").and_then(|i| {
                let from = i + "<command-args>".len();
                t[from..].find("</command-args>").map(|j| &t[from..from + j])
            });
            return Some(js_trim(&format!("/{name} {}", args.unwrap_or(""))).to_string());
        }
    }
    (!t.is_empty() && !t.starts_with('<')).then(|| t.to_string())
}

// `for (const b of v)`: an array's items, nothing from a string's
// characters (none has a type), and the TypeError JS throws for the rest.
fn blocks_of(v: &Value) -> Result<&[Value], TypeError> {
    match v {
        Value::Array(a) => Ok(a),
        Value::String(_) => Ok(&[]),
        _ => Err(TypeError("o.blocks is not iterable".into())),
    }
}

/// Calls, prompts, turn ends and tool calls, in the order they happened,
/// from Claude Code's main file and its subagents' files.
pub fn claude_events(files: Vec<File>, ids: &mut EventIds) -> Result<Vec<Event>, TypeError> {
    let ids = &mut ids.0;
    let mut events = Vec::new();
    let mut seen: HashSet<Key> = HashSet::new();
    for f in files {
        let agent = ids.val(f.agent.as_ref());
        let sub = truthy(f.agent.as_ref());
        for raw in f.lines {
            let o = ClaudeLine { raw };
            // A line that doesn't say when is left out.
            let t = o.ts()?;
            if !t.is_finite() {
                continue;
            }
            let kind = o.kind()?;
            if is_str(kind, "assistant") {
                // Each of a message's lines repeats its usage: counted once.
                let id = ids.val(o.message_id()?);
                let usage = if truthy(id.js()) && !seen.contains(&id.key()) { o.usage()? } else { None };
                if let Some(u) = usage {
                    seen.insert(id.key());
                    let mut e = Event::new(Kind::Call, t);
                    e.agent = agent.clone();
                    e.model = ids.val(Some(nn(o.model()?).unwrap_or(&Value::String("unknown".into()))));
                    e.effort = ids.val(o.effort()?);
                    e.tokens = [
                        Tok::of(Some(&u.input)),
                        Tok::of(Some(&u.cache_read)),
                        u.cache_write5m.map_or_else(Tok::N, |v| Tok::V(Some(v))),
                        Tok::of(Some(&u.cache_write1h)),
                        Tok::of(Some(&u.output)),
                    ];
                    e.fast = Some(Value::Bool(u.fast));
                    events.push(e);
                }
                for b in blocks_of(o.blocks()?)? {
                    let b = Some(b);
                    if !is_str(prop(b, "type")?, "tool_use") {
                        continue;
                    }
                    let name = prop(b, "name")?;
                    let mut e = Event::new(Kind::Tool, t);
                    e.id = ids.val(prop(b, "id")?);
                    e.name = ids.val(name);
                    e.agent = agent.clone();
                    events.push(e);
                    if !sub && matches!(to_str(name)?.as_str(), "Agent" | "Task") {
                        let description = opt_prop(prop(b, "input")?, "description");
                        if truthy(description) {
                            let mut e = Event::new(Kind::Spawn, t);
                            e.text = description.cloned();
                            events.push(e);
                        }
                    }
                }
                if o.end_turn()? && !sub {
                    events.push(Event::new(Kind::End, t));
                }
            } else if is_str(kind, "user") {
                if let Some(results) = o.results()? {
                    for r in results {
                        let mut e = Event::new(Kind::Result, t);
                        e.id = ids.val(r.id.as_ref());
                        e.error = Some(Value::Bool(r.error));
                        events.push(e);
                    }
                    continue;
                }
                // A subagent's first prompt is its task.
                if sub {
                    let task = o.text()?;
                    if !js_trim(&task).is_empty() {
                        let mut e = Event::new(Kind::Task, t);
                        e.agent = agent.clone();
                        e.text = Some(Value::String(task));
                        events.push(e);
                    }
                    continue;
                }
                if truthy(o.meta()?) || truthy(o.sidechain()?) {
                    continue;
                }
                // A compacted context carries on the turn it interrupted.
                if truthy(o.compact_summary()?) {
                    events.push(Event::new(Kind::Compact, t));
                    continue;
                }
                if o.interruption()? {
                    continue;
                }
                if let Some(text) = prompt_text(&o.text()?) {
                    let mut e = Event::new(Kind::Prompt, t);
                    e.text = Some(Value::String(text));
                    events.push(e);
                }
            }
        }
    }
    xenonet_base::sort::sort_by(&mut events, |a, b| order(a.t - b.t));
    Ok(events)
}

// /^(priority|fast)$/i.test(tier ?? "").
fn priority_tier(tier: Js<'_>) -> Result<bool, TypeError> {
    Ok(pricing::priority_tier(&to_str_or_empty(tier)?))
}

fn type_is(v: Js<'_>, names: &[&str]) -> Result<bool, TypeError> {
    let s = to_str(v)?;
    Ok(names.contains(&s.as_str()))
}

static EMPTY: LazyLock<Value> = LazyLock::new(|| Value::Object(serde_json::Map::default()));

/// A Codex rollout's calls, prompts, turn ends and tool calls.
pub fn codex_events(lines: Vec<Value>, ids: &mut EventIds) -> Result<Vec<Event>, TypeError> {
    let ids = &mut ids.0;
    let lines: Vec<CodexLine> = lines.into_iter().map(|raw| CodexLine { raw }).collect();
    let mut events = Vec::new();
    // Its prompts are its completed user messages, or in a rollout with none
    // at all, its user_message events (see lines.rs).
    let items = has_user_messages(&lines)?;
    let mut model = ids.val(Some(&Value::String("unknown".into())));
    let mut effort = ids.val(Some(&Value::Null));
    let mut priority = false;
    let mut total = Prim::Num(0.0);
    let mut provider: Value = Value::Null;
    for o in &lines {
        // A line that doesn't say when is left out.
        let t = o.ts()?;
        if !t.is_finite() {
            continue;
        }
        let p = Some(o.payload()?);
        let kind = o.kind()?;
        if is_str(kind, "session_meta") {
            provider = nn(prop(p, "model_provider")?).cloned().unwrap_or(Value::Null);
        }
        if is_str(kind, "turn_context") {
            if let Some(m) = nn(prop(p, "model")?) {
                model = ids.val(Some(m));
            }
            if let Some(e) = nn(prop(p, "effort")?) {
                effort = ids.val(Some(e));
            }
            // Older rollouts say the tier with each turn; newer ones say it in
            // the thread's settings (below), and their turns don't say it.
            let tier = prop(p, "service_tier")?;
            if tier.is_some() {
                priority = priority_tier(tier)?;
            }
        } else if is_str(kind, "event_msg") {
            let said = if items { o.user_message()? } else { o.user_event()? };
            if let Some(said) = said {
                let said = Value::String(said);
                if is_codex_prompt(Some(&said)) {
                    let mut e = Event::new(Kind::Prompt, t);
                    e.text = Some(said);
                    events.push(e);
                }
                continue;
            }
            let pt = prop(p, "type")?;
            if is_str(pt, "thread_settings_applied") {
                // All of the thread's settings, before the turn they apply to:
                // a tier not said is the default one.
                priority = priority_tier(opt_prop(prop(p, "thread_settings")?, "service_tier"))?;
            } else if is_str(pt, "task_complete") {
                events.push(Event::new(Kind::End, t));
            } else if is_str(pt, "token_count") && truthy(prop(p, "info")?) {
                let info = prop(p, "info")?;
                let now = Prim::of(Some(nn(opt_prop(prop(info, "total_token_usage")?, "total_tokens")).unwrap_or(&Value::from(0))))?;
                // The same call is reported more than once; count it when the total moves.
                if at_most(&now, &total) {
                    continue;
                }
                total = now;
                let u = Some(nn(prop(info, "last_token_usage")?).unwrap_or(&EMPTY));
                let input = match nn(prop(u, "input_tokens")?) {
                    Some(v) => Tok::of(Some(v)),
                    None => Tok::N(max0(Tok::or0(prop(u, "total_tokens")?).prim()?.num() - Tok::or0(prop(u, "output_tokens")?).prim()?.num())),
                };
                let cached = Tok::or0(prop(u, "cached_input_tokens")?);
                let write = Tok::or0(prop(u, "cache_write_input_tokens")?);
                let fresh = max0(input.prim()?.num() - cached.prim()?.num() - write.prim()?.num());
                let mut e = Event::new(Kind::Call, t);
                e.agent = ids.val(Some(&Value::Null));
                e.model = if truthy(Some(&provider)) && provider.as_str() != Some("openai") {
                    ids.val(Some(&Value::String(format!("{}/{}", to_str(Some(&provider))?, to_str(model.js())?))))
                } else {
                    model.clone()
                };
                e.effort = effort.clone();
                e.tokens = [Tok::N(fresh), cached, write, Tok::N(0.0), Tok::or0(prop(u, "output_tokens")?)];
                e.priority = Some(Value::Bool(priority));
                e.window = Some(nn(prop(info, "model_context_window")?).cloned().unwrap_or(Value::Null));
                events.push(e);
            }
        } else if is_str(kind, "response_item") && type_is(prop(p, "type")?, &["function_call", "custom_tool_call", "local_shell_call"])? {
            let mut e = Event::new(Kind::Tool, t);
            e.id = ids.val(match nn(prop(p, "call_id")?) {
                Some(v) => Some(v),
                None => prop(p, "id")?,
            });
            e.name = ids.val(match nn(prop(p, "name")?) {
                Some(v) => Some(v),
                None => prop(p, "type")?,
            });
            e.agent = ids.val(Some(&Value::Null));
            events.push(e);
        } else if is_str(kind, "response_item") && type_is(prop(p, "type")?, &["function_call_output", "custom_tool_call_output", "local_shell_call_output"])? {
            let mut e = Event::new(Kind::Result, t);
            e.id = ids.val(match nn(prop(p, "call_id")?) {
                Some(v) => Some(v),
                None => prop(p, "id")?,
            });
            e.error = Some(Value::Null);
            events.push(e);
        }
    }
    Ok(events)
}

// ---------- adding it up ----------

struct Turn {
    n: f64,
    start: f64,
    end: f64,
    prompt: Option<String>,
    tokens: Prim,
    cost: f64,
    by_model: Vec<(String, Prim, f64)>,
    tools: f64,
    errors: f64,
    ended: bool,
}

struct ModelSum {
    name: Val,
    calls: f64,
    tokens: Prim,
    cost: f64,
    priced: bool,
    from: f64,
    to: f64,
    efforts: Ordered<Val>,
}

struct SubSum {
    id: Val,
    calls: f64,
    tokens: Prim,
    cost: f64,
    model: Val,
    from: f64,
    to: f64,
}

struct ToolSum {
    name: Val,
    category: &'static str,
    server: Option<String>,
    calls: f64,
    errors: f64,
    ms: f64,
    timed: f64,
}

#[derive(Clone)]
struct Point {
    t: f64,
    used: Prim,
    window: Out,
}

fn clipped(v: Js<'_>, n: usize) -> Result<String, TypeError> {
    Ok(clip(&to_str_or_empty(v)?, n))
}

/// What a session's events add up to, as the stats panel shows it. `app`
/// is the app's name ("claude" counts output tokens into the context).
pub fn summarize(events: &[Event], app: Js<'_>) -> Result<Out, TypeError> {
    let claude = is_str(app, "claude");
    let (mut calls, mut unpriced_calls, mut cost) = (0.0, 0.0, 0.0);
    let mut kind_tokens = [Prim::Num(0.0), Prim::Num(0.0), Prim::Num(0.0), Prim::Num(0.0)];
    let mut total_tokens = Prim::Num(0.0);
    let mut kind_cost = [0.0; 4];
    let mut models: Ordered<ModelSum> = Ordered::new();
    let mut subagents: Ordered<SubSum> = Ordered::new();
    let mut tools: Ordered<ToolSum> = Ordered::new();
    let mut open: HashMap<Key, (Val, f64)> = HashMap::new();
    let mut turns: Vec<Turn> = Vec::new();
    let mut context: Vec<Point> = Vec::new();
    let mut compactions: Vec<f64> = Vec::new();
    // A subagent's task, and the Agent calls whose descriptions name the
    // subagents they start.
    let mut labels: HashMap<Key, String> = HashMap::new();
    let mut spawns: Vec<&Event> = Vec::new();
    let mut unpriced: Ordered<Val> = Ordered::new();
    let start_turn = |turns: &mut Vec<Turn>, t: f64, prompt: Js<'_>| -> Result<(), TypeError> {
        let prompt = match prompt {
            None | Some(Value::Null) => None,
            p => Some(clipped(p, 140)?),
        };
        turns.push(Turn { n: (turns.len() + 1) as f64, start: t, end: t, prompt, tokens: Prim::Num(0.0), cost: 0.0, by_model: Vec::new(), tools: 0.0, errors: 0.0, ended: false });
        Ok(())
    };

    for e in events {
        match e.kind {
            Kind::Prompt => {
                start_turn(&mut turns, e.t, e.text.as_ref())?;
                continue;
            }
            Kind::Task => {
                if let Entry::Vacant(v) = labels.entry(e.agent.key()) {
                    v.insert(clipped(e.text.as_ref(), 80)?);
                }
                continue;
            }
            Kind::Spawn => {
                spawns.push(e);
                continue;
            }
            _ => {}
        }
        if turns.is_empty() {
            start_turn(&mut turns, e.t, None)?;
        }
        let turn = turns.last_mut().expect("a turn was started above when there was none");
        if !turn.ended || e.kind == Kind::End {
            turn.end = max(turn.end, e.t);
        }
        match e.kind {
            Kind::End => turn.ended = true,
            Kind::Compact => {
                compactions.push(e.t);
                turn.ended = false;
            }
            Kind::Call => {
                // A call after the turn ended (a subagent finishing, a hook)
                // still counts towards it.
                turn.end = max(turn.end, e.t);
                let k = &e.tokens;
                let model = to_str_or_empty(e.model.js())?;
                let figures =
                    pricing::Tokens { input: k[0].number()?, cache_read: k[1].number()?, cache_write5m: k[2].number()?, cache_write1h: k[3].number()?, output: k[4].number()? };
                let parts = pricing::cost_parts(&model, Some(e.t), &figures, truthy(e.fast.as_ref()), truthy(e.priority.as_ref()));
                let [input, cache_read, write5m, write1h, output] = [k[0].prim()?, k[1].prim()?, k[2].prim()?, k[3].prim()?, k[4].prim()?];
                let tokens = [input.clone(), cache_read.clone(), write5m.add(&write1h), output.clone()];
                let sum = tokens.iter().fold(Prim::Num(0.0), |a, x| a.add(x));
                let paid = parts.map_or(0.0, |c| c.total);
                calls += 1.0;
                total_tokens = total_tokens.add(&sum);
                for (i, x) in tokens.iter().enumerate() {
                    kind_tokens[i] = kind_tokens[i].add(x);
                }
                let plain = pricing::plain_model(&model);
                let name = if plain.is_empty() { e.model.clone() } else { Val { v: Some(Value::String(plain.clone())), id: 0 } };
                match parts {
                    Some(c) => {
                        cost += c.total;
                        for (i, x) in [c.input, c.cache_read, c.cache_write, c.output].into_iter().enumerate() {
                            kind_cost[i] += x;
                        }
                    }
                    None => {
                        unpriced_calls += 1.0;
                        unpriced.set(name.key(), name.clone());
                    }
                }
                let m = models.get_or_insert_with(name.key(), || ModelSum {
                    name: name.clone(),
                    calls: 0.0,
                    tokens: Prim::Num(0.0),
                    cost: 0.0,
                    priced: parts.is_some(),
                    from: e.t,
                    to: e.t,
                    efforts: Ordered::new(),
                });
                m.calls += 1.0;
                m.tokens = m.tokens.add(&sum);
                m.cost += paid;
                m.to = e.t;
                if truthy(e.effort.js()) && !m.efforts.has(&e.effort.key()) {
                    m.efforts.set(e.effort.key(), e.effort.clone());
                }
                turn.tokens = turn.tokens.add(&sum);
                turn.cost += paid;
                // A key of the turn's byModel: the name as a string. `??=`
                // finds what every object inherits (constructor, toString,
                // __proto__…) already there, so those never get one.
                let key = if plain.is_empty() { to_str(e.model.js())? } else { plain };
                if !INHERITED.contains(&key.as_str()) {
                    match turn.by_model.iter_mut().find(|(k, _, _)| *k == key) {
                        Some(tm) => {
                            tm.1 = tm.1.add(&sum);
                            tm.2 += paid;
                        }
                        None => turn.by_model.push((key, Prim::Num(0.0).add(&sum), paid)),
                    }
                }
                if truthy(e.agent.js()) {
                    let a = subagents.get_or_insert_with(e.agent.key(), || SubSum {
                        id: e.agent.clone(),
                        calls: 0.0,
                        tokens: Prim::Num(0.0),
                        cost: 0.0,
                        model: name.clone(),
                        from: e.t,
                        to: e.t,
                    });
                    a.calls += 1.0;
                    a.tokens = a.tokens.add(&sum);
                    a.cost += paid;
                    a.to = e.t;
                } else {
                    let used = input.add(&cache_read).add(&write5m).add(&write1h).add(if claude { &output } else { &Prim::Num(0.0) });
                    let window = match nn(e.window.as_ref()) {
                        Some(w) => Out::Json(w.clone()),
                        None => Out::Num(if greater(&used, &Prim::Num(200_000.0)) || model.to_ascii_lowercase().contains("[1m]") { 1_000_000.0 } else { 200_000.0 }),
                    };
                    context.push(Point { t: e.t, used, window });
                }
            }
            Kind::Tool => {
                turn.tools += 1.0;
                open.insert(e.id.key(), (e.name.clone(), e.t));
                if !tools.has(&e.name.key()) {
                    let name = to_str_or_empty(e.name.js())?;
                    tools.set(
                        e.name.key(),
                        ToolSum { name: e.name.clone(), category: categorize(&name), server: mcp_server_of(&name), calls: 0.0, errors: 0.0, ms: 0.0, timed: 0.0 },
                    );
                }
                tools.get_mut(&e.name.key()).expect("added above when missing").calls += 1.0;
            }
            Kind::Result => {
                let Some((name, at)) = open.remove(&e.id.key()) else { continue };
                if let Some(t) = tools.get_mut(&name.key()) {
                    t.ms += max0(e.t - at);
                    t.timed += 1.0;
                    if truthy(e.error.as_ref()) {
                        t.errors += 1.0;
                        turn.errors += 1.0;
                    }
                }
            }
            _ => {}
        }
    }

    let priced = calls > unpriced_calls;
    let priced_or_null = |x: f64| if priced { Out::Num(x) } else { Out::Null };
    let first = events.first().map_or(Out::Null, |e| Out::Num(e.t));
    let last = events.last().map_or(Out::Null, |e| Out::Num(e.t));
    let active = turns.iter().fold(0.0, |a, t| a + (t.end - t.start));
    let totals = Out::obj(vec![
        ("calls", Out::Num(calls)),
        ("input", kind_tokens[0].out()),
        ("cacheRead", kind_tokens[1].out()),
        ("cacheWrite", kind_tokens[2].out()),
        ("output", kind_tokens[3].out()),
        ("tokens", total_tokens.out()),
        ("cost", priced_or_null(cost)),
        ("unpricedCalls", Out::Num(unpriced_calls)),
    ]);
    let kinds = Out::Obj(
        ["input", "cacheRead", "cacheWrite", "output"]
            .iter()
            .enumerate()
            .map(|(i, x)| (x.to_string(), Out::obj(vec![("tokens", kind_tokens[i].out()), ("cost", priced_or_null(kind_cost[i]))])))
            .collect(),
    );

    let mut model_list: Vec<&ModelSum> = models.items.iter().collect();
    let cost_of = |m: &ModelSum| if m.priced { m.cost } else { -1.0 };
    xenonet_base::sort::sort_by(&mut model_list, |a, b| {
        let by_cost = cost_of(b) - cost_of(a);
        order(if by_cost != 0.0 && !by_cost.is_nan() { by_cost } else { b.tokens.num() - a.tokens.num() })
    });
    let models_out = model_list
        .into_iter()
        .map(|m| {
            Out::obj(vec![
                ("model", m.name.out()),
                ("calls", Out::Num(m.calls)),
                ("tokens", m.tokens.out()),
                ("cost", if m.priced { Out::Num(m.cost) } else { Out::Null }),
                ("priced", Out::Bool(m.priced)),
                ("from", Out::Num(m.from)),
                ("to", Out::Num(m.to)),
                ("efforts", Out::Arr(m.efforts.items.iter().map(Val::out).collect())),
            ])
        })
        .collect();

    let turns_out = turns
        .iter()
        .map(|t| {
            Out::obj(vec![
                ("n", Out::Num(t.n)),
                ("start", Out::Num(t.start)),
                ("end", Out::Num(t.end)),
                ("prompt", t.prompt.clone().map_or(Out::Null, Out::Str)),
                ("tokens", t.tokens.out()),
                ("cost", Out::Num(t.cost)),
                ("byModel", Out::Obj(t.by_model.iter().map(|(k, tokens, cost)| (k.clone(), Out::obj(vec![("tokens", tokens.out()), ("cost", Out::Num(*cost))]))).collect())),
                ("tools", Out::Num(t.tools)),
                ("errors", Out::Num(t.errors)),
            ])
        })
        .collect();

    let mut tool_list: Vec<&ToolSum> = tools.items.iter().collect();
    xenonet_base::sort::sort_by(&mut tool_list, |a, b| order(b.calls - a.calls));
    let tools_out = tool_list
        .into_iter()
        .map(|t| {
            Out::obj(vec![
                ("name", t.name.out()),
                ("category", Out::Str(t.category.into())),
                ("server", t.server.clone().map_or(Out::Null, Out::Str)),
                ("calls", Out::Num(t.calls)),
                ("errors", Out::Num(t.errors)),
                ("ms", if t.timed != 0.0 { Out::Num(t.ms) } else { Out::Null }),
            ])
        })
        .collect();

    let mut subs: Vec<&SubSum> = subagents.items.iter().collect();
    xenonet_base::sort::sort_by(&mut subs, |a, b| order(a.from - b.from));
    let subagents_out = name_subagents(&subs, &spawns, &labels)?;

    Ok(Out::obj(vec![
        ("app", Out::of(app)),
        ("from", first),
        ("to", last),
        ("activeMs", Out::Num(active)),
        ("totals", totals),
        ("kinds", kinds),
        ("models", Out::Arr(models_out)),
        ("turns", Out::Arr(turns_out)),
        ("tools", Out::Arr(tools_out)),
        ("subagents", Out::Arr(subagents_out)),
        ("context", Out::Arr(thin(&context, CONTEXT_POINTS).into_iter().map(|p| Out::obj(vec![("t", Out::Num(p.t)), ("used", p.used.out()), ("window", p.window)])).collect())),
        ("compactions", Out::Arr(compactions.into_iter().map(Out::Num).collect())),
        ("pricing", Out::obj(vec![("checked", Out::Str(pricing::PRICES_CHECKED.into())), ("unpriced", Out::Arr(unpriced.items.iter().map(Val::out).collect()))])),
    ]))
}

// A subagent's name: the description of the Agent call made just before it
// started (each used once), or else the start of its task.
fn name_subagents(list: &[&SubSum], spawns: &[&Event], labels: &HashMap<Key, String>) -> Result<Vec<Out>, TypeError> {
    let mut used = vec![false; spawns.len()];
    let mut out = Vec::new();
    for a in list {
        let spawn = spawns.iter().enumerate().rfind(|(i, sp)| !used[*i] && sp.t <= a.from + 2000.0 && a.from - sp.t < 120e3);
        let label = match spawn {
            Some((i, sp)) => {
                used[i] = true;
                Out::Str(clipped(sp.text.as_ref(), 80)?)
            }
            None => labels.get(&a.id.key()).map_or(Out::Null, |l| Out::Str(l.clone())),
        };
        out.push(Out::obj(vec![
            ("id", a.id.out()),
            ("calls", Out::Num(a.calls)),
            ("tokens", a.tokens.out()),
            ("cost", Out::Num(a.cost)),
            ("model", a.model.out()),
            ("from", Out::Num(a.from)),
            ("to", Out::Num(a.to)),
            ("label", label),
        ]));
    }
    Ok(out)
}

// At most n points, keeping the first, the last and each stretch's peak.
fn thin(points: &[Point], n: usize) -> Vec<Point> {
    if points.len() <= n {
        return points.to_vec();
    }
    let size = points.len() as f64 / n as f64;
    let mut out: Vec<Point> = Vec::new();
    for i in 0..n {
        let from = ((i as f64 * size).floor() as usize).min(points.len());
        let to = (((i + 1) as f64 * size).floor() as usize).min(points.len());
        if from < to {
            let peak = points[from + 1..to].iter().fold(&points[from], |a, b| if greater(&b.used, &a.used) { b } else { a });
            out.push(peak.clone());
        }
    }
    let n = out.len();
    out[n - 1] = points[points.len() - 1].clone();
    out
}

// ---------- the files ----------

static UUID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").expect("a fixed pattern"));

// The first UUID in a text, either case, in lowercase.
fn uuid_in(text: &str) -> Option<String> {
    UUID.find(&text.to_ascii_lowercase()).map(|m| m.as_str().to_string())
}

// /^rollout-.*\.jsonl(\.zst)?$/.
fn is_rollout(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("rollout-") else { return false };
    let middle = rest.strip_suffix(".jsonl").or_else(|| rest.strip_suffix(".jsonl.zst"));
    middle.is_some_and(|m| !m.contains(['\n', '\r', '\u{2028}', '\u{2029}']))
}

fn base(p: &str) -> &str {
    let cut = if cfg!(windows) { p.rfind(['/', '\\']) } else { p.rfind('/') };
    cut.map_or(p, |i| &p[i + 1..])
}

fn dir_names(dir: &str) -> Option<Vec<(String, bool)>> {
    let ents = paths::read_dir_sorted(Path::new(dir)).ok()?;
    Some(ents.into_iter().map(|e| (e.file_name().to_string_lossy().into_owned(), e.file_type().is_ok_and(|t| t.is_dir()))).collect())
}

/// Where sessions' transcripts are. Codex rollouts are found by their
/// thread id, from a listing of the rollout folders made once and again
/// when an id isn't in it (at most every 10 seconds).
#[derive(Default)]
pub struct Transcripts {
    pub rollouts: Vec<(String, String)>,
    pub listed_at: f64,
}

impl Transcripts {
    /// A session's files (its own, then its subagents'), each with the
    /// subagent it's from, as of `now`.
    pub fn files(&mut self, session_id: &str, claude_root: Option<&str>, codex_roots: &[String], now: f64) -> Vec<(String, Option<String>)> {
        // sessionId.slice(0, i) and .slice(i + 1), i its first colon (or -1).
        let colon = session_id.find(':').map_or(-1, |i| jsstr::len(&session_id[..i]) as i64);
        let app = jsstr::slice(session_id, 0, Some(colon));
        let raw = jsstr::slice(session_id, colon + 1, None);
        if uuid_in(&raw).is_none() {
            return Vec::new();
        }
        if app == "claude" {
            let root = claude_root.unwrap_or("");
            for (name, is_dir) in dir_names(root).unwrap_or_default() {
                if !is_dir {
                    continue;
                }
                let main = paths::join_all(&[root, &name, &format!("{raw}.jsonl")]);
                if !Path::new(&main).exists() {
                    continue;
                }
                let sub_dir = paths::join_all(&[root, &name, &raw, "subagents"]);
                let mut out = vec![(main, None)];
                for (f, _) in dir_names(&sub_dir).unwrap_or_default() {
                    if let Some(stem) = f.strip_suffix(".jsonl") {
                        let agent = stem.strip_prefix("agent-").unwrap_or(stem).to_string();
                        out.push((paths::join(&sub_dir, &f), Some(agent)));
                    }
                }
                return out;
            }
            return Vec::new();
        }
        if app == "codex" {
            if !self.rollouts.iter().any(|(k, _)| *k == raw) && now - self.listed_at > 10e3 {
                self.list(codex_roots, now);
            }
            return self.rollouts.iter().find(|(k, _)| *k == raw).map(|(_, f)| vec![(f.clone(), None)]).unwrap_or_default();
        }
        Vec::new()
    }

    /// Lists the rollout folders again.
    pub fn list(&mut self, codex_roots: &[String], now: f64) {
        self.listed_at = now;
        for root in codex_roots {
            for file in paths::list_files(root, &mut |name| is_rollout(name), 4) {
                if let Some(id) = uuid_in(base(&file)) {
                    match self.rollouts.iter_mut().find(|(k, _)| *k == id) {
                        Some(r) => r.1 = file,
                        None => self.rollouts.push((id, file)),
                    }
                }
            }
        }
    }
}

/// The stats kept for the last sessions asked about, by what their files
/// were (size and time) when they were read: at most 40, the oldest let go
/// first.
#[derive(Default)]
pub struct Kept {
    kept: Vec<(String, String)>,
}

impl Kept {
    /// What a session's files are now, as one key.
    pub fn stamp(files: &[String]) -> String {
        files
            .iter()
            .map(|f| match std::fs::metadata(f) {
                Ok(m) => format!("{f}:{}:{}", m.len(), xfs::mtime_ms_of(&m).map_or_else(|_| "NaN".into(), json::number_string)),
                Err(_) => f.clone(),
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// Whether the stats kept for a session are of files as they are now.
    pub fn hit(&self, session_id: &str, key: &str) -> bool {
        self.kept.iter().any(|(id, k)| id == session_id && k == key)
    }

    /// Keeps a session's key; gives back the session let go to make room.
    pub fn keep(&mut self, session_id: &str, key: &str) -> Option<String> {
        match self.kept.iter_mut().find(|(id, _)| id == session_id) {
            Some(e) => e.1 = key.to_string(),
            None => self.kept.push((session_id.to_string(), key.to_string())),
        }
        (self.kept.len() > 40).then(|| self.kept.remove(0).0)
    }
}

/// A session's stats from its files, read: { id, ...summarize }.
pub fn session_stats(session_id: &str, files: Vec<File>) -> Result<Out, TypeError> {
    let app = session_id.split(':').next().unwrap_or_default();
    let mut ids = EventIds::default();
    let events = if app == "codex" { codex_events(files.into_iter().next().map(|f| f.lines).unwrap_or_default(), &mut ids)? } else { claude_events(files, &mut ids)? };
    let Out::Obj(mut fields) = summarize(&events, Some(&Value::String(app.to_string())))? else { unreachable!() };
    fields.insert(0, ("id".into(), Out::Str(session_id.to_string())));
    Ok(Out::Obj(fields))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn field<'a>(o: &'a Out, k: &str) -> &'a Out {
        let Out::Obj(fs) = o else { panic!("not an object") };
        &fs.iter().find(|(x, _)| x == k).unwrap_or_else(|| panic!("no {k}")).1
    }

    #[test]
    fn prompts_as_typed() {
        assert_eq!(prompt_text("  <command-name>/goal</command-name>\n<command-args>ship it</command-args>").as_deref(), Some("/goal ship it"));
        assert_eq!(prompt_text("<command-name>/</command-name>").as_deref(), Some("//"));
        assert_eq!(prompt_text("<command-name>x</command-name>").as_deref(), Some("/x"));
        assert_eq!(prompt_text("<local-command-stdout>ok</local-command-stdout>"), None);
        assert_eq!(prompt_text(" Add refunds "), Some("Add refunds".into()));
        assert_eq!(prompt_text("   "), None);
    }

    #[test]
    fn figures_add_as_js_adds_them() {
        assert_eq!(Prim::Num(0.0).add(&Prim::Str("10".into())).add(&Prim::Num(5.0)), Prim::Str("0105".into()));
        assert_eq!(Prim::Num(1.0).add(&Prim::Null).add(&Prim::Bool(true)), Prim::Num(2.0));
        assert!(Prim::Num(1.0).add(&Prim::Undef).num().is_nan());
        assert!(Prim::of(Some(&json!({ "toString": 1 }))).is_err());
        assert_eq!(Prim::of(Some(&json!([1, null, [2]]))).unwrap(), Prim::Str("1,,2".into()));
        assert!(at_most(&Prim::Str("12000".into()), &Prim::Num(12000.0)));
        assert!(!at_most(&Prim::Undef, &Prim::Num(0.0)), "NaN is never at most");
        assert!(greater(&Prim::Str("b".into()), &Prim::Str("a".into())));
    }

    fn at(min: f64) -> Value {
        let noon = xenonet_base::time::parse_iso("2026-10-01T12:00:00Z").unwrap();
        json!(xenonet_base::time::iso(noon + min * 60e3))
    }

    #[test]
    fn a_claude_session() {
        let usage = |i: u64, r: u64, w: u64, o: u64| json!({ "input_tokens": i, "cache_read_input_tokens": r, "cache_creation_input_tokens": w, "cache_creation": { "ephemeral_5m_input_tokens": w, "ephemeral_1h_input_tokens": 0 }, "output_tokens": o });
        let say = |min: f64, mid: &str, u: Value, content: Value| json!({ "type": "assistant", "timestamp": at(min), "message": { "id": mid, "model": "claude-opus-5-5", "usage": u, "content": content } });
        let main = vec![
            json!({ "type": "user", "timestamp": at(0.0), "message": { "role": "user", "content": "Add partial refunds" } }),
            say(1.0, "m1", usage(10, 1000, 500, 100), json!([{ "type": "text", "text": "Looking" }])),
            say(1.0, "m1", usage(10, 1000, 500, 100), json!([{ "type": "tool_use", "id": "t1", "name": "Bash", "input": { "command": "ls" } }])),
            json!({ "type": "user", "timestamp": at(2.0), "message": { "content": [{ "type": "tool_result", "tool_use_id": "t1", "is_error": true }] } }),
            say(3.0, "m2", usage(5, 2000, 0, 50), json!([{ "type": "tool_use", "id": "t2", "name": "Agent", "input": { "description": "Find the refund tests" } }])),
            json!({ "type": "user", "timestamp": at(9.0), "message": { "content": [{ "type": "tool_result", "tool_use_id": "t2" }] } }),
        ];
        let sub = vec![
            json!({ "type": "user", "timestamp": at(3.0), "isSidechain": true, "message": { "content": "Find every refund test" } }),
            json!({ "type": "assistant", "timestamp": at(4.0), "isSidechain": true, "message": { "id": "s1", "model": "claude-haiku-4-5", "usage": usage(5, 100, 0, 20), "content": [] } }),
        ];
        let s = session_stats("claude:x", vec![File { agent: None, lines: main }, File { agent: Some(json!("a1")), lines: sub }]).unwrap();
        assert_eq!(field(field(&s, "totals"), "calls"), &Out::Num(3.0));
        let Out::Arr(tools) = field(&s, "tools") else { panic!() };
        assert_eq!(field(&tools[0], "ms"), &Out::Num(60e3));
        assert_eq!(field(&tools[0], "errors"), &Out::Num(1.0));
        let Out::Arr(subs) = field(&s, "subagents") else { panic!() };
        assert_eq!(field(&subs[0], "label"), &Out::Str("Find the refund tests".into()));
        let Out::Arr(context) = field(&s, "context") else { panic!() };
        assert_eq!(context.len(), 2, "main-agent calls only");
    }

    #[test]
    fn a_codex_thread_counts_each_call_once() {
        let count = |min: f64, total: u64| json!({ "type": "event_msg", "timestamp": at(min), "payload": { "type": "token_count", "info": { "total_token_usage": { "total_tokens": total }, "last_token_usage": { "input_tokens": 11000, "cached_input_tokens": 8000, "output_tokens": 1000 } } } });
        let lines = vec![
            json!({ "type": "turn_context", "timestamp": at(0.0), "payload": { "model": "gpt-5.6-sol", "effort": "medium" } }),
            json!({ "type": "event_msg", "timestamp": at(0.0), "payload": { "type": "user_message", "message": "compress the logs" } }),
            count(2.0, 12000),
            count(2.0, 12000),
            json!({ "type": "event_msg", "timestamp": at(3.0), "payload": { "type": "task_complete" } }),
        ];
        let s = session_stats("codex:x", vec![File { agent: None, lines }]).unwrap();
        assert_eq!(field(field(&s, "totals"), "calls"), &Out::Num(1.0));
        assert_eq!(field(field(field(&s, "kinds"), "input"), "tokens"), &Out::Num(3000.0));
        let Out::Arr(turns) = field(&s, "turns") else { panic!() };
        assert_eq!(field(&turns[0], "prompt"), &Out::Str("compress the logs".into()));
    }

    #[test]
    fn thin_keeps_peaks_and_the_last() {
        let ps: Vec<Point> = (0..10).map(|i| Point { t: f64::from(i), used: Prim::Num(if i == 3 { 99.0 } else { f64::from(i) }), window: Out::Null }).collect();
        let out = thin(&ps, 4);
        assert_eq!(out.iter().map(|p| p.t).collect::<Vec<_>>(), [1.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn rollout_names() {
        assert!(is_rollout("rollout-2026-10-01T12-00-00-019a.jsonl.zst") && is_rollout("rollout-.jsonl") && !is_rollout("rollout-a\nb.jsonl") && !is_rollout("x-rollout-a.jsonl"));
        assert_eq!(uuid_in("rollout-x-019A0000-AAAA-bbbb-cccc-000000000001.jsonl").as_deref(), Some("019a0000-aaaa-bbbb-cccc-000000000001"));
    }

    #[test]
    fn kept_lets_the_oldest_go() {
        let mut k = Kept::default();
        for i in 0..40 {
            assert_eq!(k.keep(&i.to_string(), "k"), None);
        }
        assert_eq!(k.keep("0", "k2"), None, "kept again in its place");
        assert_eq!(k.keep("40", "k"), Some("0".into()));
        assert!(k.hit("40", "k") && !k.hit("40", "x"));
    }
}
