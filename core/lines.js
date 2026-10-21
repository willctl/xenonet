// Transcript lines as XenoNet reads them: a Claude Code transcript's lines
// and a Codex rollout's, through one normalizer each (core/blocks/
// normalizer.js), which the three readers share: the live tailer
// (live.js), a session's stats (stats.js) and search (search.js). Each
// line is checked against the vendor's schema on the way (vendor.js's
// transcript hooks): a sample of each file's lines of each kind whole, the
// rest only for type changes found before; what differs is kept as drift
// (drift.js), a field of another type is taken out, and a line that isn't
// an object, which nothing can be read from, is dropped.
//
// A normalized line is a view of the vendor's record with XenoNet's names
// for what the readers read alike. Where they read a line differently
// (how text is joined, what each one makes of a line), each reader keeps
// its own way, from the same view; see docs/design/blocks.md. The view reads the record when asked, the way the
// readers did, so a line that broke a reader before breaks it at the same
// point now.
import fsp from "node:fs/promises";
import zlib from "node:zlib";
import { Normalizer } from "./blocks/normalizer.js";
import { transcriptHooks } from "./vendor.js";
import { tailedApp } from "./apps.js";

// When a line was written, from its ISO 8601 timestamp: NaN when it says
// none, or something that isn't a time. A number isn't one (Date.parse
// would read 0 as the year 2000): both formats write a string. Stats and
// search leave such a line out; the live tailer takes it as written when
// it's read.
const timeOf = (t) => (typeof t === "string" ? Date.parse(t) : NaN);

// A line of a Claude Code transcript (~/.claude/projects/**/*.jsonl).
export class ClaudeLine {
  constructor(raw) {
    this.raw = raw;
  }

  // "user", "assistant", or another kind the readers pass over.
  get type() {
    return this.raw.type;
  }
  // When it was written; NaN when it doesn't say or can't be read.
  get ts() {
    return timeOf(this.raw.timestamp);
  }
  get meta() {
    return this.raw.isMeta;
  }
  get sidechain() {
    return this.raw.isSidechain;
  }
  // A compacted context's summary, which carries on from the conversation
  // before it: no prompt.
  get compactSummary() {
    return this.raw.isCompactSummary;
  }
  get effort() {
    return this.raw.effort ?? null;
  }

  // ----- a user line -----

  // What it holds: text, or a list of blocks.
  get content() {
    return this.raw.message?.content;
  }
  // Whether it carries tool results (and so is no prompt).
  get hasResults() {
    const c = this.content;
    return Array.isArray(c) && c.some((x) => x.type === "tool_result");
  }
  // The tool results it carries ({ id, error }), or null when it carries
  // none.
  get results() {
    if (!this.hasResults) return null;
    return this.content.filter((x) => x.type === "tool_result").map((x) => ({ id: x.tool_use_id, error: !!x.is_error }));
  }
  // What it says: its text, or its text blocks joined with spaces.
  get text() {
    const c = this.content;
    return typeof c === "string" ? c : (c ?? []).filter((x) => x.type === "text").map((x) => x.text).join(" ");
  }
  // Whether it's Claude Code's note that you stopped it ("[Request
  // interrupted by user]", "… for tool use]"): no prompt either.
  get interruption() {
    return this.text.trimStart().startsWith("[Request interrupted");
  }

  // ----- an assistant line -----

  get message() {
    return this.raw.message ?? {};
  }
  // The message it's part of: Claude Code writes one line per block,
  // each with the message's id and usage.
  get messageId() {
    return this.message.id;
  }
  get model() {
    return this.message.model;
  }
  // The message's tokens, or null when it doesn't say: input, cache reads,
  // cache writes (all of them, and by how long they're kept), output, and
  // whether it ran in fast mode.
  get usage() {
    const u = this.message.usage;
    if (!u) return null;
    const write1h = u.cache_creation?.ephemeral_1h_input_tokens ?? 0;
    return {
      input: u.input_tokens ?? 0,
      cacheRead: u.cache_read_input_tokens ?? 0,
      cacheWrite: u.cache_creation_input_tokens ?? 0,
      cacheWrite5m: u.cache_creation?.ephemeral_5m_input_tokens ?? Math.max(0, (u.cache_creation_input_tokens ?? 0) - write1h),
      cacheWrite1h: write1h,
      output: u.output_tokens ?? 0,
      fast: u.speed === "fast",
    };
  }
  // Its content blocks, as the vendor wrote them (text, tool_use, …).
  get blocks() {
    return this.message.content ?? [];
  }
  // Whether the turn (or a subagent's work) ends with it.
  get endTurn() {
    return this.message.stop_reason === "end_turn";
  }
}

// A line of a Codex rollout (~/.codex/sessions/**/rollout-*.jsonl): an
// envelope, and its payload as Codex wrote it.
export class CodexLine {
  constructor(raw) {
    this.raw = raw;
  }

  // "event_msg", "response_item", "turn_context", "session_meta", …
  get type() {
    return this.raw.type;
  }
  // When it was written; NaN when it doesn't say or can't be read.
  get ts() {
    return timeOf(this.raw.timestamp);
  }
  get payload() {
    return this.raw.payload ?? {};
  }
  // What you said, when the line is a completed user message (an
  // item_completed UserMessage), its parts joined with spaces; else null.
  // A rollout's prompts are these, for every reader: Codex may write the
  // same words again as a user_message event and a user response item, or
  // (lately) not at all.
  get userMessage() {
    const p = this.payload;
    if (this.type !== "event_msg" || p.type !== "item_completed" || p.item?.type !== "UserMessage") return null;
    return (p.item.content ?? []).map((c) => c?.text ?? "").join(" ");
  }
  // What you said, when the line is a user_message event; else null. A
  // rollout with no completed user messages at all (`codex exec` writes
  // none, nor did Codex before 0.130) has its prompts in these instead.
  get userEvent() {
    const p = this.payload;
    if (this.type !== "event_msg" || p.type !== "user_message") return null;
    return typeof p.message === "string" ? p.message : "";
  }
}

// Whether a rollout's lines have a completed user message: when they have
// none at all, its prompts are its user_message events (userEvent).
export const hasUserMessages = (lines) => lines.some((o) => o.userMessage != null);

// Whether what you said in a rollout is a prompt: not blank (an image
// alone), and not starting with "<" (a delegation envelope, or a note Codex
// wrote for itself). The live tailer and stats go by this, and search
// leaves out the same (and "# AGENTS.md…", as for Claude Code).
export const isCodexPrompt = (text) => typeof text === "string" && text.trim() !== "" && !text.startsWith("<");

// Claude Code writes its version on each line; a Codex rollout says its
// version once, in its first line (session_meta). Lines are sampled by
// kind: Claude Code's by type, Codex's by type and payload type (and an
// item's type). A type that isn't a string counts by what it is ("object"),
// since one that can't be turned into a string (an object whose toString
// isn't a function) would throw.
const kindPart = (v) => (typeof v === "string" ? v : typeof v);
const claudeHooks = {
  kindOf: (o) => kindPart(o.type),
  versionOf: (o, current) => (typeof o.version === "string" ? o.version : current),
};
const codexHooks = {
  kindOf: (o) => {
    const t = kindPart(o.type);
    if (t !== "event_msg" && t !== "response_item") return t;
    const p = kindPart(o.payload?.type);
    return p === "item_completed" ? `${t}:${p}:${kindPart(o.payload.item?.type)}` : `${t}:${p}`;
  },
  versionOf: (o, current) => (o.type === "session_meta" && typeof o.payload?.cli_version === "string" ? o.payload.cli_version : current),
};

export const claudeLines = new Normalizer({ name: "claude-lines", source: "anthropic/claude-code/transcript-line", normalize: (raw) => new ClaudeLine(raw) }).use(transcriptHooks("anthropic/claude-code/transcript-line", claudeHooks));
export const codexLines = new Normalizer({ name: "codex-lines", source: "openai/codex/rollout-line", normalize: (raw) => new CodexLine(raw) }).use(transcriptHooks("openai/codex/rollout-line", codexHooks));

// The normalizer for an app's transcripts: any app but Codex's are read as
// Claude Code's.
const NORMALIZERS = { claude: claudeLines, codex: codexLines };
export const linesOf = (app) => NORMALIZERS[tailedApp(app, "claude")];

const NOTHING = Buffer.alloc(0);

// The lines in `data` (bytes), through a normalizer, onto `out`, with
// lines that aren't JSON left out; what's left after the last newline is
// given back. That last line may be half written: it's read if it's
// whole, and given back as bytes if not (a read can end inside a
// character), rather than counted as a line that isn't JSON.
function take(data, lines, where, out) {
  const cut = data.lastIndexOf(10) + 1;
  for (const line of data.toString("utf8", 0, cut).split("\n")) {
    if (!line.trim()) continue;
    const rec = lines.line(line, where);
    if (rec) out.push(rec);
  }
  const rest = data.subarray(cut);
  const last = rest.toString("utf8");
  if (!last.trim()) return Buffer.from(rest);
  let raw;
  try {
    raw = JSON.parse(last);
  } catch {
    return Buffer.from(rest);
  }
  const rec = lines.record(raw, where);
  if (rec) out.push(rec);
  return NOTHING;
}

// A whole transcript's lines, through its app's normalizer (or `lines`,
// the one its mapper names: core/events.js), for a reader that reads the
// file at once (stats, and an archived rollout shown as a thread): a
// compressed archive (.jsonl.zst) too.
export async function readLines(file, app, lines = linesOf(app)) {
  const buf = await fsp.readFile(file);
  const out = [];
  take(file.endsWith(".zst") ? zlib.zstdDecompressSync(buf) : buf, lines, { file }, out);
  return out;
}

// The lines added to a transcript since the read before, for a reader that
// keeps what it read as the file grows (a Codex thread shown from its
// rollout), as the tailer reads: `from` is what that read gave back ({
// offset, rest, where }), or nothing to read the file from the start. It
// gives back { lines, offset, rest, where } for the next. A file that got
// shorter was written again: it's read from the start (`restarted`).
export async function readNewLines(file, app, from = null) {
  const size = (await fsp.stat(file)).size;
  let { offset = 0, rest = NOTHING, where = { file } } = from ?? {};
  const restarted = size < offset;
  if (restarted) [offset, rest, where] = [0, NOTHING, { file }];
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
