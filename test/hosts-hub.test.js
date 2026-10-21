// The hub (core/hosts/hub.js) in one process, with relays a script plays on
// a made-up machine and a clock the test moves: the merged view and how
// often it goes out, the cache that keeps an offline host on the map, what
// the page hears, and hosts.json trouble that mustn't stop XenoNet.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createHub } from "../core/hosts/hub.js";
import { HostsConfig } from "../core/hosts/config.js";
import { readCache } from "../core/hosts/cache.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeClock } from "./harness/clock.js";

const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-hosts-hub-"));
  made.push(dir);
  return dir;
};

const KEY = "9a2b0000000000000000000000000002";
const placed = { fx: null, fy: null, fz: null, pinned: false, actions: [] };
const rolled = { sessionCount: 1, activeCount: 1, lastActivity: null, lastSessionAt: null };
const S = "claude:0190d3e0-0000-7000-8000-000000000001";

// The xenonet binary a local link runs.
const BIN = "/x/launcher/xenonet";

function setup({ clock = fakeClock(), stateDir = tmp(), cacheDir = tmp(), key = null } = {}) {
  const machine = fakeProcesses({ clock });
  const hosts = new HostsConfig({ stateDir, clock });
  if (!hosts.get("vm1")) hosts.add({ id: "vm1", target: "lab@vm1.local" });
  if (key) hosts.learn("vm1", { key });
  const mine = { generatedAt: clock.now(), nodes: [{ id: "core", kind: "core", name: "~/Projects", path: "/x", home: "/x", ...placed, ...rolled }], links: [], relations: [], stats: { sessions: 0 }, identities: [] };
  const theirs = (title = "Remote work") => ({
    generatedAt: clock.now(),
    nodes: [
      { id: "core", kind: "core", name: "~/Projects", path: "~/Projects", home: "~", ...placed, ...rolled },
      { id: S, kind: "session", app: "claude", rawId: S.slice(7), title, preview: null, cwd: "~/Projects/atlas", createdAt: clock.now(), updatedAt: clock.now(), archived: false, model: null, branch: null, parentId: null, parentKind: null, activity: "active", ...placed },
    ],
    links: [{ source: "core", target: S, kind: "session" }],
    relations: [],
    identities: [],
  });
  const liveOf = () => ({ at: clock.now(), sessions: [], series: { claude: [0], codex: [0] }, tpm: 0, total24h: 0, today: 0, limits: { claude: null, codex: null }, limitForecast: {}, collab: [], collisions: [], resources: [], status: {} });
  let relay = null;
  // ssh, and the xenonet binary that runs another XenoNet's relay here,
  // both play the same made-up relay.
  const script = (io) => {
    relay = io;
    io.json({ t: "hello", schema: "host/hello/v1", protocol: 1, version: "0.4.0", host: { id: "lab", key: KEY }, platform: "linux", demo: false, now: clock.now(), sinceMs: 604800000, detail: false });
    io.json({ t: "snapshot", at: clock.now(), graph: theirs(), live: liveOf(), engine: { agents: [{ id: S, app: "claude", title: "x", project: null, branch: null }], messages: [] } });
    io.on("line", (l) => {
      const f = JSON.parse(l);
      if (f.t === "time") io.json({ t: "time", id: f.id, now: clock.now() });
    });
  };
  machine.program("ssh", script);
  machine.program(BIN, script);
  const notes = [];
  const log = { info: (code, message) => notes.push([code, message]) };
  const hub = createHub({ hosts, cacheDir, root: "/src/xenonet", home: "/Users/nova", local: { graph: () => mine, live: liveOf, layout: () => ({}), identities: () => ({}), aliases: () => new Map(), forgetLayout: () => 0 }, clock, linkOptions: { spawn: machine.spawn, random: () => 0.5, bin: BIN }, log });
  const out = { graph: [], live: [], hosts: [], activity: [], tokens: [] };
  for (const name of Object.keys(out)) hub.on(name, (d) => out[name].push(d));
  return { clock, machine, hosts, hub, out, notes, mine, theirs, relay: () => relay, stateDir, cacheDir, settle: () => clock.flush(40) };
}

test("a host's map joins this one, and goes out at most once a second however often it changes", async () => {
  const w = setup();
  w.hub.start();
  await w.settle();
  const merged = w.hub.graph(w.mine);
  assert.ok(merged.nodes.some((n) => n.id === `@vm1:${S}`));
  assert.equal(merged.nodes.find((n) => n.id === "@vm1:core").calls, "lab");
  assert.equal(w.hosts.get("vm1").key, KEY, "the key it learned is kept in hosts.json");
  assert.equal(w.hosts.get("vm1").name, "lab");
  await w.clock.advance(1000);
  const before = w.out.graph.length;
  for (let i = 0; i < 10; i++) w.relay().json({ t: "event", name: "graph", data: w.theirs(`Remote work ${i}`) });
  await w.settle();
  await w.clock.advance(1000);
  assert.equal(w.out.graph.length - before, 1, "ten changes, one map");
  assert.equal(w.out.graph.at(-1).nodes.find((n) => n.id === `@vm1:${S}`).title, "Remote work 9", "the last one");
  w.relay().json({ t: "event", name: "activity", data: { sessionId: S, app: "claude", kind: "turn-end", duration: 5, ts: w.clock.now() } });
  await w.settle();
  assert.deepEqual(w.out.activity.at(-1).sessionId, `@vm1:${S}`);
  assert.equal(w.hub.detail("vm1").engine.agents[0].id, `@vm1:${S}`);
  w.hub.stop();
});

test("a host's tokens for an app this XenoNet doesn't tail are left out, and the log says so", async () => {
  const w = setup();
  w.hub.start();
  await w.settle();
  w.relay().json({ t: "event", name: "tokens", data: { sessionId: S, app: "grok-build", n: 40, ts: w.clock.now() } });
  w.relay().json({ t: "event", name: "tokens", data: { sessionId: S, app: "claude", n: 50, ts: w.clock.now() } });
  await w.settle();
  assert.deepEqual(w.out.tokens.map((t) => [t.sessionId, t.app, t.n]), [[`@vm1:${S}`, "claude", 50]], "not counted as Claude Code's");
  assert.deepEqual(w.notes, [["hosts.unknown-app", 'vm1 sent tokens for "grok-build", an app this XenoNet doesn\'t tail, and they were left out']]);
  w.hub.stop();
});

test("the page hears how a host stands, not how it's reached; the command gets both", async () => {
  const w = setup();
  w.hub.start();
  await w.settle();
  const heard = w.out.hosts.at(-1)[0];
  for (const k of ["target", "key", "command", "counts", "lastDrop"]) assert.ok(!(k in heard), k);
  assert.deepEqual([heard.id, heard.state, heard.calls, heard.version], ["vm1", "live", "lab", "0.4.0"]);
  const listed = w.hub.list()[0];
  assert.deepEqual([listed.target, listed.key, listed.command], ["lab@vm1.local", KEY, "xenonet"]);
  // A host's own doctor: its summary's counts and its problems, nothing else.
  w.relay().json({ t: "event", name: "doctor", data: { at: 1, summary: { ok: 3, warn: "lots", evil: { x: 1 } }, problems: [{ id: "codex.auth", title: "Codex is signed in", status: "fail", detail: "not signed in", extra: "x" }, "junk"] } });
  await w.settle();
  assert.deepEqual(w.hub.list()[0].doctor, { at: 1, summary: { ok: 3, info: 0, warn: 0, fail: 0, skip: 0 }, problems: [{ id: "codex.auth", title: "Codex is signed in", status: "fail", detail: "not signed in" }] });
  w.hub.stop();
});

test("an offline host stays on the map from its cache, after a restart too, and forgetting it takes the cache", async () => {
  const w = setup();
  w.hub.start();
  await w.settle();
  await w.clock.advance(30e3);
  const cached = readCache(w.cacheDir, KEY);
  assert.ok(cached, "written, at most every 30 seconds");
  assert.deepEqual([cached.id, cached.hello.version, cached.graph.nodes.length], ["vm1", "0.4.0", 2]);
  w.hub.stop();

  // XenoNet starts again, and vm1 doesn't answer this time.
  const again = setup({ stateDir: w.stateDir, cacheDir: w.cacheDir });
  again.machine.program("ssh", () => ({ code: 255, stderr: "ssh: connect to host vm1.local port 22: Operation timed out\n" }));
  again.hub.start();
  await again.settle();
  const map = again.hub.graph(again.mine);
  const host = map.nodes.find((n) => n.id === "@vm1:core");
  assert.deepEqual([host.state, host.cached, host.version, host.error], ["offline", true, "0.4.0", "ssh couldn't reach vm1: connect to host vm1 port 22: Operation timed out"]);
  assert.ok(map.nodes.find((n) => n.id === `@vm1:${S}`).stale);
  assert.equal(again.hub.remove("vm1").removed, true);
  assert.equal(readCache(again.cacheDir, KEY), null);
  assert.deepEqual(again.hosts.list(), []);
  assert.equal(again.hub.graph(again.mine), again.mine, "with no hosts, the map is this machine's own");
  again.hub.stop();
});

test("a host back with another key leaves one snapshot in the cache, and forgetting or replacing it leaves none", async () => {
  const w = setup();
  const keys = ["aaaa0000000000000000000000000001", "bbbb0000000000000000000000000002", "cccc0000000000000000000000000003", "dddd0000000000000000000000000004"];
  let starts = 0;
  // A demo world, made afresh with every start, key and all.
  w.machine.program("ssh", (io) => {
    const key = keys[Math.min(starts++, keys.length - 1)];
    io.json({ t: "hello", schema: "host/hello/v1", protocol: 1, version: "0.4.0", host: { id: "lab", key }, platform: "linux", demo: true, now: w.clock.now(), sinceMs: 604800000, detail: false });
    io.json({ t: "snapshot", at: w.clock.now(), graph: w.theirs(), live: { at: w.clock.now(), sessions: [], series: { claude: [0], codex: [0] }, tpm: 0, total24h: 0, today: 0, limits: { claude: null, codex: null }, limitForecast: {}, collab: [], collisions: [], resources: [], status: {} }, engine: null });
    io.on("line", (l) => {
      const f = JSON.parse(l);
      if (f.t === "time") io.json({ t: "time", id: f.id, now: w.clock.now() });
    });
  });
  const files = (dir = w.cacheDir) => (fs.existsSync(path.join(dir, "hosts")) ? fs.readdirSync(path.join(dir, "hosts")).sort() : []);
  w.hub.start();
  await w.settle();
  await w.clock.advance(30e3);
  assert.deepEqual(files(), [`${keys[0]}.json`]);
  for (const key of keys.slice(1, 3)) {
    w.hub.reconnect("vm1");
    await w.settle();
    await w.clock.advance(30e3);
    assert.deepEqual(files(), [`${key}.json`], "kept under its new key, and only there");
  }
  // Back once more, and forgotten before its snapshot is kept again.
  w.hub.reconnect("vm1");
  await w.settle();
  assert.equal(w.hosts.get("vm1").key, keys[3]);
  w.hub.remove("vm1");
  assert.deepEqual(files(), [], "nothing of it left");
  w.hub.stop();

  // Added again with --replace: the entry forgets the key it learned, so
  // its old snapshot goes too, and the host back is kept again.
  const r = setup();
  r.hub.start();
  await r.settle();
  await r.clock.advance(30e3);
  assert.deepEqual(files(r.cacheDir), [`${KEY}.json`]);
  r.hub.add({ id: "vm1", target: "lab@vm1.local", since: "12h" }, { replace: true });
  assert.deepEqual(files(r.cacheDir), []);
  await r.settle();
  await r.clock.advance(30e3);
  assert.deepEqual(files(r.cacheDir), [`${KEY}.json`]);
  r.hub.stop();

  // One XenoNet added twice, under two ids: forgetting one leaves the
  // other's snapshot, which is the same file.
  const twice = setup();
  twice.hosts.add({ id: "vm2", target: "lab@vm1.local" });
  twice.hub.start();
  await twice.settle();
  await twice.clock.advance(30e3);
  assert.deepEqual(files(twice.cacheDir), [`${KEY}.json`]);
  twice.hub.remove("vm1");
  assert.deepEqual(files(twice.cacheDir), [`${KEY}.json`], "still vm2's");
  twice.hub.remove("vm2");
  assert.deepEqual(files(twice.cacheDir), []);
  twice.hub.stop();
});

test("hosts.json that can't be written doesn't stop the hub", async () => {
  const stateDir = tmp();
  const w = setup({ stateDir });
  // A newer XenoNet's file: read, never written.
  const file = path.join(stateDir, "hosts.json");
  const doc = JSON.parse(fs.readFileSync(file, "utf8"));
  fs.writeFileSync(file, JSON.stringify({ ...doc, schema: "host/list/v2" }));
  const later = setup({ stateDir });
  assert.doesNotThrow(() => later.hub.start());
  await later.settle();
  assert.equal(later.hub.list()[0].state, "live", "it learned the key, and kept it in memory");
  assert.equal(JSON.parse(fs.readFileSync(file, "utf8")).schema, "host/list/v2");
  later.hub.stop();
  w.hub.stop();
});

test("XenoNet Dev's simulations: another XenoNet here as a host, offline and back, and its clock off", async () => {
  const w = setup();
  w.hub.start();
  await w.settle();
  // What core/control.js sends: { a, b }.
  assert.deepEqual(w.hub.simulate("host-add", { a: "demo-b", b: "47999" }), { ok: true, kind: "host-add", id: "demo-b" });
  await w.settle();
  // The xenonet binary runs the relay, with no Node or entry script.
  const local = w.machine.spawned.find((x) => x.file === BIN);
  assert.deepEqual(local.args, ["relay", "--port", "47999", "--since", "7d"]);
  assert.equal(w.hosts.get("demo-b"), null, "in memory only, never in hosts.json");
  const state = (id) => w.hub.list().find((h) => h.id === id);
  assert.deepEqual([state("demo-b").state, state("demo-b").simulated], ["live", true]);
  assert.equal(w.hub.simulate("host-add", { a: "demo-b", b: "1" }).error, "there's already a host called demo-b");
  assert.match(w.hub.simulate("host-add", { a: "Not An Id", b: "1" }).error, /name you choose/);
  w.hub.simulate("host-offline", { a: "demo-b" });
  assert.deepEqual([state("demo-b").state, state("demo-b").error.code], ["offline", "paused"]);
  w.hub.simulate("host-online", { a: "demo-b" });
  await w.settle();
  assert.equal(state("demo-b").state, "live");
  w.hub.simulate("host-skew", { a: "demo-b", b: "-60000" });
  assert.equal(state("demo-b").offsetMs, -60000);
  w.hub.simulate("host-skew", { a: "demo-b" });
  assert.equal(state("demo-b").offsetMs, 180e3, "three minutes, when it isn't said");
  assert.equal(w.hub.simulate("host-offline", { a: "nowhere" }), null);
  assert.equal(w.hub.simulate("host-fly", { a: "demo-b" }), null);
  w.hub.stop();
});
