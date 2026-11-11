//! Claude Code's transcripts for the live tailer: which files under
//! ~/.claude/projects are its sessions' (and its subagents'), how a line
//! is sampled for the drift check, and what each line means on the
//! telemetry: a reply's tokens, context and tool calls, your prompts and
//! tool results, a turn's end, a subagent's start and stop. Grok Build's
//! lines have the same shapes and are read through `claude_line` too.
//! Claude's plan limits are read here from Anthropic's answers and Claude
//! desktop's samples. The map's listing of its sessions, a transcript's
//! page and a rename are `listing` (over `sdk`, `desktop` and
//! `transcript`).

pub mod desktop;
pub mod listing;
pub mod sdk;
pub mod session;
pub mod transcript;

use serde_json::{Map, Value};
use xenonet_adapter::reader::{Call, Classified, Emit, LineFormat, Run, Tail, Tailed, ToolCall, TypeError, VendorCheck, Where};
use xenonet_sources::line_events::{self, broken_at, usage_of, End, Field, Kind, LineEvent, LineFile};
use xenonet_sources::lines::{claude_kind_of, opt_prop, prop, ClaudeLine};
use xenonet_sources::live::js::{ev, index0, is_str, js_max, nn, num_of, nv, obj, obj_opt, pages_extra, sort_by_num, sv};
use xenonet_sources::live::telemetry::{project_limit, refusals};
use xenonet_sources::live::{categorize, claude_edits, clip, clip_value, files_of, mcp_server_of, mcp_tool_of, string_of, tool_detail, wiki_pages_of};
use xenonet_text::json::number;
use xenonet_text::text::{to_number, trim, truthy};

pub const SOURCE: &str = "anthropic/claude-code/transcript-line";
// Claude plan usage is asked of Anthropic every 5 minutes; a reading this
// old says nothing about the windows now.
const LIMITS_FRESH_MS: f64 = 45.0 * 60e3;

/// The reader: Claude's plan limits as last settled, the samples both
/// sources gave, and why there's no reading when there's none.
#[derive(Default)]
pub struct Reader {
    limits: Value,
    samples: Vec<Value>,
    reason: Value,
    anthropic: Vec<Value>,
    desktop: Vec<Value>,
}

impl Reader {
    pub fn new() -> Reader {
        Reader { limits: Value::Null, reason: Value::Null, ..Reader::default() }
    }

    /// An old reading says nothing about the windows now, so it's reported
    /// as missing, with when it was taken and why there's no newer one.
    fn claude_limits(&self, now: f64) -> Value {
        let c = &self.limits;
        if truthy(Some(c)) && now - num_of(Some(c), "observedAt") < LIMITS_FRESH_MS {
            return c.clone();
        }
        if !truthy(Some(c)) && !truthy(Some(&self.reason)) {
            return Value::Null;
        }
        let observed = nn(opt_prop(Some(c), "observedAt")).cloned().unwrap_or(Value::Null);
        obj(vec![("fiveHour", Value::Null), ("sevenDay", Value::Null), ("observedAt", observed), ("stale", Value::Bool(true)), ("reason", self.reason.clone())])
    }
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

/// What a transcript is, by its path (with / for \): a session's own
/// (`<session>.jsonl`) or a subagent's (`<session>/subagents/agent-<id>.jsonl`).
pub fn classify(norm: &str) -> Option<Classified> {
    if let Some((session, agent)) = subagent_file(norm) {
        return Some(Classified { id: session, agent: Some(agent) });
    }
    let rest = norm.strip_suffix(".jsonl")?;
    let slash = rest.rfind('/')?;
    let id = &rest[slash + 1..];
    (id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).then(|| Classified { id: id.to_string(), agent: None })
}

impl Tailed for Reader {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn source(&self) -> &'static str {
        SOURCE
    }

    fn block(&self) -> &'static str {
        "claude-lines"
    }

    /// Claude Code writes its version on each line.
    fn version_of(&self, raw: &Value, current: Option<String>) -> Option<String> {
        raw.get("version").and_then(Value::as_str).map(str::to_string).or(current)
    }

    /// Claude Code's reader reads the transcripts of an app no reader is
    /// for.
    fn reads_untailed(&self) -> bool {
        true
    }

    fn line_format(&self) -> Option<LineFormat> {
        Some(LineFormat::Claude)
    }

    fn kind_of(&self, raw: &Value) -> String {
        claude_kind_of(raw.get("type"))
    }

    fn classify(&self, norm: &str, _root: Option<&str>) -> Option<Classified> {
        classify(norm)
    }

    fn line(&mut self, t: &mut dyn Tail, _vendor: &dyn VendorCheck, at: &Where, raw: Value, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
        t.observe_servers(&raw);
        let o = ClaudeLine { raw };
        let events = line_events::claude(&o, &LineFile::new(Some(at.agent_id.clone())));
        refusals(t, "claude", at, &o.raw, &events, live, out);
        claude_line(t, at, "claude", o.ts(), &events, live, now, out)
    }

    fn limits(&self, now: f64) -> Option<Value> {
        Some(self.claude_limits(now))
    }

    /// Straight-line projection of each window's use over the recent
    /// samples: only samples from the current window, when its reset is
    /// known.
    fn forecast(&self, now: f64) -> Vec<(String, Value)> {
        let c = self.claude_limits(now);
        if !truthy(Some(&c)) || truthy(c.get("stale")) {
            return Vec::new();
        }
        let resets = c.get("resets");
        let since = |resets_at: Option<&Value>, span: f64, lookback: f64| js_max(now - lookback, if truthy(resets_at) { to_number(resets_at) - span } else { 0.0 });
        let points = |key: &str, from: f64| -> Vec<(f64, f64)> {
            self.samples.iter().filter(|x| nn(opt_prop(Some(x), key)).is_some() && num_of(Some(x), "ts") >= from).map(|x| (num_of(Some(x), "ts"), num_of(Some(x), key))).collect()
        };
        [("claudeFiveHour", "fiveHour", 5.0 * 3600e3, 3.0 * 3600e3), ("claudeSevenDay", "sevenDay", 7.0 * 86400e3, 48.0 * 3600e3)]
            .into_iter()
            .map(|(name, key, span, lookback)| {
                let resets_at = opt_prop(resets, key);
                let pts = points(key, since(resets_at, span, lookback));
                (name.to_string(), project_limit(now, &pts, c.get(key), Some(nn(resets_at).cloned().unwrap_or(Value::Null)), span))
            })
            .collect()
    }

    /// Anthropic was asked, and answered `r` ({ usage } or { reason }): a
    /// reading joins the samples (8 days are kept), or the reason there's
    /// none is kept.
    fn usage_reading(&mut self, r: Option<&Value>, now: f64) -> Result<(), TypeError> {
        let usage = prop(r, "usage")?;
        if truthy(usage) {
            self.reason = Value::Null;
            self.anthropic.retain(|x| now - num_of(Some(x), "ts") < 8.0 * 86400e3);
            let u = |k: &str| opt_prop(usage, k).cloned();
            self.anthropic.push(obj_opt(vec![
                ("ts", u("observedAt")),
                ("fiveHour", u("fiveHour")),
                ("sevenDay", u("sevenDay")),
                ("resets", u("resets")),
                ("source", sv("anthropic")),
            ]));
        } else {
            self.reason = nn(prop(r, "reason")?).cloned().unwrap_or(Value::Null);
        }
        Ok(())
    }

    /// Claude desktop's samples, as its file has them now.
    fn usage_samples(&mut self, samples: &Value) {
        if let Value::Array(a) = samples {
            self.desktop.clone_from(a);
        }
    }

    /// Both sources' samples, in time order; the newer reading wins.
    fn settle_usage(&mut self) {
        let mut samples: Vec<Value> = self.anthropic.iter().chain(&self.desktop).cloned().collect();
        sort_by_num(&mut samples, |a, b| num_of(Some(a), "ts") - num_of(Some(b), "ts"));
        if let Some(last) = samples.last().filter(|l| truthy(Some(l))) {
            let l = |k: &str| opt_prop(Some(last), k).cloned();
            let resets = nn(opt_prop(Some(last), "resets")).cloned().unwrap_or_else(|| obj(vec![("fiveHour", Value::Null), ("sevenDay", Value::Null)]));
            self.limits = obj_opt(vec![("fiveHour", l("fiveHour")), ("sevenDay", l("sevenDay")), ("resets", Some(resets)), ("observedAt", l("ts")), ("source", l("source"))]);
        }
        self.samples = samples;
    }
}

/// A Claude Code line's events, or another app's with their shapes
/// (`app`'s), with when the line was written.
#[expect(clippy::too_many_arguments, reason = "a line's events and its time, handed down as each part works them out")]
pub fn claude_line(
    t: &mut dyn Tail,
    at: &Where,
    app: &str,
    ts: Result<f64, TypeError>,
    events: &[LineEvent<'_>],
    live: bool,
    now: f64,
    out: &mut Vec<Emit>,
) -> Result<(), TypeError> {
    let i = t.session(&at.session_id, app);
    // A line that doesn't say when it was written was written when it's
    // read.
    let written = ts?;
    let ts = if written != 0.0 && !written.is_nan() { written } else { now };
    let agent = at.agent_id.clone();
    let has_agent = truthy(Some(&agent));
    if has_agent {
        if t.subagent(i, Some(&agent)).is_none() {
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
            t.add_subagent(i, Some(&agent), m);
        }
        let a = t.subagent(i, Some(&agent)).expect("added above when missing");
        let last = js_max(to_number(a.get("lastAt")), ts);
        a.insert("lastAt".into(), number(last));
    }
    // The context's fill, where a line says it (Grok Build's usage
    // updates).
    let context = events.iter().find_map(|e| match &e.kind {
        Kind::Context { used, window } => Some((*used, *window)),
        _ => None,
    });
    if let (Some((used, window)), false) = (context, has_agent) {
        if let Some(u @ Value::Number(_)) = used {
            t.set_context_used(i, u.clone());
        }
        if let Some(w @ Value::Number(n)) = window {
            if n.as_f64().is_some_and(|n| n > 0.0) {
                t.set_context_window(i, w.clone());
            }
        }
    }
    // A chunk that carries on what its run began (Grok Build's).
    if let Some((of, whole, meta)) = events.iter().find_map(|e| match &e.kind {
        Kind::More { of, whole, meta, .. } => Some((*of, whole.as_str(), *meta)),
        _ => None,
    }) {
        if !has_agent {
            t.more(i, of, whole, meta, ts, live, now, out)?;
        }
        return Ok(());
    }
    // What the line is: a reply, or what you (or a tool) said.
    let kind = events.iter().find(|e| !matches!(e.kind, Kind::Refusal(_))).map(|e| &*e.origin);
    if kind == Some("assistant") {
        claude_assistant(t, i, &agent, events, ts, live, now, out)?;
    } else if kind == Some("user") {
        claude_user(t, i, &agent, events, ts, live, now, out)?;
    }
    // Announce a subagent once its first line has given it a label.
    if has_agent {
        let a = t.subagent(i, Some(&agent)).expect("a subagent is added when the first line that names it is read");
        if !truthy(a.get("announced")) {
            a.insert("announced".into(), Value::Bool(true));
            let label = a.get("label").cloned();
            t.activity(i, ev(vec![("kind", sv("subagent-start")), ("agentId", Some(agent)), ("label", label), ("ts", nv(ts))]), live, now, out);
        }
    }
    Ok(())
}

// A reply: its tokens and context fill, its tool calls and text, and
// the end of a turn (or of a subagent's work).
#[expect(clippy::too_many_arguments, reason = "a line's events and what claude_line worked out about it (session, subagent, time)")]
fn claude_assistant(t: &mut dyn Tail, i: usize, agent: &Value, events: &[LineEvent<'_>], ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
    let has_agent = truthy(Some(agent));
    let Some(c) = events.iter().find_map(|e| match &e.kind {
        Kind::Call(c) => Some(c),
        _ => None,
    }) else {
        return Ok(());
    };
    let model = c.model.cloned();
    if !has_agent {
        if let Some(m) = model.as_ref().filter(|m| truthy(Some(m))) {
            t.set_model(i, m.clone());
        }
    }
    // Each of a message's lines repeats its usage: counted once.
    let id = c.message_id.cloned();
    let seen = truthy(id.as_ref()) && t.was_seen(i, id.as_ref());
    let u = if truthy(id.as_ref()) && !seen { usage_of(events, c)? } else { None };
    if let Some(u) = u {
        t.mark_seen(i, id.as_ref());
        let tk = &u.tokens;
        let ctx = tk.input.number() + tk.cache_read.number() + to_number(Some(&u.cache_write));
        let output = tk.output.number();
        t.add_output_tokens(i, output);
        t.add_tokens(i, ctx + output, ts, live);
        let call = Call { tokens: u.tokens.map(|f| f.priced()) };
        let call_model = nn(model.as_ref()).cloned().or_else(|| nn(Some(&t.model(i))).cloned()).unwrap_or_else(|| "unknown".into());
        t.add_call(i, &call_model, &call, ts);
        if !has_agent {
            t.set_context_used(i, number(ctx + output));
            let one_m = string_of(model.as_ref()).to_ascii_lowercase().contains("[1m]");
            if ctx + output > 200000.0 || one_m {
                t.set_context_window(i, number(1_000_000.0));
            }
        }
        if live {
            out.push(Emit::tokens(t.session_id(i), "claude", ctx + output, ts));
        }
    }
    // Its blocks in order: what it said, and its tool calls.
    let blocks = events.iter().find_map(|e| match &e.kind {
        Kind::Said { parts, .. } => *parts,
        _ => None,
    });
    let list: &[Value] = match blocks {
        Some(Value::Array(a)) => a,
        // A string's characters have no type.
        Some(Value::String(_)) => &[],
        _ => return Err(TypeError("o.blocks is not iterable".into())),
    };
    if !has_agent {
        t.set_run(i, None);
    }
    for b in list {
        let kind = prop(Some(b), "type")?;
        if is_str(kind, "tool_use") {
            claude_tool_use(t, i, agent, b, ts, live, now, out)?;
        } else if is_str(kind, "text") {
            let text = prop(Some(b), "text")?;
            let said = match text {
                None | Some(Value::Null) => false,
                Some(Value::String(x)) => !trim(x).is_empty(),
                Some(_) => return Err(TypeError("b.text?.trim is not a function".into())),
            };
            if said && !has_agent {
                t.add_message(i, text, ts)?;
                t.set_run(i, Some(Run::Message));
                t.activity(i, ev(vec![("kind", sv("message")), ("detail", Some(Value::String(clip_value(text, 72)))), ("ts", nv(ts))]), live, now, out);
            }
        }
    }
    // A refusal ends the turn too: the provider said no, and nothing more
    // comes until you write again.
    let Some(end) = events.iter().find_map(|e| match &e.kind {
        Kind::TurnEnd(end @ (End::EndTurn | End::Refusal { .. })) => Some(end),
        _ => None,
    }) else {
        return Ok(());
    };
    if let (End::Refusal { details }, false) = (end, has_agent) {
        let why = nn(opt_prop(*details, "explanation")).or(nn(opt_prop(*details, "category"))).cloned().unwrap_or_else(|| "the provider refused".into());
        t.activity(i, ev(vec![("kind", sv("refusal")), ("detail", Some(Value::String(clip_value(Some(&why), 160)))), ("ts", nv(ts))]), live, now, out);
    }
    if has_agent {
        let a = t.subagent(i, Some(agent)).expect("a subagent is added when the first line that names it is read");
        a.insert("done".into(), Value::Bool(true));
        let label = a.get("label").cloned();
        t.activity(i, ev(vec![("kind", sv("subagent-stop")), ("agentId", Some(agent.clone())), ("label", label), ("ts", nv(ts))]), live, now, out);
    } else if t.turn_active(i) {
        // A turn that began with a line read as written when it was read
        // can seem to end before it began: how long it took isn't known
        // then.
        let duration = t.duration(i, ts);
        t.end_turn(i, ts, false);
        t.activity(i, ev(vec![("kind", sv("turn-end")), ("duration", Some(duration)), ("ts", nv(ts))]), live, now, out);
        t.ask_if_question(i, ts, live, now, out);
    }
    Ok(())
}

#[expect(clippy::too_many_arguments, reason = "a tool_use block and what claude_line worked out about its line (session, subagent, time)")]
fn claude_tool_use(t: &mut dyn Tail, i: usize, agent: &Value, b: &Value, ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
    let has_agent = truthy(Some(agent));
    let bid = opt_prop(Some(b), "id").cloned();
    let name = opt_prop(Some(b), "name").cloned();
    let input = opt_prop(Some(b), "input").cloned();
    let itag = t.inflight_add(i, bid.as_ref(), name.as_ref(), ts);
    if has_agent {
        let a = t.subagent(i, Some(agent)).expect("a subagent is added when the first line that names it is read");
        let tools = to_number(a.get("tools")) + 1.0;
        a.insert("tools".into(), number(tools));
    }
    let name_s = string_of(name.as_ref());
    let server = mcp_server_of(&name_s);
    let pages = wiki_pages_of(server.as_deref(), mcp_tool_of(&name_s), t.context_server(), input.as_ref());
    let full = match opt_prop(input.as_ref(), "command") {
        Some(c @ Value::String(_)) => c.clone(),
        _ => nn(input.as_ref()).cloned().unwrap_or_else(|| Value::Object(Map::new())),
    };
    let call = ToolCall {
        name: name.clone(),
        detail: tool_detail(input.as_ref(), t.home()),
        full: Some(full),
        files: files_of(input.as_ref()),
        ts,
        agent: Some(agent.clone()),
        server: None,
        extra: pages_extra(pages),
        ok: Value::Null,
    };
    let tool = t.note_tool(i, call, live, now, out);
    t.inflight_set_tool(i, &itag, tool);
    let edit_name = name.as_ref().and_then(Value::as_str).unwrap_or_default();
    let edits = claude_edits(edit_name, input.as_ref(), t.session_app(i))?;
    if !edits.is_empty() {
        t.add_edits(i, &edits, ts)?;
    }
    let description = opt_prop(input.as_ref(), "description");
    if categorize(&name_s) == "agent" && truthy(description) {
        let id = t.session_id(i).to_string();
        t.set_pending_label(id, Value::String(clip_value(description, 40)), ts);
    }
    let asks = name.as_ref().and_then(Value::as_str).filter(|n| *n == "AskUserQuestion" || *n == "ExitPlanMode");
    if !has_agent && asks.is_some() {
        let q = if asks == Some("ExitPlanMode") {
            Value::String("Plan ready for your approval".into())
        } else {
            let first = index0(opt_prop(input.as_ref(), "questions"));
            nn(opt_prop(first.as_ref(), "question")).or(nn(opt_prop(input.as_ref(), "question"))).cloned().unwrap_or_else(|| "Claude asked a question".into())
        };
        t.set_question(i, obj_opt(vec![("callId", bid), ("text", Some(Value::String(clip_value(Some(&q), 300)))), ("ts", nv(ts))]));
        t.activity(i, ev(vec![("kind", sv("needs-you")), ("detail", Some(Value::String(clip_value(Some(&q), 120)))), ("ts", nv(ts))]), live, now, out);
    }
    Ok(())
}

// Tool results, a prompt (in a subagent's file, its task), or your
// interruption.
#[expect(clippy::too_many_arguments, reason = "a line's events and what claude_line worked out about it (session, subagent, time)")]
fn claude_user(t: &mut dyn Tail, i: usize, agent: &Value, events: &[LineEvent<'_>], ts: f64, live: bool, now: f64, out: &mut Vec<Emit>) -> Result<(), TypeError> {
    let has_agent = truthy(Some(agent));
    if let Some(b) = broken_at(events, Field::Content) {
        return Err(b.error());
    }
    let results: Vec<(Option<&Value>, bool)> = events
        .iter()
        .filter_map(|e| match &e.kind {
            Kind::Result { id, error, .. } => Some((*id, error.unwrap_or(false))),
            _ => None,
        })
        .collect();
    if !results.is_empty() {
        for (id, error) in &results {
            let tool = t.inflight_remove(i, *id);
            if !has_agent {
                t.note_tool_result(i, !error, ts, tool.as_ref());
            }
            if *error {
                t.activity(i, ev(vec![("kind", sv("tool-error")), ("ts", nv(ts)), ("agentId", Some(agent.clone()))]), live, now, out);
            }
        }
        t.touch(i, ts);
    }
    // The line's flags, from what it holds besides its results: its
    // prompt, your interruption, or the text that couldn't be read.
    let prompt = events.iter().find_map(|e| match &e.kind {
        Kind::Prompt(p) => Some(p),
        _ => None,
    });
    let interrupted = events.iter().find_map(|e| match &e.kind {
        Kind::TurnEnd(End::Interrupted { gate }) => Some(*gate),
        _ => None,
    });
    let unread = broken_at(events, Field::Text);
    if prompt.is_some() && !has_agent {
        t.set_run(i, None);
    }
    let gate = prompt.and_then(|p| p.claude.as_ref().map(|c| (c.meta, c.sidechain))).or(interrupted.flatten()).or(unread.map(|b| (b.meta, b.sidechain)));
    let Some((meta, sidechain)) = gate else { return Ok(()) };
    if truthy(meta) || truthy(sidechain) != has_agent {
        return Ok(());
    }
    if let Some(b) = unread {
        return Err(b.error());
    }
    // A compacted summary and an interruption aren't prompts; an
    // interruption ends the turn you stopped.
    if interrupted.is_some() {
        if !has_agent {
            t.interrupted(i, ts, live, now, out);
        }
        return Ok(());
    }
    let Some(p) = prompt else { return Ok(()) };
    if !results.is_empty() || p.claude.as_ref().is_some_and(|c| truthy(c.compact_summary)) {
        return Ok(());
    }
    let text = &p.text;
    if trim(text).is_empty() || text.starts_with('<') {
        return Ok(());
    }
    if has_agent {
        // A subagent's first prompt is its task; the Agent tool's
        // description is a better label.
        let id = t.session_id(i).to_string();
        let pending = t.pending_label().cloned();
        let a = t.subagent(i, Some(agent)).expect("a subagent is added when the first line that names it is read");
        if is_str(a.get("label"), "subagent") {
            let label = match pending {
                Some((session, label, at)) if session == id && (ts - at).abs() < 60000.0 => label,
                _ => Value::String(clip(text, 40)),
            };
            a.insert("label".into(), label);
        }
        return Ok(());
    }
    t.prompted(i, text, ts, live, now, out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: f64 = 1_791_000_000_000.0;

    #[test]
    fn files_say_what_they_are() {
        let r = Reader::new();
        let id = "11111111-2222-3333-4444-555555555555";
        assert_eq!(r.classify(&format!("/h/.claude/projects/p/{id}.jsonl"), None), Some(Classified { id: id.into(), agent: None }));
        assert_eq!(r.classify(&format!("/h/.claude/projects/p/{id}/subagents/agent-abc.jsonl"), None), Some(Classified { id: id.into(), agent: Some("abc".into()) }));
        assert_eq!(r.classify("/h/notes.jsonl", None), None);
    }

    #[test]
    fn limits_go_stale() {
        let mut r = Reader::new();
        let reading =
            |at: f64, five: u64, seven: u64| json!({ "usage": { "observedAt": at, "fiveHour": five, "sevenDay": seven, "resets": { "fiveHour": null, "sevenDay": null } } });
        r.usage_reading(Some(&reading(NOW - 40.0 * 60e3, 4, 18)), NOW).unwrap();
        r.usage_reading(Some(&reading(NOW - 10.0 * 60e3, 9, 19)), NOW).unwrap();
        r.settle_usage();
        assert_eq!(r.limits(NOW).unwrap()["fiveHour"], 9);
        let forecast = r.forecast(NOW);
        assert_eq!((forecast[0].0.as_str(), forecast[0].1["pct"].clone()), ("claudeFiveHour", json!(9)));
        r.usage_reading(Some(&reading(NOW - 2.0 * 86400e3, 0, 0)), NOW + 3.0 * 86400e3).unwrap();
        r.settle_usage();
        assert_eq!(r.limits(NOW + 3.0 * 86400e3).unwrap()["stale"], true);
        assert_eq!(r.forecast(NOW + 3.0 * 86400e3), []);
        assert!(r.usage_reading(None, NOW).is_err());
        r.usage_reading(Some(&json!({ "reason": "no sign-in" })), NOW).unwrap();
        assert_eq!(r.limits(NOW + 3.0 * 86400e3).unwrap()["reason"], "no sign-in");
    }
}
