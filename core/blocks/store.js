// Stores keep what XenoNet remembers between runs, in a file with a version:
// a JSON file (JsonStore, here) or a SQLite database (SqliteStore, in
// sqlite.js, apart so what reads JSON never loads SQLite). Each knows its
// version, moves older data up to it a step at a time (see migrate.js),
// writes so a crash can't leave half a file, and never loses data it can't
// use.
//
// A JSON file:
//   - no file: a fresh document;
//   - not JSON, not an object, or a version it doesn't know: moved aside to
//     <name>.broken-<time>.json, and a fresh document; never replaced by an
//     empty one and written over;
//   - an older version: kept as it was in <name>.v<n>.json, then migrated
//     and saved;
//   - a newer version: read as it is and never written back, since this
//     XenoNet would write it in an older shape;
//   - a file it can't read, back up or move aside: a fresh or migrated
//     document in memory, and the file left alone (read only).
//   Changes are saved a moment later, several in one write, through a
//   temporary file renamed into place.
import fs from "node:fs";
import path from "node:path";
import { migrate } from "../migrate.js";
import { Block } from "./lifecycle.js";

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);
const stamp = (now) => new Date(now).toISOString().replace(/:/g, "-");

// A file without a version is version 1, from before there were versions.
export const versionOf = (data) => (data.version === undefined ? 1 : data.version);

// A name beside `file` that isn't taken: <base><ext>, then <base>-<time><ext>.
export function freeName(file, base, now, ext = ".json") {
  const dir = path.dirname(file);
  const first = path.join(dir, `${base}${ext}`);
  return fs.existsSync(first) ? path.join(dir, `${base}-${stamp(now)}${ext}`) : first;
}

// Reads a versioned JSON file: { doc, report }. The report says what was
// found and done: `version` (the file's), `migrated` ({ from, to }), `backup`
// and `broken` (files written or moved aside), `readOnly` (never write the
// file), `problem` (why, in words) and, when there are any, `repaired` (the
// parts `prepare` started afresh). `context` goes to the migrations.
//
// `prepare(doc)` fills in what a document should have before it's used, and
// may return the parts it had to start afresh (a list), such as one a hand
// edit left a string. A document it throws on isn't used: a fresh one is,
// and the file is left alone. A migration's copy of the file is written only
// once the migrated document is prepared, so one that can't be isn't
// copied again at every start.
export function readVersioned(file, { version: latest, migrations = [], versionOf: versionOfDoc = versionOf, context = {}, fresh = () => ({ version: latest }), prepare = null, now = Date.now() } = {}) {
  const name = path.basename(file);
  const base = name.replace(/\.json$/, "");
  const report = { file, version: latest, migrated: null, backup: null, broken: null, readOnly: false, problem: null };
  const prepared = (doc) => {
    const repaired = prepare?.(doc);
    return Array.isArray(repaired) ? repaired : [];
  };
  const freshly = (extra = {}) => {
    const doc = fresh();
    prepared(doc);
    return { doc, report: { ...report, ...extra } };
  };
  // The document to use, prepared, or a fresh one in its place.
  const use = (doc, extra) => {
    let repaired;
    try {
      repaired = prepared(doc);
    } catch {
      return freshly({ version: extra.version ?? report.version, readOnly: true, problem: `${name} has a part XenoNet can't use, so it leaves the file alone` });
    }
    return { doc, report: { ...report, ...extra, ...(repaired.length ? { repaired } : {}) } };
  };
  let text;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch (err) {
    if (err.code === "ENOENT") return freshly();
    return freshly({ readOnly: true, problem: `XenoNet couldn't read ${name} (${err.code ?? err.message})` });
  }

  let data;
  try {
    data = JSON.parse(text);
  } catch {}
  const version = isObject(data) ? versionOfDoc(data) : null;
  if (!isObject(data) || !Number.isInteger(version) || version < 1) {
    const why = !isObject(data) ? (data === undefined ? "wasn't valid JSON" : "wasn't a JSON object") : `had a version XenoNet doesn't know (${JSON.stringify(data.version)})`;
    const aside = path.join(path.dirname(file), `${base}.broken-${stamp(now)}.json`);
    try {
      fs.renameSync(file, aside);
      return freshly({ version: null, broken: aside, problem: `${name} ${why}, so XenoNet moved it aside and started afresh` });
    } catch (err) {
      return freshly({ version: null, readOnly: true, problem: `${name} ${why}, and XenoNet couldn't move it aside (${err.code ?? err.message}), so it leaves it alone` });
    }
  }

  if (version > latest) return use(data, { version, readOnly: true, problem: `${name} is from a newer XenoNet (version ${version}), so this one reads it but doesn't save changes to it` });
  if (version === latest) return use(data, {});

  // Older: moved up, then a copy first, as it was, so the move can be undone.
  let doc;
  try {
    doc = migrate(data, migrations, { from: version, to: latest, context }).doc;
  } catch (err) {
    return use(data, { version, readOnly: true, problem: `XenoNet couldn't move ${name} from version ${version} to ${latest} (${err.message}), so it leaves it alone` });
  }
  const moved = use(doc, { version, migrated: { from: version, to: latest } });
  if (moved.report.readOnly) return moved;
  const backup = freeName(file, `${base}.v${version}`, now);
  try {
    fs.writeFileSync(backup, text, { flag: "wx" });
  } catch (err) {
    return { doc, report: { ...moved.report, readOnly: true, problem: `XenoNet couldn't keep a copy of ${name} (${err.code ?? err.message}) before moving it to version ${latest}, so it leaves the file as it is` } };
  }
  return { doc, report: { ...moved.report, backup } };
}

export class JsonStore extends Block {
  // `file`: the path, or a function giving it (the folder can move while
  // XenoNet runs, in tests). `version`, `migrations`, `versionOf`, `fresh`:
  // as readVersioned takes them; `context()`: what the migrations get.
  // `prepare(doc)`: fills in what a loaded document should have, and says
  // which parts it started afresh (see readVersioned). `what`: how the log
  // names it ("its state").
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

  // The document, read once. One `prepare` throws on becomes a problem the
  // store reports (see readVersioned), never an exception for whoever asked.
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

  // What reading the file found and did (see readVersioned).
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

  // Writes what's unsaved now, through a temporary file so a crash can't
  // leave half of one. A write that fails (a full disk, a folder XenoNet can't
  // write to) is reported and tried again with the next change, or at exit.
  flush() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
    // A file XenoNet couldn't use, or one from a newer XenoNet, stays as it is.
    if (!this.unsaved || this.report?.readOnly) return;
    const file = this.file();
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const tmp = file + ".tmp";
      fs.writeFileSync(tmp, JSON.stringify(this.doc, null, 2));
      fs.renameSync(tmp, file);
      this.unsaved = false;
      this.count("saves");
      this.ok();
    } catch (err) {
      console.error(`[xenonet] couldn't save ${this.what} to ${file}: ${err.message}`);
      this.problem(err);
    }
  }

  onStop() {
    this.flush();
  }
}
