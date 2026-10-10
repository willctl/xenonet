// Grok Bot's observer (core/actors/grok-bot.js) on a made-up process table
// and map: presence from its app, the commands it runs here by ancestry
// (counted by place, never by their arguments), the sessions they start,
// and nothing of its files but whether the app is there and the two keys of
// its Info.plist that says its version.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { GrokBotObserver, POLL_MS } from "../core/actors/grok-bot.js";
import { EngineStore } from "../core/engine/store.js";
import { createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeClock } from "./harness/clock.js";
import { fakeMap, NODES, ATLAS, LEDGER, S1, ATLAS_DIR, LEDGER_DIR } from "./fixtures/actor-map.js";

const T = Date.parse("2026-10-05T12:00:20Z");
const MIN = 60e3;
const ID = "actor:grok-bot";
const APP = "/Applications/Grok Bot.app";
const PLIST = `${APP}/Contents/Info.plist`;
const SECRET = "made-up-token-in-an-argument";

// A made-up Info.plist, as XML, with keys beside the version that XenoNet
// never takes.
const infoPlist = (version = "0.70.4") => `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.anysphere.sand</string>
	<key>CFBundleShortVersionString</key>
	<string>${version}</string>
	<key>SUFeedURL</key>
	<string>https://updates.example.com/${SECRET}</string>
</dict>
</plist>
`;

// A machine where Grok Bot is installed (unless `installed` is false), its
// app running (unless `running` is false), with its main process and
// helpers; the observer reads files only through `files`, which says what
// was asked (`asked`) and read (`read`). `plist` is its Info.plist, which
// a test may change.
function machine({ installed = true, running = true, nodes = NODES(), plist = infoPlist() } = {}) {
  const clock = { t: T, now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const store = new EngineStore(":memory:");
  const actors = new Actors({ now: () => clock.t, map: fakeMap(nodes), store });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const asked = [];
  const read = [];
  const info = { text: plist, mtimeMs: T - 86400e3 };
  const files = {
    existsSync: (p) => (asked.push(p), installed && p === APP),
    statSync: (p) => {
      asked.push(p);
      if (!installed || p !== PLIST || info.text == null) throw Object.assign(new Error("no such file"), { code: "ENOENT" });
      return { mtimeMs: info.mtimeMs, size: Buffer.byteLength(info.text) };
    },
    readFileSync: (p) => {
      if (p !== PLIST) assert.fail(`Grok Bot's files are never read, beyond its Info.plist: ${p}`);
      read.push(p);
      if (!installed || info.text == null) throw Object.assign(new Error("no such file"), { code: "ENOENT" });
      return Buffer.from(info.text);
    },
    readdirSync: () => assert.fail("Grok Bot's folders are never listed"),
  };
  let app = null;
  let helper = null;
  if (running) {
    app = m.add({ ppid: 1, program: `${APP}/Contents/MacOS/Grok Bot`, start: T - 3600e3 });
    helper = m.add({ ppid: app, program: `${APP}/Contents/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper`, start: T - 3600e3 });
    m.add({ ppid: app, program: `${APP}/Contents/Frameworks/Grok Bot Helper (Renderer).app/Contents/MacOS/Grok Bot Helper (Renderer)`, start: T - 3600e3 });
    m.add({ ppid: 1, program: `${APP}/Contents/Frameworks/Electron Framework.framework/Helpers/chrome_crashpad_handler`, start: T - 3600e3 });
  }
  const observer = new GrokBotObserver({ actors, procs, home: "/Users/nova", apps: [APP], files, alive: (pid) => m.table.has(pid), clock });
  const look = async (at = clock.t) => {
    clock.t = at;
    await observer.tick(at);
  };
  const poll = async (at = clock.t) => {
    clock.t = at;
    await observer.poll(at);
  };
  return { clock, m, store, actors, procs, observer, files, asked, read, info, app, helper, look, poll, snap: () => actors.snapshot(ID) };
}
const events = (s, kind) => s.recent.filter((e) => e.kind === kind);

test("not installed: no actor; installed with its app closed: unknown, not offline, with no heartbeat", async () => {
  const none = machine({ installed: false, running: false });
  await none.look();
  assert.equal(none.actors.has(ID), false);
  const closed = machine({ running: false });
  await closed.look();
  const s = closed.snap();
  assert.deepEqual([s.name, s.product, s.via, s.presence, s.status, s.heartbeatMs, s.identity], ["Grok Bot", { id: "grok-bot", vendor: "SpaceXAI", version: "0.70.4" }, ["observed", "attributed"], "unknown", "Its app on this Mac is closed", null, null]);
  assert.deepEqual(s.capabilities, ["run-commands", "message-people", "message-agents", "schedule", "browse"]);
  closed.clock.t += 24 * 3600e3;
  closed.actors.tick();
  assert.equal(closed.snap().presence, "unknown", "nothing here can say a cloud agent stopped");
});

test("installed, its app closed for weeks: still unknown, never offline, and still listed, across a restart too; open again, it's back", async () => {
  const DAY = 24 * 3600e3;
  const w = machine({ running: false });
  await w.look();
  await w.look(T + 6 * DAY);
  assert.equal(w.snap().presence, "unknown", "a Bot may go on in the cloud");
  w.actors.stop();
  // XenoNet restarts, more than a week after its app was last seen: a Bot
  // may still be at work in the cloud, so it's out of sight, not offline.
  const actors = new Actors({ now: () => w.clock.t, map: fakeMap(), store: w.store });
  const observer = new GrokBotObserver({ actors, procs: w.procs, home: "/Users/nova", apps: [APP], files: w.files, alive: (pid) => w.m.table.has(pid), clock: w.clock });
  w.clock.t = T + 7 * DAY + 5000;
  await observer.tick(w.clock.t);
  const snap = () => actors.snapshot(ID);
  assert.deepEqual([snap().presence, snap().status], ["unknown", "Its app on this Mac is closed"]);
  w.clock.t = T + 30 * DAY;
  await observer.tick(w.clock.t);
  actors.tick();
  assert.deepEqual(actors.list().actors.map((a) => [a.id, a.presence]), [["actor:you", "unknown"], [ID, "unknown"]], "listed for as long as it's installed");
  assert.ok(!snap().recent.some((e) => e.detail === "Went offline"));
  w.m.add({ ppid: 1, program: `${APP}/Contents/MacOS/Grok Bot`, start: w.clock.t });
  w.clock.t += 5000;
  await observer.tick(w.clock.t);
  assert.deepEqual([snap().presence, snap().recent.at(-1).detail], ["idle", "Its app is open on this Mac"]);
  assert.ok(snap().recent.some((e) => e.detail === "Came online"));
});

test("its app open: idle; a command it runs: working, in the project it runs in, counted a minute at a time", async () => {
  const w = machine();
  await w.look();
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", "Its app is open on this Mac"]);
  // A command, and what it runs: one command.
  const zsh = w.m.add({ ppid: w.helper, program: "/bin/zsh", args: ["-c", `git push https://${SECRET}@example.com`], cwd: ATLAS_DIR, start: T + 1000 });
  w.m.add({ ppid: zsh, program: "/usr/bin/git", args: ["push", SECRET], cwd: ATLAS_DIR, start: T + 1000 });
  await w.look(T + 2000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["working", "Running commands in atlas-api"]);
  assert.deepEqual(w.snap().targets.map((t) => [t.id, t.relation]), [[ATLAS, "works-on"]]);
  // It ends; a minute on, the command is an event, and half a minute after
  // its last command it's idle again.
  w.m.remove(zsh);
  await w.look(T + 20e3);
  assert.equal(w.snap().presence, "working", "a run of commands is one stretch of work");
  await w.look(Math.floor(T / MIN) * MIN + MIN + 20e3);
  assert.deepEqual(events(w.snap(), "ran").map((e) => [e.detail, e.target, e.via]), [["Ran a command in atlas-api", ATLAS, "observed"]]);
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", "Its app is open on this Mac"]);
  assert.ok(!JSON.stringify([w.actors.list(), w.actors.events({ since: 0 })]).includes(SECRET), "never a command's arguments");
  assert.ok(!JSON.stringify(w.actors.list()).includes("/usr/bin/git"), "nor its program, beyond counting it");
});

test("commands too quick for the table: caught by the poll of what the app runs", async () => {
  const w = machine();
  await w.look();
  const before = w.m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length;
  const quick = w.m.add({ ppid: w.app, program: "/bin/sh", cwd: LEDGER_DIR, start: T + 1000 });
  await w.poll(T + 1000);
  w.m.remove(quick);
  await w.poll(T + 2000);
  const other = w.m.add({ ppid: w.helper, program: "/bin/zsh", cwd: "/Users/nova/elsewhere", start: T + 2500 });
  await w.poll(T + 3000);
  w.m.remove(other);
  assert.equal(w.m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length, before, "without reading the whole table");
  assert.ok(w.m.asked.some((a) => a[0] === "pgrep"));
  await w.look(T + 4000);
  assert.equal(w.snap().presence, "working");
  assert.equal(w.snap().status, "Running commands");
  await w.look(Math.floor(T / MIN) * MIN + 2 * MIN);
  assert.deepEqual(events(w.snap(), "ran").map((e) => e.detail).sort(), ["Ran a command", "Ran a command in ledger"]);
});

test("a command the poll saw start after the shared table was taken counts once, though the look reads that older table", async () => {
  const w = machine();
  await w.look(T + 6000);
  const command = w.m.add({ ppid: w.app, program: "/bin/zsh", cwd: ATLAS_DIR, start: T + 6200 });
  await w.poll(T + 6500);
  // The table read at +6 s is shared for a moment, so this look reads it
  // again, without the command.
  await w.look(T + 7000);
  await w.poll(T + 7500);
  w.m.remove(command);
  await w.look(Math.floor(T / MIN) * MIN + MIN + 20e3);
  assert.deepEqual(events(w.snap(), "ran").map((e) => e.detail), ["Ran a command in atlas-api"]);
  assert.equal(w.observer.tracked.has(command), false, "and once it's gone, it's let go");
});

test("the sessions its commands start: attributed, by the engine's pin, else the place after two minutes", async () => {
  const w = machine();
  await w.look();
  const zsh = w.m.add({ ppid: w.helper, program: "/bin/zsh", cwd: ATLAS_DIR, start: T + 1000 });
  const claude = w.m.add({ ppid: zsh, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T + 1000 });
  w.store.pin(S1, { pid: claude, start: w.m.table.get(claude).start }, true, T);
  // Claude's own children aren't sessions Grok Bot started.
  w.m.add({ ppid: claude, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T + 1500 });
  const codex = w.m.add({ ppid: w.app, program: "/opt/homebrew/bin/codex", cwd: LEDGER_DIR, start: T + 2000 });
  await w.poll(T + 2500);
  await w.look(T + 3000);
  assert.deepEqual(events(w.snap(), "spawned").map((e) => [e.target, e.via, e.detail]), [[S1, "attributed", "Started a Claude Code session in atlas-api"]]);
  await w.look(T + 3000 + 2 * MIN);
  assert.deepEqual(events(w.snap(), "spawned").map((e) => [e.target, e.detail]), [[S1, "Started a Claude Code session in atlas-api"], [LEDGER, "Started a Codex session in ledger"]]);
  assert.ok(codex);
});

test("a claude its app runs that holds no session (claude --version) isn't a session started, nor is yours, begun nearby just after", async () => {
  const nodes = NODES();
  const w = machine({ nodes });
  await w.look();
  const version = w.m.add({ ppid: w.helper, program: "/Users/nova/.local/bin/claude", args: ["--version"], cwd: ATLAS_DIR, start: T + 1000 });
  await w.poll(T + 1500);
  w.m.remove(version);
  await w.look(T + 5000);
  // Half a minute on, you start a session of your own there, from your
  // terminal, without XenoNet's hooks to pin it.
  const yours = "claude:0190d3e0-0000-7000-8000-00000000dd01";
  nodes[yours] = { kind: "session", app: "claude", home: ATLAS, cwd: ATLAS_DIR, createdAt: T + 31e3 };
  w.m.add({ ppid: 1, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T + 31e3 });
  for (const t of [T + 35e3, T + MIN, T + 2 * MIN + 10e3, T + 3 * MIN]) await w.look(t);
  assert.deepEqual(events(w.snap(), "spawned"), []);
  assert.ok(!w.snap().targets.some((t) => t.id === yours), "no beam to your session");
  assert.deepEqual(events(w.snap(), "ran").map((e) => e.detail), ["Ran a command in atlas-api"], "it's a command it ran, all the same");
});

test("Claude Code installed with npm, a Node script: told by its arguments at the next look, even when the poll saw it first", async () => {
  const w = machine();
  await w.look();
  const zsh = w.m.add({ ppid: w.helper, program: "/bin/zsh", args: ["-c", "claude -p go"], cwd: ATLAS_DIR, start: T + 1000 });
  const node = w.m.add({ ppid: zsh, program: "/opt/homebrew/bin/node", args: ["/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/cli.js", "-p", "go"], cwd: ATLAS_DIR, start: T + 1000 });
  w.store.pin(S1, { pid: node, start: w.m.table.get(node).start }, true, T);
  // The poll finds a level at a time, and can't tell what Node runs.
  await w.poll(T + 1500);
  await w.poll(T + 2000);
  // The session starts a Codex of its own before the look reads Node's
  // arguments: it waits, and is the session's, not Grok Bot's.
  w.m.add({ ppid: node, program: "/opt/homebrew/bin/codex", cwd: ATLAS_DIR, start: T + 2200 });
  await w.poll(T + 2500);
  await w.look(T + 3000);
  assert.deepEqual(events(w.snap(), "spawned").map((e) => [e.target, e.via, e.detail]), [[S1, "attributed", "Started a Claude Code session in atlas-api"]]);
  await w.look(T + 3000 + 2 * MIN);
  assert.deepEqual(events(w.snap(), "spawned").map((e) => e.target), [S1], "the session's own Codex isn't one Grok Bot started");
  assert.ok(!JSON.stringify(w.actors.events({ since: 0 })).includes("cli.js"), "nor are its arguments kept");
});

test("its app's helpers aren't commands, and its files are never read beyond whether the app is there and its version", async () => {
  const w = machine();
  await w.look();
  w.m.add({ ppid: w.app, program: `${APP}/Contents/Frameworks/Grok Bot Helper (GPU).app/Contents/MacOS/Grok Bot Helper (GPU)` });
  await w.poll(T + 1000);
  await w.look(T + 2000);
  assert.equal(w.snap().presence, "idle");
  assert.ok(w.asked.every((p) => p === APP || p === PLIST), "the app's own path and its Info.plist, and nothing else of its");
  const st = w.observer.status();
  assert.deepEqual([st.product, st.name, st.installed, st.sources.map((s) => [s.name, s.state])], ["grok-bot", "Grok Bot", true, [["processes", "ok"], ["app", "ok"], ["app's version", "ok"]]]);
});

test("its version: CFBundleShortVersionString from its Info.plist, beside its bundle id, read again only when the file changes", async () => {
  const w = machine({ running: false });
  await w.look();
  assert.deepEqual(w.snap().product, { id: "grok-bot", vendor: "SpaceXAI", version: "0.70.4" });
  assert.ok(!JSON.stringify([w.actors.list(), w.actors.events({ since: 0 }), w.observer.status()]).includes("com.anysphere.sand"), "its id says whose it is, and isn't shown");
  assert.ok(!JSON.stringify([w.actors.list(), w.observer.status()]).includes(SECRET));
  await w.look(T + 5000);
  assert.equal(w.read.length, 1, "the same file isn't read twice");
  // An update writes a new one.
  Object.assign(w.info, { text: infoPlist("0.71.0-pre"), mtimeMs: T + 6000 });
  await w.look(T + 10e3);
  assert.deepEqual([w.snap().product.version, w.read.length], ["0.71.0-pre", 2]);
  // A value that isn't a version isn't shown, and the one before stands.
  Object.assign(w.info, { text: infoPlist("<![CDATA[a whole sentence]]>"), mtimeMs: T + 11e3 });
  await w.look(T + 15e3);
  Object.assign(w.info, { text: infoPlist("not a version at all"), mtimeMs: T + 16e3 });
  await w.look(T + 20e3);
  assert.equal(w.snap().product.version, "0.71.0-pre");
  assert.deepEqual(w.observer.status().sources.at(-1), { name: "app's version", file: PLIST, state: "unreadable" });
  // Mid-update, with no Info.plist for a moment.
  w.info.text = null;
  await w.look(T + 25e3);
  assert.deepEqual([w.snap().product.version, w.observer.status().sources.at(-1).state], ["0.71.0-pre", "missing"]);
});

test("an Info.plist written as binary: its version and bundle id through plutil, those keys alone", async () => {
  const w = machine({ running: false, plist: "bplist00 made-up bytes" });
  const asked = [];
  const keys = { CFBundleShortVersionString: "0.70.4", CFBundleIdentifier: "com.anysphere.sand" };
  const observer = new GrokBotObserver({ actors: w.actors, procs: w.procs, home: "/Users/nova", apps: [APP], files: w.files, binary: async (file, key) => (asked.push([file, key]), keys[key]), clock: w.clock });
  await observer.tick(T);
  assert.deepEqual([w.snap().product.version, asked], ["0.70.4", [[PLIST, "CFBundleShortVersionString"], [PLIST, "CFBundleIdentifier"]]]);
  const odd = machine({ running: false, plist: "bplist00 made-up bytes" });
  const quiet = new GrokBotObserver({ actors: odd.actors, procs: odd.procs, home: "/Users/nova", apps: [APP], files: odd.files, binary: async (file, key) => (key === "CFBundleIdentifier" ? keys[key] : `${SECRET} and more`), clock: odd.clock });
  await quiet.tick(T);
  assert.equal(odd.snap().product.version, null, "only what reads as a version");
});

test("another bundle named Grok Bot.app isn't Grok Bot's: no actor, though it's there", async () => {
  const w = machine({ running: false, plist: infoPlist().replace("com.anysphere.sand", "com.example.not-grok") });
  await w.look();
  assert.equal(w.actors.has(ID), false);
  assert.deepEqual(w.observer.status().sources.map((s) => [s.name, s.state]), [["processes", "ok"], ["app", "other"], ["app's version", "ok"]]);
});

test("while its app runs, what it runs is polled every 3 seconds, by pgrep alone", async () => {
  const w = machine();
  const clock = fakeClock(T);
  const observer = new GrokBotObserver({ actors: w.actors, procs: w.procs, home: "/Users/nova", apps: [APP], files: w.files, alive: (pid) => w.m.table.has(pid), every: null, clock });
  await observer.tick(T);
  await observer.start();
  const polls = () => w.m.asked.filter((a) => a[0] === "pgrep").length;
  const tables = () => w.m.asked.filter((a) => a[0] === "ps" && a[1] === "-axo").length;
  const [before, read] = [polls(), tables()];
  await clock.advance(9000);
  observer.stop();
  assert.equal(POLL_MS, 3000);
  assert.deepEqual([polls() - before, tables() - read], [3, 0]);
});

test("its observer off: nothing read, and nothing polled", async () => {
  const w = machine();
  let on = true;
  const observer = new GrokBotObserver({ actors: w.actors, procs: w.procs, home: "/Users/nova", apps: [APP], files: w.files, alive: (pid) => w.m.table.has(pid), observing: () => on, clock: w.clock });
  await observer.tick();
  assert.equal(w.actors.has(ID), true);
  on = false;
  const before = w.m.asked.length;
  await observer.tick(T + 5000);
  await observer.poll(T + 6000);
  assert.equal(w.m.asked.length, before);
});

test("commands already running at its first look count too", async () => {
  const w = machine();
  w.m.add({ ppid: w.helper, program: "/bin/zsh", cwd: ATLAS_DIR, start: T - 1000 });
  await w.look();
  assert.equal(w.snap().presence, "working");
  await w.look(Math.floor(T / MIN) * MIN + MIN + 20e3);
  assert.deepEqual(events(w.snap(), "ran").map((e) => e.detail), ["Ran a command in atlas-api"]);
});
