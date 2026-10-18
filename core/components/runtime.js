import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { spawn } from "node:child_process";
import { appDir } from "../dirs.js";
import { pathStyle } from "../paths.js";
import { rpc } from "./rpc.js";
import { SERVICE_API, negotiate } from "./protocol.js";

export const runtimeDir = (env = process.env) => env.XENONET_RUNTIME_DIR || path.join(appDir("state", { env }), "runtime");
export const descriptorFile = (env = process.env) => path.join(runtimeDir(env), `service-${env.XENONET_PORT || 47840}.json`);

// The xenonet binary's name, and the launcher's own copy of it, which
// never moves (docs/design/contracts.md, section 12).
export const binName = (platform = process.platform) => (platform === "win32" ? "xenonet.exe" : "xenonet");
export const launcherBin = (env = process.env, platform = process.platform) => path.join(runtimeDir(env), "launcher", binName(platform));

// The binary that what XenoNet writes into other programs runs: hooks, MCP
// servers, the command on PATH, a local host's relay, a tunnel's adapter.
// XENONET_BIN when it's set; installed XenoNet's launcher
// (XENONET_LAUNCHER_BIN, or the binary beside the launcher in
// XENONET_LAUNCHER_ROOT); else this checkout's build.
export function xenonetBin({ env = process.env, root, platform = process.platform, exists = fs.existsSync } = {}) {
  if (env.XENONET_BIN) return env.XENONET_BIN;
  if (env.XENONET_LAUNCHER_BIN) return env.XENONET_LAUNCHER_BIN;
  if (env.XENONET_LAUNCHER_ROOT) return pathStyle(env.XENONET_LAUNCHER_ROOT).join(env.XENONET_LAUNCHER_ROOT, binName(platform));
  const built = ["release", "debug"].map((profile) => pathStyle(root).join(root, "target", profile, binName(platform)));
  return built.find((f) => exists(f)) ?? built[1];
}

export async function connectService({ env = process.env, role = "tools", handle } = {}) {
  const descriptor = JSON.parse(fs.readFileSync(descriptorFile(env), "utf8"));
  if (!Number.isInteger(descriptor.port) || descriptor.port < 1 || descriptor.port > 65535 || !/^[a-f0-9]{64}$/.test(descriptor.token)) throw new Error("Invalid XenoNet service connection file.");
  const socket = net.connect({ host: "127.0.0.1", port: descriptor.port });
  const client = rpc(socket, handle, { incomingEvents: role === "desktop" ? ["notification"] : [] });
  try {
    client.info = await client.request("hello", { token: descriptor.token, role, api: SERVICE_API }, 5000);
    negotiate(SERVICE_API, client.info.api);
    return client;
  } catch (err) { client.close(); throw err; }
}

// `args`: what `exec` runs, server/index.js under `root` unless they're
// given (the Rust binary's ["serve"]).
export async function ensureService({ root, exec, args = null, env = process.env, timeout = 60000 }) {
  try { const c = await connectService({ env }); c.close(); return "reused"; } catch (err) {
    if (/incompatible/.test(err.message)) throw err;
  }
  const dir = runtimeDir(env);
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const out = fs.openSync(path.join(dir, `service-${env.XENONET_PORT || 47840}.log`), "a", 0o600);
  const childEnv = { ...env, XENONET_STANDALONE: "1" };
  delete childEnv.ELECTRON_RUN_AS_NODE;
  delete childEnv.XENONET_EXEC;
  const child = spawn(exec, args ?? [path.join(root, "server", "index.js")], { cwd: root, env: childEnv, detached: true, windowsHide: true, stdio: ["ignore", out, out] });
  fs.closeSync(out);
  child.unref();
  let failure;
  child.on("error", (err) => { failure = err; });
  child.on("exit", (code) => { failure = new Error(`XenoNet Service exited (${code}). See its service log.`); });
  for (const until = Date.now() + timeout; Date.now() < until;) {
    try { const c = await connectService({ env }); c.close(); return "started"; } catch {}
    if (failure) throw failure;
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error("XenoNet Service did not start. See its service log.");
}
