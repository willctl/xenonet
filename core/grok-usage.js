// A Grok Build session's tokens and cost, from its usage ledger
// (usage.json beside summary.json, in the shape `grok usage <session id>`
// prints), for the session's stats (stats.js), the live tailer's token
// counts and today's costs (live.js). Apart from grok.js so the tailer can
// read it without XenoNet's config.
import fs from "node:fs";
import path from "node:path";
import { reader } from "./vendor.js";
import { grokTime } from "./lines.js";
import { costParts, plainModel } from "./pricing.js";

const usages = reader("spacexai/grok-build/usage");

const toMs = (t) => {
  const ms = grokTime(t);
  return Number.isFinite(ms) ? ms : null;
};
const readJson = (file) => {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return undefined;
  }
};

// A count by any of the names Grok has used for it: `grok usage`'s
// camelCase, the headless result's snake_case and the status line's.
const count = (o, ...names) => {
  for (const n of names) if (typeof o?.[n] === "number" && Number.isFinite(o[n])) return o[n];
  return null;
};

// One set of counts (a session's, a turn's, a model's) in XenoNet's terms:
// input without the cache, cache reads and writes, output, the total, and
// the cost Grok put on it in dollars (null when it didn't, or said only
// part). Grok's input is the whole prompt in some places and only the
// uncached part in others; the total says which.
export function grokCounts(o) {
  if (!o || typeof o !== "object") return null;
  let input = count(o, "inputTokens", "input_tokens") ?? 0;
  const output = count(o, "outputTokens", "output_tokens") ?? 0;
  const cacheRead = count(o, "cacheReadTokens", "cacheReadInputTokens", "cache_read_input_tokens", "cachedTokens", "cached_tokens") ?? 0;
  const cacheWrite = count(o, "cacheCreationTokens", "cacheCreationInputTokens", "cache_creation_input_tokens") ?? 0;
  const total = count(o, "totalTokens", "total_tokens");
  if (total != null && input + output === total && cacheRead + cacheWrite > 0 && input >= cacheRead + cacheWrite) input -= cacheRead + cacheWrite;
  const ticks = count(o, "costUsdTicks", "cost_usd_ticks", "total_cost_usd_ticks");
  const usd = count(o, "costUSD", "costUsd", "total_cost_usd");
  const partial = o.costIsPartial === true || o.cost_is_partial === true || o.usageIsIncomplete === true;
  return {
    input,
    cacheRead,
    cacheWrite,
    output,
    reasoning: count(o, "reasoningTokens", "reasoning_tokens") ?? 0,
    total: total ?? input + cacheRead + cacheWrite + output,
    calls: count(o, "modelCalls", "model_calls") ?? null,
    cost: partial ? null : ticks != null ? ticks / 1e10 : usd,
  };
}

// A session's usage ledger: its totals, each turn's counts with when it
// ended and its model, and the totals by model. Null when there's none.
// usage.json is what `grok usage` prints; signals.json, which the guide
// says holds token usage too, is read when there's no usage.json.
export function readGrokUsage(dir) {
  for (const name of ["usage.json", "signals.json"]) {
    const raw = readJson(path.join(dir, name));
    if (raw === undefined) continue;
    const u = name === "usage.json" ? usages.check(raw) : raw;
    if (!u || typeof u !== "object") continue;
    const totals = grokCounts(u.session ?? u.totals ?? u.usage ?? u.tokenUsage ?? u.token_usage ?? u);
    const turns = (Array.isArray(u.turns) ? u.turns : []).map((t, i) => ({ n: count(t, "turnNumber", "turn_number") ?? i + 1, at: toMs(t?.endedAt ?? t?.ended_at), model: typeof t?.model === "string" ? t.model : null, ...grokCounts(t) }));
    const byModel = u.modelUsage && typeof u.modelUsage === "object" ? Object.fromEntries(Object.entries(u.modelUsage).map(([m, c]) => [m, grokCounts(c)])) : {};
    if (totals?.total || turns.length || Object.keys(byModel).length) return { totals, turns, byModel, file: name };
  }
  return null;
}

// What a set of counts cost: Grok's own figure where it gave one, else
// what XenoNet's prices make of it (pricing.js), or null for a model
// without a price.
export function grokCost(model, at, c) {
  if (c?.cost != null) return c.cost;
  const parts = costParts(model ?? "unknown", at, { input: c?.input ?? 0, cacheRead: c?.cacheRead ?? 0, cacheWrite5m: c?.cacheWrite ?? 0, cacheWrite1h: 0, output: c?.output ?? 0 });
  return parts ? parts.total : null;
}

export const grokModel = (model) => plainModel(model) || model || "unknown";
