// The experimental OpenCode reader (core/native/opencode.js): each OpenCode
// session in its opencode.db is a session on the map, "opencode:<id>", and
// its edit tools' files count in other sessions' warnings. Nothing else of
// a part is kept. The database is made up (test/fixtures/opencode.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { OpenCodeSessions, readDb, patchFiles, sessionIdOf, opencodeDbPath } from "../core/native/opencode.js";
import { observedSessions } from "../core/reported-runs.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { actionsFor } from "../core/open.js";
import { Session } from "../core/schema/session.js";
import { drift } from "../core/drift.js";
import { opencodeDb, tool } from "./fixtures/opencode.js";

const NOW = Date.parse("2026-10-16T12:00:00Z");
const A = "ses_eb95b1836ffexD4vYDGuLeJDCy";
const B = "ses_eb95b1813ffeICZZm6NrzxiEWc";
const SUB = "ses_eb95b1813ffeSubAgent0001";

function setup(opts) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-opencode-"));
  const file = path.join(dir, "opencode.db");
  if (opts) opencodeDb(file, opts);
  return { dir, file, done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

const world = () => ({
  sessions: [
    { id: A, directory: "/w/atlas", title: "Overwrite notes", created: NOW - 600e3, updated: NOW - 60e3 },
    { id: SUB, parent: A, directory: "/w/atlas", created: NOW - 300e3 },
    { id: "ses_old", directory: "/w/atlas", created: NOW - 3 * 86400e3 },
  ],
  parts: [
    { session: A, at: NOW - 120e3, data: tool("write", { filePath: "/w/atlas/notes.txt", content: "SECRET CONTENT" }) },
    { session: A, at: NOW - 110e3, data: tool("edit", { filePath: "src/app.ts", oldString: "a", newString: "b" }) },
    { session: A, at: NOW - 100e3, data: tool("apply_patch", { patchText: "*** Begin Patch\n*** Update File: src/lib.ts\n@@\n-SECRET\n+SECRET\n*** End Patch" }) },
    { session: A, at: NOW - 90e3, data: tool("write", { filePath: "/w/atlas/failed.txt" }, "error") },
    { session: A, at: NOW - 80e3, data: tool("read", { filePath: "/w/atlas/read-only.txt" }) },
    { session: A, at: NOW - 70e3, data: "not json" },
    { session: SUB, at: NOW - 3600e3, data: tool("write", { filePath: "/w/atlas/stale.txt" }) },
  ],
});

test("each OpenCode session is a session, opencode:<id>, read from its own database, with subagents under their parent", () => {
  const w = setup(world());
  try {
    const o = new OpenCodeSessions({ file: w.file }).read(NOW);
    assert.equal(o.status().state, "ok");
    assert.equal(o.status().version, "1.19.3");
    const list = o.sessions().sort((a, b) => a.createdAt - b.createdAt);
    assert.deepEqual(list.map((s) => s.id), [sessionIdOf(A), sessionIdOf(SUB)], "the one older than a day is left out");
    assert.equal(list[0].id, "opencode:ses-eb95b1836ffexD4vYDGuLeJDCy");
    assert.equal(list[0].rawId, A);
    assert.equal(list[0].title, "Overwrite notes");
    assert.equal(list[0].reader, "experimental");
    assert.equal(list[0].observed, undefined, "read, not observed");
    assert.deepEqual([list[1].parentId, list[1].parentKind], [sessionIdOf(A), "subagent"]);
    for (const s of list) assert.ok(Session.safeParse(s).success, JSON.stringify(Session.safeParse(s).error?.issues));
    assert.deepEqual(actionsFor({ ...list[0], kind: "session" }).map((a) => a.id), ["open-folder"]);
  } finally {
    w.done();
  }
});

test("its edits are the edit tools' files, relative ones joined to where it ran; failed calls, reads and old edits aren't, and nothing else is kept", () => {
  const w = setup(world());
  try {
    const o = new OpenCodeSessions({ file: w.file }).read(NOW);
    const live = o.live(sessionIdOf(A));
    assert.deepEqual([...live.edits.keys()].sort(), ["/w/atlas/notes.txt", "/w/atlas/src/app.ts", "/w/atlas/src/lib.ts"]);
    assert.equal(live.observed, false);
    assert.deepEqual([...o.live(sessionIdOf(SUB)).edits.keys()], []);
    const raw = readDb(w.file, { sinceMs: NOW - 86400e3 });
    assert.doesNotMatch(JSON.stringify(raw), /SECRET/);
  } finally {
    w.done();
  }
});

test("apply_patch's files come from its headers alone", () => {
  assert.deepEqual(patchFiles("*** Begin Patch\n*** Add File: a.txt\n+x\n*** Update File: b.txt\n*** Move to: c.txt\n*** Delete File: d.txt\n*** End Patch"), ["a.txt", "b.txt", "c.txt", "d.txt"]);
  assert.deepEqual(patchFiles(null), []);
});

test("a missing database is said, and one in another layout is drift, not an error", () => {
  const none = setup();
  try {
    const o = new OpenCodeSessions({ file: none.file }).read(NOW);
    assert.equal(o.status().state, "missing");
    assert.notEqual(o.health().state, "failed");
  } finally {
    none.done();
  }
  const odd = setup({ layout: "old" });
  try {
    const before = drift.list?.().length ?? 0;
    const o = new OpenCodeSessions({ file: odd.file }).read(NOW);
    assert.equal(o.status().state, "unreadable");
    assert.deepEqual(o.sessions(), []);
    if (drift.list) assert.ok(drift.list().length > before, "the missing columns are drift");
  } finally {
    odd.done();
  }
});

test("the database is where OpenCode keeps it", () => {
  assert.equal(opencodeDbPath({ home: "/home/ada", env: {} }), "/home/ada/.local/share/opencode/opencode.db");
  assert.equal(opencodeDbPath({ home: "/home/ada", env: { XDG_DATA_HOME: "/data" } }), "/data/opencode/opencode.db");
  assert.equal(opencodeDbPath({ home: "/home/ada", env: { OPENCODE_DB: "/x/o.db" } }), "/x/o.db");
});

test("a session about to edit a file an OpenCode session edited is warned, and can't message it", async () => {
  const w = setup(world());
  try {
    const o = new OpenCodeSessions({ file: w.file }).read(NOW);
    const t = { now: NOW };
    const now = () => t.now;
    const C = "claude:aaaaaaaa-0001";
    const O = sessionIdOf(A);
    const live = { lastEventAt: NOW - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([
      ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
      [C, { id: C, kind: "session", app: "claude", title: "Add partial refunds", cwd: "/w/atlas", updatedAt: live.lastEventAt }],
      ...o.sessions().map((s) => [s.id, { ...s, kind: "session" }]),
    ]);
    const state = { graph: {}, index, live: { sessions: new Map([[C, live]]) }, observed: observedSessions([o]), homeOf: (id) => (index.get(id)?.cwd === "/w/atlas" ? "project:atlas" : null) };
    const engine = new Engine({ state, store: new EngineStore(":memory:"), now, repos: { of: async (cwd) => (cwd?.startsWith("/w/atlas") ? { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" } : null) } });
    const conflicts = await engine.conflicts(C, ["/w/atlas/notes.txt"]);
    assert.equal(conflicts.length, 1, JSON.stringify(conflicts));
    assert.equal(conflicts[0].agent.id, O);
    assert.equal(conflicts[0].agent.app, "opencode");
    assert.equal(conflicts[0].observed, undefined, "read from its records, not inferred");
    await assert.rejects(engine.send(C, { to: O, text: "hi" }), /can't read messages/);
  } finally {
    w.done();
  }
});
