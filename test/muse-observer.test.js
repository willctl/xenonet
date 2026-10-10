// Muse's observer (core/actors/muse.js) on a made-up process table and
// map: presence from its Mac app alone, never working, only when the app
// is Meta's by its bundle id, and nothing of its files but whether the app
// is there and the two keys of its Info.plist that say its version and
// whose it is.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { MuseObserver } from "../core/actors/muse.js";
import { EngineStore } from "../core/engine/store.js";
import { createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeMap, NODES, ATLAS_DIR } from "./fixtures/actor-map.js";

const T = Date.parse("2026-10-08T18:00:20Z");
const DAY = 24 * 3600e3;
const ID = "actor:muse";
const APP = "/Applications/Muse.app";
const PLIST = `${APP}/Contents/Info.plist`;
const SECRET = "made-up-token-in-an-argument";

// A machine where Muse is installed (unless `installed` is false), its app
// running (unless `running` is false). Its Info.plist is binary, as Muse's
// is, so its keys come through `binary`, which says which were asked; the
// observer reads files only through `files`, which says what was asked
// (`asked`) and read (`read`). `id` is the bundle id the app has: Meta's,
// or another maker's.
function machine({ installed = true, running = true, id = "com.meta.endo", version = "7.0" } = {}) {
  const clock = { t: T, now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const store = new EngineStore(":memory:");
  const actors = new Actors({ now: () => clock.t, map: fakeMap(NODES()), store });
  if (installed) m.bundle(APP, id);
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const asked = [];
  const read = [];
  const keys = [];
  const info = { version, id, mtimeMs: T - DAY };
  const files = {
    existsSync: (p) => (asked.push(p), installed && p === APP),
    statSync: (p) => {
      asked.push(p);
      if (!installed || p !== PLIST) throw Object.assign(new Error("no such file"), { code: "ENOENT" });
      return { mtimeMs: info.mtimeMs, size: 4242 };
    },
    readFileSync: (p) => {
      if (p !== PLIST) assert.fail(`Muse's files are never read, beyond its Info.plist: ${p}`);
      read.push(p);
      return Buffer.from("bplist00 made-up bytes");
    },
    readdirSync: () => assert.fail("Muse's folders are never listed"),
  };
  const binary = async (file, key) => {
    keys.push([file, key]);
    return { CFBundleShortVersionString: info.version, CFBundleIdentifier: info.id }[key] ?? null;
  };
  let app = null;
  if (running) {
    app = m.add({ ppid: 1, program: `${APP}/Contents/MacOS/Muse`, start: T - 3600e3 });
    m.add({ ppid: 1, program: `${APP}/Contents/MacOS/EndoCrashpadHandler`, start: T - 3600e3 });
  }
  const make = (as = actors) => new MuseObserver({ actors: as, procs, home: "/Users/nova", apps: [APP], files, binary, clock });
  const observer = make();
  const look = async (at = clock.t) => {
    clock.t = at;
    await observer.tick(at);
  };
  return { clock, m, store, actors, procs, observer, make, files, asked, read, keys, info, app, look, snap: () => actors.snapshot(ID) };
}

test("not installed: no actor; installed with its app closed: unknown, not offline, with no heartbeat, and the presence alone", async () => {
  const none = machine({ installed: false, running: false });
  await none.look();
  assert.equal(none.actors.has(ID), false);
  const closed = machine({ running: false });
  await closed.look();
  const s = closed.snap();
  assert.deepEqual(
    [s.name, s.product, s.via, s.presence, s.status, s.heartbeatMs, s.identity],
    ["Muse", { id: "muse", vendor: "Meta", version: "7.0" }, ["observed"], "unknown", "Its app on this Mac is closed; it may still work in Meta's cloud", null, null],
  );
  assert.deepEqual(s.capabilities, ["message-people", "schedule", "browse", "use-computer"]);
  closed.clock.t += DAY;
  closed.actors.tick();
  assert.equal(closed.snap().presence, "unknown", "nothing here can say a cloud agent stopped");
});

test("its app open: idle, and never working, whatever runs under it; closed again: unknown", async () => {
  const w = machine();
  await w.look();
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", "Its app is open on this Mac; it works in Meta's cloud"]);
  // Something under its app: nothing XenoNet can say is Muse's work.
  w.m.add({ ppid: w.app, program: "/usr/bin/osascript", args: ["-e", SECRET], cwd: ATLAS_DIR, start: T + 1000 });
  await w.look(T + 2000);
  await w.look(T + 2 * 60e3);
  assert.equal(w.snap().presence, "idle");
  assert.deepEqual(w.snap().targets, [], "no places");
  assert.ok(w.snap().recent.every((e) => e.kind === "presence" || e.kind === "status"), "no events beyond its presence and status");
  assert.ok(!JSON.stringify([w.actors.list(), w.actors.events({ since: 0 })]).includes(SECRET));
  // Its crash reporter outlives it for a moment: the app is closed.
  w.m.remove(w.app);
  await w.look(T + 3 * 60e3);
  assert.equal(w.snap().presence, "unknown");
});

test("installed, its app closed for weeks: still unknown and listed, across a restart too; open again, it's back", async () => {
  const w = machine({ running: false });
  await w.look();
  await w.look(T + 6 * DAY);
  w.actors.stop();
  const actors = new Actors({ now: () => w.clock.t, map: fakeMap(), store: w.store });
  const observer = w.make(actors);
  w.clock.t = T + 30 * DAY;
  await observer.tick(w.clock.t);
  actors.tick();
  assert.deepEqual(actors.list().actors.map((a) => [a.id, a.presence]), [["actor:you", "unknown"], [ID, "unknown"]], "listed for as long as it's installed");
  w.m.add({ ppid: 1, program: `${APP}/Contents/MacOS/Muse`, start: w.clock.t });
  w.clock.t += 5000;
  await observer.tick(w.clock.t);
  assert.equal(actors.snapshot(ID).presence, "idle");
  assert.ok(actors.snapshot(ID).recent.some((e) => e.detail === "Came online"));
});

test("another maker's Muse.app isn't Meta's: no actor, though it's there and running", async () => {
  const w = machine({ id: "com.example.muse-notes" });
  await w.look();
  await w.look(T + 5000);
  assert.equal(w.actors.has(ID), false);
  const st = w.observer.status();
  assert.deepEqual([st.installed, st.sources.map((s) => [s.name, s.state])], [false, [["processes", "ok"], ["app", "other"], ["app's version", "ok"]]]);
});

test("its files: the app's own path and its Info.plist, two keys of it, read again only when it changes", async () => {
  const w = machine();
  await w.look();
  await w.look(T + 5000);
  assert.ok(w.asked.every((p) => p === APP || p === PLIST), "the app's path and its Info.plist, and nothing else of its");
  assert.deepEqual(w.keys, [[PLIST, "CFBundleShortVersionString"], [PLIST, "CFBundleIdentifier"]], "those two keys, once");
  assert.equal(w.read.length, 1);
  assert.ok(!JSON.stringify([w.actors.list(), w.observer.status()]).includes("com.meta.endo"), "its id says whose it is, and isn't shown");
  // An update writes a new one.
  Object.assign(w.info, { version: "7.1", mtimeMs: T + 6000 });
  await w.look(T + 10e3);
  assert.deepEqual([w.snap().product.version, w.read.length], ["7.1", 2]);
  // What doesn't read as a version isn't shown, and the one before stands.
  Object.assign(w.info, { version: `${SECRET} and more`, mtimeMs: T + 11e3 });
  await w.look(T + 15e3);
  assert.equal(w.snap().product.version, "7.1");
  const st = w.observer.status();
  assert.deepEqual([st.product, st.name, st.installed, st.sources.map((s) => [s.name, s.state])], ["muse", "Muse", true, [["processes", "ok"], ["app", "ok"], ["app's version", "unreadable"]]]);
});

test("its observer off: nothing read", async () => {
  const w = machine();
  const off = new MuseObserver({ actors: w.actors, procs: w.procs, home: "/Users/nova", apps: [APP], files: w.files, observing: () => false, clock: w.clock });
  assert.equal(await off.tick(T), false);
  assert.deepEqual([w.asked, w.read, w.m.asked], [[], [], []]);
  assert.equal(w.actors.has(ID), false);
});
