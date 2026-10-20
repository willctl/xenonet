// core/native/opencode.js in Rust (crates/xenonet-sources/src/native/
// opencode.rs): each OpenCode session in its opencode.db is a session on
// the map, "opencode:<id>", with its edit tools' files as its edits. Rust
// opens the database read only (rusqlite, SQLite bundled), asks what it
// asks, and keeps the runs. Given its own `open` (a test's node:sqlite
// handle, or a stand-in), the twin runs Rust's statements on that handle
// instead. vendor.js's checks and drift stay vendor.js's, called back from
// Rust.
import fs from "node:fs";
import { Block } from "../blocks/lifecycle.js";
import { reader } from "../vendor.js";
import { native } from "../../index.js";
import { APP } from "../../../../../core/apps.js";
import { FAILED, guarded, liveOf, mapOfRuns, pathArg } from "../../native-readers.js";

export const SOURCE = "anomaly/opencode/db";
export const EDIT_TOOLS = [...APP.opencode.editTools];
export const PATCH_TOOLS = ["apply_patch", "patch"];

// OpenCode's ids (ses_…) as XenoNet's session ids take them.
export const localId = (id) => native.opencodeLocalId(String(id));
export const sessionIdOf = (id) => native.opencodeSessionIdOf(String(id));

// The files an apply_patch text names, from its headers only.
export const patchFiles = (text) => native.opencodePatchFiles(text);

// vendor.js's reader for the database, got when Rust says (after the
// database opened), as readDb got it.
function vendorOf() {
  const box = {};
  let r = null;
  return {
    box,
    ready: guarded(() => {
      r = reader(SOURCE);
    }, box),
    missing: guarded((p, version) => r.missing(p, version), box),
    check: guarded((rec, version) => r.check(rec, { version }), box),
  };
}

// A handle `open` gives, for Rust's statements: query(kind, sql, param).
function queryOf(open) {
  let db = null;
  return (kind, sql, param) => {
    try {
      if (kind === "open") return [((db = open(sql)), null)];
      if (kind === "close") return [(db.close(), null)];
      const st = db.prepare(sql);
      if (kind === "get") return [st.get() ?? null];
      return [param == null ? st.all() : st.all(param)];
    } catch (err) {
      return { message: String(err?.message) };
    }
  };
}

// What's in the database since `sinceMs` (edits since `editsSinceMs`):
// { state, version, sessions, edits }, edits oldest first.
export function readDb(file, { sinceMs = 0, editsSinceMs = sinceMs, open, files = fs } = {}) {
  const v = vendorOf();
  const got = native.opencodeReadDb(!!files.existsSync(file), String(file), Number(sinceMs), Number(editsSinceMs), open === undefined ? null : queryOf(open), v.ready, v.missing, v.check, FAILED);
  if (got === true) throw v.box.err;
  return got;
}

export class OpenCodeSessions extends Block {
  #core = new native.OpenCodeCore();
  #runs = null;

  constructor({ file, open, files = fs, clock } = {}) {
    super({ name: "opencode-sessions", kind: "gatherer", ...(clock ? { clock } : {}) });
    this.file = file;
    this.open = open;
    this.files = files;
  }

  get runs() {
    return (this.#runs ??= mapOfRuns(this.#core.runs()));
  }

  get fileState() {
    return this.#core.fileState;
  }

  get version() {
    return this.#core.version ?? null;
  }

  read(now = this.clock.now()) {
    try {
      const v = vendorOf();
      const out = this.#core.read(Number(now), !!this.files.existsSync(this.file), String(this.file), this.open ? queryOf(this.open) : null, v.ready, v.missing, v.check, FAILED);
      this.#runs = null;
      if (out === true) throw v.box.err;
      if (typeof out === "string") throw new TypeError(out);
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  sessions() {
    return this.#core.sessions();
  }

  // What the engine reads of a session: its edits, read from OpenCode's own
  // records (so not observed), and that XenoNet can't send to it.
  live(id) {
    return liveOf(this.runs.get(id), false);
  }

  ids() {
    return this.#core.ids();
  }

  status() {
    return { name: "opencode", file: this.file, state: this.fileState, version: this.version, sessions: this.#core.size };
  }
}

// Where OpenCode keeps its database: OPENCODE_DB (absolute, or relative to
// its data folder), else $XDG_DATA_HOME/opencode/opencode.db, else
// ~/.local/share/opencode/opencode.db, on every OS.
export function opencodeDbPath({ env = process.env, home }) {
  const xdg = env.XDG_DATA_HOME;
  const named = env.OPENCODE_DB;
  // Home is joined only without XDG_DATA_HOME; with it, only its style counts.
  return native.opencodeDbPath(xdg ? String(home ?? "") : pathArg(home), xdg ? pathArg(xdg) : null, named && named !== ":memory:" ? pathArg(named) : null);
}
