// Grok Build's sessions (core/grok.js): listed from their folders, with
// their titles, folders and lineage, and a session's conversation as the
// console shows it.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { listGrokSessions, groupCwd, grokTime, grokFolder, grokEntries, readGrokSession } from "../core/grok.js";
import { check, explain } from "../core/schema/index.js";
import { GrokLine } from "../core/lines.js";
import { grokHome, IDS, ATLAS, LONG, T, MAIN_UPDATES } from "./fixtures/grok.js";

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
  assert.deepEqual([byId[IDS.sub].parentId, byId[IDS.sub].parentKind, byId[IDS.sub].background], [`grok:${IDS.main}`, "subagent", false]);
  assert.deepEqual([byId[IDS.fork].parentKind, byId[IDS.fork].title], ["fork", "Refunds, the careful version"]);
  assert.equal(byId[IDS.headless].background, true, "a headless run is background work");
  assert.deepEqual([byId[IDS.long].cwd, byId[IDS.long].title, byId[IDS.long].preview], [LONG, "Reconcile the ledger", "Reconcile the ledger"]);
  assert.deepEqual([byId[IDS.bare].title, typeof byId[IDS.bare].updatedAt], ["(untitled session)", "number"], "a session whose summary isn't written yet");
  assert.deepEqual(listGrokSessions({ home: path.join(dir, "nowhere") }), []);
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
  assert.deepEqual(entries.map((e) => e.role), ["user", "assistant", "tool", "tool", "tool", "assistant"]);
  assert.deepEqual(entries.filter((e) => e.role === "tool").map((e) => [e.title, e.status]), [["Execute `npm test -- orders`", "completed"], ["Edit refunds.ts", "completed"], ["linear__save_issue", "failed"]]);
  const page = await readGrokSession(IDS.main, { home, limit: 4 });
  assert.deepEqual([page.entries.length, page.cursor, page.total], [4, "4", 6]);
  const rest = await readGrokSession(IDS.main, { home, limit: 4, cursor: page.cursor });
  assert.deepEqual([rest.entries.map((e) => e.role), rest.cursor], [["user", "assistant"], null]);
  await assert.rejects(readGrokSession("019a3b5c-0000-7000-8000-000000000999", { home }), /can't find/);
});
