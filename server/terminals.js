// Real shells for the terminal drawer. node-pty drives Windows ConPTY, so
// PowerShell gets a genuine console: PSReadLine, colors, Unicode, full-screen
// TUIs, Ctrl+C; on macOS and Linux it's a plain pty. Output is batched into
// ~4ms frames to keep IPC cheap.
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const require = createRequire(import.meta.url);
const pty = require("@lydell/node-pty");

function which(cmd) {
  try {
    return execFileSync("where.exe", [cmd], { encoding: "utf8", windowsHide: true, timeout: 3000 }).split(/\r?\n/).find(Boolean) ?? null;
  } catch {
    return null;
  }
}

// macOS and Linux: the login shell first (what a new Terminal window opens),
// then the other common shells that are installed. All start as login shells,
// so they read the same profile files Terminal's do.
const POSIX_SHELLS = {
  zsh: { label: "zsh", args: ["-l"] },
  bash: { label: "bash", args: ["-l"], env: { BASH_SILENCE_DEPRECATION_WARNING: "1" } },
  fish: { label: "fish", args: ["-l"] },
  pwsh: { label: "PowerShell 7", args: ["-NoLogo", "-Login"] },
  nu: { label: "Nushell", args: ["-l"] },
};
const POSIX_DIRS = ["/bin", "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

export function posixProfiles({ loginShell, exists = fs.existsSync } = {}) {
  const out = [];
  const add = (file) => {
    const name = path.basename(file);
    if (out.some((p) => p.id === name) || !exists(file)) return;
    const known = POSIX_SHELLS[name] ?? { label: name, args: ["-l"] };
    out.push({ id: name, label: known.label, file, args: known.args, env: known.env });
  };
  if (loginShell) add(loginShell);
  for (const name of Object.keys(POSIX_SHELLS)) for (const dir of POSIX_DIRS) add(path.join(dir, name));
  return out.length ? out : [{ id: "sh", label: "sh", file: "/bin/sh", args: ["-l"] }];
}

export function detectProfiles() {
  if (process.platform !== "win32") return posixProfiles({ loginShell: process.env.SHELL || os.userInfo().shell });
  const sys = process.env.SystemRoot ?? "C:\\Windows";
  const out = [];
  const pwsh = which("pwsh") ?? ["C:\\Program Files\\PowerShell\\7\\pwsh.exe"].find((p) => fs.existsSync(p));
  if (pwsh) out.push({ id: "pwsh", label: "PowerShell 7", file: pwsh, args: ["-NoLogo"] });
  const winps = path.join(sys, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  if (fs.existsSync(winps)) out.push({ id: "powershell", label: "Windows PowerShell", file: winps, args: ["-NoLogo"] });
  out.push({ id: "cmd", label: "Command Prompt", file: process.env.ComSpec ?? path.join(sys, "System32", "cmd.exe"), args: [] });
  const gitBash = ["C:\\Program Files\\Git\\bin\\bash.exe", "C:\\Program Files (x86)\\Git\\bin\\bash.exe"].find((p) => fs.existsSync(p));
  if (gitBash) out.push({ id: "gitbash", label: "Git Bash", file: gitBash, args: ["--login", "-i"] });
  const wsl = path.join(sys, "System32", "wsl.exe");
  if (fs.existsSync(wsl)) {
    try {
      // Docker Desktop's internal distros aren't shells you'd want to open.
      const distros = execFileSync(wsl, ["-l", "-q"], { encoding: "utf16le", windowsHide: true, timeout: 4000 })
        .replace(/\0/g, "")
        .split(/\r?\n/)
        .map((s) => s.trim())
        .filter((d) => d && !d.startsWith("docker-desktop"));
      if (distros.length) out.push({ id: "wsl", label: `WSL · ${distros[0]}`, file: wsl, args: [] });
    } catch {}
  }
  return out;
}

// What XenoNet sets for its own processes, which a shell it opens mustn't
// pass on: XenoNet Dev started in a terminal of the installed XenoNet would
// take its port and folders, and the reverse.
export const OWN_ENV = ["XENONET_PORT", "XENONET_STANDALONE", "XENONET_RUNTIME_DIR", "XENONET_LAUNCHER_EXEC", "XENONET_LAUNCHER_ROOT", "XENONET_CONTROL", "XENONET_EXEC", "XENONET_NAME", "XENONET_HOT", "XENONET_INSTANCE", "XENONET_STATE_DIR", "XENONET_CACHE_DIR", "XENONET_DEMO"];

export function shellEnv(extra, base = process.env) {
  const env = { ...base, ...extra, TERM_PROGRAM: "XenoNet", COLORTERM: "truecolor", TERM: "xterm-256color" };
  for (const k of Object.keys(env)) if (k.startsWith("ELECTRON_") || k === "NODE_OPTIONS" || (OWN_ENV.includes(k) && !(extra && k in extra))) delete env[k];
  return env;
}

function safeCwd(cwd) {
  try {
    if (cwd && fs.statSync(cwd).isDirectory()) return cwd;
  } catch {}
  return os.homedir();
}

// The service owns shells. A Desktop connection only reads their output.
export function createTerminals() {
  const profiles = detectProfiles();
  const terms = new Map();
  let nextId = 1;
  const info = (t) => ({ id: t.id, pid: t.proc.pid, cwd: t.cwd, profile: t.profile.id, label: t.profile.label, exited: t.exited, cols: t.cols, rows: t.rows });
  function create({ profile, cwd, cols = 120, rows = 30 } = {}) {
    if (terms.size >= 64) throw new Error("Close a terminal before opening another (64 maximum).");
    const prof = profiles.find((p) => p.id === profile) ?? profiles[0];
    const dir = safeCwd(cwd);
    const opts = { name: "xterm-256color", cols: Math.max(2, cols | 0), rows: Math.max(2, rows | 0), cwd: dir, env: shellEnv(prof.env) };
    let proc;
    if (process.platform !== "win32") proc = pty.spawn(prof.file, prof.args, opts);
    else try { proc = pty.spawn(prof.file, prof.args, { ...opts, useConpty: true, useConptyDll: true }); }
    catch { proc = pty.spawn(prof.file, prof.args, { ...opts, useConpty: true }); }
    const t = { id: nextId++, proc, cwd: dir, profile: prof, output: "", end: 0, exited: false, cols: opts.cols, rows: opts.rows };
    terms.set(t.id, t);
    proc.onData((data) => { t.output = (t.output + data).slice(-(1 << 20)); t.end += data.length; });
    proc.onExit(({ exitCode }) => { t.exited = true; t.exitCode = exitCode; });
    return info(t);
  }
  const need = (id) => { const t = terms.get(id); if (!t) throw new Error("That terminal has closed."); return t; };
  const kill = (id) => { const t = terms.get(id); if (!t) return; terms.delete(id); try { t.proc.kill(); } catch {} };
  function read({ id, cursor = 0 }) {
    const t = need(id);
    const start = t.end - t.output.length;
    return { ...info(t), data: t.output.slice(Math.max(0, cursor - start)), cursor: t.end, truncated: cursor < start, exitCode: t.exitCode };
  }
  return {
    profiles: () => profiles.map(({ id, label, file }) => ({ id, label, file })),
    list: () => [...terms.values()].map(info), create, read, kill,
    write: ({ id, data }) => { if (typeof data !== "string" || data.length > 1 << 20) throw new Error("Invalid terminal input."); need(id).proc.write(data); },
    resize: ({ id, cols, rows }) => { const t = need(id); t.cols = Math.max(2, Math.min(1000, cols | 0)); t.rows = Math.max(2, Math.min(1000, rows | 0)); if (!t.exited) t.proc.resize(t.cols, t.rows); },
    killAll: () => [...terms.keys()].forEach(kill),
  };
}
