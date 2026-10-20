// The experimental native readers (core/native) as the map uses them, and
// what they share. Each reads an app's own records, and each run it finds
// is a session on the map with its edits. numbat's bridge (core/numbat.js)
// keeps the same contract for the runs it observes.
//
// A reader is a block (a gatherer) with:
//
//   read(now)    reads what's new; a problem is the block's, never thrown
//   sessions()   its runs as session/v1
//   live(id)     what the engine reads of a run, in the shape of a live
//                session's, or null
//   ids()        its runs' ids
//   status()     what the doctor shows: its name, what it reads, `state`
//                (ok, missing, unreadable or denied; unread before its
//                first read), `sessions` (how many runs it has), and
//                anything else it has to say
//
// NativeReader gives the bodies that are the same for every reader:
// sessions() from each run's session(r), live() and ids(), over the
// reader's `runs` (id → run, each with its updatedAt, endedAt when it has
// one, and edits: file → its last few edits). core/native/index.js says
// which reader reads which app. The Rust has the same contract:
// xenonet-sources' NativeReader.
import { Block } from "../blocks/lifecycle.js";
import { pathStyle } from "../paths.js";

// A run stays on the map for a day after it was last at work, its edits
// count for half an hour, and a reader keeps its newest runs up to
// SESSIONS_MAX.
export const DAY_MS = 24 * 3600e3;
export const EDIT_KEEP_MS = 30 * 60e3;
export const SESSIONS_MAX = 500;
// The edits a run keeps of each file, the newest last.
const EDITS_PER_FILE = 8;

// An app's run id as XenoNet's session ids take it: a letter or digit,
// then 6 to 127 letters, digits and dashes.
const ID = /^[0-9A-Za-z][0-9A-Za-z-]{6,127}$/;
export const fitsId = (id) => ID.test(id);

// A time a record writes as a string, in ms, or null.
export const ms = (v) => {
  const t = typeof v === "string" ? Date.parse(v) : NaN;
  return Number.isFinite(t) ? t : null;
};

// Where an edit's file is: its path when that's absolute, else under
// `base`, either way in the style of `base` (of the path, with no base);
// null for a relative path with no base.
export function editFile(base, p) {
  const ps = pathStyle(base ?? p);
  return ps.isAbsolute(p) ? ps.resolve(p) : base ? ps.resolve(base, p) : null;
}

// An edit added to a run's edits, which keep each file's last few.
export function pushEdit(edits, file, ts) {
  const list = edits.get(file) ?? [];
  list.push({ ts, path: file });
  if (list.length > EDITS_PER_FILE) list.shift();
  edits.set(file, list);
}

// Drops the runs last at work more than a day ago, and the files a run
// last edited more than EDIT_KEEP_MS ago.
export function pruneRuns(runs, now) {
  for (const [id, r] of runs) {
    if (now - r.updatedAt > DAY_MS) runs.delete(id);
    else for (const [p, list] of r.edits) if (now - list[list.length - 1].ts > EDIT_KEEP_MS) r.edits.delete(p);
  }
}

// Drops the oldest runs past SESSIONS_MAX.
export function capRuns(runs) {
  if (runs.size > SESSIONS_MAX) for (const id of [...runs.values()].sort((a, b) => a.updatedAt - b.updatedAt).slice(0, runs.size - SESSIONS_MAX).map((r) => r.id)) runs.delete(id);
}

// What a reader made of each file it read, kept until the file changes:
// by the mtime and size it was read at.
export class FileCache {
  #files = new Map(); // file → { mtimeMs, size, value }

  // What `read` gives for `f` ({ file, mtimeMs, size }), read again only
  // when the file changed since.
  get(f, read) {
    let hit = this.#files.get(f.file);
    if (!hit || hit.mtimeMs !== f.mtimeMs || hit.size !== f.size) {
      hit = { mtimeMs: f.mtimeMs, size: f.size, value: read() };
      this.#files.set(f.file, hit);
    }
    return hit.value;
  }

  // Forgets every file but these.
  keep(files) {
    for (const k of this.#files.keys()) if (!files.has(k)) this.#files.delete(k);
  }
}

// "<name> in <its folder's name>", or the name alone with no folder.
export const titled = (name, cwd) => `${name}${cwd ? ` in ${pathStyle(cwd).basename(cwd)}` : ""}`;

// A run as session/v1, in the order every reader writes one (as the Rust's
// RunSession does), then how XenoNet knows of it: by default from an
// experimental reader of the app's own records.
export const runSession = (r, { app, title, archived = false, model = null, branch = null, parentId = null, via = { reader: "experimental" } }) => ({
  id: r.id,
  app,
  rawId: r.rawId,
  title,
  preview: null,
  cwd: r.cwd,
  createdAt: r.createdAt,
  updatedAt: r.updatedAt,
  archived,
  model,
  branch,
  parentId,
  parentKind: parentId ? "subagent" : null,
  ...via,
});

// What the engine reads of a run, in the shape of a live session's: when
// it was last at work, its edits, whether XenoNet inferred them (observed),
// and that it has no turn XenoNet can see.
const liveOf = (r, observed) => (r ? { lastEventAt: r.updatedAt, endedAt: r.endedAt ?? null, edits: r.edits, observed, status: () => "idle", needsYou: () => null } : null);

export class NativeReader extends Block {
  #observed;

  // `name`: its block's. `observed`: its runs are observed (numbat's),
  // rather than read from the app's own records.
  constructor({ name, observed = false, clock } = {}) {
    super({ name, kind: "gatherer", ...(clock ? { clock } : {}) });
    this.#observed = observed;
  }

  sessions() {
    return [...this.runs.values()].map((r) => this.session(r));
  }

  live(id) {
    return liveOf(this.runs.get(id), this.#observed);
  }

  ids() {
    return [...this.runs.keys()];
  }
}
