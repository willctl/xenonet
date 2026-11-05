// What an OpenCode session's XenoNet plugin tells XenoNet as it happens
// (scripts/demo/opencode-plugin.js), kept in memory per session: whether a
// turn is under way, a permission it waits on, the files it edited, and
// what its finished assistant messages used. Metadata only: the plugin
// sends no prompt, reply, tool output or file content, and this keeps none.
//
// The map prefers it to reading OpenCode's database (core/native/opencode.js)
// for as long as it's fresh, and falls back to the database when the plugin
// is gone: a working mark lapses WORKING_MS after the session's last event,
// and a wait lapses after WAIT_MS. The database stays the reader for
// history and for sessions without the plugin.
//
// An event is { type, … }, one of:
//   status      { state: "busy" | "idle" }
//   permission  { state: "asked" | "replied", id, tool? }
//   edit        { path }
//   message     { id, cost?, tokens?: { input, output, reasoning, cacheRead,
//               cacheWrite }, provider?, model? }
// Anything else, or a field of the wrong kind, is ignored, so a plugin newer
// than this XenoNet does no harm. The Rust twin is xenonet-sources'
// native::opencode_events.
import { EDIT_KEEP_MS, editFile, pushEdit } from "./reader.js";

export const WORKING_MS = 5 * 60e3;
export const WAIT_MS = 30 * 60e3;
export const SESSIONS_MAX = 200;
const WAITS_MAX = 16;
const MESSAGES_MAX = 64;
const EVENTS_MAX = 20;
const PATH_MAX = 4096;
const NAME = /^[\w.:\/-]{1,80}$/;
const ID = /^[\w.:-]{1,128}$/;
const TOKENS = ["input", "output", "reasoning", "cacheRead", "cacheWrite"];

const num = (v) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : null);

export class OpenCodeEvents {
  constructor() {
    this.sessions = new Map(); // XenoNet's id → session
  }

  #of(id, now) {
    let s = this.sessions.get(id);
    if (!s) {
      s = { id, at: now, working: false, workingAt: 0, waits: new Map(), edits: new Map(), messages: new Map(), cost: null, tokens: null, model: null, provider: null, cwd: null };
      this.sessions.set(id, s);
      if (this.sessions.size > SESSIONS_MAX) this.sessions.delete([...this.sessions.values()].sort((a, b) => a.at - b.at)[0].id);
    }
    return s;
  }

  // Takes a request's events for session `id` (opencode:ses-…), run in
  // `cwd`. Returns how many it used.
  ingest(id, events, { cwd = null, now = Date.now() } = {}) {
    if (!Array.isArray(events)) return 0;
    let used = 0;
    for (const e of events.slice(0, EVENTS_MAX)) {
      if (!e || typeof e !== "object") continue;
      const s = this.#of(id, now);
      if (typeof cwd === "string" && cwd) s.cwd = cwd;
      if (this.#take(s, e, now)) (s.at = now), used++;
    }
    return used;
  }

  #take(s, e, now) {
    if (e.type === "status") {
      if (e.state !== "busy" && e.state !== "idle") return false;
      s.working = e.state === "busy";
      s.workingAt = now;
      // A turn that ended asks for nothing more.
      if (!s.working) s.waits.clear();
      return true;
    }
    if (e.type === "permission") {
      if (typeof e.id !== "string" || !ID.test(e.id)) return false;
      if (e.state === "asked") {
        if (s.waits.size >= WAITS_MAX) s.waits.delete(s.waits.keys().next().value);
        s.waits.set(e.id, { tool: typeof e.tool === "string" && NAME.test(e.tool) ? e.tool : null, since: now });
        return true;
      }
      if (e.state === "replied") return s.waits.delete(e.id) || true;
      return false;
    }
    if (e.type === "edit") {
      if (typeof e.path !== "string" || !e.path || e.path.length > PATH_MAX) return false;
      const file = editFile(s.cwd, e.path);
      if (!file) return false;
      pushEdit(s.edits, file, now);
      return true;
    }
    if (e.type === "message") {
      if (typeof e.id !== "string" || !ID.test(e.id) || s.messages.has(e.id)) return false;
      s.messages.set(e.id, now);
      if (s.messages.size > MESSAGES_MAX) s.messages.delete(s.messages.keys().next().value);
      const cost = num(e.cost);
      if (cost != null) s.cost = (s.cost ?? 0) + cost;
      if (e.tokens && typeof e.tokens === "object") {
        for (const k of TOKENS) {
          const n = num(e.tokens[k]);
          if (n == null) continue;
          s.tokens ??= Object.fromEntries(TOKENS.map((t) => [t, null]));
          s.tokens[k] = (s.tokens[k] ?? 0) + n;
        }
      }
      if (typeof e.model === "string" && NAME.test(e.model)) s.model = e.model;
      if (typeof e.provider === "string" && NAME.test(e.provider)) s.provider = e.provider;
      return true;
    }
    return false;
  }

  // The session's state as the plugin last said it, or null when it hasn't
  // been heard from lately enough to go by (the database decides then).
  state(id, now = Date.now()) {
    const s = this.sessions.get(id);
    if (!s) return null;
    for (const [p, list] of s.edits) if (now - list[list.length - 1].ts > EDIT_KEEP_MS) s.edits.delete(p);
    const heard = now - s.at < WORKING_MS;
    const wait = [...s.waits.values()].filter((w) => now - w.since < WAIT_MS).sort((a, b) => a.since - b.since)[0];
    if (!heard && !wait && !s.edits.size) return null;
    const usage = s.cost == null && !s.tokens && !s.model && !s.provider ? null : { costUsd: s.cost, tokens: s.tokens ?? Object.fromEntries(TOKENS.map((k) => [k, null])), provider: s.provider, model: s.model };
    return {
      lastEventAt: s.at,
      status: s.working && now - s.workingAt < WORKING_MS ? "working" : "idle",
      needsYou: wait ? { kind: "permission", text: wait.tool ? `Permission to use ${wait.tool}` : "Permission needed", since: wait.since } : null,
      edits: s.edits,
      model: s.model,
      usage,
    };
  }

  // The live view's session snapshots, in the shape the tailer's have, for
  // the sessions the plugin has spoken for and `has(id)` says are on the map.
  snapshot(now = Date.now(), has = () => true) {
    const out = [];
    for (const id of this.sessions.keys()) {
      const st = this.state(id, now);
      if (!st || !has(id)) continue;
      out.push({
        id,
        app: "opencode",
        status: st.status === "working" ? "thinking" : "idle",
        tokens: { processed: 0, output: 0 },
        tokensToday: 0,
        tpm: 0,
        context: { used: 0, window: 200000 },
        forecast: null,
        model: st.model,
        turnStartedAt: null,
        lastTool: null,
        toolCounts: {},
        subagents: [],
        files: [...st.edits.values()].map((l) => l[l.length - 1]).sort((a, b) => b.ts - a.ts).slice(0, 10).map((e) => ({ path: e.path, ts: e.ts })),
        mcp: [],
        needsYou: st.needsYou,
        stuck: null,
        lastMessage: null,
        lastTurnEndAt: null,
        interruptedAt: null,
        lastEventAt: st.lastEventAt,
      });
    }
    return out;
  }
}
