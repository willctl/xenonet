import os from "node:os";
import path from "node:path";
import fs from "node:fs";
import { createWorld } from "./demo.js";
import { appDir, findClaudeDesktopDir } from "./dirs.js";

export { appDir };

const home = os.homedir();

// Agent Wiki records its folder in ~/.agent-wiki/config.json.
function findWikiDir() {
  if (process.env.XENONET_WIKI_DIR) return process.env.XENONET_WIKI_DIR;
  try {
    const dir = JSON.parse(fs.readFileSync(path.join(home, ".agent-wiki", "config.json"), "utf8")).wikiDir;
    if (dir && fs.existsSync(dir)) return dir;
  } catch {}
  const fallback = path.join(home, "AgentWiki");
  return fs.existsSync(fallback) ? fallback : null;
}

// Demo mode swaps every source for a made-up workspace in a temp folder.
const demo = process.argv.includes("--demo") || process.env.XENONET_DEMO === "1";
const world = demo ? createWorld() : null;
// The server alone from a checkout (npm run dev) is XenoNet Dev's: its port
// and folders, never the installed XenoNet's, which may be running beside it
// and whose state it would otherwise share. XenoNet Dev's app sets them
// itself.
const serverDev = !demo && process.argv.includes("--dev") && !process.env.XENONET_PORT;
const devDir = (kind) => (serverDev ? appDir(kind, { name: "XenoNet Dev" }) : null);

export const config = {
  demo,
  host: "127.0.0.1",
  port: Number(process.env.XENONET_PORT ?? (demo ? 47841 : serverDev ? 47842 : 47840)),
  homeDir: world?.home ?? home,
  // The projects folder XenoNet reads. Once XenoNet's state starts, it follows
  // the folder in use, which can be one you chose (see workspace.js).
  projectsDir: world?.projectsDir ?? path.resolve(process.env.XENONET_PROJECTS_DIR || path.join(home, "Projects")),
  // How deep and how wide the projects scan goes (see scanProjects): levels
  // below the projects folder, and folders read per scan.
  projectLimits: {
    depth: Math.max(1, Number(process.env.XENONET_PROJECTS_DEPTH) || 3),
    maxDirs: Math.max(1, Number(process.env.XENONET_PROJECTS_MAX_DIRS) || 2000),
  },
  codexHome: world?.codexHome ?? path.resolve(process.env.CODEX_HOME ?? path.join(home, ".codex")),
  claudeDesktopDir: world ? world.claudeDesktopDir : findClaudeDesktopDir(),
  pollMs: Number(process.env.XENONET_POLL_MS ?? 8000),
  // Ask Anthropic for Claude plan usage (see claude-usage.js). Never in demo
  // mode, which makes its own.
  claudeUsage: !demo && process.env.XENONET_CLAUDE_USAGE !== "off",
  claudeProjectsDir: world?.claudeRoot ?? path.join(process.env.CLAUDE_CONFIG_DIR ?? path.join(home, ".claude"), "projects"),
  // Who sessions run as (see identities.js): Claude Code's and Codex's
  // sign-ins, gh's accounts and each repo's git identity. Off with
  // XENONET_IDENTITIES=off. XENONET_HIDE_IDENTITIES=1 starts the page with them
  // masked.
  identities: process.env.XENONET_IDENTITIES !== "off",
  // Actors beside the sessions (see core/actors): the person at the
  // console, the demo's, and in time registered and observed ones.
  // XENONET_ACTORS=off turns the whole layer off: no reports, no list, no
  // events.
  actors: process.env.XENONET_ACTORS !== "off",
  // The numbat bridge (core/actors/numbat.js): the NDJSON file numbat's
  // hooks write events to, read for the agents XenoNet doesn't read itself.
  // Off unless named, and never in the demo.
  numbatFile: !demo && process.env.XENONET_NUMBAT_FILE ? path.resolve(process.env.XENONET_NUMBAT_FILE) : null,
  hideIdentities: process.env.XENONET_HIDE_IDENTITIES === "1",
  claudeConfigFile: world ? path.join(world.home, ".claude.json") : path.join(process.env.CLAUDE_CONFIG_DIR ?? home, ".claude.json"),
  ghConfigDir: world
    ? path.join(world.home, ".config", "gh")
    : process.env.GH_CONFIG_DIR ?? (process.env.XDG_CONFIG_HOME ? path.join(process.env.XDG_CONFIG_HOME, "gh") : process.platform === "win32" && process.env.APPDATA ? path.join(process.env.APPDATA, "GitHub CLI") : path.join(home, ".config", "gh")),
  stateDir: world?.stateDir ?? path.resolve(process.env.XENONET_STATE_DIR ?? devDir("state") ?? appDir("state")),
  cacheDir: world?.cacheDir ?? path.resolve(process.env.XENONET_CACHE_DIR ?? devDir("cache") ?? appDir("cache")),
  wikiDir: world ? world.wikiDir : findWikiDir(),
  // MCP servers that are part of the host apps rather than shared systems;
  // they don't get their own resource node.
  hiddenResources: new Set(
    (process.env.XENONET_HIDE_RESOURCES ?? "node_repl,cua_repl,codex_app,codex_apps,visualize,terminal,mcp-registry,scheduled-tasks")
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean),
  ),
  // A session counts as "active" while its last write is this recent.
  activeWindowMs: 10 * 60 * 1000,
  // A project (or Codex project, or folder outside ~/Projects) is lit as
  // active while one of its sessions ran this recently; the rest dim.
  projectActiveMs: Number(process.env.XENONET_ACTIVE_MINUTES ?? 60) * 60 * 1000,
  recentWindowMs: 24 * 60 * 60 * 1000,
};
