// A Grok Build session's tokens and cost, from the usage.json Grok keeps
// beside its updates (~/.grok/sessions/<encoded cwd>/<session id>/), as
// the live tailer (live.js) and a session's stats (stats.js) count them
// (docs/design/grok-build.md, Tokens and cost). Apart from grok.js, which
// lists the sessions, since this loads no settings. The Rust has a twin
// (crates/xenonet-sources/src/grok.rs's usage_turns).
import fs from "node:fs";
import path from "node:path";
import { reader } from "./vendor.js";

// A time as summary.json writes it (RFC 3339), or in Unix seconds or
// milliseconds; null when it isn't one.
export function grokTime(t) {
  const ms = typeof t === "string" ? Date.parse(t) : typeof t === "number" && t > 0 ? (t < 1e12 ? t * 1000 : t) : NaN;
  return Number.isFinite(ms) ? ms : null;
}

const readJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return undefined;
  }
};

const usages = reader("spacexai/grok-build/usage");
const plain = (v) => (v && typeof v === "object" && !Array.isArray(v) ? v : {});
const count = (v) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? v : 0);

// What Grok Build's ledger says a stretch of work used, as XenoNet counts a
// call's tokens (stats.js): its input apart from what was read from the
// cache or written to it, since Grok's inputTokens is the whole prompt (its
// source sums the API's prompt_tokens, which hold cache reads, and adds
// cache reads and writes into it for a model on Anthropic's API), and its
// output, reasoning included, as OpenAI's Responses API counts it. A figure
// that isn't a number counts as none.
function usageTokens(u) {
  const cacheRead = count(u.cachedReadTokens), cacheWrite = count(u.cacheCreationTokens);
  return { input: Math.max(0, count(u.inputTokens) - cacheRead - cacheWrite), cacheRead, cacheWrite5m: cacheWrite, cacheWrite1h: 0, output: count(u.outputTokens) };
}

// What it cost, in dollars, as Grok recorded it; null when that isn't
// known: no call said (with a SuperGrok or X Premium sign-in, Grok's guide
// says, it's often left out), or only some did (costIsPartial), so the
// figure is less than it cost. Never 0 for unknown.
function usageCost(u) {
  return typeof u.costUsdTicks === "number" && Number.isFinite(u.costUsdTicks) && u.costIsPartial !== true ? u.costUsdTicks / 1e10 : null;
}

// Whether a session is a subagent's, whose turns its parent's ledger
// counts too: Grok adds a subagent's usage into the turn of its parent's
// that ran it.
export function grokSubagent(summary) {
  const s = plain(summary);
  return typeof s.session_kind === "string" && s.session_kind.startsWith("subagent") && typeof s.parent_session_id === "string" && !!s.parent_session_id;
}

// A session's turns from its usage.json (`usage`), as calls: one for each
// model a turn used, when the turn ended (`t`), with `key` naming it (its
// turn and model), its model calls, tokens and cost (usageCost). A turn
// with no time it ended is left out, as stats leave out a line with none.
// A fork starts with a copy of its parent's ledger, so when the session
// came from another (summary.json's parent_session_id), the turns that
// ended before it was made (`forked_at`, else `created_at`) are its
// parent's, and left out.
export function grokUsageTurns(usage, summary) {
  const s = plain(summary);
  const turns = Array.isArray(plain(usage).turns) ? usage.turns : [];
  const since = typeof s.parent_session_id === "string" && s.parent_session_id ? (grokTime(s.forked_at) ?? grokTime(s.created_at)) : null;
  const fallback = typeof s.current_model_id === "string" && s.current_model_id ? s.current_model_id : "grok";
  const out = [];
  for (const turn of turns) {
    const r = plain(turn);
    const t = grokTime(r.endedAt);
    if (t == null || (since != null && t <= since)) continue;
    const n = typeof r.turnNumber === "number" || typeof r.turnNumber === "string" ? r.turnNumber : "";
    const byModel = Object.entries(plain(r.modelUsage));
    const rows = byModel.length ? byModel : [[typeof r.primaryModelId === "string" && r.primaryModelId ? r.primaryModelId : fallback, r]];
    for (const [model, u] of rows) {
      const row = plain(u);
      const calls = typeof row.modelCalls === "number" && Number.isFinite(row.modelCalls) && row.modelCalls >= 0 ? row.modelCalls : 1;
      out.push({ t, key: `${n}|${model}`, model, calls, tokens: usageTokens(row), cost: usageCost(row) });
    }
  }
  return out;
}

// A session's usage.json and summary.json, from its folder: { usage,
// summary }, each null when it isn't there or doesn't read as JSON. The
// usage goes through its lenient schema (vendor.js), so a change in Grok's
// format shows as drift.
export function readGrokUsage(dir) {
  const raw = readJson(path.join(dir, "usage.json"));
  return { usage: raw === undefined ? null : (usages.sample(raw) ?? null), summary: readJson(path.join(dir, "summary.json")) ?? null };
}

// When a session's usage.json was last written, to tell it changed: its
// size and time, or null when it isn't there.
export function grokUsageStamp(dir) {
  try {
    const st = fs.statSync(path.join(dir, "usage.json"));
    return `${st.size}:${st.mtimeMs}`;
  } catch {
    return null;
  }
}
