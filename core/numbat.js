// The numbat bridge: an optional source of sessions read from the NDJSON
// event records numbat writes (numbat, by Perplexity, records what coding
// agents do through their hooks and session files;
// https://github.com/perplexityai/numbat). Each run numbat records, one per
// numbat session id, is a session on the map: "<app>:<id>", such as
// gemini-cli:…, with app the agent that ran it, marked observed through
// numbat. Its edits count in the warnings other sessions get before an edit,
// marked observed, since XenoNet inferred them.
//
// It's off unless you name the file numbat writes (XENONET_NUMBAT_FILE),
// the one its hooks write with `--emit events --output file`. XenoNet never
// installs or configures numbat, and never writes to its file.
//
// From each event record (numbat's published record schema, versions 0.2.0
// to 0.5.0), XenoNet keeps only:
//
//   source_agent       which app ran it
//   session_id         which run, and parent_session_id its parent's
//   event_type         file.write and file.delete are edits, session.end
//                      ends it, anything else is activity
//   file_path          an edited file, joined to project_path when relative
//   project_path       where it ran
//   git_branch, model  as the record has them
//   timestamp          when
//
// Never the endpoint (host, user), commands, prompts, previews, tool input
// or results. Claude Code and Codex are left out, since XenoNet reads their
// sessions itself, and records that name no session or agent: nothing
// counts twice.
import crypto from "node:crypto";
import fs from "node:fs";
import { Block } from "./blocks/lifecycle.js";
import { pathStyle } from "./paths.js";
import { SESSION_APPS } from "./schema/ids.js";

export const SCHEMA_VERSIONS = new Set(["0.2.0", "0.3.0", "0.4.0", "0.5.0"]);
// Apps XenoNet reads itself, and numbat's names that aren't a run's app.
export const LEFT_OUT = new Set(["claude-code", "codex", "hermes", "grok", "unknown"]);
const EDITS = new Set(["file.write", "file.delete"]);
const EDIT_KEEP_MS = 30 * 60e3;
const DAY_MS = 24 * 3600e3;
const SESSIONS_MAX = 500;
const START_BACK = 1 << 20; // a first read takes at most the last MiB
const READ_MAX = 4 << 20; // and each read at most 4 MiB
const LINE_MAX = 1 << 20;

const str = (v) => (typeof v === "string" && v.length ? v : null);
const abs = (p) => !!p && pathStyle(p).isAbsolute(p);
const RUN_ID = /^[0-9A-Za-z][0-9A-Za-z-]{6,}$/;
// A run's id as XenoNet's session ids take it: numbat's own when it fits,
// else a stable digest of it.
export const runId = (sid) => (RUN_ID.test(sid) && sid.length <= 128 ? sid : `n${crypto.createHash("sha256").update(sid).digest("hex").slice(0, 31)}`);
export const sessionIdOf = (app, sid) => `${app}:${runId(sid)}`;

// One NDJSON line as XenoNet reads it: { ok: true, event }, or { ok: false,
// why } for a line it passes over.
export function readRecord(line) {
  let r;
  try {
    r = JSON.parse(line);
  } catch {
    return { ok: false, why: "malformed" };
  }
  if (!r || typeof r !== "object" || Array.isArray(r)) return { ok: false, why: "malformed" };
  if (r.record_type !== "event") return { ok: false, why: "not-an-event" };
  if (!SCHEMA_VERSIONS.has(r.schema_version)) return { ok: false, why: "unsupported-version" };
  const app = str(r.source_agent);
  const type = str(r.event_type);
  if (!app || !type) return { ok: false, why: "malformed" };
  if (LEFT_OUT.has(app) || !Object.hasOwn(SESSION_APPS, app)) return { ok: false, why: "left-out" };
  const session = str(r.session_id);
  if (!session) return { ok: false, why: "no-session" };
  const at = Date.parse(r.timestamp ?? "");
  const project = str(r.project_path);
  let file = EDITS.has(type) ? str(r.file_path) : null;
  if (file && !abs(file)) file = abs(project) ? pathStyle(project).join(project, file) : null;
  if (file) file = pathStyle(file).resolve(file);
  return {
    ok: true,
    event: {
      app,
      type,
      session,
      parent: str(r.parent_session_id),
      at: Number.isFinite(at) ? at : null,
      project: abs(project) ? project : null,
      file,
      branch: str(r.git_branch),
      model: str(r.model),
    },
  };
}

export class NumbatSessions extends Block {
  // `file`: the NDJSON file numbat writes. `files`: fs, for the tests.
  constructor({ file, files = fs, clock } = {}) {
    super({ name: "numbat-sessions", kind: "gatherer", ...(clock ? { clock } : {}) });
    this.file = file;
    this.files = files;
    this.offset = null;
    this.ino = null;
    this.partial = "";
    this.runs = new Map(); // session id → its run
    this.fileState = "unread";
    this.skipped = 0;
  }

  // The new whole lines since the last read.
  #lines() {
    let st;
    try {
      st = this.files.statSync(this.file);
    } catch {
      this.fileState = "missing";
      return [];
    }
    let skipFirst = false;
    if (this.offset == null || st.size < this.offset || (this.ino != null && st.ino !== this.ino)) {
      const from = this.offset == null ? Math.max(0, st.size - START_BACK) : 0;
      skipFirst = from > 0;
      this.offset = from;
      this.partial = "";
    }
    this.ino = st.ino;
    this.fileState = "ok";
    const len = Math.min(st.size - this.offset, READ_MAX);
    if (len <= 0) return [];
    const buf = Buffer.alloc(len);
    const fd = this.files.openSync(this.file, "r");
    try {
      this.files.readSync(fd, buf, 0, len, this.offset);
    } finally {
      this.files.closeSync(fd);
    }
    this.offset += len;
    const lines = (this.partial + buf.toString("utf8")).split("\n");
    this.partial = lines.pop() ?? "";
    if (this.partial.length > LINE_MAX) this.partial = "";
    if (skipFirst) lines.shift();
    return lines.filter((l) => l.trim() && l.length <= LINE_MAX);
  }

  // Reads what's new. Throws nothing: a missing file is said in status().
  read(now = this.clock.now()) {
    try {
      for (const line of this.#lines()) {
        const r = readRecord(line);
        if (r.ok) this.#take(r.event, now);
        else this.skipped++;
      }
      this.#prune(now);
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  #take(e, now) {
    const when = Math.min(e.at ?? now, now);
    if (now - when > DAY_MS) return;
    const id = sessionIdOf(e.app, e.session);
    let run = this.runs.get(id);
    if (!run) {
      run = { id, app: e.app, rawId: e.session, cwd: null, branch: null, model: null, parentId: null, createdAt: when, updatedAt: when, endedAt: null, edits: new Map() };
      this.runs.set(id, run);
    }
    run.createdAt = Math.min(run.createdAt, when);
    run.updatedAt = Math.max(run.updatedAt, when);
    if (e.project && !run.cwd) run.cwd = e.project;
    if (e.branch) run.branch = e.branch;
    if (e.model) run.model = e.model;
    if (e.parent && e.parent !== e.session) run.parentId = sessionIdOf(e.app, e.parent);
    if (e.type === "session.end") run.endedAt = when;
    else if (run.endedAt != null && when > run.endedAt) run.endedAt = null;
    if (e.file) {
      const list = run.edits.get(e.file) ?? [];
      list.push({ ts: when, path: e.file });
      if (list.length > 8) list.shift();
      run.edits.set(e.file, list);
    }
  }

  #prune(now) {
    for (const [id, r] of this.runs) {
      if (now - r.updatedAt > DAY_MS) this.runs.delete(id);
      else for (const [p, list] of r.edits) if (now - list[list.length - 1].ts > EDIT_KEEP_MS) r.edits.delete(p);
    }
    if (this.runs.size > SESSIONS_MAX) for (const id of [...this.runs.values()].sort((a, b) => a.updatedAt - b.updatedAt).slice(0, this.runs.size - SESSIONS_MAX).map((r) => r.id)) this.runs.delete(id);
  }

  // The runs as session/v1, for the map.
  sessions() {
    return [...this.runs.values()].map((r) => ({
      id: r.id,
      app: r.app,
      rawId: r.rawId,
      title: `${SESSION_APPS[r.app]}${r.cwd ? ` in ${pathStyle(r.cwd).basename(r.cwd)}` : ""}`,
      preview: null,
      cwd: r.cwd,
      createdAt: r.createdAt,
      updatedAt: r.updatedAt,
      archived: false,
      model: r.model,
      branch: r.branch,
      parentId: r.parentId,
      parentKind: r.parentId ? "subagent" : null,
      observed: "numbat",
    }));
  }

  // What the engine reads of a run, in the shape of a live session's:
  // when it was last at work, its edits, and that it has no turn XenoNet
  // can see.
  live(id) {
    const r = this.runs.get(id);
    if (!r) return null;
    return { lastEventAt: r.updatedAt, endedAt: r.endedAt, edits: r.edits, observed: true, status: () => "idle", needsYou: () => null };
  }

  ids() {
    return [...this.runs.keys()];
  }

  // What the doctor shows: the file it reads, and whether it could.
  status() {
    return { name: "numbat", file: this.file, state: this.fileState, sessions: this.runs.size, skipped: this.skipped };
  }
}
