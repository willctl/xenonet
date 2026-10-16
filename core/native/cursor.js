// The Cursor reader (experimental): each Cursor agent conversation is a
// session on the map, "cursor:<conversation id>", from what XenoNet's Cursor
// hook (scripts/cursor/hook.mjs) writes: where it ran, the files it edited
// (Cursor's afterFileEdit), and when it ended (sessionEnd). Its edits count
// in the warnings other sessions get before an edit. XenoNet can't message a
// Cursor session, so the map shows it without a way to.
//
// It's off unless XENONET_READERS names cursor, and needs the hook set up in
// Cursor's hooks.json. XenoNet reads the hook's file and never writes it.
// Schema: core/schema/vendor/anysphere/cursor.js.
import fs from "node:fs";
import path from "node:path";
import { Block } from "../blocks/lifecycle.js";
import { pathStyle } from "../paths.js";
import { reader } from "../vendor.js";
import { cleanVersion } from "../drift.js";

export const SOURCE = "anysphere/cursor/hook-input";
const DAY_MS = 24 * 3600e3;
const EDIT_KEEP_MS = 30 * 60e3;
const SESSIONS_MAX = 500;
const READ_MAX = 4 << 20;
const CHECKS_MAX = 32;
const ID = /^[0-9A-Za-z][0-9A-Za-z-]{6,127}$/;

export const sessionIdOf = (id) => (typeof id === "string" && ID.test(id) ? `cursor:${id}` : null);

// The last READ_MAX of the file, whole lines only.
function tail(file, files) {
  const st = files.statSync(file);
  const from = Math.max(0, st.size - READ_MAX);
  const buf = Buffer.alloc(st.size - from);
  const fd = files.openSync(file, "r");
  try {
    files.readSync(fd, buf, 0, buf.length, from);
  } finally {
    files.closeSync(fd);
  }
  const lines = buf.toString("utf8").split("\n");
  if (from > 0) lines.shift();
  return { lines, mtimeMs: st.mtimeMs, size: st.size };
}

export class CursorSessions extends Block {
  constructor({ file, files = fs, clock } = {}) {
    super({ name: "cursor-sessions", kind: "gatherer", ...(clock ? { clock } : {}) });
    this.file = file;
    this.files = files;
    this.runs = new Map();
    this.fileState = "unread";
    this.seen = null;
  }

  read(now = this.clock.now()) {
    try {
      let t;
      try {
        t = tail(this.file, this.files);
      } catch (err) {
        this.fileState = err?.code === "ENOENT" ? "missing" : "unreadable";
        this.runs.clear();
        return this.ok(now), this;
      }
      this.fileState = "ok";
      const stamp = `${t.size}:${t.mtimeMs}`;
      if (stamp !== this.seen) {
        this.seen = stamp;
        this.runs = this.#parse(t.lines, now);
      } else this.#prune(now);
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  #parse(lines, now) {
    const r = reader(SOURCE);
    const runs = new Map();
    let checked = 0;
    for (const line of lines) {
      if (!line.trim()) continue;
      let e;
      try {
        e = JSON.parse(line);
      } catch {
        continue;
      }
      if (!e || typeof e !== "object") continue;
      if (checked++ < CHECKS_MAX) e = r.check(e, { version: cleanVersion(typeof e.cursor_version === "string" ? e.cursor_version : null) }) ?? e;
      const at = Date.parse(e.at ?? "");
      if (!Number.isFinite(at) || now - at > DAY_MS) continue;
      const raw = e.conversation_id ?? e.session_id;
      const id = sessionIdOf(raw);
      if (!id) continue;
      let run = runs.get(id);
      if (!run) runs.set(id, (run = { id, rawId: raw, cwd: null, createdAt: at, updatedAt: at, endedAt: null, edits: new Map() }));
      run.createdAt = Math.min(run.createdAt, at);
      run.updatedAt = Math.max(run.updatedAt, at);
      const root = Array.isArray(e.workspace_roots) ? e.workspace_roots.find((x) => typeof x === "string" && pathStyle(x).isAbsolute(x)) : null;
      if (root && !run.cwd) run.cwd = root;
      if (e.hook_event_name === "sessionEnd") run.endedAt = at;
      else if (run.endedAt != null && at > run.endedAt) run.endedAt = null;
      if (e.hook_event_name === "afterFileEdit" && typeof e.file_path === "string" && pathStyle(e.file_path).isAbsolute(e.file_path)) {
        const file = pathStyle(e.file_path).resolve(e.file_path);
        const list = run.edits.get(file) ?? [];
        list.push({ ts: Math.min(at, now), path: file });
        if (list.length > 8) list.shift();
        run.edits.set(file, list);
      }
    }
    if (runs.size > SESSIONS_MAX) for (const k of [...runs.values()].sort((a, b) => a.updatedAt - b.updatedAt).slice(0, runs.size - SESSIONS_MAX).map((x) => x.id)) runs.delete(k);
    this.runs = runs;
    this.#prune(now);
    return runs;
  }

  #prune(now) {
    for (const [id, r] of this.runs) {
      if (now - r.updatedAt > DAY_MS) this.runs.delete(id);
      else for (const [p, list] of r.edits) if (now - list[list.length - 1].ts > EDIT_KEEP_MS) r.edits.delete(p);
    }
  }

  sessions() {
    return [...this.runs.values()].map((r) => ({
      id: r.id,
      app: "cursor",
      rawId: r.rawId,
      title: `Cursor${r.cwd ? ` in ${pathStyle(r.cwd).basename(r.cwd)}` : ""}`,
      preview: null,
      cwd: r.cwd,
      createdAt: r.createdAt,
      updatedAt: r.updatedAt,
      archived: false,
      model: null,
      branch: null,
      parentId: null,
      parentKind: null,
      reader: "experimental",
    }));
  }

  live(id) {
    const r = this.runs.get(id);
    if (!r) return null;
    return { lastEventAt: r.updatedAt, endedAt: r.endedAt, edits: r.edits, observed: false, status: () => "idle", needsYou: () => null };
  }

  ids() {
    return [...this.runs.keys()];
  }

  status() {
    return { name: "cursor", file: this.file, state: this.fileState, sessions: this.runs.size };
  }
}

// The file the hook writes: XENONET_CURSOR_EVENTS, else cursor-hooks.ndjson
// in XenoNet's state folder.
export const cursorEventsFile = ({ env = process.env, stateDir }) => (env.XENONET_CURSOR_EVENTS ? path.resolve(env.XENONET_CURSOR_EVENTS) : path.join(stateDir, "cursor-hooks.ndjson"));
