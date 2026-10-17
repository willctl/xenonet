// core/blocks/store.js in Rust (crates/xenonet-blocks/src/store.rs): a JSON
// file with a version. Rust reads the file and decides what to do with it
// (a fresh document, one moved aside, one read only, one moved up and its
// copy kept), says so in the report, and writes the file through a
// temporary one. The document, its migrations (core/migrate.js) and what
// fills it in stay JS, and so do the store's timer and its health.
import path from "node:path";
import { migrate } from "../../../../../core/migrate.js";
import { native } from "../../index.js";
import { Block } from "./lifecycle.js";

const FAILED = Symbol("failed");

// A file without a version is version 1, from before there were versions.
export const versionOf = (data) => (data.version === undefined ? 1 : data.version);

// A name beside `file` that isn't taken: <base><ext>, then <base>-<time><ext>.
export const freeName = (file, base, now, ext = ".json") => native.freeName(String(file), String(base), Number(now), String(ext));

// Reads a versioned JSON file: { doc, report } (see the Rust for what the
// report says). `prepare(doc)` fills in what a document should have, and
// may return the parts it had to start afresh.
export function readVersioned(file, { version: latest, migrations = [], versionOf: versionOfDoc = versionOf, context = {}, fresh = () => ({ version: latest }), prepare = null, now = Date.now() } = {}) {
  let failure = null;
  let escaped = null;
  // The caller's code that throws stops the read, as in JS.
  const own = (fn) => (...args) => {
    try {
      return fn(...args);
    } catch (err) {
      escaped ??= { err };
      throw err;
    }
  };
  const prepared = (doc) => {
    const repaired = prepare?.(doc);
    return Array.isArray(repaired) ? repaired : [];
  };
  try {
    return native.readVersioned(
      String(file),
      latest,
      now,
      own(() => fresh()),
      own((doc) => versionOfDoc(doc)),
      (doc, from, to) => {
        try {
          return migrate(doc, migrations, { from, to, context }).doc;
        } catch (err) {
          failure = err;
          return FAILED;
        }
      },
      (doc) => {
        try {
          return prepared(doc);
        } catch {
          return FAILED;
        }
      },
      own((doc) => void prepared(doc)),
      FAILED,
      () => String(failure?.message ?? failure),
    );
  } catch (err) {
    throw escaped ? escaped.err : err;
  }
}

export class JsonStore extends Block {
  // `file`: the path, or a function giving it. `version`, `migrations`,
  // `versionOf`, `fresh`: as readVersioned takes them; `context()`: what
  // the migrations get. `prepare(doc)`: fills in what a loaded document
  // should have. `what`: how the log names it ("its state").
  constructor({ name, file, version, migrations = [], versionOf: versionOfDoc = versionOf, context = () => ({}), fresh, prepare = null, debounceMs = 200, what = `its ${name}`, clock } = {}) {
    super({ name, kind: "store", clock });
    this.file = typeof file === "function" ? file : () => file;
    this.options = { version, migrations, versionOf: versionOfDoc, fresh: fresh ?? (() => ({ version })) };
    this.context = context;
    this.prepare = prepare;
    this.debounceMs = debounceMs;
    this.what = what;
    this.doc = null;
    this.report = null;
    this.timer = null;
    this.unsaved = false;
  }

  // The document, read once.
  load() {
    if (this.doc) return this.doc;
    ({ doc: this.doc, report: this.report } = readVersioned(this.file(), { ...this.options, prepare: this.prepare, context: this.context(), now: this.clock.now() }));
    if (this.report.repaired) console.error(`[xenonet] ${path.basename(this.file())}: ${this.report.repaired.join(", ")} ${this.report.repaired.length === 1 ? "wasn't an object" : "weren't objects"}, so XenoNet started afresh there`);
    this.count("loads");
    if (this.report.problem) this.problem(this.report.problem);
    else this.ok();
    // Moved up from an older version: saved as this one.
    if (this.report.migrated && !this.report.readOnly) this.save();
    return this.doc;
  }

  loadReport() {
    this.load();
    return this.report;
  }

  // Saves a moment later, with whatever else changes meanwhile.
  save() {
    this.unsaved = true;
    this.clock.clearTimeout(this.timer);
    this.timer = this.clock.setTimeout(() => this.flush(), this.debounceMs);
  }

  // Writes what's unsaved now. A write that fails is reported and tried
  // again with the next change, or at exit.
  flush() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
    if (!this.unsaved || this.report?.readOnly) return;
    const file = this.file();
    const err = native.writeJson(String(file), this.doc, 2);
    if (!err) {
      this.unsaved = false;
      this.count("saves");
      this.ok();
    } else {
      console.error(`[xenonet] couldn't save ${this.what} to ${file}: ${err.message}`);
      this.problem(err.message);
    }
  }

  onStop() {
    this.flush();
  }
}
