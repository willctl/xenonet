// What each actor's work costs today (core/actors/spend.js): each
// session's calls given to at most one actor, by the rule (a run it
// reported, else a session it started, else the session it came from,
// else unattributed), the parts adding up to the day's estimate, and the
// registry's record of who started what, with you never named outside the
// demo. The demo's breakdown is worked out in a demo world of the test's
// own (XENONET_DEMO and a spare port before anything is imported); the
// rest on made-up actors, sessions and costs.
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
const { world, demoSessionList } = await import("../core/demo.js");
const { Actors, createActors, mapOf } = await import("../core/actors/index.js");
const { attribute, spendByActor, actorSpend, dayStart } = await import("../core/actors/spend.js");
const { LiveTelemetry } = await import("../core/live.js");
const { ReportedRuns } = await import("../core/reported-runs.js");
const { PRICES_CHECKED } = await import("../core/pricing.js");

const close = (a, b, what) => assert.ok(Math.abs(a - b) < 1e-9, `${what}: ${a} vs ${b}`);
const T = 1791230000000;
const MIN = 60e3;

// Made-up sessions: a few of yours, an agent's, its subagent and a
// session it spawned, a fork of the agent's, and a run reported through
// the gateway.
const S = (n, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(n).padStart(12, "0")}`;
const WREN = "actor:wren";
const MOTH = "actor:moth";
const KESTREL = "actor:kestrel";
const RUN = ReportedRuns.idOf(KESTREL, "codex", "nightly-bench");
const names = new Map([[WREN, "Wren"], [MOTH, "Moth"], [KESTREL, "Kestrel"]]);
const sessions = [
  { id: S(1) },
  { id: S(2) },
  { id: S(3, "codex") },
  { id: S(4, "codex"), parentId: S(3, "codex"), parentKind: "subagent" },
  { id: S(5), parentId: S(3, "codex"), parentKind: "spawn" },
  { id: S(6, "codex"), parentId: S(3, "codex"), parentKind: "fork" },
  { id: RUN, observed: "gateway", reportedBy: KESTREL, updatedAt: T },
];

test("a run is its reporter's, a session its starter's, and the sessions XenoNet can't tie to an actor no one's", () => {
  const starts = [
    { actor: WREN, session: S(3, "codex"), at: T - 30 * MIN, via: "attributed" },
    // Another actor's word that it started the reported run changes nothing:
    // the run's id comes from Kestrel's own token.
    { actor: MOTH, session: RUN, at: T - 60 * MIN, via: "registered" },
  ];
  const who = attribute({ sessions, starts, known: new Set(names.keys()) });
  assert.deepEqual(who.get(RUN), { actor: KESTREL, how: "reported" });
  assert.deepEqual(who.get(S(3, "codex")), { actor: WREN, how: "started" });
  // A subagent, and a session its process started, are the parent's work.
  assert.deepEqual(who.get(S(4, "codex")), { actor: WREN, how: "parent" });
  assert.deepEqual(who.get(S(5)), { actor: WREN, how: "parent" });
  // A fork isn't: whoever forked it may not be whoever started the original.
  assert.equal(who.get(S(6, "codex")), undefined);
  // Yours, as far as XenoNet can say: no one's.
  assert.equal(who.get(S(1)), undefined);
  assert.equal(who.get(S(2)), undefined);
});

test("when actors disagree, what XenoNet saw comes before what an actor said, then the earliest, then the id", () => {
  const known = new Set(names.keys());
  const said = { actor: MOTH, session: S(1), at: T - 90 * MIN, via: "registered" };
  const seen = { actor: WREN, session: S(1), at: T - 10 * MIN, via: "attributed" };
  assert.equal(attribute({ sessions, starts: [said, seen], known }).get(S(1)).actor, WREN, "seen beats said, however much earlier the word");
  assert.equal(attribute({ sessions, starts: [seen, said], known }).get(S(1)).actor, WREN, "in any order");
  const observed = { actor: KESTREL, session: S(1), at: T - 20 * MIN, via: "observed" };
  assert.equal(attribute({ sessions, starts: [seen, observed], known }).get(S(1)).actor, KESTREL, "the earliest of the seen");
  const tie = { actor: MOTH, session: S(1), at: T - 10 * MIN, via: "observed" };
  assert.equal(attribute({ sessions, starts: [seen, tie], known }).get(S(1)).actor, MOTH, "then the id");
  assert.equal(attribute({ sessions, starts: [tie, seen], known }).get(S(1)).actor, MOTH);
  // A session's own starter comes before its parent's.
  const child = { actor: MOTH, session: S(4, "codex"), at: T, via: "registered" };
  const parent = { actor: WREN, session: S(3, "codex"), at: T - 30 * MIN, via: "attributed" };
  assert.deepEqual(attribute({ sessions, starts: [parent, child], known }).get(S(4, "codex")), { actor: MOTH, how: "started" });
  // An actor that doesn't count (gone, or off the list) starts nothing.
  assert.equal(attribute({ sessions, starts: [{ actor: "actor:gone", session: S(2), at: T, via: "attributed" }], known }).get(S(2)), undefined);
  assert.equal(attribute({ sessions: [{ id: RUN, reportedBy: "actor:gone" }], known }).get(RUN), undefined);
});

test("lineage goes up at most 8 sessions, and a loop in it ends", () => {
  const chain = Array.from({ length: 11 }, (_, i) => ({ id: S(100 + i), parentId: i ? S(100 + i - 1) : null, parentKind: i ? "subagent" : null }));
  const who = attribute({ sessions: chain, starts: [{ actor: WREN, session: S(100), at: T, via: "attributed" }], known: new Set([WREN]) });
  assert.equal(who.get(S(108))?.actor, WREN, "8 up");
  assert.equal(who.get(S(109)), undefined, "9 up is too far");
  const loop = [
    { id: S(201), parentId: S(202), parentKind: "spawn" },
    { id: S(202), parentId: S(201), parentKind: "spawn" },
    { id: S(203), parentId: S(203), parentKind: "spawn" },
  ];
  assert.equal(attribute({ sessions: loop, starts: [], known: null }).size, 0);
});

test("spend by actor: each session counted once, the parts adding up to the day's, runs elsewhere apart", () => {
  const spend = [
    { id: S(1), cost: 1.5, tokens: 1000, unpriced: [] },
    { id: S(2), cost: 0.25, tokens: 300, unpriced: ["claude-made-up-9"] },
    { id: S(3, "codex"), cost: 2, tokens: 4000, unpriced: [] },
    { id: S(4, "codex"), cost: 0.5, tokens: 800, unpriced: ["gpt-made-up"] },
    { id: S(5), cost: 0.75, tokens: 900, unpriced: [] },
    { id: S(6, "codex"), cost: 3, tokens: 5000, unpriced: [] },
    // The same session twice is counted once.
    { id: S(5), cost: 0.75, tokens: 900, unpriced: [] },
  ];
  const starts = [{ actor: WREN, session: S(3, "codex"), at: T - 30 * MIN, via: "attributed" }];
  const out = spendByActor({ spend, sessions, starts, names, since: T - 12 * 3600e3 });
  close(out.total, 1.5 + 0.25 + 2 + 0.5 + 0.75 + 3, "total");
  close(out.actors.reduce((a, r) => a + r.cost, 0) + out.unattributed.cost, out.total, "the parts add up");
  assert.deepEqual(
    out.actors.map((r) => [r.id, r.name, r.sessions, r.tokens, r.unpriced, r.elsewhere]),
    [
      [WREN, "Wren", 3, 4000 + 800 + 900, ["gpt-made-up"], 0],
      // Kestrel's run reported from elsewhere has no transcript here, so
      // its cost isn't known: it's listed, with nothing counted.
      [KESTREL, "Kestrel", 0, 0, [], 1],
    ],
  );
  close(out.actors[0].cost, 3.25, "Wren");
  assert.equal(out.actors[1].cost, 0);
  assert.deepEqual([out.unattributed.sessions, out.unattributed.tokens, out.unattributed.unpriced], [3, 1000 + 300 + 5000, ["claude-made-up-9"]]);
  close(out.unattributed.cost, 4.75, "unattributed");
  assert.ok(!("id" in out.unattributed), "no actor's id");
  // A run that last reported before today isn't today's.
  assert.deepEqual(spendByActor({ spend: [], sessions, starts, names, since: T + 1 }).actors, []);
  // The most spent first, then the most sessions, then by name.
  const many = spendByActor({
    spend: [{ id: S(1), cost: 1, tokens: 1, unpriced: [] }, { id: S(2), cost: 1, tokens: 1, unpriced: [] }, { id: S(3, "codex"), cost: 1, tokens: 1, unpriced: [] }, { id: S(7), cost: 5, tokens: 1, unpriced: [] }],
    sessions: [],
    starts: [
      { actor: MOTH, session: S(1), at: T, via: "registered" },
      { actor: WREN, session: S(2), at: T, via: "registered" },
      { actor: KESTREL, session: S(3, "codex"), at: T, via: "registered" },
      { actor: KESTREL, session: S(7), at: T, via: "registered" },
    ],
    names: Object.fromEntries(names),
  });
  assert.deepEqual(many.actors.map((r) => r.name), ["Kestrel", "Moth", "Wren"]);
  assert.deepEqual(spendByActor(), { total: 0, actors: [], unattributed: { cost: 0, tokens: 0, sessions: 0, unpriced: [] } });
});

// A made-up map for the registry: a project and the sessions above.
const ATLAS = "project:/users/nova/projects/atlas-api";
function fakeMap() {
  const nodes = new Map([[ATLAS, { kind: "project", name: "atlas-api" }], ...sessions.map((s) => [s.id, { kind: "session", app: s.id.split(":")[0], home: ATLAS }])]);
  return {
    ready: () => true,
    has: (id) => nodes.has(id),
    describe(id) {
      const n = nodes.get(id);
      if (!n) return null;
      if (n.kind !== "session") return { kind: n.kind, app: null, name: n.name, place: null, placeId: null };
      return { kind: "session", app: n.app, name: null, place: "atlas-api", placeId: n.home };
    },
    sessions: () => [],
  };
}

test("the registry says who started what: spawned events and spawned targets, from the actors listed, and never you outside the demo", () => {
  const clock = { t: T };
  const actors = new Actors({ now: () => clock.t, map: fakeMap() });
  actors.upsert({ id: WREN, name: "Wren", product: { id: "hermes-agent", vendor: "Nous Research", version: null }, via: ["observed", "attributed"], heartbeatMs: 60000, capabilities: ["spawn-sessions"] });
  actors.upsert({ id: MOTH, name: "Moth", via: ["registered"], heartbeatMs: 30000, capabilities: ["spawn-sessions"], allowedAt: T - 3600e3 });
  actors.record(WREN, { kind: "spawned", target: S(3, "codex"), via: "attributed", at: T - 40 * MIN });
  actors.record(WREN, { kind: "spawned", target: S(3, "codex"), via: "attributed", at: T - 20 * MIN });
  actors.record(WREN, { kind: "spawned", target: ATLAS, at: T - 10 * MIN });
  // Moth holds one it says it spawned, with no event.
  actors.target(MOTH, S(2), "spawned", T - 5 * MIN);
  // You started a session in a place: never which one. And had you
  // started one by name, outside the demo it still wouldn't be said.
  actors.you.report({ presence: "working", did: [{ what: "started", node: ATLAS, app: "claude" }] }, { at: T - MIN });
  actors.record("actor:you", { kind: "spawned", target: S(1), via: "observed", at: T - MIN });
  const { names: listed, starts } = actors.sessionsStarted();
  assert.deepEqual([...listed], [[WREN, "Wren"], [MOTH, "Moth"]], "the actors listed, without you");
  const key = (s) => `${s.actor} ${s.session} ${s.via} ${s.at}`;
  assert.deepEqual(starts.map(key).sort(), [
    `${MOTH} ${S(2)} registered ${T - 5 * MIN}`,
    `${WREN} ${S(3, "codex")} attributed ${T - 40 * MIN}`,
    `${WREN} ${S(3, "codex")} observed ${T - 40 * MIN}`,
  ].sort());
  // An observed actor turned off goes out of the list, and with it its claims.
  actors.setEnabled(WREN, false);
  assert.deepEqual(actors.sessionsStarted().starts.map((s) => s.actor), [MOTH]);
  actors.stop();
});

test("the server's answer: the tailer's costs today, by actor, adding up to Token use's estimate", () => {
  const now = Date.now();
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  const ts = new Date(now).toISOString();
  const reply = (id, usage) => ({ type: "assistant", timestamp: ts, message: { id, model: "claude-opus-5-5", role: "assistant", content: [{ type: "text", text: "ok" }], usage } });
  live.applyClaude({ sessionId: S(1) }, reply("msg_1", { input_tokens: 1000, output_tokens: 500 }), false);
  live.applyClaude({ sessionId: S(5) }, reply("msg_2", { input_tokens: 2000, output_tokens: 100 }), false);
  live.applyClaude({ sessionId: S(5) }, reply("msg_3", { input_tokens: 10, output_tokens: 10 }), false);
  const actors = new Actors({ now: () => now, map: fakeMap() });
  actors.upsert({ id: WREN, name: "Wren", via: ["observed", "attributed"], heartbeatMs: 60000, capabilities: ["spawn-sessions"] });
  actors.record(WREN, { kind: "spawned", target: S(3, "codex"), via: "attributed", at: now - 30 * MIN });
  const nodes = [{ id: ATLAS, kind: "project" }, ...sessions.map((s) => ({ ...s, kind: "session" }))];
  const out = actorSpend({ live, nodes, actors, now });
  assert.deepEqual([out.at, out.since, out.checked], [now, dayStart(now), PRICES_CHECKED]);
  close(out.total, live.costs(now, []).today, "the same day's estimate");
  assert.deepEqual(out.actors.map((r) => [r.name, r.sessions]), [["Wren", 1]], "S5 came from Wren's session");
  assert.equal(out.unattributed.sessions, 1);
  close(out.actors[0].cost + out.unattributed.cost, out.total, "the parts add up");
  assert.equal(actorSpend({ live, nodes, actors: null }), null, "with actors off, none");
  actors.stop();
});

// ---------- the demo ----------

const state = new XenoNetState();
after(() => {
  state.stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});

test("the demo has a made-up breakdown: Wren's sessions, and the rest unattributed", async () => {
  const graph = await state.refresh();
  const actors = createActors({ demo: true, map: mapOf(state) });
  const titled = (title) => demoSessionList().find((s) => s.title === title).id;
  // Made-up costs for every session on the map, a dollar each.
  const onMap = graph.nodes.filter((n) => n.kind === "session");
  const live = { sessionCosts: () => onMap.map((n) => ({ id: n.id, app: n.app, cost: 1, tokens: 100, unpriced: [] })) };
  const out = actorSpend({ live, nodes: graph.nodes, actors });
  const wren = out.actors.find((r) => r.id === WREN);
  assert.ok(wren, "Wren is on it");
  assert.equal(wren.name, "Wren");
  const who = attribute({ sessions: onMap, starts: actors.sessionsStarted().starts, known: new Set([WREN, MOTH, KESTREL, "actor:you"]) });
  assert.equal(who.get(titled("Add partial refunds to the orders API"))?.actor, WREN);
  assert.equal(who.get(titled("Draft the Q4 close checklist"))?.actor, WREN);
  assert.deepEqual(who.get(titled("Review the partial refunds change")), { actor: WREN, how: "parent" }, "a session the refunds session started");
  assert.equal(wren.sessions, 3);
  assert.ok(out.unattributed.sessions > 0, "the rest");
  assert.equal(out.total, onMap.length);
  assert.ok(!out.actors.some((r) => r.id === "actor:you"), "the made-up you starts sessions in places, never a session");
  actors.stop();
});
