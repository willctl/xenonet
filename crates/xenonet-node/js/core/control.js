// core/control.js in Rust (crates/xenonet-app/src/control.rs): drives
// XenoNet Dev from scripts, tests and agents: starts an instance, talks to
// its control channel and quits it. Each method is a Rust task; what it
// waits on (the instance's server, the machine's processes, starting
// XenoNet Dev) is done here, through the `system` given.
import { execFile, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { native } from "../index.js";
import { peer } from "../peer.js";
import { drive, fetchAsked } from "../task.js";

const { installedConsole } = await import(peer("components/console.js"));
const { INSTANCES } = await import(peer("instances.js"));

export { INSTANCES };
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..");

export function electronCommand() {
  const { command, args } = native.appElectronCommand(root, os.homedir(), process.platform, process.env.XENONET_SOFTWARE_RENDERING === "1");
  return { command: command ?? createRequire(import.meta.url)("electron"), args };
}

const text = (v) => (v == null ? null : String(v));

export const userDataDir = (name, { platform = process.platform, env = process.env, home = os.homedir() } = {}) => native.appUserDataDir(String(name), String(platform), text(env.APPDATA), text(env.XDG_CONFIG_HOME), String(home));

// What launch and quit ask of the system, which tests stand in for: whether
// a process is still there, a signal to one, the machine's processes (as
// Rust reads ps, or null where ps can't list them), and starting XenoNet
// Dev.
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
  processes() {
    if (process.platform === "win32") return null;
    return new Promise((resolve) =>
      execFile("ps", ["-A", "-o", "pid=,ppid=,lstart="], { env: { ...process.env, LC_ALL: "C", TZ: "UTC" }, timeout: 5000, maxBuffer: 16 << 20 }, (err, stdout) => resolve(err ? null : native.appParseProcesses(stdout))),
    );
  },
  spawn,
  command: electronCommand,
};

export const lockHolder = (dir, { alive = system.alive } = {}) => drive(native.appLockHolder(String(dir)), async (q) => (q.op === "alive" ? alive(q.pid) : undefined));

// What a task asks of the machine, done with `sys`. A launch's child is
// watched for its exit here.
function machine(sys) {
  let exited = null;
  return async (q) => {
    switch (q.op) {
      case "fetch":
        return fetchAsked(q);
      case "alive":
        return sys.alive(q.pid);
      case "kill":
        return sys.kill(q.pid, q.signal);
      case "processes":
        return (await sys.processes?.()) ?? null;
      case "env":
        return { ...process.env };
      case "command":
        return sys.command();
      case "spawn": {
        const out = fs.openSync(q.logFile, "a");
        const child = sys.spawn(q.command, q.args, { cwd: q.cwd, env: q.env, detached: true, windowsHide: true, stdio: ["ignore", out, out] });
        child.unref();
        fs.closeSync(out);
        child.on("exit", (code, signal) => (exited = code ?? signal));
        return null;
      }
      case "exited":
        return exited;
      default:
        throw new Error(`control asked for ${q.op}`);
    }
  };
}

export function xenonet(id = "demo", { port = INSTANCES[id]?.port, userData, logDir = path.join(root, ".xenonet", "logs"), system: sys = system, settleMs = 15000, termMs = 5000 } = {}) {
  if (id === "installed") return installedConsole({ env: { ...process.env, XENONET_PORT: String(process.env.XENONET_PORT || port) } });
  const why = native.appControlCheck(String(id));
  if (why) throw new Error(why);
  const ctl = new native.AppControl(String(id), { port, userData, logDir, settleMs, termMs, root, tmpdir: os.tmpdir(), platform: process.platform, appdata: process.env.APPDATA, xdgConfigHome: process.env.XDG_CONFIG_HOME, home: os.homedir() });
  const run = (kind, args = {}) => drive(ctl.task(kind, args), machine(sys));
  return {
    id,
    name: ctl.name,
    port,
    base: ctl.base,
    health: () => run("health"),
    call: (body, { wait = 20000 } = {}) => run("call", { body, wait }),
    ping: () => run("ping"),
    ready: ({ timeout = 60000 } = {}) => run("ready", { timeout }),
    launch: ({ show, restart = false, timeout = 120000 } = {}) => run("launch", { show, restart, timeout }),
    quit: ({ timeout = 20000 } = {}) => run("quit", { timeout }),
    driver: (method, ...args) => run("driver", { method, args }),
    simulate: (kind, { a, b } = {}) => run("simulate", { kind, a, b }),
    screenshot: (opts = {}) => run("screenshot", { opts }),
    input: (opts) => run("input", { opts }),
    window: (opts) => run("window", { opts }),
    // Records the window to an MP4: { action: "start", file?, size?, width?,
    // fps?, keep? }, then { action: "stop" }.
    record: (opts) => run("record", { opts }),
    logs: (opts = {}) => run("logs", { opts }),
    eval: (expression, opts = {}) => run("eval", { expression, opts }),
    // Any read-only API route, such as /api/live.
    api: (route) => run("api", { route: String(route) }),
  };
}
