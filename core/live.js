// Real-time telemetry. Tails the transcript files both apps append to as they
// work (Claude Code: ~/.claude/projects/**/*.jsonl, Codex: ~/.codex/sessions/**
// rollout-*.jsonl) and turns new lines into activity events: tool calls,
// prompts, turns, subagents, token use and context fill. It also derives the
// signals the UI acts on: sessions waiting for you, stuck sessions, edit
// collisions, context and usage-limit forecasts.
//
// These line formats are internal to each app and change between versions.
// Everything here is best-effort: unknown lines are skipped, and the graph
// itself never depends on this module (it comes from the official APIs).
import fs from "node:fs";
import path from "node:path";
import { EventEmitter } from "node:events";
import { createHash } from "node:crypto";
import { tilde, baseName } from "./paths.js";
import { Tailer } from "./blocks/tailer.js";
import { claudeUsageGatherer, desktopUsageGatherer } from "./claude-usage.js";
import { ClaudeLine, CodexLine, linesOf, isCodexPrompt } from "./lines.js";
import { claudeSignal, codexSignal } from "./refusals/detect.js";
import { costParts, plainModel, priorityTier, PRICES_CHECKED } from "./pricing.js";
import { isContextServer } from "./xenolith.js";
import { APP, tailedApp } from "./apps.js";
import { patchPaths } from "./patch.js";

const PRIME_WINDOW_MS = 24 * 3600e3;
const SWEEP_MS = 2000;
const DISCOVER_EVERY = 15; // sweeps: every 30 seconds
const RATE_WINDOW_S = 300;
const FILE_MEMORY_MS = 20 * 60e3;
const COLLISION_WINDOW_MS = 10 * 60e3;
const SUBAGENT_IDLE_MS = 45e3;
const TURN_STALE_MS = 10 * 60e3;
const SILENT_MS = 4 * 60e3;
const QUESTION_TTL_MS = 12 * 3600e3;
const REVIEW_TTL_MS = 45 * 60e3;
const HISTORY_MAX = 20000;
// Claude plan usage is asked of Anthropic every 5 minutes (see
// claude-usage.js); a reading this old says nothing about the windows now.
const CLAUDE_LIMITS_FRESH_MS = 45 * 60e3;
const UUID_TAIL = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl$/i;
const isTranscript = (name) => name.endsWith(".jsonl");

// ---------- helpers ----------

export function mcpServerOf(toolName) {
  const m = /^mcp__(.+?)__[^_].*$/.exec(toolName ?? "");
  if (!m) return null;
  // Plugin servers are named plugin_<plugin>_<server>.
  return m[1].replace(/^plugin_[^_]+_/, "");
}

export function categorize(name) {
  const n = String(name ?? "");
  if (mcpServerOf(n)) return "mcp";
  // local_shell_call: a Codex shell call as its rollout names it, with no
  // name of its own (stats lists it by its type).
  if (/^(Bash|PowerShell|shell|exec_command|local_shell|local_shell_call|unified_exec|write_stdin)$/i.test(n)) return "shell";
  if (/^(Edit|Write|MultiEdit|NotebookEdit|apply_patch)$/i.test(n)) return "edit";
  if (/^(Read|Glob|Grep|LS|view_image)$/i.test(n)) return "read";
  if (/^(WebFetch|WebSearch|web_search)$/i.test(n)) return "web";
  if (/^(Agent|Task|spawn_agent|collab)/i.test(n)) return "agent";
  if (/^exec$/i.test(n)) return "code";
  return "other";
}

// Text as one line of at most n characters.
export const clip = (s, n = 72) => {
  const t = String(s ?? "").replace(/\s+/g, " ").trim();
  return t.length > n ? t.slice(0, n - 1) + "…" : t;
};
const tail = (s, n = 280) => {
  const t = String(s ?? "").trim();
  return t.length > n ? "…" + t.slice(-(n - 1)) : t;
};
// The local day a time falls on, as YYYY-MM-DD. The tailer asks twice for
// every reply it reads, a hundred thousand times or so when it starts on a
// busy machine, and toLocaleDateString costs about a microsecond each, so
// the last day is kept with its bounds and serves the times inside it. Its
// bounds are kept only when the key at both ends is the day's own.
const toDay = (ts) => new Date(ts).toLocaleDateString("en-CA");
let lastDay = { key: "", from: 0, to: -Infinity };
export const dayKey = (ts) => {
  if (typeof ts === "number" && ts >= lastDay.from && ts < lastDay.to) return lastDay.key;
  const key = toDay(ts);
  if (typeof ts === "number" && Number.isFinite(ts)) {
    const d = new Date(ts);
    const from = new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
    const to = new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime();
    if (from <= ts && ts < to && toDay(from) === key && toDay(to - 1) === key) lastDay = { key, from, to };
  }
  return key;
};
const fileKey = (p) => String(p).replace(/^\\\\\?\\/, "").replace(/\\/g, "/").toLowerCase();

// Compare the whole input, including pagination and arguments after long
// prefixes. Keep a digest rather than another copy of private tool inputs.
const ordered = (v) => Array.isArray(v) ? v.map(ordered) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, ordered(v[k])])) : v;
const toolKey = (name, input) => `${name}|${createHash("sha256").update(JSON.stringify(ordered(input ?? null))).digest("hex")}`;

// The agent's last words before handing the turn back read like a question.
export function looksLikeQuestion(text) {
  const t = String(text ?? "").trim();
  if (!t) return false;
  const end = t.slice(-400);
  if (/\?["'`*_)\]]*\s*$/.test(end)) return true;
  const lastPara = end.split(/\n\s*\n/).pop();
  return /\b(want me to|should i|shall i|do you want|would you like|let me know|which (one|option|do you)|can you confirm|please confirm|your call)\b/i.test(lastPara);
}

function toolDetail(name, input, home) {
  if (!input || typeof input !== "object") return clip(tilde(typeof input === "string" ? input.split("\n")[0] : "", home));
  return clip(tilde(input.command ?? input.description ?? input.file_path ?? input.pattern ?? input.url ?? input.query ?? input.prompt ?? "", home));
}

// Codex records commands as argv, usually [shell.exe, "-Command", script];
// the script is the interesting part.
export function commandText(command) {
  const argv = [].concat(command ?? []).map(String);
  if (argv.length >= 2 && /(pwsh|powershell|bash|sh|cmd|zsh)(\.exe)?$/i.test(argv[0])) return argv[argv.length - 1];
  return argv.join(" ");
}

function filesOf(input) {
  if (!input || typeof input !== "object") return [];
  return [input.file_path, input.notebook_path, input.path].filter((p) => typeof p === "string" && /[\\/]/.test(p));
}

// The files an apply_patch adds, updates and deletes. Live leaves out the
// names it moves files to (`*** Move to:`).
function patchFiles(text) {
  return patchPaths(String(text ?? ""), { moves: false });
}

function diffLines(prefix, text, max = 40) {
  const lines = String(text ?? "").split(/\r?\n/);
  const out = lines.slice(0, max).map((l) => prefix + l);
  if (lines.length > max) out.push(`${prefix}… (${lines.length - max} more lines)`);
  return out.join("\n");
}

// Edits as small diffs, from either app's tool input.
function claudeEdits(name, input) {
  if (!input?.file_path && !input?.notebook_path) return [];
  const file = input.file_path ?? input.notebook_path;
  if (name === "Edit") return [{ path: file, kind: "edit", diff: `${diffLines("- ", input.old_string)}\n${diffLines("+ ", input.new_string)}` }];
  if (name === "MultiEdit")
    return [{ path: file, kind: "edit", diff: (input.edits ?? []).slice(0, 4).map((e) => `${diffLines("- ", e.old_string, 12)}\n${diffLines("+ ", e.new_string, 12)}`).join("\n···\n") }];
  if (name === "Write") return [{ path: file, kind: "write", diff: diffLines("+ ", input.content, 30) }];
  if (name === "NotebookEdit") return [{ path: file, kind: "edit", diff: diffLines("+ ", input.new_source, 20) }];
  return [];
}

function codexEdits(changes) {
  return Object.entries(changes ?? {}).map(([p, c]) => ({
    path: p,
    kind: c?.type ?? "update",
    diff: c?.unified_diff ? String(c.unified_diff).split("\n").slice(0, 60).join("\n") : c?.type === "add" ? diffLines("+ ", c.content, 30) : c?.type === "delete" ? "(file deleted)" : "",
  }));
}

function median(xs) {
  const s = [...xs].sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : 0;
}

// Least-squares slope of y over x, in y units per x unit.
function slope(points) {
  if (points.length < 2) return 0;
  const n = points.length;
  const mx = points.reduce((a, p) => a + p[0], 0) / n;
  const my = points.reduce((a, p) => a + p[1], 0) / n;
  let num = 0, den = 0;
  for (const [x, y] of points) {
    num += (x - mx) * (y - my);
    den += (x - mx) ** 2;
  }
  return den ? num / den : 0;
}

// ---------- per-session state ----------

class LiveSession {
  constructor(id, app) {
    this.id = id;
    this.app = app;
    this.tokens = { processed: 0, output: 0 };
    this.daily = new Map(); // YYYY-MM-DD → tokens processed
    // YYYY-MM-DD → what its calls would cost at API prices, their tokens,
    // and the models without a price, for Token use's estimate.
    this.spend = new Map();
    // Codex's model provider and whether it runs on OpenAI's priority tier,
    // which prices its calls.
    this.provider = null;
    this.priority = false;
    // The app's context window until a line says otherwise; any app but
    // Codex has Claude Code's.
    this.context = { used: 0, window: APP[tailedApp(app, "claude")].context };
    this.turnContexts = [];
    this.model = null;
    this.turn = { active: false, startedAt: null };
    this.inflight = new Map();
    this.toolCounts = {};
    this.lastTool = null;
    this.subagents = new Map();
    this.files = new Map();
    this.edits = new Map(); // fileKey → [{ts, path, kind, diff}]
    this.mcp = new Map();
    this.seenMessages = new Set();
    this.rate = []; // [epochSecond, tokens]
    this.lastEventAt = 0;
    this.prompts = [];
    this.firstPrompt = null;
    this.messages = [];
    this.lastPromptAt = 0;
    this.lastTurnEndAt = 0;
    // When you stopped its last turn (Claude Code's "[Request interrupted
    // by user]", Codex's turn_aborted), until its next prompt; 0 when you
    // didn't.
    this.interruptedAt = 0;
    // A Codex rollout's prompts are its completed user messages, or its
    // user_message events until it has one (see lines.js): whether it has,
    // and the prompt taken from an event last, with the turn mark then (a
    // count of turns started and ended).
    this.codexItems = false;
    this.eventPrompt = null;
    this.turnMarks = 0;
    this.question = null; // pending AskUserQuestion / plan approval
    this.hookNeed = null; // from Claude Code Notification hooks
    this.recentTools = [];
    this.errorStreak = 0;
  }

  addTokens(n, ts, live) {
    if (!(n > 0)) return;
    this.tokens.processed += n;
    const day = dayKey(ts);
    this.daily.set(day, (this.daily.get(day) ?? 0) + n);
    if (live) {
      const s = Math.floor(ts / 1000);
      const last = this.rate[this.rate.length - 1];
      if (last && last[0] === s) last[1] += n;
      else this.rate.push([s, n]);
      const cutoff = s - RATE_WINDOW_S;
      while (this.rate.length && this.rate[0][0] < cutoff) this.rate.shift();
    }
  }

  // One call at API prices on its day, as the stats panel prices it
  // (pricing.js, stats.js). Two days are kept.
  addCall(model, k, ts, opts) {
    const day = dayKey(ts);
    let d = this.spend.get(day);
    if (!d) {
      this.spend.set(day, (d = { cost: 0, tokens: 0, unpriced: new Set() }));
      while (this.spend.size > 2) this.spend.delete(this.spend.keys().next().value);
    }
    const parts = costParts(model, ts, k, opts);
    if (!parts) {
      d.unpriced.add(plainModel(model) || model || "unknown");
      return;
    }
    d.cost += parts.total;
    d.tokens += (k.input ?? 0) + (k.cacheRead ?? 0) + (k.cacheWrite5m ?? 0) + (k.cacheWrite1h ?? 0) + (k.output ?? 0);
  }

  addPrompt(text, ts) {
    this.resetStall();
    const p = { text: tail(text, 1200), ts };
    this.firstPrompt ??= p;
    this.prompts.push(p);
    if (this.prompts.length > 4) this.prompts.shift();
    this.lastPromptAt = ts;
    this.interruptedAt = 0;
    this.question = null;
    this.hookNeed = null;
  }

  addMessage(text, ts) {
    if (!text?.trim()) return;
    this.messages.push({ text: tail(text, 1500), ts });
    if (this.messages.length > 4) this.messages.shift();
  }

  // A turn's end; `interrupted` when you stopped it.
  endTurn(ts, { interrupted = false } = {}) {
    this.resetStall();
    this.turnMarks++;
    this.turn.active = false;
    this.inflight.clear();
    if (ts >= this.lastTurnEndAt) this.interruptedAt = interrupted ? ts : 0;
    this.lastTurnEndAt = Math.max(this.lastTurnEndAt, ts);
    if (this.context.used) {
      this.turnContexts.push(this.context.used);
      if (this.turnContexts.length > 12) this.turnContexts.shift();
    }
  }

  addEdits(edits, ts) {
    for (const e of edits) {
      const k = fileKey(e.path);
      const list = this.edits.get(k) ?? [];
      list.push({ ...e, ts });
      if (list.length > 3) list.shift();
      this.edits.delete(k);
      this.edits.set(k, list);
      if (this.edits.size > 40) this.edits.delete(this.edits.keys().next().value);
    }
  }

  resetStall() {
    this.recentTools = [];
    this.errorStreak = 0;
    this.errorSince = this.lastErrorAt = this.lastResultAt = 0;
  }

  noteToolResult(ok, ts, tool = null) {
    if (tool) {
      if (tool.ok !== null) return; // A duplicate result isn't another failure.
      tool.ok = !!ok;
    }
    if (ts < (this.lastResultAt ?? 0)) return;
    this.lastResultAt = ts;
    if (!ok && !this.errorStreak) this.errorSince = ts;
    this.errorStreak = ok ? 0 : this.errorStreak + 1;
    if (ok) this.errorSince = 0;
    if (!ok) this.lastErrorAt = ts;
  }

  tokensPerMinute(now) {
    const cutoff = Math.floor(now / 1000) - 60;
    return this.rate.reduce((sum, [s, n]) => (s >= cutoff ? sum + n : sum), 0);
  }

  status(now) {
    if (this.inflight.size && now - this.lastEventAt < TURN_STALE_MS) return "tool";
    if (this.turn.active && now - this.lastEventAt < TURN_STALE_MS) return "thinking";
    return "idle";
  }

  needsYou(now) {
    if (this.hookNeed && now - this.hookNeed.ts < 30 * 60e3) return { kind: "permission", text: this.hookNeed.text, since: this.hookNeed.ts };
    if (this.question && this.inflight.has(this.question.callId)) return { kind: "question", text: this.question.text, since: this.question.ts };
    if (!this.turn.active && this.lastTurnEndAt && this.lastTurnEndAt >= this.lastPromptAt) {
      // A turn you stopped waits on you, but you know: there's nothing in
      // it to review, and what it said last asks you nothing. The
      // snapshot's interruptedAt says so instead.
      if (this.interruptedAt) return null;
      const last = this.messages[this.messages.length - 1]?.text ?? "";
      const age = now - this.lastTurnEndAt;
      if (looksLikeQuestion(last) && age < QUESTION_TTL_MS) return { kind: "question", text: tail(last, 320), since: this.lastTurnEndAt };
      if (age < REVIEW_TTL_MS) return { kind: "review", text: tail(last, 320), since: this.lastTurnEndAt };
    }
    return null;
  }

  stuck(now) {
    if (!this.turn.active || (this.question && this.inflight.has(this.question.callId)) || (this.hookNeed && now - this.hookNeed.ts < 30 * 60e3)) return null;
    const reasons = [];
    // Repetition alone is not a stall: searches, pagination and polling
    // often repeat successfully. A loop needs repeated completed failures.
    let run = 0;
    const lastTool = this.recentTools[this.recentTools.length - 1];
    for (let i = this.recentTools.length - 1; lastTool && i >= 0; i--) {
      const t = this.recentTools[i];
      if (t.key !== lastTool.key || t.ok !== false || now - t.ts > 10 * 60e3) break;
      run++;
    }
    const failedLoop = run >= 3 && lastTool?.loopable && this.errorStreak >= 3;
    if (failedLoop) reasons.push(`ran ${lastTool.label} ${run} times in a row`);
    // A burst of failed parallel calls can be one recoverable attempt.
    // Different calls need sustained failure before we ask for attention.
    if (this.errorStreak >= 3 && now - this.lastErrorAt < 10 * 60e3 && (failedLoop || this.errorStreak >= 5 && this.lastErrorAt - this.errorSince >= 60e3)) reasons.push(`${this.errorStreak} failed tool calls in a row`);
    // Long builds and test runs are quiet by nature: a tool in flight gets 15
    // minutes before silence counts, plain thinking gets 4.
    const quiet = now - this.lastEventAt;
    const limit = this.inflight.size ? 15 * 60e3 : SILENT_MS;
    if (this.turn.active && quiet > limit && quiet < Math.max(TURN_STALE_MS, limit + 10 * 60e3)) reasons.push(`silent for ${Math.round(quiet / 60e3)} min`);
    return reasons.length ? { reasons } : null;
  }

  forecast() {
    const window = this.context.window || 1;
    const growth = median(this.turnContexts.slice(1).map((v, i) => v - this.turnContexts[i]).filter((d) => d > 0));
    const threshold = window * 0.9;
    const turnsLeft = growth > 0 ? Math.max(0, Math.floor((threshold - this.context.used) / growth)) : null;
    return { pct: this.context.used / window, growthPerTurn: growth || null, turnsLeft };
  }

  snapshot(now) {
    for (const [f, ts] of this.files) if (now - ts > FILE_MEMORY_MS) this.files.delete(f);
    const subs = [...this.subagents.values()]
      .map(({ announced, ...a }) => ({ ...a, active: !a.done && now - a.lastAt < SUBAGENT_IDLE_MS }))
      .sort((a, b) => b.lastAt - a.lastAt)
      .slice(0, 12);
    return {
      id: this.id,
      app: this.app,
      status: this.status(now),
      tokens: this.tokens,
      tokensToday: this.daily.get(dayKey(now)) ?? 0,
      tpm: this.tokensPerMinute(now),
      context: this.context,
      forecast: this.forecast(),
      model: this.model,
      turnStartedAt: this.turn.active ? this.turn.startedAt : null,
      lastTool: this.lastTool,
      toolCounts: this.toolCounts,
      subagents: subs,
      files: [...this.files.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10).map(([f, ts]) => ({ path: f, ts })),
      mcp: [...this.mcp.entries()].map(([server, ts]) => ({ server, ts })),
      needsYou: this.needsYou(now),
      stuck: this.stuck(now),
      lastMessage: this.messages.length ? tail(this.messages[this.messages.length - 1].text, 280) : null,
      lastTurnEndAt: this.lastTurnEndAt || null,
      interruptedAt: this.interruptedAt || null,
      lastEventAt: this.lastEventAt,
    };
  }
}

// ---------- the tailer ----------

// What happens to transcripts' lines. Reading the files (offsets, half
// lines, watch events, sweeps, walks for new files, history at start) is
// the tailer block's (core/blocks/tailer.js); this turns each line into
// state and events.
export class LiveTelemetry extends EventEmitter {
  constructor({ claudeRoot, codexRoot, claudeDesktopDir, claudeUsage = null, home = null }) {
    super();
    this.home = home;
    // Watch canonical long paths: on Windows, libuv aborts the process when a
    // recursive watch root is an 8.3 short name (C:\Users\ABCDEF~1\...).
    this.roots = [
      { dir: claudeRoot, app: "claude" },
      { dir: codexRoot, app: "codex" },
    ]
      .filter((r) => r.dir && fs.existsSync(r.dir))
      .map((r) => ({ ...r, dir: fs.realpathSync.native(r.dir) }));
    this.claudeDesktopDir = claudeDesktopDir;
    // The readings of Claude plan usage, as gatherers: Anthropic's, when
    // claudeUsage (() => fetchClaudeUsage()) is given, and Claude
    // desktop's samples, when its folder is there.
    this.usage = {
      anthropic: claudeUsage ? claudeUsageGatherer({ fetch: claudeUsage }) : null,
      desktop: claudeDesktopDir ? desktopUsageGatherer({ dir: claudeDesktopDir }) : null,
    };
    this.claudeReason = null;
    this.claudeSamples = { anthropic: [], desktop: [] };
    this.sessions = new Map();
    this.limits = { codex: null, claude: null };
    this.limitSamples = { codex: [], claude: [] };
    this.history = [];
    this.status = { watching: false, files: 0, lines: 0, errors: 0 };
    this.tailer = new Tailer({
      name: "transcripts",
      roots: this.roots,
      classify: (file) => this.classify(file),
      pick: isTranscript,
      onLine: (line, f, prime) => this.#line(line, f, prime),
      onNewFile: (file, f) => this.emit("new-file", { file, sessionId: f.sessionId }),
      // History is read file by file, so it's put in time order once, at
      // the end.
      onPrimed: () => {
        this.history.sort((a, b) => a.ts - b.ts);
        this.trimHistory();
      },
      primeWindowMs: PRIME_WINDOW_MS,
      sweepMs: SWEEP_MS,
      discoverEvery: DISCOVER_EVERY,
      status: this.status,
    });
  }

  // What the tailer keeps, by file.
  get files() {
    return this.tailer.files;
  }
  get dormant() {
    return this.tailer.dormant;
  }
  get folders() {
    return this.tailer.folders;
  }

  session(id, app) {
    let s = this.sessions.get(id);
    if (!s) this.sessions.set(id, (s = new LiveSession(id, app)));
    return s;
  }

  // ----- discovery -----

  classify(file) {
    const norm = file.replace(/\\/g, "/");
    if (!norm.endsWith(".jsonl")) return null;
    if (norm.includes("/.codex/") || this.roots.find((r) => r.app === "codex" && norm.startsWith(r.dir.replace(/\\/g, "/")))) {
      const m = UUID_TAIL.exec(norm);
      return m && /rollout-/.test(norm) ? { app: "codex", sessionId: `codex:${m[1]}` } : null;
    }
    const sub = /\/([0-9a-f-]{36})\/subagents\/agent-([^/]+)\.jsonl$/i.exec(norm);
    if (sub) return { app: "claude", sessionId: `claude:${sub[1]}`, agentId: sub[2] };
    const main = /\/([0-9a-f-]{36})\.jsonl$/i.exec(norm);
    return main ? { app: "claude", sessionId: `claude:${main[1]}` } : null;
  }

  // Reads the transcripts written lately as history, then follows them.
  async start() {
    await this.tailer.start();
    this.limitTimer = setInterval(() => this.readClaudeLimits(), 30000);
    await this.readClaudeLimits();
  }

  stop() {
    this.tailer.stop();
    clearInterval(this.limitTimer);
  }

  // A watch event for a transcript (see the tailer).
  schedule(file) {
    this.tailer.schedule(file);
  }

  // Catches up on what no watch event announced: new lines in the
  // transcripts being read, and with `discover`, transcripts that are new
  // since the last walk of the folders. One sweep at a time.
  sweep(opts) {
    return this.tailer.sweep(opts);
  }

  // Reads what's new in a transcript, after any read of it under way.
  track(file, opts) {
    return this.tailer.track(file, opts);
  }

  // A line the tailer read, through its app's normalizer (lines.js):
  // lines that aren't JSON are skipped, and one that breaks the normalizer
  // or a handler is counted as an error, so the lines after it are read.
  #line(line, f, prime) {
    try {
      const rec = linesOf(f.app).line(line, f);
      if (!rec) return;
      this.status.lines++;
      // Any app but Codex is read as Claude Code, as linesOf reads it.
      const codex = tailedApp(f.app, "claude") === "codex";
      // A provider's refusal, as its line marks it (core/refusals): kept
      // whether read as history or live, told to you only when live.
      const sig = codex ? codexSignal(rec.raw) : claudeSignal(rec.raw);
      if (sig) this.emit("refusal", { signal: sig, sessionId: f.sessionId, app: f.app, agentId: f.agentId ?? null, file: f.file ?? null, cwd: typeof rec.raw?.cwd === "string" ? rec.raw.cwd : null, model: this.sessions.get(f.sessionId)?.model ?? null, live: !prime });
      if (codex) this.applyCodex(f, rec, !prime);
      else this.applyClaude(f, rec, !prime);
    } catch {
      this.status.errors++;
    }
  }

  trimHistory() {
    const cutoff = Date.now() - PRIME_WINDOW_MS;
    let i = 0;
    while (i < this.history.length && this.history[i].ts < cutoff) i++;
    if (i) this.history.splice(0, i);
    if (this.history.length > HISTORY_MAX) this.history.splice(0, this.history.length - HISTORY_MAX);
  }

  activity(s, ev, live) {
    s.lastEventAt = Math.max(s.lastEventAt, ev.ts);
    const full = { sessionId: s.id, app: s.app, ...ev };
    if (ev.ts > Date.now() - PRIME_WINDOW_MS) {
      // The history is in time order, which historySince and trimHistory
      // rely on. At start it's sorted once, after reading everything; after
      // that an event that arrives late (from a transcript a sweep found,
      // say) goes where it belongs.
      if (live) this.history.splice(this.#index(ev.ts, { at: true }), 0, full);
      else this.history.push(full);
      if (live && this.history.length > HISTORY_MAX + 500) this.trimHistory();
    }
    if (live) this.emit("activity", full);
  }

  // How many events in the history come before `ts` (with `at`, before or
  // at it): where historySince starts, and where a late event goes.
  #index(ts, { at = false } = {}) {
    const before = (e) => e.ts < ts || (at && e.ts === ts);
    const h = this.history;
    if (!h.length || before(h[h.length - 1])) return h.length;
    let lo = 0, hi = h.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (before(h[mid])) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  noteTool(s, { name, detail, full, files = [], ts, agentId, server, extra, ok = null }, live) {
    const category = categorize(name);
    s.toolCounts[category] = (s.toolCounts[category] ?? 0) + 1;
    s.lastTool = { name, category, detail, ts };
    for (const file of files) s.files.set(file, ts);
    const srv = server ?? mcpServerOf(name);
    if (srv) s.mcp.set(srv, ts);
    const tool = !agentId ? { key: toolKey(name, full ?? detail), label: clip(detail || name, 48), loopable: category === "shell" || category === "mcp" || category === "web", ok, ts } : null;
    if (tool) s.recentTools.push(tool);
    if (s.recentTools.length > 20) s.recentTools.shift();
    this.activity(s, { kind: "tool", tool: name, category, detail, files, server: srv, agentId, ...extra, ts }, live);
    return tool;
  }

  // Wiki writes name the pages they touch; the UI beams to them.
  wikiPagesOf(server, input) {
    if (!isContextServer(server) || !input || typeof input !== "object") return null;
    const pages = [].concat(input.pages ?? [], input.slug ?? []).filter((p) => typeof p === "string");
    return pages.length ? pages.slice(0, 6) : null;
  }

  // Claude Code Notification hooks carry permission prompts the transcript doesn't.
  noteHook(sessionId, body) {
    const s = this.session(sessionId, "claude");
    const ts = Date.now();
    if (body.hook_event_name === "Notification") s.hookNeed = { text: clip(body.message ?? "Claude needs your attention", 200), ts };
    else if (["UserPromptSubmit", "Stop", "SessionEnd", "PostToolUse"].includes(body.hook_event_name)) s.hookNeed = null;
    this.activity(s, { kind: "hook", detail: body.hook_event_name ?? "hook", ts }, true);
  }

  // ----- Claude Code transcript lines -----

  // A Claude Code transcript's line: normalized (see lines.js), or as the
  // transcript has it.
  applyClaude(f, line, live) {
    const o = line instanceof ClaudeLine ? line : new ClaudeLine(line);
    const s = this.session(f.sessionId, "claude");
    // A line that doesn't say when it was written (see lines.js) was
    // written when it's read.
    const ts = o.ts || Date.now();
    const agentId = f.agentId ?? null;
    let sub = null;
    if (agentId) {
      sub = s.subagents.get(agentId);
      if (!sub) {
        sub = { id: agentId, label: "subagent", startedAt: ts, lastAt: ts, tools: 0, done: false, announced: false };
        s.subagents.set(agentId, sub);
      }
      sub.lastAt = Math.max(sub.lastAt, ts);
    }
    this.applyClaudeLine(s, sub, agentId, o, ts, live);
    // Announce a subagent once its first line has given it a label.
    if (sub && !sub.announced) {
      sub.announced = true;
      this.activity(s, { kind: "subagent-start", agentId, label: sub.label, ts }, live);
    }
  }

  applyClaudeLine(s, sub, agentId, o, ts, live) {
    const type = o.type;
    if (type === "assistant") this.#claudeAssistant(s, sub, agentId, o, ts, live);
    else if (type === "user") this.#claudeUser(s, sub, agentId, o, ts, live);
  }

  // A reply: its tokens and context fill, its tool calls and text, and the
  // end of a turn (or of a subagent's work).
  #claudeAssistant(s, sub, agentId, o, ts, live) {
    const model = o.model;
    if (model && !agentId) s.model = model;
    // Each of a message's lines repeats its usage: counted once.
    const id = o.messageId;
    const u = id && !s.seenMessages.has(id) ? o.usage : null;
    if (u) {
      s.seenMessages.add(id);
      if (s.seenMessages.size > 400) s.seenMessages.delete(s.seenMessages.values().next().value);
      const ctx = u.input + u.cacheRead + u.cacheWrite;
      s.tokens.output += u.output;
      s.addTokens(ctx + u.output, ts, live);
      s.addCall(model ?? s.model ?? "unknown", { input: u.input, cacheRead: u.cacheRead, cacheWrite5m: u.cacheWrite5m, cacheWrite1h: u.cacheWrite1h, output: u.output }, ts, { fast: u.fast });
      if (!agentId) {
        s.context.used = ctx + u.output;
        if (s.context.used > 200000 || /\[1m\]/i.test(model ?? "")) s.context.window = 1000000;
      }
      if (live) this.emit("tokens", { sessionId: s.id, app: "claude", n: ctx + u.output, ts });
    }
    for (const b of o.blocks) {
      if (b.type === "tool_use") this.#claudeToolUse(s, sub, agentId, b, ts, live);
      else if (b.type === "text" && b.text?.trim() && !agentId) {
        s.addMessage(b.text, ts);
        this.activity(s, { kind: "message", detail: clip(b.text), ts }, live);
      }
    }
    // A refusal ends the turn too: the provider said no, and nothing more
    // comes until you write again.
    const refused = o.raw?.message?.stop_reason === "refusal";
    if (!o.endTurn && !refused) return;
    if (refused && !agentId) this.activity(s, { kind: "refusal", detail: clip(o.raw.message.stop_details?.explanation ?? o.raw.message.stop_details?.category ?? "the provider refused", 160), ts }, live);
    if (sub) {
      sub.done = true;
      this.activity(s, { kind: "subagent-stop", agentId, label: sub.label, ts }, live);
    } else if (s.turn.active) {
      // A turn that began with a line read as written when it was read
      // (see lines.js) can seem to end before it began: how long it took
      // isn't known then.
      const duration = s.turn.startedAt && ts >= s.turn.startedAt ? ts - s.turn.startedAt : null;
      s.endTurn(ts);
      this.activity(s, { kind: "turn-end", duration, ts }, live);
      const need = s.needsYou(Date.now());
      if (need && need.kind === "question") this.activity(s, { kind: "needs-you", detail: clip(need.text, 120), ts }, live);
    }
  }

  #claudeToolUse(s, sub, agentId, b, ts, live) {
    s.inflight.set(b.id, { name: b.name, ts });
    if (sub) sub.tools++;
    const server = mcpServerOf(b.name);
    const pages = this.wikiPagesOf(server, b.input);
    const full = typeof b.input?.command === "string" ? b.input.command : b.input ?? {};
    const tool = this.noteTool(s, { name: b.name, detail: toolDetail(b.name, b.input, this.home), full, files: filesOf(b.input), ts, agentId, extra: pages ? { pages } : undefined }, live);
    s.inflight.get(b.id).tool = tool;
    const edits = claudeEdits(b.name, b.input);
    if (edits.length) s.addEdits(edits, ts);
    if (categorize(b.name) === "agent" && b.input?.description) this.pendingLabel = { session: s.id, label: clip(b.input.description, 40), ts };
    if (!agentId && (b.name === "AskUserQuestion" || b.name === "ExitPlanMode")) {
      const q = b.name === "ExitPlanMode" ? "Plan ready for your approval" : b.input?.questions?.[0]?.question ?? b.input?.question ?? "Claude asked a question";
      s.question = { callId: b.id, text: clip(q, 300), ts };
      this.activity(s, { kind: "needs-you", detail: clip(q, 120), ts }, live);
    }
  }

  // Tool results, a prompt (in a subagent's file, its task), or your
  // interruption.
  #claudeUser(s, sub, agentId, o, ts, live) {
    const results = o.results;
    if (results) {
      for (const r of results) {
        const tool = s.inflight.get(r.id)?.tool;
        s.inflight.delete(r.id);
        if (!agentId) s.noteToolResult(!r.error, ts, tool);
        if (r.error) this.activity(s, { kind: "tool-error", ts, agentId }, live);
      }
      s.lastEventAt = Math.max(s.lastEventAt, ts);
    }
    if (o.meta || !!o.sidechain !== !!agentId) return;
    // A compacted summary and an interruption aren't prompts (see lines.js);
    // an interruption ends the turn you stopped.
    if (o.interruption) return void (agentId || this.#interrupted(s, ts, live));
    if (results || o.compactSummary) return;
    const text = o.text;
    if (!text.trim() || text.startsWith("<")) return;
    if (sub) {
      // A subagent's first prompt is its task; the Agent tool's description is a better label.
      if (sub.label === "subagent") {
        const pl = this.pendingLabel;
        sub.label = pl && pl.session === s.id && Math.abs(ts - pl.ts) < 60000 ? pl.label : clip(text, 40);
      }
      return;
    }
    s.turn = { active: true, startedAt: ts };
    s.addPrompt(text, ts);
    this.activity(s, { kind: "prompt", detail: clip(text, 90), ts }, live);
  }

  // You stopped the turn under way (in Claude Code, Esc or a tool use you
  // turned down; in Codex, Esc or Ctrl-C): the agent waits for you, so the
  // turn ends, and says it was stopped.
  #interrupted(s, ts, live) {
    if (!s.turn.active) return;
    const duration = s.turn.startedAt && ts >= s.turn.startedAt ? ts - s.turn.startedAt : null;
    s.endTurn(ts, { interrupted: true });
    this.activity(s, { kind: "turn-end", duration, interrupted: true, ts }, live);
  }

  // ----- Codex rollout lines -----

  // A Codex rollout's line: normalized (see lines.js), or as the rollout
  // has it.
  applyCodex(f, line, live) {
    const o = line instanceof CodexLine ? line : new CodexLine(line);
    const s = this.session(f.sessionId, "codex");
    // A line that doesn't say when it was written (see lines.js) was
    // written when it's read.
    const ts = o.ts || Date.now();
    const p = o.payload;
    const type = o.type;
    if (type === "event_msg") {
      switch (p.type) {
        case "task_started":
          s.resetStall();
          s.turnMarks++;
          s.turn = { active: true, startedAt: ts };
          if (p.model_context_window) s.context.window = p.model_context_window;
          this.activity(s, { kind: "turn-start", ts }, live);
          return;
        case "task_complete": {
          if (p.last_agent_message) s.addMessage(p.last_agent_message, ts);
          s.endTurn(ts);
          this.activity(s, { kind: "turn-end", duration: p.duration_ms ?? null, ts }, live);
          const need = s.needsYou(Date.now());
          if (need && need.kind === "question") this.activity(s, { kind: "needs-you", detail: clip(need.text, 120), ts }, live);
          return;
        }
        case "token_count": {
          const info = p.info;
          if (info?.total_token_usage) {
            const total = info.total_token_usage.total_tokens ?? 0;
            const delta = total - s.tokens.processed;
            if (delta > 0) {
              s.addTokens(delta, ts, live);
              // The call's own tokens, as stats.js reads them: cached input
              // and cache writes are part of input_tokens.
              const last = info.last_token_usage ?? {};
              const input = last.input_tokens ?? Math.max(0, (last.total_tokens ?? 0) - (last.output_tokens ?? 0));
              const cached = last.cached_input_tokens ?? 0;
              const write = last.cache_write_input_tokens ?? 0;
              const model = s.model ?? "unknown";
              s.addCall(s.provider && s.provider !== "openai" ? `${s.provider}/${model}` : model, { input: Math.max(0, input - cached - write), cacheRead: cached, cacheWrite5m: write, cacheWrite1h: 0, output: last.output_tokens ?? 0 }, ts, { priority: s.priority });
              if (live) this.emit("tokens", { sessionId: s.id, app: "codex", n: delta, ts });
            }
            s.tokens.output = (info.total_token_usage.output_tokens ?? 0) + (info.total_token_usage.reasoning_output_tokens ?? 0);
          }
          if (info?.last_token_usage) s.context.used = info.last_token_usage.total_tokens ?? s.context.used;
          if (info?.model_context_window) s.context.window = info.model_context_window;
          if (p.rate_limits) this.noteCodexLimits(p.rate_limits, ts);
          s.lastEventAt = Math.max(s.lastEventAt, ts);
          return;
        }
        case "item_completed":
          return this.codexItem(s, p.item ?? {}, ts, live);
        case "user_message":
          return this.#codexUserEvent(s, o.userEvent, ts, live);
        // You stopped the turn (Esc, or Ctrl-C): Codex writes no
        // task_complete for it. A turn the next one replaced, or a review
        // ending, isn't you stopping it; older rollouts give no reason.
        case "turn_aborted":
          if (p.reason && p.reason !== "interrupted") return;
          return this.#interrupted(s, ts, live);
        // A policy refusal: the turn ends with it.
        case "error": {
          const sig = codexSignal(o.raw);
          if (!sig || sig.kind !== "refusal") return;
          this.activity(s, { kind: "refusal", detail: clip(sig.explanation ?? sig.text ?? sig.category, 160), ts }, live);
          if (s.turn.active) s.endTurn(ts);
          return;
        }
        // All of the thread's settings: a tier not said is the default.
        case "thread_settings_applied":
          s.priority = priorityTier(p.thread_settings?.service_tier);
          return;
        default:
          return;
      }
    }
    if (type === "turn_context" && p.model) s.model = p.model;
    // What prices its calls (stats.js reads them the same way): the
    // provider, and the tier, said with a turn in older rollouts and in the
    // thread's settings in newer ones.
    if (type === "session_meta") s.provider = p.model_provider ?? s.provider;
    if (type === "turn_context" && p.service_tier !== undefined) s.priority = priorityTier(p.service_tier);
    if (type === "response_item") {
      if (p.type === "function_call" || p.type === "custom_tool_call" || p.type === "local_shell_call") {
        s.inflight.set(p.call_id ?? p.id, { name: p.name, ts });
        s.lastEventAt = Math.max(s.lastEventAt, ts);
        if (p.name === "apply_patch") this.noteTool(s, { name: "apply_patch", detail: "", files: patchFiles(p.input ?? p.arguments), ts }, live);
      } else if (p.type === "function_call_output" || p.type === "custom_tool_call_output" || p.type === "local_shell_call_output") {
        // Its output ends a call, a shell call's too, matched as stats
        // match them: by call_id, else id.
        s.inflight.delete(p.call_id ?? p.id);
        s.lastEventAt = Math.max(s.lastEventAt, ts);
      }
    }
  }

  // A user_message event: a prompt while the rollout has no completed user
  // message (`codex exec` writes none, nor did Codex before 0.130), passed
  // by once it has. The live tailer can't know a line still to come, so
  // in a rollout whose first completed user message comes turns later
  // (an old one picked up again), it keeps those turns' prompts, which
  // stats and search, reading the whole file, leave out.
  #codexUserEvent(s, text, ts, live) {
    if (s.codexItems || !isCodexPrompt(text)) return;
    s.addPrompt(text, ts);
    s.eventPrompt = { prompt: s.prompts[s.prompts.length - 1], marks: s.turnMarks };
    this.activity(s, { kind: "prompt", detail: clip(text, 90), ts }, live);
  }

  // Codex's rate limits in XenoNet's own shape (live/snapshot's limits.codex):
  // each window's use in percent, its length and when it resets, in ms.
  // Codex says when in seconds, and sends more that XenoNet doesn't show.
  noteCodexLimits(rl, ts) {
    const window = (w) =>
      w && typeof w === "object"
        ? { pct: typeof w.used_percent === "number" ? w.used_percent : null, windowMinutes: typeof w.window_minutes === "number" ? w.window_minutes : null, resetsAt: typeof w.resets_at === "number" && w.resets_at > 0 ? w.resets_at * 1000 : null }
        : null;
    const [primary, secondary] = [window(rl.primary), window(rl.secondary)];
    this.limits.codex = { primary, secondary, observedAt: ts };
    const sample = { ts, primary: primary?.pct ?? null, secondary: secondary?.pct ?? null, resets: primary?.resetsAt ?? null };
    const last = this.limitSamples.codex[this.limitSamples.codex.length - 1];
    if (last && last.primary === sample.primary && last.secondary === sample.secondary && ts - last.ts < 10 * 60e3) return;
    this.limitSamples.codex.push(sample);
    if (this.limitSamples.codex.length > 2000) this.limitSamples.codex.shift();
  }

  codexItem(s, it, ts, live) {
    switch (it.type) {
      case "UserMessage": {
        const text = (it.content ?? []).map((c) => c?.text ?? "").join(" ");
        // The rollout's first: from here on its prompts are these. Codex
        // may have written it as a user_message event just before, in the
        // same turn, which was taken for this prompt.
        const event = !s.codexItems && s.eventPrompt;
        s.codexItems = true;
        s.eventPrompt = null;
        if (event && event.prompt === s.prompts[s.prompts.length - 1] && event.marks === s.turnMarks) return;
        // A blank one (an image alone), or one that starts with "<", isn't
        // a prompt, as in stats and search.
        if (!isCodexPrompt(text)) return;
        s.addPrompt(text, ts);
        this.activity(s, { kind: "prompt", detail: clip(text, 90), ts }, live);
        return;
      }
      case "AgentMessage": {
        const text = (it.content ?? []).map((c) => c.text ?? "").join(" ");
        if (text.trim()) {
          s.addMessage(text, ts);
          this.activity(s, { kind: "message", detail: clip(text), ts }, live);
        }
        return;
      }
      case "CommandExecution": {
        const ok = it.exit_code == null || Number(it.exit_code) === 0;
        s.noteToolResult(ok && it.status !== "failed", ts);
        const cmd = commandText(it.command);
        this.noteTool(s, { name: "shell", detail: clip(tilde(cmd, this.home)), full: cmd, ok: ok && it.status !== "failed", ts }, live);
        if (!ok) this.activity(s, { kind: "tool-error", ts }, live);
        return;
      }
      case "McpToolCall": {
        const ok = it.status === "completed" && !it.error;
        s.noteToolResult(ok, ts);
        const pages = this.wikiPagesOf(it.server, it.arguments);
        this.noteTool(s, { name: `mcp__${it.server}__${it.tool}`, server: it.server, detail: `${it.server}.${it.tool}`, full: it.arguments ?? {}, ok, ts, extra: pages ? { pages } : undefined }, live);
        if (!ok) this.activity(s, { kind: "tool-error", ts }, live);
        return;
      }
      case "FileChange": {
        const files = Object.keys(it.changes ?? {});
        s.addEdits(codexEdits(it.changes), ts);
        s.noteToolResult(it.status !== "failed", ts);
        this.noteTool(s, { name: "apply_patch", detail: files.map((f) => baseName(f)).join(", "), files, ok: it.status !== "failed", ts }, live);
        return;
      }
      case "WebSearch":
        // An empty query is no query: the action's says what was searched,
        // as a thread shown from its rollout reads it.
        return this.noteTool(s, { name: "web_search", detail: clip(it.query || it.action?.query || ""), ts }, live);
      case "ImageView":
        return this.noteTool(s, { name: "view_image", detail: baseName(String(it.path ?? "")), ts }, live);
      case "CollabAgentToolCall":
      case "SubAgentActivity": {
        // Rollouts name these fields in snake_case (the app-server's API
        // says them in camelCase, which is read too).
        const id = it.agent_thread_id ?? it.agentThreadId ?? it.receiver_thread_ids?.[0] ?? it.receiverThreadIds?.[0] ?? it.id;
        if (!s.subagents.has(id)) {
          s.subagents.set(id, { id, label: clip(it.prompt ?? it.agent_path ?? it.agentPath ?? "subagent", 40), startedAt: ts, lastAt: ts, tools: 0, done: false, announced: true });
          this.activity(s, { kind: "subagent-start", agentId: id, label: s.subagents.get(id).label, ts }, live);
        }
        const sub = s.subagents.get(id);
        sub.lastAt = ts;
        // A subagent's activity says what happened to it: started,
        // interacted (given more to do), interrupted or completed. It's
        // done once it completes or is stopped, until it's given more.
        if (it.type !== "SubAgentActivity") return;
        if (it.kind === "completed" || it.kind === "interrupted") {
          if (!sub.done) this.activity(s, { kind: "subagent-stop", agentId: id, label: sub.label, ts }, live);
          sub.done = true;
        } else if (it.kind === "started" || it.kind === "interacted") sub.done = false;
        return;
      }
      default:
        return;
    }
  }

  // Claude plan usage: Anthropic's answer every 5 minutes (sooner never,
  // later when it asks for a pause), and Claude desktop's own samples (see
  // the gatherers in claude-usage.js). The newer reading wins; both feed
  // the forecast.
  async readClaudeLimits(now = Date.now()) {
    const anthropic = this.usage.anthropic;
    if (anthropic) {
      const before = anthropic.last;
      const r = await anthropic.run({ now });
      // Asked this time.
      if (anthropic.last !== before) {
        if (r.usage) {
          this.claudeReason = null;
          const u = r.usage;
          this.claudeSamples.anthropic = [...this.claudeSamples.anthropic.filter((x) => now - x.ts < 8 * 86400e3), { ts: u.observedAt, fiveHour: u.fiveHour, sevenDay: u.sevenDay, resets: u.resets, source: "anthropic" }];
        } else this.claudeReason = r.reason ?? null;
      }
    }
    if (this.usage.desktop) {
      const samples = await this.usage.desktop.run({ now });
      if (samples) this.claudeSamples.desktop = samples;
    }
    const samples = [...this.claudeSamples.anthropic, ...this.claudeSamples.desktop].sort((a, b) => a.ts - b.ts);
    this.limitSamples.claude = samples;
    const last = samples[samples.length - 1];
    if (last) this.limits.claude = { fiveHour: last.fiveHour, sevenDay: last.sevenDay, resets: last.resets ?? { fiveHour: null, sevenDay: null }, observedAt: last.ts, source: last.source };
  }

  // An old Claude reading says nothing about the windows now, so it's
  // reported as missing, with when it was taken and why there's no newer one.
  claudeLimits(now = Date.now()) {
    const c = this.limits.claude;
    if (c && now - c.observedAt < CLAUDE_LIMITS_FRESH_MS) return c;
    if (!c && !this.claudeReason) return null;
    return { fiveHour: null, sevenDay: null, observedAt: c?.observedAt ?? null, stale: true, reason: this.claudeReason };
  }

  // Straight-line projection of usage over the recent samples.
  limitForecast(now = Date.now()) {
    // `horizon` stands in for an unknown reset time: the window's length.
    const project = (points, pct, resetsAt, horizon) => {
      const perHour = slope(points.map(([t, v]) => [t / 3600e3, v]));
      if (pct == null) return null;
      const eta = perHour > 0.05 ? now + ((100 - pct) / perHour) * 3600e3 : null;
      const limit = resetsAt ?? now + horizon;
      return { pct, perHour, eta, resetsAt, hitsBeforeReset: eta != null && eta < limit };
    };
    const out = {};
    const cx = this.limits.codex;
    if (cx?.primary) {
      const { pct, windowMinutes, resetsAt } = cx.primary;
      const pts = this.limitSamples.codex.filter((x) => x.primary != null && x.resets === resetsAt && now - x.ts < 24 * 3600e3).map((x) => [x.ts, x.primary]);
      out.codex = { ...project(pts, pct, resetsAt, (windowMinutes ?? 300) * 60e3), windowMinutes };
    }
    const c = this.claudeLimits(now);
    if (c && !c.stale) {
      // Only samples from the current window, when its reset is known.
      const since = (resetsAt, span, lookback) => Math.max(now - lookback, resetsAt ? resetsAt - span : 0);
      const points = (key, from) => this.limitSamples.claude.filter((x) => x[key] != null && x.ts >= from).map((x) => [x.ts, x[key]]);
      out.claudeFiveHour = project(points("fiveHour", since(c.resets?.fiveHour, 5 * 3600e3, 3 * 3600e3)), c.fiveHour, c.resets?.fiveHour ?? null, 5 * 3600e3);
      out.claudeSevenDay = project(points("sevenDay", since(c.resets?.sevenDay, 7 * 86400e3, 48 * 3600e3)), c.sevenDay, c.resets?.sevenDay ?? null, 7 * 86400e3);
    }
    return out;
  }

  // ----- outputs -----

  // Sessions that touched (read or edited) the same file recently.
  collab(snapshots) {
    const byFile = new Map();
    for (const s of snapshots)
      for (const { path: f } of s.files) {
        const key = fileKey(f);
        if (!byFile.has(key)) byFile.set(key, new Set());
        byFile.get(key).add(s.id);
      }
    const edges = new Map();
    for (const [file, ids] of byFile) {
      if (ids.size < 2) continue;
      const list = [...ids];
      for (let i = 0; i < list.length; i++)
        for (let j = i + 1; j < list.length; j++) {
          const k = [list[i], list[j]].sort().join("|");
          if (!edges.has(k)) edges.set(k, { a: list[i], b: list[j], kind: "file", files: [] });
          edges.get(k).files.push(baseName(file));
        }
    }
    return [...edges.values()];
  }

  // Sessions that both edited the same file within COLLISION_WINDOW_MS.
  collisions(now = Date.now()) {
    const byFile = new Map();
    for (const s of this.sessions.values())
      for (const [k, list] of s.edits) {
        const last = list[list.length - 1];
        if (now - last.ts > COLLISION_WINDOW_MS) continue;
        if (!byFile.has(k)) byFile.set(k, []);
        byFile.get(k).push({ id: s.id, ts: last.ts, path: last.path });
      }
    const out = new Map();
    for (const [k, list] of byFile) {
      if (list.length < 2) continue;
      for (let i = 0; i < list.length; i++)
        for (let j = i + 1; j < list.length; j++) {
          const [a, b] = [list[i], list[j]].sort((x, y) => (x.id < y.id ? -1 : 1));
          const key = `${a.id}|${b.id}`;
          if (!out.has(key)) out.set(key, { a: a.id, b: b.id, files: [], paths: [], ts: 0 });
          const c = out.get(key);
          c.files.push(baseName(a.path));
          c.paths.push(k);
          c.ts = Math.max(c.ts, a.ts, b.ts);
        }
    }
    return [...out.values()];
  }

  collisionDiffs(a, b) {
    const sa = this.sessions.get(a), sb = this.sessions.get(b);
    if (!sa || !sb) return [];
    const shared = [...sa.edits.keys()].filter((k) => sb.edits.has(k));
    return shared.map((k) => ({ file: k, a: sa.edits.get(k), b: sb.edits.get(k) }));
  }

  // Context for a handoff brief, from what the tailer has seen.
  sessionNotes(id) {
    const s = this.sessions.get(id);
    if (!s) return null;
    return { firstPrompt: s.firstPrompt, prompts: s.prompts, messages: s.messages, files: [...s.files.keys()], edits: [...s.edits.values()].flat().map((e) => e.path), context: s.context, needsYou: s.needsYou(Date.now()) };
  }

  historySince(since) {
    return this.history.slice(this.#index(since));
  }

  // What today's calls would cost at API prices (pricing.js), by app, and
  // at this minute's pace: each app's tokens a minute at what its tokens
  // cost on average today. Models without a price are named, not guessed.
  costs(now, sessions) {
    const day = dayKey(now);
    const byApp = { claude: 0, codex: 0 };
    const tokens = { claude: 0, codex: 0 };
    const unpriced = new Set();
    for (const s of this.sessions.values()) {
      const d = s.spend.get(day);
      if (!d) continue;
      byApp[s.app] += d.cost;
      tokens[s.app] += d.tokens;
      for (const m of d.unpriced) unpriced.add(m);
    }
    let perHour = 0;
    for (const app of ["claude", "codex"]) {
      const tpm = sessions.filter((s) => s.app === app).reduce((a, s) => a + s.tpm, 0);
      if (tokens[app] > 0) perHour += tpm * 60 * (byApp[app] / tokens[app]);
    }
    return { today: byApp.claude + byApp.codex, byApp, perHour, unpriced: [...unpriced].sort(), checked: PRICES_CHECKED };
  }

  // What each session's calls today would cost at API prices, the same
  // sums costs() adds up: [{ id, app, cost, tokens, unpriced }] for each
  // session with a call today, for what each actor's work costs
  // (core/actors/spend.js).
  sessionCosts(now) {
    const day = dayKey(now);
    const out = [];
    for (const s of this.sessions.values()) {
      const d = s.spend.get(day);
      if (d) out.push({ id: s.id, app: s.app, cost: d.cost, tokens: d.tokens, unpriced: [...d.unpriced].sort() });
    }
    return out;
  }

  snapshot() {
    const now = Date.now();
    const sessions = [...this.sessions.values()].filter((s) => now - s.lastEventAt < PRIME_WINDOW_MS).map((s) => s.snapshot(now));
    const series = { claude: new Array(RATE_WINDOW_S).fill(0), codex: new Array(RATE_WINDOW_S).fill(0) };
    const nowS = Math.floor(now / 1000);
    for (const s of this.sessions.values())
      for (const [sec, n] of s.rate) {
        const i = RATE_WINDOW_S - 1 - (nowS - sec);
        if (i >= 0 && i < RATE_WINDOW_S) series[s.app][i] += n;
      }
    const resources = new Map();
    for (const s of sessions) for (const { server, ts } of s.mcp) resources.set(server, Math.max(resources.get(server) ?? 0, ts));
    return {
      at: now,
      sessions,
      series,
      tpm: sessions.reduce((a, s) => a + s.tpm, 0),
      total24h: sessions.reduce((a, s) => a + s.tokens.processed, 0),
      today: sessions.reduce((a, s) => a + s.tokensToday, 0),
      limits: { ...this.limits, claude: this.claudeLimits(now) },
      limitForecast: this.limitForecast(now),
      costs: this.costs(now, sessions),
      collab: this.collab(sessions),
      collisions: this.collisions(now),
      resources: [...resources.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8).map(([server, ts]) => ({ server, ts })),
      status: this.status,
    };
  }
}
