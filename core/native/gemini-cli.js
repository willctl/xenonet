// The Gemini CLI reader (experimental): each Gemini CLI session is a
// session on the map, "gemini-cli:<sessionId>", read from its chat records,
// <gemini home>/.gemini/tmp/<project>/chats/session-*.jsonl, with a
// subagent's run (chats/<parent>/<id>.jsonl) under the session that started
// it. The files its write_file and replace calls changed count in the
// warnings other sessions get before an edit. XenoNet can't message a Gemini
// CLI session, so the map shows it without a way to. A numbat-observed run
// with the same id is this one, and this read wins.
//
// It's off unless XENONET_READERS names gemini-cli, and never in the demo.
// Schema: core/schema/vendor/google/gemini-cli.js.
import fs from "node:fs";
import path from "node:path";
import { Block } from "../blocks/lifecycle.js";
import { pathStyle } from "../paths.js";
import { reader } from "../vendor.js";
import { APP } from "../apps.js";

export const SOURCE = "google/gemini-cli/chat-record";
export const EDIT_TOOLS = new Set(APP["gemini-cli"].editTools);
const FAILED = new Set(["error", "cancelled"]);
const DAY_MS = 24 * 3600e3;
const EDIT_KEEP_MS = 30 * 60e3;
const SESSIONS_MAX = 500;
const FILE_MAX = 16 << 20;
const CHECKS_PER_FILE = 12;
const ID = /^[0-9A-Za-z][0-9A-Za-z-]{6,127}$/;

export const sessionIdOf = (id) => (ID.test(id) ? `gemini-cli:${id}` : null);
const ms = (v) => {
  const t = typeof v === "string" ? Date.parse(v) : NaN;
  return Number.isFinite(t) ? t : null;
};

// One chat record file as XenoNet reads it: { meta, messages, edits }, the
// messages in order after rewinds; or null when it isn't one.
export function readChat(text, { check = () => {} } = {}) {
  let records;
  const t = text.trimStart();
  if (t.startsWith("{") && !t.includes("\n{")) {
    // The older layout: one JSON object with its messages inside.
    try {
      const whole = JSON.parse(t);
      const { messages = [], ...meta } = whole ?? {};
      records = [meta, ...(Array.isArray(messages) ? messages : [])];
    } catch {
      return null;
    }
  } else {
    records = [];
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      try {
        records.push(JSON.parse(line));
      } catch {}
    }
  }
  let meta = null;
  const messages = new Map();
  let checked = 0;
  for (let r of records) {
    if (!r || typeof r !== "object" || Array.isArray(r)) continue;
    if (checked < CHECKS_PER_FILE && (!meta || r.toolCalls)) {
      r = check(r) ?? r;
      checked++;
    }
    if (typeof r.sessionId === "string" && typeof r.projectHash === "string" && !("id" in r)) meta = { ...(meta ?? {}), ...r };
    else if (typeof r.$rewindTo === "string") {
      let found = false;
      for (const id of [...messages.keys()]) if ((found ||= id === r.$rewindTo)) messages.delete(id);
      if (!found) messages.clear();
    } else if (r.$set && typeof r.$set === "object") meta = { ...(meta ?? {}), ...r.$set };
    else if (typeof r.id === "string" && !("$patch" in r)) messages.set(r.id, r);
  }
  if (!meta || typeof meta.sessionId !== "string") return null;
  const edits = [];
  for (const m of messages.values()) {
    for (const c of Array.isArray(m.toolCalls) ? m.toolCalls : []) {
      if (!c || !EDIT_TOOLS.has(c.name) || FAILED.has(c.status)) continue;
      const file = c.args?.file_path;
      if (typeof file === "string" && file) edits.push({ path: file, at: ms(c.timestamp) ?? ms(m.timestamp) });
    }
  }
  const times = [...messages.values()].map((m) => ms(m.timestamp)).filter((x) => x != null);
  return {
    id: meta.sessionId,
    kind: typeof meta.kind === "string" ? meta.kind : null,
    summary: typeof meta.summary === "string" && meta.summary ? meta.summary : null,
    startedAt: ms(meta.startTime) ?? (times.length ? Math.min(...times) : null),
    updatedAt: Math.max(ms(meta.lastUpdated) ?? 0, ...times, 0) || null,
    edits,
  };
}

// The Gemini CLI's home: GEMINI_CLI_HOME, else your home.
export const geminiRoot = ({ env = process.env, home }) => path.join(env.GEMINI_CLI_HOME || home, ".gemini", "tmp");

export class GeminiCliSessions extends Block {
  constructor({ root, files = fs, clock } = {}) {
    super({ name: "gemini-cli-sessions", kind: "gatherer", ...(clock ? { clock } : {}) });
    this.root = root;
    this.files = files;
    this.runs = new Map();
    this.cache = new Map(); // file → { mtimeMs, size, chat }
    this.rootState = "unread";
  }

  // The chat files changed in the last day: [{ file, parent, cwd, mtimeMs, size }].
  #files(now) {
    const out = [];
    const ls = (d) => {
      try {
        return this.files.readdirSync(d, { withFileTypes: true });
      } catch {
        return [];
      }
    };
    let projects;
    try {
      projects = this.files.readdirSync(this.root, { withFileTypes: true });
      this.rootState = "ok";
    } catch (err) {
      this.rootState = err?.code === "ENOENT" ? "missing" : "unreadable";
      return out;
    }
    for (const p of projects) {
      if (!p.isDirectory()) continue;
      const dir = path.join(this.root, p.name);
      let cwd = null;
      try {
        cwd = this.files.readFileSync(path.join(dir, ".project_root"), "utf8").trim() || null;
      } catch {}
      const chats = path.join(dir, "chats");
      const take = (file, parent) => {
        try {
          const st = this.files.statSync(file);
          if (now - st.mtimeMs <= DAY_MS && st.size <= FILE_MAX) out.push({ file, parent, cwd, mtimeMs: st.mtimeMs, size: st.size });
        } catch {}
      };
      for (const e of ls(chats)) {
        if (e.isFile() && e.name.startsWith("session-") && /\.jsonl?$/.test(e.name)) take(path.join(chats, e.name), null);
        else if (e.isDirectory()) for (const s of ls(path.join(chats, e.name))) if (s.isFile() && /\.jsonl?$/.test(s.name)) take(path.join(chats, e.name, s.name), e.name);
      }
    }
    return out.sort((a, b) => b.mtimeMs - a.mtimeMs).slice(0, SESSIONS_MAX);
  }

  read(now = this.clock.now()) {
    try {
      const r = reader(SOURCE);
      const runs = new Map();
      const seen = new Set();
      for (const f of this.#files(now)) {
        seen.add(f.file);
        let hit = this.cache.get(f.file);
        if (!hit || hit.mtimeMs !== f.mtimeMs || hit.size !== f.size) {
          let chat = null;
          try {
            chat = readChat(this.files.readFileSync(f.file, "utf8"), { check: (rec) => r.check(rec, { version: null }) });
          } catch {}
          hit = { mtimeMs: f.mtimeMs, size: f.size, chat };
          this.cache.set(f.file, hit);
        }
        const c = hit.chat;
        const id = c && sessionIdOf(c.id);
        if (!id) continue;
        const parentId = f.parent ? sessionIdOf(f.parent) : null;
        const run = { id, rawId: c.id, cwd: f.cwd, summary: c.summary, parentId, createdAt: c.startedAt ?? f.mtimeMs, updatedAt: Math.max(c.updatedAt ?? 0, 0) || f.mtimeMs, edits: new Map() };
        for (const e of c.edits) {
          const at = Math.min(e.at ?? run.updatedAt, now);
          if (now - at > EDIT_KEEP_MS) continue;
          const ps = pathStyle(run.cwd ?? e.path);
          const file = ps.isAbsolute(e.path) ? ps.resolve(e.path) : run.cwd ? ps.resolve(run.cwd, e.path) : null;
          if (!file) continue;
          const list = run.edits.get(file) ?? [];
          list.push({ ts: at, path: file });
          if (list.length > 8) list.shift();
          run.edits.set(file, list);
        }
        const had = runs.get(id);
        if (!had || had.updatedAt < run.updatedAt) runs.set(id, run);
      }
      for (const k of this.cache.keys()) if (!seen.has(k)) this.cache.delete(k);
      this.runs = runs;
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  sessions() {
    return [...this.runs.values()].map((r) => ({
      id: r.id,
      app: "gemini-cli",
      rawId: r.rawId,
      title: r.summary ?? `Gemini CLI${r.cwd ? ` in ${pathStyle(r.cwd).basename(r.cwd)}` : ""}`,
      preview: null,
      cwd: r.cwd,
      createdAt: r.createdAt,
      updatedAt: r.updatedAt,
      archived: false,
      model: null,
      branch: null,
      parentId: r.parentId,
      parentKind: r.parentId ? "subagent" : null,
      reader: "experimental",
    }));
  }

  live(id) {
    const r = this.runs.get(id);
    if (!r) return null;
    return { lastEventAt: r.updatedAt, endedAt: null, edits: r.edits, observed: false, status: () => "idle", needsYou: () => null };
  }

  ids() {
    return [...this.runs.keys()];
  }

  status() {
    return { name: "gemini-cli", root: this.root, state: this.rootState, sessions: this.runs.size };
  }
}
