import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { currentComponent, stagePackage, selectComponent, newer, withInstallLock } from "./packages.js";
import { runtimeDir, ensureService, connectService } from "./runtime.js";
import { installCommand } from "../command.js";

const nodeName = () => process.platform === "win32" ? "node.exe" : "node";

// This small launcher never moves: hooks, MCP servers, the xenonet command
// and other programs run its path (launcher.cjs says what it does). Each
// invocation chooses a version and starts its runtime; long-lived MCP
// processes keep that version's files.
export const LAUNCHER_SOURCE = new URL("./launcher.cjs", import.meta.url);
// The version a launcher script says it is: 1 for the one before it said.
export const launcherVersion = (text) => Number(/^\/\/ xenonet-launcher (\d+)/.exec(String(text ?? ""))?.[1] ?? (text ? 1 : 0));

// Writes the launcher script when there's none or this XenoNet ships a newer
// one, through a temporary file and a rename, so a hook starting meanwhile
// reads the old or the new, never half of either. One that can't be
// replaced now (a file in use) is replaced next time.
export function writeLauncher(entry, { source = fs.readFileSync(LAUNCHER_SOURCE, "utf8") } = {}) {
  let had = null;
  try {
    had = fs.readFileSync(entry, "utf8");
  } catch {}
  if (had != null && launcherVersion(had) >= launcherVersion(source)) return false;
  fs.mkdirSync(path.dirname(entry), { recursive: true });
  const tmp = `${entry}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(tmp, source);
    fs.renameSync(tmp, entry);
    return true;
  } catch {
    fs.rmSync(tmp, { force: true });
    return false;
  }
}

export function installLauncher(env = process.env) {
  const tools = currentComponent("tools", env);
  if (!tools) return null;
  const dir = path.join(runtimeDir(env), "launcher");
  const entry = path.join(dir, "cli", "xenonet.js");
  const runtime = path.join(dir, nodeName());
  fs.mkdirSync(path.dirname(entry), { recursive: true });
  if (!fs.existsSync(runtime)) fs.copyFileSync(path.join(tools.root, nodeName()), runtime);
  if (process.platform !== "win32") fs.chmodSync(runtime, 0o755);
  writeLauncher(entry);
  return { runtime, entry, root: dir, runAsNode: false };
}

export function checkRuntime(staged) {
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const options = { cwd: staged.root, windowsHide: true, encoding: "utf8", timeout: 15000, env };
  const runtime = path.join(staged.root, nodeName());
  const result = spawnSync(runtime, ["--input-type=module", "-e", "await import('node:sqlite'); await import('@lydell/node-pty'); await import('@modelcontextprotocol/server'); await import('tar'); process.stdout.write('ok')"], options);
  if (result.status !== 0 || result.stdout !== "ok") throw new Error("The new component runtime did not pass its startup check.");
  const cli = spawnSync(runtime, [path.join(staged.root, "cli/xenonet.js"), "--version", "--json"], options);
  let version;
  try { version = JSON.parse(cli.stdout).version; } catch {}
  if (cli.status !== 0 || version !== staged.version) throw new Error("The new component command did not pass its startup check.");
}

const lifecycle = (overrides) => ({ connect: connectService, start: ensureService, check: checkRuntime, now: Date.now, pause: (ms) => new Promise((resolve) => setTimeout(resolve, ms)), stopTimeout: 15000, ...overrides });
const journalFile = (env) => path.join(runtimeDir(env), "service-update.json");
function writeJournal(env, previous, next) {
  const pointer = (value) => value ? { version: value.version, sha256: value.sha256 } : null;
  const file = journalFile(env), tmp = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify({ previous: pointer(previous), next: pointer(next) }), { mode: 0o600 });
  fs.renameSync(tmp, file);
}
function clearJournal(env) { fs.rmSync(journalFile(env), { force: true }); }
function restoreSelection(previous, env) {
  if (previous) selectComponent("service", previous, env);
  else fs.rmSync(path.join(runtimeDir(env), "components/service/current.json"), { force: true });
}
async function running(env, io) {
  const client = await io.connect({ env }).catch((err) => {
    if (/incompatible/.test(err.message)) throw err;
    return null;
  });
  if (!client) return null;
  try { return await client.request("status"); } finally { client.close(); }
}
async function stopService(env, io) {
  const client = await io.connect({ env }).catch(() => null);
  if (!client) return;
  try { await client.request("stop"); } finally { client.close(); }
  const deadline = io.now() + io.stopTimeout;
  while (await running(env, io)) {
    if (io.now() >= deadline) throw new Error("The running service did not stop. Its selected version was left unchanged.");
    await io.pause(100);
  }
}
async function startVersion(version, env, io) {
  await io.start(installedRuntime(env));
  const state = await running(env, io);
  if (state?.version !== version) throw new Error(`Expected XenoNet Service ${version}, but ${state?.version ?? "no service"} is running.`);
}

// Keep this record until a version answers its health check. A crashed updater
// can then finish the switch or restore the previous selection on next launch.
async function recover(env, io) {
  let record;
  try { record = JSON.parse(fs.readFileSync(journalFile(env), "utf8")); }
  catch (err) { if (err.code === "ENOENT") return null; throw err; }
  const valid = (value) => value && /^\d+\.\d+\.\d+$/.test(value.version) && /^[a-f0-9]{64}$/.test(value.sha256);
  if (!valid(record.next) || (record.previous !== null && !valid(record.previous))) throw new Error("Invalid service update recovery record.");
  const state = await running(env, io);
  if (state?.version === record.next.version) {
    selectComponent("service", record.next, env);
    clearJournal(env);
    return "completed";
  }
  if (state && state.version !== record.previous?.version) {
    if (state.terminals.some((terminal) => !terminal.exited)) throw new Error("Close running terminals before recovering the interrupted service update.");
    await stopService(env, io);
  }
  restoreSelection(record.previous, env);
  if (record.previous) await startVersion(record.previous.version, env, io);
  clearJournal(env);
  return "restored";
}

export async function recoverServiceUpdate({ env = process.env, operations } = {}) {
  // Avoid taking a lock on ordinary launches, including launches during updates.
  if (!fs.existsSync(journalFile(env))) return null;
  return withInstallLock(env, () => recover(env, lifecycle(operations)));
}

export async function installBundle(bundle, { env = process.env, command = true } = {}) {
  return withInstallLock(env, async () => {
    await recover(env, lifecycle());
    const manifest = JSON.parse(fs.readFileSync(path.join(bundle, "components.json"), "utf8"));
    for (const item of manifest.packages) {
      if (item.platform !== process.platform || item.arch !== process.arch) continue;
      const old = currentComponent(item.component, env);
      // Bundle updates never silently restart or upgrade the service/tools.
      if (old) continue;
      const staged = await stagePackage(path.join(bundle, item.file), item, { env });
      checkRuntime(staged);
      selectComponent(item.component, staged, env);
    }
    const launcher = installLauncher(env);
    if (command && launcher) installCommand({ ...launcher, env });
    return installedRuntime(env);
  });
}

export function installedRuntime(env = process.env) {
  const service = currentComponent("service", env);
  if (!service) throw new Error("XenoNet Service is not installed. Use the bundle installer.");
  const launcher = installLauncher(env);
  return { root: service.root, exec: path.join(service.root, nodeName()), env: { ...env, XENONET_RUNTIME_DIR: runtimeDir(env), ...(launcher ? { XENONET_LAUNCHER_EXEC: launcher.runtime, XENONET_LAUNCHER_ROOT: launcher.root } : {}) } };
}

export async function activatePackage(file, item, { env = process.env, operations } = {}) {
  return withInstallLock(env, async () => {
    const io = lifecycle(operations);
    await recover(env, io);
    const old = currentComponent(item.component, env);
    if (old && !newer(item.version, old.version)) return { component: item.component, version: old.version, status: "current" };
    const staged = await stagePackage(file, item, { env });
    io.check(staged);
    if (item.component === "tools") {
      selectComponent("tools", staged, env);
      installLauncher(env);
      return { component: "tools", version: staged.version, status: "updated", detail: "New commands use this version. Connected MCP processes keep running." };
    }
    const state = await running(env, io);
    if (state?.terminals.some((t) => !t.exited)) return { component: "service", version: staged.version, status: "staged", detail: "Close running terminals, then update the service." };
    writeJournal(env, old, staged);
    try {
      if (state) await stopService(env, io);
      selectComponent("service", staged, env);
      await startVersion(staged.version, env, io);
      clearJournal(env);
    }
    catch (err) {
      try {
        // Do not finalize the new version when its startup check just failed.
        const live = await running(env, io);
        if (live && live.version !== old?.version) await stopService(env, io);
        restoreSelection(old, env);
        if (old) await startVersion(old.version, env, io);
        clearJournal(env);
      } catch (rollback) {
        throw new Error(`The service update failed: ${err.message} Recovery is pending: ${rollback.message}`);
      }
      throw new Error(`The service update failed. ${old ? "The previous version was restored. " : ""}${err.message}`);
    }
    return { component: "service", version: staged.version, status: "updated" };
  });
}
