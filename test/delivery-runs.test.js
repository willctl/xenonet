// Headless runs (core/delivery/runs.js, and findCli in core/dirs.js): the
// program found on PATH, in Claude desktop's folder or by its variable,
// started with each argument quoted once for its platform, at most four at
// once and one per session, its output logged and its record kept across a
// restart. Made-up folders and processes; nothing is run.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { EventEmitter } from "node:events";
import { launch, admit, kept, staleLogs, createRuns, CAP, TURN_MS, LOG_MAX, KEEP_MS } from "../core/delivery/runs.js";
import { findCli, bundledClaudeCode } from "../core/dirs.js";

// Made-up folders: the files there, by path, with their mtimes.
function disk(files) {
  const paths = Object.keys(files);
  return {
    exists: (p) => paths.some((f) => f === p || f.startsWith(`${p}/`)),
    readdir: (p) => {
      const names = [...new Set(paths.filter((f) => f.startsWith(`${p}/`)).map((f) => f.slice(p.length + 1).split("/")[0]))];
      if (!names.length) throw Object.assign(new Error(`ENOENT: ${p}`), { code: "ENOENT" });
      return names;
    },
    mtime: (f) => files[f] ?? NaN,
    canRun: (f) => {
      if (!(f in files)) throw new Error(`EACCES: ${f}`);
    },
  };
}

test("a program is quoted once for its platform: as it is elsewhere, a batch file through cmd.exe on Windows", () => {
  const args = ["-p", "--resume", "0190d3e0-0000-7000-8000-00000000ab01", "--settings", "/Users/Jo Smith/listen.json"];
  assert.deepEqual(launch("/Users/Jo Smith/.local/bin/claude", args, { platform: "darwin" }), { program: "/Users/Jo Smith/.local/bin/claude", args, verbatim: false });
  assert.deepEqual(launch("/usr/bin/codex", ["exec", "resume", "x", "-"], { platform: "linux" }), { program: "/usr/bin/codex", args: ["exec", "resume", "x", "-"], verbatim: false });
  assert.deepEqual(launch("C:\\Program Files\\Claude\\claude.exe", args, { platform: "win32" }), { program: "C:\\Program Files\\Claude\\claude.exe", args, verbatim: false }, "Node quotes an .exe's arguments itself");
  assert.deepEqual(launch("C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD", ["-p", "--settings", "C:\\Users\\Jo Smith\\listen.json"], { platform: "win32" }), {
    program: "cmd.exe",
    args: ["/d", "/s", "/c", '""C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD" "-p" "--settings" "C:\\Users\\Jo Smith\\listen.json""'],
    verbatim: true,
  });
  assert.equal(launch("C:\\npm\\codex.bat", ["exec", "-"], { platform: "win32" }).args[3], '""C:\\npm\\codex.bat" "exec" "-""');
  assert.deepEqual(launch("C:\\npm\\claude.cmd", ["100%"], { platform: "win32" }), { why: "XenoNet can't pass 100% to a batch file safely" });
  assert.ok(launch("C:\\npm\\claude.cmd", ['a"b'], { platform: "win32" }).why);
  assert.equal(launch("/opt/claude.cmd", args, { platform: "linux" }).verbatim, false, "only Windows runs batch files");
});

test("Claude Code is found on PATH, then where Claude desktop keeps it (the newest version), then at XENONET_CLAUDE_BIN", () => {
  const home = "/Users/jo";
  const support = `${home}/Library/Application Support/Claude/claude-code`;
  const newest = `${support}/2.1.308/a93ded4b1b47/claude.app/Contents/MacOS/claude`;
  const d = disk({ [newest]: 2, [`${support}/2.1.99/4fa9694b7975/claude.app/Contents/MacOS/claude`]: 3, [`${support}/2.1.308/b0/.verified`]: 1, "/opt/bin/claude": 1 });
  assert.deepEqual(bundledClaudeCode(`${home}/Library/Application Support/Claude`, { platform: "darwin", ...d }), [newest, `${support}/2.1.99/4fa9694b7975/claude.app/Contents/MacOS/claude`], "2.1.308 is newer than 2.1.99");
  const at = (env, look = d, platform = "darwin") => findCli("claude", { env, platform, homeDir: home, ...look });
  assert.equal(at({ PATH: "/usr/bin:/opt/bin" }), "/opt/bin/claude", "PATH first");
  assert.equal(at({ PATH: "/usr/bin", XENONET_CLAUDE_BIN: "/x/claude" }), newest, "then Claude desktop's own");
  assert.equal(at({ PATH: "/usr/bin", XENONET_CLAUDE_BIN: "/x/claude" }, disk({})), "/x/claude", "then its variable");
  assert.equal(at({ PATH: "/usr/bin", XENONET_CLAUDE_BIN: "" }, disk({})), null, "or nowhere");
  // Windows: PATHEXT finds an npm install's claude.cmd; Claude desktop's
  // MSIX folder holds its claude.exe.
  assert.equal(at({ PATH: "/npm", PATHEXT: ".EXE;.CMD" }, disk({ "/npm/claude.CMD": 1 }), "win32"), "/npm/claude.CMD");
  const msix = "/L/Packages/Claude_pzs8sxrjxfjjc/LocalCache/Roaming/Claude/claude-code";
  const win = disk({ [`${msix}/1.0.0/x/claude.exe`]: 1, [`${msix}/1.0.0/y/bin/claude.exe`]: 2, [`${msix}/0.9.0/z/claude.exe`]: 9 });
  assert.equal(at({ PATH: "/npm", PATHEXT: ".EXE;.CMD", LOCALAPPDATA: "/L" }, win, "win32"), `${msix}/1.0.0/y/bin/claude.exe`, "the newest version's newest build");
});

test("Codex is found by XENONET_CODEX_BIN, then in the ChatGPT app, then on PATH; nothing else is found", () => {
  const d = disk({ "/Applications/ChatGPT.app/Contents/Resources/codex": 1, "/usr/bin/codex": 1, "/usr/bin/opencode": 1 });
  const at = (app, env, platform = "darwin") => findCli(app, { env, platform, homeDir: "/Users/jo", ...d });
  assert.equal(at("codex", { PATH: "/usr/bin", XENONET_CODEX_BIN: "/c/codex" }), "/c/codex");
  assert.equal(at("codex", { PATH: "/usr/bin" }), "/Applications/ChatGPT.app/Contents/Resources/codex");
  assert.equal(at("codex", { PATH: "/usr/bin" }, "linux"), "/usr/bin/codex");
  assert.equal(at("opencode", { PATH: "/usr/bin" }, "linux"), null);
});

test("four runs at once on a machine, one per session; a run past its turn or whose process is gone doesn't count; records and logs are kept a week", () => {
  const now = 1e12;
  const run = (id, key, at, end = null) => ({ id, key, app: "claude", pid: 10, at, end, exit: null, ms: null });
  const runs = [run("r1", "claude:a", now - 1000), run("r2", "claude:b", now - 1000, now - 10)];
  assert.equal(admit(runs, "claude:a", { now, alive: () => true }), "a run is under way for it");
  assert.equal(admit(runs, "claude:b", { now, alive: () => true }), null, "its run ended");
  for (const k of ["c", "d", "e"]) runs.push(run(k, `claude:${k}`, now - 5));
  assert.equal(CAP, 4);
  assert.equal(admit(runs, "claude:f", { now, alive: () => true }), "XenoNet runs at most 4 turns at once on this machine");
  assert.equal(admit(runs, "claude:f", { now: now + TURN_MS, alive: () => true }), null);
  assert.equal(admit(runs, "claude:f", { now, alive: (pid) => pid !== 10 }), null);
  assert.deepEqual(kept([run("old", "claude:z", now - KEEP_MS, now - KEEP_MS), runs[0]], now), [runs[0]]);
  assert.equal(kept(Array.from({ length: 250 }, (_, i) => run(`r${i}`, "claude:a", now - 1, now)), now)[0].id, "r50");
  assert.deepEqual(staleLogs([{ name: "run_a.log", mtime: now - KEEP_MS }, { name: "run_b.log", mtime: now - 1 }, { name: "notes.txt", mtime: 0 }], now), ["run_a.log"]);
});

// A process that says what it's told to, and exits when it's told to.
function fakeSpawn() {
  const started = [];
  let pid = 100;
  const spawn = (program, args, opts) => {
    const child = new EventEmitter();
    child.pid = ++pid;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = { on() {}, end: (text) => started.push({ program, args, opts, text, child }) };
    child.kill = () => child.emit("exit", null, "SIGTERM");
    setImmediate(() => child.emit("spawn"));
    return child;
  };
  return { spawn, started };
}

test("a run's output goes to its log, at most 1 MB; its exit and how long it took are kept, and a restart keeps counting it", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-runs-"));
  const t = { now: Date.now() };
  const { spawn, started } = fakeSpawn();
  // A log from over a week ago goes when a run starts.
  fs.mkdirSync(path.join(dir, "runs"));
  fs.writeFileSync(path.join(dir, "runs", "run_old.log"), "old");
  const old = (t.now - KEEP_MS - 1000) / 1000;
  fs.utimesSync(path.join(dir, "runs", "run_old.log"), old, old);
  const alive = new Set();
  const r = createRuns({ dir, spawn, now: () => t.now, alive: (pid) => alive.has(pid), find: (app) => `/bin/${app}`, platform: "darwin" });
  const out = await r.run({ key: "claude:a", app: "claude", args: ["-p"], cwd: dir, env: { PATH: "/bin" }, text: "hi" });
  assert.deepEqual(out, { ok: true, where: "pid 101" });
  const [first] = started;
  assert.equal(first.program, "/bin/claude");
  assert.equal(first.opts.windowsHide, true);
  assert.equal(first.text, "hi");
  assert.ok(!fs.existsSync(path.join(dir, "runs", "run_old.log")), "a week-old log is gone");
  alive.add(first.child.pid);
  first.child.stdout.emit("data", Buffer.from("out\n"));
  first.child.stderr.emit("data", Buffer.from("err\n"));
  first.child.stdout.emit("data", Buffer.alloc(LOG_MAX, "x"));
  first.child.stdout.emit("data", Buffer.from("past the cap"));
  // Still under way: a restart (a new runner on the same folder) counts it.
  const again = createRuns({ dir, spawn, now: () => t.now, alive: (pid) => alive.has(pid), find: (app) => `/bin/${app}` });
  assert.ok(again.running("claude:a"));
  assert.equal((await again.run({ key: "claude:a", app: "claude", args: [], cwd: dir })).why, "a run is under way for it");
  t.now += 1500;
  first.child.emit("exit", 0, null);
  alive.delete(first.child.pid);
  const [rec] = r.list();
  assert.deepEqual([rec.key, rec.app, rec.pid, rec.exit, rec.ms], ["claude:a", "claude", 101, 0, 1500]);
  const log = fs.readFileSync(path.join(dir, "runs", `${rec.id}.log`), "utf8");
  assert.ok(log.startsWith("out\nerr\nxxx"));
  assert.ok(log.length < LOG_MAX + 100 && !log.includes("past the cap"), "1 MB at most");
  assert.match(log, /XenoNet keeps the first 1 MB/);
  const file = path.join(dir, "delivery-runs.json");
  assert.equal(JSON.parse(fs.readFileSync(file, "utf8")).runs[0].exit, 0);
  if (process.platform !== "win32") {
    assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    assert.equal(fs.statSync(path.join(dir, "runs", `${rec.id}.log`)).mode & 0o777, 0o600);
  }
  assert.equal(createRuns({ dir, spawn, alive: () => true }).running("claude:a"), false, "it ended");
  fs.rmSync(dir, { recursive: true, force: true });
});

test("at most four runs go at once; the fifth waits for its next turn", async () => {
  const { spawn, started } = fakeSpawn();
  const r = createRuns({ spawn, alive: () => true, find: (app) => app });
  for (const k of ["a", "b", "c", "d"]) assert.equal((await r.run({ key: `claude:${k}`, app: "claude", args: [], cwd: "/" })).ok, true, k);
  assert.equal((await r.run({ key: "claude:e", app: "claude", args: [], cwd: "/" })).why, "XenoNet runs at most 4 turns at once on this machine");
  started[0].child.emit("exit", 0, null);
  assert.equal((await r.run({ key: "claude:e", app: "claude", args: [], cwd: "/" })).ok, true, "one ended");
  // A batch file on Windows goes through cmd.exe, its line as written.
  const w = createRuns({ spawn, alive: () => true, find: () => "C:\\npm\\claude.cmd", platform: "win32" });
  await w.run({ key: "claude:w", app: "claude", args: ["-p"], cwd: "/" });
  const last = started.at(-1);
  assert.deepEqual([last.program, last.args, last.opts.windowsVerbatimArguments], ["cmd.exe", ["/d", "/s", "/c", '""C:\\npm\\claude.cmd" "-p""'], true]);
  for (const s of started) s.child.emit("exit", 0, null);
});
