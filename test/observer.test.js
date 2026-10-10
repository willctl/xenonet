// What every observer shares (core/actors/observer.js) and how they tell
// which sessions an actor started (core/actors/attribution.js), on made-up
// maps, clocks and process tables.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { Observer, KEEPALIVE_MS } from "../core/actors/observer.js";
import { startedUnder, Spawns, SPAWN_WAIT_MS } from "../core/actors/attribution.js";
import { PRODUCTS } from "../core/actors/products.js";
import { EngineStore } from "../core/engine/store.js";
import { createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeMap, NODES, ATLAS, LEDGER, S1, ATLAS_DIR, LEDGER_DIR } from "./fixtures/actor-map.js";

const T = Date.parse("2026-10-05T12:00:30Z");
const MIN = 60e3;
const ID = "actor:hermes";

// An observer of Hermes Agent on a clock the test moves, whose look the
// test gives.
function setup({ observing = () => true, look = async () => {}, store = new EngineStore(":memory:") } = {}) {
  const clock = { t: T, now: () => clock.t, setTimeout, clearTimeout };
  const actors = new Actors({ now: () => clock.t, map: fakeMap(), store });
  const o = new Observer({ name: "hermes-observer", product: PRODUCTS["hermes-agent"], actors, observing, clock });
  o.look = look;
  return { actors, o, clock, store };
}
const hermes = (o, extra = {}) => o.keep(ID, { name: "Hermes", version: "0.21.5", via: ["observed", "attributed"], heartbeatMs: 60000, capabilities: ["browse", "run-commands", "edit-repos"], ...extra });

test("keep(): the actor made once, kept current, and a registered one's own name, heartbeat and capabilities left alone", () => {
  const { actors, o } = setup();
  let upserts = 0;
  const upsert = actors.upsert.bind(actors);
  actors.upsert = (def) => (upserts++, upsert(def));
  hermes(o);
  hermes(o);
  assert.equal(upserts, 1, "the same again changes nothing");
  assert.deepEqual(actors.info(ID).capabilities, ["edit-repos", "run-commands", "browse"], "capabilities in the contract's order");
  assert.deepEqual(actors.info(ID).product, { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" });
  hermes(o, { version: null });
  assert.equal(actors.info(ID).product.version, "0.21.5", "a version that can't be read just now isn't a version gone");
  hermes(o, { version: "0.22.0", capabilities: ["browse"] });
  assert.equal(upserts, 2);
  assert.deepEqual([actors.info(ID).product.version, actors.info(ID).capabilities], ["0.22.0", ["browse"]]);
  // Registered through its plugin, later: what you allowed stands.
  actors.upsert({ id: ID, name: "Hermes at work", product: actors.info(ID).product, via: ["observed", "attributed", "registered"], heartbeatMs: 30000, capabilities: ["message-agents"], allowedAt: T });
  hermes(o, { version: "0.22.1" });
  assert.deepEqual(actors.info(ID), { ...actors.info(ID), name: "Hermes at work", heartbeatMs: 30000, capabilities: ["message-agents"], via: ["observed", "attributed", "registered"] });
  assert.equal(actors.info(ID).product.version, "0.22.1");
  // One made while its product's observer is off is made off.
  const off = setup({ observing: () => false });
  hermes(off.o);
  assert.equal(off.actors.enabled(ID), false);
});

test("an actor removed and made again starts afresh: what was said of the old one doesn't hold back the new", () => {
  const { actors, o } = setup();
  hermes(o);
  o.say(ID, { presence: "idle", status: "Its gateway is starting" });
  actors.remove(ID);
  hermes(o);
  assert.equal(o.say(ID, { presence: "idle", status: "Its gateway is starting" }), true);
  assert.equal(actors.snapshot(ID).status, "Its gateway is starting");
});

test("say(): presence and status when they change, every 30 s besides, and never a registered actor's status", () => {
  const { actors, o, clock } = setup();
  hermes(o);
  let reports = 0;
  const report = actors.report.bind(actors);
  actors.report = (id, r) => (reports++, report(id, r));
  assert.equal(o.say(ID, { presence: "idle", status: null }), true);
  assert.equal(o.say(ID, { presence: "idle", status: null }), false);
  assert.equal(actors.presence(ID), "idle");
  clock.t += 10e3;
  assert.equal(o.say(ID, { presence: "working", status: "Running a command in atlas-api" }), true);
  assert.deepEqual([actors.presence(ID), actors.snapshot(ID).status], ["working", "Running a command in atlas-api"]);
  clock.t += KEEPALIVE_MS - 1;
  assert.equal(o.say(ID, { presence: "working", status: "Running a command in atlas-api" }), false);
  clock.t += 1;
  assert.equal(o.say(ID, { presence: "working", status: "Running a command in atlas-api" }), true, "heard from again");
  assert.equal(reports, 3);
  assert.equal(actors.snapshot(ID).lastSeen, clock.t);
  // An observer may say what a registered actor can't: away and unknown.
  o.say(ID, { presence: "away", status: "Its gateway has gone quiet" });
  assert.equal(actors.presence(ID), "away");
  // Gone is said once: lastSeen stays when it was last there.
  clock.t += 1000;
  assert.equal(o.say(ID, { presence: "offline", status: null }), true);
  const gone = clock.t;
  clock.t += 5 * KEEPALIVE_MS;
  assert.equal(o.say(ID, { presence: "offline", status: null }), false);
  assert.equal(actors.snapshot(ID).lastSeen, gone);
  actors.upsert({ ...actors.info(ID), id: ID, via: ["observed", "registered"], allowedAt: T });
  actors.report(ID, { instance: "gateway", status: "Its own words", via: "registered" });
  o.say(ID, { presence: "idle", status: "XenoNet's words" });
  assert.equal(actors.snapshot(ID).status, "Its own words");
});

test("an observer's unknown is said once and stands: silence doesn't age it into away or offline, after a restart either", () => {
  const store = new EngineStore(":memory:");
  const { actors, o, clock } = setup({ store });
  hermes(o);
  // Installed, nothing of it running, its session database unreadable.
  const unreadable = { presence: "unknown", status: "XenoNet can't read its session database" };
  o.say(ID, unreadable);
  for (let i = 0; i < 12; i++) {
    clock.t += MIN;
    assert.equal(o.say(ID, unreadable), false, "said once");
    actors.tick();
  }
  const presences = () => actors.snapshot(ID).recent.filter((e) => e.kind === "presence").map((e) => e.detail);
  assert.deepEqual([actors.presence(ID), presences()], ["unknown", []], "no Went away, no Went offline: nothing changed");
  // It was there, then can't be told: out of sight, once.
  o.say(ID, { presence: "idle", status: null });
  clock.t += 5000;
  o.say(ID, unreadable);
  clock.t += 20 * MIN;
  actors.tick();
  assert.deepEqual([actors.presence(ID), presences()], ["unknown", ["Came online", "Out of sight"]]);
  actors.stop();
  const again = new Actors({ now: () => clock.t + 3600e3, map: fakeMap(), store });
  again.tick();
  assert.equal(again.presence(ID), "unknown", "remembered as it was, until the observer says otherwise");
});

test("happened(): each event once by its ref, after a restart too; a registered actor's left to it but for what's attributed", () => {
  const store = new EngineStore(":memory:");
  const { actors, o } = setup({ store });
  hermes(o);
  assert.ok(o.happened(ID, { ref: "spawned:claude:1", kind: "spawned", target: S1, via: "attributed", app: "claude" }));
  assert.equal(o.happened(ID, { ref: "spawned:claude:1", kind: "spawned", target: S1, via: "attributed" }), null);
  // A new observer on the same store, as after a restart.
  const again = new Observer({ name: "hermes-observer", product: PRODUCTS["hermes-agent"], actors, clock: o.clock });
  assert.equal(again.happened(ID, { ref: "spawned:claude:1", kind: "spawned", target: S1, via: "attributed" }), null);
  assert.deepEqual(actors.snapshot(ID).recent.map((e) => [e.kind, e.via, e.detail]), [["spawned", "attributed", "Started a Claude Code session in atlas-api"]]);
  actors.upsert({ ...actors.info(ID), id: ID, via: ["observed", "attributed", "registered"], allowedAt: T });
  assert.equal(o.happened(ID, { ref: "ran:1", kind: "ran", target: ATLAS, count: 2 }), null, "its own reports say what it ran");
  assert.ok(o.happened(ID, { ref: "spawned:codex:2", kind: "spawned", target: LEDGER, via: "attributed", app: "codex" }), "what XenoNet traced to it is still XenoNet's to say");
});

test("gather() and flush(): a minute's commands in a place as one event, recorded once the minute is over", () => {
  const { actors, o, clock } = setup();
  hermes(o);
  const minute = Math.floor(T / MIN) * MIN;
  o.gather(ID, { kind: "ran", target: ATLAS, at: minute + 1000 });
  o.gather(ID, { kind: "ran", target: ATLAS, at: minute + 2000, count: 2 });
  o.gather(ID, { kind: "ran", target: LEDGER, at: minute + 3000 });
  o.gather(ID, { kind: "edited", target: ATLAS, at: minute + 4000, count: 25, paths: Array.from({ length: 25 }, (_, i) => `${ATLAS_DIR}/src/f${i}.ts`) });
  o.gather(ID, { kind: "messaged", at: minute + 5000, platform: "telegram" });
  o.flush(minute + MIN + 14e3);
  assert.equal(actors.snapshot(ID).recent.length, 0, "a minute waits a little for rows written late");
  o.flush(minute + MIN + 15e3);
  const recent = actors.snapshot(ID).recent;
  assert.deepEqual(recent.map((e) => e.detail).sort(), ["Edited f0.ts and 19 more files in atlas-api", "Ran 3 commands in atlas-api", "Ran a command in ledger", "Sent a message on Telegram"].sort());
  assert.ok(recent.every((e) => e.via === "observed"));
  // The same minute read again (after a restart) isn't recorded twice.
  o.gather(ID, { kind: "ran", target: ATLAS, at: minute + 1000 });
  const fresh = new Observer({ name: "hermes-observer", product: PRODUCTS["hermes-agent"], actors, clock });
  fresh.gather(ID, { kind: "ran", target: ATLAS, at: minute + 1000 });
  fresh.flush(minute + 2 * MIN);
  o.flush(minute + 2 * MIN);
  assert.equal(actors.snapshot(ID).recent.length, 4);
  clock.t = minute + 2 * MIN;
  assert.deepEqual(actors.snapshot(ID).targets.map((t) => [t.id, t.relation]).sort(), [[ATLAS, "edited"], [ATLAS, "works-on"], [LEDGER, "works-on"]].sort());
});

test("touch(): a place it acts on, at most once a minute", () => {
  const { actors, o, clock } = setup();
  hermes(o);
  let targets = 0;
  const target = actors.target.bind(actors);
  actors.target = (...args) => (targets++, target(...args));
  o.touch(ID, ATLAS, "works-on");
  o.touch(ID, ATLAS, "works-on");
  o.touch(ID, null, "works-on");
  clock.t += MIN;
  o.touch(ID, ATLAS, "works-on");
  o.touch(ID, ATLAS, "watches");
  assert.equal(targets, 3);
});

test("tick(): nothing read while the observer is off, and a look that throws leaves it degraded until one doesn't", async () => {
  let on = false;
  let looks = 0;
  let fail = true;
  const { o } = setup({ observing: () => on, look: async () => (looks++, fail && (() => { throw new Error("state.db went away"); })()) });
  assert.equal(await o.tick(), false);
  assert.equal(looks, 0);
  on = true;
  assert.equal(await o.tick(), true);
  assert.deepEqual([looks, o.health().state, o.health().lastError.message], [1, "degraded", "state.db went away"]);
  fail = false;
  await o.tick();
  assert.equal(o.health().state, "running");
  assert.equal(o.kind, "observer");
  // Before the map is there, nothing is read either.
  let ready = false;
  const early = setup({ look: async () => looks++ });
  early.actors.map.ready = () => ready;
  const n = looks;
  assert.equal(await early.o.tick(), false);
  assert.equal(looks, n);
  ready = true;
  await early.o.tick();
  assert.equal(looks, n + 1);
});

// ---------- attribution ----------

const HERMES_PY = "/Users/nova/.hermes/tools/python-3.14.7/bin/python3";
const GROK = "/Applications/Grok Bot.app";

test("startedUnder(): the sessions a product's processes started, and not those a session started itself", async () => {
  const m = fakeProcesses();
  const host = m.add({ ppid: 1, program: HERMES_PY, args: ["-m", "hermes_cli.main", "serve"] });
  const bash = m.add({ ppid: host, program: "/bin/bash", args: ["-c", "claude -p go"] });
  const claude = m.add({ ppid: bash, program: "/Users/nova/.local/bin/claude" });
  const inner = m.add({ ppid: claude, program: "/Users/nova/.local/bin/claude", args: ["--print"] });
  const app = m.add({ ppid: 1, program: `${GROK}/Contents/MacOS/Grok Bot` });
  const zsh = m.add({ ppid: app, program: "/bin/zsh" });
  const codex = m.add({ ppid: zsh, program: "/opt/homebrew/bin/codex" });
  const mine = m.add({ ppid: 1, program: "/Users/nova/.local/bin/claude" });
  const census = await createProcs({ platform: "darwin", exec: m.exec }).census();
  const isHost = (p) => census.of(p).product?.product === "hermes-agent" && census.of(p).product.role === "host";
  const isGrok = (p) => census.of(p).product?.product === "grok-bot";
  assert.deepEqual(startedUnder(census, isHost).map((s) => [s.pid, s.agent, s.root]), [[claude, "claude", host]]);
  assert.deepEqual(startedUnder(census, isGrok).map((s) => [s.pid, s.agent, s.root]), [[codex, "codex", app]]);
  assert.ok(!startedUnder(census, () => true).some((s) => s.pid === inner), "a session's own children are that session's");
  assert.ok(!startedUnder(census, isHost).some((s) => s.pid === mine));
});

test("Spawns: a process's session by its pin, else the one that began in its folder just after it, else its place after two minutes", async () => {
  const store = new EngineStore(":memory:");
  const nodes = NODES();
  const S3 = "claude:0190d3e0-0000-7000-8000-00000000aa03";
  const S4 = "claude:0190d3e0-0000-7000-8000-00000000aa04";
  Object.assign(nodes[S1], { cwd: ATLAS_DIR, createdAt: T + 3000 });
  nodes[S3] = { kind: "session", app: "claude", home: LEDGER, cwd: LEDGER_DIR, createdAt: T + 1000 };
  nodes[S4] = { kind: "session", app: "claude", home: LEDGER, cwd: LEDGER_DIR, createdAt: T + 5000 };
  const map = fakeMap(nodes);
  const m = fakeProcesses();
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const pinned = m.add({ ppid: 1, program: "/x/claude", start: T, cwd: "/Users/nova/elsewhere" });
  const placed = m.add({ ppid: 1, program: "/x/claude", start: T, cwd: ATLAS_DIR });
  const twice = m.add({ ppid: 1, program: "/x/claude", start: T, cwd: LEDGER_DIR });
  const start = m.table.get(pinned).start;
  store.pin("claude:0190d3e0-0000-7000-8000-00000000bb01", { pid: pinned, start }, true, T);
  const spawns = new Spawns({ store, map, procs });
  await spawns.saw(ID, [pinned, placed, twice].map((pid) => ({ pid, start, agent: "claude" })), T);
  await spawns.saw(ID, [{ pid: pinned, start, agent: "claude" }], T + 1000);
  assert.deepEqual(spawns.settle(T + 5000).map((s) => [s.session, s.cwd]), [["claude:0190d3e0-0000-7000-8000-00000000bb01", "/Users/nova/elsewhere"], [S1, ATLAS_DIR]], "two in ledger began just after: which is it isn't known yet");
  assert.deepEqual(spawns.settle(T + SPAWN_WAIT_MS - 1), []);
  assert.deepEqual(spawns.settle(T + SPAWN_WAIT_MS).map((s) => [s.session, s.cwd, s.agent]), [[null, LEDGER_DIR, "claude"]]);
  await spawns.saw(ID, [{ pid: placed, start, agent: "claude" }], T + SPAWN_WAIT_MS);
  assert.deepEqual(spawns.settle(T + 3 * SPAWN_WAIT_MS), [], "each process once");
  // A session matched once isn't matched again to another process.
  const other = m.add({ ppid: 1, program: "/x/claude", start: T, cwd: ATLAS_DIR });
  await spawns.saw(ID, [{ pid: other, start, agent: "claude" }], T + 10e3);
  assert.deepEqual(spawns.settle(T + 10e3), []);
});

test("Spawns: a process gone within seconds held no session, so neither yours begun after it nor a session started after two minutes is its", async () => {
  const store = new EngineStore(":memory:");
  const nodes = NODES();
  const map = fakeMap(nodes);
  const m = fakeProcesses();
  const procs = createProcs({ platform: "darwin", exec: m.exec });
  const alive = (s) => m.table.get(s.pid)?.start === s.start;
  const spawns = new Spawns({ store, map, procs });
  // claude --version, run by the actor in atlas-api, gone in a moment.
  const version = m.add({ ppid: 1, program: "/x/claude", start: T, cwd: ATLAS_DIR });
  await spawns.saw(ID, [{ pid: version, start: m.table.get(version).start, agent: "claude" }], T + 500);
  m.remove(version);
  assert.deepEqual(spawns.settle(T + 5000, { alive }), []);
  // Your own session, begun there half a minute later from your terminal.
  const S5 = "claude:0190d3e0-0000-7000-8000-00000000cc05";
  nodes[S5] = { kind: "session", app: "claude", home: ATLAS, cwd: ATLAS_DIR, createdAt: T + 30e3 };
  assert.deepEqual(spawns.settle(T + 35e3, { alive }), [], "yours began after it was gone");
  assert.deepEqual(spawns.settle(T + SPAWN_WAIT_MS + 5000, { alive }), [], "and it never held a session of its own");

  // One that runs on: a session another process is pinned to isn't its,
  // and with none known after two minutes, it counts in its place.
  const later = T + 10 * MIN;
  const S6 = "claude:0190d3e0-0000-7000-8000-00000000cc06";
  const long = m.add({ ppid: 1, program: "/x/claude", start: later, cwd: LEDGER_DIR });
  const yours = m.add({ ppid: 1, program: "/x/claude", start: later + 1000, cwd: LEDGER_DIR });
  nodes[S6] = { kind: "session", app: "claude", home: LEDGER, cwd: LEDGER_DIR, createdAt: later + 2000 };
  store.pin(S6, { pid: yours, start: m.table.get(yours).start }, true, later + 2000);
  await spawns.saw(ID, [{ pid: long, start: m.table.get(long).start, agent: "claude" }], later + 500);
  assert.deepEqual(spawns.settle(later + 5000, { alive }), [], "a session another process holds isn't its");
  assert.deepEqual(spawns.settle(later + SPAWN_WAIT_MS + 1000, { alive }).map((s) => [s.session, s.cwd]), [[null, LEDGER_DIR]]);
});
