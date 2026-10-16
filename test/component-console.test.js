// The installed console (core/components/console.js), as the xenonet-console
// MCP tools and xenonet console drive an installed XenoNet: each action finds
// the service again, so it survives Desktop and the service restarting;
// nothing development-only (eval, simulate) is ever sent; a service whose
// console interface doesn't match gets nothing; and pinging, quitting and
// launching Desktop say what happened. A stand-in service on 127.0.0.1
// answers as XenoNet's would; Desktop is a made-up program.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { installedConsole } from "../core/components/console.js";
import { descriptorFile } from "../core/components/runtime.js";
import { rpc } from "../core/components/rpc.js";

const TOKEN = "ab".repeat(32);

function runtime(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-console-"));
  // The stand-in Desktop can still be exiting after its marker appears.
  // Async retries let Windows release its cwd before teardown removes it.
  t.after(async () => fs.promises.rm(dir, { recursive: true, force: true, maxRetries: 20, retryDelay: 100 }));
  return { dir, env: { XENONET_RUNTIME_DIR: dir, XENONET_PORT: "47999" } };
}

// A service that writes its connection file and answers hello and console
// requests: `desktop` whether Desktop is connected, `ready` whether its page
// is, `consoleApi` the console interface it speaks, `slowHellos` how many
// hellos it leaves unanswered first, as a busy service would. `seen` is
// every console request that reached it.
async function service(env, { desktop = true, ready = true, consoleApi = { min: 1, max: 1 }, slowHellos = 0, answer } = {}) {
  const s = { desktop, ready, seen: [], hellos: 0, sockets: new Set() };
  let closing = false;
  const server = net.createServer((socket) => {
    if (closing) { socket.destroy(); return; }
    s.sockets.add(socket);
    socket.on("close", () => s.sockets.delete(socket));
    rpc(socket, async (method, params) => {
      if (method === "hello") {
        if (params.token !== TOKEN) throw new Error("XenoNet refused this connection.");
        if (s.hellos++ < slowHellos) return new Promise(() => {});
        return { version: "0.11.0", api: { min: 1, max: 1 }, consoleApi, desktop: s.desktop };
      }
      if (method !== "console") throw new Error("Unsupported request.");
      s.seen.push(params);
      if (answer) return answer(params, s);
      if (params.op === "ping") return { instance: "installed", name: "XenoNet", controlled: true, service: true, page: { ready: s.ready } };
      if (params.op === "quit") s.desktop = false;
      return { done: params.op };
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  fs.writeFileSync(descriptorFile(env), JSON.stringify({ port: server.address().port, token: TOKEN }));
  s.close = () => new Promise((r) => {
    closing = true;
    for (const socket of s.sockets) socket.destroy();
    server.close(r);
  });
  return s;
}

test("with no service there, nothing is running: ping is null, health false, quit a no-op, and launch asks for Desktop to be registered", async (t) => {
  const { env } = runtime(t);
  const c = installedConsole({ env });
  assert.equal(await c.ping(), null);
  assert.equal(await c.health(), false);
  assert.deepEqual(await c.quit(), { quit: false });
  await assert.rejects(c.launch(), /Open XenoNet once to register Desktop/);
  assert.deepEqual([c.id, c.port], ["installed", 47999]);
});

test("development-only actions are refused here and never reach the service", async (t) => {
  const { env } = runtime(t);
  const s = await service(env);
  t.after(s.close);
  const c = installedConsole({ env });
  assert.throws(() => c.eval("document.title"), /only in development instances/);
  assert.throws(() => c.simulate({ kind: "question" }), /only in development instances/);
  assert.deepEqual([s.hellos, s.seen], [0, []], "not even a connection");
});

test("a service whose console interface doesn't match gets no console request", async (t) => {
  const { env } = runtime(t);
  const s = await service(env, { consoleApi: { min: 2, max: 3 } });
  t.after(s.close);
  await assert.rejects(installedConsole({ env }).driver("select", "project:/users/nova/projects/atlas"), /incompatible/);
  assert.deepEqual(s.seen, []);
});

test("each action finds the service again, so one that restarted on another port is reached", async (t) => {
  const { env } = runtime(t);
  const c = installedConsole({ env });
  const first = await service(env);
  assert.deepEqual(await c.driver("select", "project:/users/nova/projects/atlas"), { done: "driver" });
  await first.close();
  const second = await service(env);
  t.after(second.close);
  assert.deepEqual(await c.screenshot({ width: 800 }), { done: "screenshot" });
  assert.deepEqual(first.seen, [{ op: "driver", method: "select", args: ["project:/users/nova/projects/atlas"] }]);
  assert.deepEqual(second.seen, [{ op: "screenshot", width: 800 }]);
  // The console hangs up after each action; the service hears it a moment later.
  for (const until = Date.now() + 2000; second.sockets.size && Date.now() < until;) await new Promise((r) => setTimeout(r, 10));
  assert.equal(second.sockets.size, 0, "each connection closed after its action");
});

test("with Desktop closed, ping says so without asking it, and quit has nothing to do", async (t) => {
  const { env } = runtime(t);
  const s = await service(env, { desktop: false });
  t.after(s.close);
  const c = installedConsole({ env });
  assert.deepEqual(await c.ping(), { instance: "installed", name: "XenoNet", controlled: false, service: true, page: { ready: false }, error: "XenoNet Desktop is closed or restarting." });
  assert.equal(await c.health(), true, "the service is there");
  assert.deepEqual(await c.quit(), { quit: false, service: true });
  assert.deepEqual(s.seen, []);
});

test("quit asks Desktop to close and waits until it has, leaving the service running", async (t) => {
  const { env } = runtime(t);
  const s = await service(env);
  t.after(s.close);
  assert.deepEqual(await installedConsole({ env }).quit(), { quit: true, service: true });
  assert.deepEqual(s.seen.map((x) => x.op), ["ping", "quit"]);
  assert.equal(s.desktop, false);
});

test("readiness tolerates Desktop disconnecting after hello without retrying actions", async (t) => {
  const { env } = runtime(t); let probes = 0;
  const s = await service(env, { answer: ({ op }) => {
    if (op === "ping" && ++probes > 1) return { page: { ready: true } };
    throw new Error("XenoNet Desktop is closed or restarting. The service is still running.");
  } });
  t.after(s.close);
  const c = installedConsole({ env });
  assert.equal((await c.ready({ timeout: 2000 })).page.ready, true);
  await assert.rejects(c.driver("select", "project:/users/nova/projects/atlas"), /closed or restarting/);
  assert.equal(s.seen.filter((request) => request.op === "driver").length, 1);
});

test("launching a Desktop that's already up brings its window forward, or leaves it hidden, and waits for its page", async (t) => {
  const { env } = runtime(t);
  const s = await service(env);
  t.after(s.close);
  const c = installedConsole({ env });
  assert.equal((await c.launch()).page.ready, true);
  await c.launch({ show: "hidden" });
  assert.deepEqual(s.seen.filter((x) => x.op === "window"), [{ op: "window", show: "inactive" }, { op: "window" }]);
  s.ready = false;
  await assert.rejects(c.ready({ timeout: 250 }), /did not become ready/);
});

test("waiting for Desktop goes on when one probe isn't answered in time, since a ping is safe to ask again", { timeout: 20000 }, async (t) => {
  const { env } = runtime(t);
  const s = await service(env, { slowHellos: 1 });
  t.after(s.close);
  const c = installedConsole({ env });
  const started = Date.now();
  assert.equal((await c.ready({ timeout: 15000 })).page.ready, true);
  assert.ok(Date.now() - started >= 5000, "the first probe ran out its 5 s before the next was tried");
  assert.deepEqual([s.hellos, s.seen.map((x) => x.op)], [2, ["ping"]], "one unanswered hello, then one that got through");
  // Anything else the service says still ends the wait at once: here a
  // connection file whose token it refuses.
  fs.writeFileSync(descriptorFile(env), JSON.stringify({ ...JSON.parse(fs.readFileSync(descriptorFile(env), "utf8")), token: "cd".repeat(32) }));
  await assert.rejects(c.ready({ timeout: 15000 }), /refused/);
});

test("launching starts the registered Desktop with how to show it, without ELECTRON_RUN_AS_NODE, and waits for its page", async (t) => {
  const { dir, env } = runtime(t);
  const s = await service(env, { desktop: false, ready: false });
  t.after(s.close);
  // Desktop, as desktop.json registers it: here a program that writes down
  // how it was started, which the service takes as its page being ready.
  const marker = path.join(dir, "started.json");
  const program = path.join(dir, "desktop.cjs");
  fs.writeFileSync(program, `require("node:fs").writeFileSync(${JSON.stringify(marker)}, JSON.stringify({ show: process.env.XENONET_SHOW, runAsNode: process.env.ELECTRON_RUN_AS_NODE ?? null, cwd: process.cwd() }));\n`);
  fs.writeFileSync(path.join(dir, "desktop.json"), JSON.stringify({ command: process.execPath, args: [program], cwd: dir }));
  // Desktop counts as connected, and its page as ready, once the program has run.
  const up = () => fs.existsSync(marker);
  Object.defineProperties(s, { desktop: { get: up }, ready: { get: up } });
  const c = installedConsole({ env: { ...process.env, ...env, ELECTRON_RUN_AS_NODE: "1" } });
  const state = await c.launch({ show: "active", timeout: 15000 });
  assert.equal(state.page.ready, true);
  const started = JSON.parse(fs.readFileSync(marker, "utf8"));
  assert.deepEqual([started.show, started.runAsNode, fs.realpathSync(started.cwd)], ["active", null, fs.realpathSync(dir)]);
});
