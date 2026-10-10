// A session's stats (core/stats.js), API prices by date (core/pricing.js)
// and how numbers read (core/units.js), from transcripts written here.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { priceOf, costOf, costParts, plainModel } from "../core/pricing.js";
import { Transcripts, createStats } from "../core/stats.js";
import { money, tokens, duration, share, excerpt } from "../core/units.js";
import { drift } from "../core/drift.js";

const day = (s) => Date.parse(`${s}T12:00:00Z`);
const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ≠ ${b}`);

test("prices: the model's own row, the longest id it starts with, and the price on the day", () => {
  assert.equal(priceOf("claude-opus-5-5", day("2026-10-01")).input, 4);
  assert.equal(priceOf("claude-haiku-4-5-20251001", day("2026-10-01")).output, 5);
  assert.equal(priceOf("claude-sonnet-4-5", day("2026-01-01")).input, 3, "a short id finds its dated row");
  assert.equal(priceOf("claude-opus-4-1-20250805", day("2025-09-01")).ids[0], "claude-opus-4-1-20250805", "not claude-opus-4");
  assert.equal(priceOf("gpt-5.1-codex-mini", day("2026-01-01")).input, 0.25);
  assert.equal(priceOf("gpt-5.1-codex", day("2026-01-01")).ids[0], "gpt-5.1");
  assert.equal(priceOf("gpt-5.7", day("2026-10-01")), null, "gpt-5 doesn't stand in for a model it doesn't know");
  assert.deepEqual([priceOf("gpt-5.6-sol", day("2026-07-15")).input, priceOf("gpt-5.6-sol", day("2026-09-01")).input], [5, 4], "the launch price, then the promotion");
  assert.equal(priceOf("o3", day("2025-05-01")).input, 10);
  assert.equal(priceOf("o3", day("2025-07-01")).input, 2);
  assert.equal(plainModel("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), "claude-sonnet-4-5-20250929");
  assert.equal(plainModel("claude-sonnet-4-5@20250929"), "claude-sonnet-4-5-20250929");
  assert.equal(plainModel("claude-opus-4-6[1m]"), "claude-opus-4-6");
  assert.equal(priceOf("<synthetic>"), null);
});

test("costs: cache reads and writes, long context, fast mode and OpenAI's priority tier", () => {
  const m = 1e6;
  assert.deepEqual(costParts("claude-opus-5-5", day("2026-10-01"), { input: m, cacheRead: m, cacheWrite5m: m, cacheWrite1h: m, output: m }), { input: 4, cacheRead: 0.2, cacheWrite: 13, output: 20, total: 37.2 });
  near(costOf("claude-opus-5-5", day("2026-10-01"), { input: m }, { fast: true }), 8);
  // Sonnet 4.5 above 200K input tokens, before the surcharge ended.
  near(costOf("claude-sonnet-4-5", day("2026-01-10"), { input: 150000, cacheRead: 100000, output: m / 10 }), (150000 * 6 + 100000 * 0.3 * 2 + 100000 * 22.5) / m);
  near(costOf("claude-sonnet-4-5", day("2026-05-10"), { input: 150000, cacheRead: 100000, output: m / 10 }), (150000 * 3 + 100000 * 0.3 + 100000 * 15) / m);
  // OpenAI: cached input at its rate, and a whole call above 272K at 2x / 1.5x.
  near(costOf("gpt-5.4", day("2026-04-01"), { input: 100000, cacheRead: 100000, output: 1000 }), (100000 * 2.5 + 100000 * 0.25 + 1000 * 15) / m);
  near(costOf("gpt-5.4", day("2026-04-01"), { input: 300000, output: 1000 }), (300000 * 5 + 1000 * 22.5) / m);
  near(costOf("gpt-5.5", day("2026-05-01"), { input: 1000 }, { priority: true }), (1000 * 5 * 2.5) / m);
  assert.equal(costOf("<synthetic>", day("2026-10-01"), { input: 5 }), 0);
  assert.equal(costOf("someone-elses-model", day("2026-10-01"), { input: 5 }), null);
});

test("units: money, tokens, durations, shares and long text as people read them", () => {
  assert.deepEqual([excerpt("abcdef", 4), excerpt("abc", 4), excerpt(null), excerpt("x".repeat(4001)).slice(-20)], ["abcd\n… (2 more chars)", "abc", "", "xxx\n… (1 more chars)"]);
  assert.deepEqual([money(1234.5), money(0.004), money(0), money(null)], ["$1,234.50", "<$0.01", "$0.00", "no price"]);
  assert.deepEqual([tokens(313157871), tokens(200000), tokens(14600), tokens(840)], ["313.2M", "200k", "14.6k", "840"]);
  assert.deepEqual([duration(20000), duration(52 * 60e3), duration(68 * 60e3), duration(120 * 60e3)], ["20 s", "52 min", "1 h 8 min", "2 h"]);
  assert.deepEqual([share(1, 3), share(1, 1000), share(0, 5), share(1, 0)], ["33%", "<1%", "0%", ""]);
});

// ---------- transcripts ----------

// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-stats-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));
const at = (min) => new Date(Date.UTC(2026, 9, 1, 12, min)).toISOString();
const jsonl = (lines) => lines.map((l) => JSON.stringify(l)).join("\n") + "\n";
const usage = (input, read, write, output, extra = {}) => ({ input_tokens: input, cache_read_input_tokens: read, cache_creation_input_tokens: write, cache_creation: { ephemeral_5m_input_tokens: write, ephemeral_1h_input_tokens: 0 }, output_tokens: output, ...extra });

test("a Claude Code session: calls once per message, turns, tools, subagents and a compaction", async () => {
  const root = tmp();
  const id = "11111111-2222-3333-4444-555555555555";
  const dir = path.join(root, "-w-atlas");
  fs.mkdirSync(path.join(dir, id, "subagents"), { recursive: true });
  const say = (min, mid, u, content, extra = {}) => ({ type: "assistant", timestamp: at(min), message: { id: mid, model: "claude-opus-5-5", usage: u, content, ...extra } });
  fs.writeFileSync(path.join(dir, `${id}.jsonl`), jsonl([
    { type: "user", timestamp: at(0), message: { role: "user", content: "Add partial refunds" } },
    // One message, two lines: counted once.
    say(1, "m1", usage(10, 1000, 500, 100), [{ type: "text", text: "Looking" }]),
    say(1, "m1", usage(10, 1000, 500, 100), [{ type: "tool_use", id: "t1", name: "Bash", input: { command: "ls" } }]),
    { type: "user", timestamp: at(2), message: { content: [{ type: "tool_result", tool_use_id: "t1", is_error: true }] } },
    say(3, "m2", usage(5, 2000, 0, 50), [{ type: "tool_use", id: "t2", name: "Agent", input: { description: "Find the refund tests" } }]),
    { type: "user", timestamp: at(9), message: { content: [{ type: "tool_result", tool_use_id: "t2" }] } },
    say(10, "m3", usage(5, 3000, 0, 80), [{ type: "text", text: "Done" }], { stop_reason: "end_turn" }),
    { type: "user", timestamp: at(11), message: { content: "[Request interrupted by user]" } },
    { type: "user", timestamp: at(12), message: { content: "<command-name>/goal</command-name>\n<command-message>goal</command-message>\n<command-args>ship it</command-args>" } },
    say(13, "m4", usage(5, 900000, 0, 80), [{ type: "text", text: "On it" }]),
    { type: "user", timestamp: at(14), isCompactSummary: true, message: { content: "This session is being continued from a previous conversation" } },
    say(15, "m5", usage(5, 20000, 4000, 80), [{ type: "text", text: "Shipped" }], { stop_reason: "end_turn" }),
    say(16, "m6", { input_tokens: 0, output_tokens: 0 }, [{ type: "text", text: "No response requested." }], { model: "<synthetic>" }),
  ]));
  fs.writeFileSync(path.join(dir, id, "subagents", "agent-a1.jsonl"), jsonl([
    { type: "user", timestamp: at(3), isSidechain: true, message: { content: "Find every refund test and list them" } },
    { type: "assistant", timestamp: at(4), isSidechain: true, message: { id: "s1", model: "claude-haiku-4-5", usage: usage(5, 100, 0, 20), content: [{ type: "text", text: "Found 3" }] } },
  ]));

  const stats = createStats({ transcripts: new Transcripts({ claudeRoot: root }) });
  const s = await stats(`claude:${id}`);
  assert.equal(s.totals.calls, 7, "five messages, a synthetic one, and the subagent's");
  assert.equal(s.totals.input, 10 + 5 + 5 + 5 + 5 + 5);
  assert.deepEqual(s.turns.map((t) => [t.n, t.prompt]), [[1, "Add partial refunds"], [2, "/goal ship it"]]);
  assert.equal(s.turns[0].end - s.turns[0].start, 10 * 60e3);
  assert.equal(s.turns[1].end - s.turns[1].start, 4 * 60e3, "the compaction didn't start a turn");
  assert.deepEqual(s.compactions, [Date.parse(at(14))]);
  assert.deepEqual(s.tools.map((t) => [t.name, t.calls, t.errors, t.ms]), [["Bash", 1, 1, 60e3], ["Agent", 1, 0, 6 * 60e3]]);
  assert.deepEqual(s.subagents.map((a) => [a.label, a.model, a.calls]), [["Find the refund tests", "claude-haiku-4-5", 1]]);
  assert.deepEqual(s.models.map((m) => m.model), ["claude-opus-5-5", "claude-haiku-4-5", "<synthetic>"]);
  assert.equal(s.context.length, 6, "main-agent calls only");
  assert.equal(s.context.find((p) => p.used > 900000).window, 1000000, "past 200K the window is 1M");
  const opus = (u) => costOf("claude-opus-5-5", Date.parse(at(1)), u);
  near(s.turns[0].byModel["claude-opus-5-5"].cost, opus({ input: 10, cacheRead: 1000, cacheWrite5m: 500, output: 100 }) + opus({ input: 5, cacheRead: 2000, output: 50 }) + opus({ input: 5, cacheRead: 3000, output: 80 }));
  assert.equal(s.pricing.unpriced.length, 0);
  assert.equal(await stats(`claude:${id}`), s, "unchanged files: the same answer, from memory");
  assert.equal(await stats("claude:99999999-2222-3333-4444-555555555555"), null);
});

test("a transcript being written: a last line without its newline is read when it's whole, and passed by quietly when it isn't", async () => {
  const root = tmp();
  const id = "11111111-2222-3333-4444-666666666666";
  const file = path.join(root, "-w-atlas", `${id}.jsonl`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const first = JSON.stringify({ type: "user", timestamp: at(0), message: { role: "user", content: "Add partial refunds" } });
  const second = JSON.stringify({ type: "user", timestamp: at(1), message: { role: "user", content: "And the CSV export" } });
  const stats = createStats({ transcripts: new Transcripts({ claudeRoot: root }) });
  const unparsable = () => drift.list().filter((r) => r.kind === "unparsable").length;
  const before = unparsable();
  fs.writeFileSync(file, `${first}\n${second.slice(0, 30)}`);
  assert.deepEqual((await stats(`claude:${id}`)).turns.map((t) => t.prompt), ["Add partial refunds"]);
  assert.equal(unparsable(), before, "half a line isn't drift");
  fs.writeFileSync(file, `${first}\n${second}`);
  assert.deepEqual((await stats(`claude:${id}`)).turns.map((t) => t.prompt), ["Add partial refunds", "And the CSV export"]);
});

test("a Codex thread: the model by turn, each call counted once, cached input apart, compressed or not", async () => {
  const root = tmp();
  const id = "019a0000-aaaa-bbbb-cccc-000000000001";
  const day1 = path.join(root, "sessions", "2026", "10", "01");
  fs.mkdirSync(day1, { recursive: true });
  const count = (min, total, last) => ({ type: "event_msg", timestamp: at(min), payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage: last, model_context_window: 272000 } } });
  // Prompts are completed user messages; Codex may say them again in a
  // user_message event, which isn't one more.
  const said = (min, text) => ({ type: "event_msg", timestamp: at(min), payload: { type: "item_completed", item: { type: "UserMessage", id: `u${min}`, content: [{ type: "text", text, text_elements: [] }] } } });
  const lines = [
    { type: "session_meta", timestamp: at(0), payload: { id, model_provider: "openai" } },
    { type: "turn_context", timestamp: at(0), payload: { model: "gpt-5.6-sol", effort: "medium" } },
    { type: "event_msg", timestamp: at(0), payload: { type: "user_message", message: "compress the logs" } },
    said(0, "compress the logs"),
    { type: "response_item", timestamp: at(1), payload: { type: "function_call", name: "exec_command", call_id: "c1" } },
    { type: "response_item", timestamp: at(2), payload: { type: "function_call_output", call_id: "c1" } },
    count(2, 12000, { input_tokens: 11000, cached_input_tokens: 8000, output_tokens: 1000, total_tokens: 12000 }),
    count(2, 12000, { input_tokens: 11000, cached_input_tokens: 8000, output_tokens: 1000, total_tokens: 12000 }),
    { type: "event_msg", timestamp: at(3), payload: { type: "task_complete" } },
    { type: "turn_context", timestamp: at(5), payload: { model: "gpt-6-luna", effort: "low" } },
    said(5, "and the old ones"),
    { type: "event_msg", timestamp: at(5), payload: { type: "user_message", message: "a user_message alone isn't a prompt" } },
    count(6, 20000, { input_tokens: 7500, cached_input_tokens: 7000, output_tokens: 500, total_tokens: 8000 }),
    { type: "event_msg", timestamp: at(7), payload: { type: "task_complete" } },
  ];
  fs.writeFileSync(path.join(day1, `rollout-2026-10-01T12-00-00-${id}.jsonl.zst`), zlib.zstdCompressSync(Buffer.from(jsonl(lines))));
  const stats = createStats({ transcripts: new Transcripts({ codexRoots: [path.join(root, "sessions")] }) });
  const s = await stats(`codex:${id}`);
  assert.equal(s.totals.calls, 2, "the repeated token count is one call");
  assert.deepEqual(s.models.map((m) => [m.model, m.calls, m.efforts]), [["gpt-5.6-sol", 1, ["medium"]], ["gpt-6-luna", 1, ["low"]]]);
  assert.deepEqual([s.kinds.input.tokens, s.kinds.cacheRead.tokens, s.kinds.output.tokens], [3000 + 500, 8000 + 7000, 1500]);
  near(s.totals.cost, costOf("gpt-5.6-sol", Date.parse(at(2)), { input: 3000, cacheRead: 8000, output: 1000 }) + costOf("gpt-6-luna", Date.parse(at(6)), { input: 500, cacheRead: 7000, output: 500 }));
  assert.deepEqual(s.turns.map((t) => [t.prompt, t.end - t.start]), [["compress the logs", 3 * 60e3], ["and the old ones", 2 * 60e3]]);
  assert.deepEqual(s.tools.map((t) => [t.name, t.calls, t.ms]), [["exec_command", 1, 60e3]]);
  assert.equal(s.context[0].window, 272000);
});

test("a Codex thread on the priority tier: priced as the thread's settings say, turn by turn", async () => {
  const root = tmp();
  const id = "019a0000-aaaa-bbbb-cccc-000000000002";
  const day = path.join(root, "sessions", "2026", "10", "01");
  fs.mkdirSync(day, { recursive: true });
  const settings = (min, tier) => ({ type: "event_msg", timestamp: at(min), payload: { type: "thread_settings_applied", thread_id: id, thread_settings: { model: "gpt-5.5", model_provider_id: "openai", ...(tier ? { service_tier: tier } : {}) } } });
  const turn = (min) => ({ type: "turn_context", timestamp: at(min), payload: { model: "gpt-5.5", effort: "high" } });
  const call = (min, total) => ({ type: "event_msg", timestamp: at(min), payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage: { input_tokens: 1000, output_tokens: 100, total_tokens: 1100 } } } });
  const lines = [
    { type: "session_meta", timestamp: at(0), payload: { id, model_provider: "openai", cli_version: "0.150.0" } },
    settings(0, "priority"),
    // A turn that doesn't say the tier keeps the thread's.
    turn(0),
    call(1, 1100),
    settings(2, "default"),
    turn(2),
    call(3, 2200),
    // Settings that don't say a tier are on the default one.
    settings(4, "priority"),
    settings(4, null),
    turn(4),
    call(5, 3300),
  ];
  fs.writeFileSync(path.join(day, `rollout-2026-10-01T12-00-00-${id}.jsonl`), jsonl(lines));
  const s = await createStats({ transcripts: new Transcripts({ codexRoots: [path.join(root, "sessions")] }) })(`codex:${id}`);
  const u = { input: 1000, output: 100 };
  const standard = costOf("gpt-5.5", Date.parse(at(1)), u);
  near(s.totals.cost, costOf("gpt-5.5", Date.parse(at(1)), u, { priority: true }) + 2 * standard);
  assert.ok(costOf("gpt-5.5", Date.parse(at(1)), u, { priority: true }) > 2 * standard, "the priority tier costs more");
});

// A Codex rollout of made-up lines, as stats reads it.
async function codexStats(lines, { id = "019a0000-aaaa-bbbb-cccc-0000000000ff" } = {}) {
  const root = tmp();
  const day = path.join(root, "sessions", "2026", "10", "01");
  fs.mkdirSync(day, { recursive: true });
  fs.writeFileSync(path.join(day, `rollout-2026-10-01T12-00-00-${id}.jsonl`), jsonl(lines));
  return createStats({ transcripts: new Transcripts({ codexRoots: [path.join(root, "sessions")] }) })(`codex:${id}`);
}
const codexSaid = (min, text) => ({ type: "event_msg", timestamp: at(min), payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } } });
const codexCall = (min, total) => ({ type: "event_msg", timestamp: at(min), payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage: { input_tokens: 900, output_tokens: 100, total_tokens: 1000 } } } });

test("a Codex user message that starts with < (a delegation envelope) is no prompt, nor a turn", async () => {
  const s = await codexStats([
    { type: "turn_context", timestamp: at(0), payload: { model: "gpt-5.5" } },
    codexSaid(0, "Fix the flaky test"),
    codexCall(1, 1000),
    { type: "event_msg", timestamp: at(2), payload: { type: "task_complete" } },
    codexSaid(3, "<subagent_notification>a subagent finished</subagent_notification>"),
    codexCall(4, 2000),
  ]);
  assert.deepEqual(s.turns.map((t) => [t.prompt, t.end - t.start]), [["Fix the flaky test", 4 * 60e3]], "its call still counts, to the turn before");
  assert.equal(s.totals.calls, 2);
});

test("a Codex rollout with no completed user messages, as codex exec writes: its prompts are its user_message events", async () => {
  const userEvent = (min, message) => ({ type: "event_msg", timestamp: at(min), payload: { type: "user_message", message, images: [] } });
  const s = await codexStats([
    { type: "session_meta", timestamp: at(0), payload: { cli_version: "0.146.0", originator: "codex_exec" } },
    { type: "turn_context", timestamp: at(0), payload: { model: "gpt-5.5" } },
    userEvent(0, "summarize the failing tests"),
    codexCall(1, 1000),
    { type: "event_msg", timestamp: at(2), payload: { type: "task_complete" } },
    userEvent(3, "<environment_context>…</environment_context>"),
    userEvent(4, "and suggest fixes"),
    codexCall(5, 2000),
    { type: "event_msg", timestamp: at(6), payload: { type: "agent_message", message: "Done." } },
  ]);
  assert.deepEqual(s.turns.map((t) => [t.prompt, t.start, t.end]), [["summarize the failing tests", Date.parse(at(0)), Date.parse(at(2))], ["and suggest fixes", Date.parse(at(4)), Date.parse(at(5))]]);
});

test("a Codex local shell call is a shell call, by its type, as it has no name", async () => {
  const s = await codexStats([
    { type: "turn_context", timestamp: at(0), payload: { model: "gpt-5.5" } },
    codexSaid(0, "time the build"),
    { type: "response_item", timestamp: at(1), payload: { type: "local_shell_call", call_id: "c1", status: "completed", action: { type: "exec", command: ["bash", "-lc", "time make"] } } },
    { type: "response_item", timestamp: at(3), payload: { type: "local_shell_call_output", call_id: "c1", output: "real 2m" } },
  ]);
  assert.deepEqual(s.tools.map((t) => [t.name, t.category, t.calls, t.ms]), [["local_shell_call", "shell", 1, 2 * 60e3]]);
});
