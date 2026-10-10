// The test harnesses (test/harness/): a clock moved by hand, folders of
// made-up files with the mtimes and watch events a test says, and a
// made-up machine's processes.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fakeClock } from "./harness/clock.js";
import { tempTree, fakeWatch, cleanupAll } from "./harness/files.js";
import { fakeProcesses } from "./harness/procs.js";
import { createProcs, parsePs } from "../core/procs.js";

after(cleanupAll);

test("the clock: timers fire in order as it moves, intervals repeat, and what they start runs", async () => {
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const t0 = clock.now();
  const fired = [];
  clock.setTimeout(() => fired.push(["b", clock.now() - t0]), 200);
  clock.setTimeout(() => fired.push(["a", clock.now() - t0]), 100);
  const tick = clock.setInterval(() => fired.push(["tick", clock.now() - t0]), 150);
  const gone = clock.setTimeout(() => fired.push(["never"]), 50);
  clock.clearTimeout(gone);
  clock.setTimeout(async () => {
    await Promise.resolve();
    fired.push(["after a promise", clock.now() - t0]);
  }, 120);
  await clock.advance(310);
  assert.deepEqual(fired, [["a", 100], ["after a promise", 120], ["tick", 150], ["b", 200], ["tick", 300]]);
  assert.equal(clock.now(), t0 + 310);
  clock.clearInterval(tick);
  assert.equal(clock.pending(), 0);
  // Handles answer unref() as Node's do.
  assert.doesNotThrow(() => clock.setTimeout(() => {}, 5).unref());
  clock.set("2026-10-06T00:00:00Z");
  assert.equal(new Date(clock.now()).toISOString(), "2026-10-06T00:00:00.000Z");
});

test("a tree of files: what's written, with the mtimes given, folders too", () => {
  const tree = tempTree();
  tree.write("a/b/one.jsonl", [{ type: "user" }, "not json"], { mtime: "2026-10-01T00:00:00Z" });
  tree.append("a/b/one.jsonl", "more\n");
  tree.mkdir("a/c", { mtime: Date.parse("2026-09-01T00:00:00Z") });
  tree.touch("a", new Date("2026-08-01T00:00:00Z"));
  assert.equal(tree.read("a/b/one.jsonl"), '{"type":"user"}\nnot json\nmore\n');
  assert.equal(tree.stat("a/c").mtimeMs, Date.parse("2026-09-01T00:00:00Z"));
  assert.equal(tree.stat("a").mtimeMs, Date.parse("2026-08-01T00:00:00Z"));
  assert.deepEqual(tree.files(), ["a/b/one.jsonl"]);
  assert.equal(tree.path("a/b"), path.join(tree.root, "a", "b"));
  tree.remove("a/b");
  assert.equal(tree.exists("a/b"), false);
  tree.cleanup();
  assert.equal(fs.existsSync(tree.root), false);
});

test("a watch that sends what the test says: recursive or not, errors, and refusals", () => {
  const w = fakeWatch();
  const seen = [];
  const deep = w.watch("/r", { recursive: true }, (evt, name) => seen.push(["deep", evt, name]));
  const flat = w.watch("/r", (evt, name) => seen.push(["flat", evt, name]));
  w.emit("/r/x/y.jsonl");
  w.emit("/r/top.jsonl", "rename");
  w.emit("/elsewhere/z.jsonl");
  assert.deepEqual(seen, [["deep", "change", path.join("x", "y.jsonl")], ["deep", "rename", "top.jsonl"], ["flat", "rename", "top.jsonl"]]);
  // As with fs.watch, an error nobody listens for throws.
  const errors = [];
  deep.on("error", (e) => errors.push(e.message));
  assert.throws(() => w.error(new Error("gone")), /gone/);
  flat.on("error", (e) => errors.push(`flat: ${e.message}`));
  w.error(new Error("gone"));
  assert.deepEqual(errors, ["gone", "gone", "flat: gone"]);
  deep.close();
  assert.deepEqual(w.watching(), ["/r"]);
  w.refuse("/locked");
  assert.throws(() => w.watch("/locked", {}, () => {}), /EMFILE/);
});

test("processes: a scripted program reads and answers, exits, and is killed; one that isn't there fails", async () => {
  const machine = fakeProcesses();
  machine.program("codex", (io) => {
    io.on("line", (line) => {
      const m = JSON.parse(line);
      if (m.method === "bye") return io.exit(3);
      io.json({ id: m.id, result: m.method });
    });
    io.error("warming up\n");
  });
  const child = machine.spawn("/Applications/ChatGPT.app/Contents/Resources/codex", ["app-server"]);
  assert.ok(machine.table.get(child.pid), "in the table while it runs");
  assert.deepEqual(machine.table.get(child.pid).args, ["app-server"]);
  const out = [];
  child.stdout.setEncoding("utf8").on("data", (d) => out.push(...d.split("\n").filter(Boolean)));
  let err = "";
  child.stderr.setEncoding("utf8").on("data", (d) => (err += d));
  const exited = new Promise((r) => child.on("exit", (code, signal) => r([code, signal])));
  child.stdin.write(JSON.stringify({ id: 1, method: "initialize" }) + "\n");
  child.stdin.write(JSON.stringify({ id: 2, method: "thread/list" }) + "\n" + JSON.stringify({ method: "bye" }) + "\n");
  assert.deepEqual(await exited, [3, null]);
  assert.deepEqual(out.map((l) => JSON.parse(l).result), ["initialize", "thread/list"]);
  assert.equal(err, "warming up\n");
  assert.equal(machine.table.get(child.pid), undefined, "gone once it exits");

  machine.program("sleeper", () => {});
  const sleeper = machine.spawn("sleeper");
  const killed = new Promise((r) => sleeper.on("exit", (code, signal) => r([code, signal])));
  assert.equal(sleeper.kill(), true);
  assert.deepEqual(await killed, [null, "SIGTERM"]);

  const missing = machine.spawn("nope", []);
  assert.equal(missing.pid, undefined);
  const error = await new Promise((r) => missing.on("error", r));
  assert.equal(error.code, "ENOENT");
  assert.deepEqual(machine.spawned.map((s) => s.file), ["/Applications/ChatGPT.app/Contents/Resources/codex", "sleeper", "nope"]);
});

test("execFile on the made-up machine: answers, failures with their code, input, and timeouts", async () => {
  const clock = fakeClock();
  const machine = fakeProcesses({ clock });
  machine.program("git", ({ args }) => (args.includes("status") ? { stdout: "## main\n M a.js\n" } : { stderr: "fatal: not a git repository\n", code: 128 }));
  machine.program("upper", async (io) => ({ stdout: (await io.input()).toUpperCase() }));
  machine.program("slow", { stdout: "late", delayMs: 10e3 });
  const run = (file, args, opts = {}) => new Promise((resolve) => machine.execFile(file, args, opts, (err, stdout, stderr) => resolve({ err, stdout, stderr })));

  assert.deepEqual(await run("git", ["-C", "/p", "status"]), { err: null, stdout: "## main\n M a.js\n", stderr: "" });
  const bad = await run("git", ["-C", "/p", "config"]);
  assert.equal(bad.err.code, 128);
  assert.equal(bad.stderr, "fatal: not a git repository\n");
  assert.equal((await run("upper", [])).stdout, "");

  const slow = run("slow", [], { timeout: 5000 });
  await clock.advance(5000);
  const timedOut = await slow;
  assert.equal(timedOut.err.killed, true);
  assert.equal(timedOut.err.signal, "SIGTERM");
  assert.equal(timedOut.stdout, "");
});

test("the process table reads back through ps, and core/procs.js sees the made-up machine", async () => {
  const clock = fakeClock("2026-10-03T16:34:29Z");
  const machine = fakeProcesses({ clock });
  const claude = machine.add({ program: "/Users/ada/.local/bin/claude", args: [] });
  const node = machine.add({ ppid: claude, program: "/usr/local/bin/node", args: ["/x/node_modules/@openai/codex/bin/codex.js"] });
  const shell = machine.add({ ppid: node, program: "/bin/zsh" });
  const t = parsePs(machine.ps());
  assert.deepEqual(t.get(shell), { pid: shell, ppid: node, start: clock.now(), program: "/bin/zsh" });
  assert.equal(t.get(1).program, "/sbin/launchd");

  const procs = createProcs({ platform: "darwin", exec: machine.exec, now: clock.now });
  const chain = await procs.chain(shell);
  assert.deepEqual(chain.map((p) => [p.pid, p.agent]), [[shell, null], [node, "codex"], [claude, "claude"], [machine.self.pid, null]]);
  assert.equal(await procs.alive({ pid: claude, start: clock.now() }), true);
  machine.sign(claude, "Q6L2SF6YDW");
  assert.equal(await procs.signed(chain[2]), true);
  assert.equal(await procs.signed(chain[1]), false);
  machine.connect({ pid: shell, clientPort: 51000, serverPort: 47842 });
  assert.equal(await procs.peer({ remotePort: 51000, localPort: 47842 }), shell);
});
