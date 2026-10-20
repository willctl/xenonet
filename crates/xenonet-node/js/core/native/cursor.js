// core/native/cursor.js in Rust (crates/xenonet-sources/src/native/
// cursor.rs): each Cursor agent conversation is a session on the map,
// "cursor:<conversation id>", from what XenoNet's Cursor hook writes, with
// Cursor's afterFileEdit files as its edits. Rust reads the file's tail
// (through `files` when a test gives its own) and keeps the runs; the
// vendor check stays vendor.js's, called back from Rust.
import fs from "node:fs";
import path from "node:path";
import { NativeReader } from "../../../../../core/native/reader.js";
import { reader } from "../vendor.js";
import { native } from "../../index.js";
import { FAILED, guarded, ioOf, mapOfRuns, pathArg } from "../../native-readers.js";

export const SOURCE = "anysphere/cursor/hook-input";

export const sessionIdOf = (id) => (typeof id === "string" ? (native.cursorSessionIdOf(id) ?? null) : null);

export class CursorSessions extends NativeReader {
  #core = new native.CursorCore();
  #runs = null;

  constructor({ file, files = fs, clock } = {}) {
    super({ name: "cursor-sessions", clock });
    this.file = file;
    this.files = files;
  }

  get runs() {
    return (this.#runs ??= mapOfRuns(this.#core.runs()));
  }

  get fileState() {
    return this.#core.fileState;
  }

  // The size and mtime of the file when it was last parsed.
  get seen() {
    return this.#core.seen ?? null;
  }

  read(now = this.clock.now()) {
    try {
      // The reader is got when a record is first checked, as JS got it to
      // parse the file.
      const box = {};
      let r = null;
      const check = guarded((rec, version) => (r ??= reader(SOURCE)).check(rec, { version }), box);
      const threw = this.#core.read(this.file == null ? null : String(this.file), Number(now), check, FAILED, ioOf(this.files, ["statSync", "openSync", "readSync", "closeSync"]));
      this.#runs = null;
      if (threw) throw box.err;
      this.ok(now);
    } catch (err) {
      this.problem(err, now);
    }
    return this;
  }

  sessions() {
    return this.#core.sessions();
  }

  ids() {
    return this.#core.ids();
  }

  status() {
    return { name: "cursor", file: this.file, state: this.fileState, sessions: this.#core.size };
  }
}

// The file the hook writes: XENONET_CURSOR_EVENTS, else cursor-hooks.ndjson
// in XenoNet's state folder.
export const cursorEventsFile = ({ env = process.env, stateDir }) => {
  const named = env.XENONET_CURSOR_EVENTS;
  return native.cursorEventsFile(named ? pathArg(named, path.resolve) : null, named ? "" : pathArg(stateDir));
};
