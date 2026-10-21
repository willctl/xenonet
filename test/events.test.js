// Transcript lines as one stream of events (core/events.js): what each
// app's mapper makes of a line, a field it can't read, and a made-up third
// app whose transcripts go through stats and search as Claude Code's and
// Codex's do, with nothing of either reader changed for it.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { claude, codex, useMapper } from "../core/events.js";
import { ClaudeLine, CodexLine } from "../core/lines.js";
import { Normalizer } from "../core/blocks/normalizer.js";
import { createStats } from "../core/stats.js";
import { TranscriptSearch, docsFromLine } from "../core/search.js";
import { LiveTelemetry } from "../core/live.js";

const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-events-"));
after(() => fs.rmSync(home, { recursive: true, force: true }));

const at = (min) => new Date(Date.parse("2026-10-03T11:00:00Z") + min * 60e3).toISOString();
const kinds = (events) => events.map((e) => (e.kind === "broken" ? `broken:${e.at}` : e.kind));

test("a Claude Code line's events: in the order the readers read it, and its flags for what isn't a prompt", () => {
  const command = claude.events(new ClaudeLine({ type: "user", timestamp: at(0), message: { content: "<command-name>/goal</command-name><command-args>ship it</command-args>" } }));
  assert.deepEqual(kinds(command), ["prompt"]);
  assert.deepEqual(command[0].command, { name: "goal", args: "ship it" });
  assert.equal(command[0].origin, "user");

  const reply = claude.events(new ClaudeLine({ type: "assistant", timestamp: at(1), message: { id: "m1", model: "claude-opus-5-5", stop_reason: "end_turn", content: [{ type: "text", text: "Done." }, { type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } }] } }), { agentId: "a1" });
  assert.deepEqual(kinds(reply), ["call", "said", "tool", "turnEnd"]);
  assert.equal(reply[0].agentId, "a1");

  const stopped = claude.events(new ClaudeLine({ type: "user", timestamp: at(2), message: { content: [{ type: "tool_result", tool_use_id: "t1", is_error: true }, { type: "text", text: "[Request interrupted by user for tool use]" }] } }));
  assert.deepEqual(kinds(stopped), ["result", "turnEnd"], "a line with results is no prompt, but an interruption ends the turn");
  assert.equal(stopped[1].reason, "interrupted");

  const summary = claude.events(new ClaudeLine({ type: "user", timestamp: at(3), isCompactSummary: true, message: { content: "Carrying on." } }));
  assert.deepEqual(kinds(summary), ["compact", "prompt"]);
  assert.equal(summary[1].compactSummary, true);
});

test("a Codex line's events: its prompts by the rollout's mode, its calls, and its refusals", () => {
  const line = (payload) => new CodexLine({ type: "event_msg", timestamp: at(0), payload });
  const said = line({ type: "user_message", message: "compress the logs" });
  const done = line({ type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text: "compress the logs" }] } });
  assert.deepEqual(kinds(codex.events(said, codex.file({ items: false }))), ["prompt"], "a rollout without completed user messages: its events");
  assert.deepEqual(kinds(codex.events(said, codex.file({ items: true }))), [], "one with them: not its events");
  assert.deepEqual(kinds(codex.events(done, codex.file({ items: true }))), ["item", "prompt"]);
  assert.deepEqual(kinds(codex.events(done, codex.file({ items: false }))), ["item"]);
  assert.deepEqual(kinds(codex.events(done, codex.file({ follow: true }))), ["item", "prompt"], "live reads every completed user message");
  assert.deepEqual(kinds(codex.events(line({ type: "user_message", message: "<note>" }))), [], "not a prompt (isCodexPrompt)");

  const count = codex.events(line({ type: "token_count", info: { total_token_usage: { total_tokens: 12000 }, last_token_usage: { input_tokens: 11000, cached_input_tokens: 8000, output_tokens: 1000 } } }));
  assert.deepEqual(kinds(count), ["call", "context"]);
  assert.deepEqual([count[0].usage.input, count[0].usage.cacheRead, count[0].total.total_tokens], [3000, 8000, 12000]);

  const refused = codex.events(line({ type: "error", codex_error_info: "cyber_policy" }));
  assert.deepEqual(kinds(refused), ["refusal", "turnEnd"]);
  assert.equal(refused[1].reason, "refusal");
});

test("a field the mapper can't read is broken: the readers that read it throw what reading it threw, the others go on", () => {
  // Content that isn't text or blocks: its text can't be read.
  const odd = { type: "user", timestamp: at(0), message: { content: 5 } };
  const events = claude.events(new ClaudeLine(odd));
  assert.deepEqual(kinds(events), ["broken:text"]);
  let thrown;
  try {
    void new ClaudeLine(odd).text;
  } catch (err) {
    thrown = err;
  }
  assert.equal(events[0].error.message, thrown.message);
  assert.throws(() => docsFromLine(odd, "claude"), { message: thrown.message }, "search reads a user line's text");
  assert.deepEqual(docsFromLine({ ...odd, isMeta: true }, "claude"), [], "but not a meta line's");

  // A block that isn't there, after a tool result: search reads a user line
  // only as far as its first result; the live tailer reads them all.
  const gap = { type: "user", timestamp: at(1), message: { content: [{ type: "tool_result", tool_use_id: "t1" }, null] } };
  assert.deepEqual(kinds(claude.events(new ClaudeLine(gap))), ["broken:content"]);
  assert.deepEqual(docsFromLine(gap, "claude"), []);
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null });
  assert.throws(() => live.applyClaude({ sessionId: "claude:s" }, gap, false), /reading 'type'/);
});

// ----- a made-up third app -----

// Its transcripts: one JSON line per message, `{ at, by: "me" | "bot",
// say, id, tokens, run }`. Its sampling kinds are what Claude Code's are
// ("user", "assistant"), and its events are shaped as Claude Code's.
class ToyLine {
  constructor(raw) {
    this.raw = raw;
  }
}
const toy = {
  app: "toy",
  lines: new Normalizer({ name: "toy-lines", normalize: (raw) => new ToyLine(raw) }),
  Line: ToyLine,
  sorted: true,
  file: ({ agentId = null } = {}) => ({ agentId }),
  events(o, file = {}) {
    const r = o.raw;
    const base = { t: typeof r.at === "string" ? Date.parse(r.at) : NaN, agentId: file.agentId ?? null, origin: r.by === "me" ? "user" : "assistant" };
    if (r.by === "me") return [{ kind: "prompt", ...base, text: r.say, parts: r.say, command: null }];
    const out = [{ kind: "call", ...base, messageId: r.id, model: "toy-1", effort: null, usage: { input: r.tokens, cacheRead: 0, cacheWrite: 0, cacheWrite5m: 0, cacheWrite1h: 0, output: 10, fast: false } }];
    out.push({ kind: "said", ...base, parts: [{ type: "text", text: r.say }] });
    if (r.run) out.push({ kind: "tool", ...base, id: `${r.id}-run`, name: r.run, input: {} }, { kind: "result", ...base, id: `${r.id}-run`, error: false });
    out.push({ kind: "turnEnd", ...base, reason: "end_turn" });
    return out;
  },
};

test("a made-up app's transcripts go through stats and search with neither changed for it", async () => {
  const drop = useMapper("toy", toy);
  after(drop);
  const id = "0190d3e0-0000-7000-8000-00000000c0de";
  const file = path.join(home, "toy", `${id}.jsonl`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const lines = [
    { at: at(0), by: "me", say: "Add partial refunds to the orders API" },
    { at: at(1), by: "bot", id: "r1", tokens: 1000, say: "Refunds go through the ledger.", run: "Bash" },
    { at: at(5), by: "me", say: "Now the receipts" },
    { at: at(6), by: "bot", id: "r2", tokens: 500, say: "Receipts list each refund." },
  ];
  fs.writeFileSync(file, lines.map((l) => JSON.stringify(l)).join("\n") + "\n");

  const stats = await createStats({ transcripts: { files: async () => [{ file, agentId: null }] } })(`toy:${id}`);
  assert.equal(stats.app, "toy");
  assert.equal(stats.totals.calls, 2);
  assert.equal(stats.totals.input, 1500);
  assert.deepEqual(stats.turns.map((t) => t.prompt), ["Add partial refunds to the orders API", "Now the receipts"]);
  assert.deepEqual(stats.tools.map((t) => [t.name, t.category, t.calls]), [["Bash", "shell", 1]]);

  const search = new TranscriptSearch({ dbFile: path.join(home, "search.sqlite") });
  search.open();
  assert.equal(search.indexText(file, "toy", `toy:${id}`, fs.readFileSync(file, "utf8")), 4);
  const hits = search.search("ledger refunds");
  assert.deepEqual(hits.map((h) => [h.sessionId, h.app]), [[`toy:${id}`, "toy"]]);
  assert.deepEqual(search.search("receipts")[0].snippets.map((s) => s.role).sort(), ["assistant", "user"]);
  search.close();
});
