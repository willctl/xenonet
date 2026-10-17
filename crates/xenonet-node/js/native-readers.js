// What the twins of core/native/'s readers and core/numbat.js share: the
// JS that Rust calls back during a read, made so it never throws into Rust.
// vendor.js's checks return FAILED in place of throwing; a test's own
// `files` and a node:sqlite handle a test opened answer [value], or
// { code, message } for what the call threw.
import fs from "node:fs";
import path from "node:path";

export const FAILED = Symbol("failed");

// A function that doesn't throw: what it throws is kept in box.err, and
// FAILED is returned in its place.
export function guarded(fn, box = {}) {
  return (...args) => {
    try {
      return fn(...args);
    } catch (err) {
      box.err = err;
      return FAILED;
    }
  };
}

// The calls a reader makes on `files`, for Rust to read through, or null
// when `files` is Node's own fs (Rust reads the disk itself then).
export function ioOf(files, names) {
  if (names.every((n) => files?.[n] === fs[n])) return null;
  const answer = (fn) => {
    try {
      return [fn()];
    } catch (err) {
      return { code: err?.code ?? null, message: String(err?.message ?? err) };
    }
  };
  return (op, p, offset, len) =>
    answer(() => {
      switch (op) {
        case "readdir":
          return files.readdirSync(p, { withFileTypes: true }).map((e) => [String(e.name), !!e.isDirectory(), !!e.isFile()]);
        case "stat": {
          const st = files.statSync(p);
          return { mtimeMs: st.mtimeMs, size: st.size, ino: st.ino ?? null };
        }
        case "text":
          return files.readFileSync(p, "utf8");
        case "read": {
          // As the JS read: one readSync into a zeroed buffer, handed over
          // as latin1, one character a byte.
          const buf = Buffer.alloc(len);
          const fd = files.openSync(p, "r");
          try {
            files.readSync(fd, buf, 0, len, offset);
          } finally {
            files.closeSync(fd);
          }
          return buf.toString("latin1");
        }
        case "exists":
          return !!files.existsSync(p);
      }
      throw new Error(`no such call: ${op}`);
    });
}

// A path argument as Node's path functions take it: a string, else the
// TypeError the function (path.join by default) throws.
export const pathArg = (v, fn = path.join) => (typeof v === "string" ? v : (fn(v), String(v)));

// A reader's runs, from Rust, as the Map the JS kept: id → run, its edits
// a Map of file → [{ ts, path }].
export const mapOfRuns = (list) => new Map(list.map((r) => [r.id, { ...r, edits: new Map(r.edits) }]));

// What the engine reads of a run, in the shape of a live session's: when
// it was last at work, its edits, and that it has no turn XenoNet can see.
export const liveOf = (r, observed) => (r ? { lastEventAt: r.updatedAt, endedAt: r.endedAt ?? null, edits: r.edits, observed, status: () => "idle", needsYou: () => null } : null);
