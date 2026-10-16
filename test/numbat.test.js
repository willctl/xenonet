// The numbat bridge (core/numbat.js): numbat's NDJSON event records, read
// from the file its hooks write. Each run numbat records is a session, one
// per numbat session id, with its app the agent that ran it, marked
// observed through numbat; its edits count in other sessions' warnings,
// and nothing private in a record is kept. The records are made up, in the
// shape of numbat's published schema (test/fixtures/numbat.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { NumbatSessions, readRecord, sessionIdOf } from "../core/numbat.js";
import { observedSessions } from "../core/reported-runs.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { buildGraph } from "../core/graph.js";
import { actionsFor } from "../core/open.js";
import { Session } from "../core/schema/session.js";
import { schemaFor } from "../core/schema/index.js";
import { ATLAS_DIR } from "./fixtures/actor-map.js";
import { STREAM, event, ndjson } from "./fixtures/numbat.js";

const T0 = Date.parse("2026-10-16T12:00:00Z");

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-numbat-"));
  const file = path.join(dir, "records.ndjson");
  const clock = { t: T0, now: () => clock.t, setTimeout, clearTimeout };
  const n = new NumbatSessions({ file, clock });
  return { dir, file, clock, n, done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

test("a record is read for what XenoNet needs, and other records, versions and agents are passed over", () => {
  const ok = readRecord(JSON.stringify(event({ event_type: "file.write", project_path: ATLAS_DIR, file_path: "src/a.js", session_id: "s-0000001" })));
  assert.equal(ok.ok, true);
  assert.equal(ok.event.file, `${ATLAS_DIR}/src/a.js`, "a relative path joins its project");
  assert.deepEqual(Object.keys(ok.event).sort(), ["app", "at", "branch", "file", "model", "parent", "project", "session", "type"], "no endpoint, command or content");
  assert.equal(readRecord("{not json").why, "malformed");
  assert.equal(readRecord(JSON.stringify({ record_type: "finding", schema_version: "0.5.0" })).why, "not-an-event");
  assert.equal(readRecord(JSON.stringify(event({ schema_version: "1.0.0", session_id: "s" }))).why, "unsupported-version");
  assert.equal(readRecord(JSON.stringify(event({}))).why, "no-session");
  for (const agent of ["claude-code", "codex", "hermes", "unknown"]) assert.equal(readRecord(JSON.stringify(event({ source_agent: agent, session_id: "s" }))).why, "left-out", agent);
  assert.equal(readRecord(JSON.stringify(event({ event_type: "file.read", file_path: "/x", session_id: "s" }))).event.file, null, "a read isn't an edit");
  assert.equal(readRecord(JSON.stringify(event({ event_type: "file.write", project_path: "C:\\w\\atlas", file_path: "src\\a.js", session_id: "s" }))).event.file, "C:\\w\\atlas\\src\\a.js", "Windows paths stay Windows paths");
  assert.match(sessionIdOf("cursor", "has spaces/and slashes"), /^cursor:n[0-9a-f]{31}$/, "an id that doesn't fit is digested");
});

test("each run numbat records is a session, with its app the agent, marked observed, and only what XenoNet needs", () => {
  const w = setup();
  try {
    fs.writeFileSync(w.file, ndjson(STREAM(T0)));
    w.n.read();
    const sessions = w.n.sessions();
    assert.deepEqual(sessions.map((s) => s.id).sort(), ["cursor:c-0000001", "gemini-cli:g-0000001"]);
    const g = sessions.find((s) => s.app === "gemini-cli");
    assert.equal(g.title, "Gemini CLI in atlas-api");
    assert.equal(g.cwd, ATLAS_DIR);
    assert.equal(g.observed, "numbat");
    for (const s of sessions) assert.ok(schemaFor("session/v1").safeParse(s).success, `${s.id} is a session/v1`);
    assert.deepEqual([...w.n.live(g.id).edits.keys()].sort(), [`${ATLAS_DIR}/src/ledger.js`, `${ATLAS_DIR}/src/refunds.js`]);
    const kept = JSON.stringify([sessions, [...w.n.runs.values()].map((r) => ({ ...r, edits: [...r.edits] }))]);
    for (const secret of ["made-up-host", "made-up-secret", "made-up preview", "npm test", "nova\""]) assert.equal(kept.includes(secret), false, secret);
    // On the map it's a session in its project, with only Open folder.
    const graph = buildGraph({ projectsDir: path.dirname(ATLAS_DIR), projects: [{ key: ATLAS_DIR.toLowerCase(), path: ATLAS_DIR, name: "atlas-api" }], sessions, now: T0, windows: { activeWindowMs: 6e5, recentWindowMs: 36e5 } });
    const node = graph.nodes.find((x) => x.id === g.id);
    assert.equal(node.kind, "session");
    assert.deepEqual(actionsFor(node).map((a) => a.id), ["open-folder"]);
    // session.end ends it.
    fs.appendFileSync(w.file, ndjson([event({ event_type: "session.end", session_id: "g-0000001", timestamp: new Date(T0).toISOString() })]));
    w.n.read();
    assert.equal(w.n.live(g.id).endedAt, T0);
  } finally {
    w.done();
  }
});

test("the file is tailed: new lines are read once, a partial line waits, and a file rotated or cut short is read from the start", () => {
  const w = setup();
  try {
    fs.writeFileSync(w.file, "");
    w.n.read();
    assert.equal(w.n.status().state, "ok");
    const edit = (f) => JSON.stringify(event({ event_type: "file.write", session_id: "g-0000002", project_path: ATLAS_DIR, file_path: `${ATLAS_DIR}/${f}`, timestamp: new Date(T0).toISOString() }));
    const line = edit("a.js");
    fs.appendFileSync(w.file, line.slice(0, 40));
    w.n.read();
    assert.equal(w.n.sessions().length, 0, "half a line waits");
    fs.appendFileSync(w.file, line.slice(40) + "\n");
    w.n.read();
    assert.deepEqual([...w.n.live("gemini-cli:g-0000002").edits.keys()], [`${ATLAS_DIR}/a.js`]);
    fs.renameSync(w.file, `${w.file}.1`);
    fs.writeFileSync(w.file, edit("b.js") + "\n");
    w.n.read();
    assert.deepEqual([...w.n.live("gemini-cli:g-0000002").edits.keys()].sort(), [`${ATLAS_DIR}/a.js`, `${ATLAS_DIR}/b.js`]);
    // A day later the run is gone.
    w.clock.t += 25 * 3600e3;
    w.n.read();
    assert.equal(w.n.sessions().length, 0);
  } finally {
    w.done();
  }
});

test("a missing file is said in the doctor's words, not an error", () => {
  const w = setup();
  try {
    w.n.read();
    assert.equal(w.n.status().state, "missing");
    assert.notEqual(w.n.health().state, "failed");
  } finally {
    w.done();
  }
});

test("a session about to edit a file a numbat-observed run edited is warned, told XenoNet inferred it, and can't message that run", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-numbat-"));
  try {
    const file = path.join(dir, "records.ndjson");
    const t = { now: Date.now() };
    const now = () => t.now;
    const numbat = new NumbatSessions({ file, clock: { now, setTimeout, clearTimeout } });
    fs.writeFileSync(file, ndjson([event({ event_type: "file.write", session_id: "g-0000009", project_path: "/w/atlas", file_path: "/w/atlas/src/refunds.js", timestamp: new Date(t.now - 60e3).toISOString() })]));
    numbat.read();
    const A = "claude:aaaaaaaa-0001";
    const G = "gemini-cli:g-0000009";
    const live = { lastEventAt: t.now - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([
      ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
      [A, { id: A, kind: "session", app: "claude", title: "Add partial refunds", cwd: "/w/atlas", updatedAt: live.lastEventAt }],
      ...numbat.sessions().map((s) => [s.id, { ...s, kind: "session" }]),
    ]);
    const state = { graph: {}, index, live: { sessions: new Map([[A, live]]) }, observed: observedSessions([numbat]), homeOf: (id) => (index.get(id)?.cwd === "/w/atlas" ? "project:atlas" : null) };
    const store = new EngineStore(":memory:");
    const engine = new Engine({ state, store, now, repos: { of: async (cwd) => (cwd?.startsWith("/w/atlas") ? { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" } : null) } });
    const conflicts = await engine.conflicts(A, ["/w/atlas/src/refunds.js"]);
    assert.equal(conflicts.length, 1, JSON.stringify(conflicts));
    assert.equal(conflicts[0].agent.id, G);
    assert.equal(conflicts[0].agent.app, "gemini-cli");
    assert.equal(conflicts[0].observed, true);
    const peers = await engine.peers(A);
    assert.ok(JSON.stringify(peers).includes(G), "a peer in the repo");
    await assert.rejects(engine.send(A, { to: G, text: "hi" }), /only observes/);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
