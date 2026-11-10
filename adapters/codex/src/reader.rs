//! Codex's rollouts for the live tailer: which files under ~/.codex are its
//! sessions', how a line is sampled for the drift check, and what each
//! line means on the telemetry: a turn's start and end, a call's tokens,
//! the context, its items (what you said, what it said, a command, an MCP
//! call, a patch, a search, a subagent), and its rate limits, which are
//! the snapshot's limits.codex. The map's listing of its threads, a
//! thread's items and a rename go through its app-server (`app_server`,
//! over `threads` and `rollouts`); Codex desktop's projects are
//! `state_db`.

pub mod app_server;
pub mod rollouts;
pub mod state_db;
pub mod threads;

use serde_json::{Map, Value};
use std::collections::HashMap;
use xenonet_adapter::reader::{Call, Classified, Emit, Tail, Tailed, ToolCall, TypeError, VendorCheck, Where};
use xenonet_model::usage::pricing;
use xenonet_model::usage::{Speed, TokenUsage};
use xenonet_os::paths;
use xenonet_sources::line_events::{self, broken_at, user_text, End, Field, Kind, LineEvent, LineFile};
use xenonet_sources::lines::{codex_kind_of, codex_version_of, opt_prop, prop, CodexLine};
use xenonet_sources::live::js::{ev, index0, is_str, js_max, map_opt, nn, num_of, nv, obj, object_keys, or0, pages_extra, strict_eq, sv, tpl};
use xenonet_sources::live::telemetry::{project_limit, refusals};
use xenonet_sources::live::{clip, clip_value, codex_edits, command_text, patch_files, string_of, wiki_pages_of};
use xenonet_text::json::number;
use xenonet_text::text::{display, join, to_number, trim, truthy};

pub const SOURCE: &str = "openai/codex/rollout-line";

/// What the reader keeps of a rollout as its lines come: whether its
/// prompts are its completed user messages yet, and the prompt its last
/// user_message event made (its sequence number, and the turn it came
/// in), which the first completed user message may repeat.
#[derive(Default)]
struct Rollout {
    items: bool,
    event_prompt: Option<(u64, f64)>,
}

/// The reader: Codex's rate limits in XenoNet's shape, their samples, and
/// what it keeps of each rollout.
#[derive(Default)]
pub struct Reader {
    limits: Value,
    samples: Vec<Value>,
    rollouts: HashMap<String, Rollout>,
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

impl Reader {
    pub fn new() -> Reader {
        Reader { limits: Value::Null, ..Reader::default() }
    }

    fn rollout(&mut self, id: &str) -> &mut Rollout {
        self.rollouts.entry(id.to_string()).or_default()
    }

    // What the mapper is told of a rollout: whether its prompts are its
    // completed user messages yet, which the tailer finds out as the
    // lines come.
    fn file(&self, id: &str) -> LineFile {
        LineFile { items: self.rollouts.get(id).is_some_and(|r| r.items), follow: true, ..LineFile::default() }
    }

    #[expect(clippy::too_many_arguments, reason = "a line's events and its time, handed down as each part works them out")]
    fn codex_events(&mut self, t: &mut dyn Tail, i: usize, events: &[LineEvent<'_>], ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        for e in events {
            match &e.kind {
                Kind::TurnStart { window } => {
                    t.reset_stall(i);
                    t.add_turn_mark(i);
                    t.begin_turn(i, ts);
                    if let Some(w) = window.filter(|w| truthy(Some(w))) {
                        t.set_context_window(i, w.clone());
                    }
                    t.activity(i, ev(vec![("kind", sv("turn-start")), ("ts", nv(ts))]), live, now, out);
                }
                // What it said last, with the turn's end (a completed
                // message is read with its item).
                Kind::Said { parts, .. } if &*e.origin == "event_msg:task_complete" => t.add_message(i, *parts, ts)?,
                Kind::TurnEnd(End::TaskComplete { duration }) => {
                    t.end_turn(i, ts, false);
                    let duration = nn(*duration).cloned().unwrap_or(Value::Null);
                    t.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("ts", nv(ts))]), live, now, out);
                    t.ask_if_question(i, ts, live, now, out);
                }
                // You stopped the turn (Esc, or Ctrl-C): Codex writes no
                // task_complete for it.
                Kind::TurnEnd(End::Interrupted { .. }) => t.interrupted(i, ts, live, now, out),
                // A policy refusal: the turn ends with it.
                Kind::TurnEnd(End::Refused(sig)) => {
                    let g = |k: &str| nn(sig.get(k));
                    let why = g("explanation").or(g("text")).or(sig.get("category")).cloned();
                    t.activity(i, ev(vec![("kind", sv("refusal")), ("detail", Some(Value::String(clip_value(why.as_ref(), 160)))), ("ts", nv(ts))]), live, now, out);
                    if t.turn_active(i) {
                        t.end_turn(i, ts, false);
                    }
                }
                Kind::Call(c) => {
                    if let Some(total) = c.total {
                        Self::codex_tokens(t, i, total, c.last, ts, live, out);
                    }
                }
                // The context from the last call.
                Kind::Context { used, window } => {
                    if let Some(v) = nn(*used) {
                        t.set_context_used(i, v.clone());
                    }
                    if let Some(w) = window.filter(|w| truthy(Some(w))) {
                        t.set_context_window(i, w.clone());
                    }
                    t.touch(i, ts);
                }
                Kind::Limits(rl) => self.note_limits(rl, ts),
                Kind::Item(it) => self.codex_item_of(t, i, it, events, ts, live, now, out)?,
                // A completed user message is read with its item.
                Kind::Prompt(p) if &*e.origin == "event_msg:user_message" => self.user_event(t, i, &p.text, ts, live, now, out),
                Kind::Settings { model, tier, provider, .. } => {
                    if &*e.origin == "turn_context" {
                        if let Some(m) = model.filter(|m| truthy(Some(m))) {
                            t.set_model(i, m.clone());
                        }
                    }
                    // What prices its calls: the provider, and the tier, said
                    // with a turn in older rollouts and in the thread's
                    // settings in newer ones, where a tier not said is the
                    // default.
                    if &*e.origin == "session_meta" {
                        if let Some(v) = nn(*provider) {
                            t.set_provider(i, v.clone());
                        }
                    }
                    if let Some(tier) = tier {
                        t.set_priority(i, pricing::priority_tier(&string_of(*tier)));
                    }
                }
                // A call (a response item, whose type live reads as written)
                // starts a tool in flight.
                Kind::Tool { id, name, input, call } if call.is_some_and(Value::is_string) => {
                    t.inflight_add(i, *id, *name, ts);
                    t.touch(i, ts);
                    if is_str(*name, "apply_patch") {
                        let files = patch_files(*input);
                        t.note_tool(i, ToolCall { name: sv("apply_patch"), files, ts, ..ToolCall::default() }, live, now, out);
                    }
                }
                // Its output ends a call, a shell call's too, matched as
                // stats match them: by call_id, else id.
                Kind::Result { id, call, .. } if call.is_some_and(Value::is_string) => {
                    t.inflight_remove(i, *id);
                    t.touch(i, ts);
                }
                Kind::Broken(b) if b.at == Field::Line => return Err(b.error()),
                _ => {}
            }
        }
        Ok(())
    }

    // A call's tokens: the growth of the total is what's new, and the
    // call's own tokens, as stats read them, from the last one's. The
    // output is output_tokens alone: OpenAI counts reasoning inside it, as
    // pricing prices it.
    fn codex_tokens(t: &mut dyn Tail, i: usize, total_usage: Option<&Value>, last: Option<&Value>, ts: f64, live: bool, out: &mut Vec<Emit>) {
        if truthy(total_usage) {
            let total = or0(opt_prop(total_usage, "total_tokens"));
            let delta = total - t.tokens_processed(i);
            if delta > 0.0 {
                t.add_tokens(i, delta, ts, live);
                // The call's own tokens, as stats read them: cached input
                // and cache writes are part of input_tokens.
                let empty = Value::Object(Map::new());
                let last = Some(nn(last).unwrap_or(&empty));
                let input = match nn(opt_prop(last, "input_tokens")) {
                    Some(v) => to_number(Some(v)),
                    None => js_max(0.0, or0(opt_prop(last, "total_tokens")) - or0(opt_prop(last, "output_tokens"))),
                };
                let cached = or0(opt_prop(last, "cached_input_tokens"));
                let write = or0(opt_prop(last, "cache_write_input_tokens"));
                let session_model = t.model(i);
                let model = nn(Some(&session_model)).cloned().unwrap_or_else(|| "unknown".into());
                let provider = t.provider(i);
                let call_model =
                    if truthy(Some(&provider)) && !is_str(Some(&provider), "openai") { Value::String(format!("{}/{}", display(&provider), display(&model))) } else { model };
                let call = Call {
                    tokens: TokenUsage {
                        input: Some(js_max(0.0, input - cached - write)),
                        cache_read: Some(cached),
                        cache_write_5m: Some(write),
                        cache_write_1h: Some(0.0),
                        output: Some(or0(opt_prop(last, "output_tokens"))),
                        speed: Speed { fast: false, priority: t.priority(i) },
                    },
                };
                t.add_call(i, &call_model, &call, ts);
                if live {
                    out.push(Emit::tokens(t.session_id(i), "codex", delta, ts));
                }
            }
            t.set_output_tokens(i, or0(opt_prop(total_usage, "output_tokens")));
        }
    }

    // A user_message event: a prompt while the rollout has no completed
    // user message (`codex exec` writes none, nor did Codex before 0.130),
    // passed by once it has (the mapper says which).
    #[expect(clippy::too_many_arguments, reason = "an event's text and its time, handed down as each part works them out")]
    fn user_event(&mut self, t: &mut dyn Tail, i: usize, text: &str, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) {
        t.add_prompt(i, text, ts);
        let seq = t.last_prompt_seq(i).expect("add_prompt just added one");
        let marks = t.turn_marks(i);
        let id = t.session_id(i).to_string();
        self.rollout(&id).event_prompt = Some((seq, marks));
        t.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(text, 90)))), ("ts", nv(ts))]), live, now, out);
    }

    /// Codex's rate limits in XenoNet's own shape (the snapshot's
    /// limits.codex): each window's use in percent, its length and when it
    /// resets, in ms.
    fn note_limits(&mut self, rl: &Value, ts: f64) {
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
        self.limits = obj(vec![("primary", primary), ("secondary", secondary), ("observedAt", number(ts))]);
        if let Some(last) = self.samples.last().filter(|l| truthy(Some(l))) {
            let same = strict_eq(opt_prop(Some(last), "primary"), sample.get("primary")) && strict_eq(opt_prop(Some(last), "secondary"), sample.get("secondary"));
            if same && ts - num_of(Some(last), "ts") < 10.0 * 60e3 {
                return;
            }
        }
        self.samples.push(sample);
        if self.samples.len() > 2000 {
            self.samples.remove(0);
        }
    }

    #[expect(clippy::too_many_arguments, reason = "an item, its line's events and what codex_line worked out about it (session, time)")]
    fn codex_item_of(&mut self, t: &mut dyn Tail, i: usize, it: &Value, events: &[LineEvent<'_>], ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let item = Some(it);
        let get = |k: &str| opt_prop(item, k);
        match get("type").and_then(Value::as_str) {
            Some("UserMessage") => {
                // What it says, as live reads it: one that can't be read
                // fails here.
                if let Some(b) = broken_at(events, Field::Text) {
                    user_text(it)?;
                    return Err(b.error());
                }
                // The rollout's first: from here on its prompts are these.
                // Codex may have written it as a user_message event just
                // before, in the same turn, which was taken for this prompt.
                let id = t.session_id(i).to_string();
                let r = self.rollout(&id);
                let event = if r.items { None } else { r.event_prompt };
                r.items = true;
                r.event_prompt = None;
                if let Some((seq, marks)) = event {
                    if t.last_prompt_seq(i) == Some(seq) && marks == t.turn_marks(i) {
                        return Ok(());
                    }
                }
                // A blank one (an image alone), or one that starts with "<",
                // isn't a prompt, as in stats and search: the mapper makes no
                // prompt of it.
                let Some(text) = events.iter().find_map(|e| match &e.kind {
                    Kind::Prompt(p) => Some(&p.text),
                    _ => None,
                }) else {
                    return Ok(());
                };
                t.add_prompt(i, text, ts);
                t.activity(i, ev(vec![("kind", sv("prompt")), ("detail", Some(Value::String(clip(text, 90)))), ("ts", nv(ts))]), live, now, out);
            }
            Some("AgentMessage") => {
                let content = nn(get("content"));
                let list: &[Value] = match content {
                    None => &[],
                    Some(Value::Array(a)) => a,
                    Some(_) => return Err(TypeError("(it.content ?? []).map is not a function".into())),
                };
                let mut parts = Vec::new();
                for c in list {
                    parts.push(prop(Some(c), "text")?);
                }
                let text = join(parts.into_iter(), " ");
                if !trim(&text).is_empty() {
                    t.add_message(i, Some(&Value::String(text.clone())), ts)?;
                    t.activity(i, ev(vec![("kind", sv("message")), ("detail", Some(Value::String(clip(&text, 72)))), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("CommandExecution") => {
                let code = get("exit_code");
                let ok = nn(code).is_none() || to_number(code) == 0.0;
                let done = ok && !is_str(get("status"), "failed");
                t.note_tool_result(i, done, ts, None);
                let cmd = command_text(get("command"));
                let detail = clip(&paths::tilde(&cmd, t.home().unwrap_or("")), 72);
                t.note_tool(i, ToolCall { name: sv("shell"), detail, full: Some(Value::String(cmd)), ok: Value::Bool(done), ts, ..ToolCall::default() }, live, now, out);
                if !ok {
                    t.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("McpToolCall") => {
                let ok = is_str(get("status"), "completed") && !truthy(get("error"));
                t.note_tool_result(i, ok, ts, None);
                let (server, tool) = (get("server"), get("tool"));
                let pages = wiki_pages_of(server.and_then(Value::as_str), tool.and_then(Value::as_str), t.context_server(), get("arguments"));
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
                t.note_tool(i, call, live, now, out);
                if !ok {
                    t.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts))]), live, now, out);
                }
            }
            Some("FileChange") => {
                let changes = get("changes");
                let files = object_keys(nn(changes));
                t.add_edits(i, &codex_edits(changes), ts)?;
                let done = !is_str(get("status"), "failed");
                t.note_tool_result(i, done, ts, None);
                let detail = files.iter().map(|f| paths::base_name(f)).collect::<Vec<_>>().join(", ");
                t.note_tool(i, ToolCall { name: sv("apply_patch"), detail, files, ok: Value::Bool(done), ts, ..ToolCall::default() }, live, now, out);
            }
            // An empty query is no query: the action's says what was
            // searched, as a thread shown from its rollout reads it. An
            // extension's web search is a web search; its other kinds (an
            // image made, a pause) aren't read.
            Some(kind @ ("WebSearch" | "Extension")) if kind == "WebSearch" || is_str(get("kind"), "web.search") => {
                let q = [get("query"), opt_prop(get("action"), "query")].into_iter().find(|v| truthy(*v)).flatten();
                t.note_tool(i, ToolCall { name: sv("web_search"), detail: clip_value(q, 72), ts, ..ToolCall::default() }, live, now, out);
            }
            Some("ImageView") => {
                let detail = paths::base_name(&string_of(get("path")));
                t.note_tool(i, ToolCall { name: sv("view_image"), detail, ts, ..ToolCall::default() }, live, now, out);
            }
            Some(kind @ ("CollabAgentToolCall" | "SubAgentActivity")) => {
                // Rollouts name these fields in snake_case (the app-server's
                // API says them in camelCase, which is read too).
                let id = [get("agent_thread_id").cloned(), get("agentThreadId").cloned(), index0(get("receiver_thread_ids")), index0(get("receiverThreadIds")), get("id").cloned()]
                    .into_iter()
                    .find(|v| v.as_ref().is_some_and(|x| !x.is_null()))
                    .flatten();
                if t.subagent(i, id.as_ref()).is_none() {
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
                    t.add_subagent(i, id.as_ref(), m);
                    t.activity(i, ev(vec![("kind", sv("subagent-start")), ("agentId", id.clone()), ("label", Some(label)), ("ts", nv(ts))]), live, now, out);
                }
                let a = t.subagent(i, id.as_ref()).expect("added above when missing");
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
                        t.activity(i, ev(vec![("kind", sv("subagent-stop")), ("agentId", id), ("label", label), ("ts", nv(ts))]), live, now, out);
                    }
                } else if is_str(what, "started") || is_str(what, "interacted") {
                    a.insert("done".into(), Value::Bool(false));
                }
            }
            _ => {}
        }
        Ok(())
    }
}

impl Tailed for Reader {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn source(&self) -> &'static str {
        SOURCE
    }

    fn block(&self) -> &'static str {
        "codex-lines"
    }

    fn version_of(&self, raw: &Value, current: Option<String>) -> Option<String> {
        codex_version_of(raw.get("type"), raw.get("payload").and_then(|p| p.get("cli_version")), current)
    }

    fn kind_of(&self, raw: &Value) -> String {
        let payload = raw.get("payload");
        codex_kind_of(raw.get("type"), payload.and_then(|p| p.get("type")), payload.and_then(|p| p.get("item")).and_then(|i| i.get("type")))
    }

    /// A rollout, under the sessions folder or anywhere under ~/.codex.
    fn classify(&self, norm: &str, root: Option<&str>) -> Option<Classified> {
        if root.is_none() && !norm.contains("/.codex/") {
            return None;
        }
        let id = uuid_before(norm)?;
        norm.contains("rollout-").then_some(Classified { id, agent: None })
    }

    fn line(&mut self, t: &mut dyn Tail, _vendor: &dyn VendorCheck, at: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        let o = CodexLine { raw };
        let events = line_events::codex(&o, &self.file(&at.session_id));
        refusals(t, "codex", at, &o.raw, &events, live, out);
        let i = t.session(&at.session_id, "codex");
        let written = o.ts()?;
        let ts = if written != 0.0 && !written.is_nan() { written } else { now };
        self.codex_events(t, i, events.as_slice(), ts, live, now, out)
    }

    fn limits(&self, _now: f64) -> Option<Value> {
        Some(self.limits.clone())
    }

    /// Straight-line projection of the primary window's use over the
    /// samples of the current window.
    fn forecast(&self, now: f64) -> Vec<(String, Value)> {
        let primary = opt_prop(Some(&self.limits), "primary");
        if !truthy(primary) {
            return Vec::new();
        }
        let (pct, minutes, resets_at) = (opt_prop(primary, "pct"), opt_prop(primary, "windowMinutes"), opt_prop(primary, "resetsAt"));
        let points: Vec<(f64, f64)> = self
            .samples
            .iter()
            .filter(|x| nn(opt_prop(Some(x), "primary")).is_some() && strict_eq(opt_prop(Some(x), "resets"), resets_at) && now - num_of(Some(x), "ts") < 24.0 * 3600e3)
            .map(|x| (num_of(Some(x), "ts"), num_of(Some(x), "primary")))
            .collect();
        let horizon = nn(minutes).map_or(300.0, |m| to_number(Some(m))) * 60e3;
        let mut m = match project_limit(now, &points, pct, resets_at.cloned(), horizon) {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        if let Some(w) = minutes {
            m.insert("windowMinutes".into(), w.clone());
        }
        vec![("codex".to_string(), Value::Object(m))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_say_what_they_are() {
        let r = Reader::new();
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(r.classify(&format!("/x/.codex/sessions/2026/rollout-2026-T-{id}.jsonl"), Some("/x/.codex/sessions")), Some(Classified { id: id.into(), agent: None }));
        assert_eq!(r.classify(&format!("/x/.codex/sessions/2026/other-{id}.jsonl"), Some("/x/.codex/sessions")), None);
        assert_eq!(r.classify(&format!("/h/.codex/archived/rollout-{id}.jsonl"), None), Some(Classified { id: id.into(), agent: None }));
        assert_eq!(r.classify(&format!("/h/elsewhere/rollout-{id}.jsonl"), None), None);
    }
}
