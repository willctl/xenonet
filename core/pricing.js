// What a call to a model cost at API prices on the day it was made: the
// prices Anthropic, OpenAI and SpaceXAI (formerly xAI) published, with the
// date each took effect, in US dollars per million tokens at the standard
// tier. Sessions run on a plan
// (Claude Max, ChatGPT Pro…) cost the plan, not this; it's what the same work
// would cost on the API.
//
// Checked 2026-10-03 against:
//   Anthropic  platform.claude.com/docs/en/about-claude/pricing, each model's
//              overview page, the release notes, and archived copies of the
//              pricing page (web.archive.org) for earlier prices
//   OpenAI     developers.openai.com/api/docs/pricing, each model's page, the
//              changelog, and archived copies of the pricing page
//
// and SpaceXAI's checked 2026-10-10 against docs.x.ai/developers/pricing
// (the Text API table, global endpoint) and each model's page there.
//
// Not counted: batch (half price), data residency (1.1x; xAI's US endpoint
// too), and Codex models OpenAI doesn't price (codex-auto-review,
// gpt-5.3-codex-spark).
//
// A row: [models, from, input, output, cacheWrite5m, cacheWrite1h, cacheRead, extra]
//   models   ids separated by |; a model matches its own id, or the longest
//            id it starts with followed by "-" (claude-opus-4-1-20250805
//            matches claude-opus-4-1)
//   from     the day the price took effect (or the model reached the API)
//   OpenAI rows keep their one cache-write price (GPT-5.6 and later) in
//   cacheWrite5m.
//   extra    longContext: the rates for a whole call above `over` input
//            tokens, until `until`; fast: Anthropic's fast mode rates
//            (cache rates scale with input)
export const PRICES_CHECKED = "2026-10-03";

const ANTHROPIC = [
  ["claude-3-5-sonnet-20240620|claude-3-5-sonnet-20241022|claude-3-5-sonnet", "2024-06-20", 3, 15, 3.75, 6, 0.3],
  ["claude-3-5-haiku-20241022|claude-3-5-haiku", "2024-11-04", 1, 5, 1.25, null, 0.1],
  ["claude-3-5-haiku-20241022|claude-3-5-haiku", "2024-12-03", 0.8, 4, 1, 1.6, 0.08],
  ["claude-3-7-sonnet-20250219|claude-3-7-sonnet", "2025-02-24", 3, 15, 3.75, 6, 0.3],
  ["claude-opus-4-20250514|claude-opus-4-0|claude-opus-4", "2025-05-22", 15, 75, 18.75, 30, 1.5],
  ["claude-sonnet-4-20250514|claude-sonnet-4-0|claude-sonnet-4", "2025-05-22", 3, 15, 3.75, 6, 0.3, { longContext: { over: 200000, from: "2025-08-12", until: "2026-04-29", input: 6, output: 22.5 } }],
  ["claude-opus-4-1-20250805|claude-opus-4-1", "2025-08-05", 15, 75, 18.75, 30, 1.5],
  ["claude-sonnet-4-5-20250929|claude-sonnet-4-5", "2025-09-29", 3, 15, 3.75, 6, 0.3, { longContext: { over: 200000, until: "2026-04-29", input: 6, output: 22.5 } }],
  ["claude-haiku-4-5-20251001|claude-haiku-4-5", "2025-10-15", 1, 5, 1.25, 2, 0.1],
  ["claude-opus-4-5-20251101|claude-opus-4-5", "2025-11-24", 5, 25, 6.25, 10, 0.5],
  ["claude-opus-4-6", "2026-02-05", 5, 25, 6.25, 10, 0.5, { longContext: { over: 200000, until: "2026-03-12", input: 10, output: 37.5 }, fast: { input: 30, output: 150 } }],
  ["claude-sonnet-4-6", "2026-02-17", 3, 15, 3.75, 6, 0.3, { longContext: { over: 200000, until: "2026-03-12", input: 6, output: 22.5 } }],
  ["claude-opus-4-7", "2026-04-16", 5, 25, 6.25, 10, 0.5, { fast: { input: 30, output: 150 } }],
  ["claude-opus-4-8", "2026-05-28", 5, 25, 6.25, 10, 0.5, { fast: { input: 10, output: 50 } }],
  ["claude-fable-5|claude-mythos-5", "2026-06-09", 10, 50, 12.5, 20, 1],
  ["claude-sonnet-5", "2026-06-30", 2, 10, 2.5, 4, 0.2],
  ["claude-opus-5", "2026-07-24", 5, 25, 6.25, 10, 0.5, { fast: { input: 10, output: 50 } }],
  ["claude-fable-5-1|claude-mythos-5-1", "2026-09-01", 10, 50, 12.5, 20, 0.25],
  ["claude-opus-5-5", "2026-09-22", 4, 20, 5, 8, 0.2, { fast: { input: 8, output: 40 } }],
  ["claude-sonnet-5-5", "2026-09-28", 2, 10, 2.5, 4, 0.2],
];

// OpenAI's long context: above 272K input tokens the whole call costs 2x for
// input, cached input and cache writes, and 1.5x for output.
const lc = (input, output) => ({ longContext: { over: 272000, input: input * 2, output: output * 1.5, scaleCache: 2 } });
const OPENAI = [
  ["gpt-4.1", "2025-04-14", 2, 8, null, null, 0.5],
  ["o3", "2025-04-16", 10, 40, null, null, 2.5],
  ["o3", "2025-06-10", 2, 8, null, null, 0.5],
  ["o4-mini", "2025-04-16", 1.1, 4.4, null, null, 0.275],
  ["codex-mini-latest", "2025-05-15", 1.5, 6, null, null, 0.375],
  ["gpt-5", "2025-08-07", 1.25, 10, null, null, 0.125],
  ["gpt-5-mini", "2025-08-07", 0.25, 2, null, null, 0.025],
  ["gpt-5-codex", "2025-09-15", 1.25, 10, null, null, 0.125],
  ["gpt-5.1|gpt-5.1-codex", "2025-11-13", 1.25, 10, null, null, 0.125],
  ["gpt-5.1-codex-mini", "2025-11-13", 0.25, 2, null, null, 0.025],
  ["gpt-5.1-codex-max", "2025-11-19", 1.25, 10, null, null, 0.125],
  ["gpt-5.2", "2025-12-11", 1.75, 14, null, null, 0.175],
  ["gpt-5.2-codex", "2025-12-18", 1.75, 14, null, null, 0.175],
  ["gpt-5.3-codex", "2026-02-05", 1.75, 14, null, null, 0.175],
  ["gpt-5.4", "2026-03-05", 2.5, 15, null, null, 0.25, lc(2.5, 15)],
  ["gpt-5.4-mini", "2026-03-17", 0.75, 4.5, null, null, 0.075],
  ["gpt-5.5", "2026-04-23", 5, 30, null, null, 0.5, lc(5, 30)],
  ["gpt-5.6-sol|gpt-5.6", "2026-07-09", 5, 30, 6.25, null, 0.5, lc(5, 30)],
  ["gpt-5.6-sol|gpt-5.6", "2026-08-21", 4, 20, 5, null, 0.4, lc(4, 20)],
  ["gpt-5.6-terra", "2026-07-09", 2.5, 15, 3.125, null, 0.25, lc(2.5, 15)],
  ["gpt-5.6-terra", "2026-07-30", 2, 12, 2.5, null, 0.2, lc(2, 12)],
  ["gpt-5.6-luna", "2026-07-09", 1, 6, 1.25, null, 0.1, lc(1, 6)],
  ["gpt-5.6-luna", "2026-07-30", 0.2, 1.2, 0.25, null, 0.02, lc(0.2, 1.2)],
  ["gpt-6-astra", "2026-09-03", 10, 50, 12.5, null, 1, lc(10, 50)],
  ["gpt-6-sol", "2026-09-22", 2, 10, 2.5, null, 0.2, lc(2, 10)],
  ["gpt-6-luna", "2026-09-22", 0.1, 0.5, 0.125, null, 0.01, lc(0.1, 0.5)],
  ["gpt-6.1-sol", "2026-09-29", 2, 10, 2.5, null, 0.1, lc(2, 10)],
];

// SpaceXAI's Grok models, which Grok Build runs. Above 200K prompt tokens
// the whole call costs the long-context rates, cached input twice its
// price; writing the cache costs what input does. xAI's pages don't say
// when each price took effect, so each row starts on the day it was
// checked, and a call before it gets the same price (see priceOf).
const lcx = (input, output) => ({ longContext: { over: 199999, input, output, scaleCache: 2 } });
const XAI = [
  ["grok-4.20-0309-reasoning|grok-4.20-0309-non-reasoning|grok-4.20-multi-agent-0309|grok-4.20", "2026-10-10", 1.25, 2.5, 1.25, null, 0.2, lcx(2.5, 5)],
  ["grok-4.3", "2026-10-10", 1.25, 2.5, 1.25, null, 0.2, lcx(2.5, 5)],
  ["grok-4.5", "2026-10-10", 2, 6, 2, null, 0.3, lcx(4, 12)],
  ["grok-4.6", "2026-10-10", 2, 6, 2, null, 0.5, lcx(4, 12)],
  ["grok-build-0.1|grok-build", "2026-10-10", 1, 2, 1, null, 0.2, lcx(2, 4)],
];

// OpenAI's priority tier (Fast in Codex): about twice standard.
const PRIORITY = { "gpt-5.5": 2.5, "gpt-5-mini": 1.8, o3: 1.75, "gpt-4.1": 1.75 };

const rows = [...ANTHROPIC.map((r) => ["anthropic", ...r]), ...OPENAI.map((r) => ["openai", ...r]), ...XAI.map((r) => ["xai", ...r])].map(([provider, ids, from, input, output, write5m, write1h, read, extra = {}]) => ({
  provider,
  ids: ids.split("|"),
  from: Date.parse(`${from}T00:00:00Z`),
  input,
  output,
  write5m,
  write1h,
  read,
  ...extra,
}));

// Whether a service tier is OpenAI's priority tier (Fast, in Codex).
export const priorityTier = (tier) => /^(priority|fast)$/i.test(tier ?? "");

// The tailer prices every reply it reads, a few models over and over, and
// matching an id against every row took most of a call, so each model's
// plain id and its price rows are worked out once. Each map holds 500
// models at most, and starts over past that.
const KEPT_MODELS = 500;
const keep = (map, key, value) => {
  if (map.size >= KEPT_MODELS) map.clear();
  map.set(key, value);
  return value;
};
const plainIds = new Map(); // model as written → plain id
const rowsOf = new Map(); // plain id → its price rows, oldest first, or null

// A model id as the transcripts write it, plain: no provider or region
// prefix (Bedrock's us.anthropic.…-v1:0, Vertex's …@date, xai/grok-4.6),
// no [1m] flag.
export function plainModel(model) {
  const written = String(model ?? "");
  const id = plainIds.get(written);
  if (id !== undefined) return id;
  return keep(
    plainIds,
    written,
    written
      .toLowerCase()
      .replace(/\[[^\]]*\]$/, "")
      .replace(/^(?:[a-z]{2,4}\.)?(?:anthropic|openai|x-?ai|spacexai)[./]/, "")
      .replace(/-v\d+(?::\d+)?$/, "")
      .replace(/@(\d{8})$/, "-$1"),
  );
}

// A plain id's price rows, oldest first: those of the longest id it is or
// starts with, followed by "-". Null when XenoNet doesn't know one.
function priceRows(id) {
  let best = null;
  for (const r of rows)
    for (const alt of r.ids)
      if ((id === alt || id.startsWith(`${alt}-`)) && (!best || alt.length > best.alt.length)) best = { alt, key: r.ids[0] };
  return best ? rows.filter((r) => r.ids[0] === best.key).sort((a, b) => a.from - b.from) : null;
}

// The price row for a model on a day, or null when XenoNet doesn't know one.
export function priceOf(model, at = Date.now()) {
  const id = plainModel(model);
  if (!id || id === "<synthetic>") return null;
  const options = rowsOf.has(id) ? rowsOf.get(id) : keep(rowsOf, id, priceRows(id));
  if (!options) return null;
  return options.filter((r) => r.from <= at).pop() ?? options[0];
}

// What one call cost, in dollars, by kind of token ({ input, cacheRead,
// cacheWrite, output, total }), or null for a model without a price. `u` is
// the call's tokens: input (not cached), cacheRead, cacheWrite5m,
// cacheWrite1h and output (reasoning included); `fast` is Anthropic's fast
// mode, `priority` OpenAI's priority tier.
export function costParts(model, at, u, { fast = false, priority = false } = {}) {
  if (plainModel(model) === "<synthetic>") return { input: 0, cacheRead: 0, cacheWrite: 0, output: 0, total: 0 };
  const p = priceOf(model, at);
  if (!p) return null;
  const prompt = (u.input ?? 0) + (u.cacheRead ?? 0) + (u.cacheWrite5m ?? 0) + (u.cacheWrite1h ?? 0);
  const l = p.longContext;
  const long = l && prompt > l.over && (!l.from || at >= Date.parse(`${l.from}T00:00:00Z`)) && (!l.until || at < Date.parse(`${l.until}T00:00:00Z`) + 86400e3);
  const [input, output] = long ? [l.input, l.output] : [p.input, p.output];
  const cacheScale = long ? (l.scaleCache ?? l.input / p.input) : 1;
  let scale = 1;
  if (fast && p.fast) scale = p.fast.input / p.input;
  if (priority && p.provider === "openai") scale = PRIORITY[p.ids[0]] ?? 2;
  // Cache writes without a price of their own: Anthropic's are 1.25x input
  // (5 minutes) and 2x (an hour); OpenAI's, before GPT-5.6, cost nothing extra.
  const write5m = p.write5m ?? (p.provider === "openai" ? p.input : p.input * 1.25);
  const write1h = p.write1h ?? p.input * 2;
  const d = (n, rate) => ((n ?? 0) * rate * scale) / 1e6;
  const parts = {
    input: d(u.input, input),
    cacheRead: d(u.cacheRead, p.read * cacheScale),
    cacheWrite: d(u.cacheWrite5m, write5m * cacheScale) + d(u.cacheWrite1h, write1h * cacheScale),
    output: d(u.output, output),
  };
  return { ...parts, total: parts.input + parts.cacheRead + parts.cacheWrite + parts.output };
}

export const costOf = (model, at, u, opts) => costParts(model, at, u, opts)?.total ?? null;
