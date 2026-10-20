// Transcript lines as one stream of events (docs/design/vendor-layer.md,
// "One event stream"). Each app XenoNet tails has one mapper from its
// normalized lines (lines.js) to the events below, and the readers read
// those: the live tailer (live.js), a session's stats (stats.js), search
// (search.js) and the refusals (refusals/detect.js).
//
//   mapper.events(line, file) → [event]
//
// `file` is what a reader keeps for a file (mapper.file(...)): a
// subagent's id, and for Codex the prompt mode. Stats and search set the
// mode from the whole file before mapping (hasUserMessages); the live
// tailer has the mapper set it as lines come (`follow`). What's kept per
// session stays the reader's: the message ids it counted, live's turn
// marks and its last token total.
//
// Every event has `kind`, `t` (the line's time, NaN when it can't be
// read), `agentId` (a subagent's file, else null) and `origin` (the line's
// kind, as the sampling hooks name it):
//
//   refusal    signal: as detect.js builds it
//   prompt     text (text blocks joined with spaces), parts (the content as
//              written: its blocks, or its text), and for Claude Code
//              command, meta, sidechain, compactSummary and interruption;
//              for Codex first (a rollout's first completed user message)
//   said       parts: what the assistant wrote, as written; sidechain
//   call       messageId, usage (lines.js's shape); Claude Code's model and
//              effort; Codex's last (last_token_usage), total
//              (total_token_usage) and window, as written
//   tool       id, name, input, and for Codex call (the item's type)
//   result     id, and error (Claude Code) or call (Codex)
//   item       a Codex completed item: item, as written, and its type
//   turnStart  window
//   turnEnd    reason: end_turn, task_complete, interrupted or refusal;
//              details (Claude Code's refusal), duration (task_complete),
//              signal (Codex's refusal), meta and sidechain (Claude
//              Code's interruption)
//   settings   model, effort, tier (only when the line says one), provider
//   compact    meta, sidechain
//   context    usage and window, as a token_count says them
//   limits     rateLimits, as written
//   broken     at: the field that couldn't be read
//
// Where the readers agree, the mapper decides: a compacted summary and an
// interruption aren't prompts (they're flagged, and an interruption is a
// turnEnd), a Codex rollout's prompts are its completed user messages, or
// its user_message events in a rollout with none, and blank ones and ones
// starting with "<" aren't prompts (isCodexPrompt). Where each reader keeps
// its own way (docs/design/blocks.md), the event carries what its rule
// needs, as written, and the rule stays in the reader.
//
// The mapper never throws. A field it can't read ends the line's events
// with `broken`, saying which and what reading it threw, so a reader that
// would have read that field throws that, as it did when it read the line
// itself, and one that wouldn't ignores it. Two are read apart from the
// rest of the line, so their `broken` stands in their place and the line's
// events go on: the refusal (read first, by the refusals' readers) and a
// message's usage (read once per message id).
import { ClaudeLine, CodexLine, claudeLines, codexLines, isCodexPrompt } from "./lines.js";
import { claudeRefusal, codexRefusal } from "./refusals/detect.js";

// The sampling hooks' names for a line's kind (lines.js): Claude Code's by
// type, Codex's by type and payload type (and an item's type).
const kindPart = (v) => (typeof v === "string" ? v : typeof v);
const claudeKind = (raw) => kindPart(raw?.type);
function codexKind(raw) {
  const t = kindPart(raw?.type);
  if (t !== "event_msg" && t !== "response_item") return t;
  const p = kindPart(raw.payload?.type);
  return p === "item_completed" ? `${t}:${p}:${kindPart(raw.payload.item?.type)}` : `${t}:${p}`;
}

// A field that couldn't be read: where, and what reading it threw, which
// a reader that reads it throws again.
const broken = (base, at, error, more) => ({ kind: "broken", ...base, at, error, ...more });

// A line's refusal, first: read apart from the rest of the line.
function refusal(out, base, read, raw) {
  try {
    const signal = read(raw);
    if (signal) out.push({ kind: "refusal", ...base, signal });
    return signal;
  } catch (error) {
    out.push(broken(base, "refusal", error));
    return null;
  }
}

// ---------- Claude Code ----------

// A prompt's slash command: its name and arguments, or null.
function commandOf(text) {
  const t = text.trim();
  const m = /^<command-name>\/?([^<]+)<\/command-name>/.exec(t);
  return m ? { name: m[1], args: /<command-args>([\s\S]*?)<\/command-args>/.exec(t)?.[1] ?? "" } : null;
}

function claudeAssistant(o, base, out) {
  // Each of a message's lines repeats its usage; the readers count it once
  // per id, so it's read apart.
  let usage = null, bad = null;
  try {
    usage = o.usage;
  } catch (error) {
    bad = error;
  }
  out.push({ kind: "call", ...base, messageId: o.messageId, model: o.model, effort: o.effort, usage });
  if (bad) out.push(broken(base, "usage", bad));
  out.push({ kind: "said", ...base, parts: o.blocks, sidechain: o.sidechain });
  // Its blocks as the readers go through them: a string's characters are
  // blocks without a type, and anything else that isn't a list, or a block
  // that isn't there, can't be read.
  try {
    for (const b of o.blocks) if (b.type === "tool_use") out.push({ kind: "tool", ...base, id: b.id, name: b.name, input: b.input });
  } catch (error) {
    return void out.push(broken(base, "blocks", error));
  }
  const reason = o.message.stop_reason;
  if (reason === "end_turn") out.push({ kind: "turnEnd", ...base, reason });
  else if (reason === "refusal") out.push({ kind: "turnEnd", ...base, reason, details: o.message.stop_details });
}

function claudeUser(o, base, out) {
  const meta = o.meta, sidechain = o.sidechain, compactSummary = o.compactSummary;
  const content = o.content;
  let results;
  try {
    results = o.results ?? [];
  } catch (error) {
    // A block that isn't there can't be read; search reads a line only as
    // far as its first tool result.
    const afterResult = content.slice(0, content.indexOf(null)).some((x) => x?.type === "tool_result");
    return void out.push(broken(base, "content", error, { afterResult, meta }));
  }
  for (const r of results) out.push({ kind: "result", ...base, id: r.id, error: r.error });
  if (!results.length && compactSummary) out.push({ kind: "compact", ...base, meta, sidechain });
  let text;
  try {
    text = o.text;
  } catch (error) {
    return void out.push(broken(base, "text", error, { meta, sidechain, compactSummary, results: results.length > 0 }));
  }
  const interruption = text.trimStart().startsWith("[Request interrupted");
  if (!results.length) out.push({ kind: "prompt", ...base, text, parts: content, command: commandOf(text), meta, sidechain, compactSummary, interruption });
  if (interruption) out.push({ kind: "turnEnd", ...base, reason: "interrupted", meta, sidechain });
}

// Claude Code: a transcript's lines, and its subagents' files beside it, so
// a session's events are put in time order.
export const claude = {
  app: "claude",
  lines: claudeLines,
  Line: ClaudeLine,
  sorted: true,
  file: ({ agentId = null } = {}) => ({ agentId }),
  events(o, file = {}) {
    const out = [];
    let base;
    try {
      base = { t: o.ts, agentId: file.agentId ?? null, origin: claudeKind(o.raw) };
    } catch (error) {
      return [broken({ t: NaN, agentId: file.agentId ?? null, origin: claudeKind(o.raw) }, "line", error)];
    }
    refusal(out, base, claudeRefusal, o.raw);
    const type = o.type;
    if (type === "assistant") claudeAssistant(o, base, out);
    else if (type === "user") claudeUser(o, base, out);
    return out;
  },
};

// ---------- Codex ----------

// A call's tokens from a token_count's last_token_usage, as Claude Code's
// usage has them: cached input and cache writes are part of input_tokens,
// and each figure stays as written.
function codexUsage(u) {
  const input = u.input_tokens ?? Math.max(0, (u.total_tokens ?? 0) - (u.output_tokens ?? 0));
  const cached = u.cached_input_tokens ?? 0, write = u.cache_write_input_tokens ?? 0;
  return { input: Math.max(0, input - cached - write), cacheRead: cached, cacheWrite5m: write, cacheWrite1h: 0, output: u.output_tokens ?? 0 };
}

// What a completed user message says: its parts' text joined with spaces,
// as CodexLine.userMessage reads it from a line, here from the item (live
// reads it this way, and so throws this way).
export const userText = (it) => (it.content ?? []).map((c) => c?.text ?? "").join(" ");

const CALLS = new Set(["function_call", "custom_tool_call", "local_shell_call"]);
const OUTPUTS = new Set(["function_call_output", "custom_tool_call_output", "local_shell_call_output"]);

// A completed item's events: the item, and what a user or agent message
// says (`said`: the user message's text, as read from its line).
function itemEvents(it, base, file, out, said = () => userText(it)) {
  out.push({ kind: "item", ...base, item: it, type: it.type });
  if (it.type === "UserMessage") {
    // A rollout's prompts are its completed user messages, or its
    // user_message events while it has none (lines.js); live finds out as
    // the lines come, and the first one counts from then on.
    if (!file.follow && !file.items) return;
    let text;
    try {
      text = said();
    } catch (error) {
      return void out.push(broken(base, "text", error));
    }
    const first = !file.items;
    if (file.follow) file.items = true;
    if (isCodexPrompt(text)) out.push({ kind: "prompt", ...base, text, parts: it.content, first });
  } else if (it.type === "AgentMessage") out.push({ kind: "said", ...base, parts: it.content });
}

function codexEvent(o, p, base, signal, file, out) {
  switch (p.type) {
    case "task_started":
      return void out.push({ kind: "turnStart", ...base, window: p.model_context_window });
    case "task_complete":
      if (p.last_agent_message) out.push({ kind: "said", ...base, parts: p.last_agent_message });
      return void out.push({ kind: "turnEnd", ...base, reason: "task_complete", duration: p.duration_ms });
    case "token_count": {
      // Codex reports a call's tokens more than once: each reader counts it
      // when the total grows, by its own rule. Its usage is read apart, as
      // a reader reads it only for a call it counts.
      const info = p.info;
      if (info) {
        let usage = null, bad = null;
        try {
          usage = codexUsage(info.last_token_usage ?? {});
        } catch (error) {
          bad = error;
        }
        out.push({ kind: "call", ...base, messageId: null, usage, last: info.last_token_usage, total: info.total_token_usage, window: info.model_context_window });
        if (bad) out.push(broken(base, "usage", bad));
      }
      out.push({ kind: "context", ...base, used: info?.last_token_usage?.total_tokens, window: info?.model_context_window });
      if (p.rate_limits) out.push({ kind: "limits", ...base, rateLimits: p.rate_limits });
      return;
    }
    case "item_completed":
      return itemEvents(p.item ?? {}, base, file, out, () => o.userMessage);
    case "user_message": {
      const text = o.userEvent;
      if (!file.items && isCodexPrompt(text)) out.push({ kind: "prompt", ...base, text, parts: text, first: false });
      return;
    }
    // You stopped the turn (Esc, or Ctrl-C). A turn the next one replaced,
    // or a review ending, isn't you stopping it; older rollouts give no
    // reason.
    case "turn_aborted":
      if (!p.reason || p.reason === "interrupted") out.push({ kind: "turnEnd", ...base, reason: "interrupted" });
      return;
    // A policy refusal ends the turn.
    case "error":
      if (signal?.kind === "refusal") out.push({ kind: "turnEnd", ...base, reason: "refusal", signal });
      return;
    // All of the thread's settings: a tier not said is the default one.
    case "thread_settings_applied":
      return void out.push({ kind: "settings", ...base, tier: p.thread_settings?.service_tier ?? null });
    case "agent_message":
      return void out.push({ kind: "said", ...base, parts: p.message });
    default:
      return;
  }
}

// A response item's type as stats matches it, as text: a type that isn't a
// string is turned into one, which can throw.
function callType(t, base, out) {
  if (typeof t === "string") return t;
  try {
    return String(t);
  } catch (error) {
    out.push(broken(base, "type", error));
    return null;
  }
}

// Codex: a rollout per thread.
export const codex = {
  app: "codex",
  lines: codexLines,
  Line: CodexLine,
  sorted: false,
  // `items`: whether the rollout has a completed user message, as read from
  // the whole file; `follow`: found out as the lines come.
  file: ({ agentId = null, items = false, follow = false } = {}) => ({ agentId, items, follow }),
  events(o, file = codex.file()) {
    const out = [];
    let base;
    try {
      base = { t: o.ts, agentId: file.agentId ?? null, origin: codexKind(o.raw) };
    } catch (error) {
      return [broken({ t: NaN, agentId: file.agentId ?? null, origin: codexKind(o.raw) }, "line", error)];
    }
    const signal = refusal(out, base, codexRefusal, o.raw);
    const p = o.payload;
    const type = o.type;
    if (type === "event_msg") codexEvent(o, p, base, signal, file, out);
    else if (type === "turn_context") {
      const s = { kind: "settings", ...base, model: p.model, effort: p.effort };
      // Older rollouts say the tier with each turn; newer ones in the
      // thread's settings.
      if (p.service_tier !== undefined) s.tier = p.service_tier;
      out.push(s);
    } else if (type === "session_meta") out.push({ kind: "settings", ...base, provider: p.model_provider });
    else if (type === "response_item") {
      if (p.type === "message" && p.role === "assistant") out.push({ kind: "said", ...base, parts: p.content });
      else {
        const t = callType(p.type, base, out);
        if (CALLS.has(t)) out.push({ kind: "tool", ...base, id: p.call_id ?? p.id, name: p.name, call: p.type, input: p.input ?? p.arguments });
        else if (OUTPUTS.has(t)) out.push({ kind: "result", ...base, id: p.call_id ?? p.id, call: p.type });
      }
    }
    return out;
  },
  // A completed item's events, for a reader handed the item (live's
  // codexItem).
  item(it, base, file) {
    const out = [];
    itemEvents(it, { t: NaN, agentId: null, origin: `event_msg:item_completed:${kindPart(it?.type)}`, ...base }, file, out);
    return out;
  },
};

// ---------- which app's ----------

// The tailed apps' mappers. Until the app registry (core/apps.js) holds
// them, an app is looked up here, with the fallback each reader had; `use`
// adds one, for a test.
const MAPPERS = new Map([
  ["claude", claude],
  ["codex", codex],
]);

export const mapperOf = (app, fallback = "claude") => MAPPERS.get(app) ?? MAPPERS.get(fallback);

// Adds a mapper for an app; gives back what takes it away again.
export function useMapper(app, mapper) {
  MAPPERS.set(app, mapper);
  return () => MAPPERS.delete(app);
}

// The `broken` of a line's events at one of `fields`, if there is one.
export const brokenAt = (events, ...fields) => events.find((e) => e.kind === "broken" && fields.includes(e.at)) ?? null;

// What a reader that would have read a broken field does: throw what
// reading it threw.
export function fail(e) {
  throw e.error;
}
