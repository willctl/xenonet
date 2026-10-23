// Xenolith (xenolith/ in this repo), XenoNet's shared context: where its
// folder is, and which MCP server writes to it, so the map can show it as
// its own constellation and beam a session's writes to the pages they
// name. XenoNet only reads the folder, by the format Xenolith documents
// (xenolith/FORMAT.md); Xenolith's own server does the writing. Agent Wiki,
// which Xenolith came from, is found the same way when Xenolith isn't
// there.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// The MCP servers whose calls write to the shared context, by the names
// Xenolith and Agent Wiki install them under. A folder's marker can name
// another (see readMarker), which the config then names (wikiServer).
export const CONTEXT_SERVERS = new Set(["xenolith", "agent-wiki"]);
export const isContextServer = (server) => CONTEXT_SERVERS.has(server);

// Xenolith's tools. Under the server the config names for the shared
// context (wikiServer), a call to one of them goes to the shared context
// whatever that server is called.
export const CONTEXT_TOOLS = new Set(["context_start", "context_search", "context_read", "context_log", "context_upsert_page"]);

// The version of Xenolith's folder format XenoNet reads, and the marker at
// the folder's root that says which version a folder follows and which MCP
// server writes it. Agent Wiki's folders, and Xenolith's from before the
// marker, have none.
export const CONTEXT_FORMAT = 1;
export const MARKER_FILE = "xenolith.json";

// A name an MCP server goes by in a session's tool names
// (mcp__<server>__<tool>) and on the map (resource:<server>).
const SERVER_NAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
// The marker is a line of JSON; anything much bigger isn't one.
const MARKER_MAX = 64 * 1024;

// What a folder's marker says: { format, server }. Null when there's no
// marker, or one XenoNet can't take: not a file, not a JSON object, or
// without a format that's a whole number from 1. Its server is null when
// it isn't a server's name, and the folder's server is then found as if
// it had no marker.
export function readMarker(dir) {
  if (!dir) return null;
  try {
    const file = path.join(dir, MARKER_FILE);
    const st = fs.lstatSync(file);
    if (!st.isFile() || st.size > MARKER_MAX) return null;
    const v = JSON.parse(fs.readFileSync(file, "utf8").replace(/^﻿/, ""));
    if (!v || typeof v !== "object" || Array.isArray(v) || !Number.isSafeInteger(v.format) || v.format < 1) return null;
    return { format: v.format, server: typeof v.server === "string" && SERVER_NAME.test(v.server) ? v.server : null };
  } catch {
    return null;
  }
}

// Why XenoNet can't read a folder whose marker this is, in a sentence for
// the map, or null when it can. A folder in a later format than XenoNet
// knows may have changed in ways it would misread, so XenoNet doesn't
// guess at it.
export function formatProblem(marker) {
  if (!marker || marker.format <= CONTEXT_FORMAT) return null;
  return `This folder follows version ${marker.format} of Xenolith's format, and this XenoNet reads version ${CONTEXT_FORMAT}, so it doesn't show its pages, log or inbox. Update XenoNet to see them.`;
}

// Whether an MCP call goes to the shared context: any call to a server of
// a known name, or a call to one of Xenolith's tools under `named`.
export const isContextCall = (server, tool, named = null) => isContextServer(server) || (typeof named === "string" && named !== "" && server === named && CONTEXT_TOOLS.has(tool));

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

// `makesDir`: Xenolith makes the folder XENOLITH_DIR or its config's
// wikiDir names when it starts, so XenoNet takes that folder before it's
// there (xenolith/FORMAT.md, "Finding the folder").
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
// ~/.agent-wiki) counts while its new one isn't there. The server is the
// one the folder's marker names, else the one whose lookup found it. Null
// when neither is there.
export function findContextDir(options) {
  const o = optsOf(options);
  const found = (dir, server) => ({ dir, server: readMarker(dir)?.server ?? server });
  for (const p of PRODUCTS) {
    const fromEnv = o.env[`${p.env}_DIR`];
    if (fromEnv && (p.makesDir || fs.existsSync(fromEnv))) return found(path.resolve(fromEnv), p.server);
    const fromConfig = namedIn(configFile(p, o));
    if (fromConfig && (p.makesDir || fs.existsSync(fromConfig))) return found(fromConfig, p.server);
    const fallback = path.join(o.home, p.folder);
    if (fs.existsSync(fallback)) return found(fallback, p.server);
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
// the one its marker names. Without a marker, the product whose own lookup
// names that folder (its <ENV>_DIR, its config's wikiDir or its folder in
// home), so a wiki that moved keeps its server. Only for a folder neither
// names does the protocol it was seeded with decide, since people edit it:
// Xenolith's starts "# Xenolith protocol". Anything else is read as Agent
// Wiki's, as XenoNet always has.
export function contextServerOf(dir, options) {
  if (!dir) return null;
  const marked = readMarker(dir)?.server;
  if (marked) return marked;
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

// The shared context's name on the map. Agent Wiki's server is Agent
// Wiki's; any other is Xenolith's, under the name it installs itself as or
// the one its folder's marker gives.
export const contextName = (server) => (typeof server === "string" && server !== "" && server !== "agent-wiki" ? "Xenolith" : "Agent Wiki");

// Xenolith's spaces (xenolith/rust/crates/core/src/spaces.rs): one wiki
// each, for a project, an identity, an actor or a machine. The wiki's own
// folder is this machine's, machine:local; every other is laid out the same
// way in spaces/<kind>/<name>/ under it.
export const SPACE_KINDS = ["project", "identity", "actor", "machine"];
const SPACE_NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;

// The space a context_* call works in, from its `space` as Xenolith's
// server reads it: null for this machine's own (and when none is given),
// the space's id (project:atlas) for another, and undefined for one
// Xenolith refuses, so that the call wrote nothing.
export function spaceOf(v) {
  if (typeof v !== "string" || !v.trim()) return null;
  const t = v.trim().toLowerCase();
  const at = t.indexOf(":");
  if (at < 0) return undefined;
  const [kind, name] = [t.slice(0, at), t.slice(at + 1)];
  if (!SPACE_KINDS.includes(kind) || !SPACE_NAME.test(name) || name.includes("..")) return undefined;
  return kind === "machine" && name === "local" ? null : `${kind}:${name}`;
}
