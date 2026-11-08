// How numbers read (src/shared/units.js).
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { money, tokens, duration, share, excerpt } from "../src/shared/units.js";
const day = (s) => Date.parse(`${s}T12:00:00Z`);
const near = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ≠ ${b}`);

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

// A Codex rollout of made-up lines, as stats reads it.
const codexSaid = (min, text) => ({ type: "event_msg", timestamp: at(min), payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } } });
const codexCall = (min, total) => ({ type: "event_msg", timestamp: at(min), payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage: { input_tokens: 900, output_tokens: 100, total_tokens: 1000 } } } });

