// core/search.js in Rust (crates/xenonet-state/src/search.rs): full-text
// search across both apps' transcripts, in a local SQLite FTS5 index
// (search.sqlite in the cache dir; see config.js). Indexing is incremental:
// appended lines are read from the last offset, and Codex's compressed
// .jsonl.zst archives are decompressed once.
//
// The index is Rust's own connection to the same file, with the same
// schema and version. Rust says what to read and decides the rest; the
// files are read here (an archive is unpacked here), and each line goes
// through the transcripts' normalizer (lines.js) on its way in. `db` is a
// handle to Rust's connection that reads as node:sqlite's does.
import fsp from "node:fs/promises";
import zlib from "node:zlib";
import { native } from "../index.js";
import { ClaudeLine, CodexLine, linesOf } from "./lines.js";

// One searchable record per human or agent message, from a line:
// normalized (see lines.js), or as the transcript has it.
export function docsFromLine(line, app, { userEvents = false } = {}) {
  const own = app === "claude" ? line instanceof ClaudeLine : line instanceof CodexLine;
  return native.searchDocsFromLine(own ? line.raw : line, String(app), !!userEvents);
}

// A transcript's text: a compressed archive's decompressed.
async function readWhole(file) {
  const buf = await fsp.readFile(file);
  return (file.endsWith(".zst") ? zlib.zstdDecompressSync(buf) : buf).toString("utf8");
}

// What was added to a file from `offset` to `size`.
async function readFrom(file, offset, size) {
  const fh = await fsp.open(file, "r");
  try {
    const buf = Buffer.alloc(size - offset);
    await fh.read(buf, 0, buf.length, offset);
    return buf.toString("utf8");
  } finally {
    await fh.close();
  }
}

// FTS5 query from free text: every word must match, as a prefix.
export const ftsQuery = (q) => native.searchFtsQuery(String(q ?? ""));

// node:sqlite's rows have no prototype.
const bare = (row) => Object.assign(Object.create(null), row);

// A statement on the index's handle, as node:sqlite's StatementSync runs one.
class Statement {
  #core;
  #generation;
  #sql;
  constructor(core, generation, sql) {
    this.#core = core;
    this.#generation = generation;
    this.#sql = sql;
  }
  get(...params) {
    const [row] = this.#core.query(this.#generation, this.#sql, params, false);
    return row === undefined ? undefined : bare(row);
  }
  all(...params) {
    return this.#core.query(this.#generation, this.#sql, params, true).map(bare);
  }
  run(...params) {
    return this.#core.runSql(this.#generation, this.#sql, params);
  }
}

// The index's handle, as node:sqlite's DatabaseSync reads one: it stays
// with the connection it was made for, and is closed with it.
class Database {
  #core;
  #generation;
  constructor(core, generation) {
    this.#core = core;
    this.#generation = generation;
  }
  prepare(sql) {
    this.#core.check(this.#generation, String(sql));
    return new Statement(this.#core, this.#generation, String(sql));
  }
  exec(sql) {
    this.#core.exec(this.#generation, String(sql));
  }
  close() {
    this.#core.closeHandle(this.#generation);
  }
}

export class TranscriptSearch {
  #core;

  constructor({ dbFile, claudeRoot, codexRoots }) {
    this.dbFile = dbFile;
    this.#core = new native.SearchCore(typeof dbFile === "string" ? dbFile : null, claudeRoot ? String(claudeRoot) : null, (codexRoots ?? []).map((d) => (typeof d === "string" ? d : null)));
    this.roots = this.#core.roots;
    this.status = { files: 0, docs: 0, indexing: false, lastIndexAt: 0, ms: 0 };
  }

  open() {
    if (this.db) return this.db;
    this.#core.connect();
    this.db = new Database(this.#core, this.#core.generation);
    this.#core.prepare();
    return this.db;
  }

  sessionOf(file, app) {
    return native.searchSessionOf(String(file), String(app)) ?? null;
  }

  // Indexes a transcript's lines, through its app's normalizer (lines.js);
  // lines that aren't JSON are left out. `userEvents`: see docsFromLine.
  indexText(file, app, sessionId, text, { userEvents = false } = {}) {
    return this.#index(file, app, sessionId, native.searchLines(String(text)), !!userEvents);
  }

  #index(file, app, sessionId, texts, userEvents) {
    const run = new native.SearchRun(String(file), String(app), String(sessionId), userEvents);
    const lines = linesOf(app);
    const where = { file };
    for (const text of texts) {
      const o = lines.line(text, where);
      if (o) this.#core.add(run, o.raw);
    }
    return run.n;
  }

  async indexFile(file, app) {
    const plan = this.#core.plan(String(file), String(app));
    if (!plan) return 0;
    const text = plan.whole ? await readWhole(file) : await readFrom(file, plan.offset, plan.size);
    // A Codex rollout found to have a completed user message is read whole.
    let lines = this.#core.begin(plan, text);
    if (lines === null) lines = this.#core.begin(plan, await readWhole(file));
    try {
      const n = this.#index(file, app, plan.sessionId, lines, plan.userEvents);
      this.#core.commit(plan);
      return n;
    } catch (e) {
      this.#core.rollback();
      throw e;
    }
  }

  async index() {
    if (this.status.indexing) return this.status;
    this.open();
    this.status.indexing = true;
    const t0 = Date.now();
    try {
      for (const r of this.roots) {
        for (const f of native.searchListFiles(String(r.dir))) {
          try {
            this.status.docs += await this.indexFile(f, r.app);
          } catch {
            this.status.errors = (this.status.errors ?? 0) + 1;
          }
          await new Promise((res) => setImmediate(res));
        }
      }
      [this.status.files, this.status.docs] = this.#core.counts();
    } finally {
      this.status.indexing = false;
      this.status.lastIndexAt = Date.now();
      this.status.ms = Date.now() - t0;
    }
    return this.status;
  }

  search(q, limit = 30) {
    this.open();
    return this.#core.search(String(q ?? ""), Number(limit));
  }

  close() {
    this.db?.close();
    this.db = null;
  }
}
