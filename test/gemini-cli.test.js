// The experimental Gemini CLI reader (core/native/gemini-cli.js): each
// session in Gemini CLI's chat records is a session on the map,
// "gemini-cli:<sessionId>", with its write_file and replace calls' files
// as its edits. The records are made up, in the shape Gemini CLI's
// ChatRecordingService writes them.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { GeminiCliSessions, readChat, geminiRoot } from "../core/native/gemini-cli.js";
import { NumbatSessions } from "../core/numbat.js";
import { observedSessions } from "../core/reported-runs.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { Session } from "../core/schema/session.js";
import { event, ndjson } from "./fixtures/numbat.js";

const NOW = Date.parse("2026-10-16T12:00:00Z");
const iso = (dt) => new Date(NOW + dt).toISOString();
const SID = "8f1d2c3b-1111-4000-8000-000000000001";
const SUB = "8f1d2c3b-2222-4000-8000-000000000002";
const call = (name, args, status = "success", dt = -60e3) => ({ id: `c${dt}`, name, args, status, timestamp: iso(dt), result: [{ text: "SECRET RESULT" }] });
const lines = (...rs) => rs.map((r) => JSON.stringify(r)).join("\n") + "\n";

function setup() {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-gemini-"));
  const root = geminiRoot({ home, env: {} });
  const proj = path.join(root, "atlas");
  fs.mkdirSync(path.join(proj, "chats", SID), { recursive: true });
  fs.writeFileSync(path.join(proj, ".project_root"), "/w/atlas\n");
  fs.writeFileSync(
    path.join(proj, "chats", "session-2026-10-16T11-50-8f1d2c3b.jsonl"),
    lines(
      { sessionId: SID, projectHash: "ab12", startTime: iso(-600e3), lastUpdated: iso(-600e3), kind: "main" },
      { id: "u1", timestamp: iso(-300e3), type: "user", content: [{ text: "SECRET PROMPT" }] },
      { id: "g1", timestamp: iso(-120e3), type: "gemini", content: "", toolCalls: [call("write_file", { file_path: "/w/atlas/notes.txt", content: "SECRET" }), call("replace", { file_path: "src/app.ts", old_string: "a", new_string: "b" }, "success", -110e3), call("write_file", { file_path: "/w/atlas/failed.txt" }, "error"), call("read_file", { file_path: "/w/atlas/read.txt" })] },
      { id: "g2", timestamp: iso(-100e3), type: "gemini", toolCalls: [call("write_file", { file_path: "/w/atlas/rewound.txt" }, "success", -100e3)] },
      { $rewindTo: "g2" },
      { $set: { lastUpdated: iso(-90e3), summary: "Partial refunds" } },
    ),
  );
  fs.writeFileSync(path.join(proj, "chats", SID, `${SUB}.jsonl`), lines({ sessionId: SUB, projectHash: "ab12", startTime: iso(-200e3), lastUpdated: iso(-150e3), kind: "subagent", directories: ["/w/atlas"] }));
  // The older layout, one JSON object, from yesterday's morning: too old.
  fs.writeFileSync(path.join(proj, "chats", "session-2026-10-14T09-00-aaaaaaaa.json"), JSON.stringify({ sessionId: "aaaaaaaa-0000-4000-8000-000000000003", projectHash: "ab12", startTime: iso(-2 * 86400e3), lastUpdated: iso(-2 * 86400e3), messages: [] }));
  fs.utimesSync(path.join(proj, "chats", "session-2026-10-14T09-00-aaaaaaaa.json"), (NOW - 2 * 86400e3) / 1000, (NOW - 2 * 86400e3) / 1000);
  for (const f of ["session-2026-10-16T11-50-8f1d2c3b.jsonl", path.join(SID, `${SUB}.jsonl`)]) fs.utimesSync(path.join(proj, "chats", f), (NOW - 60e3) / 1000, (NOW - 60e3) / 1000);
  return { home, root, done: () => fs.rmSync(home, { recursive: true, force: true }) };
}

test("each Gemini CLI session is a session, gemini-cli:<sessionId>, in its project's folder, with a subagent's under its parent", () => {
  const w = setup();
  try {
    const g = new GeminiCliSessions({ root: w.root }).read(NOW);
    const list = g.sessions().sort((a, b) => a.createdAt - b.createdAt);
    assert.deepEqual(list.map((s) => s.id), [`gemini-cli:${SID}`, `gemini-cli:${SUB}`]);
    assert.equal(list[0].title, "Partial refunds", "its summary");
    assert.equal(list[0].cwd, "/w/atlas", "from .project_root");
    assert.equal(list[0].updatedAt, NOW - 90e3);
    assert.equal(list[0].reader, "experimental");
    assert.deepEqual([list[1].parentId, list[1].parentKind], [`gemini-cli:${SID}`, "subagent"]);
    for (const s of list) assert.ok(Session.safeParse(s).success, JSON.stringify(Session.safeParse(s).error?.issues));
  } finally {
    w.done();
  }
});

test("its edits are write_file's and replace's files after rewinds; failed calls and reads aren't, and nothing else is kept", () => {
  const w = setup();
  try {
    const g = new GeminiCliSessions({ root: w.root }).read(NOW);
    const live = g.live(`gemini-cli:${SID}`);
    assert.deepEqual([...live.edits.keys()].sort(), ["/w/atlas/notes.txt", "/w/atlas/src/app.ts"]);
    assert.doesNotMatch(JSON.stringify([...g.runs.values()].map((r) => ({ ...r, edits: [...r.edits] }))), /SECRET/);
  } finally {
    w.done();
  }
});

test("the older one-object layout reads too, and what isn't a chat record is passed over", () => {
  const c = readChat(JSON.stringify({ sessionId: SID, projectHash: "x", startTime: iso(0), lastUpdated: iso(5e3), messages: [{ id: "g", timestamp: iso(1e3), type: "gemini", toolCalls: [call("write_file", { file_path: "/a/b" })] }] }));
  assert.equal(c.id, SID);
  assert.deepEqual(c.edits.map((e) => e.path), ["/a/b"]);
  assert.equal(readChat("not json"), null);
  assert.equal(readChat(lines({ id: "m" })), null);
});

test("a missing folder is said, not an error", () => {
  const g = new GeminiCliSessions({ root: path.join(os.tmpdir(), "xenonet-no-such-gemini") }).read(NOW);
  assert.equal(g.status().state, "missing");
  assert.notEqual(g.health().state, "failed");
});

test("a run numbat also records is one session, read here; and another session is warned before editing its file", async () => {
  const w = setup();
  const nf = path.join(w.home, "records.ndjson");
  try {
    fs.writeFileSync(nf, ndjson([event({ source_agent: "gemini-cli", event_type: "file.write", session_id: SID, project_path: "/w/atlas", file_path: "/w/atlas/notes.txt", timestamp: iso(-60e3) })]));
    const clock = { now: () => NOW, setTimeout, clearTimeout };
    const g = new GeminiCliSessions({ root: w.root }).read(NOW);
    const numbat = new NumbatSessions({ file: nf, clock }).read(NOW);
    const observed = observedSessions([g, numbat]);
    const mine = observed.sessions().filter((s) => s.id === `gemini-cli:${SID}`);
    assert.equal(mine.length, 1);
    assert.equal(mine[0].reader, "experimental");
    const C = "claude:aaaaaaaa-0001";
    const live = { lastEventAt: NOW - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([[C, { id: C, kind: "session", app: "claude", title: "me", cwd: "/w/atlas", updatedAt: NOW }], ...observed.sessions().map((s) => [s.id, { ...s, kind: "session" }])]);
    const state = { graph: {}, index, live: { sessions: new Map([[C, live]]) }, observed, homeOf: () => null };
    const engine = new Engine({ state, store: new EngineStore(":memory:"), now: () => NOW, repos: { of: async () => ({ top: "/w/atlas", common: "/w/atlas/.git", branch: "main" }) } });
    const conflicts = await engine.conflicts(C, ["/w/atlas/notes.txt"]);
    assert.deepEqual(conflicts.map((c) => [c.agent.id, c.observed]), [[`gemini-cli:${SID}`, undefined]]);
    await assert.rejects(engine.send(C, { to: `gemini-cli:${SID}`, text: "hi" }), /can't read messages/);
  } finally {
    w.done();
  }
});
