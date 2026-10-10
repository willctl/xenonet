// Reading Hermes Agent's state (core/actors/hermes-files.js) from a made-up
// install (test/fixtures/hermes.js): each source for metadata only, against
// its vendor schema, with drift for what doesn't fit, and none of the
// content, secrets or names Hermes keeps beside it.
import { test, after, beforeEach } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { drift } from "../core/drift.js";
import { msOf, sameStart, hermesRoot, listProfiles, installed, readStamp, readLedger, readGateway, readTicker, readProcesses, readJobs, readStateDb } from "../core/actors/hermes-files.js";
import { SECRET, hermesRoot as makeRoot, setLedger, setGateway, setTicker, setProcesses, setJobs, stateDb, profileHome } from "./fixtures/hermes.js";

const dirs = [];
after(() => dirs.forEach((d) => fs.rmSync(d, { recursive: true, force: true })));
const tmp = () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-hermes-"));
  dirs.push(d);
  return d;
};
beforeEach(() => drift.clear());
const kinds = (source) => drift.list().filter((r) => r.source === `nous/hermes-agent/${source}`).map((r) => [r.kind, r.path]);
const T = Date.parse("2026-10-05T12:00:00Z");
const MIN = 60e3;

test("times and starts as Hermes writes them", () => {
  assert.equal(msOf(1791230000.25), 1791230000250);
  assert.equal(msOf("2026-10-05T12:00:00.000000+00:00"), T);
  assert.deepEqual([msOf(null), msOf("soon"), msOf(-1), msOf({})], [null, null, null, null]);
  // psutil's seconds in the ledger, centiseconds in the gateway's record,
  // against ps's start to the second.
  assert.ok(sameStart(1791230000.37, 1791230000000));
  assert.ok(sameStart(179123000037, 1791230000000));
  assert.ok(!sameStart(1791230009.0, 1791230000000), "nine seconds off is another process");
  assert.ok(!sameStart(null, 1791230000000));
});

test("the install: its root, its profiles, whether it's there, and its version", () => {
  const home = tmp();
  assert.equal(hermesRoot(home, {}), path.join(home, ".hermes"));
  assert.equal(hermesRoot(home, { HERMES_HOME: "/opt/hermes" }), path.resolve("/opt/hermes"));
  const root = path.join(home, ".hermes");
  assert.equal(installed(root), false);
  assert.deepEqual(readStamp(root), { state: "missing", version: null });
  makeRoot(root);
  fs.mkdirSync(path.join(root, "profiles", "work"), { recursive: true });
  fs.mkdirSync(path.join(root, "profiles", ".trash"), { recursive: true });
  assert.equal(installed(root), true);
  assert.deepEqual(readStamp(root), { state: "ok", version: "0.21.5" });
  assert.deepEqual(listProfiles(root).map((p) => [p.name, p.home]), [["default", root], ["work", path.join(root, "profiles", "work")]]);
  // A stamp in a format the schema wasn't made for is drift.
  fs.writeFileSync(path.join(root, "hermes-agent", "install-stamp.json"), JSON.stringify({ schemaVersion: 3, baseVersion: "0.22.0" }));
  assert.deepEqual(readStamp(root), { state: "ok", version: "0.22.0" });
  assert.deepEqual(kinds("install-stamp"), [["new-version", null]]);
});

test("the spawn ledger: Hermes's processes, never their argv", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"), {
    ledger: [
      { purpose: "serve", pid: 4242, createTime: 1791229000.12, port: 51234, profile: "default", spawner_pid: 4100 },
      { purpose: "gateway", pid: 4300, createTime: 1791229100, profile: "work" },
    ],
  });
  const r = readLedger(root, { version: "0.21.5" });
  assert.deepEqual(r, {
    state: "ok",
    entries: [
      { pid: 4242, createTime: 1791229000.12, purpose: "serve", port: 51234, profile: "default", spawnerPid: 4100 },
      { pid: 4300, createTime: 1791229100, purpose: "gateway", port: null, profile: "work", spawnerPid: null },
    ],
  });
  assert.ok(!JSON.stringify(r).includes(SECRET));
  assert.deepEqual(kinds("spawn-ledger"), []);
  setLedger(root, [{ purpose: "watchdog", pid: 1 }]);
  assert.equal(readLedger(root, { version: "0.21.5" }).entries[0].purpose, "watchdog", "kept, for what XenoNet can read of it");
  assert.deepEqual(kinds("spawn-ledger"), [["new-variant", "[].purpose"]]);
  assert.deepEqual(readLedger(path.join(tmp(), "none")), { state: "missing", entries: [] });
});

test("the gateway's state: its state, heartbeat, agents and platforms, never its error texts or argv", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  setGateway(root, { state: "running", pid: 4300, startTime: 179122900012, updatedAt: T, activeAgents: 2, platforms: { telegram: "connected", discord: "retrying" } });
  const r = readGateway(root, { version: "0.21.5" });
  assert.deepEqual(r, { state: "ok", gateway: { state: "running", pid: 4300, startTime: 179122900012, updatedAt: T, activeAgents: 2, exitReason: null, platforms: [{ name: "telegram", state: "connected" }, { name: "discord", state: "retrying" }] } });
  assert.ok(!JSON.stringify(r).includes(SECRET));
  fs.writeFileSync(path.join(root, "gateway_state.json"), "{ half written");
  assert.deepEqual(readGateway(root, { version: "0.21.5" }), { state: "unreadable", gateway: null });
  assert.deepEqual(kinds("gateway-state"), [["unparsable", null]]);
  fs.rmSync(path.join(root, "gateway_state.json"));
  assert.deepEqual(readGateway(root), { state: "missing", gateway: null });
});

test("the scheduler's ticker: when it last beat, and whose it is", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  assert.deepEqual(readTicker(root), { state: "missing", ticker: null });
  setTicker(root, { at: T, pid: 4242, lastSuccess: T - MIN });
  assert.deepEqual(readTicker(root, { version: "0.21.5" }), { state: "ok", ticker: { at: T, pid: 4242, lastSuccess: T - MIN } });
  fs.writeFileSync(path.join(root, "cron", "ticker_heartbeat"), "soon 4242");
  assert.deepEqual(readTicker(root, { version: "0.21.5" }), { state: "unreadable", ticker: null });
  assert.deepEqual(kinds("cron-ticker"), [["unparsable", null]]);
});

test("background processes and scheduled jobs, never their commands, names or prompts", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  setProcesses(root, [{ pid: 5100, startTime: 179123000000, cwd: "/Users/ada/atlas", startedAt: T, session: "20261005_120000_a1b2c3" }, { pid: 5200, pid_scope: "sandbox" }]);
  const p = readProcesses(root, { version: "0.21.5" });
  assert.deepEqual(p, { state: "ok", entries: [{ pid: 5100, startTime: 179123000000, cwd: "/Users/ada/atlas", startedAt: T, session: "20261005_120000_a1b2c3" }] }, "a pid inside a sandbox means nothing here");
  setJobs(root, [
    { id: "j1", last_status: "ok", last_run_at: "2026-10-05T03:00:00+00:00", next_run_at: "2026-10-06T03:00:00+00:00", workdir: "/Users/ada/atlas" },
    { id: "j2", last_status: "error", enabled: false, state: "paused" },
  ]);
  const j = readJobs(root, { version: "0.21.5" });
  assert.deepEqual(j, {
    state: "ok",
    jobs: [
      { id: "j1", status: "ok", state: "scheduled", enabled: true, workdir: "/Users/ada/atlas", lastRunAt: Date.parse("2026-10-05T03:00:00Z"), nextRunAt: Date.parse("2026-10-06T03:00:00Z") },
      { id: "j2", status: "error", state: "paused", enabled: false, workdir: null, lastRunAt: null, nextRunAt: null },
    ],
  });
  assert.ok(!JSON.stringify([p, j]).includes(SECRET));
  assert.deepEqual([...kinds("processes"), ...kinds("cron-jobs")], []);
});

// A database in the current layout: a TUI session at work in atlas, with a
// command, an edit (a relative path, an absolute one, one that isn't
// text), a broken tool_calls, an archived generation, and an old session.
function busyDb(file) {
  const db = stateDb(file);
  db.session({ id: "s-old", source: "cli", cwd: "/Users/ada/ledger", startedAt: T - 3 * 3600e3, endedAt: T - 2 * 3600e3, endReason: "cli_close" });
  db.message({ session: "s-old", role: "tool", tool: "terminal", at: T - 2 * 3600e3 });
  db.session({ id: "s1", source: "tui", cwd: "/Users/ada/atlas", repo: "/Users/ada/atlas", startedAt: T - 5 * MIN, activityAt: T - 10e3, activity: "executing tool: terminal" });
  db.message({ session: "s1", role: "user", at: T - 4 * MIN });
  db.message({ session: "s1", at: T - 3 * MIN, calls: [{ name: "write_file", args: { path: "src/app.ts" } }, { name: "terminal", args: { command: `echo ${SECRET}` } }] });
  db.message({ session: "s1", role: "tool", tool: "write_file", at: T - 3 * MIN + 1000 });
  db.message({ session: "s1", role: "tool", tool: "terminal", at: T - 3 * MIN + 2000 });
  db.message({ session: "s1", at: T - 2 * MIN, calls: [{ name: "patch", args: { path: "/Users/ada/atlas/README.md", old_string: SECRET, new_string: SECRET } }, { name: "write_file", args: { path: 42 } }] });
  db.message({ session: "s1", at: T - 2 * MIN, rawCalls: "not json" });
  db.message({ session: "s1", role: "tool", tool: "terminal", at: T - MIN, active: 0 });
  db.heartbeat({ pid: 4242, startedAt: T - 10 * MIN, at: T - 20e3 });
  return db;
}

test("the session database: what's new since the last read, by named columns, and paths taken out by SQLite", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  const file = path.join(root, "state.db");
  const db = busyDb(file);
  const first = readStateDb(file, { sinceMs: T - 10 * MIN });
  assert.equal(first.state, "ok");
  assert.equal(first.version, "31");
  assert.deepEqual(first.messages.map((m) => [m.session_id, m.role, m.tool_name]), [["s1", "user", null], ["s1", "assistant", null], ["s1", "tool", "write_file"], ["s1", "tool", "terminal"], ["s1", "assistant", null], ["s1", "assistant", null]], "the last ten minutes only, and not the archived generation");
  assert.deepEqual(first.toolPaths.map((t) => [t.tool, t.path]), [["write_file", "src/app.ts"], ["patch", "/Users/ada/atlas/README.md"], ["write_file", null]]);
  assert.deepEqual(first.sessions.map((s) => [s.id, s.source, s.cwd, s.git_repo_root, s.last_activity_description]), [["s1", "tui", "/Users/ada/atlas", "/Users/ada/atlas", "executing tool: terminal"]]);
  assert.deepEqual(first.heartbeats, [{ pid: 4242, started_at: (T - 10 * MIN) / 1000, last_heartbeat: (T - 20e3) / 1000, profile: "default" }]);
  assert.ok(!JSON.stringify(first).includes(SECRET), "no content, title, chat id, host or argument but the path");
  assert.deepEqual(kinds("state-db"), []);
  // The next read starts where this one stopped.
  const again = readStateDb(file, { afterId: first.lastId, sinceMs: T - 3600e3 });
  assert.deepEqual([again.messages.length, again.toolPaths.length], [0, 0]);
  db.message({ session: "s1", role: "tool", tool: "terminal", at: T + 5000 });
  db.update("s1", { endedAt: T + 6000, endReason: "tui_close" });
  const next = readStateDb(file, { afterId: first.lastId, sinceMs: T - 3600e3 });
  assert.deepEqual(next.messages.map((m) => m.tool_name), ["terminal"]);
  assert.deepEqual(next.sessions.map((s) => [s.id, s.ended_at, s.end_reason]), [["s1", (T + 6000) / 1000, "tui_close"]]);
  db.close();
});

test("the session database: a call in tool_calls that isn't an object is passed over, and the read goes on", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  const file = path.join(root, "state.db");
  const db = stateDb(file);
  db.session({ id: "s1", source: "tui", cwd: "/Users/ada/atlas", startedAt: T - 5 * MIN, activityAt: T - 10e3 });
  // A string where a call should be makes json_extract fail outright; a
  // number or a null only reads as nothing.
  db.message({ session: "s1", at: T - 3 * MIN, rawCalls: '["write_file"]' });
  db.message({ session: "s1", at: T - 3 * MIN, rawCalls: "[5, null, true]" });
  db.message({ session: "s1", at: T - 2 * MIN, rawCalls: JSON.stringify(["patch", { id: "call_0", type: "function", function: { name: "write_file", arguments: JSON.stringify({ path: "src/app.ts", content: SECRET }) } }]) });
  db.message({ session: "s1", role: "tool", tool: "write_file", at: T - 2 * MIN + 1000 });
  const r = readStateDb(file, { sinceMs: T - 10 * MIN });
  assert.equal(r.state, "ok");
  assert.deepEqual(r.toolPaths.map((t) => [t.tool, t.path]), [["write_file", "src/app.ts"]]);
  assert.deepEqual(r.messages.map((m) => m.tool_name), [null, null, null, "write_file"], "and the rows after it are read");
  assert.ok(!JSON.stringify(r).includes(SECRET));
  db.close();
});

test("the session database: the older layout, a column gone, and files that aren't Hermes's", () => {
  const dir = tmp();
  const old = stateDb(path.join(dir, "old.db"), { layout: "old" });
  old.session({ id: "s1", source: "cli", cwd: "/Users/ada", startedAt: T - MIN });
  old.message({ session: "s1", role: "tool", tool: "terminal", at: T - 30e3 });
  old.close();
  const r = readStateDb(path.join(dir, "old.db"), { sinceMs: T - 10 * MIN });
  assert.deepEqual([r.state, r.version, r.sessions.map((s) => s.id), r.messages.map((m) => m.tool_name), r.heartbeats], ["ok", "12", ["s1"], ["terminal"], []]);
  assert.deepEqual(kinds("state-db"), [], "an older layout isn't drift");
  // A needed column gone is.
  const gone = stateDb(path.join(dir, "gone.db"));
  gone.db.exec("ALTER TABLE sessions DROP COLUMN cwd");
  gone.close();
  assert.equal(readStateDb(path.join(dir, "gone.db"), { sinceMs: T - MIN }).state, "ok");
  assert.deepEqual(kinds("state-db"), [["missing-field", "sessions[].cwd"]]);
  fs.writeFileSync(path.join(dir, "junk.db"), "not a database at all");
  assert.equal(readStateDb(path.join(dir, "junk.db"), { sinceMs: T }).state, "unreadable");
  assert.equal(readStateDb(path.join(dir, "none.db"), { sinceMs: T }).state, "missing");
});

test("each profile's files are its own", () => {
  const root = makeRoot(path.join(tmp(), ".hermes"));
  const work = profileHome(root, "work");
  setGateway(work, { state: "degraded", pid: 4400, updatedAt: T - 5 * MIN });
  assert.equal(readGateway(root).state, "missing");
  assert.equal(readGateway(work).gateway.state, "degraded");
});
