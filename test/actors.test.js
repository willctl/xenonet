// The actor registry (core/actors) and its routes: the list, snapshots and
// events as the contracts have them, presence by the interface's rules,
// what's kept in engine.sqlite, the demo's actors through the same calls
// real ones use, the person at the console seen only by the window, and
// XenoNet Dev's simulations. The demo's tests run in a demo world of their
// own (XENONET_DEMO and a spare port before anything is imported); the rest
// on made-up maps and clocks.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
process.env.XENONET_DEMO = "1";
process.env.XENONET_PORT = String(port);
const { XenoNetState } = await import("../core/state.js");
const { world } = await import("../core/demo.js");
const { Actors, createActors, mapOf, LIST_THROTTLE_MS, CLOCK_MS, PRUNE_MS } = await import("../core/actors/index.js");
const { addDemoActors, STEP_MS } = await import("../core/actors/demo.js");
const { createWindowKey } = await import("../core/actors/key.js");
const { createBucket } = await import("../core/actors/limits.js");
const { EngineStore } = await import("../core/engine/store.js");
const { actorRoutes } = await import("../server/routes/actors.js");
const { check, explain } = await import("../core/schema/index.js");
const { fromFile } = await import("./fixtures/schema-files.js");

const state = new XenoNetState();
after(() => {
  state.stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});
const graph = await state.refresh();
const onMap = new Set(graph.nodes.map((n) => n.id));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const T = 1791230000000;
const MIN = 60e3;
const HOUR = 3600e3;
const DAY = 24 * HOUR;

// Checked strictly against the definition, and against the written file.
function valid(id, data, what = id) {
  const d = JSON.parse(JSON.stringify(data));
  const r = check(id, d, { strict: true });
  assert.ok(r.ok, `${what} doesn't fit ${id}:\n${explain(r)}`);
  assert.ok(fromFile(id).safeParse(d).success, `${what} doesn't fit schemas/${id}.json`);
}

// A made-up map: two projects, a folder outside them and a session in each
// project.
const ATLAS = "project:/users/nova/projects/atlas-api";
const LEDGER = "project:/users/nova/projects/ledger";
const SCRATCH = "zone:/users/nova/scratch";
const S1 = "claude:0190d3e0-0000-7000-8000-00000000aa01";
const S2 = "codex:0190d3e0-0000-7000-8000-00000000aa02";
function fakeMap(nodes = { [ATLAS]: { kind: "project", name: "atlas-api" }, [LEDGER]: { kind: "project", name: "ledger" }, [SCRATCH]: { kind: "zone", name: "scratch" }, [S1]: { kind: "session", app: "claude", home: ATLAS, title: "Add partial refunds" }, [S2]: { kind: "session", app: "codex", home: LEDGER, title: "Close the month" } }) {
  return {
    nodes,
    has: (id) => Object.hasOwn(nodes, id),
    describe(id) {
      const n = nodes[id];
      if (!n) return null;
      if (n.kind !== "session") return { kind: n.kind, app: null, name: n.name, place: null, placeId: null };
      return { kind: "session", app: n.app, name: null, place: nodes[n.home]?.name ?? null, placeId: n.home ?? null };
    },
    recentSession: () => S1,
  };
}

// A registry on a clock the test moves, in a store of its own.
function cast({ t = T, map = fakeMap(), demo = false, store } = {}) {
  const clock = { t };
  const actors = new Actors({ now: () => clock.t, map, demo, store });
  return { actors, clock };
}
const moth = (actors, at) => actors.upsert({ id: "actor:moth", name: "Moth", via: ["registered"], heartbeatMs: 30000, capabilities: ["message-agents"], allowedAt: at });
const lastEvent = (actors, id = "actor:moth") => actors.snapshot(id, { view: "window" }).recent.at(-1);

// A demo cast on a clock the test moves.
function demoCast(start = Date.now()) {
  const clock = { t: start };
  const actors = new Actors({ now: () => clock.t, map: mapOf(state), demo: true });
  const step = addDemoActors(actors);
  return { actors, step, clock };
}

// ---------- the list ----------

test("an offline simulation stays offline through demo steps until online or clear", () => {
  const { actors, step, clock } = demoCast();
  actors.simulate("actor-offline", { a: "actor:moth" });
  for (let i = 0; i < 100; i++) { clock.t += STEP_MS; step(); }
  assert.equal(actors.snapshot("actor:moth").presence, "offline");
  actors.simulate("actor-online", { a: "actor:moth" });
  assert.equal(actors.snapshot("actor:moth").presence, "working");
  actors.simulate("actor-offline", { a: "actor:moth" });
  actors.simulate("clear");
  assert.equal(actors.snapshot("actor:moth").presence, "working");
  actors.stop();
});

test("outside the demo, just you, as a placeholder", () => {
  const actors = createActors();
  const list = actors.list();
  valid("actor/list/v1", list);
  assert.deepEqual(list.actors, [
    { schema: "actor/snapshot/v1", id: "actor:you", kind: "human", name: "You", product: null, host: null, via: ["observed"], presence: "unknown", presenceSince: null, lastSeen: null, heartbeatMs: null, status: null, capabilities: [], targets: [], recent: [], counts: { events24h: 0, spawned24h: 0, messages24h: 0 }, identity: null },
  ]);
});

test("the demo's actors: you and three made-up agents, with hours of history, every one as the schemas have it", () => {
  const actors = createActors({ demo: true, map: mapOf(state) });
  const list = actors.list();
  valid("actor/list/v1", list);
  const by = Object.fromEntries(list.actors.map((a) => [a.id, a]));
  assert.deepEqual(Object.keys(by), ["actor:you", "actor:wren", "actor:kestrel", "actor:moth"], "you first");
  assert.deepEqual(
    list.actors.map((a) => [a.name, a.kind, a.product?.id ?? null, a.via, a.heartbeatMs]),
    [
      ["You", "human", null, ["demo", "observed"], 60000],
      ["Wren", "agent", "hermes-agent", ["demo", "observed", "attributed"], 60000],
      ["Kestrel", "agent", "grok-bot", ["demo", "observed"], null],
      ["Moth", "agent", null, ["demo", "registered"], 30000],
    ],
  );
  assert.deepEqual([by["actor:you"].identity, by["actor:you"].capabilities], [null, []], "even the made-up you is never named");
  for (const a of list.actors) {
    assert.ok(a.counts.events24h > 4 && a.recent.length > 4, `${a.name} has a history`);
    assert.ok(a.recent.every((e, i, all) => !i || all[i - 1].at <= e.at), `${a.name}'s events are newest last`);
    assert.ok(a.presence !== "away" && a.presence !== "unknown" && a.presence !== "offline", `${a.name} is there at the start: ${a.presence}`);
    assert.ok(a.targets.length, `${a.name} acts on something`);
    for (const t of a.targets) assert.ok(onMap.has(t.id), `${a.name}'s target ${t.id} is on the map`);
    for (const e of a.recent) {
      assert.ok(e.target === null || onMap.has(e.target), `${a.name}'s event target ${e.target}`);
      assert.ok(e.detail && !/Add partial refunds|Draft the Q4|Draw delivery zones|Review the partial/.test(e.detail), `${e.id} names no session's title: ${e.detail}`);
      assert.notEqual(e.via, "demo", "the demo plays real sources");
    }
  }
  // What each one does.
  const kindOf = (id) => graph.nodes.find((n) => n.id === id);
  assert.ok(by["actor:wren"].counts.spawned24h >= 2);
  assert.deepEqual([...new Set(by["actor:wren"].targets.filter((t) => t.relation === "spawned").map((t) => kindOf(t.id).app))], ["claude"], "Wren starts Claude Code sessions");
  assert.deepEqual([...new Set(by["actor:wren"].targets.filter((t) => t.relation === "works-on").map((t) => kindOf(t.id).name))].sort(), ["atlas-api", "ledger"], "in two projects");
  assert.ok(by["actor:wren"].recent.some((e) => e.via === "attributed"), "and XenoNet traces some of it back to Wren");
  assert.ok(by["actor:kestrel"].recent.some((e) => e.kind === "ran"), "Kestrel runs commands");
  assert.deepEqual(by["actor:kestrel"].targets.map((t) => kindOf(t.id).name), ["pathfinder"]);
  assert.ok(by["actor:moth"].targets.some((t) => t.relation === "watches") && by["actor:moth"].targets.some((t) => t.relation === "messaged"), "Moth watches a project and says things to a session there");
  assert.ok(by["actor:moth"].counts.messages24h > 0 && by["actor:moth"].recent.every((e) => e.via === "registered"), "in its own words");
  assert.ok(by["actor:you"].recent.some((e) => e.kind === "status" && /^Looking at /.test(e.detail)) && by["actor:you"].targets.some((t) => t.relation === "watches"), "you look at things");
});

test("the scripts play on a step at a time, every event sent as the schemas have it", () => {
  const { actors, step, clock } = demoCast();
  const events = [];
  actors.on("event", (e) => events.push(e));
  for (let i = 0; i < 70; i++) {
    clock.t += STEP_MS;
    step();
  }
  assert.ok(events.length > 50, `${events.length} events`);
  for (const e of events) valid("actor/event/v1", e, `${e.id} (${e.kind})`);
  assert.deepEqual([...new Set(events.map((e) => e.actor))].sort(), ["actor:kestrel", "actor:moth", "actor:wren", "actor:you"]);
  const kestrel = events.filter((e) => e.actor === "actor:kestrel" && e.kind === "presence").map((e) => e.detail);
  assert.ok(kestrel.includes("Came online") && kestrel.includes("Out of sight"), `Kestrel comes and goes with its app: ${kestrel}`);
  assert.ok(!kestrel.includes("Went offline") && !kestrel.includes("Went away"), "and never turns offline or away");
  assert.equal(new Set(events.map((e) => e.id)).size, events.length, "ids are unique");
  const list = actors.list();
  valid("actor/list/v1", list);
  for (const a of list.actors) assert.ok(a.recent.length <= 20);
});

// ---------- presence ----------

test("presence: unknown until heard from, away after two missed heartbeats, back with the next, offline after ten or a goodbye", () => {
  const { actors, clock } = cast();
  moth(actors, T);
  const now = () => actors.snapshot("actor:moth");
  assert.equal(now().presence, "unknown", "allowed, not heard from yet");
  actors.report("actor:moth", { presence: "working", via: "registered" });
  assert.deepEqual([now().presence, now().presenceSince, lastEvent(actors).detail], ["working", T, "Came online"]);
  clock.t += 65e3;
  actors.tick();
  assert.equal(now().presence, "working", "two heartbeats and 5 s of grace");
  clock.t += 1e3;
  actors.tick();
  assert.deepEqual([now().presence, now().presenceSince, lastEvent(actors).detail, lastEvent(actors).via], ["away", T + 66e3, "Went away", "registered"]);
  actors.report("actor:moth", { via: "registered" });
  assert.deepEqual([now().presence, lastEvent(actors).detail], ["working", "Came back"], "a heartbeat keeps its last word");
  clock.t += 300e3 + 1;
  actors.tick();
  assert.deepEqual([now().presence, lastEvent(actors).detail], ["offline", "Went offline"], "ten heartbeats of silence");
  actors.report("actor:moth", { presence: "idle", via: "registered" });
  assert.deepEqual([now().presence, lastEvent(actors).detail], ["idle", "Came online"]);
  actors.report("actor:moth", { presence: "offline", via: "registered" });
  assert.deepEqual([now().presence, lastEvent(actors).detail], ["offline", "Went offline"], "goodbye");
  clock.t += DAY;
  actors.tick();
  assert.equal(now().presence, "offline", "offline isn't away");
  assert.throws(() => actors.report("actor:nobody", {}), /no actor/);
});

test("the most present instance wins, and at most 8 are kept, the one silent longest going", () => {
  const { actors, clock } = cast();
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["registered"], heartbeatMs: 60000, allowedAt: T });
  actors.report("actor:wren", { instance: "gateway", presence: "idle" });
  actors.report("actor:wren", { instance: "cli", presence: "working" });
  assert.equal(actors.snapshot("actor:wren").presence, "working");
  actors.report("actor:wren", { instance: "cli", presence: "offline" });
  assert.equal(actors.snapshot("actor:wren").presence, "idle", "the gateway is still there");
  for (let i = 0; i < 9; i++) {
    clock.t += 1000;
    actors.report("actor:wren", { instance: `serve-${i}`, presence: "idle" });
  }
  const a = actors.actors.get("actor:wren");
  assert.equal(a.instances.size, 8);
  assert.ok(!a.instances.has("gateway") && !a.instances.has("cli") && a.instances.has("serve-8"), [...a.instances.keys()].join());
});

test("an action a registered actor reports keeps it working for 90 s; an observer's word stands for its own", () => {
  const { actors, clock } = cast();
  moth(actors, T);
  actors.report("actor:moth", { presence: "idle", via: "registered" });
  actors.record("actor:moth", { kind: "said", target: S1, count: 1, via: "registered" });
  assert.equal(actors.snapshot("actor:moth").presence, "working");
  clock.t += 89e3;
  actors.report("actor:moth", { via: "registered" });
  assert.equal(actors.snapshot("actor:moth").presence, "working");
  clock.t += 2e3;
  actors.tick();
  assert.deepEqual([actors.snapshot("actor:moth").presence, lastEvent(actors).detail], ["idle", "Went idle"]);

  actors.upsert({ id: "actor:kestrel", name: "Kestrel", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed"] });
  actors.report("actor:kestrel", { instance: "app", presence: "unknown", via: "observed" });
  actors.record("actor:kestrel", { kind: "ran", target: ATLAS, via: "observed" });
  assert.equal(actors.snapshot("actor:kestrel").presence, "unknown", "its app is closed, whatever came before");
  clock.t += DAY;
  actors.tick();
  assert.equal(actors.snapshot("actor:kestrel").presence, "unknown", "without a heartbeat it never turns away");
});

test("an event heard of late says what happened then: it doesn't take presence back to that time, nor make a registered actor working", () => {
  const { actors, clock } = cast();
  moth(actors, T);
  actors.report("actor:moth", { presence: "idle", via: "registered" });
  clock.t += 10 * MIN;
  actors.report("actor:moth", { via: "registered" });
  // Something it did two hours ago, kept and reported now.
  const then = clock.t - 2 * HOUR;
  actors.record("actor:moth", { kind: "said", target: S1, count: 1, via: "registered", at: then });
  const presences = () => actors.snapshot("actor:moth").recent.filter((e) => e.kind === "presence").map((e) => [e.detail, e.at - T]);
  assert.equal(actors.presence("actor:moth"), "idle");
  clock.t += 5000;
  actors.tick();
  assert.deepEqual([actors.presence("actor:moth"), presences()], ["idle", [["Came online", 0]]], "no Started working two hours back, and no Went idle after it");
  assert.deepEqual(actors.snapshot("actor:moth").recent.filter((e) => e.kind === "said").map((e) => [e.detail, e.at]), [["Said something to an agent in atlas-api", then]], "the event itself is kept, at its own time");
  // One half a minute ago is still within the 90 s: working from now.
  actors.record("actor:moth", { kind: "said", target: S1, count: 1, via: "registered", at: clock.t - 30e3 });
  assert.deepEqual([actors.presence("actor:moth"), actors.presenceSince("actor:moth")], ["working", clock.t]);
  clock.t += 61e3;
  actors.report("actor:moth", { via: "registered" });
  assert.equal(actors.presence("actor:moth"), "idle", "90 s after the action, not after it was heard of");
  // An observer's word about a job's last run, dated earlier, is the same:
  // it doesn't bring back what was true then.
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"], heartbeatMs: 60000 });
  actors.report("actor:wren", { instance: "observed", presence: "idle", via: "observed" });
  clock.t += 3 * MIN;
  actors.tick();
  const away = clock.t;
  assert.equal(actors.presence("actor:wren"), "away");
  actors.record("actor:wren", { kind: "finished", subject: "job", target: ATLAS, via: "observed", at: clock.t - 3 * HOUR });
  assert.deepEqual([actors.presence("actor:wren"), actors.presenceSince("actor:wren")], ["away", away]);
});

test("after a restart, an actor keeps its last presence while it's within two heartbeats, and the rules take over", () => {
  const store = new EngineStore(":memory:");
  const first = cast({ store });
  moth(first.actors, T);
  first.actors.report("actor:moth", { presence: "working", status: "Watching geofence", via: "registered" });
  first.actors.record("actor:moth", { kind: "said", target: S1, via: "registered" });
  first.actors.stop();
  const again = cast({ store, t: T + 30e3 });
  const s = again.actors.snapshot("actor:moth");
  assert.deepEqual([s.presence, s.presenceSince, s.status, s.recent.map((e) => e.detail), s.targets.map((t) => [t.relation, t.id])], ["working", T, "Watching geofence", ["Came online", "Watching geofence", "Said something to an agent in atlas-api"], [["messaged", S1]]]);
  again.clock.t = T + 66e3;
  again.actors.tick();
  assert.deepEqual([again.actors.snapshot("actor:moth").presence, lastEvent(again.actors).id], ["away", "moth:4"], "its events go on numbering where they were");
});

test("a presence kept across a restart gives way to the first source that speaks, even with no heartbeat to age it", () => {
  const store = new EngineStore(":memory:");
  const first = cast({ store });
  const kestrel = (actors) => actors.upsert({ id: "actor:kestrel", name: "Kestrel", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed", "attributed"], heartbeatMs: null });
  kestrel(first.actors);
  first.actors.report("actor:kestrel", { instance: "observed", presence: "idle", via: "observed" });
  first.actors.stop();
  const again = cast({ store, t: T + DAY });
  again.clock.t = T + 2 * DAY;
  again.actors.tick();
  assert.equal(again.actors.presence("actor:kestrel"), "idle", "nothing has said otherwise, and it has no heartbeat to miss");
  again.actors.report("actor:kestrel", { instance: "observed", presence: "unknown", via: "observed" });
  assert.equal(again.actors.presence("actor:kestrel"), "unknown", "its app closed while XenoNet was away");
  // The same instance carries on from where it was.
  const moths = cast({ store });
  moth(moths.actors, T);
  moths.actors.report("actor:moth", { presence: "working", via: "registered" });
  moths.actors.stop();
  const later = cast({ store, t: T + 10e3 });
  later.actors.report("actor:moth", { via: "registered" });
  assert.equal(later.actors.presence("actor:moth"), "working", "a heartbeat without a presence keeps the last one");
});

test("a status: a new one is an event, the same again isn't, one like a key never gets in; and whether it waits on you", () => {
  const { actors } = cast();
  moth(actors, T);
  actors.report("actor:moth", { presence: "working", status: "Watching geofence" });
  actors.report("actor:moth", { status: "Watching geofence" });
  actors.report("actor:moth", { status: `Deploying with ${"ab12".repeat(10)}` });
  const s = actors.snapshot("actor:moth");
  assert.equal(s.status, "Watching geofence");
  assert.deepEqual(s.recent.map((e) => e.detail), ["Came online", "Watching geofence"]);
  actors.report("actor:moth", { status: "" });
  assert.deepEqual([actors.snapshot("actor:moth").status, actors.snapshot("actor:moth").recent.length], [null, 2], "an empty one clears it, quietly");
  assert.throws(() => actors.record("actor:moth", { kind: "status", status: "  " }), /needs a line/);

  actors.report("actor:moth", { needsYou: { kind: "question", text: "Waiting for an answer about geofence" } });
  const waiting = actors.snapshot("actor:moth");
  valid("actor/snapshot/v1", waiting);
  assert.deepEqual(waiting.needsYou, { kind: "question", text: "Waiting for an answer about geofence", since: T });
  actors.report("actor:moth", { needsYou: { kind: "question", text: `token ghp_${"x".repeat(20)}` } });
  assert.equal(actors.snapshot("actor:moth").needsYou.text, "Waiting for you", "a line like a key is XenoNet's own words instead");
  actors.report("actor:moth", { needsYou: null });
  assert.equal("needsYou" in actors.snapshot("actor:moth"), false);
});

test("who the list shows: you, registered actors even offline, and observed ones heard of in the last week; offline stays offline across a restart", () => {
  const store = new EngineStore(":memory:");
  const { actors, clock } = cast({ store });
  moth(actors, T);
  actors.report("actor:moth", { presence: "offline" });
  actors.upsert({ id: "actor:kestrel", name: "Kestrel", via: ["observed"] });
  actors.report("actor:kestrel", { instance: "app", presence: "unknown" });
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  assert.deepEqual(actors.list().actors.map((a) => a.id), ["actor:you", "actor:moth", "actor:kestrel"], "Wren hasn't been heard of");
  clock.t += 8 * DAY;
  assert.deepEqual(actors.list().actors.map((a) => a.id), ["actor:you", "actor:moth"], "a week on, Kestrel's gone from it; Moth stays to be seen and removed");
  actors.stop();
  const again = cast({ store, t: clock.t });
  again.actors.tick();
  assert.deepEqual(["actor:moth", "actor:kestrel", "actor:wren"].map((id) => again.actors.snapshot(id).presence), ["offline", "unknown", "unknown"]);
});

test("an observed product its observer sees installed stays listed however long it hasn't run; one that isn't goes a week after it was last heard of", () => {
  const { actors, clock } = cast();
  actors.upsert({ id: "actor:kestrel", name: "Kestrel", via: ["observed"] });
  actors.report("actor:kestrel", { instance: "observed", presence: "unknown" });
  actors.upsert({ id: "actor:heron", name: "Heron", via: ["observed"] });
  actors.report("actor:heron", { instance: "observed", presence: "offline" });
  actors.setInstalled("actor:heron", true);
  actors.setInstalled("actor:kestrel", false);
  clock.t += 8 * DAY;
  actors.tick();
  assert.deepEqual(actors.list().actors.map((a) => [a.id, a.presence]), [["actor:you", "unknown"], ["actor:heron", "offline"]]);
  assert.equal(actors.snapshot("actor:heron").lastSeen, T, "still when it was last heard of");
  actors.setEnabled("actor:heron", false);
  assert.deepEqual(actors.list().actors.map((a) => a.id), ["actor:you"], "its observer off, it's off the list");
});

// ---------- events and targets ----------

test("what a snapshot holds: its last 20 events, a day's counts, places as targets and actors only in events", () => {
  const { actors, clock } = cast();
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed", "attributed"], heartbeatMs: 60000 });
  actors.report("actor:wren", { instance: "gateway", presence: "working" });
  actors.record("actor:wren", { kind: "spawned", target: S1, via: "attributed" });
  clock.t += 25 * HOUR;
  actors.report("actor:wren", { instance: "gateway", presence: "working" });
  for (let i = 0; i < 25; i++) actors.record("actor:wren", { kind: "messaged", target: "actor:you", platform: "telegram" });
  const s = actors.snapshot("actor:wren");
  valid("actor/snapshot/v1", s);
  assert.equal(s.recent.length, 20);
  assert.equal(s.recent.at(-1).id, "wren:27");
  assert.equal(s.recent.at(-1).detail, "Sent a message on Telegram");
  assert.deepEqual(s.counts, { events24h: 25, spawned24h: 0, messages24h: 25 }, "the spawn was more than a day ago");
  assert.deepEqual(s.targets, [], "the session's target left after a day, and you're not a place on the map");
  assert.equal(actors.store.lastActorEvents("actor:wren", 100).length, 27, "all of them are kept");
});

test("targets: since and lastAt, 16 shown and 32 kept, and what the map doesn't have left out (a session after two minutes)", () => {
  const nodes = { ...fakeMap().nodes };
  const projects = Array.from({ length: 40 }, (_, i) => `project:/users/nova/projects/p${i}`);
  for (const [i, id] of projects.entries()) nodes[id] = { kind: "project", name: `p${i}` };
  const map = fakeMap(nodes);
  const { actors, clock } = cast({ map });
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  actors.target("actor:wren", ATLAS, "works-on");
  clock.t += 1000;
  actors.target("actor:wren", ATLAS, "works-on");
  assert.deepEqual(actors.snapshot("actor:wren").targets, [{ id: ATLAS, relation: "works-on", since: T, lastAt: T + 1000 }]);
  for (const id of projects) {
    clock.t += 1000;
    actors.target("actor:wren", id, "watches");
  }
  const shown = actors.snapshot("actor:wren").targets;
  assert.equal(shown.length, 16);
  assert.equal(shown[0].id, projects.at(-1), "the most recent first");
  assert.equal(actors.actors.get("actor:wren").targets.size, 32);
  assert.equal(actors.store.actorTargets().length, 32, "and the store keeps the same");
  assert.throws(() => actors.target("actor:wren", "actor:moth", "watches"), /not a node/);

  // A session the map doesn't have yet, and a project it doesn't have.
  const fresh = "claude:0190d3e0-0000-7000-8000-00000000bb01";
  const { actors: b, clock: c } = cast({ map });
  b.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  b.record("actor:wren", { kind: "spawned", target: fresh, via: "attributed" });
  b.target("actor:wren", "project:/users/nova/projects/gone", "works-on");
  assert.deepEqual(b.snapshot("actor:wren").targets.map((t) => t.id), [fresh], "the session waits for its transcript; the unknown project doesn't show");
  c.t += 2 * MIN;
  b.tick();
  assert.deepEqual(b.snapshot("actor:wren").targets, []);
  assert.ok(!b.store.actorTargets().some((t) => t.target === fresh), "it's let go");
  assert.equal(b.snapshot("actor:wren").recent.at(-1).target, fresh, "the event still says what it was");

  // Before XenoNet's first look at the map, at start, nothing is let go.
  const early = { ...map, ready: () => false };
  const store = new EngineStore(":memory:");
  const before = cast({ map: early, store });
  before.actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  before.actors.record("actor:wren", { kind: "spawned", target: fresh, via: "attributed" });
  before.clock.t += 10 * MIN;
  before.actors.tick();
  assert.deepEqual(before.actors.snapshot("actor:wren").targets.map((t) => t.id), [fresh], "shown, and kept");
  assert.equal(store.actorTargets().length, 1);
});

test("events since a time: oldest first, a week back at most, a limit, and never yours outside the demo", () => {
  const { actors, clock } = cast();
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  actors.record("actor:wren", { kind: "ran", target: ATLAS, at: T - 8 * DAY });
  actors.record("actor:wren", { kind: "ran", target: ATLAS, at: T - 6 * DAY });
  actors.you.report({ presence: "working", did: [{ what: "looked", node: ATLAS }] });
  clock.t += 1000;
  actors.record("actor:wren", { kind: "ran", target: ATLAS, count: 2 });
  const all = actors.events({ since: 0 });
  valid("actor/events/v1", all);
  assert.deepEqual(all.events.map((e) => e.id), ["wren:2", "wren:3"], "a week at most, and none of yours");
  assert.deepEqual(actors.events({ since: 0, limit: 1 }).events.map((e) => e.id), ["wren:2"]);
  assert.deepEqual(actors.events({ since: T + 1 }).events.map((e) => e.id), ["wren:3"]);
  assert.deepEqual(actors.events({ since: 0, only: "actor:you" }).events, [], "yours aren't anyone's to ask for");
  const mine = actors.events({ since: 0, only: "actor:you", view: "window" });
  valid("actor/events/v1", mine);
  assert.deepEqual(mine.events.map((e) => e.detail), ["Came online", "Looking at atlas-api"], "but the window's");
});

// ---------- the list, sent as it changes ----------

test("the list goes out at most every 500 ms after changes, and not when only you changed", async () => {
  const actors = createActors({ map: fakeMap() });
  const lists = [];
  actors.on("actors", (l) => lists.push(l));
  actors.upsert({ id: "actor:moth", name: "Moth", via: ["registered"], heartbeatMs: 30000, allowedAt: Date.now() });
  await wait(LIST_THROTTLE_MS + 400);
  assert.equal(lists.length, 1, "Moth joined");
  for (let i = 0; i < 5; i++) actors.report("actor:moth", { status: `step ${i}` });
  assert.equal(lists.length, 1, "not at once");
  await wait(LIST_THROTTLE_MS + 400);
  assert.equal(lists.length, 2, "once, for all five");
  assert.equal(lists[1].actors.find((a) => a.id === "actor:moth").status, "step 4");
  for (const l of lists) valid("actor/list/v1", l);
  actors.you.report({ presence: "working", did: [{ what: "looked", node: ATLAS }] });
  actors.tick();
  await wait(LIST_THROTTLE_MS + 400);
  assert.equal(lists.length, 2, "your doings send nothing, so nothing can time its work to them");
  actors.stop();
});

// ---------- you ----------

test("you are the window's alone: everywhere else a placeholder, your events never sent", () => {
  const { actors } = cast();
  const sent = [];
  actors.on("event", (e) => sent.push(e));
  actors.you.report({ presence: "working", did: [{ what: "looked", node: ATLAS }, { what: "read", node: S1 }] });
  assert.deepEqual(sent, [], "no actor-event for you");
  const outside = actors.snapshot("actor:you");
  assert.deepEqual([outside.presence, outside.status, outside.targets, outside.recent, outside.lastSeen, outside.heartbeatMs, outside.counts.events24h], ["unknown", null, [], [], null, null, 0]);
  assert.deepEqual(actors.list().actors[0], outside);
  const inside = actors.snapshot("actor:you", { view: "window" });
  valid("actor/snapshot/v1", inside);
  assert.deepEqual([inside.presence, inside.status, inside.heartbeatMs, inside.identity, inside.capabilities, inside.name], ["working", "Reading a Claude Code session in atlas-api", 60000, null, [], "You"]);
  assert.deepEqual(inside.targets.map((t) => [t.relation, t.id]).sort(), [["reviews", S1], ["watches", ATLAS]]);
  assert.equal(inside.counts.events24h, 3);
  assert.equal(actors.simulate("actor-spawn", { a: "actor:you" }), null, "and nothing but the window acts as you");
});

test("what the window reports becomes events in XenoNet's words, as the interface's table has it", () => {
  const { actors, clock } = cast();
  const did = (what, node, extra = {}) => actors.you.report({ did: [{ what, node, at: clock.t, ...extra }] });
  actors.you.report({ presence: "working" });
  const rows = [
    [() => did("looked", ATLAS), "status", ATLAS, "Looking at atlas-api", ["watches", ATLAS]],
    [() => did("looked", S2), "status", S2, "Looking at a Codex session in ledger", ["watches", S2]],
    [() => did("looked", SCRATCH), "status", SCRATCH, "Looking at scratch", ["watches", SCRATCH]],
    [() => did("read", S2), "status", S2, "Reading a Codex session in ledger", ["reviews", S2]],
    [() => did("opened", S1), "status", S1, "Opened a Claude Code session in atlas-api", ["reviews", S1]],
    [() => did("handed-off", S1, { to: "codex" }), "spawned", ATLAS, "Handed a Claude Code session in atlas-api to Codex", ["works-on", ATLAS]],
    [() => did("started", LEDGER, { app: "claude" }), "spawned", LEDGER, "Started a Claude Code session in ledger", ["works-on", LEDGER]],
    [() => did("terminal", S2), "status", LEDGER, "Opened a terminal in ledger", ["works-on", LEDGER]],
    [() => did("terminal", SCRATCH), "status", SCRATCH, "Opened a terminal in scratch", ["works-on", SCRATCH]],
  ];
  for (const [act, kind, target, detail, [relation, node]] of rows) {
    clock.t += 1000;
    assert.deepEqual(act(), { accepted: 1, dropped: [] }, detail);
    const e = lastEvent(actors, "actor:you");
    assert.deepEqual([e.kind, e.target, e.detail, e.via], [kind, target, detail, "observed"]);
    assert.ok(actors.snapshot("actor:you", { view: "window" }).targets.some((t) => t.relation === relation && t.id === node && t.lastAt === clock.t), `${detail}: ${relation} ${node}`);
  }
  assert.equal(actors.snapshot("actor:you", { view: "window" }).status, "Opened a terminal in scratch");
});

test("a place whose name reads like a key is named for what it is, so your status still gets in", () => {
  const hashed = "project:/users/nova/projects/hashed";
  const map = fakeMap({ ...fakeMap().nodes, [hashed]: { kind: "project", name: "ab12".repeat(10) } });
  const { actors } = cast({ map });
  assert.deepEqual(actors.you.report({ presence: "working", did: [{ what: "looked", node: hashed }] }), { accepted: 1, dropped: [] });
  assert.equal(actors.snapshot("actor:you", { view: "window" }).status, "Looking at a project");
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  assert.equal(actors.record("actor:wren", { kind: "ran", target: hashed }).detail, "Ran a command in a project");
});

test("what the window may not report: unknown actions and nodes, the wrong kind of node, old ones, a handoff without its app", () => {
  const { actors, clock } = cast();
  const r = actors.you.report({
    presence: "idle",
    did: [
      { what: "typed", node: ATLAS },
      { what: "looked", node: "project:/users/nova/projects/nowhere" },
      { what: "read", node: ATLAS },
      { what: "started", node: S1 },
      { what: "handed-off", node: S1 },
      { what: "looked", node: ATLAS, at: T - DAY - 1 },
      { what: "looked", node: "actor:wren" },
      { what: "looked", node: LEDGER, at: T + 10 * MIN },
    ],
  });
  assert.deepEqual(r.dropped.map((d) => d.index), [0, 1, 2, 3, 4, 5, 6]);
  for (const d of r.dropped) assert.ok(typeof d.reason === "string" && d.reason.length > 5, JSON.stringify(d));
  assert.equal(r.accepted, 1);
  assert.equal(lastEvent(actors, "actor:you").at, clock.t, "a time ahead of XenoNet's is now");
});

test("the desktop app's word: hidden is away, closed offline, and shown gives the page's back; ten minutes of silence is offline", () => {
  const { actors, clock } = cast();
  const presence = () => actors.snapshot("actor:you", { view: "window" }).presence;
  actors.you.window("shown");
  assert.equal(presence(), "unknown", "the window shows, but the page hasn't said anything");
  actors.you.report({ presence: "working" });
  assert.equal(presence(), "working");
  actors.you.window("hidden");
  actors.you.report({ presence: "working" });
  assert.equal(presence(), "away", "a hidden window is away, whatever the page thinks");
  actors.you.window("shown");
  assert.equal(presence(), "working");
  actors.you.window("closed");
  assert.deepEqual([presence(), lastEvent(actors, "actor:you").detail], ["offline", "Went offline"]);
  actors.you.window("shown");
  actors.you.report({ presence: "idle" });
  clock.t += 125e3 + 1;
  actors.tick();
  assert.equal(presence(), "away", "two missed minutes");
  clock.t += 10 * MIN;
  actors.tick();
  assert.equal(presence(), "offline", "ten minutes of silence");
  actors.you.window("sideways");
  assert.equal(presence(), "offline", "a word it doesn't know changes nothing");
});

test("turned off the map: your record goes at once, and nothing more is said of you, the window's word included, until the page reports again", () => {
  const store = new EngineStore(":memory:");
  const { actors, clock } = cast({ store });
  actors.you.report({ presence: "working", did: [{ what: "looked", node: ATLAS }] });
  actors.you.window("hidden");
  const me = (a = actors) => a.snapshot("actor:you", { view: "window" });
  assert.deepEqual([me().presence, me().status], ["away", "Looking at atlas-api"]);
  const lastId = me().recent.at(-1).id;
  actors.you.forget();
  const empty = (s) => [s.presence, s.presenceSince, s.lastSeen, s.status, s.targets, s.recent];
  assert.deepEqual(empty(me()), ["unknown", null, null, null, [], []]);
  assert.deepEqual(actors.events({ since: 0, only: "actor:you", view: "window" }).events, []);
  assert.deepEqual(empty(new Actors({ now: () => clock.t, map: fakeMap(), store }).snapshot("actor:you", { view: "window" })), ["unknown", null, null, null, [], []], "nothing comes back after a restart");
  for (const w of ["shown", "hidden", "closed", "shown"]) actors.you.window(w);
  clock.t += 20 * MIN;
  actors.tick();
  assert.deepEqual(empty(me()), ["unknown", null, null, null, [], []], "the desktop app's word on the window says nothing of you either");
  // The switch on again: the page reports, and you're back.
  actors.you.report({ presence: "working" });
  assert.deepEqual([me().presence, me().recent.map((e) => e.detail)], ["working", ["Came online"]]);
  assert.ok(Number(me().recent[0].id.split(":")[1]) > Number(lastId.split(":")[1]), "with a new id, never an old one's");
});

test("the window's word counts only once the page has said something", () => {
  const { actors } = cast();
  actors.you.window("hidden");
  actors.you.window("closed");
  assert.deepEqual([actors.snapshot("actor:you", { view: "window" }).presence, actors.snapshot("actor:you", { view: "window" }).recent], ["unknown", []], "the page reports nothing while you're off the map, and neither does the window");
});

test("the demo's made-up you keeps still once you turn yourself off, until the page reports again", () => {
  const { actors, step, clock } = demoCast();
  actors.you.forget();
  for (let i = 0; i < 21; i++) {
    clock.t += STEP_MS;
    step();
  }
  assert.deepEqual(actors.snapshot("actor:you").recent, []);
  actors.you.report({ presence: "working" });
  for (let i = 0; i < 21; i++) {
    clock.t += STEP_MS;
    step();
  }
  assert.ok(actors.snapshot("actor:you").recent.length > 1, "it plays on");
});

// ---------- what's kept ----------

test("pruning on the clock: a week of events, a day of yours, and what's in memory follows", () => {
  const { actors, clock } = cast();
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  actors.record("actor:wren", { kind: "ran", target: ATLAS });
  actors.you.report({ presence: "working", did: [{ what: "looked", node: ATLAS }] });
  clock.t += 25 * HOUR;
  actors.prune();
  const you = actors.snapshot("actor:you", { view: "window" });
  assert.deepEqual([you.recent, you.status, you.targets], [[], null, []], "yours went after a day");
  assert.equal(actors.snapshot("actor:wren").recent.length, 1, "Wren's stay for a week");
  clock.t += 6 * DAY;
  actors.prune();
  assert.deepEqual(actors.snapshot("actor:wren").recent, []);
  assert.ok(PRUNE_MS === HOUR && CLOCK_MS === 5000, "every hour, on a 5-second clock");
});

test("actors come and go: added again keeps their record, removed takes their events and targets; only you are human", () => {
  const { actors } = cast();
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed"] });
  actors.record("actor:wren", { kind: "ran", target: ATLAS });
  actors.upsert({ id: "actor:wren", name: "Wren", via: ["observed", "registered"], capabilities: ["run-commands"] });
  assert.deepEqual([actors.snapshot("actor:wren").via, actors.snapshot("actor:wren").recent.length], [["observed", "registered"], 1]);
  actors.remove("actor:wren");
  assert.equal(actors.snapshot("actor:wren"), null);
  assert.deepEqual([actors.store.actorRow("wren"), actors.store.lastActorEvents("actor:wren"), actors.store.actorTargets()], [null, [], []]);
  assert.throws(() => actors.upsert({ id: "actor:eve", kind: "human", name: "Eve" }), /only actor:you is human/);
  assert.throws(() => actors.upsert({ id: "@vm1:actor:wren", name: "Wren" }), /of this machine/);
  assert.throws(() => actors.record("actor:you", { kind: "presence" }), /presence comes from reports/);
});

// ---------- XenoNet Dev's simulations ----------

test("XenoNet Dev's simulations: an actor spawns, edits, says something, comes and goes", () => {
  const { actors } = demoCast();
  const events = [];
  actors.on("event", (e) => events.push(e));
  const recent = mapOf(state).recentSession();
  assert.ok(onMap.has(recent));

  assert.deepEqual(actors.simulate("actor-spawn"), { ok: true, kind: "actor-spawn", a: "actor:wren", b: recent }, "the first agent, and the session busy most lately");
  assert.match(events.at(-1).detail, /^Started a (Claude Code|Codex) session in \S+/);
  const edit = actors.simulate("actor-edit", { a: "actor:kestrel" });
  assert.equal(edit.b, state.homeOf(recent), "an edit lands on the place the session lives");
  assert.match(events.find((e) => e.kind === "edited").detail, /^Edited a file in \S+/);
  const atlas = graph.nodes.find((n) => n.kind === "session" && n.title === "Add partial refunds to the orders API").id;
  assert.equal(actors.simulate("actor-message", { a: "actor:moth", b: atlas }).b, atlas);
  assert.equal(events.find((e) => e.kind === "said").detail, "Said something to an agent in atlas-api");
  assert.ok(actors.snapshot("actor:moth").targets.some((t) => t.relation === "messaged" && t.id === atlas), "a beam to the session");
  assert.deepEqual(actors.simulate("actor-offline", { a: "actor:moth" }), { ok: true, kind: "actor-offline", a: "actor:moth" });
  assert.equal(actors.snapshot("actor:moth").presence, "offline");
  actors.simulate("actor-online", { a: "actor:moth" });
  assert.equal(actors.snapshot("actor:moth").presence, "working");
  for (const e of events) valid("actor/event/v1", e, e.kind);
  valid("actor/list/v1", actors.list());

  assert.equal(actors.simulate("actor-dance"), null);
  assert.equal(actors.simulate("actor-spawn", { a: "actor:nobody" }), null);
  assert.equal(actors.simulate("actor-spawn", { a: "actor:you" }).a, "actor:you", "the demo's made-up you can be made to act");
  assert.equal(createActors({ map: mapOf(state) }).simulate("actor-spawn"), null, "outside the demo, with no agents, there's no one to act");
});

// ---------- the routes ----------

async function readJson(req, limit = 1 << 20) {
  if (req.body === "not json") throw Object.assign(new Error("the body isn't JSON"), { status: 400 });
  if (JSON.stringify(req.body ?? {}).length > limit) throw Object.assign(new Error("the body is too big"), { status: 413 });
  return req.body ?? {};
}
function routesFor(actors, { verdict = { ok: true, checked: true }, now = Date.now } = {}) {
  const key = createWindowKey();
  const verify = { calls: 0, window: async () => (verify.calls++, verdict) };
  const route = actorRoutes({ actors, windowKey: key, verify, readJson, send: (res, status, body, headers = {}) => Object.assign(res, { status, body, headers }), now });
  const go = async (method, path, { body, headers = {} } = {}) => {
    const res = {};
    await route({ method, headers, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  const window = (extra = {}) => ({ "x-xenonet-window": key.reveal(), ...extra });
  return { go, key, verify, window };
}

test("GET /api/actors, one actor by its slug, and events since a time", async () => {
  const { go } = routesFor(createActors({ demo: true, map: mapOf(state) }));
  const list = await go("GET", "/api/actors");
  assert.equal(list.status, 200);
  valid("actor/list/v1", list.body);
  assert.equal(list.body.actors.length, 4);
  const wren = await go("GET", "/api/actors/wren");
  assert.equal(wren.status, 200);
  valid("actor/snapshot/v1", wren.body);
  assert.equal(wren.body.name, "Wren");
  for (const path of ["/api/actors/nobody", "/api/actors/Wren", "/api/actors/%E0%A4%A", "/api/actors/wren/more"]) assert.deepEqual([(await go("GET", path)).status, (await go("GET", path)).body.code], [404, "not-found"], path);
  const events = await go("GET", `/api/actors/events?since=${Date.now() - HOUR}`);
  assert.equal(events.status, 200);
  valid("actor/events/v1", events.body);
  assert.ok(events.body.events.length > 0 && events.body.events.every((e, i, all) => !i || all[i - 1].at <= e.at));
  assert.ok(events.body.events.some((e) => e.actor === "actor:you"), "the demo's made-up you is shown like any actor");
  assert.deepEqual([(await go("GET", "/api/actors/events?since=soon")).status, (await go("GET", "/api/actors/events?since=soon")).body.code], [400, "bad-request"]);
  assert.equal((await go("GET", "/api/actors/events")).status, 200, "the last day, without since");
  assert.equal((await go("PUT", "/api/actors")).body.code, "not-found");
  assert.deepEqual([(await go("POST", "/api/actors")).status, (await go("POST", "/api/actors")).body.code], [403, "yours"], "adding an actor is the window's");
  assert.deepEqual([(await go("POST", "/api/actors/register")).status, (await go("POST", "/api/actors/register")).body.code], [400, "bad-request"], "a registration needs a name");
});

test("the window's routes: its key and its process; your record and your events for it, the placeholder for anyone else", async () => {
  const actors = createActors({ map: fakeMap() });
  const { go, verify, window } = routesFor(actors);
  const report = { schema: "actor/console/v1", presence: "working", did: [{ what: "looked", node: ATLAS }] };
  for (const headers of [{}, { "x-xenonet-window": "" }, { "x-xenonet-window": "f".repeat(64) }, { "x-xenonet-window": `${window()["x-xenonet-window"]}0` }]) {
    const r = await go("POST", "/api/actors/you", { body: report, headers });
    assert.deepEqual([r.status, r.body.code], [403, "yours"], JSON.stringify(headers).slice(0, 40));
  }
  assert.equal(verify.calls, 0, "a request without the key goes no further");
  assert.equal(actors.snapshot("actor:you", { view: "window" }).recent.length, 0, "and changes nothing");

  const ok = await go("POST", "/api/actors/you", { body: report, headers: window() });
  assert.equal(ok.status, 200);
  valid("actor/me/v1", ok.body);
  assert.deepEqual(ok.body, { schema: "actor/me/v1", id: "actor:you", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 0, accepted: 1, dropped: [] });
  assert.equal(verify.calls, 1, "the process is checked");

  const record = await go("GET", "/api/actors/you", { headers: window() });
  valid("actor/snapshot/v1", record.body);
  assert.deepEqual([record.body.presence, record.body.status], ["working", "Looking at atlas-api"]);
  const mine = await go("GET", "/api/actors/you?since=0", { headers: window() });
  valid("actor/events/v1", mine.body);
  assert.deepEqual(mine.body.events.map((e) => e.detail), ["Came online", "Looking at atlas-api"]);

  const anyone = await go("GET", "/api/actors/you");
  assert.deepEqual([anyone.status, anyone.body.presence, anyone.body.status, anyone.body.recent], [200, "unknown", null, []], "without a key, what the list says");
  assert.deepEqual((await go("GET", "/api/actors/you", { headers: { "x-xenonet-window": "nope" } })).status, 403, "a wrong key is refused, so the page knows to ask again");
  assert.deepEqual((await go("GET", "/api/actors")).body.actors[0].presence, "unknown");
  assert.deepEqual((await go("GET", "/api/actors/events?since=0")).body.events, []);

  const refused = routesFor(actors, { verdict: { ok: false, checked: true, reason: "this is for XenoNet's own window, and it came from under a Claude Code process" } });
  const r = await refused.go("POST", "/api/actors/you", { body: report, headers: refused.window() });
  assert.deepEqual([r.status, r.body.code, r.body.error], [403, "yours", "this is for XenoNet's own window, and it came from under a Claude Code process"], "the key isn't enough from the wrong process");
  // Show me on the map turned off: your record goes, from the window alone.
  for (const headers of [{}, { "x-xenonet-window": "f".repeat(64) }, { ...window(), authorization: `Bearer xenonet_actor_${"ab".repeat(32)}` }]) assert.deepEqual([(await go("DELETE", "/api/actors/you", { headers })).status], [403], JSON.stringify(Object.keys(headers)));
  assert.equal((await refused.go("DELETE", "/api/actors/you", { headers: refused.window() })).body.code, "yours", "nor from the wrong process");
  assert.equal(actors.snapshot("actor:you", { view: "window" }).recent.length, 2, "and nothing went");
  const deleted = await go("DELETE", "/api/actors/you", { headers: window() });
  assert.deepEqual([deleted.status, deleted.body], [200, { deleted: "actor:you" }]);
  assert.deepEqual((await go("GET", "/api/actors/you?since=0", { headers: window() })).body.events, []);
  assert.deepEqual([(await go("GET", "/api/actors/you", { headers: window() })).body.presence, (await go("GET", "/api/actors/you", { headers: window() })).body.recent], ["unknown", []]);
  assert.equal((await go("PUT", "/api/actors/you", { headers: window() })).status, 404);
});

test("the window's reports: a body that fits, 8 KiB at most, and a rate", async () => {
  const clock = { t: Date.now() };
  const actors = createActors({ map: fakeMap() });
  const { go, window } = routesFor(actors, { now: () => clock.t });
  const post = (body) => go("POST", "/api/actors/you", { body, headers: window() });
  for (const body of [{ presence: "offline" }, { did: [{ what: "looked" }] }, { did: "everything" }, { presence: "working", did: Array.from({ length: 21 }, () => ({ what: "looked", node: ATLAS })) }]) {
    const r = await post(body);
    assert.deepEqual([r.status, r.body.code], [400, "bad-request"], JSON.stringify(body).slice(0, 60));
    assert.match(r.body.error, /actor\/console\/v1/);
  }
  assert.deepEqual([(await post("not json")).status, (await post("not json")).body.code], [400, "bad-request"]);
  const big = await post({ presence: "working", pad: "x".repeat(9000) });
  assert.deepEqual([big.status, big.body.code], [413, "too-big"]);
  const extra = await post({ presence: "working", typed: "nothing of this is kept" });
  assert.equal(extra.status, 200, "keys it doesn't know are passed over");
  // Ten at once, then two a second.
  clock.t += 60e3;
  const statuses = [];
  for (let i = 0; i < 11; i++) statuses.push((await post({ presence: "working" })).status);
  assert.deepEqual(statuses, [...Array(10).fill(200), 429]);
  const slow = await post({ presence: "working" });
  assert.deepEqual([slow.status, slow.body.code, slow.headers["retry-after"]], [429, "too-many", "1"]);
  clock.t += 500;
  assert.equal((await post({ presence: "working" })).status, 200);
  assert.deepEqual(JSON.stringify(actors.list()).includes("nothing of this"), false);
});

test("with actors off, every route says so", async () => {
  const route = actorRoutes({ actors: null, readJson, send: (res, status, body) => Object.assign(res, { status, body }) });
  for (const [method, path] of [["GET", "/api/actors"], ["GET", "/api/actors/wren"], ["POST", "/api/actors/you"], ["GET", "/api/actors/events?since=0"]]) {
    const res = {};
    await route({ method, headers: {}, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    assert.deepEqual([res.status, res.body.code], [503, "off"], path);
  }
});

test("a rate's bucket: a burst, then the rate, and how long until the next", () => {
  const clock = { t: 0 };
  const b = createBucket({ rate: 2, burst: 3, now: () => clock.t });
  assert.deepEqual([b.take(), b.take(), b.take()], [0, 0, 0]);
  assert.equal(b.take(), 500);
  clock.t += 500;
  assert.deepEqual([b.take(), b.take()], [0, 500]);
});

test("the window key: random each time, compared whole, and never shown by accident", async () => {
  const a = createWindowKey(), b = createWindowKey();
  assert.match(a.reveal(), /^[0-9a-f]{64}$/);
  assert.notEqual(a.reveal(), b.reveal());
  assert.equal(a.matches(a.reveal()), true);
  for (const wrong of [b.reveal(), a.reveal().slice(0, 63), `${a.reveal()}0`, a.reveal().toUpperCase(), undefined, null, 42, ["x"], `${a.reveal().slice(0, 63)}é`]) assert.equal(a.matches(wrong), false, String(wrong));
  const { inspect } = await import("node:util");
  for (const shown of [JSON.stringify({ a }), String(a), inspect(a), `${a}`]) assert.ok(!shown.includes(a.reveal()), shown);
});
