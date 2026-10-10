// What the actors layer draws (src/actors-model.js), worked out without a
// page: colors, presence, which targets get beams, and where actors float.
// The actors are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { hashOf, colorOf, formOf, presenceOf, presenceWord, actorNeeds, beamsOf, allBeams, placeActors, boundsOf, youHere, createTargetWatch, pointerTakes, createSaidEcho, gateOf, gateSpot, PRESENCE, LIVE_MS, BEAMS_MAX, BEAMS_PER_ACTOR, GAP, HOVER, STANDBY } from "../src/actors-model.js";
import { themes } from "../core/themes.js";
import { scopeGraph, DEFAULT_SCOPE } from "../src/scope.js";

const NOW = 1791234567000;
const MIN = 60e3;
const agent = (slug, extra = {}) => ({ id: `actor:${slug}`, kind: "agent", name: slug, presence: "working", targets: [], ...extra });
const target = (id, relation, ago, since = ago) => ({ id, relation, since: NOW - since, lastAt: NOW - ago });

test("colors: you in the theme's human color, every agent one of its palette by a stable hash of its id", () => {
  const all = themes();
  assert.equal(hashOf("actor:wren"), hashOf("actor:wren"));
  assert.notEqual(hashOf("actor:wren"), hashOf("actor:kestrel"));
  for (const t of Object.values(all)) {
    assert.equal(colorOf({ id: "actor:you", kind: "human" }, t), t.colors.human, `${t.id}: you`);
    const demo = ["actor:wren", "actor:kestrel", "actor:moth"].map((id) => colorOf({ id, kind: "agent" }, t));
    for (const c of demo) assert.ok(t.actors.palette.includes(c), `${t.id}: ${c} is in the palette`);
    // The demo's three agents tell apart in every palette, of four or six.
    assert.equal(new Set(demo).size, 3, `${t.id}: the demo's agents in three colors, not ${demo}`);
  }
  assert.equal(colorOf({ id: "actor:wren", kind: "agent" }, { colors: { actor: "#123456" }, actors: { palette: [] } }), "#123456", "no palette: the actor color");
});

test("forms: agents in the theme's form, you in its human form, an orb when it names none", () => {
  const all = themes();
  assert.equal(formOf({ kind: "agent" }, all.trench), "jelly");
  assert.equal(formOf({ kind: "human" }, all.trench), "orb");
  assert.equal(formOf({ kind: "agent" }, { actors: {} }), "orb");
});

test("presence: an open set, drawn as unknown when it's a value the console doesn't know", () => {
  assert.equal(presenceOf({ presence: "working" }), PRESENCE.working);
  assert.equal(presenceOf({ presence: "hibernating" }), PRESENCE.unknown);
  assert.equal(presenceOf(null), PRESENCE.unknown);
  for (const [k, look] of Object.entries(PRESENCE)) assert.ok(look.light > 0 && look.light <= 1 && look.scale > 0, k);
  assert.ok(PRESENCE.working.light > PRESENCE.idle.light && PRESENCE.idle.light > PRESENCE.away.light && PRESENCE.away.light > PRESENCE.offline.light);
});

test("presence in a word: unknown is not heard from until XenoNet has heard from it, and out of sight after", () => {
  assert.equal(presenceWord({ presence: "working" }), "working");
  assert.equal(presenceWord({ presence: "unknown", lastSeen: null }), "not heard from", "allowed, and silent so far");
  assert.equal(presenceWord({ presence: "unknown", lastSeen: NOW - MIN }), "out of sight", "Grok Bot with its app closed");
  assert.equal(presenceWord({ presence: "hibernating", lastSeen: NOW }), "out of sight", "a value the console doesn't know is unknown");
  assert.equal(presenceWord(null), "not heard from");
});

test("beams: to what it acted on lately, while it's there, one per node and as bright as its freshest tie", () => {
  const wren = agent("wren", {
    targets: [
      target("project:/p/atlas-api", "works-on", 5 * MIN),
      target("project:/p/atlas-api", "edited", 1 * MIN),
      target("claude:0190d3e0-0000-7000-8000-000000000001", "spawned", 10 * MIN),
      target("claude:0190d3e0-0000-7000-8000-000000000002", "spawned", 45 * MIN),
      target("project:/p/ledger", "works-on", 100 * MIN),
      target("project:/p/old", "works-on", 3 * 3600e3),
    ],
  });
  const beams = beamsOf(wren, NOW);
  assert.deepEqual(
    beams.map((b) => b.node),
    ["project:/p/atlas-api", "claude:0190d3e0-0000-7000-8000-000000000001", "project:/p/ledger"],
    "a spawn 45 minutes ago and work 3 hours ago are over",
  );
  assert.deepEqual([beams[0].relations.sort(), beams[0].relation], [["edited", "works-on"], "edited"], "two ties to one node make one beam, named for the latest");
  assert.ok(beams[0].strength > beams[1].strength && beams[1].strength > beams[2].strength, "fresher is brighter");
  assert.ok(beams.every((b) => b.strength > 0 && b.strength <= 1));
  // Only nodes on the map to land on.
  assert.deepEqual(beamsOf(wren, NOW, (id) => id.startsWith("claude:")).map((b) => b.node), ["claude:0190d3e0-0000-7000-8000-000000000001"]);
  // Not while it's gone or unheard of, and dimmer while idle or away.
  assert.deepEqual(beamsOf({ ...wren, presence: "offline" }, NOW), []);
  assert.deepEqual(beamsOf({ ...wren, presence: "unknown" }, NOW), []);
  assert.ok(beamsOf({ ...wren, presence: "idle" }, NOW)[0].strength < beams[0].strength);
  assert.ok(beamsOf({ ...wren, presence: "away" }, NOW)[0].strength < beamsOf({ ...wren, presence: "idle" }, NOW)[0].strength);
  // Each relation keeps its own window.
  assert.ok(LIVE_MS["works-on"] > LIVE_MS.spawned && LIVE_MS.watches >= LIVE_MS["works-on"]);
});

test("beams on a replay's clock: none before a tie began, and fresh between its first and last touch", () => {
  const moth = agent("moth", { targets: [{ id: "project:/p/geofence", relation: "watches", since: NOW - 3 * 3600e3, lastAt: NOW - MIN }] });
  assert.deepEqual(beamsOf(moth, NOW - 4 * 3600e3), [], "before it began");
  const mid = beamsOf(moth, NOW - 2 * 3600e3);
  assert.equal(mid.length, 1);
  assert.equal(mid[0].strength, PRESENCE.working.beams, "as fresh as can be");
});

test("beams: at most a few per actor and BEAMS_MAX in all, the brightest kept", () => {
  const busy = (slug) => agent(slug, { targets: Array.from({ length: 16 }, (_, i) => target(`claude:0190d3e0-0000-7000-8000-${String(i).padStart(12, "0")}`, "spawned", i * MIN)) });
  assert.equal(beamsOf(busy("a"), NOW).length, BEAMS_PER_ACTOR);
  const actors = ["a", "b", "c", "d", "e"].map(busy);
  const all = allBeams(actors, NOW);
  assert.equal(all.length, BEAMS_MAX);
  assert.ok(all.every((b, i) => !i || all[i - 1].strength >= b.strength));
  assert.ok(all.every((b) => b.key === `${b.actor}>${b.node}`));
});

test("placing: over what it acts on, on a ring when it acts on nothing, and never on top of another", () => {
  const bounds = { center: [0, 0, 0], radius: 300 };
  const spots = placeActors(
    [
      { id: "actor:wren", kind: "agent", presence: "working", targets: [[100, 20, 0, 1], [140, 40, 20, 1]] },
      { id: "actor:kestrel", kind: "agent", presence: "working", targets: [[120, 30, 10, 1]] },
      { id: "actor:moth", kind: "agent", presence: "idle", targets: [] },
      { id: "actor:you", kind: "human", presence: "working", targets: [] },
      { id: "actor:gone", kind: "agent", presence: "offline", targets: [] },
    ],
    bounds,
  );
  const [wx, wy, wz] = spots.get("actor:wren");
  assert.ok(wy > 30 + HOVER * 0.5, "above its targets");
  assert.ok(Math.hypot(wx - 120, wz - 10) < 60, "over them, not far off");
  const ring = (id) => Math.hypot(spots.get(id)[0], spots.get(id)[2]);
  assert.ok(ring("actor:moth") > 200, "an actor with nothing to do waits round the map");
  assert.ok(ring("actor:you") < ring("actor:moth"), "you wait nearer the middle");
  assert.ok(spots.get("actor:gone")[1] < spots.get("actor:moth")[1] + 20, "one that's gone hangs lower");
  const all = [...spots.values()];
  for (let i = 0; i < all.length; i++) for (let j = i + 1; j < all.length; j++) assert.ok(Math.hypot(all[i][0] - all[j][0], all[i][1] - all[j][1], all[i][2] - all[j][2]) >= GAP - 0.5, "apart");
  // The same actors land in the same places every time.
  assert.deepEqual(placeActors([{ id: "actor:moth", kind: "agent", presence: "idle", targets: [] }], bounds), placeActors([{ id: "actor:moth", kind: "agent", presence: "idle", targets: [] }], bounds));
});

test("you, outside the demo: drawn from your input in this window, the list as it was otherwise", () => {
  const placeholder = { id: "actor:you", kind: "human", name: "You", via: ["observed"], presence: "unknown" };
  const list = { schema: "actor/list/v1", at: NOW, actors: [placeholder, agent("wren")] };
  const you = (o) => youHere(list, o).actors[0].presence;
  assert.equal(you({ quietMs: 30e3 }), "working");
  assert.equal(you({ quietMs: 5 * MIN }), "idle");
  assert.equal(you({ quietMs: 11 * MIN }), "away");
  assert.equal(you({ quietMs: 1000, hiddenMs: 0 }), "away", "a hidden window is away");
  assert.equal(you({ quietMs: 1000, hiddenMs: 9 * MIN }), "away");
  assert.equal(you({ quietMs: 1000, hiddenMs: 10 * MIN }), "offline", "and offline ten minutes on, as XenoNet has you");
  assert.equal(you({ quietMs: 1000, hiddenMs: null }), "working", "shown again, what your input says");
  assert.equal(youHere(list, { quietMs: 0 }).actors[1], list.actors[1], "the others untouched");
  // The demo's made-up you, and one the server has heard from, as they are.
  const demo = { ...list, actors: [{ ...placeholder, via: ["demo", "observed"] }] };
  assert.equal(youHere(demo, { quietMs: 0 }), demo);
  const heard = { ...list, actors: [{ ...placeholder, presence: "idle" }] };
  assert.equal(youHere(heard, { quietMs: 0 }), heard);
  assert.equal(youHere({ actors: [] }, { quietMs: 0 }).actors.length, 0);
});

test("you, with your record: where you've looked and what you've done, so your beams show; who you are stays the placeholder's", () => {
  const placeholder = { id: "actor:you", kind: "human", name: "You", via: ["observed"], presence: "unknown", status: null, targets: [], recent: [], identity: null };
  const list = { schema: "actor/list/v1", at: NOW, actors: [placeholder, agent("wren")] };
  const looked = { id: "project:/users/nova/projects/atlas-api", relation: "watches", since: NOW - MIN, lastAt: NOW - MIN };
  const record = { ...placeholder, name: "Someone", identity: "github:someone", presence: "idle", status: "Looking at atlas-api", lastSeen: NOW - 5000, targets: [looked], recent: [{ id: "you:3", kind: "status", detail: "Looking at atlas-api", at: NOW - MIN }] };
  const you = youHere(list, { quietMs: 1000, record }).actors[0];
  assert.deepEqual([you.status, you.targets, you.recent.length, you.lastSeen], ["Looking at atlas-api", [looked], 1, NOW - 5000]);
  assert.deepEqual([you.name, you.identity, you.presence], ["You", null, "working"], "never named, and present as this window sees you");
  assert.deepEqual(beamsOf(you, NOW).map((b) => b.node), [looked.id], "a beam to where you looked");
  // A record that isn't yours, or isn't shaped like one, changes nothing.
  assert.deepEqual(youHere(list, { quietMs: 1000, record: { ...record, id: "actor:wren" } }).actors[0].targets, []);
  assert.deepEqual(youHere(list, { quietMs: 1000, record: { ...record, targets: "atlas" } }).actors[0].targets, []);
  // The demo's made-up you is drawn from the list, as before.
  const demo = { ...list, actors: [{ ...placeholder, via: ["demo", "observed"] }] };
  assert.equal(youHere(demo, { quietMs: 0, record }), demo);
});

test("actors waiting on you reach the strip, the latest first, in words of their kind", () => {
  const list = {
    actors: [
      agent("wren", { name: "Wren", needsYou: { kind: "question", text: "Which branch should the release go on?", since: NOW - 5 * MIN } }),
      agent("moth", { name: "Moth", needsYou: { kind: "permission", text: "May I message the team?", since: NOW - MIN } }),
      agent("kite", { name: "Kite", needsYou: { kind: "nudge", text: "", since: NOW - 2 * MIN } }),
      agent("kestrel", { name: "Kestrel", needsYou: null }),
      { id: "actor:you", kind: "human", name: "You", needsYou: { kind: "question", text: "never you", since: NOW } },
    ],
  };
  assert.deepEqual(actorNeeds(list), [
    { kind: "actor-need", id: "actor:moth", name: "Moth", sub: "needs permission", text: "May I message the team?", since: NOW - MIN },
    { kind: "actor-need", id: "actor:kite", name: "Kite", sub: "waits for you", text: "Waiting for you", since: NOW - 2 * MIN },
    { kind: "actor-need", id: "actor:wren", name: "Wren", sub: "asks", text: "Which branch should the release go on?", since: NOW - 5 * MIN },
  ]);
  assert.deepEqual(actorNeeds(null), []);
});

test("a place an actor acts on is drawn once, and one the map can't draw doesn't redraw it with every list", () => {
  // The home folder and a Codex project, each with sessions older than the
  // week the map shows, and a project with an old session of its own.
  const DAY = 24 * 60 * MIN;
  const old = (id, days) => ({ id, kind: "session", app: "claude", updatedAt: NOW - days * DAY });
  const graph = {
    nodes: [
      { id: "core", kind: "core" },
      { id: "zone:home", kind: "zone" },
      { id: "native:codex:p1", kind: "native" },
      { id: "project:/p/atlas", kind: "project", lastActivity: NOW - 20 * DAY },
      old("claude:1", 9),
      old("claude:2", 10),
      old("claude:3", 12),
    ],
    links: [
      { source: "core", target: "zone:home", kind: "zone" },
      { source: "core", target: "native:codex:p1", kind: "native" },
      { source: "core", target: "project:/p/atlas", kind: "contains" },
      { source: "zone:home", target: "claude:1", kind: "session" },
      { source: "native:codex:p1", target: "claude:2", kind: "session" },
      { source: "project:/p/atlas", target: "claude:3", kind: "session" },
    ],
    relations: [],
  };
  // main.js's loop: the map drawn with the actors' targets kept, and drawn
  // again when the watch wants it.
  const known = new Set(graph.nodes.map((n) => n.id));
  let drawn = new Set();
  let redraws = 0;
  const redraw = (targets) => {
    redraws++;
    drawn = new Set(scopeGraph(graph, DEFAULT_SCOPE, { keep: new Set(targets), now: NOW }).nodes.map((n) => n.id));
  };
  const map = { known: (id) => known.has(id), drawn: (id) => drawn.has(id) };
  const watch = createTargetWatch();
  const listCame = (targets) => {
    if (!watch.wants(targets, map)) return;
    redraw(targets);
    watch.drew(targets, map);
  };
  redraw([]);
  redraws = 0;
  const places = ["zone:home", "native:codex:p1"];
  for (let i = 0; i < 10; i++) listCame(places);
  assert.equal(redraws, 1, "tried once; neither can be drawn without something in it");
  assert.ok(!drawn.has("zone:home") && !drawn.has("native:codex:p1"));
  // A session it starts working on can be drawn, so it is, once.
  for (let i = 0; i < 10; i++) listCame([...places, "claude:3"]);
  assert.equal(redraws, 2);
  assert.ok(drawn.has("claude:3") && drawn.has("project:/p/atlas"), "with the project it's in");
  // A place that isn't on the map at all waits for the map, as before.
  listCame([...places, "claude:3", "claude:99"]);
  assert.equal(redraws, 2);
});

test("actors float around this machine's map, not over another machine's constellation", () => {
  // This machine: its core and a ring of projects 216 out. Another machine:
  // its host node on an orbit at 398, its own nodes around it.
  const local = [{ id: "core", x: 0, y: 0, z: 0 }, ...[0, 1, 2, 3].map((i) => ({ id: `project:/p/${i}`, x: 216 * Math.cos((i * Math.PI) / 2), y: 0, z: 216 * Math.sin((i * Math.PI) / 2) }))];
  const host = { id: "@vm1:core", kind: "host", host: "vm1", x: 398, y: 0, z: 0 };
  const remote = [host, ...[0, 1, 2].map((i) => ({ id: `@vm1:project:~/p/${i}`, host: "vm1", x: 398 + 100 * Math.cos(i * 2), y: 0, z: 100 * Math.sin(i * 2) }))];
  const bounds = boundsOf([...local, ...remote]);
  assert.deepEqual(bounds.center.map((v) => Math.round(v) || 0), [0, 0, 0], "the middle of this machine's map");
  assert.equal(Math.round(bounds.radius), 216, "and its reach");
  // Waiting with nothing to beam into, an agent stays clear of the host.
  for (const slug of ["wren", "kestrel", "moth", "heron"]) {
    const [x, , z] = placeActors([agent(slug, { presence: "idle" })], bounds).get(`actor:${slug}`);
    assert.ok(Math.hypot(x, z) <= 216, `${slug} is around this map`);
    assert.ok(Math.hypot(x - host.x, z - host.z) > 134, `${slug} isn't in vm1's constellation`);
  }
  // Where a gathered node lives counts, and with nothing of this machine's
  // drawn there's nothing to go by.
  assert.deepEqual(boundsOf([{ id: "a", x: 50, y: 0, z: 0 }], () => ({ x: 10, y: 0, z: 0 })).center, [10, 0, 0]);
  assert.equal(boundsOf(remote), null);
});

test("the pointer takes whatever is drawn in front: an actor before a node's body, a body before an actor, and an actor over glow", () => {
  assert.equal(pointerTakes(80, 120), "actor", "an actor in front of a node's body");
  assert.equal(pointerTakes(120, 80), "node", "a node's body in front of an actor");
  assert.equal(pointerTakes(120, Infinity), "actor", "only the node's glow under the pointer");
  assert.equal(pointerTakes(Infinity, 80), "node", "no actor there");
  assert.equal(pointerTakes(Infinity, Infinity), null);
});

test("an actor's message is drawn and listed once: the said event XenoNet writes of it just after is its echo", () => {
  const clock = { t: NOW };
  const echoes = createSaidEcho({ now: () => clock.t });
  const said = (actor) => ({ actor, kind: "said", target: "claude:s1", at: clock.t });
  assert.equal(echoes.echo(said("actor:moth")), false, "a said event with no message before it is drawn as itself");
  echoes.spoke("actor:moth");
  clock.t += 50;
  assert.equal(echoes.echo(said("actor:moth")), true, "the message's own said event");
  assert.equal(echoes.echo({ ...said("actor:moth"), kind: "edited" }), false, "anything else it does is news");
  assert.equal(echoes.echo(said("actor:wren")), false, "another actor's is too");
  clock.t += 5000;
  assert.equal(echoes.echo(said("actor:moth")), false, "a while later, a said event is news again");
  for (let i = 0; i < 100; i++) echoes.spoke(`actor:a${i}`);
  echoes.spoke("actor:moth");
  assert.equal(echoes.echo(said("actor:moth")), true, "it keeps the latest, however many have spoken");
});

test("a tunnel's gate: open with an adapter connected, a sliver without, none without an active connection through a tunnel", () => {
  const remote = (over = {}) => agent("dots", { via: ["remote"], remote: { active: true, connections: 1, bridge: "connected", accountLink: "unverified", permissions: ["read"], transports: ["openai-secure-mcp-tunnel"], ...over } });
  assert.equal(gateOf(remote()), 1);
  assert.equal(gateOf(remote({ bridge: "not_connected" })), STANDBY);
  assert.ok(STANDBY > 0 && STANDBY < 0.5, "shut, but there");
  assert.equal(gateOf(remote({ active: false })), null, "revoked or expired: no gate");
  assert.equal(gateOf(remote({ transports: ["local-stdio"] })), null, "a local client is no tunnel");
  assert.equal(gateOf(remote({ transports: undefined })), null, "a snapshot from before transports");
  assert.equal(gateOf(agent("wren")), null);
});

test("a tunnel's gate sits between its actor and what it reads, a step along the way and never past part of it", () => {
  const near = gateSpot([0, 100, 0], [[0, 0, 0, 1]], [500, 0, 0], { gap: 30 });
  assert.deepEqual(near.at, [0, 70, 0], "toward its targets, not the map's middle");
  assert.deepEqual(near.dir, [0, -1, 0]);
  const close = gateSpot([0, 20, 0], [[0, 0, 0, 1]], [0, 0, 0], { gap: 30, share: 0.45 });
  assert.equal(close.at[1], 11, "at most share of the way");
  const weighted = gateSpot([0, 100, 0], [[100, 0, 0, 3], [-100, 0, 0, 1]], [0, 0, 0]);
  assert.ok(weighted.at[0] > 0, "toward the heavier target");
  const none = gateSpot([0, 100, 0], [], [0, 0, 0]);
  assert.deepEqual(none.dir, [0, -1, 0], "with no beams, toward the map's middle");
  const over = gateSpot([5, 5, 5], [], [5, 5, 5]);
  assert.deepEqual(over.dir, [0, -1, 0], "right over it: straight down");
  for (const v of [...near.at, ...weighted.at, ...over.at]) assert.ok(Number.isFinite(v));
});
