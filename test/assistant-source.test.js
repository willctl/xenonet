// What a paired assistant reads from (core/assistant/source.js): this
// machine's places and sessions only, never another host's; what needs you
// by its kind, never its text; collisions by how many files, never which;
// each app's source as ready, degraded or unavailable from its gatherer and
// the tailer; one view a second however many reads come in; and actors
// reachable only when they report themselves. A made-up state and registry.
import { test } from "node:test";
import assert from "node:assert/strict";
import { stateSource } from "../core/assistant/source.js";

const T = 1791234567000;
const HERE = "claude:0190fa00-0000-7000-8000-000000000001";
const IDLE = "codex:0190fa00-0000-7000-8000-000000000002";
const THERE = "@studio:claude:0190fa00-0000-7000-8000-000000000003";

const running = (lastOk) => ({ health: () => ({ state: "running", lastOk }) });

// XenoNet's state as the service holds it: `nodes` on the map, `live` the
// tailer's sessions, and each block's health.
function xenonet({ nodes, live = [], collisions = [], claude = running(T - 5000), codex = running(T - 9000), tailer = running(T - 1000) } = {}) {
  const s = {
    snapshots: 0,
    index: new Map(nodes.map((n) => [n.id, n])),
    homeOf: (id) => (id === THERE ? "@studio:project:/users/nova/projects/atlas" : "project:/users/nova/projects/atlas"),
    liveSnapshot: () => (s.snapshots++, { sessions: live, collisions }),
    claudeSessions: claude,
    codexThreads: codex,
    live: { tailer },
  };
  return s;
}

const MAP = [
  { id: "project:/users/nova/projects/atlas", kind: "project", name: "atlas" },
  { id: "zone:/users/nova/projects/labs", kind: "zone", name: "labs" },
  { id: "@studio:project:/users/nova/projects/atlas", kind: "project", name: "atlas on the studio", host: "studio" },
  { id: HERE, kind: "session", app: "claude", title: "Fix the parser", updatedAt: T - 60e3 },
  { id: IDLE, kind: "session", app: "codex", archived: true },
  { id: THERE, kind: "session", app: "claude", title: "The studio's own work", host: "studio" },
  { id: "repo:atlas", kind: "repo", name: "atlas" },
];

test("the view holds this machine's places and sessions only, never another host's", () => {
  const src = stateSource({ state: xenonet({ nodes: MAP }), now: () => T });
  const v = src.view();
  assert.deepEqual([...v.places.entries()], [
    ["project:/users/nova/projects/atlas", { name: "atlas", kind: "project" }],
    ["zone:/users/nova/projects/labs", { name: "labs", kind: "zone" }],
  ]);
  assert.deepEqual(v.sessions.map((s) => s.id), [HERE, IDLE]);
  assert.ok(!JSON.stringify(v.sessions).includes("studio"), "nothing of the studio's");
  assert.deepEqual(v.sessions[1], { id: IDLE, app: "codex", place: "project:/users/nova/projects/atlas", title: null, updatedAt: null, archived: true, live: null });
  assert.equal(v.fixture, false);
  assert.equal(src.has(HERE), true);
  assert.equal(src.has("project:/users/nova/projects/elsewhere"), false);
});

test("what a live session is doing goes in by kind and time: never a question's text, a collision's files or anything else the tailer holds", () => {
  const live = [{ id: HERE, status: "waiting", needsYou: { kind: "question", text: "Shall I push the release key?", since: T - 30e3 }, stuck: { reasons: ["same-tool"], detail: "ran rm -rf build 12 times" }, turnStartedAt: T - 90e3, lastEventAt: T - 30e3, cwd: "/Users/nova/Projects/atlas", lastPrompt: "the secret plan" }];
  const collisions = [{ a: HERE, b: IDLE, files: ["/Users/nova/Projects/atlas/src/keys.ts", "/Users/nova/Projects/atlas/.env"], ts: T - 10e3 }];
  const v = stateSource({ state: xenonet({ nodes: MAP, live, collisions }), now: () => T }).view();
  assert.deepEqual(v.sessions[0].live, { status: "waiting", needsYou: { kind: "question", since: T - 30e3 }, stuck: { reasons: ["same-tool"] }, turnStartedAt: T - 90e3, lastTurnEndAt: null, interruptedAt: null, lastEventAt: T - 30e3 });
  assert.deepEqual(v.collisions, [{ a: HERE, b: IDLE, files: 2, ts: T - 10e3 }]);
  const text = JSON.stringify({ ...v, places: [...v.places] });
  for (const secret of ["release key", "rm -rf", "secret plan", "keys.ts", ".env", "/Users/nova/Projects"]) assert.ok(!text.includes(secret), secret);
});

test("each app's source: ready while its list and the tailer run, degraded when one has a problem, unavailable when the list failed, stopped or hasn't run", () => {
  const sources = (over) => stateSource({ state: xenonet({ nodes: MAP, ...over }), now: () => T }).view().sources;
  assert.deepEqual(sources({}).claude, { state: "ready", observedAt: T - 5000, coverage: "complete", reason: "ok" });
  // The tailer stopped a minute ago: what the list had, as of the older of
  // the two.
  const tailerDown = sources({ tailer: { health: () => ({ state: "failed", lastOk: T - 60e3 }) } });
  assert.deepEqual(tailerDown.claude, { state: "degraded", observedAt: T - 60e3, coverage: "partial", reason: "read_failed" });
  assert.deepEqual(sources({ tailer: null }).codex, { state: "degraded", observedAt: T - 9000, coverage: "partial", reason: "read_failed" }, "no tailer at all: the list's own time");
  assert.deepEqual(sources({ codex: { health: () => ({ state: "degraded", lastOk: T - 2000 }) } }).codex, { state: "degraded", observedAt: T - 2000, coverage: "partial", reason: "read_failed" });
  assert.deepEqual(sources({ claude: { health: () => ({ state: "failed", lastOk: T - 600e3 }) } }).claude, { state: "unavailable", observedAt: T - 600e3, coverage: "unknown", reason: "read_failed" });
  assert.deepEqual(sources({ claude: { health: () => ({ state: "stopped", lastOk: T - 600e3 }) } }).claude, { state: "unavailable", observedAt: T - 600e3, coverage: "unknown", reason: "stopped" });
  for (const notYet of [null, { health: () => ({ state: "idle", lastOk: null }) }, { health: () => ({ state: "running", lastOk: null }) }]) {
    assert.deepEqual(sources({ codex: notYet }).codex, { state: "unavailable", observedAt: null, coverage: "unknown", reason: "not_started" });
  }
});

test("one view a second, however many reads come in", () => {
  let t = T;
  const state = xenonet({ nodes: MAP });
  const src = stateSource({ state, now: () => t });
  const first = src.view();
  t += 999;
  assert.equal(src.view(), first, "the same view within the second");
  assert.equal(state.snapshots, 1);
  state.index.set("claude:0190fa00-0000-7000-8000-000000000009", { id: "claude:0190fa00-0000-7000-8000-000000000009", kind: "session", app: "claude" });
  t += 1;
  const next = src.view();
  assert.notEqual(next, first);
  assert.deepEqual([state.snapshots, next.at, next.sessions.length], [2, T + 1000, 3]);
});

test("actors: reachable only when they report themselves and aren't offline; a remote connection never makes one reachable", () => {
  const listed = [
    { id: "actor:wren", kind: "agent", name: "Wren", product: null, presence: "working", via: ["registered"], targets: [{ id: "project:/users/nova/projects/atlas", relation: "works-on", since: T }] },
    { id: "actor:kite", kind: "agent", name: "Kite", product: { id: "hermes-agent", vendor: "Nous Research" }, presence: "offline", via: ["registered", "observed"] },
    { id: "actor:dots", kind: "agent", name: "dots", product: { id: "dots" }, presence: "working", via: ["remote"], remote: { connections: 1 } },
    { id: "actor:moth", kind: "agent", name: "Moth", product: { id: "made-up-product" }, presence: "idle" },
  ];
  const registered = new Set(["actor:wren", "actor:kite"]);
  const actors = { list: ({ now }) => (assert.equal(now, T), { actors: listed }), isRegistered: (id) => registered.has(id), snapshot: (id) => listed.find((a) => a.id === id) ?? null };
  const src = stateSource({ state: xenonet({ nodes: MAP }), actors, now: () => T });
  const v = src.view();
  assert.deepEqual(v.actors.map((a) => [a.id, a.reachable, a.productId]), [["actor:wren", true, null], ["actor:kite", false, "hermes-agent"], ["actor:dots", false, "dots"], ["actor:moth", false, null]]);
  assert.deepEqual(v.actors[0].targets, [{ id: "project:/users/nova/projects/atlas", relation: "works-on" }], "a target's id and relation, nothing more");
  assert.deepEqual(v.actors[2].remote, { connections: 1 });
  listed[0].via.push("observed");
  assert.deepEqual(v.actors[0].via, ["registered"], "a copy, not the registry's own list");

  // One actor, as the assistant shows itself: never said to be reachable.
  assert.deepEqual(src.actor("actor:wren"), { id: "actor:wren", kind: "agent", productId: null, name: "Wren", presence: "working", via: ["registered", "observed"], remote: null, reachable: false, targets: [{ id: "project:/users/nova/projects/atlas", relation: "works-on" }] });
  assert.equal(src.actor("actor:nobody"), null);
  assert.equal(stateSource({ state: xenonet({ nodes: MAP }), now: () => T }).actor("actor:wren"), null, "no registry, no actors");
  assert.deepEqual(stateSource({ state: xenonet({ nodes: MAP }), now: () => T }).view().actors, []);
});
