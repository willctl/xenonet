import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";
import { connectService, runtimeDir } from "./runtime.js";
import { CONSOLE_API, negotiate } from "./protocol.js";

// An MCP process survives a Desktop restart. Each action finds the service
// again, and an interrupted action is never automatically sent twice.
export function installedConsole({ env = process.env } = {}) {
  const noDesktop = () => ({ instance: "installed", name: "XenoNet", controlled: false, service: true, page: { ready: false }, error: "XenoNet Desktop is closed or restarting." });
  async function call(body) {
    const client = await connectService({ env });
    try {
      negotiate(CONSOLE_API, client.info.consoleApi);
      return await client.request("console", body);
    } finally { client.close(); }
  }
  async function ping() {
    let client;
    try {
      client = await connectService({ env });
      if (!client.info.desktop) return noDesktop();
      return await client.request("console", { op: "ping" });
    } catch (err) {
      // Desktop may disconnect between the hello response and this read-only probe.
      if (/^XenoNet Desktop is closed or restarting\./.test(err.message)) return noDesktop();
      if (err.code === "ENOENT" || /disconnect/.test(err.message)) return null;
      throw err;
    } finally { client?.close(); }
  }
  async function ready({ timeout = 60000 } = {}) {
    for (const until = Date.now() + timeout; Date.now() < until;) {
      const state = await ping();
      if (state?.page?.ready) return state;
      await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error("XenoNet Desktop did not become ready.");
  }
  async function quit() {
    const state = await ping();
    if (!state?.page) return { quit: false };
    if (state.controlled === false) return { quit: false, service: true };
    await call({ op: "quit" });
    for (const until = Date.now() + 20000; Date.now() < until;) {
      const p = await ping();
      if (!p || p.controlled === false) return { quit: true, service: true };
      await new Promise((r) => setTimeout(r, 100));
    }
    throw new Error("XenoNet Desktop did not close. The service is still running.");
  }
  async function launch({ show = "inactive", restart = false, timeout } = {}) {
    if (restart) await quit();
    if ((await ping())?.page?.ready) { await call({ op: "window", ...(show === "hidden" ? {} : { show }) }); return ready({ timeout }); }
    let desktop;
    try { desktop = JSON.parse(fs.readFileSync(path.join(runtimeDir(env), "desktop.json"), "utf8")); }
    catch { throw new Error("Open XenoNet once to register Desktop, or install the XenoNet bundle."); }
    const childEnv = { ...env, XENONET_SHOW: show };
    delete childEnv.ELECTRON_RUN_AS_NODE;
    const child = spawn(desktop.command, desktop.args, { cwd: desktop.cwd, env: childEnv, detached: true, windowsHide: true, stdio: "ignore" });
    const started = new Promise((resolve, reject) => { child.once("spawn", resolve); child.once("error", reject); });
    child.unref();
    await started;
    return ready({ timeout });
  }
  const onlyDev = () => { throw new Error("This action is available only in development instances."); };
  return {
    id: "installed", name: "XenoNet", port: Number(env.XENONET_PORT || 47840), call, ping, ready, launch, quit,
    health: async () => !!(await ping()),
    driver: (method, ...args) => call({ op: "driver", method, args }),
    screenshot: (opts) => call({ op: "screenshot", ...opts }),
    input: (opts) => call({ op: "input", ...opts }),
    window: (opts) => call({ op: "window", ...opts }),
    logs: (opts) => call({ op: "logs", ...opts }),
    record: (opts) => call({ op: "record", ...opts }),
    eval: onlyDev, simulate: onlyDev,
    api: async (route) => (await fetch(`http://127.0.0.1:${env.XENONET_PORT || 47840}${route}`)).json(),
  };
}
