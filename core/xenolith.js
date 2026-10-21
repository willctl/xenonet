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
// (xenolith/rust/crates/core/src/paths.rs): <ENV>_CONFIG_DIR, then
// <ENV>_HOME (portable mode, everything in one folder), then the platform's
// own place.
function configDir(names, { env, home, platform }) {
  if (env[`${names.env}_CONFIG_DIR`]) return path.resolve(env[`${names.env}_CONFIG_DIR`]);
  const portable = env[`${names.env}_HOME`];
  if (portable) return path.resolve(portable);
  if (platform === "win32") return path.join(env.APPDATA || path.join(home, "AppData", "Roaming"), names.windows);
  const xdg = abs(env.XDG_CONFIG_HOME);
  if (platform === "darwin") return xdg ? path.join(xdg, names.xdg) : path.join(home, "Library", "Application Support", names.mac);
  return path.join(xdg ?? path.join(home, ".config"), names.xdg);
}

// The config.json a product reads (Xenolith's config_path in
// xenolith/rust/crates/core/src/wiki.rs): the one in its config folder, or,
// until its installer has moved it there, the one in its old folder in home.
// Never the old one in portable mode or with <ENV>_CONFIG_DIR set.
function configFile(p, opts) {
  const file = path.join(configDir(p, opts), "config.json");
  if (!opts.env[`${p.env}_HOME`] && opts.env[`${p.env}_CONFIG_DIR`] === undefined && !fs.existsSync(file)) {
    const legacy = path.join(opts.home, p.legacy, "config.json");
    if (fs.existsSync(legacy)) return legacy;
  }
  return file;
}

// `makesDir`: Xenolith makes the folder XENOLITH_DIR names when it starts,
// so XenoNet takes that folder before it's there.
const PRODUCTS = [
  { server: "xenolith", env: "XENOLITH", windows: "Xenolith", mac: "Xenolith", xdg: "xenolith", folder: "Xenolith", legacy: ".xenolith", makesDir: true },
  { server: "agent-wiki", env: "AGENT_WIKI", windows: "AgentWiki", mac: "Agent Wiki", xdg: "agent-wiki", folder: "AgentWiki", legacy: ".agent-wiki", makesDir: false },
];

// The wikiDir a config file names, there or not.
const namedIn = (file) => {
  try {
    const dir = JSON.parse(fs.readFileSync(file, "utf8").replace(/^﻿/, "")).wikiDir;
    return typeof dir === "string" && dir.trim() ? dir : null;
  } catch {
    return null;
  }
};

const optsOf = ({ env = process.env, home = os.homedir(), platform = process.platform } = {}) => ({ env, home, platform });

// The shared context's folder and the server that owns it: Xenolith's
// (XENOLITH_DIR, its config.json's wikiDir, ~/Xenolith), else Agent Wiki's
// the same way. Each product's old config in home (~/.xenolith,
// ~/.agent-wiki) counts while its new one isn't there. Null when neither is
// there.
export function findContextDir(options) {
  const o = optsOf(options);
  for (const p of PRODUCTS) {
    const fromEnv = o.env[`${p.env}_DIR`];
    if (fromEnv && (p.makesDir || fs.existsSync(fromEnv))) return { dir: path.resolve(fromEnv), server: p.server };
    const fromConfig = namedIn(configFile(p, o));
    if (fromConfig && fs.existsSync(fromConfig)) return { dir: fromConfig, server: p.server };
    const fallback = path.join(o.home, p.folder);
    if (fs.existsSync(fallback)) return { dir: fallback, server: p.server };
  }
  return null;
}

const real = (p) => {
  try {
    return fs.realpathSync.native(p);
  } catch {
    return null;
  }
};

// Two spellings of one folder: the same once resolved, or once their links
// are followed.
function sameDir(a, b) {
  if (path.resolve(a) === path.resolve(b)) return true;
  const [ra, rb] = [real(a), real(b)];
  return ra !== null && ra === rb;
}

// Which server a wiki folder belongs to (one XENONET_WIKI_DIR names, say):
// the product whose own lookup names that folder (its <ENV>_DIR, its
// config's wikiDir or its folder in home), so a wiki that moved keeps its
// server. Only for a folder neither names does the protocol it was seeded
// with decide, since people edit it: Xenolith's starts "# Xenolith
// protocol". Anything else is read as Agent Wiki's, as XenoNet always has.
export function contextServerOf(dir, options) {
  if (!dir) return null;
  const o = optsOf(options);
  for (const p of PRODUCTS) {
    const named = [o.env[`${p.env}_DIR`], namedIn(configFile(p, o)), path.join(o.home, p.folder)];
    if (named.some((d) => d && sameDir(d, dir))) return p.server;
  }
  try {
    const head = fs.readFileSync(path.join(dir, "PROTOCOL.md"), "utf8").slice(0, 200);
    if (/^#\s*Xenolith protocol/m.test(head)) return "xenolith";
  } catch {}
  return "agent-wiki";
}

export const contextName = (server) => (server === "xenolith" ? "Xenolith" : "Agent Wiki");
