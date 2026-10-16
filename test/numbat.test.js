// The numbat bridge (core/actors/numbat.js): numbat's NDJSON event records,
// read from the file its hooks write, show the agents XenoNet doesn't read
// itself as observed actors, and their edits count in the warnings, while
// nothing private in a record is kept. The records are made up, in the shape
// of numbat's published schema (test/fixtures/numbat.js).
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Actors } from "../core/actors/index.js";
import { EngineStore } from "../core/engine/store.js";
import { createObservers } from "../core/actors/observers.js";
import { createRegistration } from "../core/actors/registration.js";
import { NumbatObserver, readRecord } from "../core/actors/numbat.js";
import { fakeMap, ATLAS, ATLAS_DIR } from "./fixtures/actor-map.js";
import { STREAM, event, ndjson } from "./fixtures/numbat.js";

const T0 = Date.parse("2026-10-16T12:00:00Z");

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-numbat-"));
  const file = path.join(dir, "records.ndjson");
  const clock = { t: T0, now: () => clock.t, setTimeout, clearTimeout };
  const store = new EngineStore(":memory:");
  const actors = new Actors({ now: () => clock.t, map: fakeMap(), store });
  const o = new NumbatObserver({ actors, file, every: null, clock });
  return { dir, file, clock, store, actors, o, done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

test("a record is read for what XenoNet needs, and other records, versions and agents are passed over", () => {
  const ok = readRecord(JSON.stringify(event({ event_type: "file.write", project_path: ATLAS_DIR, file_path: "src/a.js", session_id: "s" })));
  assert.equal(ok.ok, true);
  assert.equal(ok.event.file, path.join(ATLAS_DIR, "src/a.js"), "a relative path joins its project");
  assert.deepEqual(Object.keys(ok.event).sort(), ["agent", "at", "file", "id", "project", "session", "type"], "no endpoint, command or content");
  assert.equal(readRecord("{not json").why, "malformed");
  assert.equal(readRecord(JSON.stringify({ record_type: "finding", schema_version: "0.5.0" })).why, "not-an-event");
  assert.equal(readRecord(JSON.stringify(event({ schema_version: "1.0.0" }))).why, "unsupported-version");
  for (const agent of ["claude-code", "codex", "hermes", "unknown"]) assert.equal(readRecord(JSON.stringify(event({ source_agent: agent }))).why, "left-out", agent);
  assert.equal(readRecord(JSON.stringify(event({ event_type: "file.read", file_path: "/x" }))).event.file, null, "a read isn't an edit");
});

test("agents in numbat's records appear as observed actors, and their edits in places on the map count as observed edits", async () => {
  const w = setup();
  try {
    fs.writeFileSync(w.file, ndjson(STREAM(T0)));
    await w.o.tick();
    assert.ok(w.actors.has("actor:numbat-gemini-cli"));
    assert.ok(w.actors.has("actor:numbat-cursor"));
    for (const id of ["actor:numbat-claude-code", "actor:numbat-codex"]) assert.equal(w.actors.has(id), false, "XenoNet reads those itself");
    const g = w.actors.info("actor:numbat-gemini-cli");
    assert.equal(g.name, "Gemini CLI (through numbat)");
    assert.deepEqual(g.via, ["observed"]);
    assert.equal(w.actors.presence("actor:numbat-gemini-cli"), "working");
    const view = w.actors.engineView("actor:numbat-gemini-cli");
    assert.deepEqual(view.edits.map((e) => [e.path, e.observed]).sort(), [[`${ATLAS_DIR}/src/ledger.js`, true], [`${ATLAS_DIR}/src/refunds.js`, true]]);
    assert.equal(view.place?.id, ATLAS);
    assert.deepEqual(w.actors.engineView("actor:numbat-cursor").edits, [], "an edit outside every place is left out");
    // Nothing private is kept: not the host, the user, the command or a preview.
    const kept = JSON.stringify([w.actors.info("actor:numbat-gemini-cli"), w.actors.recent?.("actor:numbat-gemini-cli") ?? null, w.store.actorFactsSince("actor:numbat-gemini-cli", "edited", 0)]);
    for (const secret of ["made-up-host", "made-up-secret", "made-up preview", "npm test"]) assert.equal(kept.includes(secret), false, secret);
    // The same records again, after a restart: each counts once.
    const again = new NumbatObserver({ actors: w.actors, file: w.file, every: null, clock: w.clock });
    await again.tick();
    assert.equal(w.store.actorFactsSince("actor:numbat-gemini-cli", "edited", 0).length, 2);
  } finally {
    w.done();
  }
});

test("the file is tailed: new lines are read once, a partial line waits, and a file rotated or cut short is read again from the start", async () => {
  const w = setup();
  try {
    fs.writeFileSync(w.file, "");
    await w.o.tick();
    assert.equal(w.o.status().sources[0].state, "ok");
    const line = JSON.stringify(event({ event_type: "file.write", project_path: ATLAS_DIR, file_path: `${ATLAS_DIR}/a.js`, timestamp: new Date(T0).toISOString() }));
    fs.appendFileSync(w.file, line.slice(0, 40));
    await w.o.tick();
    assert.equal(w.actors.has("actor:numbat-gemini-cli"), false, "half a line waits");
    fs.appendFileSync(w.file, line.slice(40) + "\n");
    await w.o.tick();
    assert.deepEqual(w.actors.engineView("actor:numbat-gemini-cli").edits.map((e) => e.path), [`${ATLAS_DIR}/a.js`]);
    // Rotated: the old file moved aside and a new one begun.
    fs.renameSync(w.file, `${w.file}.1`);
    fs.writeFileSync(w.file, JSON.stringify(event({ event_type: "file.write", project_path: ATLAS_DIR, file_path: `${ATLAS_DIR}/b.js`, timestamp: new Date(T0).toISOString() })) + "\n");
    await w.o.tick();
    assert.deepEqual(w.actors.engineView("actor:numbat-gemini-cli").edits.map((e) => e.path).sort(), [`${ATLAS_DIR}/a.js`, `${ATLAS_DIR}/b.js`]);
    // Quiet for an hour: away.
    w.clock.t += 3600e3;
    await w.o.tick();
    assert.equal(w.actors.presence("actor:numbat-gemini-cli"), "away");
  } finally {
    w.done();
  }
});

test("a missing file is said in the doctor's words, not an error", async () => {
  const w = setup();
  try {
    await w.o.tick();
    assert.equal(w.o.health().state === "degraded", false);
    assert.deepEqual(w.o.status().sources.map((s) => s.state), ["missing"]);
  } finally {
    w.done();
  }
});

test("off by default: no numbat observer unless its file is named, and turning it off reads nothing", async () => {
  const actors = new Actors({ map: fakeMap() });
  const registration = createRegistration({ actors });
  const procs = { census: async () => ({ table: new Map(), of: () => ({}) }) };
  const without = createObservers({ actors, registration, procs, home: "/nowhere" });
  assert.equal(without.list.some((o) => o.product.id === "numbat"), false);
  const w = setup();
  try {
    fs.writeFileSync(w.file, ndjson(STREAM(T0)));
    const reg = createRegistration({ actors: w.actors });
    const withIt = createObservers({ actors: w.actors, registration: reg, procs, home: "/nowhere", numbat: { file: w.file } });
    const o = withIt.list.find((x) => x.product.id === "numbat");
    assert.ok(o);
    reg.observe("numbat", false);
    assert.equal(await o.tick(), false, "off reads nothing");
    assert.equal(w.actors.has("actor:numbat-gemini-cli"), false);
  } finally {
    w.done();
  }
});

test("a session about to edit a file an agent edited through numbat is warned, and told XenoNet inferred it", async () => {
  const { Engine } = await import("../core/engine/engine.js");
  const { mapOf } = await import("../core/actors/index.js");
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-numbat-"));
  try {
    const file = path.join(dir, "records.ndjson");
    const t = { now: Date.now() };
    const now = () => t.now;
    const A = "claude:aaaaaaaa-0001";
    const live = { lastEventAt: t.now - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
    const index = new Map([
      ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
      [A, { id: A, kind: "session", app: "claude", title: "Add partial refunds", cwd: "/w/atlas", updatedAt: live.lastEventAt }],
    ]);
    const state = { graph: {}, index, live: { sessions: new Map([[A, live]]) }, homeOf: (id) => (index.get(id)?.cwd === "/w/atlas" ? "project:atlas" : null) };
    const store = new EngineStore(":memory:");
    const actors = new Actors({ store, now, map: mapOf(state) });
    const engine = new Engine({ state, store, actors, now, repos: { of: async (cwd) => (cwd?.startsWith("/w/atlas") ? { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" } : null) } });
    fs.writeFileSync(file, ndjson([event({ event_type: "file.write", session_id: "g-9", project_path: "/w/atlas", file_path: "/w/atlas/src/refunds.js", timestamp: new Date(t.now - 60e3).toISOString() })]));
    const o = new NumbatObserver({ actors, file, every: null, clock: { now, setTimeout, clearTimeout } });
    await o.tick();
    const conflicts = await engine.conflicts(A, ["/w/atlas/src/refunds.js"]);
    assert.equal(conflicts.length, 1, JSON.stringify(conflicts));
    assert.equal(conflicts[0].agent.id, "actor:numbat-gemini-cli");
    assert.equal(conflicts[0].observed, true);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
