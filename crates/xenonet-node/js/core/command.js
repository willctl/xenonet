// core/command.js in Rust (crates/xenonet-app/src/command.rs): the xenonet
// command on your PATH, a small script that runs the xenonet binary. Only
// XenoNet's own script, told apart by its marker line, is replaced or
// taken out.
import os from "node:os";
import { native } from "../index.js";
import { settle } from "../settle.js";

const text = (v) => (v == null ? null : String(v));

export const commandDir = ({ home = os.homedir(), platform = process.platform, env = process.env } = {}) => native.appCommandDir(String(home), String(platform), text(env.LOCALAPPDATA));

export function commandScript({ bin, platform = process.platform }) {
  return settle(native.appCommandScript(String(bin), String(platform)));
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

export function installCommand({ dir = commandDir(), bin, force = false, env = process.env, platform = process.platform }) {
  return settle(native.appInstallCommand(String(dir), String(bin), !!force, text(env.PATH), String(platform)));
}

export function uninstallCommand({ dir = commandDir(), env = process.env, platform = process.platform } = {}) {
  return settle(native.appUninstallCommand(String(dir), text(env.PATH), String(platform)));
}
