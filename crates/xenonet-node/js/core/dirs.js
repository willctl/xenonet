// core/dirs.js in Rust (crates/xenonet-base/src/dirs.rs): where XenoNet
// keeps its own files and where the desktop apps it reads keep theirs.
// What's on disk is looked at through the functions given here (Node's fs
// by default), as the JS did.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { native } from "../index.js";
import { onPath } from "./paths.js";

const home = os.homedir();

// (A field napi reads from an object is missing as undefined, never null.)
const envOf = (env) => ({ claudeDesktopDir: env.XENONET_CLAUDE_DESKTOP_DIR || undefined, localAppData: env.LOCALAPPDATA ?? undefined, appData: env.APPDATA ?? undefined, xdgConfigHome: env.XDG_CONFIG_HOME ?? undefined });
const looks = (exists, readdir) => [(p) => !!exists(p), (p) => [...readdir(p)].map(String)];

export function appDir(kind, { env = process.env, platform = process.platform, homeDir = home, name = "XenoNet" } = {}) {
  return native.appDir(String(kind), env.XDG_STATE_HOME ?? null, env.XDG_CACHE_HOME ?? null, env.LOCALAPPDATA ?? null, String(platform), String(homeDir), String(name));
}

export function findClaudeDesktopDir({ env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync } = {}) {
  return native.findClaudeDesktopDir(envOf(env), String(platform), String(homeDir), ...looks(exists, readdir)) ?? null;
}

export function bundledCodexBinaries({ env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs } = {}) {
  return native.bundledCodexBinaries(envOf(env), String(platform), String(homeDir), ...looks(exists, readdir), (f) => Number(mtime(f)));
}

export function bundledClaudeCode(desktopDir, { platform = process.platform, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs } = {}) {
  return native.bundledClaudeCode(String(desktopDir), String(platform), ...looks(exists, readdir), (f) => Number(mtime(f)));
}

const cliEnvOf = (env, platform, homeDir) => ({
  path: String(env.PATH ?? ""),
  pathext: env.PATHEXT ?? undefined,
  claudeBin: env.XENONET_CLAUDE_BIN ?? undefined,
  codexBin: env.XENONET_CODEX_BIN ?? undefined,
  dirs: envOf(env),
  platform: String(platform),
  home: String(homeDir),
});

export function findCli(app, { env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs, canRun = null } = {}) {
  return native.findCli(String(app), cliEnvOf(env, platform, homeDir), ...looks(exists, readdir), (f) => Number(mtime(f)), canRun) ?? null;
}

export function findCodexBinary({ env = process.env } = {}) {
  if (env.XENONET_CODEX_BIN) return env.XENONET_CODEX_BIN;
  return bundledCodexBinaries({ env })[0] ?? "codex";
}

export const findFfmpeg = (env = process.env) => onPath("ffmpeg", { env: { ...env, PATH: [env.PATH, "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].filter(Boolean).join(path.delimiter) } });
