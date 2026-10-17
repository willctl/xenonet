// Xenolith (xenolith/ in this repo), XenoNet's shared context: where its
// wiki is, and which MCP server writes to it, so the map can show the wiki
// as its own constellation and beam a session's writes to the pages they
// name. XenoNet only reads the folder; Xenolith's own server does the
// writing. Agent Wiki, which Xenolith came from, is found the same way when
// Xenolith isn't there.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// The MCP servers whose calls write to the shared context.
export const CONTEXT_SERVERS = new Set(["xenolith", "agent-wiki"]);
export const isContextServer = (server) => CONTEXT_SERVERS.has(server);

const abs = (v) => (v && path.isAbsolute(v) ? v : null);

// A product's config folder, as its programs find it
// (xenolith/rust/crates/core/src/paths.rs): XENOLITH_HOME (or
// XENOLITH_CONFIG_DIR) first, then the platform's own place.
function configDir(names, { env, home, platform }) {
  const portable = env[`${names.env}_HOME`];
  if (portable) return path.resolve(portable);
  if (env[`${names.env}_CONFIG_DIR`]) return path.resolve(env[`${names.env}_CONFIG_DIR`]);
  if (platform === "win32") return path.join(env.APPDATA || path.join(home, "AppData", "Roaming"), names.windows);
  const xdg = abs(env.XDG_CONFIG_HOME);
  if (platform === "darwin") return xdg ? path.join(xdg, names.xdg) : path.join(home, "Library", "Application Support", names.mac);
  return path.join(xdg ?? path.join(home, ".config"), names.xdg);
}

const PRODUCTS = [
  { server: "xenolith", env: "XENOLITH", windows: "Xenolith", mac: "Xenolith", xdg: "xenolith", folder: "Xenolith", legacy: null },
  { server: "agent-wiki", env: "AGENT_WIKI", windows: "AgentWiki", mac: "Agent Wiki", xdg: "agent-wiki", folder: "AgentWiki", legacy: ".agent-wiki" },
];

const wikiDirIn = (file) => {
  try {
    const dir = JSON.parse(fs.readFileSync(file, "utf8").replace(/^﻿/, "")).wikiDir;
    return typeof dir === "string" && dir.trim() && fs.existsSync(dir) ? dir : null;
  } catch {
    return null;
  }
};

// The shared context's folder and the server that owns it: Xenolith's
// (XENOLITH_DIR, its config.json's wikiDir, ~/Xenolith), else Agent Wiki's
// the same way (and its pre-1.3 ~/.agent-wiki/config.json). Null when
// neither is there.
export function findContextDir({ env = process.env, home = os.homedir(), platform = process.platform } = {}) {
  for (const p of PRODUCTS) {
    const fromEnv = env[`${p.env}_DIR`];
    if (fromEnv && fs.existsSync(fromEnv)) return { dir: path.resolve(fromEnv), server: p.server };
    const fromConfig = wikiDirIn(path.join(configDir(p, { env, home, platform }), "config.json")) ?? (p.legacy ? wikiDirIn(path.join(home, p.legacy, "config.json")) : null);
    if (fromConfig) return { dir: fromConfig, server: p.server };
    const fallback = path.join(home, p.folder);
    if (fs.existsSync(fallback)) return { dir: fallback, server: p.server };
  }
  return null;
}

// Which server a wiki folder belongs to, by the protocol it was seeded
// with: Xenolith's says "# Xenolith protocol". Anything else is read as
// Agent Wiki's, as XenoNet always has.
export function contextServerOf(dir) {
  if (!dir) return null;
  try {
    const head = fs.readFileSync(path.join(dir, "PROTOCOL.md"), "utf8").slice(0, 200);
    if (/^#\s*Xenolith protocol/m.test(head)) return "xenolith";
  } catch {}
  return "agent-wiki";
}

export const contextName = (server) => (server === "xenolith" ? "Xenolith" : "Agent Wiki");
