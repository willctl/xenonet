// The xenonet command on your PATH: a small script, ~/.local/bin/xenonet, that
// runs the xenonet binary (installed XenoNet's launcher, which never moves; see
// components/runtime.js's xenonetBin). One an earlier XenoNet wrote ran a node
// and cli/xenonet.js; it's read as XenoNet's too, and installing again
// replaces it. XenoNet's script is told apart from another program's by its
// marker line, and only XenoNet's is replaced or taken out.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const MARKER = "Added by XenoNet";
const quote = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
const batchQuote = (s) => {
  if (/["\r\n\0]/.test(s)) throw new Error("a command path can't contain a quote or a line break");
  return `"${String(s).replace(/%/g, "%%")}"`;
};

// Windows uses the app's standard local data folder; Unix uses ~/.local/bin.
export const commandDir = ({ home = os.homedir(), platform = process.platform, env = process.env } = {}) => (platform === "win32" ? path.join(env.LOCALAPPDATA ?? path.join(home, "AppData", "Local"), "XenoNet", "bin") : path.join(home, ".local", "bin"));

// The script itself: it runs `bin`, the xenonet binary.
export function commandScript({ bin, platform = process.platform }) {
  if (platform === "win32") return `@echo off\n:: ${MARKER}: the xenonet command. xenonet uninstall takes it out.\nsetlocal DisableDelayedExpansion\n${batchQuote(bin)} %*\n`;
  return `#!/bin/sh\n# ${MARKER}: the xenonet command. xenonet uninstall takes it out.\nexec ${quote(bin)} "$@"\n`;
}

// What's there now: whether there's a xenonet, whether it's XenoNet's, what it
// runs (and whether those files still exist), and whether the folder is on
// PATH.
export function commandState({ dir = commandDir(), env = process.env, platform = process.platform } = {}) {
  if (!dir) return { file: null, installed: false, ours: false, runs: null, broken: false, onPath: false };
  const file = path.join(dir, platform === "win32" ? "xenonet.cmd" : "xenonet");
  let text = null;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch {}
  // The binary, or an earlier XenoNet's node and its entry.
  const runs = platform === "win32" ? text?.match(/^"([^"]+)"(?: "([^"]+)")? %\*$/m)?.slice(1).filter((s) => s != null).map((s) => s.replace(/%%/g, "%")) ?? null : text?.match(/exec '((?:[^']|'\\'')+)'(?: '((?:[^']|'\\'')+)')? "\$@"/)?.slice(1).filter((s) => s != null).map((s) => s.replace(/'\\''/g, "'")) ?? null;
  const onPath = (env.PATH ?? "").split(path.delimiter).some((d) => d && samePlace(d, dir, platform));
  return { file, installed: text != null, ours: !!text?.includes(MARKER), runs, broken: !!runs && runs.some((f) => !fs.existsSync(f)), onPath };
}

// Windows paths differ only in case and a trailing backslash.
export function samePlace(a, b, platform = process.platform) {
  if (platform !== "win32") return path.resolve(a) === path.resolve(b);
  const norm = (p) => path.win32.resolve(p).replace(/\\+$/, "").toLowerCase();
  return norm(a) === norm(b);
}

// The Path a terminal opened now gets on Windows: the machine's, then the
// user's, as the registry keeps them. A process's own PATH is what it was
// given when it started, so a XenoNet started before Setup added its folder
// (by Setup itself, or from a terminal already open) doesn't see it there.
// `run` is (file, args) => { code, stdout }; null if reg can't be read.
const PATH_KEYS = ["HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment", "HKCU\\Environment"];
export async function savedPath({ run, env = process.env }) {
  const reg = path.win32.join(envValue(env, "SystemRoot") ?? "C:\\Windows", "System32", "reg.exe");
  const dirs = [];
  let read = false;
  for (const key of PATH_KEYS) {
    const res = await run(reg, ["query", key, "/v", "Path"]);
    if (res.code !== 0) continue;
    read = true;
    dirs.push(...regPath(res.stdout, env));
  }
  return read ? dirs : null;
}

// reg query's line for Path, split into folders with %VARS% filled in.
export function regPath(stdout, env = process.env) {
  const value = String(stdout).match(/^\s+Path\s+REG_(?:EXPAND_)?SZ\s+(.*?)\s*$/im)?.[1];
  if (!value) return [];
  return value
    .split(";")
    .map((d) => d.replace(/%([^%]+)%/g, (whole, name) => envValue(env, name) ?? whole).trim())
    .filter(Boolean);
}

// Windows' environment names ignore case; a plain object's keys don't.
function envValue(env, name) {
  if (env[name] != null) return env[name];
  const key = Object.keys(env).find((k) => k.toLowerCase() === name.toLowerCase());
  return key ? env[key] : null;
}

export function installCommand({ dir = commandDir(), bin, force = false, env = process.env, platform = process.platform }) {
  const before = commandState({ dir, env, platform });
  if (before.installed && !before.ours && !force) throw new Error(`${before.file} is already there, and it isn't XenoNet's`);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = `${before.file}.tmp-xenonet`;
  fs.writeFileSync(tmp, commandScript({ bin, platform }), { mode: 0o755 });
  fs.renameSync(tmp, before.file);
  return commandState({ dir, env, platform });
}

export function uninstallCommand({ dir = commandDir(), env = process.env, platform = process.platform } = {}) {
  const st = commandState({ dir, env, platform });
  if (!st.installed) return st;
  if (!st.ours) throw new Error(`${st.file} isn't XenoNet's, so XenoNet leaves it alone`);
  fs.unlinkSync(st.file);
  return commandState({ dir, env, platform });
}
