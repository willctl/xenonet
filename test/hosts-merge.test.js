// Merging hosts into this XenoNet's view (core/hosts/merge.js). The demo
// world stands in for both machines: its map and live state as this one's,
// and the same again as another host's, the way its relay would send them
// (paths as ~, repo keys in). Made-up graphs cover what a host mustn't
// send. Checked strictly against the schemas the API answers with.
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
const { config } = await import("../core/config.js");
const { world } = await import("../core/demo.js");
const { check, explain } = await import("../core/schema/index.js");
const { shapeGraph, shapeLive, homeScrubber } = await import("../core/hosts/relay.js");
const { mergeGraph, mergeLive, mergeSummary, qualifyGraph, qualifyEvent, qualifyEngine, eventNote, sameRepo, mergeIdentities, mergeLimits, remoteId, identityId, cleanNode, cleanOnce, MAX_NODES } = await import("../core/hosts/merge.js");
const { parseSshAliases } = await import("../core/identities.js");
const { declared } = await import("../core/hosts/declared.js");
const { z } = await import("../core/schema/common.js");

const state = new XenoNetState();
after(() => {
  state.stop();
  fs.rmSync(world.root, { recursive: true, force: true, maxRetries: 10 });
});
await state.live.start();
state.live.stop();
await state.refresh();
await state.refreshRepos();

const NOW = Date.now();
const MIN = 60e3;
const SESSION = (i, app = "claude") => `${app}:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`;
const valid = (id, data, what = id) => {
  const c = check(id, JSON.parse(JSON.stringify(data)), { strict: true });
  assert.ok(c.ok, `${what} doesn't fit ${id}:\n${explain(c)}`);
};

// What the demo-b host's relay would send: this world's own, shaped and
// with its home folder as ~.
const scrub = homeScrubber(config.homeDir);
const aliases = parseSshAliases(fs.readFileSync(`${world.home}/.ssh/config`, "utf8"));
const relayed = () => {
  const { graph, dropped } = shapeGraph(state.graph, { now: NOW, aliases, detail: true });
  return { graph: scrub(graph), live: scrub(shapeLive(state.liveSnapshot(), { dropped, detail: true })), engine: null };
};
const live = (extra = {}) => ({ id: "demo-b", state: "live", since: NOW - 10 * MIN, lastHeard: NOW, offsetMs: 0, hello: { id: "lab", key: "9a2b0000000000000000000000000002", version: "0.4.0", platform: "linux", demo: true }, error: null, ...extra });

test("another host's map joins this one under @<host>: ids, hanging from a host node of its own", () => {
  const local = state.graph;
  valid("graph/v1", local, "this machine's own map");
  const merged = mergeGraph(local, [{ info: live(), snapshot: relayed() }], { now: NOW, windows: config, aliases, self: "studio" });
  valid("graph/v1", merged, "the merged map");
  const remote = merged.nodes.filter((n) => n.host === "demo-b");
  assert.equal(remote.length, local.nodes.length, "every node of the other world, its core as its host node");
  assert.ok(merged.nodes.slice(0, local.nodes.length).every((n, i) => n.id === local.nodes[i].id), "this machine's ids don't change");
  const host = merged.nodes.find((n) => n.id === "@demo-b:core");
  assert.deepEqual([host.kind, host.name, host.path, host.calls, host.state, host.stale, host.demo, host.version], ["host", "demo-b", "~/Projects", "lab", "live", false, true, "0.4.0"]);
  assert.ok(merged.links.some((l) => l.source === "core" && l.target === "@demo-b:core" && l.kind === "host"));
  assert.ok(merged.nodes.some((n) => n.id === "@demo-b:project:~/projects/atlas-api"), "path keys as the host tilded them");
  assert.ok(merged.nodes.some((n) => n.id === `@demo-b:${SESSION(1)}`));
  // One tree: every node from the host has one parent, and they all lead to it.
  const parent = new Map(merged.links.filter((l) => l.kind !== "lineage-cross").map((l) => [l.target, l.source]));
  for (const n of remote) {
    let cur = n.id;
    for (let i = 0; i < 20 && cur !== "core"; i++) cur = parent.get(cur);
    assert.equal(cur, "core", `${n.id} leads back to this machine's core`);
  }
  assert.equal(merged.self, "studio");
  assert.deepEqual(merged.hosts.map((h) => [h.id, h.state, h.nodes, h.sessions]), [["demo-b", "live", local.nodes.length - 1, local.stats.sessions]]);
  assert.equal(merged.stats.sessions, local.stats.sessions * 2, "counts add up");
});

test("checkouts of one repo on two machines are bridged", () => {
  const merged = mergeGraph(state.graph, [{ info: live(), snapshot: relayed() }], { now: NOW, windows: config, aliases });
  const bridges = merged.relations.filter((r) => r.kind === "same-repo");
  const atlas = state.graph.nodes.find((n) => n.name === "atlas-api" && n.kind === "project").id;
  assert.ok(bridges.some((r) => r.a === atlas && r.b === "@demo-b:project:~/projects/atlas-api"), "atlas-api here and on demo-b");
  assert.ok(bridges.some((r) => r.a.endsWith("/route-sim") && r.b.endsWith("/route-sim")), "route-sim too, its SSH alias resolved on each machine");
  assert.equal(merged.nodes.find((n) => n.id === atlas).repoKey, "github.com/atlas-co/atlas-api", "this machine's projects get their keys here");
  assert.ok(!bridges.some((r) => !r.a.startsWith("@") && !r.b.startsWith("@")), "never between two checkouts on one machine");
  assert.ok(!bridges.some((r) => merged.nodes.find((n) => n.id === r.a)?.satelliteOf), "a satellite is left to its repo");
  assert.deepEqual(state.graph.nodes.find((n) => n.id === atlas).repoKey, undefined, "this XenoNet's own map isn't changed by merging");
  // At most a few per repo, however many clones.
  const clones = Array.from({ length: 12 }, (_, i) => ({ id: `@h${i}:project:x`, kind: "project", host: `h${i}`, repoKey: "github.com/a/b" }));
  assert.equal(sameRepo(clones).length, 8);
});

test("the console's layout, not the host's; Preview and Stats only; activity by this machine's clock, three minutes behind the host's", () => {
  const snap = relayed();
  const s1 = snap.graph.nodes.find((n) => n.id === SESSION(1));
  s1.fx = 1;
  s1.pinned = true; // where the host's own person put it
  s1.updatedAt = NOW - 2 * MIN + 3 * MIN; // two minutes ago, by a clock three minutes ahead
  const s2 = snap.graph.nodes.find((n) => n.id === SESSION(2, "codex"));
  s2.updatedAt = NOW - 9 * MIN + 3 * MIN;
  const merged = mergeGraph(state.graph, [{ info: live({ offsetMs: 3 * MIN }), snapshot: snap }], { now: NOW, windows: config, aliases, layout: { [`@demo-b:${SESSION(2, "codex")}`]: { x: 10, y: 20, z: 30 } } });
  const r1 = merged.nodes.find((n) => n.id === `@demo-b:${SESSION(1)}`);
  const r2 = merged.nodes.find((n) => n.id === `@demo-b:${SESSION(2, "codex")}`);
  assert.deepEqual([r1.fx, r1.pinned], [null, false], "the host's layout is dropped");
  assert.deepEqual([r2.fx, r2.fy, r2.fz, r2.pinned], [10, 20, 30, true], "the console's applies under the qualified id");
  assert.deepEqual(r1.actions, [{ id: "preview", label: "Preview transcript", client: true }, { id: "stats", label: "Stats", client: true }]);
  assert.ok(merged.nodes.filter((n) => n.host === "demo-b" && n.kind !== "session").every((n) => n.actions.length === 0), "nothing opens on this machine for another's places");
  assert.equal(r1.updatedAt, NOW - 2 * MIN, "times move by the offset");
  assert.equal(r1.activity, "active");
  assert.equal(r2.activity, "active", "nine minutes ago by this clock is still within ten");
  const later = mergeGraph(state.graph, [{ info: live({ offsetMs: 3 * MIN }), snapshot: snap }], { now: NOW + 2 * MIN, windows: config, aliases });
  assert.equal(later.nodes.find((n) => n.id === `@demo-b:${SESSION(2, "codex")}`).activity, "recent", "worked out again by this machine's clock");
  assert.ok(later.nodes.find((n) => n.id === "@demo-b:core").activeCount < merged.nodes.find((n) => n.id === "@demo-b:core").activeCount);
});

test("a host that isn't live: its nodes kept and marked stale, nothing busy, a question as of when it was heard", () => {
  const snap = relayed();
  const info = live({ state: "offline", lastHeard: NOW - 30 * MIN, since: NOW - 29 * MIN, error: { code: "unreachable", message: "ssh couldn't reach demo-b" } });
  const merged = mergeGraph(state.graph, [{ info: { ...info, cached: true }, snapshot: snap }], { now: NOW, windows: config, aliases });
  valid("graph/v1", merged);
  const remote = merged.nodes.filter((n) => n.host === "demo-b");
  assert.ok(remote.length > 1 && remote.every((n) => n.stale === true));
  const host = remote.find((n) => n.kind === "host");
  assert.deepEqual([host.state, host.error, host.cached], ["offline", "ssh couldn't reach demo-b", true]);

  const merged2 = mergeLive(state.liveSnapshot(), [{ info, snapshot: snap }]);
  valid("live/snapshot/v1", merged2);
  const theirs = merged2.sessions.filter((s) => s.host === "demo-b");
  assert.ok(theirs.length > 0 && theirs.every((s) => s.stale && s.status === "idle" && s.tpm === 0 && s.stuck === null));
  const asking = theirs.find((s) => s.needsYou?.kind === "question");
  assert.equal(asking.needsYou.asOf, NOW - 30 * MIN);
  assert.ok(!merged2.collisions.some((c) => c.a.startsWith("@")), "collisions that involve it are dropped");
  assert.equal(merged2.tpm, state.liveSnapshot().tpm, "it adds nothing to the rates");

  // Never heard from at all: a host node, alone.
  const lonely = mergeGraph(state.graph, [{ info: live({ state: "connecting", hello: null, lastHeard: null }), snapshot: null }], { now: NOW, windows: config });
  valid("graph/v1", lonely);
  assert.deepEqual(lonely.nodes.filter((n) => n.host === "demo-b").map((n) => n.id), ["@demo-b:core"]);
});

test("live state: sessions qualified, rates and today's tokens added, the series lined up by the clock offset", () => {
  const snap = relayed();
  const local = state.liveSnapshot();
  snap.live.series = { claude: local.series.claude.map(() => 0), codex: local.series.codex.map(() => 0) };
  snap.live.series.claude[snap.live.series.claude.length - 1] = 1000; // its last second
  snap.live.at = local.at + 3 * MIN - 5000; // five seconds before this one's, by a clock three minutes ahead
  snap.live.tpm = 120;
  const merged = mergeLive(local, [{ info: live({ offsetMs: 3 * MIN }), snapshot: snap }]);
  valid("live/snapshot/v1", merged);
  assert.equal(merged.sessions.filter((s) => s.host === "demo-b").length, snap.live.sessions.length);
  assert.ok(merged.sessions.some((s) => s.id === `@demo-b:${SESSION(1)}`));
  assert.equal(merged.tpm, local.tpm + 120);
  assert.equal(merged.today, local.today + snap.live.today);
  const n = local.series.claude.length;
  assert.equal(merged.series.claude[n - 1 - 5] - local.series.claude[n - 1 - 5], 1000, "five seconds back");
  assert.ok(merged.collisions.some((c) => c.a.startsWith("@demo-b:") && c.b.startsWith("@demo-b:")), "a host's own collisions, qualified");
  assert.equal(local.sessions.some((s) => s.host), false, "this machine's own state isn't changed by merging");
});

test("a host's sessions' times move by its clock's offset, when you stopped a turn too", () => {
  const snap = relayed();
  const [stopped, other] = snap.live.sessions;
  // By a clock three minutes ahead of this one's.
  Object.assign(stopped, { lastTurnEndAt: NOW + 3 * MIN, interruptedAt: NOW + 3 * MIN });
  Object.assign(other, { interruptedAt: null });
  const merged = mergeLive(state.liveSnapshot(), [{ info: live({ offsetMs: 3 * MIN }), snapshot: snap }]);
  valid("live/snapshot/v1", merged);
  const theirs = (s) => merged.sessions.find((x) => x.id === `@demo-b:${s.id}`);
  assert.deepEqual([theirs(stopped).lastTurnEndAt, theirs(stopped).interruptedAt], [NOW, NOW]);
  assert.equal(theirs(other).interruptedAt, null);
});

test("plan limits merge per account: a newer reading of the same account wins, another account is kept as its own", () => {
  const mine = { fiveHour: 40, sevenDay: 50, resets: {}, observedAt: NOW - 10 * MIN, source: "anthropic", identity: "claude:a:o" };
  const codex = { primary: null, secondary: null, observedAt: NOW - MIN, identity: "chatgpt:w" };
  const newer = { ...mine, fiveHour: 55, observedAt: NOW - MIN };
  const merged = mergeLimits({ claude: mine, codex }, [
    { app: "claude", host: "vm1", reading: newer },
    { app: "claude", host: "vm2", reading: { ...mine, observedAt: NOW - 20 * MIN } },
    { app: "claude", host: "vm2", reading: { ...mine, identity: "claude:b:o", fiveHour: 5 } },
    { app: "claude", host: "vm3", reading: { ...mine, identity: "claude:b:o", fiveHour: 7, observedAt: NOW } },
    { app: "codex", host: "vm1", reading: { ...codex, identity: "@vm1:openai:api-key" } },
  ]);
  assert.deepEqual(merged.claude, { ...newer, host: "vm1" }, "the newest reading of this account, wherever it was read");
  assert.equal(merged.codex, codex);
  assert.deepEqual(merged.others.map((o) => [o.app, o.identity, o.host, o.reading.fiveHour ?? null]), [["claude", "claude:b:o", "vm3", 7], ["codex", "@vm1:openai:api-key", "vm1", null]]);
  // Through mergeLive, from what a host sends: its reading's time moves.
  const snap = relayed();
  snap.live.limits = { claude: { ...mine, observedAt: NOW + 3 * MIN }, codex: null };
  const live2 = mergeLive({ ...state.liveSnapshot(), limits: { claude: { ...mine, observedAt: NOW - 30 * MIN }, codex: null } }, [{ info: live({ offsetMs: 3 * MIN }), snapshot: snap }]);
  assert.deepEqual([live2.limits.claude.observedAt, live2.limits.claude.host], [NOW, "demo-b"]);
  valid("live/snapshot/v1", live2);
  // A reading that doesn't fit is rebuilt from what does, or left out.
  snap.live.limits = { claude: { fiveHour: "lots", sevenDay: 9, resets: "soon", observedAt: NOW, source: "guess", identity: "claude:b:o", evil: { x: 1 } }, codex: { primary: "full", observedAt: "yesterday", identity: "chatgpt:w" } };
  const live3 = mergeLive(state.liveSnapshot(), [{ info: live(), snapshot: snap }]);
  valid("live/snapshot/v1", live3);
  assert.deepEqual(live3.limits.others, [{ app: "claude", identity: "claude:b:o", host: "demo-b", reading: { fiveHour: null, sevenDay: 9, resets: { fiveHour: null, sevenDay: null }, observedAt: NOW, source: "anthropic", identity: "claude:b:o" } }]);
});

test("identities: one per account across machines, counts added, an API key kept to its machine", () => {
  const local = [{ id: "claude:a:o", kind: "claude", handle: null, detail: {}, signedIn: true, active: false, sources: ["~/.claude.json"], firstSeen: NOW - 9e6, name: "work", hidden: false, sessions: 3, projects: 1 }];
  const theirs = [
    { id: "claude:a:o", kind: "claude", handle: "nova@example.com", detail: { plan: "Max" }, signedIn: true, active: false, sources: ["~/.claude.json"], firstSeen: NOW - 9e7, name: "theirs", hidden: false, sessions: 2, projects: 2 },
    { id: "openai:api-key", kind: "openai", handle: null, detail: {}, signedIn: true, active: false, sources: [], firstSeen: NOW, name: "lab key", hidden: false, sessions: 1, projects: 0 },
  ];
  const merged = mergeIdentities(local, [{ host: "vm1", identities: theirs.map((i) => ({ ...i, id: identityId("vm1", i.id) })) }], { names: {}, hidden: { "@vm1:openai:api-key": true } });
  const claude = merged.find((i) => i.id === "claude:a:o");
  assert.deepEqual([claude.sessions, claude.projects, claude.handle, claude.name, claude.detail.plan, claude.firstSeen], [5, 3, "nova@example.com", "work", "Max", NOW - 9e7]);
  assert.deepEqual([claude.hosts, claude.signedInOn, claude.sources], [[null, "vm1"], [null, "vm1"], ["~/.claude.json", "vm1: ~/.claude.json"]]);
  const key = merged.find((i) => i.id === "@vm1:openai:api-key");
  assert.deepEqual([key.name, key.hidden, key.hosts], ["lab key", true, ["vm1"]], "a name given there fills in, and your hiding applies");
  for (const i of merged) valid("identity/v1", i, i.id);
  assert.equal(identityId("vm1", "@vm2:openai:api-key"), null);
  // An id named like what every object has finds none of your names.
  const odd = mergeIdentities([], [{ host: "vm1", identities: ["constructor", "toString"].map((id) => ({ ...theirs[0], id, name: null })) }], { names: {}, hidden: {} });
  assert.deepEqual(odd.map((i) => [i.id, i.name, i.hidden]), [["constructor", null, false], ["toString", null, false]]);
  // And through a whole map.
  const snap = relayed();
  const merged2 = mergeGraph(state.graph, [{ info: live(), snapshot: snap }], { now: NOW, windows: config });
  for (const i of state.graph.identities) assert.equal(merged2.identities.find((x) => x.id === i.id).sessions, i.sessions * 2, i.id);
});

test("what a host mustn't send is dropped: ids already qualified, a second core, nodes that don't fit, loops, and more than 5,000 nodes", () => {
  const placed = { fx: 5, fy: 5, fz: 5, pinned: true, actions: [] };
  const rolled = { sessionCount: 0, activeCount: 0, lastActivity: null, lastSessionAt: null };
  const session = (id, extra = {}) => ({ id, kind: "session", app: "claude", rawId: "x", title: "t", preview: null, cwd: null, createdAt: null, updatedAt: NOW, archived: false, model: null, branch: null, parentId: null, parentKind: null, activity: "active", ...placed, actions: [{ id: "open-app", label: "Open" }], ...extra });
  const zone = (id) => ({ id, kind: "zone", name: id, path: null, app: null, external: true, ...placed, ...rolled, active: false });
  const graph = {
    nodes: [
      { id: "core", kind: "core", name: "~/Projects", path: "~/Projects", home: "~", ...placed, ...rolled },
      session("claude:ok"),
      session("@vm9:claude:smuggled"),
      session("@studio:claude:pose-as-this-machine"),
      { ...zone("core"), kind: "zone" }, // another node called core
      { id: "x".repeat(3000), kind: "zone", name: "long", path: null, app: null, external: true, ...placed, ...rolled, active: false },
      session("claude:bad", { activity: "busy" }),
      session("claude:typed", { title: { toString: "evil" } }),
      { id: "comet:1", kind: "comet", ...placed },
      // Kinds every object has: no kind at all, rather than a crash.
      { id: "zone:proto", kind: "__proto__", ...placed },
      { id: "zone:string", kind: "toString", ...placed },
      zone("zone:a"),
      zone("zone:b"),
      zone("zone:c"),
      session("claude:loop", { parentId: "@vm9:claude:smuggled", parentKind: "spawn" }),
    ],
    links: [
      { source: "core", target: "claude:ok", kind: "session" },
      { source: "core", target: "@vm9:claude:smuggled", kind: "session" },
      { source: "zone:a", target: "zone:b", kind: "zone" },
      { source: "zone:b", target: "zone:a", kind: "zone" }, // a loop, cut off from the rest
      { source: "@vm9:claude:smuggled", target: "claude:loop", kind: "lineage" },
      { source: "core", target: "zone:c", kind: "orbit" },
      { source: "core", target: "zone:c", kind: "zone" },
      { source: "core", target: "zone:c", kind: "zone" }, // twice
    ],
    relations: [{ a: "zone:a", b: "@vm9:claude:smuggled", kind: "wikilink" }, { a: "zone:a", b: "zone:b", kind: "teleport" }],
    identities: [{ id: "@vm9:claude:x", kind: "claude" }],
  };
  const part = qualifyGraph(graph, live(), { now: NOW });
  const ids = part.nodes.map((n) => n.id);
  assert.deepEqual(ids.sort(), ["@demo-b:claude:loop", "@demo-b:claude:ok", "@demo-b:core", "@demo-b:zone:a", "@demo-b:zone:b", "@demo-b:zone:c"].sort());
  assert.equal(part.dropped, 9);
  assert.ok(part.links.every((l) => !/@vm9|@studio/.test(l.source + l.target)));
  const parent = new Map(part.links.filter((l) => l.kind !== "lineage-cross").map((l) => [l.target, l.source]));
  assert.equal(parent.size, ids.length, "one parent each, the host node's being this machine's core");
  for (const id of ids) {
    let cur = id;
    for (let i = 0; i < 10 && cur !== "core"; i++) cur = parent.get(cur);
    assert.equal(cur, "core", `${id} is in the tree`);
  }
  assert.equal(parent.get("@demo-b:claude:loop"), "@demo-b:core", "its parent never came");
  assert.equal(part.nodes.find((n) => n.id === "@demo-b:claude:loop").parentId, null);
  assert.deepEqual(part.relations, []);
  assert.deepEqual(part.identities, []);
  assert.equal(remoteId("vm1", "@vm1:x"), null);

  // More than 5,000 nodes: places first, then the newest sessions.
  const many = { nodes: [{ id: "core", kind: "core" }, zone("zone:a"), ...Array.from({ length: MAX_NODES + 50 }, (_, i) => session(`claude:s${i}`, { updatedAt: NOW - i * 1000 }))], links: [] };
  const big = qualifyGraph(many, live(), { now: NOW });
  assert.equal(big.nodes.length, MAX_NODES);
  assert.equal(big.truncated, true);
  assert.ok(big.nodes.some((n) => n.id === "@demo-b:zone:a") && big.nodes.some((n) => n.id === "@demo-b:claude:s0") && !big.nodes.some((n) => n.id === `@demo-b:claude:s${MAX_NODES + 49}`));
  assert.equal(big.hostNode.truncated, true);

  // And so for the rest of what a host sends: live sessions, collisions,
  // relations, identities.
  const one = relayed().live.sessions[0];
  const crowd = { ...relayed().live, sessions: Array.from({ length: 1500 }, (_, i) => ({ ...one, id: `claude:crowd-${i}` })), collisions: Array.from({ length: 900 }, () => ({ a: "claude:a", b: "claude:b", files: [], paths: [], ts: NOW, branches: [null, null], sameBranch: false, titles: ["a", "b"] })) };
  const merged = mergeLive(state.liveSnapshot(), [{ info: live(), snapshot: { live: crowd } }]);
  assert.equal(merged.sessions.filter((x) => x.host === "demo-b").length, 1000);
  assert.equal(merged.collisions.filter((c) => c.a.startsWith("@")).length, 200);
});

test("events and the engine come across qualified, and only the kinds the page draws", () => {
  const h = live({ offsetMs: 3 * MIN });
  assert.deepEqual(qualifyEvent("activity", { sessionId: SESSION(1), app: "claude", kind: "prompt", detail: "", ts: NOW + 3 * MIN }, h), { sessionId: `@demo-b:${SESSION(1)}`, app: "claude", kind: "prompt", detail: "", ts: NOW, host: "demo-b" });
  const shipIt = qualifyEvent("activity", { sessionId: SESSION(1), app: "claude", kind: "ship-it", detail: "2 uncommitted files in atlas-api", projectId: "project:~/projects/atlas-api", ts: NOW }, h);
  assert.equal(shipIt.projectId, "@demo-b:project:~/projects/atlas-api");
  valid("live/activity/v1", shipIt);
  assert.equal(qualifyEvent("activity", { sessionId: "@vm9:claude:x", app: "claude", kind: "prompt", detail: "", ts: NOW }, h), null);
  assert.equal(qualifyEvent("activity", { sessionId: SESSION(1), app: "claude", kind: "exploded", ts: NOW }, h), null, "a kind that doesn't fit");
  assert.equal(qualifyEvent("activity", { kind: "identity", app: "claude", identity: "claude:a:o", before: null, ts: NOW }, h), null, "a host's sign-ins are its own");
  assert.deepEqual(qualifyEvent("tokens", { sessionId: SESSION(1), app: "claude", n: 40, ts: NOW }, h), { sessionId: `@demo-b:${SESSION(1)}`, app: "claude", n: 40, ts: NOW - 3 * MIN, host: "demo-b" });
  for (const app of ["grok-build", "gemini-cli", "", undefined, 5]) assert.equal(qualifyEvent("tokens", { sessionId: SESSION(1), app, n: 40, ts: NOW }, h), null, `tokens for ${app} aren't counted as Claude Code's`);
  assert.equal(eventNote("tokens", { sessionId: SESSION(1), app: "grok-build", n: 40, ts: NOW }, h), 'demo-b sent tokens for "grok-build", an app this XenoNet doesn\'t tail, and they were left out');
  assert.equal(eventNote("tokens", { sessionId: SESSION(1), n: 40 }, h), "demo-b sent tokens for null, an app this XenoNet doesn't tail, and they were left out");
  assert.equal(eventNote("tokens", { sessionId: SESSION(1), app: "codex", n: 40 }, h), null, "nothing to note for an app it tails");
  assert.equal(eventNote("pulse", { nodeId: SESSION(1) }, h), null);
  assert.deepEqual(qualifyEvent("pulse", { nodeId: SESSION(1), reason: "updated" }, h), { nodeId: `@demo-b:${SESSION(1)}`, reason: "updated", host: "demo-b" });
  assert.deepEqual(qualifyEvent("agent-message", { id: 7, from: SESSION(1), to: [SESSION(3), "@vm9:claude:x"], target: null, text: "", ts: NOW }, h), { id: "@demo-b:7", from: `@demo-b:${SESSION(1)}`, to: [`@demo-b:${SESSION(3)}`], target: null, text: "", ts: NOW - 3 * MIN, host: "demo-b" });
  for (const name of ["theme", "doctor", "graph", "actors", "agent-message"]) assert.equal(qualifyEvent(name, name === "agent-message" ? { from: "@x:y" } : {}, h), null, name);
  const agent = (id) => ({ id, app: "claude", title: "x", project: null, branch: null });
  const engine = qualifyEngine({ agents: [agent(SESSION(1)), agent("@vm9:claude:x"), { id: SESSION(2), title: "no app" }], messages: [{ id: 3, from: SESSION(1), target: "repo", text: "", at: NOW, to: [{ id: SESSION(3), seen: true }] }, { id: 4, from: SESSION(1) }] }, h);
  assert.deepEqual([engine.agents.map((a) => a.id), engine.messages.map((m) => m.id), engine.messages[0].to], [[`@demo-b:${SESSION(1)}`], ["@demo-b:3"], [{ id: `@demo-b:${SESSION(3)}`, seen: true }]], "only what fits the engine's schemas");
});

test("fields a host adds that the schemas don't declare are left out, at every level", () => {
  // Markup in a field the page never expects from a node, say a project's
  // app, which it writes into a class as it is.
  const evil = '"><img src=x onerror=alert(1)>';
  const snap = relayed();
  const atlas = snap.graph.nodes.find((n) => n.id === "project:~/projects/atlas-api");
  Object.assign(atlas, { app: evil, onclick: evil, git: { ...atlas.git, evil }, repo: atlas.repo && { ...atlas.repo, evil }, identities: atlas.identities.map((l) => ({ ...l, evil })) });
  const s1 = snap.graph.nodes.find((n) => n.id === SESSION(1));
  Object.assign(s1, { evil, ranAs: s1.ranAs && { ...s1.ranAs, evil } });
  const page = snap.graph.nodes.find((n) => n.kind === "page");
  page.tags = [...page.tags, { evil }];
  const wiki = snap.graph.nodes.find((n) => n.kind === "wiki");
  wiki.inbox = [...wiki.inbox, { id: "wiki:x", title: "x", app: null, evil }];
  Object.assign(snap.graph.identities[0], { evil, detail: { ...snap.graph.identities[0].detail, evil } });
  const ls = snap.live.sessions[0];
  Object.assign(ls, { evil, tokens: { ...ls.tokens, evil }, subagents: [...ls.subagents, { id: "a1", label: "x", startedAt: NOW, lastAt: NOW, tools: 0, done: false, active: true, evil }] });
  snap.live.collisions[0].evil = evil;
  snap.live.collab = [{ a: SESSION(1), b: SESSION(3), kind: "file", files: [], evil }];

  const merged = mergeGraph(state.graph, [{ info: live(), snapshot: snap }], { now: NOW, windows: config, aliases });
  valid("graph/v1", merged, "the merged map, strictly");
  assert.equal(merged.nodes.find((n) => n.id === "@demo-b:project:~/projects/atlas-api").app, undefined);
  assert.doesNotMatch(JSON.stringify(merged), /onerror|onclick/);
  assert.ok(merged.nodes.find((n) => n.id === `@demo-b:${page.id}`).tags.some((t) => JSON.stringify(t) === "{}"), "an object where anything goes keeps none of its own");
  const ml = mergeLive(state.liveSnapshot(), [{ info: live(), snapshot: snap }]);
  valid("live/snapshot/v1", ml, "the merged live state, strictly");
  assert.doesNotMatch(JSON.stringify(ml), /onerror/);
  assert.equal(ml.collab.at(-1).a, `@demo-b:${SESSION(1)}`);
  assert.doesNotMatch(JSON.stringify(mergeSummary(state.summary(), [{ info: live(), snapshot: snap }])), /onerror/);

  // An event, and the engine: the same.
  const h = live();
  const tool = qualifyEvent("activity", { sessionId: atlas.id, app: "claude", kind: "turn-start", ts: NOW, evil, projectId: "project:~/projects/elsewhere" }, h);
  assert.deepEqual(tool, { sessionId: `@demo-b:${atlas.id}`, app: "claude", kind: "turn-start", ts: NOW, host: "demo-b" }, "nothing its kind doesn't have, a project's id included");
  const engine = qualifyEngine({ agents: [{ id: SESSION(1), app: "claude", title: "x", project: null, branch: null, evil, claims: [{ pattern: "", note: null, until: NOW, evil }] }], messages: [{ id: 3, from: SESSION(1), target: "repo", text: "", at: NOW, to: [{ id: SESSION(3), seen: true, evil }], evil }] }, h);
  assert.doesNotMatch(JSON.stringify(engine), /onerror/);
  assert.deepEqual(engine.agents[0].claims, [{ pattern: "", note: null, until: NOW }]);
});

test("declared() keeps what a schema declares and nothing else, through each kind of schema", () => {
  const schema = z.looseObject({
    a: z.string(),
    pair: z.tuple([z.looseObject({ x: z.number() }), z.string()]),
    counts: z.record(z.string(), z.looseObject({ n: z.number() })),
    either: z.union([z.string(), z.looseObject({ k: z.literal(1) })]),
    list: z.array(z.unknown()),
    maybe: z.looseObject({ y: z.boolean() }).nullable().optional(),
    both: z.intersection(z.looseObject({ l: z.number() }), z.looseObject({ r: z.number() })),
  });
  const value = { a: "a", extra: 1, pair: [{ x: 1, no: 2 }, "s"], counts: { any: { n: 1, no: 2 } }, either: { k: 1, no: 2 }, list: ["s", 1, null, { no: 1 }, [{ no: 2 }]], maybe: null, both: { l: 1, r: 2, no: 3 } };
  assert.ok(schema.safeParse(value).success, "loose: zod lets the extra keys through");
  assert.deepEqual(declared(schema, value), { a: "a", pair: [{ x: 1 }, "s"], counts: { any: { n: 1 } }, either: { k: 1 }, list: ["s", 1, null, {}, [{}]], maybe: null, both: { l: 1, r: 2 } });
  assert.deepEqual(declared(schema, { a: "a", pair: [{ x: 1 }, "s"], counts: {}, either: "e", list: [], both: { l: 1, r: 2 } }).either, "e");
});

test("a host's nodes are checked and copied once each, and again when they change", () => {
  const snap = relayed();
  const once = cleanOnce();
  const first = qualifyGraph(snap.graph, live(), { now: NOW, windows: config, clean: once });
  assert.equal(first.dropped, 0, "the demo's real nodes all fit");
  const s1 = snap.graph.nodes.find((n) => n.id === SESSION(1));
  const changed = structuredClone(snap.graph);
  changed.nodes.find((n) => n.id === SESSION(1)).activity = "busy";
  const again = qualifyGraph(changed, live(), { now: NOW, windows: config, clean: once });
  assert.equal(again.dropped, 1, "the node that changed is checked again, and no longer fits");
  assert.ok(!again.nodes.some((n) => n.id === `@demo-b:${SESSION(1)}`));
  assert.ok(cleanNode(s1));
  const third = qualifyGraph(snap.graph, live(), { now: NOW, windows: config, clean: once });
  assert.equal(third.dropped, 0);
  // What it kept is shared between merges, so nothing a merge does to its
  // nodes reaches the next.
  const r1 = (part) => part.nodes.find((n) => n.id === `@demo-b:${SESSION(1)}`);
  r1(third).title = "changed on the map";
  const fourth = qualifyGraph(snap.graph, live(), { now: NOW, windows: config, clean: once });
  assert.notEqual(r1(fourth), r1(third));
  assert.equal(r1(fourth).title, s1.title);
  // A field added since is left out of the copy too.
  const added = structuredClone(snap.graph);
  added.nodes.find((n) => n.id === SESSION(1)).onclick = "x";
  assert.equal("onclick" in r1(qualifyGraph(added, live(), { now: NOW, windows: config, clean: once })), false);
});

test("the tray's summary counts a live host's alerts, each with its host and its title, and leaves a host that isn't live out", () => {
  const local = state.summary();
  assert.ok(local.needsYou.length > 0, "the demo has a question waiting");
  const snap = relayed();
  const merged = mergeSummary(local, [{ info: live(), snapshot: snap }]);
  const theirs = merged.needsYou.filter((n) => n.host === "demo-b");
  assert.equal(theirs.length, local.needsYou.length, "the same world's questions, on the other machine");
  for (const n of theirs) {
    assert.ok(n.id.startsWith("@demo-b:"), n.id);
    assert.equal(n.title, state.titleOf(n.id.slice("@demo-b:".length)), "titled from the host's map");
  }
  assert.ok(merged.needsYou.slice(0, local.needsYou.length).every((n, i) => n === local.needsYou[i]), "this machine's first, as they were");
  assert.ok(merged.working >= local.working && merged.tpm >= local.tpm);
  const stale = mergeSummary(local, [{ info: live({ state: "stale" }), snapshot: snap }]);
  assert.deepEqual([stale.needsYou.length, stale.stuck.length, stale.working, stale.collisions], [local.needsYou.length, local.stuck.length, local.working, local.collisions]);
  assert.equal(mergeSummary(local, []), local, "with no host, as it was");
});
