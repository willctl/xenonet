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
// tier (the priority tier costs more) in the thread's settings. Grok Build's
// updates say what was asked and which tools ran, and its usage ledger
// says each turn's tokens, model and cost (grok-usage.js), which are its
// calls here: Grok's own cost where it gave one.
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import { categorize, mcpServerOf, clip } from "./live.js";
import { costParts, plainModel, priorityTier, PRICES_CHECKED } from "./pricing.js";
import { listFiles } from "./paths.js";
import { readLines, isCodexPrompt, hasUserMessages, GrokLine } from "./lines.js";
import { readGrokUsage } from "./grok-usage.js";

const UUID = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})/i;
const CONTEXT_POINTS = 400;

// ---------- finding the files ----------

// Where sessions' transcripts are. Codex rollouts are found by their thread
// id, from a listing of the rollout folders made once and again when an id
// isn't in it (at most every 10 seconds).
export class Transcripts {
  constructor({ claudeRoot, codexRoots = [], grokRoot = null }) {
    this.claudeRoot = claudeRoot;
    this.codexRoots = codexRoots;
    this.grokRoot = grokRoot;
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
    // A Grok Build session's updates, in the folder named for it, in one
    // of the sessions folder's groups.
    if (app === "grok") {
      for (const group of await fsp.readdir(this.grokRoot ?? "", { withFileTypes: true }).catch(() => [])) {
        if (!group.isDirectory()) continue;
        const file = path.join(this.grokRoot, group.name, raw, "updates.jsonl");
        if (fs.existsSync(file)) return [{ file, agentId: null }];
      }
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
// from Claude Code's main file and its subagents' files.
function claudeEvents(files) {
  const events = [];
  const seen = new Set();
  for (const { lines, agentId } of files) {
    for (const o of lines) {
      // A line that doesn't say when is left out.
      const t = o.ts;
      if (!Number.isFinite(t)) continue;
      const type = o.type;
      if (type === "assistant") {
        // Each of a message's lines repeats its usage: counted once.
        const id = o.messageId;
        const u = id && !seen.has(id) ? o.usage : null;
        if (u) {
          seen.add(id);
          events.push({
            kind: "call",
            t,
            agentId,
            model: o.model ?? "unknown",
            effort: o.effort,
            tokens: { input: u.input, cacheRead: u.cacheRead, cacheWrite5m: u.cacheWrite5m, cacheWrite1h: u.cacheWrite1h, output: u.output },
            fast: u.fast,
          });
        }
        for (const b of o.blocks) {
          if (b.type !== "tool_use") continue;
          events.push({ kind: "tool", t, id: b.id, name: b.name, agentId });
          if (!agentId && /^(Agent|Task)$/.test(b.name) && b.input?.description) events.push({ kind: "spawn", t, text: b.input.description });
        }
        if (o.endTurn && !agentId) events.push({ kind: "end", t });
      } else if (type === "user") {
        const results = o.results;
        if (results) {
          for (const r of results) events.push({ kind: "result", t, id: r.id, error: r.error });
          continue;
        }
        // A subagent's first prompt is its task.
        if (agentId) {
          const task = o.text;
          if (task.trim()) events.push({ kind: "task", t, agentId, text: task });
          continue;
        }
        if (o.meta || o.sidechain) continue;
        // A compacted context carries on the turn it interrupted.
        if (o.compactSummary) {
          events.push({ kind: "compact", t });
          continue;
        }
        if (o.interruption) continue;
        const text = promptText(o.text);
        if (text) events.push({ kind: "prompt", t, text });
      }
    }
  }
  return events.sort((a, b) => a.t - b.t);
}

// What someone typed: a slash command as /name and its arguments; nothing
// for Claude Code's own notes (command output and the like).
function promptText(text) {
  const t = text.trim();
  const command = /^<command-name>\/?([^<]+)<\/command-name>/.exec(t);
  if (command) return `/${command[1]} ${/<command-args>([\s\S]*?)<\/command-args>/.exec(t)?.[1] ?? ""}`.trim();
  return t && !t.startsWith("<") ? t : null;
}


function codexEvents(lines) {
  const events = [];
  // Its prompts are its completed user messages, or in a rollout with none
  // at all, its user_message events (see lines.js).
  const items = hasUserMessages(lines);
  let model = "unknown", effort = null, priority = false, total = 0, provider = null;
  for (const o of lines) {
    // A line that doesn't say when is left out.
    const t = o.ts;
    if (!Number.isFinite(t)) continue;
    const p = o.payload;
    const type = o.type;
    if (type === "session_meta") provider = p.model_provider ?? null;
    if (type === "turn_context") {
      model = p.model ?? model;
      effort = p.effort ?? effort;
      // Older rollouts say the tier with each turn; newer ones say it in
      // the thread's settings (below), and their turns don't say it.
      if (p.service_tier !== undefined) priority = priorityTier(p.service_tier);
    } else if (type === "event_msg") {
      const said = items ? o.userMessage : o.userEvent;
      if (said != null) {
        if (isCodexPrompt(said)) events.push({ kind: "prompt", t, text: said });
      } else if (p.type === "thread_settings_applied") {
        // All of the thread's settings, before the turn they apply to: a
        // tier not said is the default one.
        priority = priorityTier(p.thread_settings?.service_tier);
      } else if (p.type === "task_complete") events.push({ kind: "end", t });
      else if (p.type === "token_count" && p.info) {
        const now = p.info.total_token_usage?.total_tokens ?? 0;
        // The same call is reported more than once; count it when the total moves.
        if (now <= total) continue;
        total = now;
        const u = p.info.last_token_usage ?? {};
        const input = u.input_tokens ?? Math.max(0, (u.total_tokens ?? 0) - (u.output_tokens ?? 0));
        const cached = u.cached_input_tokens ?? 0, write = u.cache_write_input_tokens ?? 0;
        events.push({
          kind: "call",
          t,
          agentId: null,
          model: provider && provider !== "openai" ? `${provider}/${model}` : model,
          effort,
          tokens: { input: Math.max(0, input - cached - write), cacheRead: cached, cacheWrite5m: write, cacheWrite1h: 0, output: u.output_tokens ?? 0 },
          priority,
          window: p.info.model_context_window ?? null,
        });
      }
    } else if (type === "response_item" && /^(function_call|custom_tool_call|local_shell_call)$/.test(p.type)) {
      events.push({ kind: "tool", t, id: p.call_id ?? p.id, name: p.name ?? p.type, agentId: null });
    } else if (type === "response_item" && /^(function_call_output|custom_tool_call_output|local_shell_call_output)$/.test(p.type)) {
      events.push({ kind: "result", t, id: p.call_id ?? p.id, error: null });
    }
  }
  return events;
}

// A Grok Build session's prompts (each run of your chunks), tool calls and
// their ends from its updates, and its calls from its usage ledger: each
// turn's tokens as one call when the turn ended, with Grok's cost for it.
// `window`: the context window its usage updates said last.
export function grokEvents(lines, usage) {
  const events = [];
  let run = null;
  let window = null;
  const end = () => {
    if (run?.text.trim()) events.push({ kind: "prompt", t: run.t, text: run.text.trim() });
    run = null;
  };
  for (const raw of lines) {
    const o = raw instanceof GrokLine ? raw : new GrokLine(raw);
    const t = o.ts;
    if (!Number.isFinite(t)) continue;
    if (o.kind === "user_message_chunk") {
      if (run) run.text += o.text;
      else run = { t, text: o.text };
      continue;
    }
    end();
    const u = o.update;
    if (o.kind === "tool_call") {
      const name = o.toolName ?? (u.title || "tool");
      events.push({ kind: "tool", t, id: o.toolCallId, name, agentId: null });
      if (categorize(name) === "agent") events.push({ kind: "spawn", t, text: String(u.rawInput?.description ?? u.title ?? "subagent") });
      if (u.status === "completed" || u.status === "failed") events.push({ kind: "result", t, id: o.toolCallId, error: u.status === "failed" });
    } else if (o.kind === "tool_call_update" && (u.status === "completed" || u.status === "failed")) {
      events.push({ kind: "result", t, id: o.toolCallId, error: u.status === "failed" });
    } else if (o.kind === "usage_update" && typeof u.size === "number" && u.size > 0) window = u.size;
  }
  end();
  for (const turn of usage?.turns ?? []) {
    if (!Number.isFinite(turn.at) || !(turn.total > 0)) continue;
    events.push({ kind: "call", t: turn.at, agentId: null, model: turn.model ?? "unknown", tokens: { input: turn.input, cacheRead: turn.cacheRead, cacheWrite5m: turn.cacheWrite, cacheWrite1h: 0, output: turn.output }, cost: turn.cost, window });
  }
  return events.sort((a, b) => a.t - b.t);
}

// A call's cost by kind of token: XenoNet's prices, or where the app gave
// the call's cost itself, that cost split as XenoNet's prices split it, or
// by tokens for a model without a price.
function partsOf(e) {
  const parts = costParts(e.model, e.t, e.tokens, { fast: e.fast, priority: e.priority });
  if (e.cost == null) return parts;
  const k = e.tokens;
  const weights = parts && parts.total > 0 ? parts : { input: k.input, cacheRead: k.cacheRead, cacheWrite: k.cacheWrite5m + k.cacheWrite1h, output: k.output, total: k.input + k.cacheRead + k.cacheWrite5m + k.cacheWrite1h + k.output };
  const share = (x) => (weights.total > 0 ? (e.cost * weights[x]) / weights.total : 0);
  return { input: share("input"), cacheRead: share("cacheRead"), cacheWrite: share("cacheWrite"), output: share("output"), total: e.cost };
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
      const parts = partsOf(e);
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
    // A Grok Build session's usage ledger changes its stats too.
    const ledger = sessionId.startsWith("grok:") ? [path.join(path.dirname(files[0].file), "usage.json")] : [];
    const stamps = await Promise.all([...files.map((f) => f.file), ...ledger].map((file) => fsp.stat(file).then((s) => `${file}:${s.size}:${s.mtimeMs}`, () => file)));
    const key = stamps.join("|");
    const hit = cache.get(sessionId);
    if (hit?.key === key) return hit.stats;
    const app = sessionId.split(":")[0];
    const read = await Promise.all(files.map(async (f) => ({ ...f, lines: await readLines(f.file, app).catch(() => []) })));
    const events = app === "codex" ? codexEvents(read[0].lines) : app === "grok" ? grokEvents(read[0].lines, readGrokUsage(path.dirname(read[0].file))) : claudeEvents(read);
    const stats = { id: sessionId, ...summarize(events, { app }) };
    cache.set(sessionId, { key, stats });
    if (cache.size > 40) cache.delete(cache.keys().next().value);
    return stats;
  };
}
