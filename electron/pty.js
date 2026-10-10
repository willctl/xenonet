// Real shells for the terminal drawer. node-pty drives Windows ConPTY, so
// PowerShell gets a genuine console: PSReadLine, colors, Unicode, full-screen
// TUIs, Ctrl+C; on macOS and Linux it's a plain pty. Output is batched into
// ~4ms frames to keep IPC cheap.
import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { ipcMain, shell } from "electron";

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

function shellEnv(extra) {
  const env = { ...process.env, ...extra, TERM_PROGRAM: "XenoNet", COLORTERM: "truecolor", TERM: "xterm-256color" };
  for (const k of Object.keys(env)) if (k.startsWith("ELECTRON_") || k === "NODE_OPTIONS") delete env[k];
  return env;
}

function safeCwd(cwd) {
  try {
    if (cwd && fs.statSync(cwd).isDirectory()) return cwd;
  } catch {}
  return os.homedir();
}

// `allowed(event)`: whether a call comes from XenoNet's own page (main.js's
// ownPage). A shell is the most a page could ask for, so every call is
// checked, and one from anywhere else gets nothing.
export function registerPty({ allowed }) {
  ipcMain.handle("term:list", () => []);
  ipcMain.handle("term:watch", () => {});
  const profiles = detectProfiles();
  const terms = new Map(); // id → { proc, owner }
  let nextId = 1;

  ipcMain.handle("term:profiles", (event) => (allowed(event) ? profiles.map(({ id, label, file }) => ({ id, label, file })) : []));

  ipcMain.handle("term:create", (event, { profile, cwd, cols = 120, rows = 30 } = {}) => {
    if (!allowed(event)) return null;
    const prof = profiles.find((p) => p.id === profile) ?? profiles[0];
    const dir = safeCwd(cwd);
    const opts = { name: "xterm-256color", cols: Math.max(2, cols | 0), rows: Math.max(2, rows | 0), cwd: dir, env: shellEnv(prof.env) };
    let proc;
    if (process.platform !== "win32") proc = pty.spawn(prof.file, prof.args, opts);
    else
      try {
        // The bundled ConPTY (OpenConsole) passes VT through with fewer quirks.
        proc = pty.spawn(prof.file, prof.args, { ...opts, useConpty: true, useConptyDll: true });
      } catch {
        proc = pty.spawn(prof.file, prof.args, { ...opts, useConpty: true });
      }
    const id = nextId++;
    const sender = event.sender;
    let buf = "";
    let timer = null;
    const flush = () => {
      timer = null;
      if (buf && !sender.isDestroyed()) sender.send("term:data", id, buf);
      buf = "";
    };
    proc.onData((d) => {
      buf += d;
      if (buf.length > 256 * 1024) flush();
      else timer ??= setTimeout(flush, 4);
    });
    proc.onExit(({ exitCode }) => {
      flush();
      terms.delete(id);
      if (!sender.isDestroyed()) sender.send("term:exit", id, exitCode);
    });
    terms.set(id, { proc, owner: sender.id });
    sender.once("destroyed", () => {
      for (const [tid, t] of terms) if (t.owner === sender.id) killTerm(tid);
    });
    return { id, pid: proc.pid, cwd: dir, profile: prof.id, label: prof.label };
  });

  const mine = (event, id) => (allowed(event) ? terms.get(id) : null);
  ipcMain.on("term:write", (e, id, data) => mine(e, id)?.proc.write(data));
  ipcMain.on("term:resize", (e, id, cols, rows) => {
    try {
      mine(e, id)?.proc.resize(Math.max(2, cols | 0), Math.max(2, rows | 0));
    } catch {}
  });
  const killTerm = (id) => {
    const t = terms.get(id);
    if (!t) return;
    terms.delete(id);
    try {
      t.proc.kill();
    } catch {}
  };
  ipcMain.handle("term:kill", (e, id) => {
    if (mine(e, id)) killTerm(id);
  });
  ipcMain.handle("app:open-external", (e, url) => {
    if (allowed(e) && /^https?:\/\//i.test(url)) shell.openExternal(url);
  });

  return { profiles, killAll: () => [...terms.keys()].forEach(killTerm), count: () => terms.size };
}
