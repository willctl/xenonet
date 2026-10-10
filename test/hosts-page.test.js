// Other machines on the page (src/hosts.js): each host's constellation and
// the forces that keep it beside this machine's map, the words for how a
// host stands, the machines' chips and the alert strip's marks. Made-up
// maps; no browser, no three.js.
import { test } from "node:test";
import assert from "node:assert/strict";
import { depthsOf, hostLayout, installHostLayout, standing, labelOf, hostVisual, whereText, whereHtml, sameHost, hostRows, tipText, hostTag, alertFrom, alertHostHtml, alertText, collisionFrom, hostOfId, createHostFilters, capHosts, decorate, HERE, STALE_LIGHT, HOST_SCALE, HOST_CAP } from "../src/hosts.js";

// This machine: core → a group → a project → two sessions, one spawned by
// the other. Each host: its host node → a project → a session.
function world(hosts = ["vm1"], { pinned = {} } = {}) {
  const nodes = [
    { id: "core", kind: "core", name: "~/Projects" },
    { id: "project:/p/labs", kind: "project", group: true },
    { id: "project:/p/labs/geo", kind: "project" },
    { id: "claude:1", kind: "session" },
    { id: "claude:2", kind: "session" },
  ];
  const links = [
    { source: "core", target: "project:/p/labs", kind: "contains" },
    { source: "project:/p/labs", target: "project:/p/labs/geo", kind: "nested" },
    { source: "project:/p/labs/geo", target: "claude:1", kind: "session" },
    { source: "claude:1", target: "claude:2", kind: "lineage" },
  ];
  for (const h of hosts) {
    const core = `@${h}:core`;
    nodes.push({ id: core, kind: "host", host: h, name: h, state: "live", ...(pinned[h] ? { pinned: true, fx: pinned[h][0], fy: pinned[h][1], fz: pinned[h][2] } : {}) });
    nodes.push({ id: `@${h}:project:~/projects/atlas`, kind: "project", host: h });
    nodes.push({ id: `@${h}:claude:9`, kind: "session", host: h });
    links.push({ source: "core", target: core, kind: "host" });
    links.push({ source: core, target: `@${h}:project:~/projects/atlas`, kind: "contains" });
    links.push({ source: `@${h}:project:~/projects/atlas`, target: `@${h}:claude:9`, kind: "session" });
  }
  return { nodes, links, byId: new Map(nodes.map((n) => [n.id, n])) };
}

test("depth is the longest way down from a root, over the nodes asked for, as the force graph counts it", () => {
  const { nodes, links } = world();
  const local = depthsOf(nodes, links, (n) => !n.host);
  assert.deepEqual(Object.fromEntries(local), { core: 0, "project:/p/labs": 1, "project:/p/labs/geo": 2, "claude:1": 3, "claude:2": 4 });
  const remote = depthsOf(nodes, links, (n) => !!n.host);
  assert.deepEqual(Object.fromEntries(remote), { "@vm1:core": 0, "@vm1:project:~/projects/atlas": 1, "@vm1:claude:9": 2 });
  // A link object (as the force graph leaves them) counts as its ids, and a
  // loop puts its nodes one ring out rather than nowhere.
  const loop = depthsOf([{ id: "a" }, { id: "b" }, { id: "c" }], [{ source: { id: "a" }, target: { id: "b" }, kind: "session" }, { source: "b", target: "c", kind: "session" }, { source: "c", target: "b", kind: "session" }], () => true);
  assert.deepEqual(Object.fromEntries(loop), { a: 0, b: 1, c: 1 });
});

test("with no host on the map, the layout does nothing", () => {
  const layout = hostLayout();
  const { nodes, links } = world([]);
  assert.equal(layout.prepare(nodes, links), false);
  assert.deepEqual(layout.systems, []);
  assert.ok(nodes.every((n) => n.x === undefined && n.fx === undefined), "nothing placed or held");
});

test("each host is held on an orbit clear of this machine's map, its nodes start on their own rings around it", () => {
  const layout = hostLayout({ random: () => 0.25 });
  const { nodes, links, byId } = world(["vm1"]);
  assert.equal(layout.prepare(nodes, links), true);
  const [sys] = layout.systems;
  const step = 72 * HOST_SCALE;
  // This machine's map reaches four rings out (72 apart); the host's own
  // system takes at least three of its smaller rings, and there's room
  // between.
  assert.ok(sys.anchor.x > 4 * 72 + 3 * step, `orbit ${sys.anchor.x}`);
  assert.deepEqual([sys.anchor.y, sys.anchor.z], [0, 0], "one host: beside the map, on the right");
  const host = byId.get("@vm1:core");
  assert.deepEqual([host.fx, host.fy, host.fz], [sys.anchor.x, 0, 0], "held there");
  assert.deepEqual([host.x, host.y, host.z], [sys.anchor.x, 0, 0]);
  for (const [id, ring] of [["@vm1:project:~/projects/atlas", 1], ["@vm1:claude:9", 2]]) {
    const n = byId.get(id);
    const d = Math.hypot(n.x - host.x, n.y - host.y, n.z - host.z);
    assert.ok(Math.abs(d - ring * step) < 1e-6, `${id} starts on ring ${ring}: ${d}`);
  }
  assert.equal(byId.get("claude:1").x, undefined, "this machine's nodes are the force graph's to place");
});

test("two hosts sit either side, more around a circle wide enough that none overlap", () => {
  const two = hostLayout();
  const a = world(["vm1", "studio"]);
  two.prepare(a.nodes, a.links);
  const [s1, s2] = two.systems;
  assert.deepEqual([s1.host, s2.host], ["studio", "vm1"], "in order of their ids, so they keep their places");
  assert.equal(s1.anchor.x, -s2.anchor.x);
  const five = hostLayout();
  const b = world(["h1", "h2", "h3", "h4", "h5"]);
  five.prepare(b.nodes, b.links);
  const s = five.systems;
  for (let i = 0; i < s.length; i++) {
    const p = s[i].anchor, q = s[(i + 1) % s.length].anchor;
    assert.ok(Math.hypot(p.x - q.x, p.z - q.z) >= s[i].radius + s[(i + 1) % s.length].radius, "neighbours clear of each other");
  }
});

test("a host node you pinned stays where you put it, and its constellation goes with it", () => {
  const layout = hostLayout();
  const { nodes, links, byId } = world(["vm1"], { pinned: { vm1: [10, 400, -30] } });
  byId.get("@vm1:core").x = 10;
  byId.get("@vm1:core").y = 400;
  byId.get("@vm1:core").z = -30;
  layout.prepare(nodes, links);
  const host = byId.get("@vm1:core");
  assert.deepEqual([host.fx, host.fy, host.fz], [10, 400, -30]);
  const p = byId.get("@vm1:project:~/projects/atlas");
  assert.ok(Math.abs(Math.hypot(p.x - 10, p.y - 400, p.z + 30) - 72 * HOST_SCALE) < 1e-6, "it starts around the pin");
});

test("the rings' force draws a host's nodes to their ring around its host node, in the radial layout only", () => {
  let radial = true;
  const layout = hostLayout({ radial: () => radial });
  const { nodes, links, byId } = world(["vm1"]);
  layout.prepare(nodes, links);
  const host = byId.get("@vm1:core");
  const p = byId.get("@vm1:project:~/projects/atlas");
  Object.assign(p, { x: host.x + 200, y: 0, z: 0, vx: 0, vy: 0, vz: 0 });
  layout.force(0.5);
  assert.ok(p.vx < 0 && Math.abs(p.vy) < 1e-3 && Math.abs(p.vz) < 1e-3, "pulled in toward its ring");
  Object.assign(p, { x: host.x + 5, vx: 0 });
  layout.force(0.5);
  assert.ok(p.vx > 0, "pushed out to it");
  radial = false;
  Object.assign(p, { x: host.x + 200, vx: 0 });
  layout.force(0.5);
  assert.equal(p.vx, 0, "the free layout leaves them to the other forces");
});

test("the centering moves only this machine's nodes, so hosts stay on their orbits", () => {
  const layout = hostLayout();
  const center = layout.center();
  const nodes = [
    { id: "core", x: 10, y: 0, z: 0 },
    { id: "claude:1", x: 30, y: 4, z: -2 },
    { id: "@vm1:core", host: "vm1", x: 500, y: 0, z: 0 },
  ];
  center.initialize(nodes);
  center();
  assert.deepEqual(nodes.map((n) => [n.x, n.y, n.z]), [[-10, -2, 1], [10, 2, -1], [500, 0, 0]]);
});

test("the layout goes into the force graph once: remote nodes out of the radial layout, the host link pulling on neither end", () => {
  const forces = {};
  let strength = (l) => (l.kind === "contains" ? 0.5 : 0.25);
  const link = { strength: (fn) => (fn ? ((strength = fn), link) : strength) };
  const graph = {
    filter: null,
    dagNodeFilter(fn) {
      this.filter = fn;
      return this;
    },
    d3Force(name, f) {
      if (f === undefined) return name === "link" ? link : forces[name];
      forces[name] = f;
      return this;
    },
  };
  const layout = hostLayout();
  installHostLayout(graph, layout);
  assert.equal(graph.filter({ id: "core" }), true);
  assert.equal(graph.filter({ id: "@vm1:core", host: "vm1" }), false);
  assert.equal(forces.hosts, layout.force);
  assert.equal(typeof forces.center.initialize, "function");
  assert.equal(strength({ kind: "host" }, 0, []), 0);
  assert.equal(strength({ kind: "contains" }, 0, []), 0.5, "the rest as before");
});

test("how a host stands, in words, on its label, and what makes the map draw it again", () => {
  const now = Date.parse("2026-10-05T15:00:00");
  const at = Date.parse("2026-10-05T14:02:00");
  const time = new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  assert.equal(standing({ state: "live" }, now), "live");
  assert.equal(standing({ state: "offline", lastHeard: at }, now), `last seen ${time}`);
  assert.equal(standing({ state: "stale", lastHeard: at }, now), `no word since ${time}`);
  assert.equal(standing({ state: "connecting" }, now), "connecting");
  assert.equal(standing({ state: "error", error: "unknown host key" }, now), "needs you");
  assert.equal(labelOf({ name: "vm1", state: "live" }, now), "VM1\nLIVE");
  assert.equal(labelOf({ name: "vm1", state: "offline", lastHeard: at }, now), `VM1\nSEEN ${time.toUpperCase()}`);
  // A live host's last word moves with every frame; its look doesn't.
  assert.equal(hostVisual({ kind: "host", state: "live", lastHeard: 1, stale: false }), hostVisual({ kind: "host", state: "live", lastHeard: 2, stale: false }));
  assert.notEqual(hostVisual({ kind: "host", state: "live", stale: false }), hostVisual({ kind: "host", state: "offline", lastHeard: 1, stale: true }));
  assert.equal(hostVisual({ kind: "session", stale: true }), hostVisual({ kind: "session" }), "its other nodes dim frame by frame, with nothing rebuilt");
  assert.ok(STALE_LIGHT > 0 && STALE_LIGHT < 1);
});

test("a remote node says where it lives, read-only, and every word from a host is escaped", () => {
  const index = new Map([["@vm1:core", { id: "@vm1:core", kind: "host", host: "vm1", state: "live" }]]);
  const session = { id: "@vm1:claude:9", kind: "session", host: "vm1" };
  assert.equal(whereText(session, index), "on vm1 · live");
  assert.match(whereHtml(session, index), /on vm1 · live.*read-only/);
  assert.equal(whereText({ id: "claude:1", kind: "session" }, index), "", "this machine's say nothing");
  assert.equal(whereText(index.get("@vm1:core"), index), "", "nor does the host node itself");
  const hostile = { id: "@x:claude:1", kind: "session", host: '<img src=x onerror="alert(1)">', stale: true };
  for (const html of [whereHtml(hostile, new Map()), hostTag(hostile), alertHostHtml({ host: hostile.host }), alertHostHtml({ host: hostile.host, stale: true, asOf: 1 })]) {
    assert.doesNotMatch(html, /<img/, html);
    assert.match(html, /&lt;img/);
  }
  const rows = hostRows({ kind: "host", state: "error", error: "<b>refused</b>", version: "0.4.0", offsetMs: 180e3, sessionCount: 3 }, { row: (k, v) => (v == null || v === "" ? "" : `${k}=${v};`), mono: (v) => (v == null ? null : v) });
  assert.match(rows, /problem=<span class="pressure">&lt;b&gt;refused&lt;\/b&gt;<\/span>/);
  assert.match(rows, /clock=\+3 min against this clock/);
  assert.equal(tipText(session, index), "on vm1");
  assert.equal(sameHost(session, "project:~/projects/labs"), "@vm1:project:~/projects/labs");
  assert.equal(sameHost({ id: "project:/p" }, "project:/p/labs"), "project:/p/labs");
});

test("the alert strip: where a remote alert is, as of when for a host that isn't live, and where to read its question", () => {
  assert.deepEqual(alertFrom({ id: "claude:1" }), {});
  assert.deepEqual(alertFrom({ id: "@vm1:claude:9", host: "vm1", stale: true, needsYou: { asOf: 5 } }), { host: "vm1", stale: true, asOf: 5 });
  assert.match(alertHostHtml({ host: "vm1" }), /on vm1/);
  assert.match(alertHostHtml({ host: "vm1", stale: true, asOf: Date.now() }), /as of .*, vm1 not live/);
  assert.equal(alertText({ host: "vm1", text: "" }), "The question is on vm1: preview the transcript to read it.");
  assert.equal(alertText({ text: "Ship it?" }), "Ship it?");
  assert.deepEqual(collisionFrom({ a: "@vm1:claude:1", b: "@vm1:codex:2" }), { host: "vm1", stale: false });
  assert.deepEqual(collisionFrom({ a: "claude:1", b: "codex:2" }), {});
  assert.equal(hostOfId("@vm1:project:~/x"), "vm1");
  assert.equal(hostOfId("claude:1"), null);
});

test("the machines' chips: one per machine with its sessions, and a filter that shows only some", () => {
  const changes = [];
  const f = createHostFilters({ onFilter: () => changes.push(f.selected) });
  const { nodes } = world(["vm1", "studio"]);
  const graph = { nodes, hosts: [{ id: "studio", state: "offline" }, { id: "vm1", state: "live" }], self: "mac" };
  f.setData(graph);
  assert.deepEqual(f.chips().map((c) => [c.key, c.label, c.state, c.sessions]), [[HERE, "mac", "live", 2], ["studio", "studio", "offline", 1], ["vm1", "vm1", "live", 1]]);
  // Nothing picked: host nodes join what the other filters let through.
  const all = f.restrict(new Set(["core", "claude:1", "@vm1:claude:9"]));
  assert.deepEqual([...all].sort(), ["@studio:core", "@vm1:claude:9", "@vm1:core", "claude:1", "core"]);
  // vm1 alone: this machine's core stays as the map's center.
  f.setSelected(["vm1"]);
  assert.deepEqual([...f.restrict(new Set(["core", "claude:1", "@vm1:claude:9", "@studio:claude:9"]))].sort(), ["@vm1:claude:9", "@vm1:core", "core"]);
  // This machine, by its own name or "here".
  f.setSelected(["mac"]);
  assert.deepEqual(f.selected, [HERE]);
  f.setSelected(["here", "nope"]);
  assert.deepEqual(f.selected, [HERE], "a host that isn't there is no choice");
  assert.deepEqual([...f.restrict(new Set(["core", "claude:1", "@vm1:claude:9"]))].sort(), ["claude:1", "core"]);
  f.setSelected([]);
  assert.equal(changes.length, 4);
  // A host going away leaves the filter.
  f.setSelected(["studio"]);
  f.setData({ ...graph, hosts: [{ id: "vm1", state: "live" }] });
  assert.deepEqual(f.selected, []);
  f.setHosts([{ id: "vm1", state: "stale" }]);
  assert.equal(f.chips().find((c) => c.key === "vm1").state, "stale");
  // With no host on the map, nothing changes.
  const none = createHostFilters();
  none.setData({ nodes: world([]).nodes });
  const v = new Set(["core"]);
  assert.equal(none.restrict(v), v);
  assert.deepEqual(none.chips(), []);
});

test("a machine's chip says why it needs you, from the map or from a hosts event", () => {
  // Just enough of a page for the chips to render into.
  const box = { hidden: true, innerHTML: "" };
  globalThis.document = { getElementById: (id) => (id === "host-filters" ? box : null) };
  try {
    const f = createHostFilters();
    f.setData({ nodes: world(["vm1"]).nodes, hosts: [{ id: "vm1", state: "error", error: "vm1 refused the key" }], self: "mac" });
    const title = () => /data-host-key="vm1" title="([^"]*)"/.exec(box.innerHTML)?.[1];
    assert.match(title(), /^vm1 · needs you · vm1 refused the key\n/);
    // The hosts event carries the hub's view, whose error is an object.
    f.setHosts([{ id: "vm1", state: "error", error: { code: "refused", message: "vm1 refused the key", fix: "Install the key in vm1's ~/.ssh/authorized_keys" } }]);
    assert.match(title(), /^vm1 · needs you · vm1 refused the key\n/, "in words, not [object Object]");
    assert.equal(f.hosts[0].error, "vm1 refused the key");
    f.setHosts([{ id: "vm1", state: "live", error: null }]);
    assert.match(title(), /^vm1 · live\n/);
  } finally {
    delete globalThis.document;
  }
});

test("this machine's core takes its own name beside its folder once hosts are on the map", () => {
  const graph = { nodes: [{ id: "core", kind: "core", name: "~/Projects" }], hosts: [{ id: "vm1" }], self: "studio" };
  decorate(graph);
  assert.equal(graph.nodes[0].selfName, "studio");
  assert.equal(hostVisual(graph.nodes[0]), "studio");
  decorate({ ...graph, hosts: [] });
  assert.equal(graph.nodes[0].selfName, null);
});

// A host of some size for the cap: its host node, places (the first one
// worked in lately), each with sessions (the first of each active), and a
// wiki with a page. Times go down the list, so earlier is more recent.
function bigHost(h, { places = 4, sessions = 3 } = {}) {
  const core = `@${h}:core`;
  const nodes = [{ id: core, kind: "host", host: h, name: h, state: "live" }];
  const links = [{ source: "core", target: core, kind: "host" }];
  let t = 1e6;
  for (let p = 0; p < places; p++) {
    const place = `@${h}:project:~/projects/p${p}`;
    nodes.push({ id: place, kind: "project", host: h, active: p === 0, lastActivity: (t -= 10) });
    links.push({ source: core, target: place, kind: "contains" });
    for (let k = 0; k < sessions; k++) {
      const id = `@${h}:claude:${p}-${k}`;
      nodes.push({ id, kind: "session", host: h, activity: k === 0 ? "active" : "idle", updatedAt: (t -= 1) });
      links.push({ source: place, target: id, kind: "session" });
    }
  }
  nodes.push({ id: `@${h}:resource:agent-wiki`, kind: "wiki", host: h }, { id: `@${h}:wiki:refunds`, kind: "page", host: h });
  links.push({ source: core, target: `@${h}:resource:agent-wiki`, kind: "wiki" }, { source: `@${h}:resource:agent-wiki`, target: `@${h}:wiki:refunds`, kind: "page" });
  return { nodes, links };
}
const parentsOf = (links) => new Map(links.filter((l) => l.kind !== "host").map((l) => [l.target, l.source]));

test("the cap: the hosts you're not looking at share it in turn, places first, and what waits on you is drawn whatever it", () => {
  assert.equal(HOST_CAP, 24);
  const a = bigHost("vm1"), b = bigHost("studio", { places: 2 });
  const nodes = [...a.nodes, ...b.nodes];
  const parentOf = parentsOf([...a.links, ...b.links]);
  const drawnOf = (r) => nodes.filter((n) => n.kind !== "host" && !r.hidden.has(n.id)).map((n) => n.id);
  // Six between two hosts: three each, the places worked in lately first,
  // then the most recent places.
  let r = capHosts(nodes, { parentOf, cap: 6 });
  assert.deepEqual(drawnOf(r), ["@vm1:project:~/projects/p0", "@vm1:project:~/projects/p1", "@vm1:project:~/projects/p2", "@studio:project:~/projects/p0", "@studio:claude:0-0", "@studio:project:~/projects/p1"]);
  assert.deepEqual(Object.fromEntries(r.hosts), { studio: { drawn: 3, of: 10 }, vm1: { drawn: 3, of: 18 } });
  assert.ok(!r.hidden.has("@vm1:core") && !r.hidden.has("@studio:core"), "host nodes aren't the cap's");
  // Room for more: after the places, active sessions, then the rest, and
  // a host with nothing left leaves its turn to the other.
  r = capHosts(nodes, { parentOf, cap: 20 });
  assert.deepEqual(Object.fromEntries(r.hosts), { studio: { drawn: 10, of: 10 }, vm1: { drawn: 10, of: 18 } });
  assert.ok(["@vm1:claude:0-0", "@vm1:claude:1-0", "@vm1:claude:2-0", "@vm1:claude:3-0"].every((id) => !r.hidden.has(id)), "vm1's active sessions");
  assert.ok(!r.hidden.has("@vm1:claude:0-1") && !r.hidden.has("@vm1:claude:0-2"), "then its most recent sessions");
  assert.ok(r.hidden.has("@vm1:claude:1-1") && r.hidden.has("@vm1:resource:agent-wiki"));
  // A session that waits on you is drawn even with no room, and so is the
  // place it's in, so it isn't cut off from its host.
  r = capHosts(nodes, { parentOf, cap: 0, attention: new Set(["@vm1:claude:3-2"]) });
  assert.deepEqual(drawnOf(r), ["@vm1:project:~/projects/p3", "@vm1:claude:3-2"]);
  // What was drawn stays while nothing outranks it, though other sessions
  // have since been more recent: no flicker as sessions take turns.
  const first = capHosts(nodes, { parentOf, cap: 12 });
  const later = nodes.map((n) => (n.id === "@vm1:claude:3-0" ? { ...n, updatedAt: 2e6 } : n));
  assert.notDeepEqual(drawnOf(capHosts(later, { parentOf, cap: 12 })), drawnOf(first), "on its own, the most recent would take a place");
  const kept = new Set(nodes.filter((n) => n.kind !== "host" && !first.hidden.has(n.id)).map((n) => n.id));
  assert.deepEqual(drawnOf(capHosts(later, { parentOf, cap: 12, keep: kept })), drawnOf(first));
  // Nothing of this machine's is the cap's, and with no cap it's all drawn.
  r = capHosts([...nodes, { id: "claude:1", kind: "session" }], { parentOf, cap: 2 });
  assert.ok(!r.hidden.has("claude:1") && !r.hosts.has(undefined));
  assert.equal(capHosts(nodes, { parentOf, cap: Infinity }).hidden.size, 0);
});

test("the machines' filter caps the hosts you're not looking at, once it's told what you're looking at, and draws a host in full while you are", () => {
  const a = bigHost("vm1"), b = bigHost("studio");
  const graph = { nodes: [{ id: "core", kind: "core" }, { id: "claude:1", kind: "session" }, ...a.nodes, ...b.nodes], links: [...a.links, ...b.links], hosts: [{ id: "studio", state: "live" }, { id: "vm1", state: "offline" }], self: "mac" };
  let asked = 0;
  const f = createHostFilters({ cap: 8, onFilter: () => asked++ });
  f.setData(graph);
  const all = new Set(graph.nodes.map((n) => n.id));
  const remoteDrawn = (vis, host) => [...vis].filter((id) => id.startsWith(`@${host}:`) && !id.endsWith(":core")).length;
  // Not told yet: everything the other filters let through.
  assert.equal(f.restrict(all).size, all.size);
  assert.ok(f.chips().every((c) => !c.capped));
  // The first live state, which comes as the page loads, turns the cap on:
  // at the overview, eight between the two, every host node, and all of
  // this machine.
  f.setLive({ sessions: [] });
  assert.equal(asked, 1, "the filters run again");
  let vis = f.restrict(all);
  assert.deepEqual([remoteDrawn(vis, "vm1"), remoteDrawn(vis, "studio")], [4, 4]);
  assert.ok(vis.has("@vm1:core") && vis.has("@studio:core") && vis.has("claude:1") && vis.has("core"));
  assert.deepEqual(f.chips().map((c) => c.capped ?? null), [null, { drawn: 4, of: 18 }, { drawn: 4, of: 18 }]);
  f.look(null);
  assert.equal(asked, 1, "nothing selected, as it was");
  // Looking at one of vm1's sessions draws all of vm1; studio gets the cap.
  f.look(a.nodes.find((n) => n.id === "@vm1:claude:2-2"));
  assert.equal(asked, 2);
  vis = f.restrict(all);
  assert.deepEqual([remoteDrawn(vis, "vm1"), remoteDrawn(vis, "studio")], [18, 8]);
  f.look(a.nodes.find((n) => n.id === "@vm1:project:~/projects/p0"));
  assert.equal(asked, 2, "the same host: nothing to do again");
  // Back to this machine's own; then studio's chip, which is looking at it.
  f.look({ id: "claude:1", kind: "session" });
  assert.deepEqual([remoteDrawn(f.restrict(all), "vm1"), remoteDrawn(f.restrict(all), "studio")], [4, 4]);
  f.setSelected(["studio", HERE]);
  vis = f.restrict(all);
  assert.deepEqual([remoteDrawn(vis, "vm1"), remoteDrawn(vis, "studio")], [0, 18]);
  f.setSelected([]);
  // A remote session that starts waiting on you is drawn at once; the
  // filters run again only when that changes.
  asked = 0;
  f.setLive({ sessions: [{ id: "@vm1:claude:3-2", host: "vm1", needsYou: { kind: "question" } }, { id: "claude:1", needsYou: { kind: "question" } }] });
  f.setLive({ sessions: [{ id: "@vm1:claude:3-2", host: "vm1", needsYou: { kind: "question" } }, { id: "@vm1:claude:1-1", host: "vm1", status: "tool" }] });
  assert.equal(asked, 1);
  vis = f.restrict(all);
  assert.ok(vis.has("@vm1:claude:3-2") && vis.has("@vm1:project:~/projects/p3"));
  // Filters still apply first: what they hide isn't counted, or drawn.
  const some = new Set([...all].filter((id) => !id.startsWith("@studio:claude:")));
  assert.equal(f.chips().length, 3);
  vis = f.restrict(some);
  assert.ok(![...vis].some((id) => id.startsWith("@studio:claude:")));
  assert.deepEqual(f.chips().filter((c) => c.capped).map((c) => [c.key, c.capped]), [["studio", { drawn: 3, of: 6 }], ["vm1", { drawn: 5, of: 18 }]], "what waits on you counts toward the cap");

  // A selection turns it on as well.
  const byLook = createHostFilters({ cap: 8 });
  byLook.setData(graph);
  byLook.look(null);
  assert.equal(remoteDrawn(byLook.restrict(all), "vm1"), 4);

  // A capped host's chip says how much of it is drawn, and how to see the rest.
  const box = { hidden: true, innerHTML: "" };
  globalThis.document = { getElementById: (id) => (id === "host-filters" ? box : null) };
  try {
    const g = createHostFilters({ cap: 8 });
    g.setData(graph);
    g.look(null);
    g.restrict(all);
    assert.match(/data-host-key="studio" title="([^"]*)"/.exec(box.innerHTML)[1], /^studio · live\n12 sessions on the map, 4 of its 18 nodes drawn until you look at it: select one, or click to show only its work\.$/);
  } finally {
    delete globalThis.document;
  }
});
