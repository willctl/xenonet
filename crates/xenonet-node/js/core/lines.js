// core/lines.js in Rust (crates/xenonet-sources/src/lines.rs): transcript
// lines as XenoNet reads them, a Claude Code transcript's and a Codex
// rollout's, through one normalizer each, which the three readers share.
//
// A view is a Rust object over the record, which reads it when asked the
// way the JS did: what JS would throw reading a field of an odd record, it
// throws (a TypeError). The record itself stays here as `raw`, the object
// it was made from. Rust also splits what a read gave into lines, and says
// where a read of a growing file starts; the reads stay here.
import fsp from "node:fs/promises";
import zlib from "node:zlib";
import { transcriptHooks } from "./vendor.js";
import { native } from "../index.js";
import { Normalizer } from "./blocks/normalizer.js";
import { tailedApp } from "../../../../core/apps.js";

// A line of a Claude Code transcript (~/.claude/projects/**/*.jsonl).
export class ClaudeLine {
  #core;
  constructor(raw) {
    this.raw = raw;
    this.#core = new native.ClaudeLineCore(raw);
  }

  // "user", "assistant", or another kind the readers pass over.
  get type() {
    return this.#core.kind;
  }
  // When it was written; NaN when it doesn't say or can't be read.
  get ts() {
    return this.#core.ts;
  }
  get meta() {
    return this.#core.meta;
  }
  get sidechain() {
    return this.#core.sidechain;
  }
  get compactSummary() {
    return this.#core.compactSummary;
  }
  get effort() {
    return this.#core.effort;
  }

  // ----- a user line -----

  get content() {
    return this.#core.content;
  }
  get hasResults() {
    return this.#core.hasResults;
  }
  get results() {
    return this.#core.results;
  }
  get text() {
    return this.#core.text;
  }
  get interruption() {
    return this.#core.interruption;
  }

  // ----- an assistant line -----

  get message() {
    return this.#core.message;
  }
  get messageId() {
    return this.#core.messageId;
  }
  get model() {
    return this.#core.model;
  }
  get usage() {
    return this.#core.usage;
  }
  get blocks() {
    return this.#core.blocks;
  }
  get endTurn() {
    return this.#core.endTurn;
  }
}

// A line of a Codex rollout (~/.codex/sessions/**/rollout-*.jsonl): an
// envelope, and its payload as Codex wrote it.
export class CodexLine {
  #core;
  constructor(raw) {
    this.raw = raw;
    this.#core = new native.CodexLineCore(raw);
  }

  get type() {
    return this.#core.kind;
  }
  get ts() {
    return this.#core.ts;
  }
  get payload() {
    return this.#core.payload;
  }
  get userMessage() {
    return this.#core.userMessage;
  }
  get userEvent() {
    return this.#core.userEvent;
  }
}

// Whether a rollout's lines have a completed user message.
export const hasUserMessages = (lines) => lines.some((o) => o.userMessage != null);

// Whether what you said in a rollout is a prompt: not blank, and not
// starting with "<".
export const isCodexPrompt = (text) => native.isCodexPrompt(text);

// How the drift hooks sort and date lines (vendor.js's transcriptHooks).
const claudeHooks = {
  kindOf: (o) => native.claudeLineKindOf(o.type),
  versionOf: (o, current) => native.claudeLineVersionOf(o.version, current ?? null),
};
const codexHooks = {
  kindOf: (o) => native.codexLineKindOf(o.type, o.payload?.type, o.payload?.item?.type),
  versionOf: (o, current) => native.codexLineVersionOf(o.type, o.payload?.cli_version, current ?? null),
};

export const claudeLines = new Normalizer({ name: "claude-lines", source: "anthropic/claude-code/transcript-line", normalize: (raw) => new ClaudeLine(raw) }).use(transcriptHooks("anthropic/claude-code/transcript-line", claudeHooks));
export const codexLines = new Normalizer({ name: "codex-lines", source: "openai/codex/rollout-line", normalize: (raw) => new CodexLine(raw) }).use(transcriptHooks("openai/codex/rollout-line", codexHooks));

// The normalizer for an app's transcripts: any app but Codex's are read as
// Claude Code's.
const NORMALIZERS = { claude: claudeLines, codex: codexLines };
export const linesOf = (app) => NORMALIZERS[tailedApp(app, "claude")];

const NOTHING = Buffer.alloc(0);

// The lines in `data`, through a normalizer, onto `out`; what's left after
// the last newline when it isn't a whole record is given back, as bytes.
function take(data, lines, where, out) {
  const t = native.takeLines(data);
  for (const line of t.lines) {
    const rec = lines.line(line, where);
    if (rec) out.push(rec);
  }
  if (t.hasLast) {
    const rec = lines.record(t.last, where);
    if (rec) out.push(rec);
  }
  return t.rest;
}

// A whole transcript's lines, through its app's normalizer, a compressed
// archive (.jsonl.zst) too.
export async function readLines(file, app) {
  const buf = await fsp.readFile(file);
  const out = [];
  take(file.endsWith(".zst") ? zlib.zstdDecompressSync(buf) : buf, linesOf(app), { file }, out);
  return out;
}

// The lines added to a transcript since the read before: `from` is what
// that read gave back ({ offset, rest, where }), or nothing to read the
// file from the start. A file that got shorter was written again: it's read
// from the start (`restarted`).
export async function readNewLines(file, app, from = null) {
  const size = (await fsp.stat(file)).size;
  let { offset = 0, rest = NOTHING, where = { file } } = from ?? {};
  const [restarted, start] = native.readFrom(size, offset);
  if (restarted) [offset, rest, where] = [start, NOTHING, { file }];
  const out = [];
  if (size > offset) {
    const fh = await fsp.open(file, "r");
    const buf = Buffer.alloc(size - offset);
    try {
      await fh.read(buf, 0, buf.length, offset);
    } finally {
      await fh.close();
    }
    offset = size;
    rest = take(rest.length ? Buffer.concat([rest, buf]) : buf, linesOf(app), where, out);
  }
  return { lines: out, offset, rest, where, restarted };
}
