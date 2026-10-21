// core/numbat.js in Rust (crates/xenonet-sources/src/numbat.rs): the numbat
// bridge, sessions read from the NDJSON event records numbat writes. Each
// run numbat records is a session on the map, "<app>:<id>", marked observed
// through numbat, its edits counted in other sessions' warnings. Rust tails
// the file (through `files` when a test gives its own), reads the records
// and keeps the runs; this keeps the block's API.
import fs from "node:fs";
import { NativeReader } from "../../../../core/native/reader.js";
import { native } from "../index.js";
import { ioOf, mapOfRuns } from "../native-readers.js";

export const SCHEMA_VERSIONS = new Set(["0.2.0", "0.3.0", "0.4.0", "0.5.0"]);
// Apps XenoNet reads itself, and numbat's names that aren't a run's app.
export const LEFT_OUT = new Set(["claude-code", "codex", "hermes", "grok", "unknown"]);

// A run's id as XenoNet's session ids take it: numbat's own when it fits,
// else a stable digest of it.
export const runId = (sid) => native.numbatRunId(String(sid));
export const sessionIdOf = (app, sid) => native.numbatSessionIdOf(String(app), String(sid));

// One NDJSON line as XenoNet reads it: { ok: true, event }, or { ok: false,
// why } for a line it passes over.
export const readRecord = (line) => native.numbatReadRecord(String(line));

export class NumbatSessions extends NativeReader {
  #core = new native.NumbatCore();
  #runs = null;

  // `file`: the NDJSON file numbat writes. `files`: fs, for the tests.
  constructor({ file, files = fs, clock } = {}) {
    super({ name: "numbat-sessions", observed: true, clock });
    this.file = file;
    this.files = files;
  }

  get offset() {
    return this.#core.offset ?? null;
  }
  get ino() {
    return this.#core.ino ?? null;
  }
  get partial() {
    return this.#core.partial;
  }
  get fileState() {
    return this.#core.fileState;
  }
  get skipped() {
    return this.#core.skipped;
  }
  set skipped(n) {
    this.#core.skipped = Number(n);
  }

  get runs() {
    return (this.#runs ??= mapOfRuns(this.#core.runs()));
  }

  // Reads what's new. Throws nothing: a missing file is said in status().
  read(now = this.clock.now()) {
    try {
      const failed = this.#core.read(this.file == null ? null : String(this.file), Number(now), ioOf(this.files, ["statSync", "openSync", "readSync", "closeSync"]));
      this.#runs = null;
      if (failed != null) throw new Error(failed);
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  // The runs as session/v1, for the map.
  sessions() {
    return this.#core.sessions();
  }

  ids() {
    return this.#core.ids();
  }

  // What the doctor shows: the file it reads, and whether it could.
  status() {
    return { name: "numbat", file: this.file, state: this.fileState, sessions: this.#core.size, skipped: this.skipped };
  }
}
