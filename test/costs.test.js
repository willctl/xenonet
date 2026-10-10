// About what today's calls would cost (core/live.js costs, at
// core/pricing.js's API prices, as the stats panel prices them), and what
// OpenRouter's key has spent (core/openrouter-usage.js): Claude's replies
// once each with their cache writes and fast mode, Codex's calls once each
// with cached input inside input and its priority tier, models without a
// price named; the key only in a header, once a minute. Keys are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { costParts, plainModel, priceOf, PRICES_CHECKED } from "../core/pricing.js";
import { createOpenRouterUsage, usageOf, KEY_URL } from "../core/openrouter-usage.js";
import { KEY_TARGET } from "../core/decide.js";
import { createMemorySecrets } from "../core/secret-store.js";
import { LiveTelemetry } from "../core/live.js";

const close = (a, b, what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} vs ${b}`);

test("today's cost: Claude's replies once each, Codex's calls once each on their tier, and models without a price named", () => {
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  const now = Date.now();
  const ts = new Date(now).toISOString();
  const reply = (id, model, usage) => ({ type: "assistant", timestamp: ts, message: { id, model, role: "assistant", content: [{ type: "text", text: "ok" }], usage } });
  const f = { sessionId: "claude:11111111-2222-3333-4444-555555555555" };
  const opusUsage = { input_tokens: 1000, cache_read_input_tokens: 100000, cache_creation_input_tokens: 3000, cache_creation: { ephemeral_5m_input_tokens: 2000, ephemeral_1h_input_tokens: 1000 }, output_tokens: 500 };
  live.applyClaude(f, reply("msg_1", "claude-opus-5-5", opusUsage), false);
  live.applyClaude(f, reply("msg_1", "claude-opus-5-5", opusUsage), false);
  live.applyClaude(f, reply("msg_2", "claude-opus-5-5", { input_tokens: 10, output_tokens: 100, speed: "fast" }), false);
  live.applyClaude(f, reply("msg_3", "claude-made-up-9", { input_tokens: 10, output_tokens: 10 }), false);
  const opus = costParts("claude-opus-5-5", now, { input: 1000, cacheRead: 100000, cacheWrite5m: 2000, cacheWrite1h: 1000, output: 500 }).total + costParts("claude-opus-5-5", now, { input: 10, output: 100 }, { fast: true }).total;

  const cx = { sessionId: "codex:0190ffff-0000-7000-8000-000000000001" };
  live.applyCodex(cx, { type: "session_meta", timestamp: ts, payload: { id: "x", model_provider: "openai" } }, false);
  live.applyCodex(cx, { type: "turn_context", timestamp: ts, payload: { model: "gpt-5.6-sol" } }, false);
  const count = (total) => ({ type: "event_msg", timestamp: ts, payload: { type: "token_count", info: { total_token_usage: { total_tokens: total }, last_token_usage: { input_tokens: 2000, cached_input_tokens: 1500, output_tokens: 1000, total_tokens: 3000 } } } });
  live.applyCodex(cx, count(3000), false);
  live.applyCodex(cx, count(3000), false);
  live.applyCodex(cx, { type: "event_msg", timestamp: ts, payload: { type: "thread_settings_applied", thread_settings: { service_tier: "priority" } } }, false);
  live.applyCodex(cx, count(6000), false);
  const call = { input: 500, cacheRead: 1500, cacheWrite5m: 0, cacheWrite1h: 0, output: 1000 };
  const sol = costParts("gpt-5.6-sol", now, call).total + costParts("gpt-5.6-sol", now, call, { priority: true }).total;
  assert.ok(sol > 0 && opus > 0, "both models have prices");

  const c = live.costs(now, []);
  close(c.byApp.claude, opus, "claude");
  close(c.byApp.codex, sol, "codex");
  close(c.today, opus + sol, "today");
  assert.deepEqual(c.unpriced, ["claude-made-up-9"]);
  assert.equal(c.checked, PRICES_CHECKED);
  // At this pace: each app's tokens a minute at what its tokens cost today.
  const claudeTokens = 1000 + 100000 + 3000 + 500 + 10 + 100;
  close(live.costs(now, [{ app: "claude", tpm: 1000 }]).perHour, 1000 * 60 * (opus / claudeTokens), "pace");
  // Yesterday's calls aren't today's.
  close(live.costs(now + 86400e3, []).today, 0, "tomorrow");
});

// Each model's price rows are worked out once and kept: the price on a
// day must come out the same asked once or again, in any order, and after
// more models than are kept.
test("a model's price on a day is the same however often it's asked, and after many other models", () => {
  const models = ["claude-opus-4-1-20250805", "us.anthropic.claude-sonnet-4-5-20250929-v1:0", "claude-sonnet-4-5@20250929", "claude-opus-4-6[1m]", "Claude-Haiku-4-5", "claude-3-5-haiku", "o3", "openai/gpt-5.6", "gpt-5.6-terra", "gpt-5.1-codex-mini", "claude-made-up-9", "<synthetic>", "", null, undefined, 42];
  const days = ["2024-11-10", "2024-12-10", "2025-05-01", "2025-07-01", "2026-07-20", "2026-08-25", "2026-10-03"].map((d) => Date.parse(`${d}T12:00:00Z`));
  const u = { input: 300000, cacheRead: 5000, cacheWrite5m: 100, cacheWrite1h: 50, output: 900 };
  const ask = () => models.flatMap((m) => days.map((at) => [plainModel(m), priceOf(m, at), costParts(m, at, u)]));
  const first = ask();
  assert.deepEqual(first.find(([id]) => id === "claude-opus-4-1-20250805")?.[1]?.ids, ["claude-opus-4-1-20250805", "claude-opus-4-1"]);
  assert.equal(plainModel("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), "claude-sonnet-4-5-20250929");
  assert.equal(priceOf("claude-3-5-haiku", Date.parse("2024-11-10T12:00:00Z")).input, 1);
  assert.equal(priceOf("claude-3-5-haiku", Date.parse("2024-12-10T12:00:00Z")).input, 0.8);
  assert.equal(priceOf("o3", Date.parse("2025-01-01T12:00:00Z")).input, 10);
  assert.equal(priceOf("claude-made-up-9"), null);
  assert.deepEqual(ask(), first);
  for (let i = 0; i < 1200; i++) priceOf(`made-up-model-${i}`, days[0]);
  assert.deepEqual(ask(), first);
  assert.deepEqual([...models].reverse().flatMap((m) => [...days].reverse().map((at) => [plainModel(m), priceOf(m, at), costParts(m, at, u)])), [...first].reverse());
});

test("OpenRouter's key: what it spent, with the key only in a header, once a minute, and none without a key", async () => {
  const secrets = createMemorySecrets();
  let t = 1_800_000_000_000;
  const calls = [];
  let status = 200;
  const fetch = async (url, init) => {
    calls.push({ url, auth: init.headers.authorization });
    return { ok: status === 200, status, json: async () => ({ data: { usage: 63.75, usage_daily: 0.84, usage_weekly: 6.12, usage_monthly: 21.4, limit: 50, limit_remaining: 28.6, limit_reset: "monthly", is_free_tier: false, label: "sk-or-v1-abc...xyz" } }) };
  };
  const usage = createOpenRouterUsage({ secrets, fetch, now: () => t });
  assert.deepEqual(await usage.status(), { key: "missing" });
  assert.equal(calls.length, 0);
  await secrets.set(KEY_TARGET, "sk-or-v1-made-up-key-0123456789abcdef");
  assert.deepEqual(await usage.status(), { key: "missing" }, "the answer stands for a minute");
  usage.forget();
  const r = await usage.status();
  assert.deepEqual(r, { key: "stored", at: t, today: 0.84, week: 6.12, month: 21.4, total: 63.75, limit: 50, remaining: 28.6, reset: "monthly", freeTier: false });
  assert.deepEqual(calls, [{ url: KEY_URL, auth: "Bearer sk-or-v1-made-up-key-0123456789abcdef" }]);
  assert.ok(!JSON.stringify(r).includes("sk-or"), "the key and its label stay out of what the page gets");
  t += 30e3;
  await usage.status();
  assert.equal(calls.length, 1);
  t += 61e3;
  status = 401;
  assert.deepEqual(await usage.status(), { key: "rejected", error: "OpenRouter didn't take the key" });
  assert.equal(usageOf({ data: { usage_daily: "x", limit: null } }).limit, null);
  assert.equal(usageOf({ data: { usage_daily: "x" } }).today, null);
});
