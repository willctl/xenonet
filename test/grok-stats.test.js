// A Grok Build session's stats, search records and prices: its updates and
// usage ledger (test/fixtures/grok.js) read as the session details and the
// search index read them, and SpaceXAI's prices (core/pricing.js).
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { grokHome, IDS, MAIN_UPDATES, T } from "./fixtures/grok.js";
import { Transcripts, createStats } from "../core/stats.js";
import { TranscriptSearch, docsFromGrokLines } from "../core/search.js";
import { costParts, priceOf, plainModel } from "../core/pricing.js";
import { grokCost } from "../core/grok-usage.js";

const tmp = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-stats-")));
after(() => fs.rmSync(tmp, { recursive: true, force: true }));
const home = grokHome(path.join(tmp, "home"));
const grokRoot = path.join(home, "sessions");

test("a session's stats: its turn and tools from its updates, its calls and Grok's own cost from its ledger", async () => {
  const stats = createStats({ transcripts: new Transcripts({ claudeRoot: null, grokRoot }) });
  const s = await stats(`grok:${IDS.main}`);
  assert.equal(s.app, "grok");
  assert.deepEqual(s.totals, { calls: 2, input: 12000, cacheRead: 40000, cacheWrite: 0, output: 3000, tokens: 55000, cost: 0.12345, unpricedCalls: 0 });
  assert.ok(Math.abs(Object.values(s.kinds).reduce((a, k) => a + k.cost, 0) - 0.12345) < 1e-9, "Grok's cost, split as XenoNet's prices split it");
  assert.deepEqual(s.models.map((m) => [m.model, m.calls, m.cost]), [["grok-4.6", 2, 0.12345]]);
  assert.deepEqual(s.turns.map((t) => [t.prompt, t.tools]), [["Add partial refunds to the orders API.", 3]]);
  assert.deepEqual(s.tools.map((t) => [t.name, t.category, t.errors]), [
    ["run_terminal_command", "shell", 0],
    ["search_replace", "edit", 0],
    ["mcp__linear__save_issue", "mcp", 0],
  ]);
  assert.deepEqual(s.context.map((c) => c.window), [500000, 500000], "the window its usage updates said");
  // A session with no ledger: its turns and tools, no calls.
  const fork = await stats(`grok:${IDS.fork}`);
  assert.deepEqual([fork.totals.calls, fork.totals.cost, fork.turns.length], [0, null, 1]);
  assert.equal(await stats(`grok:019a3b5c-0000-7000-8000-0000000000ff`), null);
  // A ledger that changes changes the stats.
  const file = path.join(grokRoot, encodeURIComponent("/Users/ada/atlas"), IDS.fork, "usage.json");
  fs.writeFileSync(file, JSON.stringify({ turns: [{ turnNumber: 1, endedAt: new Date(T).toISOString(), model: "grok-build-0.1", inputTokens: 1000, outputTokens: 500, totalTokens: 1500 }] }));
  const again = await stats(`grok:${IDS.fork}`);
  assert.equal(again.totals.calls, 1);
  assert.ok(Math.abs(again.totals.cost - (1000 * 1 + 500 * 2) / 1e6) < 1e-12, "priced by XenoNet: Grok gave no cost");
});

test("search: each run of chunks is one message, and a session's updates are found by their words", async () => {
  assert.deepEqual(
    docsFromGrokLines(MAIN_UPDATES).map((d) => [d.role, d.text]),
    [
      ["user", "Add partial refunds to the orders API."],
      ["assistant", "I'll start with the tests."],
      ["assistant", "Partial refunds work. Should I open a PR?"],
    ],
  );
  const search = new TranscriptSearch({ dbFile: path.join(tmp, "search.sqlite"), claudeRoot: null, codexRoots: [], grokRoot });
  try {
    assert.equal(search.sessionOf(path.join(grokRoot, "g", IDS.main, "updates.jsonl"), "grok"), `grok:${IDS.main}`);
    assert.equal(search.sessionOf(path.join(grokRoot, "g", IDS.main, "chat_history.jsonl"), "grok"), null);
    assert.equal(search.sessionOf(path.join(grokRoot, "g", IDS.main, "compaction_checkpoints", "updates.jsonl"), "grok"), null);
    await search.index();
    const [hit] = search.search("partial refunds");
    assert.deepEqual([hit.sessionId, hit.app, hit.hits], [`grok:${IDS.main}`, "grok", 2]);
    assert.ok(search.search("feature flag").some((r) => r.sessionId === `grok:${IDS.fork}`));
  } finally {
    search.close();
  }
});

test("SpaceXAI's prices: per million tokens, the long-context rates from 200K, and Grok's own figure first", () => {
  const at = Date.parse("2026-10-10T12:00:00Z");
  const m = { input: 1e6, cacheRead: 0, cacheWrite5m: 0, cacheWrite1h: 0, output: 0 };
  assert.equal(plainModel("xai/grok-4.6"), "grok-4.6");
  for (const [model, input, cached, output] of [
    ["grok-4.6", 2, 0.5, 6],
    ["grok-4.5", 2, 0.3, 6],
    ["grok-build-0.1", 1, 0.2, 2],
    ["grok-build", 1, 0.2, 2],
    ["grok-4.3", 1.25, 0.2, 2.5],
    ["grok-4.20-0309-reasoning", 1.25, 0.2, 2.5],
    ["grok-4.6-latest", 2, 0.5, 6],
  ]) {
    const p = priceOf(model, at);
    assert.deepEqual([p.provider, p.input, p.read, p.output], ["xai", input, cached, output], model);
    // Under 200K prompt tokens, the standard rates; writing the cache costs input.
    assert.ok(Math.abs(costParts(model, at, { input: 100000, cacheRead: 50000, cacheWrite5m: 1000, cacheWrite1h: 0, output: 1000 }).total - (100000 * input + 50000 * cached + 1000 * input + 1000 * output) / 1e6) < 1e-12, model);
  }
  // From 200K: the whole call at the long-context rates, cached input too.
  assert.equal(costParts("grok-4.6", at, m).total, 4);
  assert.equal(costParts("grok-4.6", at, { ...m, input: 199999 }).total, (199999 * 2) / 1e6);
  assert.equal(costParts("grok-4.6", at, { input: 100000, cacheRead: 100000, cacheWrite5m: 0, cacheWrite1h: 0, output: 1000 }).total, (100000 * 4 + 100000 * 1 + 1000 * 12) / 1e6);
  // An older call gets the price xAI publishes now: there's no earlier one.
  assert.equal(priceOf("grok-4.6", Date.parse("2026-01-01")).input, 2);
  assert.equal(priceOf("grok-4", at), null, "a model without a published price");
  assert.equal(grokCost("grok-4.6", at, { cost: 0.5, input: 1e6 }), 0.5, "Grok's own figure");
  assert.equal(grokCost("grok-4.6", at, { input: 1000, output: 1000 }), (1000 * 2 + 1000 * 6) / 1e6);
  assert.equal(grokCost("grok-4", at, { input: 1000 }), null);
});
