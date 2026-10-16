// The experimental Copilot CLI reader (core/native/copilot-cli.js): each
// session's event log is a session on the map, "copilot:<session id>", with
// its edit tools' files and its shutdown's changed files as its edits. The
// events are made up, in the shape of the Copilot SDK's published schema.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { CopilotCliSessions, readEvents, editedBy, copilotRoot } from "../core/native/copilot-cli.js";
import { observedSessions } from "../core/reported-runs.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { Session } from "../core/schema/session.js";

const NOW = Date.parse("2026-10-16T12:00:00Z");
const iso = (dt) => new Date(NOW + dt).toISOString();
const SID = "0f3c2b1a-1111-4000-8000-000000000001";
const OLD = "0f3c2b1a-2222-4000-8000-000000000002";
let n = 0;
const ev = (type, data, dt, extra = {}) => ({ id: `e${++n}`, timestamp: iso(dt), parentId: null, type, data, ...extra });
const lines = (...rs) => rs.map((r) => JSON.stringify(r)).join("\n") + "\n";

function setup() {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-copilot-"));
  const root = copilotRoot({ home });
  fs.mkdirSync(path.join(root, SID), { recursive: true });
  const file = path.join(root, SID, "events.jsonl");
  fs.writeFileSync(
    file,
    lines(
      ev("session.start", { sessionId: SID, version: 1, producer: "copilot-agent", copilotVersion: "1.0.98", startTime: iso(-600e3), context: { cwd: "/w/atlas", branch: "main" } }, -600e3),
      ev("user.message", { content: "SECRET PROMPT" }, -500e3),
      ev("session.title_changed", { title: "Partial refunds" }, -490e3),
      ev("tool.execution_start", { toolCallId: "t1", toolName: "str_replace_editor", arguments: { command: "str_replace", path: "src/app.ts", old_str: "SECRET", new_str: "SECRET" } }, -120e3),
      ev("tool.execution_start", { toolCallId: "t2", toolName: "str_replace_editor", arguments: { command: "view", path: "/w/atlas/read.txt" } }, -110e3),
      ev("tool.execution_start", { toolCallId: "t3", toolName: "bash", arguments: { command: "echo SECRET > /w/atlas/x" } }, -100e3),
      ev("tool.execution_complete", { toolCallId: "t1", result: { content: "SECRET RESULT" } }, -99e3),
      ev("session.shutdown", { shutdownType: "routine", codeChanges: { filesModified: ["/w/atlas/notes.txt"], linesAdded: 1, linesRemoved: 0 }, modelMetrics: {} }, -60e3),
    ),
  );
  fs.utimesSync(file, (NOW - 60e3) / 1000, (NOW - 60e3) / 1000);
  // The layout before 1.0.11, from two days ago: too old.
  const old = path.join(root, `${OLD}.jsonl`);
  fs.writeFileSync(old, lines(ev("session.start", { sessionId: OLD, version: 1, startTime: iso(-2 * 86400e3), context: { cwd: "/w/atlas" } }, -2 * 86400e3)));
  fs.utimesSync(old, (NOW - 2 * 86400e3) / 1000, (NOW - 2 * 86400e3) / 1000);
  return { home, root, done: () => fs.rmSync(home, { recursive: true, force: true }) };
}

test("each Copilot CLI session is a session, copilot:<id>, in its working folder, ended at its shutdown", () => {
  const w = setup();
  try {
    const c = new CopilotCliSessions({ root: w.root }).read(NOW);
    const [s, ...rest] = c.sessions();
    assert.deepEqual(rest, [], "the one older than a day is left out");
    assert.equal(s.id, `copilot:${SID}`);
    assert.deepEqual([s.title, s.cwd, s.branch, s.reader], ["Partial refunds", "/w/atlas", "main", "experimental"]);
    assert.ok(Session.safeParse(s).success, JSON.stringify(Session.safeParse(s).error?.issues));
    assert.equal(c.live(s.id).endedAt, NOW - 60e3);
  } finally {
    w.done();
  }
});

test("its edits are its edit tools' files and its shutdown's, not views or shell commands, and nothing else is kept", () => {
  const w = setup();
  try {
    const c = new CopilotCliSessions({ root: w.root }).read(NOW);
    assert.deepEqual([...c.live(`copilot:${SID}`).edits.keys()].sort(), ["/w/atlas/notes.txt", "/w/atlas/src/app.ts"]);
    assert.doesNotMatch(JSON.stringify([...c.runs.values()].map((r) => ({ ...r, edits: [...r.edits] }))), /SECRET/);
  } finally {
    w.done();
  }
});

test("which tool calls are edits", () => {
  assert.deepEqual(editedBy("create", { path: "/a" }), ["/a"]);
  assert.deepEqual(editedBy("apply_patch", "*** Begin Patch\n*** Update File: b.ts\n*** End Patch"), ["b.ts"]);
  assert.deepEqual(editedBy("view", { path: "/a" }), []);
  assert.equal(readEvents(lines({ id: "x", timestamp: iso(0), type: "user.message", data: {} })), null, "no session.start, no session");
});

test("a missing folder is said, not an error", () => {
  const c = new CopilotCliSessions({ root: path.join(os.tmpdir(), "xenonet-no-such-copilot") }).read(NOW);
  assert.equal(c.status().state, "missing");
  assert.notEqual(c.health().state, "failed");
});

test("a session about to edit a file a Copilot CLI session still at work edited is warned, and can't message it", async () => {
  const w = setup();
  try {
    // Still at work: no shutdown yet.
    const f = path.join(w.root, SID, "events.jsonl");
    fs.writeFileSync(f, fs.readFileSync(f, "utf8").split("\n").filter((l) => !l.includes("session.shutdown")).join("\n"));
    fs.utimesSync(f, (NOW - 60e3) / 1000, (NOW - 60e3) / 1000);
    const c = new CopilotCliSessions({ root: w.root }).read(NOW);
    const observed = observedSessions([c]);
    const C = "claude:aaaaaaaa-0001";
    const live = { lastEventAt: NOW - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([[C, { id: C, kind: "session", app: "claude", title: "me", cwd: "/w/atlas", updatedAt: NOW }], ...observed.sessions().map((s) => [s.id, { ...s, kind: "session" }])]);
    const state = { graph: {}, index, live: { sessions: new Map([[C, live]]) }, observed, homeOf: () => null };
    const engine = new Engine({ state, store: new EngineStore(":memory:"), now: () => NOW, repos: { of: async () => ({ top: "/w/atlas", common: "/w/atlas/.git", branch: "main" }) } });
    const conflicts = await engine.conflicts(C, ["/w/atlas/src/app.ts"]);
    assert.deepEqual(conflicts.map((x) => x.agent.id), [`copilot:${SID}`]);
    await assert.rejects(engine.send(C, { to: `copilot:${SID}`, text: "hi" }), /can't read messages/);
  } finally {
    w.done();
  }
});
