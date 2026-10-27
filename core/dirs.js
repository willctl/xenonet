// Where XenoNet keeps its own files and where the desktop apps it reads keep
// theirs. The server and the desktop shell (for uninstalling) both use these,
// so this module has no side effects.
import os from "node:os";
import path from "node:path";
import { onPath } from "./paths.js";
import fs from "node:fs";

const home = os.homedir();

// XenoNet's own files follow the XDG base directories when they're set,
// otherwise the platform's standard places; it never adds a dot dir to ~.
//   state (layout, dismissed alerts)  cache (search index, rebuildable)
//   Windows: %LOCALAPPDATA%\XenoNet  %LOCALAPPDATA%\XenoNet\Cache
//   macOS:   ~/Library/Application Support/XenoNet  ~/Library/Caches/XenoNet
//   Linux:   ~/.local/state/xenonet  ~/.cache/xenonet
// A copy running from source passes name "XenoNet Dev" for folders of its own.
export function appDir(kind, { env = process.env, platform = process.platform, homeDir = home, name = "XenoNet" } = {}) {
  const slug = name.toLowerCase().replace(/\s+/g, "-");
  const xdg = env[kind === "state" ? "XDG_STATE_HOME" : "XDG_CACHE_HOME"];
  if (xdg) return path.join(xdg, slug);
  if (platform === "win32") return path.join(env.LOCALAPPDATA ?? path.join(homeDir, "AppData", "Local"), name, ...(kind === "cache" ? ["Cache"] : []));
  if (platform === "darwin") return path.join(homeDir, "Library", kind === "state" ? "Application Support" : "Caches", name);
  return path.join(homeDir, kind === "state" ? ".local/state" : ".cache", slug);
}

// Claude desktop's data folder.
//   Windows: the MSIX package redirects %APPDATA%\Claude into
//            %LOCALAPPDATA%\Packages\Claude_<publisher>\LocalCache\Roaming\Claude.
//   macOS:   ~/Library/Application Support/Claude
//   Linux:   $XDG_CONFIG_HOME/Claude, else ~/.config/Claude
export function findClaudeDesktopDir({ env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync } = {}) {
  if (env.XENONET_CLAUDE_DESKTOP_DIR) return env.XENONET_CLAUDE_DESKTOP_DIR;
  const candidates = [];
  if (platform === "win32") {
    const packages = path.join(env.LOCALAPPDATA ?? path.join(homeDir, "AppData", "Local"), "Packages");
    try {
      for (const name of readdir(packages)) if (name.startsWith("Claude_")) candidates.push(path.join(packages, name, "LocalCache", "Roaming", "Claude"));
    } catch {}
    candidates.push(path.join(env.APPDATA ?? path.join(homeDir, "AppData", "Roaming"), "Claude"));
  } else if (platform === "darwin") {
    candidates.push(path.join(homeDir, "Library", "Application Support", "Claude"));
  } else {
    candidates.push(path.join(env.XDG_CONFIG_HOME ?? path.join(homeDir, ".config"), "Claude"));
  }
  return candidates.find((dir) => exists(dir)) ?? null;
}

// The codex binary that ships inside the ChatGPT desktop app, so the protocol
// matches whatever is writing ~/.codex.
//   Windows: %LOCALAPPDATA%\OpenAI\Codex\bin\<hash>\codex.exe, newest first
//            (the folder name is a hash that changes on update)
//   macOS:   ChatGPT.app/Contents/Resources/codex, in /Applications or ~/Applications
export function bundledCodexBinaries({ env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs } = {}) {
  if (platform === "win32") {
    const binDir = path.join(env.LOCALAPPDATA ?? path.join(homeDir, "AppData", "Local"), "OpenAI", "Codex", "bin");
    try {
      return readdir(binDir)
        .map((d) => path.join(binDir, d, "codex.exe"))
        .filter((f) => exists(f))
        .sort((a, b) => mtime(b) - mtime(a));
    } catch {
      return [];
    }
  }
  if (platform === "darwin") {
    const out = [];
    for (const apps of ["/Applications", path.join(homeDir, "Applications")])
      for (const bundle of ["ChatGPT.app", "Codex.app"]) out.push(path.join(apps, bundle, "Contents", "Resources", "codex"));
    return out.filter((f) => exists(f));
  }
  return [];
}

// Two version folders' names in order: part by part between the dots,
// numbers as numbers, anything else as text.
function versionOrder(a, b) {
  const [x, y] = [a.split("."), b.split(".")];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if (i >= x.length) return -1;
    if (i >= y.length) return 1;
    const digits = /^\d+$/.test(x[i]) && /^\d+$/.test(y[i]);
    const o = digits ? Math.sign(Number(x[i]) - Number(y[i])) : x[i] < y[i] ? -1 : x[i] > y[i] ? 1 : 0;
    if (o) return o;
  }
  return 0;
}

// The Claude Code that Claude desktop keeps a copy of, given Claude
// desktop's folder (findClaudeDesktopDir): the newest version first, and
// within one, the newest build.
//   macOS:   <Claude>/claude-code/<version>/<build>/claude.app/Contents/MacOS/claude
//   Windows: <Claude>\claude-code\<version>\<build>\claude.exe, or in a
//            folder inside the build
export function bundledClaudeCode(desktopDir, { platform = process.platform, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs } = {}) {
  const list = (d) => {
    try {
      return [...readdir(d)].map(String);
    } catch {
      return null;
    }
  };
  const root = path.join(desktopDir, "claude-code");
  const out = [];
  for (const v of (list(root) ?? []).sort((a, b) => versionOrder(b, a))) {
    const dir = path.join(root, v);
    const builds = list(dir);
    if (!builds) continue;
    const found = [];
    for (const b of builds) {
      const build = path.join(dir, b);
      const candidates =
        platform === "darwin" ? [path.join(build, "claude.app", "Contents", "MacOS", "claude")] : platform === "win32" ? [path.join(build, "claude.exe"), ...(list(build) ?? []).map((n) => path.join(build, n, "claude.exe"))] : [];
      for (const f of candidates) if (exists(f)) found.push([f, mtime(f)]);
    }
    found.sort((a, b) => b[1] - a[1]);
    out.push(...found.map(([f]) => f));
  }
  return out;
}

// The program to run for an app's CLI, or null when it isn't here.
//   claude: on PATH (with PATHEXT on Windows, so an npm install's
//           claude.cmd), then the Claude Code that Claude desktop keeps
//           (bundledClaudeCode), then XENONET_CLAUDE_BIN
//   codex:  XENONET_CODEX_BIN, then the one the ChatGPT desktop app ships
//           (bundledCodexBinaries), then on PATH
// `canRun` throws for a file on PATH that may not be run.
export function findCli(app, { env = process.env, platform = process.platform, homeDir = home, exists = fs.existsSync, readdir = fs.readdirSync, mtime = (f) => fs.statSync(f).mtimeMs, canRun } = {}) {
  const look = { env, platform, homeDir, exists, readdir, mtime };
  const desktop = () => {
    const dir = findClaudeDesktopDir(look);
    return dir ? bundledClaudeCode(dir, look)[0] : undefined;
  };
  if (app === "claude") return onPath("claude", { env, platform, canRun }) ?? desktop() ?? (env.XENONET_CLAUDE_BIN || null);
  if (app === "codex") return env.XENONET_CODEX_BIN || bundledCodexBinaries(look)[0] || onPath("codex", { env, platform, canRun });
  return null;
}

// The codex to run: the one bundled with the desktop app when there is one,
// so the protocol matches whatever is writing ~/.codex, else codex on PATH.
// XENONET_CODEX_BIN overrides both.
export function findCodexBinary({ env = process.env } = {}) {
  if (env.XENONET_CODEX_BIN) return env.XENONET_CODEX_BIN;
  return bundledCodexBinaries({ env })[0] ?? "codex";
}

// ffmpeg, which films of the day need (electron/recorder.js): on PATH, or
// where Homebrew puts it.
export const findFfmpeg = (env = process.env) => onPath("ffmpeg", { env: { ...env, PATH: [env.PATH, "/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].filter(Boolean).join(path.delimiter) } });
