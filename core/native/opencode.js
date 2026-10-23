// The OpenCode reader (experimental): each OpenCode session is a session on
// the map, "opencode:<id>", read from OpenCode's own database, opencode.db.
// Its edits (the edit, write, multiedit and apply_patch tools' files) count
// in the warnings other sessions get before an edit. XenoNet can't message
// an OpenCode session, so the map shows it without a way to.
//
// It's off unless XENONET_READERS names opencode, and never in the demo.
// The database is opened read only for each read and closed after it.
// Schema: core/schema/vendor/anomaly/opencode.js.
import fs from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { pathStyle } from "../paths.js";
import { reader } from "../vendor.js";
import { cleanVersion } from "../drift.js";
import { APP } from "../apps.js";
import { patchPaths } from "../patch.js";
import { NativeReader, DAY_MS, EDIT_KEEP_MS, SESSIONS_MAX, editFile, pushEdit, runSession, titled } from "./reader.js";

export const SOURCE = "anomaly/opencode/db";
export const EDIT_TOOLS = [...APP.opencode.editTools];
export const PATCH_TOOLS = ["apply_patch", "patch"];
const SESSION_COLUMNS = ["id", "parent_id", "directory", "title", "version", "time_created", "time_updated", "time_archived"];
const NEEDED = { session: ["id", "directory", "time_created", "time_updated"], part: ["session_id", "time_created", "data"] };
const EDITS_MAX = 5000;

// OpenCode's ids as XenoNet's session ids take them, and back. A session's
// id here has letters, digits and dashes (core/schema/ids.js), and OpenCode
// makes ses_ and letters and digits. So an id of letters and digits in runs
// joined by single underscores takes a dash for each underscore (ses_4f… is
// ses-4f…), and never has two dashes together. Any other id is x-- and its
// UTF-8 bytes in hex, which always has. openCodeId reads either back, so
// delivery reaches the session the id came from, and numbat's runs of
// OpenCode take the same ids (core/numbat.js).
const PLAIN = /^[0-9A-Za-z]+(?:_[0-9A-Za-z]+)*$/;
export const localId = (id) => {
  const s = String(id);
  return PLAIN.test(s) ? s.replace(/_/g, "-") : `x--${Buffer.from(s, "utf8").toString("hex")}`;
};
export const sessionIdOf = (id) => `opencode:${localId(id)}`;

// XenoNet's id for an OpenCode session (opencode:…, or what follows it) as
// OpenCode's own. Hex that isn't UTF-8 reads as the plain form would.
export function openCodeId(id) {
  const s = String(id).replace(/^opencode:/, "");
  const hex = /^x--((?:[0-9a-f]{2})*)$/.exec(s);
  if (hex) {
    try {
      return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(Buffer.from(hex[1], "hex"));
    } catch {}
  }
  return s.replace(/-/g, "_");
}

// The files an apply_patch text names, from its headers only: those it
// adds, updates, deletes and moves to, less a header with no path.
export function patchFiles(text) {
  if (typeof text !== "string") return [];
  return patchPaths(text).filter(Boolean);
}

// The files edit tools changed since `since`, taken out by SQLite: a part's
// other fields never leave the database, apart from apply_patch's text,
// whose file headers are kept and the rest dropped here. Newest first, so
// a busy day can't crowd out the edits that matter: the last ones.
const editsSql = `
  SELECT session_id, time_created,
    json_extract(data, '$.tool') AS tool,
    CASE WHEN json_type(data, '$.state.input.filePath') = 'text' THEN json_extract(data, '$.state.input.filePath') END AS path,
    CASE WHEN json_extract(data, '$.tool') IN (${PATCH_TOOLS.map((t) => `'${t}'`).join(", ")}) AND json_type(data, '$.state.input.patchText') = 'text' THEN json_extract(data, '$.state.input.patchText') END AS patch
  FROM part
  WHERE time_created > ? AND json_valid(data) AND json_extract(data, '$.type') = 'tool'
    AND json_extract(data, '$.tool') IN (${[...EDIT_TOOLS, ...PATCH_TOOLS].map((t) => `'${t}'`).join(", ")})
    AND COALESCE(json_extract(data, '$.state.status'), '') <> 'error'
  ORDER BY time_created DESC LIMIT ${EDITS_MAX}`;

// What's in the database since `sinceMs` (edits since `editsSinceMs`):
// { state, version, sessions, edits }, edits oldest first.
export function readDb(file, { sinceMs = 0, editsSinceMs = sinceMs, open = (f) => new DatabaseSync(f, { readOnly: true }), files = fs } = {}) {
  const empty = { version: null, sessions: [], edits: [] };
  if (!files.existsSync(file)) return { state: "missing", ...empty };
  let db;
  try {
    db = open(file);
  } catch (err) {
    return { state: /permission|EACCES|EPERM/i.test(String(err?.message)) ? "denied" : "unreadable", ...empty };
  }
  const r = reader(SOURCE);
  try {
    const columns = (t) => new Set(db.prepare(`PRAGMA table_info(${t})`).all().map((c) => c.name));
    const has = { session: columns("session"), part: columns("part") };
    // The newest session's version, by rowid when time_updated is gone, so
    // the drift below is still recorded with a version.
    const version = has.session.has("version") ? cleanVersion(db.prepare(`SELECT version AS v FROM session ORDER BY ${has.session.has("time_updated") ? "time_updated" : "rowid"} DESC LIMIT 1`).get()?.v ?? null) : null;
    for (const [t, needed] of Object.entries(NEEDED)) {
      if (!has[t].size) r.missing(t, version);
      else for (const c of needed) if (!has[t].has(c)) r.missing(`${t}[].${c}`, version);
    }
    if (!NEEDED.session.every((c) => has.session.has(c))) return { state: "unreadable", ...empty, version };
    const cols = SESSION_COLUMNS.filter((c) => has.session.has(c));
    const session = db.prepare(`SELECT ${cols.join(", ")} FROM session WHERE time_updated > ? ORDER BY time_updated DESC LIMIT ${SESSIONS_MAX}`).all(sinceMs).map((x) => ({ ...x }));
    const raw = NEEDED.part.every((c) => has.part.has(c)) ? db.prepare(editsSql).all(editsSinceMs).reverse() : [];
    const edits = [];
    for (const e of raw) {
      if (e.path) edits.push({ session_id: e.session_id, time_created: e.time_created, tool: e.tool, path: e.path });
      for (const p of patchFiles(e.patch)) edits.push({ session_id: e.session_id, time_created: e.time_created, tool: e.tool, path: p });
    }
    const checked = r.check({ session, edits }, { version }) ?? {};
    return { state: "ok", version, sessions: checked.session ?? [], edits: checked.edits ?? [] };
  } catch {
    return { state: "unreadable", ...empty };
  } finally {
    try {
      db.close();
    } catch {}
  }
}

export class OpenCodeSessions extends NativeReader {
  constructor({ file, open, files = fs, clock } = {}) {
    super({ name: "opencode-sessions", clock });
    this.file = file;
    this.open = open;
    this.files = files;
    this.runs = new Map();
    this.fileState = "unread";
    this.version = null;
  }

  read(now = this.clock.now()) {
    try {
      const got = readDb(this.file, { sinceMs: now - DAY_MS, editsSinceMs: now - EDIT_KEEP_MS, files: this.files, ...(this.open ? { open: this.open } : {}) });
      this.fileState = got.state;
      this.version = got.version;
      // A read that failed once (the database busy mid-write, say) keeps
      // the last good one, so sessions don't flicker off the map; a
      // database that's gone or can't be opened drops them.
      if (got.state !== "ok") {
        if (got.state !== "unreadable") this.runs.clear();
        return this.ok(now), this;
      }
      const runs = new Map();
      for (const s of got.sessions) {
        if (typeof s.id !== "string") continue;
        const id = sessionIdOf(s.id);
        runs.set(id, {
          id,
          rawId: s.id,
          cwd: typeof s.directory === "string" ? s.directory : null,
          title: typeof s.title === "string" && s.title ? s.title : null,
          parentId: typeof s.parent_id === "string" ? sessionIdOf(s.parent_id) : null,
          createdAt: s.time_created,
          updatedAt: s.time_updated,
          archived: s.time_archived != null,
          edits: new Map(),
        });
      }
      for (const e of got.edits) {
        const run = runs.get(sessionIdOf(e.session_id));
        if (!run || typeof e.path !== "string" || now - e.time_created > EDIT_KEEP_MS) continue;
        const file = editFile(run.cwd, e.path);
        if (!file) continue;
        pushEdit(run.edits, file, Math.min(e.time_created, now));
        run.updatedAt = Math.max(run.updatedAt, e.time_created);
      }
      this.runs = runs;
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  // A run as a session: OpenCode's title for it, and its parent when it's
  // a subagent's. Its edits are read from OpenCode's own records, so the
  // engine doesn't take them as observed.
  session(r) {
    return runSession(r, { app: "opencode", title: r.title ?? titled("OpenCode", r.cwd), archived: r.archived, parentId: r.parentId });
  }

  status() {
    return { name: "opencode", file: this.file, state: this.fileState, version: this.version, sessions: this.runs.size };
  }
}

// Where OpenCode keeps its database: OPENCODE_DB (absolute, or relative to
// its data folder), else $XDG_DATA_HOME/opencode/opencode.db, else
// ~/.local/share/opencode/opencode.db, on every OS.
export function opencodeDbPath({ env = process.env, home }) {
  const dir = pathStyle(home).join(env.XDG_DATA_HOME || pathStyle(home).join(home, ".local", "share"), "opencode");
  const named = env.OPENCODE_DB;
  if (named && named !== ":memory:") return pathStyle(home).isAbsolute(named) ? named : pathStyle(home).join(dir, named);
  return pathStyle(home).join(dir, "opencode.db");
}
