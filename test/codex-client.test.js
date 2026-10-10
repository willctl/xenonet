// The Codex app-server client (core/codex.js) as a building block, against
// a made-up app-server on a made-up machine (test/harness/procs.js): the
// handshake, answers, refusals, time limits, exits and stops, and what its
// health says of each.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { AppServerClient, codexThreadsGatherer } from "../core/codex.js";
import { fakeProcesses } from "./harness/procs.js";
import { fakeClock } from "./harness/clock.js";

const BIN = "/Applications/ChatGPT.app/Contents/Resources/codex";

// An app-server that answers thread/list, never answers "slow", refuses
// what it doesn't know, exits on "crash", and asks XenoNet something of its
// own after the handshake.
function appServer(machine, { log = [] } = {}) {
  machine.program(BIN, (io) => {
    io.on("line", (line) => {
      const m = JSON.parse(line);
      log.push(m);
      if (m.method === "initialize") {
        io.write("not json\n");
        io.json({ jsonrpc: "2.0", method: "thread/started", params: {} });
        io.json({ jsonrpc: "2.0", id: "ask-1", method: "item/commandExecution/requestApproval", params: {} });
        io.json({ jsonrpc: "2.0", id: m.id, result: { userAgent: "codex/0.146.0" } });
      } else if (m.method === "thread/list") io.json({ jsonrpc: "2.0", id: m.id, result: { data: [{ id: "t1", name: "Thread t1" }], nextCursor: null } });
      else if (m.method === "crash") {
        io.error("panicked at src/main.rs\n");
        io.exit(101);
      } else if (m.method !== "slow" && m.id !== undefined && m.method) io.json({ jsonrpc: "2.0", id: m.id, error: { code: -32601, message: `unknown method ${m.method}` } });
    });
  });
  return log;
}

test("the first request starts the app-server and shakes hands; it runs from the first answer", async () => {
  const machine = fakeProcesses();
  const log = appServer(machine);
  const client = new AppServerClient({ spawn: machine.spawn, bin: () => BIN });
  assert.equal(client.health().state, "idle");
  const res = await client.request("thread/list", { archived: false });
  assert.deepEqual(res.data.map((t) => t.id), ["t1"]);
  assert.deepEqual(machine.spawned.map((s) => [s.file, s.args]), [[BIN, ["app-server"]]]);
  const hello = log.find((m) => m.method === "initialize");
  assert.deepEqual(hello.params.capabilities, { experimentalApi: true });
  const { version } = JSON.parse(fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.deepEqual(hello.params.clientInfo, { name: "xenonet", title: "XenoNet", version }, "with XenoNet's own version");
  assert.ok(log.some((m) => m.method === "initialized" && m.id === undefined), "and says it's ready");
  assert.deepEqual(client.info, { userAgent: "codex/0.146.0" });
  // XenoNet only reads, so what the app-server asked of it was refused.
  const answer = log.find((m) => m.id === "ask-1");
  assert.deepEqual([answer.error.code, answer.error.message], [-32601, "XenoNet is read-only"]);
  const h = client.health();
  assert.deepEqual([h.state, h.counters.starts, h.counters.requests], ["running", 1, 2]);
  client.stop();
});

test("a refusal or no answer in time degrades it; an exit fails it, and the next request starts it again", async () => {
  const clock = fakeClock();
  const machine = fakeProcesses({ clock });
  appServer(machine);
  const client = new AppServerClient({ spawn: machine.spawn, bin: () => BIN, timeoutMs: 20e3, clock });
  await client.request("thread/list", {});
  await assert.rejects(client.request("thread/unsubscribe", {}), /unknown method thread\/unsubscribe/);
  assert.deepEqual([client.state, client.lastError.message], ["degraded", "unknown method thread/unsubscribe"]);
  await client.request("thread/list", {});
  assert.equal(client.state, "running");

  const slow = client.request("slow", {}).catch((err) => err);
  await clock.advance(20e3);
  assert.match((await slow).message, /slow timed out/);
  assert.deepEqual([client.state, client.counters.timeouts], ["degraded", 1]);

  await assert.rejects(client.request("crash", {}), /codex app-server exited \(101\): panicked at src\/main.rs/);
  assert.deepEqual([client.state, client.proc, machine.running().length], ["failed", null, 0]);
  await client.request("thread/list", {});
  assert.deepEqual([client.state, client.counters.starts], ["running", 2]);
  client.stop();
});

test("a codex that isn't there fails the request and the client; stopping it isn't a failure", async () => {
  const machine = fakeProcesses();
  const missing = new AppServerClient({ spawn: machine.spawn, bin: () => "/nowhere/codex" });
  await assert.rejects(missing.request("thread/list", {}), /spawn \/nowhere\/codex ENOENT/);
  assert.deepEqual([missing.state, missing.lastError.message], ["failed", "spawn /nowhere/codex ENOENT"]);

  appServer(machine);
  const client = new AppServerClient({ spawn: machine.spawn, bin: () => BIN });
  await client.start();
  assert.equal(client.state, "running", "started as a block, it connects");
  client.stop();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual([client.state, machine.running().length], ["stopped", 0]);
  await client.request("thread/list", {});
  assert.equal(client.state, "running", "and a request after a stop starts it again");
  client.stop();
});

test("Codex's threads as a gatherer: empty, and failed, while the app-server can't answer", async () => {
  let fail = true;
  const g = codexThreadsGatherer({ gather: async () => (fail ? Promise.reject(new Error("codex app-server exited (2): not signed in")) : [{ id: "codex:t1" }]) });
  assert.deepEqual(await g.run(), []);
  assert.deepEqual([g.name, g.state, g.last.error], ["codex-threads", "failed", "codex app-server exited (2): not signed in"]);
  fail = false;
  assert.deepEqual(await g.run(), [{ id: "codex:t1" }]);
});

test("a request right after a stop starts a new app-server, and the old one's exit doesn't touch it", async () => {
  const machine = fakeProcesses();
  appServer(machine);
  // The old app-server takes its time to die, as on a busy machine.
  const realSpawn = machine.spawn;
  const spawn = (...args) => {
    const child = realSpawn(...args);
    const kill = child.kill;
    child.kill = (signal) => (setTimeout(() => kill(signal), 50), (child.killed = true));
    return child;
  };
  const client = new AppServerClient({ spawn, bin: () => BIN });
  await client.request("thread/list", {});
  const slow = client.request("slow", {}).catch((err) => err);
  await new Promise((r) => setImmediate(r));
  const unsent = client.request("thread/list", {}).catch((err) => err);
  client.stop();
  assert.match((await slow).message, /codex app-server stopped/, "what waited on it fails at once");
  assert.match((await unsent).message, /codex app-server isn't running/, "and what was about to ask it too");
  const res = await client.request("thread/list", {});
  assert.deepEqual(res.data.map((t) => t.id), ["t1"]);
  assert.equal(machine.spawned.length, 2, "a new one");
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(client.state, "running", "the old one's exit, later, changes nothing");
  assert.deepEqual((await client.request("thread/list", {})).data.map((t) => t.id), ["t1"]);
  client.stop();
});
