// core/command.js in Rust (crates/xenonet-app/src/command.rs): the xenonet
// command on your PATH, a small script that runs cli/xenonet.js. Only
// XenoNet's own script, told apart by its marker line, is replaced or
// taken out.
import os from "node:os";
import { native } from "../index.js";
import { settle } from "../settle.js";

const text = (v) => (v == null ? null : String(v));

export const commandDir = ({ home = os.homedir(), platform = process.platform, env = process.env } = {}) => native.appCommandDir(String(home), String(platform), text(env.LOCALAPPDATA));

export function currentRuntime({ env = process.env, execPath = process.execPath, electron = process.versions.electron } = {}) {
  if (env.XENONET_EXEC) return { runtime: env.XENONET_EXEC, runAsNode: true };
  return native.appCurrentRuntime(null, String(execPath), !!electron, text(env.PATH));
}

export function commandScript({ runtime, entry, runAsNode = false, platform = process.platform }) {
  return settle(native.appCommandScript(String(runtime), String(entry), !!runAsNode, String(platform)));
}

export function commandState({ dir = commandDir(), env = process.env, platform = process.platform } = {}) {
  return native.appCommandState(dir ? String(dir) : null, text(env.PATH), String(platform));
}

export const samePlace = (a, b, platform = process.platform) => native.appSamePlace(String(a), String(b), String(platform));

// `run` is (file, args) => { code, stdout }; null if reg can't be read.
export async function savedPath({ run, env = process.env }) {
  const { reg, queries } = native.appSavedPathQueries({ ...env });
  const dirs = [];
  let read = false;
  for (const args of queries) {
    const res = await run(reg, args);
    if (res.code !== 0) continue;
    read = true;
    dirs.push(...regPath(res.stdout, env));
  }
  return read ? dirs : null;
}

export const regPath = (stdout, env = process.env) => native.appRegPath(String(stdout), { ...env });

export function installCommand({ dir = commandDir(), runtime, entry, runAsNode = false, force = false, env = process.env, platform = process.platform }) {
  return settle(native.appInstallCommand(String(dir), String(runtime), String(entry), !!runAsNode, !!force, text(env.PATH), String(platform)));
}

export function uninstallCommand({ dir = commandDir(), env = process.env, platform = process.platform } = {}) {
  return settle(native.appUninstallCommand(String(dir), text(env.PATH), String(platform)));
}
