// Drives XenoNet Dev from scripts, tests and SIs: starts an instance
// (see instances.js), talks to its control channel
// (electron/control.js, reached through POST /api/dev/control on the
// instance's own server) and quits it. The console MCP server
// (mcp/console.js), xenonet console and the end-to-end tests (test/e2e) all go
// through here.
//
//   import { xenonet } from "./control.js";
//   const s = xenonet("demo");
//   await s.launch();                                  // starts it if needed, waits until it's ready
//   const { nodes } = await s.driver("find", { query: "atlas-api", kind: "project" });
//   await s.driver("select", nodes[0].id);             // returns once the view has settled
//   await s.screenshot({ name: "atlas" });             // .xenonet/atlas.png
import { spawn } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import { installedConsole } from "./components/console.js";
import { INSTANCES } from "./instances.js";

export { INSTANCES };
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// How to start XenoNet Dev from this checkout: XenoNet Dev.app on a Mac when
// `npm run dev:install` put it in place for this checkout (it carries the
// name and icon), otherwise the Electron that npm installed.
export function electronCommand() {
  const bundle = path.join(os.homedir(), "Applications", "XenoNet Dev.app");
  const pointer = path.join(bundle, "Contents", "Resources", "app", "main.js");
  try {
    if (process.platform === "darwin" && fs.readFileSync(pointer, "utf8").includes(pathToFileURL(path.join(root, "electron", "main.js")).href)) return { command: path.join(bundle, "Contents", "MacOS", "Electron"), args: [] };
  } catch {}
  return { command: createRequire(import.meta.url)("electron"), args: [root] };
}

// Where an instance keeps its Electron data, its single-instance lock among
// it: electron/main.js gives each instance a folder of its name in the
// system's app data folder (Electron's appData).
export function userDataDir(name, { platform = process.platform, env = process.env, home = os.homedir() } = {}) {
  if (platform === "darwin") return path.join(home, "Library", "Application Support", name);
  if (platform === "win32") return path.join(env.APPDATA ?? path.join(home, "AppData", "Roaming"), name);
  return path.join(env.XDG_CONFIG_HOME || path.join(home, ".config"), name);
}

// What launch and quit ask of the system, which tests stand in for: whether
// a process is still there, a signal to one, and starting XenoNet Dev.
const system = {
  alive(pid) {
    try {
      process.kill(pid, 0);
      return true;
    } catch (err) {
      return err.code === "EPERM";
    }
  },
  kill(pid, signal) {
    try {
      process.kill(pid, signal);
    } catch {}
  },
  spawn,
  command: electronCommand,
};

// The process holding an instance's single-instance lock, or null. Chromium
// keeps the lock in the data folder: a link, SingletonLock, naming the host
// and pid that hold it ("mac.local-4321"), and the socket a second launch
// hands its arguments to before it quits (SingletonSocket). A process that
// died leaves both behind, and its pid may since be another program's, so
// only a live process listening on the socket counts. Connecting and
// sending nothing costs the holder a line in its log ("Invalid socket
// message"), not a second-instance event. Windows has neither file, so
// there it's always null.
export async function lockHolder(dir, { alive = system.alive } = {}) {
  let link;
  try {
    link = fs.readlinkSync(path.join(dir, "SingletonLock"));
  } catch {
    return null;
  }
  const pid = Number(/-(\d+)$/.exec(link)?.[1]);
  if (!(pid > 0) || !alive(pid)) return null;
  // The socket's link points into a temporary folder: a path short enough
  // for a socket, where the data folder's might not be.
  let socket = path.join(dir, "SingletonSocket");
  try {
    socket = fs.realpathSync(socket);
  } catch {}
  const answers = await new Promise((resolve) => {
    const conn = net.connect(socket);
    const done = (ok) => (conn.destroy(), resolve(ok));
    conn.once("connect", () => done(true));
    conn.once("error", () => done(false));
    conn.setTimeout(1000, () => done(false));
  });
  return answers ? pid : null;
}

// `port` runs the instance on another port than its own, as the end-to-end
// tests do so that nothing else drives their copy. The rest stand in for the
// machine in tests: `userData` (where the instance's lock is), `logDir`
// (where launch writes its output), `system` (see above), `settleMs` (how
// long launch waits for a copy on its way in or out) and `termMs` (how long
// a copy launch ends has after SIGTERM, before SIGKILL).
export function xenonet(id = "demo", { port = INSTANCES[id]?.port, userData, logDir = path.join(root, ".xenonet", "logs"), system: sys = system, settleMs = 15000, termMs = 5000 } = {}) {
  if (id === "installed") return installedConsole({ env: { ...process.env, XENONET_PORT: String(process.env.XENONET_PORT || port) } });
  if (!INSTANCES[id]) throw new Error(`no instance "${id}"; there's ${Object.keys(INSTANCES).join(", ")}`);
  // A copy with made-up data only (demo, test) loses nothing when launch
  // ends it; dev and film show your own sessions.
  const { name, demo: madeUp } = INSTANCES[id];
  const base = `http://127.0.0.1:${port}`;
  const lockDir = userData ?? userDataDir(name);

  // What's on the port: "up" (a XenoNet answers), "down" (nothing listens, so
  // the connection is refused) or "busy" (something listens but doesn't
  // answer, such as a server on its way out).
  async function probe() {
    try {
      return (await fetch(`${base}/api/health`, { signal: AbortSignal.timeout(2000) })).ok ? "up" : "busy";
    } catch (err) {
      return err.cause?.code === "ECONNREFUSED" ? "down" : "busy";
    }
  }
  const health = async () => (await probe()) === "up";

  // Until `pid` has exited, for `ms` at most; whether it did.
  async function gone(pid, ms) {
    for (const until = Date.now() + ms; sys.alive(pid); await sleep(100)) if (Date.now() >= until) return false;
    return true;
  }

  // What's said of a copy that holds the lock without answering here.
  const held = (pid) =>
    `process ${pid} holds ${name}'s single-instance lock but doesn't answer on port ${port}: it's stuck (on its way out, or on an error page) or running on another port, and a new copy would quit at once. Launch with restart to end it, or end it yourself: kill ${pid}, then kill -9 ${pid} if it stays.`;

  // One control request. While the server restarts (an edit under server/
  // does that), the request waits for it to come back, for up to `wait` ms;
  // it's sent only once, so nothing runs twice.
  async function call(body, { wait = 20000 } = {}) {
    const until = Date.now() + wait;
    for (;;) {
      let res;
      try {
        res = await fetch(`${base}/api/dev/control`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
      } catch (err) {
        if (err.cause?.code === "ECONNREFUSED" && Date.now() < until) {
          await sleep(400);
          continue;
        }
        // Sent, then cut off: it may have run, so it isn't sent again.
        throw new Error(err.cause?.code === "ECONNREFUSED" ? `${name} isn't running (nothing on port ${port}); launch it first` : `lost the connection to ${name} mid-request (its server restarted?): ${err.cause?.message ?? err.message}`);
      }
      const data = await res.json().catch(() => ({ error: `${res.status} ${res.statusText}` }));
      if (!res.ok) throw new Error(data?.error ?? `${res.status} ${res.statusText}`);
      return data;
    }
  }

  // The instance and its window, or null if it isn't running.
  async function ping() {
    if (!(await health())) return null;
    try {
      return await call({ op: "ping" }, { wait: 0 });
    } catch (err) {
      return { instance: id, name, port, controlled: false, error: err.message };
    }
  }

  async function ready({ timeout = 60000 } = {}) {
    const until = Date.now() + timeout;
    let last = null;
    while (Date.now() < until) {
      last = await ping();
      if (last?.page?.ready) return last;
      if (last && last.controlled === false) throw new Error(`port ${port} has a XenoNet server with no window to control (${last.error})`);
      await sleep(500);
    }
    throw new Error(`${name} wasn't ready after ${timeout / 1000}s${last ? `: ${JSON.stringify(last.page)}` : ""}`);
  }

  // Starts the instance unless it's running, and waits until its page is
  // ready. `show`: "inactive" brings the window up without taking focus,
  // "front" focuses it, "hidden" never shows it (the default for test).
  // `restart` quits it first, and ends a copy stuck holding its lock (see
  // settle). What it returns says what launch ended on the way, if
  // anything: `ended`, each copy's pid and why.
  async function launch({ show = id === "test" ? "hidden" : "inactive", restart = false, timeout = 120000 } = {}) {
    // An unknown word would reach the window as XENONET_SHOW, which shows it
    // in front: refused here instead.
    if (!["inactive", "front", "hidden"].includes(show)) throw new Error(`a window shows inactive, front or hidden, not "${show}"`);
    const ended = [];
    const done = (p) => (ended.length ? { ...p, ended } : p);
    let running = await ping();
    if (running && restart) {
      await quit();
      running = null;
    }
    if (running?.controlled === false) throw new Error(`port ${port} has a XenoNet server with no window (npm run dev or npm run demo?); stop it, or use another instance`);
    running ??= await settle({ restart, ended });
    if (running) {
      if (show !== "hidden" && !running.window?.visible) await call({ op: "window", show });
      return done(await ready({ timeout }));
    }
    const logFile = path.join(logDir, `${id}.log`);
    fs.mkdirSync(logDir, { recursive: true });
    const out = fs.openSync(logFile, "a");
    const env = { ...process.env, XENONET_INSTANCE: id, XENONET_SHOW: show };
    // Set by Electron apps for their child processes; XenoNet Dev would start
    // as plain Node.
    delete env.ELECTRON_RUN_AS_NODE;
    for (const k of ["XENONET_DEMO", "XENONET_OFFSCREEN", "XENONET_CONTROL"]) delete env[k];
    env.XENONET_PORT = String(port);
    // Muted, unless XENONET_MUTED=0 says otherwise; the film and showcase
    // instances keep their sound, for recordings.
    if (["dev", "demo", "test"].includes(id)) env.XENONET_MUTED ??= "1";
    const { command, args } = sys.command();
    const child = sys.spawn(command, args, { cwd: root, env, detached: true, windowsHide: true, stdio: ["ignore", out, out] });
    child.unref();
    fs.closeSync(out);
    let exited = null;
    child.on("exit", (code, signal) => (exited = code ?? signal));
    const until = Date.now() + timeout;
    while (Date.now() < until) {
      const p = await ping();
      if (p?.page?.ready) return done(p);
      if (exited !== null && !p) {
        // It handed off to a copy that took the lock in the meantime.
        const holder = await lockHolder(lockDir, { alive: sys.alive });
        if (holder) throw new Error(`${name} quit at once: ${held(holder)}`);
        const tail = fs.readFileSync(logFile, "utf8").trim().split("\n").slice(-12).join("\n");
        throw new Error(`${name} exited (${exited}) before it was ready. The end of ${path.relative(root, logFile)}:\n${tail}`);
      }
      await sleep(500);
    }
    throw new Error(`${name} wasn't ready after ${timeout / 1000}s; see ${path.relative(root, logFile)}`);
  }

  // Before a new copy starts, the port has to be free and the instance's
  // lock not held: a copy holding the lock takes a new one's launch and
  // quits it, and a server still on the port is in its way. A copy on its
  // way in (launched from elsewhere) or out (quitting) gets `settleMs` to
  // get there. One that holds the lock all that time without answering here
  // is stuck: it's ended when its data is made up (demo, test), and
  // reported otherwise, for a launch with restart to end, which doesn't
  // wait. Each copy ended goes on `ended`. Returns the instance if it came
  // up meanwhile.
  async function settle({ restart, ended }) {
    let until = Date.now() + settleMs;
    for (;;) {
      const state = await probe();
      if (state === "up") {
        const p = await ping();
        if (p && p.controlled !== false) return p;
      }
      const holder = await lockHolder(lockDir, { alive: sys.alive });
      if (state === "down" && !holder) return null;
      const late = Date.now() >= until;
      if (holder && (restart || (late && madeUp))) {
        await end(holder);
        ended.push({ pid: holder, reason: `it held ${name}'s single-instance lock without answering on port ${port}` });
        // Its server gets as long again to let go of the port.
        until = Date.now() + settleMs;
        continue;
      }
      if (late) {
        if (holder) throw new Error(held(holder));
        if (state === "up") throw new Error(`port ${port} has a XenoNet server with no window (npm run dev or npm run demo?); stop it, or use another instance`);
        throw new Error(`something on port ${port} doesn't answer (a server still on its way out, or another program); stop it, or run ${name} on another port`);
      }
      await sleep(300);
    }
  }

  // Ends a copy holding the lock: SIGTERM asks it to quit, which one stuck
  // on its way out ignores (it's quitting already), so SIGKILL follows.
  async function end(pid) {
    sys.kill(pid, "SIGTERM");
    if (await gone(pid, termMs)) return;
    sys.kill(pid, "SIGKILL");
    if (!(await gone(pid, 5000))) throw new Error(`process ${pid} holds ${name}'s single-instance lock and is still there after kill -9`);
  }

  // Quits the instance, and returns once its process has exited and its
  // server has let go of the port. Until then the process holds the
  // single-instance lock, so a copy launched meanwhile would hand off to it
  // and quit, or find the old server still answering. A process still there
  // `timeout` ms after its server went is stuck on its way out (a page that
  // won't close, say) and is killed: SIGTERM would only ask it to quit
  // again, and with its server gone there's nothing left to save.
  async function quit({ timeout = 20000 } = {}) {
    const running = await ping();
    if (!running) return { quit: false, running: false };
    const pid = Number.isInteger(running.pid) ? running.pid : null;
    await call({ op: "quit" }, { wait: 0 }).catch(() => {});
    const until = Date.now() + timeout;
    for (;;) {
      const state = await probe();
      const alive = pid !== null && sys.alive(pid);
      if (state === "down" && !alive) return { quit: true };
      if (Date.now() >= until) {
        if (state === "up") throw new Error(`${name} didn't quit in ${timeout / 1000}s`);
        if (!alive) throw new Error(`${name} quit, but something on port ${port} still doesn't answer`);
        sys.kill(pid, "SIGKILL");
        if (await gone(pid, 5000)) return { quit: true, killed: pid };
        throw new Error(`${name} didn't quit in ${timeout / 1000}s, and process ${pid} is still there after kill -9`);
      }
      await sleep(300);
    }
  }

  // A call into the page's driver (src/driver.js). If the page is reloading
  // (it does after a server restart), it waits for it, then tries once more.
  async function driver(method, ...args) {
    const timeout = args.find((a) => a && typeof a === "object" && "timeout" in a)?.timeout;
    try {
      return await call({ op: "driver", method, args, timeout });
    } catch (err) {
      if (!/has no driver/.test(err.message)) throw err;
      await ready();
      return call({ op: "driver", method, args, timeout });
    }
  }

  async function simulate(kind, { a, b } = {}) {
    const res = await fetch(`${base}/api/dev/simulate`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ kind, a, b }) });
    const data = await res.json();
    if (!res.ok) throw new Error(data.error ?? res.statusText);
    return data;
  }

  return {
    id,
    name,
    port,
    base,
    health,
    call,
    ping,
    ready,
    launch,
    quit,
    driver,
    simulate,
    screenshot: (opts = {}) => call({ op: "screenshot", ...opts }),
    input: (opts) => call({ op: "input", ...opts }),
    window: (opts) => call({ op: "window", ...opts }),
    // Records the window to an MP4: { action: "start", file?, size?, width?,
    // fps?, keep? }, then { action: "stop" }, which gives the window its own
    // size back unless keep was set.
    record: (opts) => call({ op: "record", ...opts, ...(opts?.action === "stop" ? { timeout: 120000 } : {}) }),
    logs: (opts = {}) => call({ op: "logs", ...opts }),
    eval: (expression, opts = {}) => call({ op: "eval", expression, ...opts }),
    // Any read-only API route, such as /api/live.
    api: async (route) => (await fetch(`${base}${route}`)).json(),
  };
}
