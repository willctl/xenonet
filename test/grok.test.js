// Grok Build's sessions (core/grok.js): listed from their folders, with
// their titles, folders and lineage, and a session's conversation as the
// console shows it.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { listGrokSessions, groupCwd, grokTime, grokFolder, grokEntries, readGrokSession } from "../core/grok.js";
import { readGrokUsage, grokUsageTurns, grokUsageStamp, grokSubagent } from "../core/grok-usage.js";
import { check, explain } from "../core/schema/index.js";
import { GrokLine } from "../core/lines.js";
import { grokHome, IDS, ATLAS, LONG, T, MAIN_UPDATES, up, text } from "./fixtures/grok.js";

const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-"));
after(() => fs.rmSync(dir, { recursive: true, force: true }));
const home = grokHome(path.join(dir, "grok"));

test("Grok Build's sessions, from their folders", () => {
  const list = listGrokSessions({ home });
  const byId = Object.fromEntries(list.map((s) => [s.rawId, s]));
  assert.deepEqual(list.map((s) => s.id).sort(), Object.values(IDS).map((id) => `grok:${id}`).sort());
  for (const s of list) {
    const r = check("session/v1", s, { strict: true });
    assert.ok(r.ok, `${s.id} fits session/v1:\n${explain(r)}`);
  }
  const main = byId[IDS.main];
  assert.deepEqual([main.title, main.cwd, main.branch, main.model, main.updatedAt, main.parentId], ["Add partial refunds", ATLAS, "feat/refunds", "grok-build", T + 14000, null]);
  assert.deepEqual([byId[IDS.sub].parentId, byId[IDS.sub].parentKind, byId[IDS.sub].background, byId[IDS.sub].agentKind], [`grok:${IDS.main}`, "subagent", false, "explore"]);
  assert.equal(byId[IDS.fork].agentKind, undefined, "a session no meta.json names has no subagent type");
  assert.deepEqual([byId[IDS.fork].parentKind, byId[IDS.fork].title], ["fork", "Refunds, the careful version"]);
  assert.equal(byId[IDS.headless].background, true, "a headless run is background work");
  assert.deepEqual([byId[IDS.long].cwd, byId[IDS.long].title, byId[IDS.long].preview], [LONG, "Reconcile the ledger", "Reconcile the ledger"]);
  assert.deepEqual([byId[IDS.bare].title, typeof byId[IDS.bare].updatedAt], ["(untitled session)", "number"], "a session whose summary isn't written yet");
  assert.deepEqual(listGrokSessions({ home: path.join(dir, "nowhere") }), []);
});

test("a subagent's session is its parent's by what the parent's folder says, before its own summary does", () => {
  const other = path.join(dir, "grok-meta");
  const folder = (id) => path.join(other, "sessions", encodeURIComponent(ATLAS), id);
  const write = (file, value) => {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, typeof value === "string" ? value : JSON.stringify(value));
  };
  write(path.join(folder("p"), "summary.json"), { info: { id: "p", cwd: ATLAS }, generated_title: "Refunds" });
  // Grok's cloud copy names them in camel case; a meta.json that isn't JSON
  // is passed over.
  write(path.join(folder("p"), "subagents", "sa-1", "meta.json"), { subagentId: "sa-1", parentSessionId: "p", childSessionId: "c", subagentType: "general-purpose", description: "Check the refund tests" });
  write(path.join(folder("p"), "subagents", "sa-2", "meta.json"), "{");
  write(path.join(folder("c"), "updates.jsonl"), "");
  const c = listGrokSessions({ home: other }).find((s) => s.rawId === "c");
  assert.deepEqual([c.parentId, c.parentKind, c.agentKind, c.title], ["grok:p", "subagent", "general-purpose", "Check the refund tests"]);
  assert.ok(check("session/v1", c, { strict: true }).ok);
});

test("a group's folder, and Grok's times", () => {
  assert.equal(groupCwd(path.join(dir, "x"), encodeURIComponent("/Users/ada/atlas")), "/Users/ada/atlas");
  assert.equal(groupCwd(path.join(dir, "x"), "%E0%A4%A"), null, "a name that doesn't decode");
  assert.equal(groupCwd(path.join(dir, "x"), "relative%20name"), null);
  assert.deepEqual([grokTime("2026-10-09T15:00:00Z"), grokTime(1791558000), grokTime("soon")], [1791558000000, 1791558000000, null]);
  assert.equal(grokFolder(IDS.fork, { home }), path.join(home, "sessions", encodeURIComponent(ATLAS), IDS.fork));
});

test("a session's conversation, a page at a time", async () => {
  const entries = grokEntries(MAIN_UPDATES.map((u) => new GrokLine(u)));
  assert.deepEqual(entries.map((e) => e.role), ["user", "reasoning", "assistant", "tool", "tool", "tool", "assistant"]);
  // A run of chunks is one entry, with all its text, at its first chunk.
  assert.deepEqual(entries.slice(0, 3).map((e) => [e.id, e.text]), [
    ["update-0", "Add partial refunds to the orders API."],
    ["update-2", "Look at the refund route first."],
    ["update-3", "I'll start with the tests. Then the route."],
  ]);
  assert.deepEqual(entries.filter((e) => e.role === "tool").map((e) => [e.title, e.status]), [["Execute `npm test -- orders`", "completed"], ["Edit refunds.ts", "completed"], ["linear__save_issue", "failed"]]);
  const page = await readGrokSession(IDS.main, { home, limit: 4 });
  assert.deepEqual([page.entries.length, page.cursor, page.total], [4, "4", 7]);
  const rest = await readGrokSession(IDS.main, { home, limit: 4, cursor: page.cursor });
  assert.deepEqual([rest.entries.map((e) => e.role), rest.cursor], [["user", "reasoning", "assistant"], null]);
  // A run whose first chunks say nothing begins with its first words.
  const late = grokEntries([up(T, text("agent_message_chunk", "")), up(T + 1000, text("agent_message_chunk", "Done.")), up(T + 2000, text("agent_message_chunk", " All green."))].map((u) => new GrokLine(u)));
  assert.deepEqual(late.map((e) => [e.id, e.role, e.text]), [["update-1", "assistant", "Done. All green."]]);
  await assert.rejects(readGrokSession("019a3b5c-0000-7000-8000-000000000999", { home }), /can't find/);
});

test("a session's turns from its usage.json: by model, when each ended, and a fork's own", () => {
  const read = (id) => readGrokUsage(grokFolder(id, { home }));
  const main = read(IDS.main);
  assert.deepEqual(grokUsageTurns(main.usage, main.summary), [
    // Grok's input is the whole prompt; XenoNet counts what came from the
    // cache apart. With no cost recorded, the cost isn't known.
    { t: T + 14500, key: "1|grok-build", model: "grok-build", calls: 4, tokens: { input: 12000, cacheRead: 40000, cacheWrite5m: 0, cacheWrite1h: 0, output: 1800 }, cost: null },
  ]);
  // A fork's ledger starts as a copy of its parent's: the turn that ended
  // before it was made is the parent's.
  const fork = read(IDS.fork);
  assert.deepEqual(
    grokUsageTurns(fork.usage, fork.summary).map((c) => [c.key, c.t, c.calls, c.tokens.input, c.cost]),
    [
      ["2|grok-build", T - 60e3, 3, 5000, 0.012],
      ["2|grok-build-mini", T - 60e3, 1, 4000, 0.0002],
    ],
  );
  assert.equal(grokUsageTurns(fork.usage, {}).length, 3, "a session that came from none has all its turns");
  const sub = read(IDS.sub);
  assert.deepEqual([grokSubagent(sub.summary), grokSubagent(main.summary), grokSubagent(null)], [true, false, false]);
  // A partial cost isn't the cost; a turn with no models falls back to its
  // primary one, then the session's; one with no end is left out; and what
  // isn't a number counts as none.
  const odd = {
    turns: [
      { turnNumber: 4, endedAt: "2026-10-09T15:01:00Z", inputTokens: 100, outputTokens: 10, costUsdTicks: 5e6, costIsPartial: true },
      { turnNumber: 5, endedAt: "2026-10-09T15:02:00Z", primaryModelId: "grok-4.7", inputTokens: "100", outputTokens: 10, cachedReadTokens: 400, costUsdTicks: 5e6 },
      { turnNumber: 6, inputTokens: 100 },
      "not a turn",
    ],
  };
  assert.deepEqual(
    grokUsageTurns(odd, { current_model_id: "grok-build" }).map((c) => [c.key, c.model, c.calls, c.tokens.input, c.tokens.cacheRead, c.tokens.output, c.cost]),
    [
      ["4|grok-build", "grok-build", 1, 100, 0, 10, null],
      ["5|grok-4.7", "grok-4.7", 1, 0, 400, 10, 0.0005],
    ],
  );
  assert.deepEqual([grokUsageTurns(null, null), grokUsageTurns({ turns: 5 }, "x")], [[], []]);
  assert.equal(grokUsageStamp(path.join(home, "nowhere")), null);
  assert.match(grokUsageStamp(grokFolder(IDS.main, { home })), /^\d+:\d+(\.\d+)?$/);
  assert.deepEqual(readGrokUsage(path.join(home, "nowhere")), { usage: null, summary: null });
});
