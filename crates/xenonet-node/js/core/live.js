// core/live.js in Rust (crates/xenonet-sources/src/live/): real-time
// telemetry from the transcripts both apps append to as they work. Rust
// holds each session's state and makes every decision: what a line means
// (tool calls, prompts, turns, subagents, tokens, costs, context fill,
// edits, refusals), the signals the UI acts on (waiting for you, stuck,
// edit collisions, forecasts), the history and the snapshot. Each line
// goes in with the time, and the events it makes come back in order, for
// this EventEmitter to emit.
//
// What stays here: the tailer (its own twin), the gatherers of Claude plan
// usage and their timers, and a face for what JS reaches into: `sessions`
// is a Map of views onto each session's state in Rust, whose fields and
// Maps read and write through, and `limits` and `limitSamples` set Rust's.
import fs from "node:fs";
import { EventEmitter } from "node:events";
import { native } from "../index.js";
import { Tailer } from "./blocks/tailer.js";
import { claudeUsageGatherer, desktopUsageGatherer } from "./claude-usage.js";
import { ClaudeLine, CodexLine, linesOf } from "./lines.js";
import { tailedApp } from "../../../../core/apps.js";

const PRIME_WINDOW_MS = 24 * 3600e3;
const SWEEP_MS = 2000;
const DISCOVER_EVERY = 15; // sweeps: every 30 seconds
const isTranscript = (name) => name.endsWith(".jsonl");

// ---------- helpers ----------

export const mcpServerOf = (toolName) => native.liveMcpServerOf(String(toolName ?? "")) ?? null;

export const categorize = (name) => native.liveCategorize(String(name ?? ""));

// Text as one line of at most n characters.
export const clip = (s, n = 72) => native.liveClip(String(s ?? ""), Number(n));

// The local day a time falls on, as YYYY-MM-DD (what toLocaleDateString
// says, "Invalid Date" for a time that isn't one).
export const dayKey = (ts) => native.liveDayKey(typeof ts === "number" ? ts : new Date(ts).getTime());

// The agent's last words before handing the turn back read like a question.
export const looksLikeQuestion = (text) => native.liveLooksLikeQuestion(String(text ?? ""));

// Codex records commands as argv, usually [shell.exe, "-Command", script];
// the script is the interesting part.
export const commandText = (command) => native.liveCommandText(command);

// ---------- the faces of a session's state ----------

// One of a session's Maps, read and written in Rust.
class MapView {
  #core;
  #id;
  #map;
  constructor(core, id, map) {
    this.#core = core;
    this.#id = id;
    this.#map = map;
  }
  #op(op, key, value) {
    return this.#core.sessionMap(this.#id, this.#map, op, key, value);
  }
  get size() {
    return this.#op("size");
  }
  get(key) {
    return this.#op("get", key);
  }
  has(key) {
    return this.#op("has", key);
  }
  set(key, value) {
    this.#op("set", key, value);
    return this;
  }
  delete(key) {
    return this.#op("delete", key);
  }
  clear() {
    this.#op("clear");
  }
  entries() {
    return (this.#op("entries") ?? [])[Symbol.iterator]();
  }
  keys() {
    return (this.#op("entries") ?? []).map(([k]) => k)[Symbol.iterator]();
  }
  values() {
    return (this.#op("entries") ?? []).map(([, v]) => v)[Symbol.iterator]();
  }
  forEach(fn, self) {
    for (const [k, v] of this.entries()) fn.call(self, v, k, this);
  }
  [Symbol.iterator]() {
    return this.entries();
  }
}

// The messages a session has counted, a Set.
class SetView {
  #map;
  constructor(core, id, map) {
    this.#map = new MapView(core, id, map);
  }
  get size() {
    return this.#map.size;
  }
  has(v) {
    return this.#map.has(v);
  }
  add(v) {
    this.#map.set(v, true);
    return this;
  }
  delete(v) {
    return this.#map.delete(v);
  }
  clear() {
    this.#map.clear();
  }
  values() {
    return this.#map.keys();
  }
  keys() {
    return this.#map.keys();
  }
  entries() {
    return [...this.#map.keys()].map((k) => [k, k])[Symbol.iterator]();
  }
  forEach(fn, self) {
    for (const k of this.#map.keys()) fn.call(self, k, k, this);
  }
  [Symbol.iterator]() {
    return this.#map.keys();
  }
}

// Fields read and set whole: a value read is a copy, so a change to it
// reaches Rust only when it's set again. The objects live.js's snapshot and
// notes hand out are the session's own, which JS that holds them sees
// change: those are kept here as copies Rust's changes are written into
// (MIRRORED).
const FIELDS = ["app", "provider", "priority", "turnContexts", "model", "turn", "lastTool", "rate", "lastEventAt", "firstPrompt", "lastPromptAt", "lastTurnEndAt", "interruptedAt", "codexItems", "eventPrompt", "turnMarks", "question", "hookNeed", "errorStreak", "errorSince", "lastErrorAt", "lastResultAt"];
const MIRRORED = ["tokens", "context", "toolCounts", "prompts", "messages"];
const MAPS = ["daily", "spend", "inflight", "subagents", "files", "edits", "mcp"];

// An object's fields made `from`'s, in place.
function fill(o, from) {
  for (const k of Object.keys(o)) if (!Object.hasOwn(from, k)) delete o[k];
  return Object.assign(o, from);
}

// A session, as JS sees it: a view onto its state in Rust.
class LiveSession {
  #core;
  #id;
  #maps = new Map();
  #mirror = null;
  constructor(core, id) {
    this.#core = core;
    this.#id = id;
  }
  get id() {
    return this.#id;
  }
  // The session's own objects, as they are now.
  mirror() {
    this.#mirror ??= { tokens: {}, context: {}, toolCounts: {}, prompts: [], messages: [], texts: -1 };
    this.refresh();
    return this.#mirror;
  }
  // Writes Rust's state into the objects handed out, once there are any.
  refresh() {
    const m = this.#mirror;
    if (!m) return;
    const now = this.#core.sessionMirror(this.#id, m.texts);
    if (!now) return;
    fill(m.tokens, now.tokens);
    fill(m.context, now.context);
    fill(m.toolCounts, now.toolCounts);
    if (now.prompts) {
      m.prompts.splice(0, m.prompts.length, ...now.prompts);
      m.messages.splice(0, m.messages.length, ...now.messages);
    }
    m.texts = now.texts;
  }
  // Pushes onto the list reach Rust (the console's made-up states push
  // there); other changes to it don't.
  get recentTools() {
    const list = this.#core.sessionGet(this.#id, "recentTools") ?? [];
    const core = this.#core, id = this.#id;
    Object.defineProperty(list, "push", {
      value: (...items) => {
        core.sessionPushTools(id, items);
        return Array.prototype.push.apply(list, items);
      },
    });
    return list;
  }
  set recentTools(list) {
    this.#core.sessionSet(this.#id, "recentTools", list);
  }
  get seenMessages() {
    return this.#view("seenMessages", SetView);
  }
  #view(name, Kind = MapView) {
    if (!this.#maps.has(name)) this.#maps.set(name, new Kind(this.#core, this.#id, name));
    return this.#maps.get(name);
  }
  // A call that changes the session, and the objects handed out after it.
  #change(fn) {
    try {
      return fn();
    } finally {
      this.refresh();
    }
  }

  addTokens(n, ts, live) {
    this.#change(() => this.#core.sessionAddTokens(this.#id, Number(n), Number(ts), !!live));
  }
  addCall(model, k, ts, opts) {
    this.#change(() => this.#core.sessionAddCall(this.#id, model, k, Number(ts), opts));
  }
  addPrompt(text, ts) {
    this.#change(() => this.#core.sessionAddPrompt(this.#id, String(text ?? ""), Number(ts)));
  }
  addMessage(text, ts) {
    this.#change(() => this.#core.sessionAddMessage(this.#id, text, Number(ts)));
  }
  endTurn(ts, { interrupted = false } = {}) {
    this.#change(() => this.#core.sessionEndTurn(this.#id, Number(ts), !!interrupted));
  }
  addEdits(edits, ts) {
    this.#core.sessionAddEdits(this.#id, edits, Number(ts));
  }
  resetStall() {
    this.#core.sessionResetStall(this.#id);
  }
  // A record of the call given here is JS's own copy, which Rust doesn't
  // follow: the result counts as one without a record.
  noteToolResult(ok, ts) {
    this.#core.sessionNoteToolResult(this.#id, !!ok, Number(ts));
  }
  tokensPerMinute(now) {
    return this.#core.sessionTokensPerMinute(this.#id, Number(now));
  }
  status(now) {
    return this.#core.sessionStatus(this.#id, Number(now));
  }
  needsYou(now) {
    return this.#core.sessionNeedsYou(this.#id, Number(now));
  }
  stuck(now) {
    return this.#core.sessionStuck(this.#id, Number(now));
  }
  forecast() {
    return this.#core.sessionForecast(this.#id);
  }
  snapshot(now) {
    const snap = this.#core.sessionSnapshot(this.#id, Number(now));
    if (!snap) return snap;
    const m = this.mirror();
    return Object.assign(snap, { tokens: m.tokens, context: m.context, toolCounts: m.toolCounts });
  }

  static {
    for (const name of FIELDS)
      Object.defineProperty(LiveSession.prototype, name, {
        get() {
          return this.#core.sessionGet(this.#id, name);
        },
        set(v) {
          this.#core.sessionSet(this.#id, name, v);
        },
      });
    for (const name of MIRRORED)
      Object.defineProperty(LiveSession.prototype, name, {
        get() {
          return this.mirror()[name];
        },
        set(v) {
          this.#change(() => this.#core.sessionSet(this.#id, name, v));
        },
      });
    for (const name of MAPS)
      Object.defineProperty(LiveSession.prototype, name, {
        get() {
          return this.#view(name);
        },
      });
  }
}

// The sessions, a Map of their views (sessions are never forgotten).
class Sessions {
  #core;
  #face;
  constructor(core, face) {
    this.#core = core;
    this.#face = face;
  }
  get size() {
    return this.#core.ids().length;
  }
  has(id) {
    return this.#core.has(String(id));
  }
  get(id) {
    return this.#core.has(String(id)) ? this.#face(String(id)) : undefined;
  }
  keys() {
    return this.#core.ids()[Symbol.iterator]();
  }
  values() {
    return this.#core.ids().map((id) => this.#face(id))[Symbol.iterator]();
  }
  entries() {
    return this.#core.ids().map((id) => [id, this.#face(id)])[Symbol.iterator]();
  }
  forEach(fn, self) {
    for (const [id, s] of this.entries()) fn.call(self, s, id, this);
  }
  [Symbol.iterator]() {
    return this.entries();
  }
}

// `limits` and `limitSamples`: { codex, claude }, kept in Rust.
function pair(get, set) {
  const face = {};
  for (const which of ["codex", "claude"]) Object.defineProperty(face, which, { get: () => get(which), set: (v) => set(which, v), enumerable: true });
  return face;
}

// ---------- the tailer ----------

// What happens to transcripts' lines. Reading the files is the tailer
// block's (core/blocks/tailer.js); Rust turns each line into state and
// events.
export class LiveTelemetry extends EventEmitter {
  #core;
  #faces = new Map();

  constructor({ claudeRoot, codexRoot, grokRoot = null, claudeDesktopDir, claudeUsage = null, home = null, contextServer = null }) {
    super();
    this.contextServer = contextServer;
    // Watch canonical long paths: on Windows, libuv aborts the process when a
    // recursive watch root is an 8.3 short name (C:\Users\ABCDEF~1\...).
    this.roots = [
      { dir: claudeRoot, app: "claude" },
      { dir: codexRoot, app: "codex" },
      { dir: grokRoot, app: "grok" },
    ]
      .filter((r) => r.dir && fs.existsSync(r.dir))
      .map((r) => ({ ...r, dir: fs.realpathSync.native(r.dir) }));
    const core = (this.#core = new native.LiveCore(home ? String(home) : null, this.roots, typeof contextServer === "string" ? contextServer : null));
    this.claudeDesktopDir = claudeDesktopDir;
    // The readings of Claude plan usage, as gatherers: Anthropic's, when
    // claudeUsage (() => fetchClaudeUsage()) is given, and Claude
    // desktop's samples, when its folder is there.
    this.usage = {
      anthropic: claudeUsage ? claudeUsageGatherer({ fetch: claudeUsage }) : null,
      desktop: claudeDesktopDir ? desktopUsageGatherer({ dir: claudeDesktopDir }) : null,
    };
    this.sessions = new Sessions(core, (id) => this.#face(id));
    this.limits = pair((w) => core.limits(w), (w, v) => core.setLimits(w, v));
    this.limitSamples = pair((w) => core.limitSamples(w), (w, v) => core.setLimitSamples(w, v));
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
      onPrimed: () => core.primed(Date.now()),
      primeWindowMs: PRIME_WINDOW_MS,
      sweepMs: SWEEP_MS,
      discoverEvery: DISCOVER_EVERY,
      status: this.status,
    });
  }

  get home() {
    return this.#core.home ?? null;
  }
  set home(v) {
    this.#core.home = v ? String(v) : null;
  }
  // Every event of the last day, in time order.
  get history() {
    return this.#core.history();
  }
  get claudeReason() {
    return this.#core.claudeReason;
  }
  set claudeReason(v) {
    this.#core.setClaudeReason(v);
  }
  // A copy: { anthropic, desktop }.
  get claudeSamples() {
    return this.#core.claudeSamples;
  }
  get pendingLabel() {
    return this.#core.pendingLabel;
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

  #face(id) {
    let s = this.#faces.get(id);
    if (!s) this.#faces.set(id, (s = new LiveSession(this.#core, id)));
    return s;
  }

  // Emits what a call into Rust made, in order, and says what JS would
  // have thrown after it.
  #run(res, id) {
    this.#faces.get(id)?.refresh();
    for (const [name, payload] of res.emits) this.emit(name, payload);
    return res.error;
  }

  session(id, app) {
    this.#core.session(String(id), String(app));
    return this.#face(String(id));
  }

  // ----- discovery -----

  classify(file) {
    return this.#core.classify(String(file));
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

  // Catches up on what no watch event announced. One sweep at a time.
  sweep(opts) {
    return this.tailer.sweep(opts);
  }

  // Reads what's new in a transcript, after any read of it under way.
  track(file, opts) {
    return this.tailer.track(file, opts);
  }

  // A line the tailer read, through its app's normalizer (lines.js):
  // lines that aren't JSON are skipped, and one that breaks the normalizer
  // or what Rust makes of it is counted as an error, so the lines after it
  // are read.
  #line(line, f, prime) {
    try {
      const rec = linesOf(f.app).line(line, f);
      if (!rec) return;
      this.status.lines++;
      const id = String(f.sessionId);
      // Any app that isn't tailed is read as Claude Code, as linesOf reads
      // it.
      const res = this.#core.line(tailedApp(f.app, "claude"), id, f.agentId ?? null, f.file ?? null, rec.raw, !prime, Date.now());
      if (this.#run(res, id) != null) this.status.errors++;
    } catch {
      this.status.errors++;
    }
  }

  trimHistory() {
    this.#core.trimHistory(Date.now());
  }

  // An event for a session, which takes its place in the history.
  activity(s, ev, live) {
    this.#run(this.#core.activity(String(s.id), String(s.app), ev, !!live, Date.now()), String(s.id));
  }

  noteTool(s, call, live) {
    const res = this.#core.noteTool(String(s.id), call, !!live, Date.now());
    this.#run(res, String(s.id));
    return res.tool;
  }

  // Writes to the shared context name the pages they touch; the UI beams
  // to them.
  wikiPagesOf(server, input, tool = null) {
    return native.liveTelemetryPagesOf(server, input, tool, this.contextServer) ?? null;
  }

  // Claude Code Notification hooks carry permission prompts the transcript
  // doesn't; Grok Build's hooks say when its turns end.
  noteHook(sessionId, body) {
    this.#throw(this.#core.noteHook(String(sessionId), body, Date.now()), String(sessionId));
  }

  // Turns gone quiet in an app whose lines don't say when a turn ends.
  settle(now = Date.now()) {
    this.#core.settle(Number(now));
  }

  #throw(res, id) {
    const err = this.#run(res, id);
    if (err != null) throw new TypeError(err);
  }

  // A Claude Code transcript's line: normalized (see lines.js), or as the
  // transcript has it.
  applyClaude(f, line, live) {
    const raw = line instanceof ClaudeLine ? line.raw : line;
    const id = String(f.sessionId);
    this.#throw(this.#core.apply(false, id, f.agentId ?? null, raw, !!live, Date.now()), id);
  }

  // A Codex rollout's line: normalized (see lines.js), or as the rollout
  // has it.
  applyCodex(f, line, live) {
    const raw = line instanceof CodexLine ? line.raw : line;
    const id = String(f.sessionId);
    this.#throw(this.#core.apply(true, id, f.agentId ?? null, raw, !!live, Date.now()), id);
  }

  codexItem(s, it, ts, live) {
    this.#throw(this.#core.codexItem(String(s.id), it, Number(ts), !!live, Date.now()), String(s.id));
  }

  // Codex's rate limits in XenoNet's own shape (live/snapshot's limits.codex).
  noteCodexLimits(rl, ts) {
    this.#core.noteCodexLimits(rl, Number(ts));
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
      if (anthropic.last !== before) this.#core.claudeAsked(r, Number(now));
    }
    if (this.usage.desktop) {
      const samples = await this.usage.desktop.run({ now });
      if (samples) this.#core.desktopRead(samples);
    }
    this.#core.settleClaude();
  }

  // An old Claude reading says nothing about the windows now, so it's
  // reported as missing, with when it was taken and why there's no newer one.
  claudeLimits(now = Date.now()) {
    return this.#core.claudeLimits(Number(now));
  }

  // Straight-line projection of usage over the recent samples.
  limitForecast(now = Date.now()) {
    return this.#core.limitForecast(Number(now));
  }

  // ----- outputs -----

  // Sessions that touched (read or edited) the same file recently.
  collab(snapshots) {
    return this.#core.collab(snapshots);
  }

  // Sessions that both edited the same file within ten minutes.
  collisions(now = Date.now()) {
    return this.#core.collisions(Number(now));
  }

  collisionDiffs(a, b) {
    return this.#core.collisionDiffs(String(a), String(b));
  }

  // Context for a handoff brief, from what the tailer has seen.
  sessionNotes(id) {
    const notes = this.#core.sessionNotes(String(id), Date.now());
    if (!notes) return notes;
    const m = this.#face(String(id)).mirror();
    return Object.assign(notes, { prompts: m.prompts, messages: m.messages, context: m.context });
  }

  historySince(since) {
    return this.#core.historySince(Number(since));
  }

  // What today's calls would cost at API prices (pricing.js), by app, and
  // at this minute's pace. Models without a price are named, not guessed.
  costs(now, sessions) {
    return this.#core.costs(Number(now), sessions);
  }

  // What each session's calls today would cost at API prices, the same
  // sums costs() adds up (core/actors/spend.js).
  sessionCosts(now) {
    return this.#core.sessionCosts(Number(now));
  }

  snapshot() {
    const snap = this.#core.snapshot(Date.now());
    for (const s of snap.sessions) {
      const m = this.#face(s.id).mirror();
      Object.assign(s, { tokens: m.tokens, context: m.context, toolCounts: m.toolCounts });
    }
    snap.status = this.status;
    return snap;
  }
}
