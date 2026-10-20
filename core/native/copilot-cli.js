// The Copilot CLI reader (experimental): each GitHub Copilot CLI session is
// a session on the map, "copilot:<session id>", read from its event log,
// ~/.copilot/session-state/<id>/events.jsonl (or <id>.jsonl, the layout
// before 1.0.11). The files its edit tools name, and the files its shutdown
// says it changed, count in the warnings other sessions get before an edit.
// XenoNet can't message a Copilot CLI session, so the map shows it without
// a way to.
//
// It's off unless XENONET_READERS names copilot, and never in the demo.
// Schema: core/schema/vendor/github/copilot-cli.js.
import fs from "node:fs";
import path from "node:path";
import { reader } from "../vendor.js";
import { APP } from "../apps.js";
import { patchPaths } from "../patch.js";
import { NativeReader, DAY_MS, EDIT_KEEP_MS, SESSIONS_MAX, FileCache, editFile, fitsId, ms, pushEdit, runSession, titled } from "./reader.js";

export const SOURCE = "github/copilot-cli/session-event";
// Copilot CLI's built-in file tools; str_replace_editor's view isn't an edit.
export const EDIT_TOOLS = new Set(APP.copilot.editTools);
const FILE_MAX = 32 << 20;
const CHECKS_PER_FILE = 16;

export const sessionIdOf = (id) => (fitsId(id) ? `copilot:${id}` : null);

// The files a tool call names, if it's an edit.
export function editedBy(toolName, args) {
  if (!EDIT_TOOLS.has(toolName)) return [];
  if (typeof args === "string") return toolName === "apply_patch" ? patchPaths(args) : [];
  if (!args || typeof args !== "object") return [];
  if (toolName === "str_replace_editor" && args.command === "view") return [];
  const out = [];
  for (const k of ["path", "file_path", "filePath"]) if (typeof args[k] === "string" && args[k]) out.push(args[k]);
  if (toolName === "apply_patch") for (const v of Object.values(args)) if (typeof v === "string") out.push(...patchPaths(v));
  return out;
}

// One session's event log as XenoNet reads it, or null when it has no
// session.start.
export function readEvents(text, { check = (r) => r } = {}) {
  let start = null;
  let cwd = null;
  let branch = null;
  let title = null;
  let endedAt = null;
  let last = null;
  const edits = [];
  let checked = 0;
  for (const line of text.split("\n")) {
    if (!line.trim()) continue;
    let e;
    try {
      e = JSON.parse(line);
    } catch {
      continue;
    }
    if (!e || typeof e !== "object" || typeof e.type !== "string") continue;
    if (checked < CHECKS_PER_FILE && (e.type.startsWith("session.") || e.type === "tool.execution_start")) {
      e = check(e) ?? e;
      checked++;
    }
    const at = ms(e.timestamp);
    if (at != null) last = Math.max(last ?? 0, at);
    const d = e.data && typeof e.data === "object" ? e.data : {};
    if (e.type === "session.start" && typeof d.sessionId === "string") {
      start = { id: d.sessionId, at: ms(d.startTime) ?? at, version: d.version };
      if (typeof d.context?.cwd === "string") cwd = d.context.cwd;
      if (typeof d.context?.branch === "string") branch = d.context.branch;
    } else if (e.type === "session.resume") {
      endedAt = null;
      if (typeof d.context?.cwd === "string") cwd = d.context.cwd;
    } else if (e.type === "session.context_changed") {
      if (typeof d.cwd === "string") cwd = d.cwd;
      if (typeof d.branch === "string") branch = d.branch;
    } else if (e.type === "session.title_changed" && typeof d.title === "string") title = d.title;
    else if (e.type === "session.shutdown") {
      endedAt = at;
      for (const f of Array.isArray(d.codeChanges?.filesModified) ? d.codeChanges.filesModified : []) if (typeof f === "string") edits.push({ path: f, at, cwd });
    } else if (e.type === "tool.execution_start" && !e.ephemeral) for (const f of editedBy(d.toolName, d.arguments)) edits.push({ path: f, at, cwd });
  }
  if (!start) return null;
  return { id: start.id, version: start.version, startedAt: start.at ?? last, updatedAt: last ?? start.at, endedAt, cwd, branch, title, edits };
}

export const copilotRoot = ({ home }) => path.join(home, ".copilot", "session-state");

export class CopilotCliSessions extends NativeReader {
  constructor({ root, files = fs, clock } = {}) {
    super({ name: "copilot-sessions", clock });
    this.root = root;
    this.files = files;
    this.runs = new Map();
    this.cache = new FileCache(); // each event log as read
    this.rootState = "unread";
  }

  #files(now) {
    let entries;
    try {
      entries = this.files.readdirSync(this.root, { withFileTypes: true });
      this.rootState = "ok";
    } catch (err) {
      this.rootState = err?.code === "ENOENT" ? "missing" : "unreadable";
      return [];
    }
    const out = [];
    for (const e of entries) {
      const file = e.isDirectory() ? path.join(this.root, e.name, "events.jsonl") : e.isFile() && e.name.endsWith(".jsonl") ? path.join(this.root, e.name) : null;
      if (!file) continue;
      try {
        const st = this.files.statSync(file);
        if (now - st.mtimeMs <= DAY_MS && st.size <= FILE_MAX) out.push({ file, mtimeMs: st.mtimeMs, size: st.size });
      } catch {}
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
        const l = this.cache.get(f, () => {
          try {
            let version = null;
            return readEvents(this.files.readFileSync(f.file, "utf8"), { check: (rec) => (rec.type === "session.start" && typeof rec.data?.version === "number" && (version = String(rec.data.version)), r.check(rec, { version })) });
          } catch {
            return null;
          }
        });
        const id = l && sessionIdOf(l.id);
        if (!id) continue;
        const run = { id, rawId: l.id, cwd: l.cwd, branch: l.branch, title: l.title, createdAt: l.startedAt ?? f.mtimeMs, updatedAt: l.updatedAt ?? f.mtimeMs, endedAt: l.endedAt, edits: new Map() };
        for (const e of l.edits) {
          const at = Math.min(e.at ?? run.updatedAt, now);
          if (now - at > EDIT_KEEP_MS) continue;
          // Under the folder the session was in when it made the edit.
          const file = editFile(e.cwd ?? run.cwd, e.path);
          if (file) pushEdit(run.edits, file, at);
        }
        runs.set(id, run);
      }
      this.cache.keep(seen);
      this.runs = runs;
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  // A run as a session: the title Copilot CLI gave it, and its branch.
  session(r) {
    return runSession(r, { app: "copilot", title: r.title ?? titled("Copilot CLI", r.cwd), branch: r.branch ?? null });
  }

  status() {
    return { name: "copilot", root: this.root, state: this.rootState, sessions: this.runs.size };
  }
}
