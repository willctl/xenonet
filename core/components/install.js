import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { currentComponent, stagePackage, selectComponent, newer, withInstallLock, fitsHere, componentsDir, TRAY } from "./packages.js";
import { runtimeDir, ensureService, connectService, binName, launcherBin } from "./runtime.js";
import { installCommand } from "../command.js";

// The node an earlier XenoNet put in its packages and copied into the
// launcher's folder, which the entries it wrote run.
const earlierNode = () => (process.platform === "win32" ? "node.exe" : "node");

// The launcher never moves: hooks, MCP servers, the xenonet command and other
// programs run its path. It's the Tools' xenonet binary, copied to
// <runtime>/launcher/xenonet and copied again when the Tools change; a running
// MCP process keeps the file it started from. Entries an earlier XenoNet wrote
// run a node of the launcher's own and cli/xenonet.js (launcher.cjs says what
// that does), which XenoNet keeps current but never puts there itself.
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

// Why the launcher's binary is still the old one when it couldn't be
// replaced, and when it will be.
export const LAUNCHER_PENDING = "XenoNet replaces it the next time it starts, or the next time you run xenonet update";

// Puts `tmp` in `to`'s place. Windows won't replace a program that's
// running, and the launcher runs as long as an MCP server or the xenonet
// command does, but it lets the running file be renamed: that copy goes
// aside, as <to>.<pid>-<ms>.old, the new one takes its name, and sweepAside
// deletes the old one on a later start, once nothing runs it. If the new one
// can't take the name, the old one goes back.
export function replaceFile(tmp, to, { rename = fs.renameSync } = {}) {
  try {
    return rename(tmp, to);
  } catch (err) {
    if (!fs.existsSync(to)) throw err;
  }
  const aside = `${to}.${process.pid}-${Date.now()}.old`;
  rename(to, aside);
  try {
    rename(tmp, to);
  } catch (err) {
    try { rename(aside, to); } catch {}
    throw err;
  }
}

// Deletes the copies replaceFile put aside beside `to`. One still running (on
// Windows) stays until a later start.
export function sweepAside(to) {
  const dir = path.dirname(to), prefix = `${path.basename(to)}.`;
  let names = [];
  try { names = fs.readdirSync(dir); } catch {}
  for (const name of names) if (name.startsWith(prefix) && name.endsWith(".old")) try { fs.unlinkSync(path.join(dir, name)); } catch {}
}

// Copies the Tools' binary to the launcher's, when it isn't the copy of these
// Tools already (its .from file says which it is), through a temporary file
// and a rename, so a hook starting meanwhile runs the old or the new; a
// launcher that's running is put aside first (replaceFile). True when it was
// written, false when there was nothing to do; throws why it couldn't be.
export function writeLauncherBin(tools, to) {
  sweepAside(to);
  const from = path.join(tools.root, "bin", binName());
  if (!fs.existsSync(from)) return false;
  const source = `${tools.version}-${tools.sha256}`;
  let had = null;
  try {
    had = fs.readFileSync(`${to}.from`, "utf8");
  } catch {}
  if (had === source && fs.existsSync(to)) return false;
  const tmp = `${to}.${process.pid}.tmp`;
  try {
    fs.copyFileSync(from, tmp);
    if (process.platform !== "win32") fs.chmodSync(tmp, 0o755);
    replaceFile(tmp, to);
    fs.writeFileSync(`${to}.from`, source);
    return true;
  } catch (err) {
    fs.rmSync(tmp, { force: true });
    throw err;
  }
}

// `pending` says why the binary is still an earlier Tools' when it couldn't
// be replaced, and when it will be.
export function installLauncher(env = process.env) {
  const tools = currentComponent("tools", env);
  if (!tools) return null;
  const dir = path.join(runtimeDir(env), "launcher");
  const bin = launcherBin(env);
  fs.mkdirSync(dir, { recursive: true });
  let pending = null;
  try {
    writeLauncherBin(tools, bin);
  } catch (err) {
    pending = `the launcher couldn't be replaced (${err.message}); ${LAUNCHER_PENDING}`;
  }
  const entry = path.join(dir, "cli", "xenonet.js");
  if (fs.existsSync(path.join(dir, earlierNode())) && fs.existsSync(entry)) writeLauncher(entry);
  return { bin: fs.existsSync(bin) ? bin : null, root: dir, pending };
}

// Whether a selected Service or Tools has its xenonet binary. One from
// before 0.4.0 has none: its package was a node and XenoNet's JS, which
// this XenoNet replaces (installBundle, or an update) rather than runs,
// except to bring back a service that couldn't be replaced
// (installedRuntime).
export function hasBinary(selected) {
  return !!fs.statSync(path.join(selected.root, "bin", binName()), { throwIfNoEntry: false })?.isFile();
}

// The startup check of a staged component: its xenonet binary runs, without
// ELECTRON_RUN_AS_NODE, and says the version it was promised.
export function checkRuntime(staged) {
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  const result = spawnSync(path.join(staged.root, "bin", binName()), ["--version", "--json"], { cwd: staged.root, windowsHide: true, encoding: "utf8", timeout: 15000, env });
  let version;
  try { version = JSON.parse(result.stdout).version; } catch {}
  if (result.status !== 0 || version == null) throw new Error("The new component runtime did not pass its startup check.");
  if (version !== staged.version) throw new Error("The new component command did not pass its startup check.");
}

// The folder the selected Service runs from, <runtime>/components/service/
// active: a copy of the selected version's own folder. A firewall lets a
// program in by its path, so the service keeps one path across updates
// (docs/guide/components.md). Its .from file says which version it's a
// copy of, as the launcher's does.
const ACTIVE = "active";
const serviceDir = (env) => path.join(componentsDir(env), "service");
const activeMark = (selected) => `${selected.version}-${selected.sha256}`;

// Where the selected Service runs from: `active` when it's a copy of that
// version, with its binary. Otherwise its version's own folder, and why,
// unless there's no `active` at all, as before this version's first
// activation.
function serviceRoot(service, env) {
  const active = path.join(serviceDir(env), ACTIVE);
  if (!fs.existsSync(active)) return { root: service.root, why: null };
  let said = "";
  try { said = fs.readFileSync(path.join(active, ".from"), "utf8"); } catch {}
  if (!said) return { root: service.root, why: `${ACTIVE} doesn't say which version it's a copy of` };
  if (said !== activeMark(service)) return { root: service.root, why: `${ACTIVE} is a copy of ${said}, not ${activeMark(service)}` };
  if (!hasBinary({ root: active })) return { root: service.root, why: `${ACTIVE} has no ${binName()}` };
  return { root: active, why: null };
}

// Copies a folder: files and folders only, each file with its mode, as a
// clone where the disk can (APFS). A package of XenoNet's has no links.
function copyTree(from, to) {
  fs.mkdirSync(to);
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const [src, dst] = [path.join(from, entry.name), path.join(to, entry.name)];
    if (entry.isDirectory()) copyTree(src, dst);
    else if (entry.isFile()) fs.copyFileSync(src, dst, fs.constants.COPYFILE_FICLONE);
    else throw new Error(`${src} isn't a file or a folder`);
  }
}

// Renames a folder; on Windows, which can hold a program a moment after it
// exits, tried again for up to 30 s, as stagePackage's last rename is.
async function renameDir(from, to) {
  for (const until = Date.now() + 30000;;) {
    try { return fs.renameSync(from, to); }
    catch (err) {
      if (process.platform !== "win32" || !["EPERM", "EACCES", "EBUSY"].includes(err.code) || Date.now() >= until) throw err;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
}

// Makes `active` a copy of the selected Service, with the service stopped:
// what an activation cut short left (active.next, active.prev) goes, the
// version's folder is copied to active.next and marked, then `active` moves
// aside and active.next takes its place. An `active` of that version
// already stays as it is. A Service from before 0.4.0 runs from its own
// folder, so there's no `active`, nor with none selected.
async function activateService(env) {
  const dir = serviceDir(env);
  const [active, next, prev] = [ACTIVE, "active.next", "active.prev"].map((name) => path.join(dir, name));
  for (const left of [next, prev]) fs.rmSync(left, { recursive: true, force: true });
  const selected = currentComponent("service", env);
  if (!selected || !hasBinary(selected)) return fs.rmSync(active, { recursive: true, force: true });
  if (serviceRoot(selected, env).root === active) return;
  copyTree(selected.root, next);
  fs.writeFileSync(path.join(next, ".from"), activeMark(selected));
  if (fs.existsSync(active)) await renameDir(active, prev);
  fs.renameSync(next, active);
  // One that can't go now goes at the next activation.
  try { fs.rmSync(prev, { recursive: true, force: true }); } catch {}
}

// activateService, whose failure leaves the service to run from its
// version's folder (serviceRoot): it still starts, though a firewall may ask
// about it again. So it's said, not thrown.
async function activate(env) {
  try { await activateService(env); }
  catch (err) { console.error(`[xenonet] XenoNet Service runs from its version's folder this time: ${err.message}`); }
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
// Selects the previous service again, and runs it from `active`, with the
// service stopped.
async function restoreSelection(previous, env) {
  if (previous) selectComponent("service", previous, env);
  else fs.rmSync(path.join(runtimeDir(env), "components/service/current.json"), { force: true });
  await activate(env);
}
async function running(env, io) {
  const client = await io.connect({ env }).catch((err) => {
    if (/incompatible/.test(err.message)) throw err;
    return null;
  });
  if (!client) return null;
  try { return await client.request("status"); } finally { client.close(); }
}
// The service answers `stop` and shuts down a moment later, so on a busy
// machine its answer can be lost with the connection. A stop sent and then
// cut off may have worked: whether the service has gone decides, as it does
// for one that answered. An answer that says no is a failure.
const dropped = (err) => /^XenoNet disconnected/.test(err?.message ?? "");
async function stopService(env, io) {
  const client = await io.connect({ env }).catch(() => null);
  if (!client) return;
  try { await client.request("stop"); }
  catch (err) { if (!dropped(err)) throw err; }
  finally { client.close(); }
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
  await restoreSelection(record.previous, env);
  if (record.previous) await startVersion(record.previous.version, env, io);
  clearJournal(env);
  return "restored";
}

export async function recoverServiceUpdate({ env = process.env, operations } = {}) {
  // Avoid taking a lock on ordinary launches, including launches during updates.
  if (!fs.existsSync(journalFile(env))) return null;
  return withInstallLock(env, () => recover(env, lifecycle(operations)));
}

// Installs a bundle's components that aren't installed yet, and replaces
// any from before 0.4.0 (see hasBinary), with the launcher and, when asked,
// the xenonet command; says how to start the service. A bundle never
// silently restarts or upgrades a Service or Tools that has its binary. On
// a Mac, a newer tray than the one installed replaces it.
export async function installBundle(bundle, { env = process.env, command = true, operations } = {}) {
  return withInstallLock(env, async () => {
    const io = lifecycle(operations);
    await recover(env, io);
    const manifest = JSON.parse(fs.readFileSync(path.join(bundle, "components.json"), "utf8"));
    let earlier = null;
    for (const item of manifest.packages) {
      if (!fitsHere(item)) continue;
      const old = currentComponent(item.component, env);
      // The tray, on a Mac: installed when it's missing or this one is
      // newer. Nothing runs it from here; xenonet service install puts it
      // in Applications and starts it at login.
      if (item.component === TRAY) {
        if (!old || newer(item.version, old.version)) selectComponent(TRAY, await stagePackage(path.join(bundle, item.file), item, { env }), env);
        continue;
      }
      if (old && hasBinary(old)) continue;
      const staged = await stagePackage(path.join(bundle, item.file), item, { env });
      checkRuntime(staged);
      // A service from before 0.4.0 may be running: it's switched as an
      // update switches it, once the Tools and the launcher it names are
      // in place.
      if (old && item.component === "service") earlier = { old, staged };
      else {
        selectComponent(item.component, staged, env);
        // A first Service: nothing of it runs yet.
        if (item.component === "service") await activate(env);
      }
    }
    const launcher = installLauncher(env);
    if (command && launcher?.bin) installCommand({ bin: launcher.bin, env });
    if (earlier) await switchService(env, io, earlier.old, earlier.staged);
    return installedRuntime(env);
  });
}

// How to start the selected service: its xenonet binary, with `serve`, and
// the env it runs with (the runtime folder, and the launcher's binary when
// there is one). It runs from `active` (serviceRoot), or its version's
// folder when that isn't a copy of it. A Service from before 0.4.0 runs its
// node and server, as that XenoNet ran it, told of the node launcher its
// entries run, so an update that couldn't replace it can bring it back as
// it was.
export function installedRuntime(env = process.env) {
  const service = currentComponent("service", env);
  if (!service) throw new Error("XenoNet Service is not installed. Use the bundle installer.");
  const launcher = installLauncher(env);
  const runEnv = { ...env, XENONET_RUNTIME_DIR: runtimeDir(env), ...(launcher?.bin ? { XENONET_LAUNCHER_BIN: launcher.bin } : {}) };
  if (hasBinary(service)) {
    const { root, why } = serviceRoot(service, env);
    if (why) console.error(`[xenonet] XenoNet Service runs from ${root}: ${why}`);
    return { root, exec: path.join(root, "bin", binName()), args: ["serve"], env: runEnv };
  }
  const dir = path.join(runtimeDir(env), "launcher");
  const node = path.join(dir, earlierNode());
  const nodeLauncher = fs.existsSync(node) && fs.existsSync(path.join(dir, "cli", "xenonet.js")) ? { XENONET_LAUNCHER_EXEC: node, XENONET_LAUNCHER_ROOT: dir } : {};
  return { root: service.root, exec: path.join(service.root, earlierNode()), args: [path.join(service.root, "server", "index.js")], env: { ...runEnv, ...nodeLauncher } };
}

// Switches the selected service from `old` (or none) to `staged`, as an
// update does: never while a terminal is open in the running service, which
// leaves the new version staged; with a record of the switch until the new
// version answers; and back to the old one when it doesn't.
async function switchService(env, io, old, staged) {
  const state = await running(env, io);
  if (state?.terminals.some((t) => !t.exited)) return { component: "service", version: staged.version, status: "staged", detail: "Close running terminals, then update the service." };
  writeJournal(env, old, staged);
  try {
    if (state) await stopService(env, io);
    selectComponent("service", staged, env);
    await activate(env);
    await startVersion(staged.version, env, io);
    clearJournal(env);
  }
  catch (err) {
    try {
      // Do not finalize the new version when its startup check just failed.
      const live = await running(env, io);
      if (live && live.version !== old?.version) await stopService(env, io);
      await restoreSelection(old, env);
      if (old) await startVersion(old.version, env, io);
      clearJournal(env);
    } catch (rollback) {
      throw new Error(`The service update failed: ${err.message} Recovery is pending: ${rollback.message}`);
    }
    throw new Error(`The service update failed. ${old ? "The previous version was restored. " : ""}${err.message}`);
  }
  return { component: "service", version: staged.version, status: "updated" };
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
      const pending = installLauncher(env)?.pending;
      const detail = pending ? `New commands use this version once the launcher is replaced: ${pending}. Connected MCP processes keep running.` : "New commands use this version. Connected MCP processes keep running.";
      return { component: "tools", version: staged.version, status: "updated", detail };
    }
    return switchService(env, io, old, staged);
  });
}
