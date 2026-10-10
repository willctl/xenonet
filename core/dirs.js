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
