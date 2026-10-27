// Headless runs (core/delivery/runs.js, and findCli in core/dirs.js): the
// program found on PATH, in Claude desktop's folder or by its variable,
// started with each argument quoted once for its platform, at most four at
// once and one per session, its output logged and its record kept across a
// restart. Made-up folders and processes; nothing is run. Paths are this
// system's own (path.join, path.delimiter), so the same cases hold on
// Windows.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { EventEmitter } from "node:events";
import { launch, admit, kept, staleLogs, createRuns, cmdExe, CAP, TURN_MS, LOG_MAX, KEEP_MS } from "../core/delivery/runs.js";
import { findCli, bundledClaudeCode } from "../core/dirs.js";

const p = (...parts) => path.join(path.sep, ...parts);
const PATHS = (...dirs) => dirs.join(path.delimiter);

// Made-up folders: the files there, by path, with their mtimes.
function disk(files) {
  const paths = Object.keys(files);
  return {
    exists: (f) => paths.some((x) => x === f || x.startsWith(`${f}${path.sep}`)),
    readdir: (d) => {
      const names = [...new Set(paths.filter((x) => x.startsWith(`${d}${path.sep}`)).map((x) => x.slice(d.length + 1).split(path.sep)[0]))];
      if (!names.length) throw Object.assign(new Error(`ENOENT: ${d}`), { code: "ENOENT" });
      return names;
    },
    mtime: (f) => files[f] ?? NaN,
    canRun: (f) => {
      if (!(f in files)) throw new Error(`EACCES: ${f}`);
    },
  };
}

test("a program is quoted once for its platform: as it is elsewhere, a batch file through cmd.exe, by its full path, on Windows", () => {
  const args = ["-p", "--resume", "0190d3e0-0000-7000-8000-00000000ab01", "--settings", "/Users/Jo Smith/listen.json"];
  assert.deepEqual(launch("/Users/Jo Smith/.local/bin/claude", args, { platform: "darwin" }), { program: "/Users/Jo Smith/.local/bin/claude", args, verbatim: false });
  assert.deepEqual(launch("/usr/bin/codex", ["exec", "resume", "x", "-"], { platform: "linux" }), { program: "/usr/bin/codex", args: ["exec", "resume", "x", "-"], verbatim: false });
  assert.deepEqual(launch("C:\\Program Files\\Claude\\claude.exe", args, { platform: "win32" }), { program: "C:\\Program Files\\Claude\\claude.exe", args, verbatim: false }, "Node quotes an .exe's arguments itself");
  const env = { ComSpec: "C:\\Windows\\system32\\cmd.exe" };
  assert.deepEqual(launch("C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD", ["-p", "--settings", "C:\\Users\\Jo Smith\\listen.json"], { platform: "win32", env }), {
    program: "C:\\Windows\\system32\\cmd.exe",
    args: ["/d", "/s", "/c", '""C:\\Users\\Jo Smith\\AppData\\Roaming\\npm\\claude.CMD" "-p" "--settings" "C:\\Users\\Jo Smith\\listen.json""'],
    verbatim: true,
  });
  assert.equal(launch("C:\\npm\\codex.bat", ["exec", "-"], { platform: "win32", env }).args[3], '""C:\\npm\\codex.bat" "exec" "-""');
  assert.deepEqual(launch("C:\\npm\\claude.cmd", ["100%"], { platform: "win32", env }), { why: "XenoNet can't pass 100% to a batch file safely" });
  for (const bad of ['a"b', "hi!", "C:\\dir\\", "a\nb"]) assert.ok(launch("C:\\npm\\claude.cmd", [bad], { platform: "win32", env }).why, bad);
  assert.equal(launch("/opt/claude.cmd", args, { platform: "linux" }).verbatim, false, "only Windows runs batch files");
  // cmd.exe is %ComSpec%, else in %SystemRoot%: never looked for by name in
  // the folder the run starts in.
  assert.equal(launch("C:\\npm\\claude.cmd", [], { platform: "win32", env: { SystemRoot: "E:\\Win" } }).program, "E:\\Win\\System32\\cmd.exe");
  assert.equal(cmdExe(undefined, undefined), "C:\\Windows\\System32\\cmd.exe");
});

test("Claude Code is found on PATH, then where Claude desktop keeps it (the newest verified build), then at XENONET_CLAUDE_BIN", () => {
  const home = p("Users", "jo");
  const support = path.join(home, "Library", "Application Support", "Claude", "claude-code");
  const binary = (version, build) => path.join(support, version, build, "claude.app", "Contents", "MacOS", "claude");
  const verified = (version, build) => path.join(support, version, build, ".verified");
  const d = disk({
    [binary("2.1.308", "a93ded4b1b47")]: 2,
    [verified("2.1.308", "a93ded4b1b47")]: 2,
    [binary("2.1.99", "4fa9694b7975")]: 3,
    [verified("2.1.99", "4fa9694b7975")]: 3,
    // Newer, but not verified: a download under way.
    [binary("2.1.308", "b0")]: 9,
    [p("opt", "bin", "claude")]: 1,
  });
  assert.deepEqual(bundledClaudeCode(path.join(home, "Library", "Application Support", "Claude"), { platform: "darwin", ...d }), [binary("2.1.308", "a93ded4b1b47"), binary("2.1.99", "4fa9694b7975")], "2.1.308 is newer than 2.1.99");
  const unverified = disk({ [binary("2.2.0", "new")]: 2, [binary("2.1.0", "old")]: 1, [verified("2.1.0", "old")]: 1 });
  assert.deepEqual(bundledClaudeCode(path.join(home, "Library", "Application Support", "Claude"), { platform: "darwin", ...unverified }), [binary("2.1.0", "old")], "the newest verified build");
  const at = (env, look = d, platform = "darwin") => findCli("claude", { env, platform, homeDir: home, ...look });
  assert.equal(at({ PATH: PATHS(p("usr", "bin"), p("opt", "bin")) }), p("opt", "bin", "claude"), "PATH first");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: p("x", "claude") }), binary("2.1.308", "a93ded4b1b47"), "then Claude desktop's own");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: p("x", "claude") }, disk({})), p("x", "claude"), "then its variable");
  assert.equal(at({ PATH: p("usr", "bin"), XENONET_CLAUDE_BIN: "" }, disk({})), null, "or nowhere");
  // Windows: PATHEXT finds an npm install's claude.cmd; Claude desktop's
  // MSIX folder holds its claude.exe.
  assert.equal(at({ PATH: p("npm"), PATHEXT: ".EXE;.CMD" }, disk({ [p("npm", "claude.CMD")]: 1 }), "win32"), p("npm", "claude.CMD"));
  const msix = path.join("L", "Packages", "Claude_pzs8sxrjxfjjc", "LocalCache", "Roaming", "Claude", "claude-code");
  const win = disk({
    [path.join(msix, "1.0.0", "x", "claude.exe")]: 1,
    [path.join(msix, "1.0.0", "x", ".verified")]: 1,
    [path.join(msix, "1.0.0", "y", "bin", "claude.exe")]: 2,
    [path.join(msix, "1.0.0", "y", ".verified")]: 2,
    [path.join(msix, "0.9.0", "z", "claude.exe")]: 9,
    [path.join(msix, "0.9.0", "z", ".verified")]: 9,
  });
  assert.equal(at({ PATH: p("npm"), PATHEXT: ".EXE;.CMD", LOCALAPPDATA: "L" }, win, "win32"), path.join(msix, "1.0.0", "y", "bin", "claude.exe"), "the newest version's newest build");
});

test("Codex is found by XENONET_CODEX_BIN, then in the ChatGPT app, then on PATH; nothing else is found", () => {
  const app = path.join("/Applications", "ChatGPT.app", "Contents", "Resources", "codex");
  const d = disk({ [app]: 1, [p("usr", "bin", "codex")]: 1, [p("usr", "bin", "opencode")]: 1 });
  const at = (cli, env, platform = "darwin") => findCli(cli, { env, platform, homeDir: p("Users", "jo"), ...d });
  assert.equal(at("codex", { PATH: p("usr", "bin"), XENONET_CODEX_BIN: p("c", "codex") }), p("c", "codex"));
  assert.equal(at("codex", { PATH: p("usr", "bin") }), app);
  assert.equal(at("codex", { PATH: p("usr", "bin") }, "linux"), p("usr", "bin", "codex"));
  assert.equal(at("opencode", { PATH: p("usr", "bin") }, "linux"), null);
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
  // A runner's own runs under way count once, record or not.
  assert.equal(admit(runs.slice(0, 2), "claude:g", { now, alive: () => true, also: ["claude:a", "claude:g"] }), "a run is under way for it");
  assert.equal(admit(runs.slice(0, 2), "claude:h", { now, alive: () => true, also: ["claude:a", "claude:g"] }), null, "claude:a counted once");
  assert.deepEqual(kept([run("old", "claude:z", now - KEEP_MS, now - KEEP_MS), runs[0]], now), [runs[0]]);
  assert.equal(kept(Array.from({ length: 250 }, (_, i) => run(`r${i}`, "claude:a", now - 1, now)), now)[0].id, "r50");
  assert.deepEqual(staleLogs([{ name: "run_a.log", mtime: now - KEEP_MS }, { name: "run_b.log", mtime: now - 1 }, { name: "notes.txt", mtime: 0 }], now), ["run_a.log"]);
});

// Processes that say what they're told to and exit when they're told to;
// every program started is listed, taskkill too.
function fakeSpawn() {
  const started = [];
  const calls = [];
  let pid = 100;
  const spawn = (program, args, opts) => {
    calls.push({ program, args });
    const child = new EventEmitter();
    child.pid = ++pid;
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    child.stdin = { on() {}, end: (text) => started.push({ program, args, opts, text, child }) };
    child.kill = (signal) => ((child.signal = signal ?? "SIGTERM"), child.emit("exit", null, child.signal));
    setImmediate(() => child.emit("spawn"));
    return child;
  };
  return { spawn, started, calls };
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
  const r = createRuns({ dir, spawn, now: () => t.now, alive: (pid) => alive.has(pid), find: (app) => path.join(dir, app), platform: "darwin" });
  const out = await r.run({ key: "claude:a", app: "claude", args: ["-p"], cwd: dir, env: { PATH: dir }, text: "hi" });
  assert.deepEqual(out, { ok: true, where: "pid 101" });
  const [first] = started;
  assert.equal(first.program, path.join(dir, "claude"));
  assert.equal(first.opts.windowsHide, true);
  assert.equal(first.text, "hi");
  assert.ok(!fs.existsSync(path.join(dir, "runs", "run_old.log")), "a week-old log is gone");
  alive.add(first.child.pid);
  first.child.stdout.emit("data", Buffer.from("out\n"));
  first.child.stderr.emit("data", Buffer.from("err\n"));
  first.child.stdout.emit("data", Buffer.alloc(LOG_MAX, "x"));
  first.child.stdout.emit("data", Buffer.from("past the cap"));
  // Still under way: a restart (a new runner on the same folder) counts it.
  const again = createRuns({ dir, spawn, now: () => t.now, alive: (pid) => alive.has(pid), find: (app) => path.join(dir, app) });
  assert.ok(again.running("claude:a"));
  assert.equal((await again.run({ key: "claude:a", app: "claude", args: [], cwd: dir })).why, "a run is under way for it");
  t.now += 1500;
  first.child.emit("exit", 0, null);
  alive.delete(first.child.pid);
  const [rec] = r.list();
  assert.deepEqual([rec.key, rec.app, rec.pid, rec.exit, rec.ms, rec.timedOut], ["claude:a", "claude", 101, 0, 1500, undefined]);
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
  // A run that doesn't start leaves no log behind.
  const failing = () => {
    const child = new EventEmitter();
    setImmediate(() => child.emit("error", Object.assign(new Error("spawn EACCES"), { code: "EACCES" })));
    return child;
  };
  const before = fs.readdirSync(path.join(dir, "runs")).length;
  assert.equal((await createRuns({ dir, spawn: failing, find: (app) => app }).run({ key: "claude:b", app: "claude", args: [], cwd: dir })).why, "spawn EACCES");
  assert.equal(fs.readdirSync(path.join(dir, "runs")).length, before);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("a run counts from the moment it's admitted: two started in one tick can't both run, and a fifth waits", async () => {
  const { spawn, started } = fakeSpawn();
  const r = createRuns({ spawn, alive: () => true, find: (app) => app });
  const at = (key) => r.run({ key, app: "claude", args: [], cwd: "/" });
  const [one, two] = await Promise.all([at("claude:a"), at("claude:a")]);
  assert.deepEqual([one.ok, two.why], [true, "a run is under way for it"], "one per session, in one tick too");
  const rest = await Promise.all(["b", "c", "d", "e"].map((k) => at(`claude:${k}`)));
  assert.deepEqual(rest.map((x) => x.ok), [true, true, true, false]);
  assert.equal(rest[3].why, "XenoNet runs at most 4 turns at once on this machine");
  assert.equal((await r.run({ key: "claude:f", app: "nothing", args: [], cwd: "/" })).why, "XenoNet runs at most 4 turns at once on this machine");
  started[0].child.emit("exit", 0, null);
  assert.equal((await at("claude:e")).ok, true, "one ended");
  // One whose program isn't found frees its session.
  started[1].child.emit("exit", 0, null);
  const none = createRuns({ spawn, alive: () => true, find: () => null });
  assert.equal((await none.run({ key: "claude:z", app: "claude", args: [], cwd: "/" })).why, "claude isn't installed here");
  assert.equal(none.running("claude:z"), false);
  for (const s of started) s.child.emit("exit", 0, null);
});

test("a turn past its cap is stopped at once, and its record says so; on Windows the whole tree, and a batch file goes through cmd.exe", async () => {
  const { spawn, started, calls } = fakeSpawn();
  const r = createRuns({ spawn, alive: () => true, find: (app) => app, timeoutMs: 5, platform: "linux" });
  await r.run({ key: "claude:a", app: "claude", args: [], cwd: "/" });
  await new Promise((done) => setTimeout(done, 30));
  assert.equal(started[0].child.signal, "SIGKILL", "a forced kill, as the binary's");
  assert.equal(r.list()[0].timedOut, true);
  assert.equal(r.running("claude:a"), false);
  const w = createRuns({ spawn, alive: () => true, find: () => "C:\\npm\\claude.cmd", timeoutMs: 5, platform: "win32" });
  await w.run({ key: "claude:w", app: "claude", args: ["-p"], cwd: "/" });
  const run = started.at(-1);
  assert.deepEqual([run.program, run.args, run.opts.windowsVerbatimArguments], [cmdExe(process.env.ComSpec ?? process.env.COMSPEC, process.env.SystemRoot ?? process.env.SYSTEMROOT), ["/d", "/s", "/c", '""C:\\npm\\claude.cmd" "-p""'], true]);
  await new Promise((done) => setTimeout(done, 30));
  const kill = calls.at(-1);
  assert.match(kill.program, /\\System32\\taskkill\.exe$/);
  assert.deepEqual(kill.args, ["/T", "/F", "/PID", String(run.child.pid)], "cmd.exe and the node it started");
  run.child.emit("exit", 1, null);
  assert.equal(w.list()[0].timedOut, true);
});
