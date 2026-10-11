import fs from "node:fs";
import fsp from "node:fs/promises";
import path from "node:path";

// Paths read from another machine keep that machine's syntax. Inferring it
// from an absolute path also keeps fixture data independent of the test OS.
export function pathStyle(p) {
  const s = String(p ?? "");
  if (/^[A-Za-z]:|^\\\\/.test(s) || s.includes("\\")) return path.win32;
  return s.startsWith("/") ? path.posix : path;
}

// Path keys are compared case-insensitively with forward slashes, because the
// apps report the same Windows folder as `C:\x`, `c:/x` or `\\?\C:\x`. On
// macOS, /tmp, /var and /etc are links into /private, and tools report either
// form (os.tmpdir() says /var/folders/…, realpath says /private/var/folders/…).
export function pathKey(p) {
  if (!p) return null;
  let s = String(p).replace(/^\\\\\?\\/, "").replace(/\\/g, "/");
  s = s.replace(/^\/private\/(var|tmp|etc)(?=\/|$)/, "/$1");
  s = s.replace(/\/+$/, "");
  return s.toLowerCase();
}

export function isUnder(childKey, parentKey) {
  if (!childKey || !parentKey) return false;
  return childKey === parentKey || childKey.startsWith(parentKey + "/");
}

// Writes paths under the home folder as "~", so activity details, summaries
// and screenshots don't carry the user name.
export function tilde(text, home) {
  if (!text || !home) return text;
  const h = String(home).replace(/[\\/]+$/, "");
  let out = String(text);
  for (const form of new Set([h, h.replace(/\\/g, "/"), h.replace(/\//g, "\\")])) {
    const re = new RegExp(form.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "(?![\\w.-])", "gi");
    out = out.replace(re, "~");
  }
  return out;
}

export function baseName(p) {
  const parts = String(p ?? "").replace(/\\/g, "/").replace(/\/+$/, "").split("/");
  return parts[parts.length - 1] || String(p ?? "");
}

// Both desktop apps create a throwaway folder per session when you start one
// without picking a project, and Grok Build makes a worktree per session it
// isolates. Group those into one zone each instead of scattering a zone per
// folder.
const SCRATCH_ZONES = [
  { id: "zone:claude-scratch", name: "Claude scratch", app: "claude", test: (k) => k.includes("/claude/scratch-workspaces/") },
  { id: "zone:codex-projectless", name: "Codex without a project", app: "codex", test: (k) => /\/documents\/codex\/\d{4}-\d{2}-\d{2}(\/|$)/.test(k) },
  // Grok Build's worktrees, when XenoNet can't tell which repo one came from.
  { id: "zone:grok-worktrees", name: "Grok worktrees", app: "grok", test: (k) => /\/\.grok\/(worktrees|worktree_pool)\//.test(k) },
  // %TEMP% on Windows, $TMPDIR (/var/folders/xx/…/T) on macOS, /tmp anywhere.
  { id: "zone:temp", name: "Temp", app: null, test: (k) => `${k}/`.includes("/appdata/local/temp/") || `${k}/`.startsWith("/tmp/") || /^\/var\/folders\/[^/]+\/[^/]+\/t(\/|$)/.test(k) },
];

export function scratchZone(key) {
  return key ? SCRATCH_ZONES.find((z) => z.test(key)) ?? null : null;
}

// The first `cmd` on PATH, or null.
export function onPath(cmd, { env = process.env, platform = process.platform, canRun = (f) => fs.accessSync(f, fs.constants.X_OK) } = {}) {
  const exts = platform === "win32" ? (env.PATHEXT ?? ".EXE;.CMD;.BAT").split(";") : [""];
  for (const dir of String(env.PATH ?? "").split(path.delimiter).filter(Boolean)) {
    for (const ext of exts) {
      const file = path.join(dir, cmd + ext);
      try {
        canRun(file);
        return file;
      } catch {}
    }
  }
  return null;
}

// The files under `dir` whose names `match` picks, in folders down to
// `depth` levels below it. A folder that can't be read is passed over.
export async function listFiles(dir, match, depth = 6) {
  const out = [];
  const walk = async (d, level) => {
    for (const e of await fsp.readdir(d, { withFileTypes: true }).catch(() => [])) {
      const full = path.join(d, e.name);
      if (e.isDirectory()) {
        if (level < depth) await walk(full, level + 1);
      } else if (e.isFile() && match(e.name)) out.push(full);
    }
  };
  await walk(dir, 0);
  return out;
}
