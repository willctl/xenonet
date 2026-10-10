// Hermes Agent's observer (core/actors/hermes.js) on a made-up install
// (test/fixtures/hermes.js), a made-up process table and a made-up map:
// presence by the interface's table, its status in XenoNet's words, events
// from facts, the sessions it started, profiles, and none of what Hermes
// keeps beside its metadata.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Actors } from "../core/actors/index.js";
import { HermesObserver, doingOf, endingOf, profileSlug } from "../core/actors/hermes.js";
import { EngineStore } from "../core/engine/store.js";
import { createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeMap, NODES, ATLAS, LEDGER, S1, ATLAS_DIR, LEDGER_DIR } from "./fixtures/actor-map.js";
import { SECRET, hermesRoot, setLedger, setGateway, setTicker, setProcesses, setJobs, stateDb, profileHome } from "./fixtures/hermes.js";
import { drift } from "../core/drift.js";

const dirs = [];
const databases = [];
after(() => databases.forEach((db) => { if (db.db.isOpen) db.close(); }));
after(() => dirs.forEach((d) => fs.rmSync(d, { recursive: true, force: true })));
const T = Date.parse("2026-10-05T12:00:20Z");
const MIN = 60e3;
const ID = "actor:hermes";

// A machine with Hermes installed under a made-up home: Hermes.app and its
// desktop backend running (unless `running` is false), the backend in the
// spawn ledger and ticking, and the map's two projects.
function machine({ running = true, nodes = NODES() } = {}) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-hermes-home-"));
  dirs.push(home);
  const root = hermesRoot(path.join(home, ".hermes"));
  const clock = { t: T, now: () => clock.t, setTimeout, clearTimeout };
  const m = fakeProcesses({ clock });
  const store = new EngineStore(":memory:");
  const actors = new Actors({ now: () => clock.t, map: fakeMap(nodes), store });
  const procs = createProcs({ platform: "darwin", exec: m.exec, now: () => clock.t });
  const appPath = `${root}/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app/Contents/MacOS/Hermes`;
  const python = `${root}/tools/python-3.14.7/bin/python3`;
  let app = null;
  let host = null;
  if (running) {
    app = m.add({ ppid: 1, program: appPath, start: T - 3600e3 });
    host = m.add({ ppid: app, program: python, args: ["-I", "-c", "import hermes_cli.main", "serve", "--host", "127.0.0.1", "--port", "0"], start: T - 3500e3 });
    setLedger(root, [{ purpose: "serve", pid: host, createTime: m.table.get(host).start / 1000, port: 51234, spawner_pid: app }]);
    setTicker(root, { at: T - 10e3, pid: host });
  }
  const observer = new HermesObserver({ actors, procs, home, env: {}, clock });
  const db = stateDb(path.join(root, "state.db"));
  databases.push(db);
  const look = async (at = clock.t) => {
    clock.t = at;
    await observer.tick(at);
    actors.tick(at);
  };
  return { home, root, clock, m, store, actors, procs, observer, db, app, host, python, appPath, look, snap: () => actors.snapshot(ID) };
}
const details = (s) => s.recent.filter((e) => e.kind !== "presence" && e.kind !== "status").map((e) => e.detail);

test("Hermes's words: what an activity stamp says it's doing, and how a session ended", () => {
  assert.equal(doingOf("executing tool: terminal"), "Running a command");
  assert.equal(doingOf("tool completed: patch (2.1s)"), "Editing files");
  assert.equal(doingOf("sequential tool running (12s): browser_navigate"), "Browsing the web");
  assert.equal(doingOf("executing 3 tools concurrently: read_file, search_files"), "Reading files");
  assert.equal(doingOf("waiting for stream response (12s, thinking)"), "Thinking");
  assert.equal(doingOf("context compression in progress"), "Tidying its context");
  assert.equal(doingOf(`a free line with ${SECRET}`), null, "anything else isn't shown");
  assert.equal(doingOf("executing tool: some_mcp_tool"), null);
  assert.deepEqual(["cli_close", "tui_close", "compression", "provider_unreachable", "crash", "new_session"].map(endingOf), ["finished", "finished", null, "failed", "failed", "finished"]);
});

test("its desktop backend running with nothing to do: idle, observed and attributed, at its version, with what it can do", async () => {
  const w = machine();
  await w.look();
  const s = w.snap();
  assert.deepEqual([s.name, s.kind, s.product, s.via, s.presence, s.status, s.heartbeatMs, s.identity], ["Hermes", "agent", { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }, ["observed", "attributed"], "idle", null, 60000, null]);
  assert.deepEqual(s.capabilities, ["edit-repos", "run-commands", "schedule", "browse"]);
  // Its skills for Claude Code and Codex, and the computer-use driver, add theirs.
  fs.mkdirSync(path.join(w.root, "skills", "autonomous-ai-agents"), { recursive: true });
  fs.mkdirSync(path.join(w.root, "tools", "cua-driver-0.21.0"), { recursive: true });
  w.observer.profilesAt = -Infinity;
  await w.look(T + 5000);
  assert.deepEqual(w.snap().capabilities, ["spawn-sessions", "edit-repos", "run-commands", "schedule", "browse", "use-computer"]);
  assert.equal(w.observer.health().state, "running");
  assert.deepEqual(w.observer.status().recognized, { expected: 1, found: 1, purposes: [] }, "the process check knows its backend as Hermes's");
});

test("at work: working, saying what it does in XenoNet's words, and what it did, a minute at a time", async () => {
  const w = machine();
  w.db.session({ id: "s1", source: "desktop", cwd: ATLAS_DIR, repo: ATLAS_DIR, startedAt: T - 5 * MIN, activityAt: T - 5000, activity: "executing tool: terminal" });
  w.db.message({ session: "s1", at: T - 4000, calls: [{ name: "write_file", args: { path: "src/app.ts" } }, { name: "patch", args: { path: "/etc/hosts" } }] });
  for (const k of [3000, 2000, 1000]) w.db.message({ session: "s1", role: "tool", tool: "terminal", at: T - k });
  await w.look();
  let s = w.snap();
  assert.deepEqual([s.presence, s.status], ["working", "Running a command in atlas-api"]);
  assert.deepEqual(s.targets.map((t) => [t.id, t.relation]), [[ATLAS, "works-on"]]);
  // Its minute is over: one event for its commands, one for its edits; the
  // file outside every place XenoNet knows is left out.
  await w.look(Math.floor(T / MIN) * MIN + MIN + 20e3);
  s = w.snap();
  assert.deepEqual(details(s).sort(), ["Edited app.ts in atlas-api", "Ran 3 commands in atlas-api"]);
  assert.ok(s.recent.every((e) => e.kind === "presence" || e.kind === "status" || e.via === "observed"));
  // The file it wrote holds it for agents, as an agent's edit does.
  assert.deepEqual(w.actors.engineView(ID).edits.map((e) => e.path), [`${ATLAS_DIR}/src/app.ts`]);
  // Nothing new for two minutes: idle again.
  await w.look(T + 3 * MIN);
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", null]);
  assert.ok(!JSON.stringify([w.actors.list(), w.actors.events({ since: 0 })]).includes(SECRET), "no content, title, argument or name of Hermes's");
});

test("sessions that end: finished or failed, once each; a subagent's and a compaction's aren't news", async () => {
  const w = machine();
  w.db.session({ id: "s1", source: "tui", cwd: ATLAS_DIR, startedAt: T - 9 * MIN, endedAt: T - 2 * MIN, endReason: "tui_close" });
  w.db.session({ id: "s2", source: "cron", cwd: LEDGER_DIR, startedAt: T - 9 * MIN, endedAt: T - MIN, endReason: "provider_unreachable" });
  w.db.session({ id: "s3", source: "subagent", parent: "s1", cwd: ATLAS_DIR, startedAt: T - 8 * MIN, endedAt: T - 3 * MIN, endReason: "done" });
  w.db.session({ id: "s4", source: "tui", cwd: ATLAS_DIR, startedAt: T - 8 * MIN, endedAt: T - 3 * MIN, endReason: "compression" });
  await w.look();
  await w.look(T + 5000);
  assert.deepEqual(details(w.snap()), ["Finished a session in atlas-api", "A session failed in ledger"]);
  // After a restart, the same endings aren't told again.
  const again = new HermesObserver({ actors: w.actors, procs: w.procs, home: w.home, env: {}, clock: w.clock });
  await again.tick(T + 10e3);
  assert.equal(details(w.snap()).length, 2);
});

test("a reply in a session from a messaging platform, by the platform's name alone; its tool calls aren't replies", async () => {
  const w = machine();
  w.db.session({ id: "tg", source: "telegram", startedAt: T - MIN, activityAt: T - 5000 });
  w.db.message({ session: "tg", role: "user", at: T - 6000 });
  w.db.message({ session: "tg", at: T - 5500, calls: [{ name: "web_search", args: { query: SECRET } }] });
  w.db.message({ session: "tg", at: T - 5000 });
  await w.look();
  await w.look(Math.floor(T / MIN) * MIN + MIN + 20e3);
  assert.deepEqual(details(w.snap()), ["Sent a message on Telegram"]);
});

test("its gateway: agents at work, a heartbeat gone quiet, a state that isn't running, and platforms that can message people", async () => {
  const w = machine({ running: false });
  const gw = w.m.add({ ppid: 1, program: w.python, args: ["-m", "hermes_cli.main", "gateway", "run"], start: T - 3600e3 });
  const startTime = (w.m.table.get(gw).start / 1000) * 100;
  setGateway(w.root, { state: "running", pid: gw, startTime, updatedAt: T - 20e3, activeAgents: 1, platforms: { telegram: "connected" } });
  await w.look();
  assert.deepEqual([w.snap().presence, w.snap().status], ["working", "Working through its gateway"]);
  assert.ok(w.snap().capabilities.includes("message-people"));
  setGateway(w.root, { state: "running", pid: gw, startTime, updatedAt: T - 3 * MIN, activeAgents: 0, platforms: { telegram: "connected" } });
  await w.look(T + 1000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["away", "Its gateway has gone quiet"]);
  setGateway(w.root, { state: "degraded", pid: gw, startTime, updatedAt: T + 1000, platforms: { telegram: "retrying" } });
  await w.look(T + 2000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", "Its gateway is degraded"]);
  assert.ok(!w.snap().capabilities.includes("message-people"));
  // A record naming a process that started at another time is another
  // process's pid, used again: no gateway runs.
  setGateway(w.root, { state: "running", pid: gw, startTime: startTime + 99_000, updatedAt: T + 3000 });
  await w.look(T + 3000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", null], "the process still runs as Hermes's, with no gateway XenoNet can vouch for");
  w.m.remove(gw);
  setGateway(w.root, { state: "startup_failed", pid: gw, startTime, updatedAt: T + 3000 });
  await w.look(T + 4000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["offline", "Its gateway failed to start"]);
});

test("a gateway gone quiet while its process runs is away, whatever it last said of agents at work; its sessions' own activity still counts", async () => {
  const w = machine({ running: false });
  const gw = w.m.add({ ppid: 1, program: w.python, args: ["-m", "hermes_cli.main", "gateway", "run"], start: T - 3600e3 });
  const startTime = (w.m.table.get(gw).start / 1000) * 100;
  // Hung mid-task: its file still says running, with an agent at work, and
  // hasn't been written for ten minutes.
  setGateway(w.root, { state: "running", pid: gw, startTime, updatedAt: T - 10 * MIN, activeAgents: 1 });
  await w.look();
  assert.deepEqual([w.snap().presence, w.snap().status], ["away", "Its gateway has gone quiet"]);
  // A session of its own at work says more than the gateway's file.
  w.db.session({ id: "s1", source: "telegram", cwd: ATLAS_DIR, repo: ATLAS_DIR, startedAt: T - MIN, activityAt: T + 4000, activity: "executing tool: terminal" });
  await w.look(T + 5000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["working", "Running a command in atlas-api"]);
});

test("Hermes.app alone, nothing at all, and Hermes not installed", async () => {
  const w = machine({ running: false });
  const app = w.m.add({ ppid: 1, program: w.appPath });
  await w.look();
  assert.deepEqual([w.snap().presence, w.snap().status], ["idle", "Hermes.app runs with no backend on this Mac"]);
  w.m.remove(app);
  await w.look(T + 5000);
  assert.deepEqual([w.snap().presence, w.snap().status], ["offline", null]);
  // Not installed: no actor.
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-no-hermes-"));
  dirs.push(home);
  const actors = new Actors({ map: fakeMap() });
  const none = new HermesObserver({ actors, procs: w.procs, home, env: {} });
  await none.tick();
  assert.equal(actors.has(ID), false);
  assert.equal(none.status().installed, false);
});

test("installed, with nothing of it running for over a week: still listed, offline", async () => {
  const w = machine({ running: false });
  await w.look();
  await w.look(T + 8 * 24 * 3600e3);
  assert.deepEqual(w.actors.list().actors.map((a) => [a.id, a.presence]), [["actor:you", "unknown"], [ID, "offline"]]);
  // Taken off this Mac, it goes a week after it was last heard of.
  fs.rmSync(path.join(w.root, "hermes-agent"), { recursive: true, force: true });
  w.db.close();
  fs.rmSync(path.join(w.root, "state.db"), { force: true });
  await w.look(T + 8 * 24 * 3600e3 + 5000);
  assert.deepEqual(w.actors.list().actors.map((a) => a.id), ["actor:you"]);
});

test("the sessions it started: by ancestry, by its background processes, by Codex's originator and in its scratch worktrees", async () => {
  const nodes = NODES();
  const S3 = "codex:0190d3e0-0000-7000-8000-00000000aa03";
  const S4 = "claude:0190d3e0-0000-7000-8000-00000000aa04";
  const S5 = "codex:0190d3e0-0000-7000-8000-00000000aa05";
  nodes[S3] = { kind: "session", app: "codex", home: LEDGER, cwd: LEDGER_DIR, createdAt: T - MIN, originator: "hermes" };
  nodes[S5] = { kind: "session", app: "codex", home: LEDGER, cwd: LEDGER_DIR, createdAt: T - MIN, originator: "codex_exec" };
  const w = machine({ nodes });
  nodes[S4] = { kind: "session", app: "claude", home: ATLAS, cwd: path.join(w.root, "cache", "scratch", "tidy"), createdAt: T - MIN };
  // claude -p under its backend, pinned to its session by Claude Code's hooks.
  const bash = w.m.add({ ppid: w.host, program: "/bin/bash", args: ["-c", "claude -p go"], cwd: ATLAS_DIR });
  const claude = w.m.add({ ppid: bash, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T - 5000 });
  w.store.pin(S1, { pid: claude, start: w.m.table.get(claude).start }, true, T);
  // A child of that session's isn't Hermes's.
  w.m.add({ ppid: claude, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR });
  // codex exec in the background, its host gone, its session not known.
  const bg = w.m.add({ ppid: 1, program: "/bin/bash", args: ["-c", "codex exec x"], cwd: LEDGER_DIR, start: T - 4000 });
  const codex = w.m.add({ ppid: bg, program: "/opt/homebrew/bin/codex", cwd: LEDGER_DIR, start: T - 4000 });
  setProcesses(w.root, [{ pid: bg, startTime: (w.m.table.get(bg).start / 1000) * 100, cwd: LEDGER_DIR, startedAt: T - 4000, session: "s1" }]);
  // Yours, in your own terminal.
  w.m.add({ ppid: 1, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR });
  await w.look();
  let s = w.snap();
  assert.deepEqual(s.recent.filter((e) => e.kind === "spawned").map((e) => [e.target, e.via]).sort(), [[S1, "attributed"], [S3, "attributed"], [S4, "attributed"]].sort());
  // Two minutes on, the background codex's session still isn't known: its
  // place counts.
  await w.look(T + 2 * MIN);
  s = w.snap();
  const spawned = s.recent.filter((e) => e.kind === "spawned");
  assert.equal(spawned.length, 4);
  assert.deepEqual([spawned.at(-1).target, spawned.at(-1).detail], [LEDGER, "Started a Codex session in ledger"]);
  assert.ok(!spawned.some((e) => e.target === S5), "a Codex thread of another originator isn't Hermes's");
  assert.ok(s.targets.some((t) => t.id === S1 && t.relation === "spawned"));
  await w.look(T + 3 * MIN);
  assert.equal(w.snap().recent.filter((e) => e.kind === "spawned").length, 4, "each once");
  assert.ok(codex);
});

test("a named profile is an actor of its own once something of it runs", async () => {
  const w = machine();
  const work = profileHome(w.root, "work");
  fs.mkdirSync(work, { recursive: true });
  await w.look();
  assert.equal(w.actors.has("actor:hermes-work"), false, "a profile nothing of which has run isn't one yet");
  const gw = w.m.add({ ppid: 1, program: w.python, args: ["-m", "hermes_cli.main", "--profile", "work", "gateway", "run"] });
  setGateway(work, { state: "running", pid: gw, updatedAt: T, activeAgents: 0 });
  w.observer.profilesAt = -Infinity;
  await w.look(T + 5000);
  const s = w.actors.snapshot("actor:hermes-work");
  assert.deepEqual([s.name, s.presence, s.product.id], ["Hermes (work)", "idle", "hermes-agent"]);
  assert.equal(w.snap().presence, "idle", "and the default one goes on as it was");
});

test("profiles whose names differ only in punctuation or case, or have no letters of a-z, are actors of their own", async () => {
  assert.deepEqual(["work", "Work", "my-work", "my_work", "工作", "работа"].map(profileSlug), ["hermes-work", "hermes-work-104a", "hermes-my-work", "hermes-my-work-1718", "hermes-actor-bc62", "hermes-actor-4913"]);
  const w = machine();
  for (const name of ["my-work", "my_work"]) fs.mkdirSync(profileHome(w.root, name), { recursive: true });
  // my_work's gateway runs; my-work has never run.
  const gw = w.m.add({ ppid: 1, program: w.python, args: ["-m", "hermes_cli.main", "--profile", "my_work", "gateway", "run"] });
  w.observer.profilesAt = -Infinity;
  for (let k = 0; k < 6; k++) {
    setGateway(profileHome(w.root, "my_work"), { state: "running", pid: gw, updatedAt: T + k * 5000, activeAgents: 0 });
    await w.look(T + k * 5000);
  }
  assert.deepEqual(w.actors.ids().filter((id) => id.startsWith("actor:hermes-")), ["actor:hermes-my-work-1718"], "my-work isn't an actor until something of it runs");
  const s = w.actors.snapshot("actor:hermes-my-work-1718");
  assert.deepEqual([s.name, s.presence], ["Hermes (my_work)", "idle"]);
  assert.deepEqual(s.recent.filter((e) => e.kind === "presence").map((e) => e.detail), ["Came online"], "and its presence holds still");
});

test("registered through its plugin later: its events are its own to report, while XenoNet keeps presence and attribution", async () => {
  const w = machine();
  await w.look();
  w.actors.upsert({ ...w.actors.info(ID), id: ID, name: "Hermes", via: ["observed", "attributed", "registered"], capabilities: ["run-commands"], allowedAt: T });
  w.db.session({ id: "s1", source: "tui", cwd: ATLAS_DIR, startedAt: T - MIN, activityAt: T - 1000, activity: "executing tool: terminal" });
  w.db.message({ session: "s1", role: "tool", tool: "terminal", at: T - 1000 });
  await w.look(T + 1000);
  await w.look(Math.floor(T / MIN) * MIN + 2 * MIN + 20e3);
  assert.deepEqual(details(w.snap()), [], "its plugin says what it ran");
  assert.deepEqual(w.actors.info(ID).capabilities, ["run-commands"], "what you allowed stands");
});

test("its observer off: nothing read, not even the process table", async () => {
  const w = machine();
  let on = false;
  const observer = new HermesObserver({ actors: w.actors, procs: w.procs, home: w.home, env: {}, observing: () => on, clock: w.clock });
  const before = w.m.asked.length;
  await observer.tick();
  assert.equal(w.m.asked.length, before);
  assert.equal(w.actors.has(ID), false);
  on = true;
  await observer.tick();
  assert.equal(w.actors.has(ID), true);
});

test("for the doctor: each source and whether XenoNet could read it, and a backend the process check wouldn't know", async () => {
  const w = machine();
  setJobs(w.root, [{ id: "j1", last_status: "ok", last_run_at: new Date(T - MIN).toISOString(), workdir: ATLAS_DIR }]);
  await w.look();
  const st = w.observer.status();
  assert.deepEqual([st.product, st.name, st.installed], ["hermes-agent", "Hermes Agent", true]);
  const byName = Object.fromEntries(st.sources.map((s) => [s.name, s]));
  assert.deepEqual([byName["build stamp"].state, byName["spawn ledger"].state, byName["session database"].state, byName["gateway's state"].state, byName["scheduled jobs"].state], ["ok", "ok", "ok", "missing", "ok"]);
  assert.match(byName["session database"].file, /^~[\\/]\.hermes[\\/]state\.db$/, "paths under the home folder with ~");
  // The job's run is an event, and its folder something Hermes watches.
  assert.deepEqual(details(w.snap()), ["A scheduled job ran in atlas-api"]);
  assert.ok(w.snap().targets.some((t) => t.id === ATLAS && t.relation === "watches"));
  // A backend the ledger lists that the process table doesn't know as
  // Hermes's: a token of Hermes's would be refused from under it.
  const odd = w.m.add({ ppid: 1, program: "/opt/homebrew/bin/python3", args: ["-m", "hermes_cli.main", "serve"] });
  setLedger(w.root, [{ purpose: "serve", pid: w.host, createTime: w.m.table.get(w.host).start / 1000 }, { purpose: "gateway", pid: odd, createTime: w.m.table.get(odd).start / 1000 }]);
  await w.look(T + 5000);
  assert.deepEqual(w.observer.status().recognized, { expected: 2, found: 1, purposes: ["gateway"] });
  assert.deepEqual(drift.list().filter((r) => r.source.startsWith("nous/")).map((r) => r.kind), []);
});

test("a profile a process names is a plain name, so none reaches outside Hermes's folder", async () => {
  const w = machine();
  const gw = w.m.add({ ppid: 1, program: w.python, args: ["-m", "hermes_cli.main", "gateway", "run"] });
  setLedger(w.root, [{ purpose: "serve", pid: w.host, createTime: w.m.table.get(w.host).start / 1000 }, { purpose: "gateway", pid: gw, createTime: w.m.table.get(gw).start / 1000, profile: "../../elsewhere" }]);
  await w.look();
  assert.deepEqual(w.actors.ids().filter((id) => id.startsWith("actor:hermes")), [ID], "its process counts as the default profile's");
  assert.ok(!w.observer.status().sources.some((x) => /elsewhere/.test(x.file ?? "")));
});

test("a session it started that isn't on the map yet is still named by its app", async () => {
  const w = machine();
  const bash = w.m.add({ ppid: w.host, program: "/bin/bash", cwd: ATLAS_DIR });
  const claude = w.m.add({ ppid: bash, program: "/Users/nova/.local/bin/claude", cwd: ATLAS_DIR, start: T - 2000 });
  const fresh = "claude:0190d3e0-0000-7000-8000-00000000ff01";
  w.store.pin(fresh, { pid: claude, start: w.m.table.get(claude).start }, true, T);
  await w.look();
  assert.deepEqual(w.snap().recent.filter((e) => e.kind === "spawned").map((e) => [e.target, e.detail]), [[fresh, "Started a Claude Code session"]]);
});

test("what it reads of Hermes's files: the sources hermes.md allows, and never its secrets, logs, memories or settings", async () => {
  const w = machine();
  setGateway(w.root, { state: "running", pid: w.host, updatedAt: T, platforms: { telegram: "connected" } });
  setProcesses(w.root, [{ pid: w.host, cwd: ATLAS_DIR }]);
  setJobs(w.root, [{ id: "j1", last_status: "ok" }]);
  fs.mkdirSync(path.join(w.root, "logs"), { recursive: true });
  fs.writeFileSync(path.join(w.root, "logs", "agent.log"), SECRET);
  fs.writeFileSync(path.join(w.root, "config.yaml"), `api_key: ${SECRET}`);
  const read = [];
  const files = new Proxy(fs, { get: (t, k) => (k === "readFileSync" ? (f, ...a) => (read.push(path.relative(w.root, String(f)).split(path.sep).join("/")), t.readFileSync(f, ...a)) : t[k]) });
  const observer = new HermesObserver({ actors: w.actors, procs: w.procs, home: w.home, env: {}, files, clock: w.clock });
  await observer.tick(T);
  assert.deepEqual([...new Set(read)].sort(), ["cron/jobs.json", "cron/ticker_heartbeat", "cron/ticker_last_success", "gateway_state.json", "hermes-agent/install-stamp.json", "processes.json", "spawn-ledger.json"]);
});
