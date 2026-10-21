// The experimental Cursor reader (core/native/cursor.js) and the hook that
// feeds it (scripts/cursor/hook.mjs): each Cursor agent conversation is a
// session on the map, "cursor:<conversation id>", with Cursor's
// afterFileEdit files as its edits. The hook input is made up, in the shape
// of Cursor's hooks reference.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { CursorSessions, cursorEventsFile, SOURCE } from "../core/native/cursor.js";
import { reader } from "../core/vendor.js";
import { observedSessions } from "../core/reported-runs.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { Session } from "../core/schema/session.js";

const HOOK = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "scripts", "cursor", "hook.mjs");
const CID = "6b0e5f1c-1111-4000-8000-000000000001";
const common = { conversation_id: CID, generation_id: "g1", model: "SECRET-MODEL", cursor_version: "2.4.1", workspace_roots: ["/w/atlas"], user_email: "SECRET@example.com", transcript_path: "/SECRET/t.txt" };

function hook(file, input) {
  const r = spawnSync(process.execPath, [HOOK, file], { input: JSON.stringify(input), encoding: "utf8" });
  assert.equal(r.status, 0);
  assert.equal(r.stdout.trim(), "{}");
}

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-cursor-"));
  const file = path.join(dir, "cursor-hooks.ndjson");
  return { dir, file, done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

test("the hook keeps only where, which file and when, never the prompt, edits, model or email, and never fails the agent", () => {
  const w = setup();
  try {
    hook(w.file, { ...common, hook_event_name: "sessionStart", session_id: CID, composer_mode: "agent" });
    hook(w.file, { ...common, hook_event_name: "afterFileEdit", file_path: "/w/atlas/notes.txt", edits: [{ old_string: "SECRET", new_string: "SECRET" }] });
    hook(w.file, { ...common, hook_event_name: "beforeSubmitPrompt", prompt: "SECRET PROMPT" });
    const r = spawnSync(process.execPath, [HOOK, w.file], { input: "not json", encoding: "utf8" });
    assert.deepEqual([r.status, r.stdout.trim()], [0, "{}"]);
    const text = fs.readFileSync(w.file, "utf8");
    assert.equal(text.trim().split("\n").length, 2, "only the hooks it's for");
    assert.doesNotMatch(text, /SECRET/);
    if (process.platform !== "win32") assert.equal(fs.statSync(w.file).mode & 0o777, 0o600);
  } finally {
    w.done();
  }
});

test("each Cursor conversation is a session, cursor:<id>, in its workspace, with its edits, ended at sessionEnd", () => {
  const w = setup();
  try {
    hook(w.file, { ...common, hook_event_name: "sessionStart", session_id: CID });
    hook(w.file, { ...common, hook_event_name: "afterFileEdit", file_path: "/w/atlas/notes.txt" });
    const now = Date.now();
    const c = new CursorSessions({ file: w.file }).read(now);
    const [s, ...rest] = c.sessions();
    assert.deepEqual(rest, []);
    assert.deepEqual([s.id, s.app, s.cwd, s.reader], [`cursor:${CID}`, "cursor", "/w/atlas", "experimental"]);
    assert.ok(Session.safeParse(s).success, JSON.stringify(Session.safeParse(s).error?.issues));
    assert.deepEqual([...c.live(s.id).edits.keys()], ["/w/atlas/notes.txt"]);
    assert.equal(c.live(s.id).endedAt, null);
    hook(w.file, { ...common, hook_event_name: "sessionEnd", session_id: CID, reason: "completed" });
    assert.ok(c.read(Date.now()).live(s.id).endedAt != null);
  } finally {
    w.done();
  }
});

test("a vendor check that throws is the reader's problem, and the file is read again once it changes", (t) => {
  const w = setup();
  try {
    hook(w.file, { ...common, hook_event_name: "sessionStart", session_id: CID });
    const check = t.mock.method(reader(SOURCE), "check", () => {
      throw new Error("a broken schema");
    });
    const c = new CursorSessions({ file: w.file }).read(Date.now());
    assert.equal(c.health().state, "degraded");
    assert.deepEqual([c.status().state, c.sessions()], ["ok", []]);
    check.mock.restore();
    c.read(Date.now());
    assert.deepEqual(c.sessions(), [], "the same file isn't parsed again");
    hook(w.file, { ...common, hook_event_name: "afterFileEdit", file_path: "/w/atlas/notes.txt" });
    assert.deepEqual(c.read(Date.now()).sessions().map((s) => s.id), [`cursor:${CID}`]);
    assert.ok(!["degraded", "failed"].includes(c.health().state), c.health().state);
  } finally {
    w.done();
  }
});

test("a missing file is said, not an error; and the file is where it's told to be", () => {
  const c = new CursorSessions({ file: path.join(os.tmpdir(), "xenonet-no-such-cursor.ndjson") }).read();
  assert.equal(c.status().state, "missing");
  assert.notEqual(c.health().state, "failed");
  assert.equal(cursorEventsFile({ env: {}, stateDir: "/s" }), path.join("/s", "cursor-hooks.ndjson"));
});

test("a session about to edit a file a Cursor session edited is warned, and can't message it", async () => {
  const w = setup();
  try {
    hook(w.file, { ...common, hook_event_name: "afterFileEdit", file_path: "/w/atlas/notes.txt" });
    const NOW = Date.now();
    const c = new CursorSessions({ file: w.file }).read(NOW);
    const observed = observedSessions([c]);
    const C = "claude:aaaaaaaa-0001";
    const live = { lastEventAt: NOW - 1e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([[C, { id: C, kind: "session", app: "claude", title: "me", cwd: "/w/atlas", updatedAt: NOW }], ...observed.sessions().map((s) => [s.id, { ...s, kind: "session" }])]);
    const state = { graph: {}, index, live: { sessions: new Map([[C, live]]) }, observed, homeOf: () => null };
    const engine = new Engine({ state, store: new EngineStore(":memory:"), now: () => NOW, repos: { of: async () => ({ top: "/w/atlas", common: "/w/atlas/.git", branch: "main" }) } });
    assert.deepEqual((await engine.conflicts(C, ["/w/atlas/notes.txt"])).map((x) => x.agent.id), [`cursor:${CID}`]);
    await assert.rejects(engine.send(C, { to: `cursor:${CID}`, text: "hi" }), /can't read messages/);
  } finally {
    w.done();
  }
});
