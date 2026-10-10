// Transcript lines as XenoNet reads them (core/lines.js): one normalizer per
// app, which the live tailer, stats and search share, and the views it
// gives with XenoNet's names for what the readers read alike.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ClaudeLine, CodexLine, claudeLines, codexLines, linesOf, readLines, isCodexPrompt } from "../core/lines.js";
import { TranscriptSearch } from "../core/search.js";

const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

const at = "2026-10-03T12:00:00.000Z";

test("a Claude Code line: its kind, time and flags, a user line's text and tool results", () => {
  const user = claudeLines.line(JSON.stringify({ type: "user", timestamp: at, isSidechain: true, isMeta: false, message: { role: "user", content: [{ type: "text", text: "Fix" }, { type: "image" }, { type: "text", text: "the parser" }] } }));
  assert.ok(user instanceof ClaudeLine);
  assert.deepEqual([user.type, user.ts, user.sidechain, user.meta, user.compactSummary, user.effort], ["user", Date.parse(at), true, false, undefined, null]);
  assert.equal(user.text, "Fix the parser", "text blocks, joined with spaces");
  assert.deepEqual([user.hasResults, user.results], [false, null]);
  assert.equal(new ClaudeLine({ message: { content: "plain" } }).text, "plain");

  const results = new ClaudeLine({ type: "user", message: { content: [{ type: "tool_result", tool_use_id: "t1", is_error: true }, { type: "text", text: "x" }, { type: "tool_result", tool_use_id: "t2" }] } });
  assert.deepEqual(results.results, [{ id: "t1", error: true }, { id: "t2", error: false }]);
  assert.ok(Number.isNaN(results.ts), "no time: NaN, which stats and search leave out and the live tailer reads as now");
  for (const timestamp of [0, 1791027000000, "not a date", null]) {
    assert.ok(Number.isNaN(new ClaudeLine({ type: "user", timestamp }).ts), `${JSON.stringify(timestamp)} isn't a time`);
    assert.ok(Number.isNaN(new CodexLine({ type: "event_msg", timestamp }).ts), `${JSON.stringify(timestamp)} isn't a time`);
  }
});

test("an assistant line: its message, usage counted XenoNet's way, blocks and the end of a turn", () => {
  const usage = { input_tokens: 10, cache_read_input_tokens: 2000, cache_creation_input_tokens: 300, cache_creation: { ephemeral_1h_input_tokens: 100 }, output_tokens: 50, speed: "fast" };
  const line = new ClaudeLine({ type: "assistant", timestamp: at, effort: "high", message: { id: "msg_1", model: "claude-opus-5-5", stop_reason: "end_turn", usage, content: [{ type: "text", text: "Done." }] } });
  assert.deepEqual([line.messageId, line.model, line.endTurn, line.effort], ["msg_1", "claude-opus-5-5", true, "high"]);
  assert.deepEqual(line.usage, { input: 10, cacheRead: 2000, cacheWrite: 300, cacheWrite5m: 200, cacheWrite1h: 100, output: 50, fast: true });
  assert.deepEqual(line.blocks, [{ type: "text", text: "Done." }]);
  const bare = new ClaudeLine({ type: "assistant", message: { id: "m", usage: {} } });
  assert.deepEqual(bare.usage, { input: 0, cacheRead: 0, cacheWrite: 0, cacheWrite5m: 0, cacheWrite1h: 0, output: 0, fast: false });
  assert.deepEqual([new ClaudeLine({ type: "assistant" }).usage, new ClaudeLine({ type: "assistant" }).blocks, new ClaudeLine({}).endTurn], [null, [], false]);
});

test("a Codex line: an envelope, its time and its payload as Codex wrote it", () => {
  const line = codexLines.line(JSON.stringify({ type: "event_msg", timestamp: at, payload: { type: "task_started", model_context_window: 258400 } }));
  assert.ok(line instanceof CodexLine);
  assert.deepEqual([line.type, line.ts, line.payload.type], ["event_msg", Date.parse(at), "task_started"]);
  assert.deepEqual(new CodexLine({ type: "response_item" }).payload, {});
  // A rollout's prompts are its completed user messages, and nothing else.
  const said = new CodexLine({ type: "event_msg", timestamp: at, payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text: "Why is" }, { type: "local_image", path: "/x.png" }, { type: "text", text: "it slow?" }] } } });
  assert.equal(said.userMessage, "Why is  it slow?");
  for (const raw of [{ type: "event_msg", payload: { type: "user_message", message: "Why?" } }, { type: "response_item", payload: { type: "message", role: "user", content: [{ type: "input_text", text: "Why?" }] } }, { type: "event_msg", payload: { type: "item_completed", item: { type: "AgentMessage", content: [] } } }]) assert.equal(new CodexLine(raw).userMessage, null);
  // What's a prompt, for the live tailer and stats alike: not blank, and not
  // starting with "<".
  assert.deepEqual(["Why?", "  ", "", "<subagent_notification>…", null].map(isCodexPrompt), [true, false, false, false, false]);
  assert.equal(linesOf("codex"), codexLines);
  assert.equal(linesOf("claude"), claudeLines);
});

test("a line that isn't JSON, or isn't an object, is passed by: nothing in it can be read", () => {
  const before = { ...claudeLines.counters };
  assert.equal(claudeLines.line("{ half"), null);
  assert.equal(claudeLines.line("null"), null);
  assert.equal(claudeLines.line("123"), null);
  assert.equal(codexLines.line('"a string"'), null);
  assert.deepEqual([claudeLines.counters.unparsable - before.unparsable, claudeLines.counters.records - before.records, claudeLines.counters.dropped - before.dropped], [1, 2, 2]);
});

test("a line whose field is of another type is read without it", () => {
  // The first line of its kind in a file is checked whole.
  const odd = claudeLines.line(JSON.stringify({ type: "user", timestamp: at, message: { role: "user", content: { text: "an object" } } }), {});
  assert.equal(odd.content, undefined);
  assert.equal(odd.text, "", "read the way a line without content is");
  const fine = claudeLines.line(JSON.stringify({ type: "user", timestamp: at, message: { role: "user", content: "fine" } }), {});
  assert.equal(fine.text, "fine");
});

test("a line whose type can't be turned into a string is read, not thrown on, by every reader", async () => {
  const odd = [
    { type: "event_msg", timestamp: at, payload: { type: { toString: 1 } } },
    { type: "event_msg", timestamp: at, payload: { type: "item_completed", item: { type: { toString: 1, valueOf: 2 } } } },
    { type: "response_item", timestamp: at, payload: { type: { toString: 1 } } },
    { type: { toString: 1 }, timestamp: at },
  ];
  for (const raw of odd) assert.ok(codexLines.line(JSON.stringify(raw), {}) instanceof CodexLine);
  assert.ok(claudeLines.line(JSON.stringify({ type: { toString: 1 }, timestamp: at }), {}) instanceof ClaudeLine);

  // Stats and a thread shown from its rollout read the file whole; search
  // indexes it, rather than rolling back and trying again every minute.
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-lines-"));
  made.push(dir);
  const file = path.join(dir, "rollout-2026-10-03T12-00-00-0190d3e0-0000-7000-8000-00000000000a.jsonl");
  const said = { type: "event_msg", timestamp: at, payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text: "after the odd lines" }] } } };
  fs.writeFileSync(file, [...odd, said].map((l) => JSON.stringify(l) + "\n").join(""));
  const lines = await readLines(file, "codex");
  assert.deepEqual([lines.length, lines.at(-1).userMessage], [5, "after the odd lines"]);
  const search = new TranscriptSearch({ dbFile: path.join(dir, "search.sqlite"), codexRoots: [dir] });
  search.open();
  try {
    assert.equal(await search.indexFile(file, "codex"), 1);
    assert.equal(search.search("odd lines")[0].sessionId, "codex:0190d3e0-0000-7000-8000-00000000000a");
  } finally {
    search.close();
  }
});
