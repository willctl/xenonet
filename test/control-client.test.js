// The client for XenoNet Dev's control channel (core/control.js), against a
// stand-in for an instance's server, and launching and quitting on a
// made-up machine. Starting XenoNet Dev itself is for the end-to-end tests.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet, electronCommand, lockHolder, userDataDir, INSTANCES } from "../core/control.js";
import { fakeProcesses } from "./harness/procs.js";

// Temp folders, and what the made-up machine opens (lock sockets, the
// copies of XenoNet Dev it starts), taken away after the tests, even after
// one that fails partway.
const made = [];
const open = [];
const tmp = (within = fs.realpathSync.native(os.tmpdir())) => {
  const dir = fs.mkdtempSync(path.join(within, "xenonet-ctl-"));
  made.push(dir);
  return dir;
};
after(async () => {
  for (const close of open) await close();
  for (const dir of made) fs.rmSync(dir, { recursive: true, force: true });
});
// Chromium's lock is a link and a Unix socket on macOS and Linux; Windows
// locks another way.
const posix = process.platform !== "win32";

const freePort = () =>
  new Promise((resolve) => {
    const probe = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = probe.address();
      probe.close(() => resolve(port));
    });
  });

// An instance's server: health, the control route and simulations. `state`
// says how it behaves (`pid`, the process it says it is; `onQuit`, what
// happens when it's told to quit, beyond its server closing); `asked`
// records the control requests.
async function instance(state = {}, port = 0) {
  Object.assign(state, { healthy: true, controlled: true, ready: true, visible: true, noDriver: 0, ...state });
  const asked = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const json = (status, data) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
      if (req.url === "/api/health") return state.healthy ? json(200, { ok: true }) : json(503, {});
      if (req.url === "/api/live") return json(200, { sessions: [] });
      const b = body ? JSON.parse(body) : {};
      if (req.url === "/api/dev/simulate") return b.kind === "stuck" ? json(200, { ok: true, kind: b.kind }) : json(400, { error: "unknown simulation or session" });
      if (req.url !== "/api/dev/control") return json(404, { error: "not found" });
      asked.push(b);
      if (state.cut) return req.socket.destroy();
      if (!state.controlled) return json(503, { error: "this server has no window to control" });
      if (b.op === "ping") return json(200, { instance: "demo", ...(state.pid ? { pid: state.pid } : {}), page: { ready: state.ready }, window: { visible: state.visible } });
      if (b.op === "quit" && state.stubborn) return json(200, { quitting: true });
      if (b.op === "quit") {
        // Its server goes a moment after it answers, as XenoNet Dev's does.
        state.healthy = false;
        state.onQuit?.();
        setTimeout(() => (server.closeAllConnections(), server.close()), 50);
        return json(200, { quitting: true });
      }
      if (b.op === "driver" && state.noDriver > 0) return state.noDriver--, json(400, { error: "the page has no driver yet" });
      if (b.op === "driver" && b.method === "boom") return json(400, { error: "the driver has no boom()" });
      json(200, b);
    });
  });
  await new Promise((r) => server.listen(port, "127.0.0.1", r));
  return { server, asked, state, port: server.address().port, close: () => new Promise((r) => server.close(r)) };
}

test("an instance by name, on its own port or another", () => {
  assert.equal(xenonet("demo").port, INSTANCES.demo.port);
  assert.equal(xenonet("test", { port: 5999 }).base, "http://127.0.0.1:5999");
  assert.throws(() => xenonet("prod"), /no instance "prod"; there's dev, demo, test, film/);
  const { command, args } = electronCommand();
  assert.ok(typeof command === "string" && command.length > 0 && Array.isArray(args));
  // Without XenoNet Dev.app in ~/Applications, the Electron npm installed.
  const home = process.env.HOME;
  process.env.HOME = os.tmpdir();
  try {
    const plain = electronCommand();
    assert.match(plain.command, /electron/i);
    assert.equal(plain.args.length, 1);
  } finally {
    process.env.HOME = home;
  }
});

test("calls reach the control route, and its errors come back as errors", async () => {
  const i = await instance();
  const s = xenonet("demo", { port: i.port });
  try {
    assert.equal(await s.health(), true);
    assert.deepEqual(await s.driver("select", "project:x", { timeout: 5 }), { op: "driver", method: "select", args: ["project:x", { timeout: 5 }], timeout: 5 });
    assert.deepEqual(await s.screenshot({ name: "x" }), { op: "screenshot", name: "x" });
    assert.deepEqual(await s.input({ keys: "Escape" }), { op: "input", keys: "Escape" });
    assert.deepEqual(await s.window({ hide: true }), { op: "window", hide: true });
    assert.deepEqual(await s.logs(), { op: "logs" });
    assert.deepEqual(await s.eval("1+1"), { op: "eval", expression: "1+1" });
    assert.deepEqual(await s.record({ action: "start" }), { op: "record", action: "start" });
    assert.deepEqual(await s.record({ action: "stop" }), { op: "record", action: "stop", timeout: 120000 });
    assert.deepEqual(await s.api("/api/live"), { sessions: [] });
    assert.deepEqual(await s.simulate("stuck"), { ok: true, kind: "stuck" });
    await assert.rejects(s.simulate("earthquake"), /unknown simulation/);
    await assert.rejects(s.driver("boom"), /the driver has no boom\(\)/);
  } finally {
    await i.close();
  }
});

test("a page reloading has no driver for a moment: the call waits and tries once more", async () => {
  const i = await instance({ noDriver: 1 });
  try {
    assert.deepEqual((await xenonet("demo", { port: i.port }).driver("state")).method, "state");
    assert.equal(i.asked.filter((b) => b.op === "driver").length, 2);
  } finally {
    await i.close();
  }
});

test("ping, ready and launch: a running instance, one that isn't, and a server with no window", async () => {
  const i = await instance({ visible: false });
  const s = xenonet("demo", { port: i.port });
  try {
    assert.equal((await s.ping()).page.ready, true);
    const launched = await s.launch({ show: "inactive" });
    assert.equal(launched.page.ready, true);
    assert.deepEqual(i.asked.find((b) => b.op === "window"), { op: "window", show: "inactive" }, "a hidden window is brought out");
    i.state.ready = false;
    await assert.rejects(s.ready({ timeout: 700 }), /wasn't ready after 0.7s/);
    i.state.controlled = false;
    assert.deepEqual(await s.ping(), { instance: "demo", name: INSTANCES.demo.name, port: i.port, controlled: false, error: "this server has no window to control" });
    await assert.rejects(s.ready(), /has a XenoNet server with no window to control/);
    await assert.rejects(s.launch(), /has a XenoNet server with no window \(npm run dev or npm run demo\?\)/);
  } finally {
    await i.close();
  }
  assert.equal(await s.ping(), null, "nothing running");
  assert.equal(await s.health(), false);
});

test("quit: asks, then waits until the instance is gone", async () => {
  const i = await instance();
  const s = xenonet("demo", { port: i.port });
  try {
    assert.deepEqual(await s.quit(), { quit: true });
    assert.deepEqual(await s.quit(), { quit: false, running: false });
  } finally {
    await i.close();
  }
  // One that stays up says so.
  const stubborn = await instance({ stubborn: true });
  try {
    await assert.rejects(xenonet("demo", { port: stubborn.port }).quit({ timeout: 700 }), /didn't quit in 0.7s/);
  } finally {
    await stubborn.close();
  }
});

test("a server restarting is waited for; one that cuts a request off isn't asked twice", async () => {
  const port = await freePort();
  const s = xenonet("demo", { port });
  await assert.rejects(s.call({ op: "ping" }, { wait: 0 }), /isn't running \(nothing on port \d+\); launch it first/);
  // The server comes back while the call waits.
  const late = new Promise((r) => setTimeout(() => instance({}, port).then(r), 500));
  assert.deepEqual(await s.call({ op: "logs" }, { wait: 10000 }), { op: "logs" });
  const i = await late;
  i.state.cut = true;
  await assert.rejects(s.call({ op: "eval", expression: "x" }), /lost the connection to XenoNet Dev \(demo\) mid-request/);
  assert.equal(i.asked.filter((b) => b.op === "eval").length, 1);
  await i.close();
});

// A made-up machine for launch and quit: its processes (the harness's
// table), what each does with a signal, and the signals sent.
function fakeMachine() {
  const m = fakeProcesses();
  const signals = [];
  const handlers = new Map();
  const system = {
    alive: (pid) => m.table.has(pid),
    kill: (pid, signal) => (signals.push([pid, signal]), handlers.get(pid)?.(signal)),
    spawn: m.spawn,
    command: () => ({ command: "electron", args: ["/src/xenonet"] }),
  };
  return { m, signals, handlers, system };
}

// Chromium's single-instance lock in a data folder, held by `pid`: the link
// naming the host and pid, and the socket a second launch hands off to,
// kept in a short temporary folder with a link to it. A lock left by a
// process that's gone is taken over. release() takes both away, as a clean
// exit does; drop() only closes the socket and leaves the link, as a killed
// process does.
async function holdLock(dir, pid) {
  // A socket's path has room for about a hundred bytes.
  const within = fs.realpathSync.native(os.tmpdir());
  const socket = path.join(tmp(within.length > 60 ? "/tmp" : within), "SingletonSocket");
  const server = net.createServer((c) => c.destroy());
  await new Promise((r) => server.listen(socket, r));
  const drop = () => new Promise((r) => server.close(() => r()));
  open.push(drop);
  for (const [name, to] of [["SingletonLock", `made-up-host-${pid}`], ["SingletonSocket", socket]]) {
    fs.rmSync(path.join(dir, name), { force: true });
    fs.symlinkSync(to, path.join(dir, name));
  }
  return {
    drop,
    release: async () => {
      await drop();
      for (const name of ["SingletonLock", "SingletonSocket"]) fs.rmSync(path.join(dir, name), { force: true });
    },
  };
}

// XenoNet Dev starting on the made-up machine: a copy that finds the lock held
// hands its launch to the holder and quits at once, as Chromium does;
// otherwise it takes the lock and serves on its port until it's told to
// quit or killed. Returns the pids that got that far.
function fakeXenoNetDev(fm, dir) {
  const started = [];
  fm.m.program("electron", async (io) => {
    if (await lockHolder(dir, { alive: fm.system.alive })) return { code: 0 };
    const lock = await holdLock(dir, io.pid);
    let i = null;
    const exit = async (signal = null) => {
      await i?.close();
      await lock.release();
      io.exit(signal ? null : 0, signal);
    };
    i = await instance({ pid: io.pid, onQuit: () => setTimeout(exit, 100) }, Number(io.env.XENONET_PORT));
    open.push(() => exit());
    fm.handlers.set(io.pid, (signal) => exit(signal));
    started.push(io.pid);
  });
  return started;
}

test("where an instance keeps its data, and who holds its lock: a live process on its socket, not a link left behind", { skip: !posix }, async () => {
  assert.equal(userDataDir("XenoNet Dev (demo)", { platform: "darwin", home: "/Users/nova" }), "/Users/nova/Library/Application Support/XenoNet Dev (demo)");
  assert.equal(userDataDir("XenoNet Dev", { platform: "linux", home: "/home/nova", env: {} }), "/home/nova/.config/XenoNet Dev");
  assert.equal(userDataDir("XenoNet Dev", { platform: "linux", home: "/home/nova", env: { XDG_CONFIG_HOME: "/xdg" } }), "/xdg/XenoNet Dev");
  assert.equal(userDataDir("XenoNet Dev", { platform: "win32", home: "C:\\Users\\nova", env: { APPDATA: "D:\\Roaming" } }), path.join("D:\\Roaming", "XenoNet Dev"));

  const fm = fakeMachine();
  const alive = fm.system.alive;
  const dir = tmp();
  assert.equal(await lockHolder(dir, { alive }), null, "no lock");
  const pid = fm.m.add({ program: "/Applications/Electron.app/Contents/MacOS/Electron" });
  const lock = await holdLock(dir, pid);
  assert.equal(await lockHolder(dir, { alive }), pid);
  await lock.drop();
  assert.equal(await lockHolder(dir, { alive }), null, "a link nobody listens behind, though its pid is in use (another program's, by now)");
  fm.m.remove(pid);
  assert.equal(await lockHolder(dir, { alive }), null, "a pid that's gone");
  await lock.release();
  assert.equal(await lockHolder(dir, { alive }), null);
});

test("quit returns once the instance's process has exited, not when its server stops", async () => {
  const fm = fakeMachine();
  const pid = fm.m.add({ program: "Electron" });
  let exited = false;
  const i = await instance({ pid, onQuit: () => setTimeout(() => (fm.m.remove(pid), (exited = true)), 700) });
  const s = xenonet("demo", { port: i.port, userData: tmp(), system: fm.system });
  try {
    assert.deepEqual(await s.quit(), { quit: true });
    assert.equal(exited, true, "a launch now isn't handed to a copy on its way out");
    assert.deepEqual(fm.signals, []);
  } finally {
    await i.close();
  }
});

test("quit kills a process stuck on its way out, once its server is gone", async () => {
  const fm = fakeMachine();
  const pid = fm.m.add({ program: "Electron" });
  // Quitting already, it takes SIGTERM as one more request to quit.
  fm.handlers.set(pid, (signal) => signal === "SIGKILL" && fm.m.remove(pid));
  const i = await instance({ pid });
  const s = xenonet("demo", { port: i.port, userData: tmp(), system: fm.system });
  try {
    assert.deepEqual(await s.quit({ timeout: 800 }), { quit: true, killed: pid });
    assert.deepEqual(fm.signals, [[pid, "SIGKILL"]]);
  } finally {
    await i.close();
  }
});

test("launch waits for a copy on its way out to let go of the lock, then starts its own", { skip: !posix }, async () => {
  const fm = fakeMachine();
  const dir = tmp();
  const old = fm.m.add({ program: "Electron" });
  const lock = await holdLock(dir, old);
  setTimeout(() => (fm.m.remove(old), lock.release()), 600);
  const started = fakeXenoNetDev(fm, dir);
  const s = xenonet("demo", { port: await freePort(), userData: dir, logDir: tmp(), system: fm.system });
  assert.equal((await s.launch({ show: "hidden" })).page.ready, true);
  assert.deepEqual([fm.m.spawned.length, started.length], [1, 1], "one copy, which got the lock rather than hand off and quit");
  assert.deepEqual(await s.quit(), { quit: true });
  assert.equal(await lockHolder(dir, { alive: fm.system.alive }), null);
});

test("launch takes a copy that comes up meanwhile, rather than start another", { skip: !posix }, async () => {
  const fm = fakeMachine();
  const dir = tmp();
  const port = await freePort();
  const other = fm.m.add({ program: "Electron" });
  await holdLock(dir, other);
  const coming = new Promise((resolve) => setTimeout(() => resolve(instance({ pid: other }, port)), 600));
  fakeXenoNetDev(fm, dir);
  const s = xenonet("demo", { port, userData: dir, logDir: tmp(), system: fm.system });
  try {
    assert.equal((await s.launch({ show: "hidden" })).page.ready, true);
    assert.equal(fm.m.spawned.length, 0);
  } finally {
    await (await coming).close();
  }
});

test("launch reports a dev copy that holds the lock without answering, and restart ends it and says so", { skip: !posix }, async () => {
  const fm = fakeMachine();
  const dir = tmp();
  const port = await freePort();
  const stuck = fm.m.add({ program: "Electron" });
  const lock = await holdLock(dir, stuck);
  // Stuck on its way out: SIGTERM only asks it to quit again, and SIGKILL
  // leaves its lock behind.
  fm.handlers.set(stuck, (signal) => signal === "SIGKILL" && (fm.m.remove(stuck), lock.drop()));
  const started = fakeXenoNetDev(fm, dir);
  // The dev instance shows your own sessions, so launch leaves its copy
  // alone unless told to restart.
  const s = xenonet("dev", { port, userData: dir, logDir: tmp(), system: fm.system, settleMs: 600, termMs: 300 });
  await assert.rejects(s.launch({ show: "hidden" }), new RegExp(`^Error: process ${stuck} holds XenoNet Dev's single-instance lock but doesn't answer on port ${port}: .* kill ${stuck}, then kill -9 ${stuck} if it stays\\.$`));
  assert.deepEqual([fm.signals, fm.m.spawned.length], [[], 0], "reported, and left alone");

  const launched = await s.launch({ show: "hidden", restart: true });
  assert.equal(launched.page.ready, true);
  assert.deepEqual(launched.ended, [{ pid: stuck, reason: `it held XenoNet Dev's single-instance lock without answering on port ${port}` }]);
  assert.deepEqual(fm.signals, [[stuck, "SIGTERM"], [stuck, "SIGKILL"]]);
  assert.equal(started.length, 1);
  assert.deepEqual(await s.quit(), { quit: true });
});

test("launch ends a demo or test copy that holds the lock without answering by itself, once it's had its time, and says so", { skip: !posix }, async () => {
  for (const id of ["demo", "test"]) {
    const fm = fakeMachine();
    const dir = tmp();
    const port = await freePort();
    const stuck = fm.m.add({ program: "Electron" });
    const lock = await holdLock(dir, stuck);
    // Stuck on an error page, say: SIGTERM ends it, and its lock's link
    // stays behind.
    fm.handlers.set(stuck, () => (fm.m.remove(stuck), lock.drop()));
    const started = fakeXenoNetDev(fm, dir);
    const s = xenonet(id, { port, userData: dir, logDir: tmp(), system: fm.system, settleMs: 600, termMs: 300 });
    const t0 = Date.now();
    const launched = await s.launch({ show: "hidden" });
    assert.ok(Date.now() - t0 >= 600, `${id}: a copy on its way in or out has its time first`);
    assert.equal(launched.page.ready, true);
    assert.deepEqual(launched.ended, [{ pid: stuck, reason: `it held ${INSTANCES[id].name}'s single-instance lock without answering on port ${port}` }], id);
    assert.deepEqual([fm.signals, started.length], [[[stuck, "SIGTERM"]], 1], id);
    // A launch that ends nothing says nothing of it.
    assert.equal("ended" in (await s.launch({ show: "hidden" })), false);
    assert.deepEqual(await s.quit(), { quit: true });
  }
});

test("launch says so when its port is taken by something that isn't XenoNet Dev, rather than start into it", async () => {
  const fm = fakeMachine();
  const other = http.createServer((_req, res) => res.writeHead(503).end());
  await new Promise((r) => other.listen(0, "127.0.0.1", r));
  const s = xenonet("demo", { port: other.address().port, userData: tmp(), logDir: tmp(), system: fm.system, settleMs: 300 });
  try {
    await assert.rejects(s.launch(), new RegExp(`something on port ${other.address().port} doesn't answer`));
    assert.equal(fm.m.spawned.length, 0);
  } finally {
    other.closeAllConnections();
    await new Promise((r) => other.close(r));
  }
});
