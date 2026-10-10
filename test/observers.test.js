// The observers together (core/actors/observers.js): one for each product
// XenoNet observes, on or off with its product's observer, and none at all in
// the demo and test instances or with the layer off.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Actors } from "../core/actors/index.js";
import { createObservers } from "../core/actors/observers.js";
import { createRegistration } from "../core/actors/registration.js";
import { EngineStore } from "../core/engine/store.js";
import { createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeMap, NODES, ATLAS, ATLAS_DIR, LEDGER_DIR } from "./fixtures/actor-map.js";
import { takeEvents } from "../core/actors/reports.js";
import { hermesRoot } from "./fixtures/hermes.js";
import { CHECKS } from "../core/doctor/index.js";

const machine = () => {
  const m = fakeProcesses();
  return { m, procs: createProcs({ platform: "darwin", exec: m.exec }) };
};

test("none in the demo and test instances, whose actors are made up, nor with actors off", () => {
  const { m, procs } = machine();
  assert.equal(createObservers({ actors: new Actors({ demo: true, map: fakeMap() }), procs, home: "/Users/nova" }), null);
  assert.equal(createObservers({ actors: null, procs, home: "/Users/nova" }), null);
  assert.equal(m.asked.length, 0, "nothing looked at");
});

test("one for each product XenoNet observes, building blocks of their own, each on or off with its product's observer", async () => {
  const { procs } = machine();
  const actors = new Actors({ map: fakeMap() });
  const registration = createRegistration({ actors });
  const observers = createObservers({ actors, registration, procs, home: "/Users/nova/nowhere", grokBot: { apps: ["/Users/nova/nowhere/Grok Bot.app"] }, muse: { apps: ["/Users/nova/nowhere/Muse.app"] } });
  assert.deepEqual(observers.list.map((o) => [o.name, o.kind, o.product.id]), [["hermes-observer", "observer", "hermes-agent"], ["grok-bot-observer", "observer", "grok-bot"], ["muse-observer", "observer", "muse"]]);
  assert.deepEqual(observers.status().map((s) => [s.product, s.on, s.installed]), [["hermes-agent", true, false], ["grok-bot", true, false], ["muse", true, false]]);
  registration.observe("grok-bot", false);
  assert.deepEqual(observers.status().map((s) => [s.product, s.on]), [["hermes-agent", true], ["grok-bot", false], ["muse", true]]);
  assert.equal(await observers.list[1].tick(), false, "an observer that's off doesn't look");
  observers.start();
  observers.stop();
  assert.deepEqual(observers.list.map((o) => o.health().state), ["stopped", "stopped", "stopped"]);
});

test("they look together, sharing one read of the process table; Grok Bot's, not installed, reads it once a minute", async () => {
  const clock = { t: Date.parse("2026-10-05T12:00:00Z"), now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const actors = new Actors({ now: () => clock.t, map: fakeMap() });
  // A home with Hermes installed, so its observer reads the table too.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-observers-"));
  try {
    hermesRoot(path.join(home, ".hermes"));
    const observers = createObservers({ actors, procs, home, clock, grokBot: { apps: [path.join(home, "Applications", "Grok Bot.app")] }, muse: { apps: [path.join(home, "Applications", "Muse.app")] } });
    const tables = () => m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length;
    await observers.look();
    assert.equal(tables(), 1, "one read for both");
    assert.ok(actors.has("actor:hermes"));
    // Hermes's observer reads the table every look; Grok Bot's, with its
    // app nowhere, only once a minute.
    const grok = observers.list[1];
    clock.t += 5000;
    const before = tables();
    await grok.tick(clock.t);
    assert.equal(tables(), before, "not installed, and looked a moment ago");
    clock.t += 60e3;
    await grok.tick(clock.t);
    assert.equal(tables(), before + 1);
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
});

test("with their observers off, the doctor still knows the products are here, from their files alone, and says the observers are off", async () => {
  const { m, procs } = machine();
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-observers-off-"));
  try {
    hermesRoot(path.join(home, ".hermes"));
    const app = path.join(home, "Applications", "Grok Bot.app");
    fs.mkdirSync(app, { recursive: true });
    // A Muse.app too, whose Info.plist would say whose it is: with its
    // observer off, that isn't read, and an app of its name counts.
    fs.mkdirSync(path.join(home, "Applications", "Muse.app", "Contents"), { recursive: true });
    // Turned off before XenoNet ever looked (off when it last ran, say).
    const actors = new Actors({ map: fakeMap() });
    const registration = createRegistration({ actors });
    registration.observe("hermes-agent", false);
    registration.observe("grok-bot", false);
    registration.observe("muse", false);
    const observers = createObservers({ actors, registration, procs, home, grokBot: { apps: [app] }, muse: { apps: [path.join(home, "Applications", "Muse.app")] } });
    await observers.look();
    assert.equal(m.asked.length, 0, "nothing looked at while they're off");
    assert.deepEqual(observers.status().map((s) => [s.product, s.on, s.installed]), [["hermes-agent", false, true], ["grok-bot", false, true], ["muse", false, true]]);
    const check = CHECKS.find((c) => c.id === "actors.observers");
    assert.deepEqual(await check.run({ platform: "darwin", actors: { observers: () => observers.status() } }), {
      status: "info",
      detail: "the observers of Hermes Agent, Grok Bot and Muse are off",
      fix: "Turn one on in XenoNet's Actors panel, or with xenonet actor observe <product> on.",
    });
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
});

test("an observer turned on for a product that isn't installed lists nothing until the product is seen, and one long gone stays off the list", async () => {
  const DAY = 24 * 3600e3;
  const clock = { t: Date.parse("2026-10-05T12:00:00Z"), now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-observers-phantom-"));
  try {
    const store = new EngineStore(":memory:");
    const actors = new Actors({ now: () => clock.t, map: fakeMap(), store });
    const registration = createRegistration({ actors, now: () => clock.t });
    const app = path.join(home, "Applications", "Grok Bot.app");
    const observers = createObservers({ actors, registration, procs, home, clock, grokBot: { apps: [app] }, muse: { apps: [path.join(home, "Applications", "Muse.app")] } });
    const listed = () => actors.list().actors.map((a) => a.id);
    // Neither is on this Mac: the switch makes a placeholder for each.
    registration.observe("hermes-agent", true);
    registration.observe("grok-bot", true);
    await observers.look();
    clock.t += 60e3;
    await observers.look();
    actors.tick();
    assert.deepEqual(listed(), ["actor:you"], "no phantom, offline or out of sight, for a week");
    assert.deepEqual([actors.snapshot("actor:grok-bot").lastSeen, actors.snapshot("actor:hermes").lastSeen], [null, null], "never heard from");
    // Grok Bot installed and opened: seen, and listed.
    fs.mkdirSync(app, { recursive: true });
    const main = m.add({ ppid: 1, program: `${app}/Contents/MacOS/Grok Bot`, start: clock.t });
    clock.t += 5000;
    await observers.look();
    assert.deepEqual([listed(), actors.presence("actor:grok-bot")], [["actor:you", "actor:grok-bot"], "idle"]);
    // Closed and taken off this Mac: listed for a week after it was last seen.
    m.remove(main);
    fs.rmSync(app, { recursive: true, force: true });
    clock.t += 5000;
    await observers.look();
    const gone = clock.t;
    assert.equal(actors.presence("actor:grok-bot"), "unknown");
    clock.t += 8 * DAY;
    await observers.look();
    assert.deepEqual(listed(), ["actor:you"]);
    // Its observer off and on again, weeks later, and XenoNet started again:
    // still gone, still unlisted.
    registration.observe("grok-bot", false);
    registration.observe("grok-bot", true);
    clock.t += 14 * DAY;
    await observers.look();
    actors.stop();
    const again = new Actors({ now: () => clock.t, map: fakeMap(), store });
    const restarted = createObservers({ actors: again, registration: createRegistration({ actors: again, now: () => clock.t }), procs, home, clock, grokBot: { apps: [app] }, muse: { apps: [path.join(home, "Applications", "Muse.app")] } });
    await restarted.look();
    again.tick();
    assert.deepEqual([again.list().actors.map((a) => a.id), again.snapshot("actor:grok-bot").lastSeen, again.presence("actor:grok-bot")], [["actor:you"], gone, "unknown"]);
  } finally {
    fs.rmSync(home, { recursive: true, force: true });
  }
});

test("a spawned report that names no session waits two minutes for one to start under the actor's own processes there, then names it in a second event, attributed", async () => {
  const T = Date.parse("2026-10-05T12:00:30Z");
  const clock = { t: T, now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const nodes = NODES();
  const store = new EngineStore(":memory:");
  const actors = new Actors({ now: () => clock.t, map: fakeMap(nodes), store });
  const observers = createObservers({ actors, procs, home: "/Users/nova/nowhere", clock, grokBot: { apps: ["/Users/nova/nowhere/Grok Bot.app"] }, muse: { apps: ["/Users/nova/nowhere/Muse.app"] } });
  const WREN = "actor:wren";
  actors.upsert({ id: WREN, name: "Wren", via: ["registered"], heartbeatMs: 60000, capabilities: ["spawn-sessions"], allowedAt: T });
  // Wren, run from your terminal, starts claude -p in atlas-api, and you
  // start a session of your own there too.
  const term = m.add({ ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", start: T - 3600e3 });
  const wren = m.add({ ppid: term, program: "/Users/nova/bin/wren", start: T - 60e3 });
  const claude = m.add({ ppid: wren, program: "/Users/nova/.local/bin/claude", args: ["-p", "go"], cwd: ATLAS_DIR, start: T });
  m.add({ ppid: term, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T });
  const told = await takeEvents(actors, WREN, [{ kind: "spawned", app: "claude", path: ATLAS_DIR }], { now: T + 1000, own: new Set([wren]) });
  assert.deepEqual([told.accepted, actors.waiting.length], [1, 1]);
  clock.t = T + 5000;
  await observers.look();
  assert.equal(actors.waiting.length, 1, "its session isn't on the map yet");
  // Its session and yours reach the map, and Claude Code's hooks pin its
  // own to its process.
  const S7 = "claude:0190d3e0-0000-7000-8000-00000000ee07";
  const S8 = "claude:0190d3e0-0000-7000-8000-00000000ee08";
  nodes[S7] = { kind: "session", app: "claude", home: ATLAS, cwd: ATLAS_DIR, createdAt: T + 2000 };
  nodes[S8] = { kind: "session", app: "claude", home: ATLAS, cwd: ATLAS_DIR, createdAt: T + 2500 };
  store.pin(S7, { pid: claude, start: m.table.get(claude).start }, true, T + 3000);
  clock.t = T + 10e3;
  await observers.look();
  const spawned = () => actors.snapshot(WREN).recent.filter((e) => e.kind === "spawned").map((e) => [e.target, e.via, e.detail]);
  assert.deepEqual(spawned(), [[S7, "attributed", "Started a Claude Code session in atlas-api"], [ATLAS, "registered", "Started a Claude Code session in atlas-api"]], "its own report stands, and the session it started is named beside it");
  assert.ok(actors.snapshot(WREN).targets.some((t) => t.id === S7 && t.relation === "spawned"));
  assert.ok(!actors.snapshot(WREN).targets.some((t) => t.id === S8), "never yours");
  assert.equal(actors.waiting.length, 0);
  // Another, whose session never shows: after two minutes it stays as told.
  await takeEvents(actors, WREN, [{ kind: "spawned", app: "codex", path: LEDGER_DIR }], { now: clock.t, own: new Set([wren]) });
  clock.t += 2 * 60e3;
  await observers.look();
  assert.deepEqual([actors.waiting.length, spawned().length], [0, 3]);
  // Without its processes known (not macOS), nothing waits.
  await takeEvents(actors, WREN, [{ kind: "spawned", app: "claude", path: ATLAS_DIR }], { now: clock.t, own: null });
  assert.equal(actors.waiting.length, 0);
});
