// Full-text search across the apps' transcripts, in a local SQLite FTS5 index
// (search.sqlite in the cache dir; see config.js). Indexing is incremental:
// appended lines are read from the last offset, and Codex's compressed
// .jsonl.zst archives are decompressed once.
//
// The index has a version (PRAGMA user_version). 1: the files table says
// which Codex rollouts have a completed user message, since one with none
// at all has its prompts in its user_message events (see lines.js); an
// index from before is made again for Codex's rollouts. 2: a Grok Build
// session's message is one record of all its chunks, and the files table
// keeps a run of chunks left open where a read ended (`run`); an index
// from before is made again for Grok Build's sessions.
import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";
import zlib from "node:zlib";
import { DatabaseSync } from "node:sqlite";
import { listFiles } from "./paths.js";
import { CodexLine } from "./lines.js";
import { mapperOf, fail } from "./events.js";
import { tailedOr, untailed } from "./apps.js";

const UUID = /([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})\.jsonl(\.zst)?$/i;
const MAX_TEXT = 6000;
const VERSION = 2;

function textOf(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return "";
  return content.map((c) => (typeof c === "string" ? c : c.text ?? "")).join("\n");
}

const skippable = (t) => !t.trim() || t.startsWith("<") || t.startsWith("# AGENTS.md");

// A record, unless there's nothing worth searching in it, or the line
// doesn't say when it was written (as stats leave such a line out).
const doc = (role, text, ts) => (skippable(text) || !Number.isFinite(ts) ? [] : [{ role, text, ts }]);

// One searchable record per human or agent message, from a line's events
// (events.js), each joined with newlines. Tool results, a compacted
// summary and an interruption aren't anyone's message.
function docsOf(events) {
  for (const e of events) {
    switch (e.kind) {
      // A line with tool results is passed over whole.
      case "result":
        return [];
      case "prompt":
        // A Codex rollout's user_message events are its prompts while it
        // has no completed user message (those are read as items, below).
        if (e.origin === "event_msg:user_message") return doc("user", e.text, e.t);
        // Claude Code's (which carry their slash command): not a meta line.
        if (!("command" in e)) break;
        if (e.meta || e.compactSummary || e.interruption) return [];
        return doc(e.sidechain ? "subagent" : "user", textOf(e.parts), e.t);
      case "said": {
        if (e.origin === "event_msg:agent_message") return doc("assistant", String(e.parts ?? ""), e.t);
        if (e.origin === "response_item:message") return doc("assistant", textOf(e.parts), e.t);
        if (e.origin !== "assistant") break;
        // A reply's blocks, which search reads as a list.
        if (!Array.isArray(e.parts)) throw new TypeError("o.blocks.filter is not a function");
        const t = e.parts.filter((b) => b.type === "text").map((b) => b.text).join("\n");
        return t.trim() ? doc(e.sidechain ? "subagent" : "assistant", t, e.t) : [];
      }
      // What you said in Codex is a rollout's completed user messages,
      // whatever else may say it again.
      case "item":
        if (e.type === "UserMessage" || e.type === "AgentMessage") return doc(e.type === "UserMessage" ? "user" : "assistant", textOf(e.item.content), e.t);
        break;
      // A chunk that carries on a prompt or a reply (Grok Build's runs):
      // the message's whole text so far, which takes the place of the
      // record its first chunks made (`more`). Thinking isn't indexed, as
      // Claude Code's isn't.
      case "more":
        if (e.of === "thinking" || e.meta) return [];
        return doc(e.of === "prompt" ? "user" : "assistant", e.whole, e.t).map((d) => ({ ...d, more: true }));
      case "broken":
        if (reaches(e)) fail(e);
        break;
    }
  }
  return [];
}

// Whether search reads the field a line broke at: a user line only as far
// as its first tool result, and not a meta one; its text unless it's a
// compacted summary; and neither the refusal, a usage nor the type stats
// reads a Codex call by.
function reaches(e) {
  switch (e.at) {
    case "content":
      return !e.meta && !e.afterResult;
    case "text":
      return e.origin === "user" && !e.meta && !e.results && !e.compactSummary;
    case "line":
    case "blocks":
      return true;
    default:
      return false;
  }
}

// The documents of a line: normalized (see lines.js), or as the transcript
// has it. `userEvents`: a Codex rollout with no completed user messages at
// all, whose prompts are its user_message events.
export function docsFromLine(line, app, { userEvents = false } = {}) {
  // An app XenoNet doesn't tail is refused, not read as Codex.
  const mapper = mapperOf(app, null);
  if (!mapper) throw new Error(untailed(app));
  const o = line instanceof mapper.Line ? line : new mapper.Line(line);
  // A record that isn't an object fails where search read it first.
  void (o instanceof CodexLine ? o.payload : o.type);
  return docsOf(mapper.events(o, mapper.file({ items: !userEvents })));
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

// Whether a rollout's text has a completed user message: only the lines
// that name one are read, as rollouts can be hundreds of megabytes.
function holdsUserMessages(text) {
  for (let i = text.indexOf('"UserMessage"'); i !== -1; ) {
    const start = text.lastIndexOf("\n", i) + 1;
    const nl = text.indexOf("\n", i);
    const end = nl === -1 ? text.length : nl;
    try {
      if (new CodexLine(JSON.parse(text.slice(start, end))).userMessage != null) return true;
    } catch {}
    i = text.indexOf('"UserMessage"', end);
  }
  return false;
}

// FTS5 query from free text: every word must match, as a prefix.
export function ftsQuery(q) {
  const words = String(q ?? "").match(/[\p{L}\p{N}_]+/gu) ?? [];
  return words.slice(0, 8).map((w) => `"${w}"*`).join(" AND ");
}

export class TranscriptSearch {
  constructor({ dbFile, claudeRoot, codexRoots, grokRoot = null }) {
    this.dbFile = dbFile;
    this.roots = [
      ...(claudeRoot ? [{ dir: claudeRoot, app: "claude" }] : []),
      ...(codexRoots ?? []).map((dir) => ({ dir, app: "codex" })),
      ...(grokRoot ? [{ dir: grokRoot, app: "grok" }] : []),
    ].filter((r) => fs.existsSync(r.dir));
    this.status = { files: 0, docs: 0, indexing: false, lastIndexAt: 0, ms: 0 };
  }

  open() {
    if (this.db) return this.db;
    fs.mkdirSync(path.dirname(this.dbFile), { recursive: true });
    this.db = new DatabaseSync(this.dbFile);
    this.db.exec(`
      create virtual table if not exists docs using fts5(text, session_id unindexed, app unindexed, role unindexed, ts unindexed, file unindexed, tokenize = 'porter unicode61 remove_diacritics 2');
      create table if not exists files(path text primary key, size integer, mtime integer, offset integer, items integer not null default 0, run text);
    `);
    this.#migrate();
    this.insert = this.db.prepare("insert into docs(text, session_id, app, role, ts, file) values (?, ?, ?, ?, ?, ?)");
    this.rewrite = this.db.prepare("update docs set text = ? where rowid = ?");
    this.getFile = this.db.prepare("select size, mtime, offset, items, run from files where path = ?");
    this.putFile = this.db.prepare("insert into files(path, size, mtime, offset, items, run) values (?, ?, ?, ?, ?, ?) on conflict(path) do update set size = excluded.size, mtime = excluded.mtime, offset = excluded.offset, items = excluded.items, run = excluded.run");
    this.dropFile = this.db.prepare("delete from docs where file = ?");
    return this.db;
  }

  // An index from before version 1 has no `items`, and took a Codex
  // rollout's prompts from its completed user messages alone: Codex's
  // rollouts are indexed again, once. One from before version 2 has no
  // `run`, and kept a Grok Build message's first chunk alone: Grok Build's
  // sessions are indexed again, once.
  #migrate() {
    const version = this.db.prepare("pragma user_version").get().user_version;
    if (version >= VERSION) return;
    const columns = this.db.prepare("pragma table_info(files)").all().map((c) => c.name);
    this.db.exec("begin");
    try {
      if (!columns.includes("items")) this.db.exec("alter table files add column items integer not null default 0");
      if (!columns.includes("run")) this.db.exec("alter table files add column run text");
      if (version < 1) this.#again("codex");
      if (version < 2) this.#again("grok");
      this.db.exec(`pragma user_version = ${VERSION}`);
      this.db.exec("commit");
    } catch (e) {
      this.db.exec("rollback");
      throw e;
    }
  }

  // An app's transcripts, to be indexed again: the files it found
  // something in, and those under the app's folders, which it may have
  // found nothing in.
  #again(app) {
    const found = this.db.prepare("select distinct file from docs where app = ?").all(app);
    this.db.prepare("delete from docs where app = ?").run(app);
    const drop = this.db.prepare("delete from files where path = ?");
    for (const { file } of found) drop.run(file);
    const under = this.db.prepare("delete from files where substr(path, 1, ?) = ?");
    for (const r of this.roots.filter((x) => x.app === app)) {
      const prefix = r.dir.endsWith(path.sep) ? r.dir : r.dir + path.sep;
      under.run(prefix.length, prefix);
    }
  }

  // The session a transcript is of; none for an app XenoNet doesn't tail.
  sessionOf(file, app) {
    const norm = file.replace(/\\/g, "/");
    const tailed = tailedOr(app, null);
    if (!tailed) return null;
    // A Grok Build session's updates are in its folder, which is named for
    // it; its other files aren't read.
    if (tailed === "grok") {
      const m = /\/([^/]+)\/updates\.jsonl$/.exec(norm);
      return m ? `grok:${m[1]}` : null;
    }
    if (tailed === "claude") {
      const sub = /\/([0-9a-f-]{36})\/subagents\/[^/]+\.jsonl$/i.exec(norm);
      if (sub) return `claude:${sub[1]}`;
    } else if (!/rollout-/.test(norm)) return null;
    const m = UUID.exec(norm);
    return m ? `${app}:${m[1]}` : null;
  }

  // Indexes a transcript's lines, through its app's normalizer (lines.js);
  // lines that aren't JSON are left out. `userEvents`: see docsFromLine.
  // `open`: a run of chunks (Grok Build's) left open where the last read
  // ended, { run, doc }: what the mapper kept of it, and the record its
  // chunks so far made (a rowid, or null). It's left as the run open at
  // the end of this text. Gives back how many records it added.
  indexText(file, app, sessionId, text, { userEvents = false, open: carry = { run: null, doc: null } } = {}) {
    let n = 0;
    const seen = new Set();
    // Nothing of an app XenoNet doesn't tail is indexed.
    const mapper = mapperOf(app, null);
    if (!mapper) return 0;
    const state = mapper.file({ items: !userEvents, run: carry.run ?? null });
    // The record of the run of chunks still open, which its later chunks
    // write their whole text into.
    let open = carry.doc ?? null;
    const where = { file };
    for (const line of text.split("\n")) {
      if (!line.trim()) continue;
      const o = mapper.lines.line(line, where);
      if (!o) continue;
      const before = state.run;
      const docs = docsOf(mapper.events(o, state));
      if (!state.run || state.run !== before) open = null;
      for (const d of docs) {
        const body = d.text.slice(0, MAX_TEXT);
        const key = body.slice(0, 200);
        // The same text in a file is indexed once, as far as it's gone.
        if (d.more && open != null) {
          this.rewrite.run(body, open);
          seen.add(key);
          continue;
        }
        if (seen.has(key)) continue;
        seen.add(key);
        const { lastInsertRowid } = this.insert.run(body, sessionId, app, d.role, d.ts, file);
        if (state.run) open = Number(lastInsertRowid);
        n++;
      }
    }
    carry.run = state.run ?? null;
    carry.doc = state.run ? open : null;
    return n;
  }

  async indexFile(file, app) {
    const sessionId = this.sessionOf(file, app);
    if (!sessionId) return 0;
    const st = await fsp.stat(file).catch(() => null);
    if (!st) return 0;
    const row = this.getFile.get(file);
    const mtime = Math.floor(st.mtimeMs);
    if (row && row.size === st.size && row.mtime === mtime) return 0;
    const zst = file.endsWith(".zst");
    let whole = zst || !row || st.size < row.offset;
    let text = whole ? await readWhole(file) : await readFrom(file, row.offset, st.size);
    // Only index complete lines; a half-written last line waits for next time.
    const body = () => (zst ? text : text.slice(0, text.lastIndexOf("\n") + 1));
    // A Codex rollout's prompts are its completed user messages, or in one
    // with none at all its user_message events (see lines.js). One indexed
    // so far by its events that has a completed user message now is
    // indexed again, whole, by those.
    let items = app === "codex" && !whole && row.items === 1;
    if (app === "codex" && !items) {
      items = holdsUserMessages(body());
      if (items && !whole) [whole, text] = [true, await readWhole(file)];
    }
    const offset = whole ? 0 : row.offset;
    const indexed = body();
    // A run of chunks the last read left open goes on where it ended.
    const open = { run: null, doc: null };
    if (!whole && row.run) {
      try {
        Object.assign(open, JSON.parse(row.run));
      } catch {}
    }
    this.db.exec("begin");
    let n = 0;
    try {
      if (whole) this.dropFile.run(file);
      n = this.indexText(file, app, sessionId, indexed, { userEvents: app === "codex" && !items, open });
      this.putFile.run(file, st.size, mtime, zst ? st.size : offset + Buffer.byteLength(indexed), items ? 1 : 0, open.run ? JSON.stringify(open) : null);
      this.db.exec("commit");
    } catch (e) {
      this.db.exec("rollback");
      throw e;
    }
    return n;
  }

  async index() {
    if (this.status.indexing) return this.status;
    this.open();
    this.status.indexing = true;
    const t0 = Date.now();
    try {
      for (const r of this.roots) {
        for (const f of await listFiles(r.dir, (name) => /\.jsonl(\.zst)?$/.test(name))) {
          try {
            this.status.docs += await this.indexFile(f, r.app);
          } catch {
            this.status.errors = (this.status.errors ?? 0) + 1;
          }
          await new Promise((res) => setImmediate(res));
        }
      }
      this.status.files = this.db.prepare("select count(*) n from files").get().n;
      this.status.docs = this.db.prepare("select count(*) n from docs").get().n;
    } finally {
      this.status.indexing = false;
      this.status.lastIndexAt = Date.now();
      this.status.ms = Date.now() - t0;
    }
    return this.status;
  }

  search(q, limit = 30) {
    this.open();
    const match = ftsQuery(q);
    if (!match) return [];
    const rows = this.db
      .prepare(`select session_id, app, role, ts, snippet(docs, 0, '⟦', '⟧', ' … ', 16) as snip, bm25(docs) as score from docs where docs match ? order by score limit 500`)
      .all(match);
    const bySession = new Map();
    for (const r of rows) {
      let s = bySession.get(r.session_id);
      if (!s) bySession.set(r.session_id, (s = { sessionId: r.session_id, app: r.app, score: r.score, hits: 0, snippets: [] }));
      s.hits++;
      s.score = Math.min(s.score, r.score);
      if (s.snippets.length < 3) s.snippets.push({ role: r.role, ts: Number(r.ts), text: r.snip });
    }
    return [...bySession.values()].sort((a, b) => a.score - b.score || b.hits - a.hits).slice(0, limit);
  }

  close() {
    this.db?.close();
    this.db = null;
  }
}
