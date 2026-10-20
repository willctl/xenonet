// A session's stats, from its transcript: the models it used and when, its
// tokens and what they'd cost at API prices on the day (pricing.js), its
// turns and how long they took, its tools, its subagents, and how full its
// context got. The console's session details, `xenonet session` and the API
// (GET /api/session/:id/stats) all show this.
//
// Claude Code writes one line per content block, repeating the message's
// usage, so calls are counted once per message id; subagents write files of
// their own. Codex writes a token_count event after each call, with that
// call's tokens in last_token_usage (cached and written tokens are part of
// input_tokens), the model in the turn_context before it, and the service
// tier (the priority tier costs more) in the thread's settings.
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { categorize, mcpServerOf, clip } from "./live.js";
import { costParts, plainModel, priorityTier, PRICES_CHECKED } from "./pricing.js";
import { listFiles } from "./paths.js";
import { readLines, hasUserMessages } from "./lines.js";
import { mapperOf, brokenAt, fail } from "./events.js";

const UUID = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;
const CONTEXT_POINTS = 400;

// ---------- finding the files ----------

// Where sessions' transcripts are. Codex rollouts are found by their thread
// id, from a listing of the rollout folders made once and again when an id
// isn't in it (at most every 10 seconds).
export class Transcripts {
  constructor({ claudeRoot, codexRoots = [] }) {
    this.claudeRoot = claudeRoot;
    this.codexRoots = codexRoots;
    this.rollouts = new Map();
    this.listedAt = 0;
  }

  async files(sessionId) {
    const [app, raw] = [sessionId.slice(0, sessionId.indexOf(":")), sessionId.slice(sessionId.indexOf(":") + 1)];
    if (!UUID.test(raw)) return [];
    if (app === "claude") {
      for (const dir of await fsp.readdir(this.claudeRoot ?? "", { withFileTypes: true }).catch(() => [])) {
        if (!dir.isDirectory()) continue;
        const main = path.join(this.claudeRoot, dir.name, `${raw}.jsonl`);
        if (!fs.existsSync(main)) continue;
        const subDir = path.join(this.claudeRoot, dir.name, raw, "subagents");
        const subs = (await fsp.readdir(subDir).catch(() => [])).filter((f) => f.endsWith(".jsonl")).map((f) => ({ file: path.join(subDir, f), agentId: f.replace(/^agent-/, "").replace(/\.jsonl$/, "") }));
        return [{ file: main, agentId: null }, ...subs];
      }
      return [];
    }
    if (app === "codex") {
      if (!this.rollouts.has(raw) && Date.now() - this.listedAt > 10e3) await this.list();
      const file = this.rollouts.get(raw);
      return file ? [{ file, agentId: null }] : [];
    }
    return [];
  }

  async list() {
    this.listedAt = Date.now();
    for (const root of this.codexRoots) {
      for (const file of await listFiles(root, (name) => /^rollout-.*\.jsonl(\.zst)?$/.test(name), 4)) {
        const m = UUID.exec(path.basename(file));
        if (m) this.rollouts.set(m[1].toLowerCase(), file);
      }
    }
  }
}

// ---------- reading them ----------

// Calls, prompts, turn ends and tool calls, in the order they happened,
// from a session's files (Claude Code's main file and its subagents'), as
// stats counts each line's events (events.js).
function sessionEvents(files, mapper) {
  const events = [];
  // The message ids counted, in every file; and what Codex says apart from
  // its calls: the model, effort, tier and provider, and the total so far.
  const seen = new Set();
  const set = { model: "unknown", effort: null, priority: false, total: 0, provider: null };
  for (const { lines, agentId } of files) {
    // A rollout's prompts are its completed user messages, or in one with
    // none at all, its user_message events (see lines.js).
    const file = mapper.file({ agentId, items: hasUserMessages(lines) });
    for (const o of lines) lineEvents(mapper.events(o, file), seen, set, events);
  }
  return mapper.sorted ? events.sort((a, b) => a.t - b.t) : events;
}

// A line's events as stats counts them, onto `events`.
function lineEvents(evs, seen, set, events) {
  for (const e of evs) {
    // A line that doesn't say when is left out (one that can't say is
    // broken).
    const t = e.t;
    if (!Number.isFinite(t) && !(e.kind === "broken" && e.at === "line")) return;
    const agentId = e.agentId;
    switch (e.kind) {
      case "settings":
        if (e.origin === "session_meta") set.provider = e.provider ?? null;
        else if (e.origin === "turn_context") {
          set.model = e.model ?? set.model;
          set.effort = e.effort ?? set.effort;
          // Older rollouts say the tier with each turn; newer ones say it in
          // the thread's settings, and their turns don't say it.
          if (e.tier !== undefined) set.priority = priorityTier(e.tier);
        }
        // All of the thread's settings, before the turn they apply to: a
        // tier not said is the default one.
        else if (e.origin === "event_msg:thread_settings_applied") set.priority = priorityTier(e.tier);
        break;
      case "call": {
        if ("total" in e) {
          // Codex reports a call more than once; it's counted when the total
          // moves.
          const now = e.total?.total_tokens ?? 0;
          if (now <= set.total) break;
          set.total = now;
          const u = usageOf(evs, e);
          events.push({ kind: "call", t, agentId, model: set.provider && set.provider !== "openai" ? `${set.provider}/${set.model}` : set.model, effort: set.effort, tokens: tokensOf(u), priority: set.priority, window: e.window ?? null });
          break;
        }
        // Each of a message's lines repeats its usage: counted once.
        const id = e.messageId;
        const u = id && !seen.has(id) ? usageOf(evs, e) : null;
        if (!u) break;
        seen.add(id);
        events.push({ kind: "call", t, agentId, model: e.model ?? "unknown", effort: e.effort, tokens: tokensOf(u), fast: u.fast });
        break;
      }
      case "tool":
        // A Codex call (a response item, with its type) is named by its
        // type when it has no name: a local_shell_call is a shell call.
        if (e.call !== undefined) {
          events.push({ kind: "tool", t, id: e.id, name: e.name ?? e.call, agentId });
          break;
        }
        // The Agent call that starts a subagent names it.
        events.push({ kind: "tool", t, id: e.id, name: e.name, agentId });
        if (!agentId && /^(Agent|Task)$/.test(e.name) && e.input?.description) events.push({ kind: "spawn", t, text: e.input.description });
        break;
      case "result":
        events.push({ kind: "result", t, id: e.id, error: e.error ?? null });
        break;
      case "turnEnd":
        // A subagent's end isn't the turn's; an interruption or a refusal
        // isn't counted.
        if ((e.reason === "end_turn" || e.reason === "task_complete") && !agentId) events.push({ kind: "end", t });
        break;
      // A compacted context carries on the turn it interrupted.
      case "compact":
        if (!agentId && !e.meta && !e.sidechain) events.push({ kind: "compact", t });
        break;
      case "prompt": {
        // A subagent's first prompt is its task.
        if (agentId) {
          if (e.text.trim()) events.push({ kind: "task", t, agentId, text: e.text });
          break;
        }
        if (e.meta || e.sidechain || e.compactSummary || e.interruption) break;
        const text = "command" in e ? promptText(e) : e.text;
        if (text) events.push({ kind: "prompt", t, text });
        break;
      }
      case "broken":
        if (reaches(e)) fail(e);
        break;
    }
  }
}

// Whether stats reads the field a line broke at: every one but the
// refusal, a usage it reads when it counts the call (usageOf), and a
// user line's text, which it reads for a subagent's task and for a prompt.
function reaches(e) {
  if (e.at === "refusal" || e.at === "usage") return false;
  if (e.at === "text") return !e.results && (!!e.agentId || (!e.meta && !e.sidechain && !e.compactSummary));
  return true;
}

// A call's usage, which stats reads only for a call it counts.
function usageOf(evs, e) {
  const bad = brokenAt(evs, "usage");
  if (bad) fail(bad);
  return e.usage;
}

const tokensOf = (u) => ({ input: u.input, cacheRead: u.cacheRead, cacheWrite5m: u.cacheWrite5m, cacheWrite1h: u.cacheWrite1h, output: u.output });

// What someone typed in Claude Code: a slash command as /name and its
// arguments; nothing for Claude Code's own notes (command output and the
// like).
function promptText({ text, command }) {
  if (command) return `/${command.name} ${command.args}`.trim();
  const t = text.trim();
  return t && !t.startsWith("<") ? t : null;
}

// ---------- adding it up ----------

const KINDS = ["input", "cacheRead", "cacheWrite", "output"];
const zero = () => ({ input: 0, cacheRead: 0, cacheWrite: 0, output: 0 });

export function summarize(events, { app }) {
  const totals = { calls: 0, ...zero(), tokens: 0, cost: 0, unpricedCalls: 0 };
  const kindCost = zero();
  const models = new Map();
  const subagents = new Map();
  const tools = new Map();
  const open = new Map();
  const turns = [];
  const context = [];
  const compactions = [];
  const labels = new Map(); // subagent → its task
  const spawns = []; // Agent calls, whose descriptions name the subagents they start
  const unpriced = new Set();
  let turn = null;
  const startTurn = (t, prompt) => {
    turn = { n: turns.length + 1, start: t, end: t, prompt: prompt == null ? null : clip(prompt, 140), tokens: 0, cost: 0, byModel: {}, tools: 0, errors: 0, ended: false };
    turns.push(turn);
  };

  for (const e of events) {
    if (e.kind === "prompt") {
      startTurn(e.t, e.text);
      continue;
    }
    if (e.kind === "task") {
      labels.set(e.agentId, labels.get(e.agentId) ?? clip(e.text, 80));
      continue;
    }
    if (e.kind === "spawn") {
      spawns.push(e);
      continue;
    }
    if (!turn) startTurn(e.t, null);
    if (!turn.ended || e.kind === "end") turn.end = Math.max(turn.end, e.t);
    if (e.kind === "end") turn.ended = true;
    else if (e.kind === "compact") {
      compactions.push(e.t);
      turn.ended = false;
    }
    else if (e.kind === "call") {
      // A call after the turn ended (a subagent finishing, a hook) still counts towards it.
      turn.end = Math.max(turn.end, e.t);
      const k = e.tokens;
      const parts = costParts(e.model, e.t, k, { fast: e.fast, priority: e.priority });
      const tokens = { input: k.input, cacheRead: k.cacheRead, cacheWrite: k.cacheWrite5m + k.cacheWrite1h, output: k.output };
      const sum = KINDS.reduce((a, x) => a + tokens[x], 0);
      totals.calls++;
      totals.tokens += sum;
      for (const x of KINDS) totals[x] += tokens[x];
      if (parts) {
        totals.cost += parts.total;
        for (const x of KINDS) kindCost[x] += parts[x];
      } else {
        totals.unpricedCalls++;
        unpriced.add(plainModel(e.model) || e.model);
      }
      const name = plainModel(e.model) || e.model;
      const m = models.get(name) ?? models.set(name, { model: name, calls: 0, tokens: 0, cost: 0, priced: !!parts, from: e.t, to: e.t, efforts: new Set() }).get(name);
      m.calls++;
      m.tokens += sum;
      m.cost += parts?.total ?? 0;
      m.to = e.t;
      if (e.effort) m.efforts.add(e.effort);
      turn.tokens += sum;
      turn.cost += parts?.total ?? 0;
      const tm = (turn.byModel[name] ??= { tokens: 0, cost: 0 });
      tm.tokens += sum;
      tm.cost += parts?.total ?? 0;
      if (e.agentId) {
        const a = subagents.get(e.agentId) ?? subagents.set(e.agentId, { id: e.agentId, calls: 0, tokens: 0, cost: 0, model: name, from: e.t, to: e.t }).get(e.agentId);
        a.calls++;
        a.tokens += sum;
        a.cost += parts?.total ?? 0;
        a.to = e.t;
      } else {
        const used = k.input + k.cacheRead + k.cacheWrite5m + k.cacheWrite1h + (app === "claude" ? k.output : 0);
        const window = e.window ?? (used > 200000 || /\[1m\]/i.test(e.model) ? 1000000 : 200000);
        context.push({ t: e.t, used, window });
      }
    } else if (e.kind === "tool") {
      turn.tools++;
      open.set(e.id, e);
      const t = tools.get(e.name) ?? tools.set(e.name, { name: e.name, category: categorize(e.name), server: mcpServerOf(e.name), calls: 0, errors: 0, ms: 0, timed: 0 }).get(e.name);
      t.calls++;
    } else if (e.kind === "result") {
      const use = open.get(e.id);
      if (!use) continue;
      open.delete(e.id);
      const t = tools.get(use.name);
      t.ms += Math.max(0, e.t - use.t);
      t.timed++;
      if (e.error) {
        t.errors++;
        turn.errors++;
      }
    }
  }

  for (const t of turns) delete t.ended;
  const first = events[0]?.t ?? null, last = events.at(-1)?.t ?? null;
  const priced = totals.calls > totals.unpricedCalls;
  return {
    app,
    from: first,
    to: last,
    activeMs: turns.reduce((a, t) => a + (t.end - t.start), 0),
    totals: { ...totals, cost: priced ? totals.cost : null },
    kinds: Object.fromEntries(KINDS.map((x) => [x, { tokens: totals[x], cost: priced ? kindCost[x] : null }])),
    models: [...models.values()].map(({ efforts, ...m }) => ({ ...m, cost: m.priced ? m.cost : null, efforts: [...efforts] })).sort((a, b) => (b.cost ?? -1) - (a.cost ?? -1) || b.tokens - a.tokens),
    turns,
    tools: [...tools.values()].map(({ timed, ...t }) => ({ ...t, ms: timed ? t.ms : null })).sort((a, b) => b.calls - a.calls),
    subagents: nameSubagents([...subagents.values()].sort((a, b) => a.from - b.from), spawns, labels),
    context: thin(context, CONTEXT_POINTS),
    compactions,
    pricing: { checked: PRICES_CHECKED, unpriced: [...unpriced] },
  };
}

// A subagent's name: the description of the Agent call made just before it
// started (each used once), or else the start of its task.
function nameSubagents(list, spawns, labels) {
  const used = new Set();
  return list.map((a) => {
    const spawn = spawns.filter((sp) => !used.has(sp) && sp.t <= a.from + 2000 && a.from - sp.t < 120e3).at(-1);
    if (spawn) used.add(spawn);
    return { ...a, label: spawn ? clip(spawn.text, 80) : (labels.get(a.id) ?? null) };
  });
}

// At most n points, keeping the first, the last and each stretch's peak.
function thin(points, n) {
  if (points.length <= n) return points;
  const size = points.length / n;
  const out = [];
  for (let i = 0; i < n; i++) {
    const slice = points.slice(Math.floor(i * size), Math.floor((i + 1) * size));
    if (slice.length) out.push(slice.reduce((a, b) => (b.used > a.used ? b : a)));
  }
  out[out.length - 1] = points.at(-1);
  return out;
}

// ---------- the whole thing, cached ----------

export function createStats({ transcripts }) {
  const cache = new Map(); // session id → { key, stats }

  return async function stats(sessionId) {
    const files = await transcripts.files(sessionId);
    if (!files.length) return null;
    const stamps = await Promise.all(files.map((f) => fsp.stat(f.file).then((s) => `${f.file}:${s.size}:${s.mtimeMs}`, () => f.file)));
    const key = stamps.join("|");
    const hit = cache.get(sessionId);
    if (hit?.key === key) return hit.stats;
    const app = sessionId.split(":")[0];
    const mapper = mapperOf(app);
    const read = await Promise.all(files.map(async (f) => ({ ...f, lines: await readLines(f.file, app, mapper.lines).catch(() => []) })));
    const events = sessionEvents(read, mapper);
    const stats = { id: sessionId, ...summarize(events, { app }) };
    cache.set(sessionId, { key, stats });
    if (cache.size > 40) cache.delete(cache.keys().next().value);
    return stats;
  };
}
