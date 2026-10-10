// The projects folder XenoNet reads, its root (see scanProjects):
// XENONET_PROJECTS_DIR when it's set, else the folder you chose in the console
// or with xenonet root (state.json's workspace.root), else ~/Projects. The demo
// reads its own made-up home's, and never XENONET_PROJECTS_DIR.
//
// A folder is checked before XenoNet reads it: it has to exist, be a folder
// XenoNet can read, and be neither the whole disk nor your home folder (nor a
// folder holding it), judged by where it leads when it's a link. A quick
// scan, with the real one's limits so the numbers match, says how many
// projects and groups XenoNet would find there. XenoNetState.setRoot does the
// switch, and says what it does to the rest.
import fs from "node:fs";
import path from "node:path";
import { config, appDir } from "./config.js";
import { candidates, PORTS } from "./client.js";
import { INSTANCES } from "./instances.js";
import { pathKey, isUnder, tilde } from "./paths.js";
import { scanProjects, VCS_MARKERS } from "./projects.js";
import * as store from "./store.js";

const fromEnv = () => {
  const dir = config.demo ? null : process.env.XENONET_PROJECTS_DIR;
  return dir ? path.resolve(dir) : null;
};

// The folder you chose, as state.json has it, read as the file is:
// nothing moved up to this version, set aside or written. For what only
// looks (xenonet doctor and npm run contracts with no XenoNet running), since
// the file is a running XenoNet's to change. Null when there's none, or no
// file XenoNet can read.
export function savedRoot(file = path.join(config.stateDir, "state.json")) {
  try {
    const saved = JSON.parse(fs.readFileSync(file, "utf8"))?.workspace?.root;
    return typeof saved === "string" ? saved : null;
  } catch {
    return null;
  }
}

// The folder an instance of XenoNet keeps its "state" or "cache" in, as its
// server has it (XenoNet Dev's, run from a checkout: electron/main.js), or
// the one XENONET_STATE_DIR or XENONET_CACHE_DIR says. For what only looks at
// a XenoNet it can't ask: its state.json is in that instance's folder, not
// in the installed XenoNet's.
export function instanceDir(kind, name, { env = process.env } = {}) {
  const set = env[kind === "state" ? "XENONET_STATE_DIR" : "XENONET_CACHE_DIR"];
  return set ? path.resolve(set) : appDir(kind, { name });
}

// The instance a command that finds no XenoNet running stands in for, by
// name, when it isn't the installed XenoNet: XenoNet Dev, when it's the one
// asked about (--dev, or its port, as the MCP server XenoNet Dev connects
// asks). Null for XenoNet.
export function standsFor(opts = {}) {
  const asked = candidates(opts);
  return asked.length === 1 && asked[0] === PORTS.dev ? INSTANCES.dev.name : null;
}

// The folder in use, and where that comes from: `source` is "env"
// (XENONET_PROJECTS_DIR), "chosen" or "default", and `env`, `chosen` and
// `default` say what each would be. `home` is the folder ~ stands for.
// With `peek`, the choice is read from the file as it is (savedRoot), not
// through XenoNet's store: from `stateDir`'s state.json (XenoNet's own state
// folder by default).
export function root({ peek = false, stateDir = config.stateDir } = {}) {
  const env = fromEnv();
  const saved = peek ? savedRoot(path.join(stateDir, "state.json")) : store.getWorkspaceRoot();
  // One that isn't a full path (state.json edited by hand) is no choice.
  const chosen = typeof saved === "string" && path.isAbsolute(saved) ? saved : null;
  const fallback = path.join(config.homeDir, "Projects");
  return { path: env ?? chosen ?? fallback, source: env ? "env" : chosen ? "chosen" : "default", env, chosen, default: fallback, home: config.homeDir };
}

// config.projectsDir follows the folder in use: XenoNet's state sets it as
// it starts, and again with each change; npm run contracts as it starts,
// with `peek`.
export function applyRoot(opts) {
  const r = root(opts);
  config.projectsDir = r.path;
  return r;
}

// Why the folder can't be chosen here, or null: while XENONET_PROJECTS_DIR is
// set, it decides, and a choice here would do nothing.
export function locked() {
  const env = fromEnv();
  return env ? Object.assign(new Error(`XENONET_PROJECTS_DIR is set, so it decides which folder XenoNet reads (${tilde(env, config.homeDir)}). To choose one here, unset it and restart XenoNet.`), { status: 409 }) : null;
}

// A folder with its links followed, the path sessions record (Codex's is
// getcwd's), or as given when it can't be followed (it's gone).
const real = (dir) => {
  try {
    return fs.realpathSync.native(dir);
  } catch {
    return path.resolve(dir);
  }
};

// Saves the folder you chose (null: none) and puts it in use. The default
// folder itself is saved as none, so it stays the default. A link is saved
// as the folder it leads to, so sessions in it count as the folder's.
export function choose(dir) {
  const no = locked();
  if (no) throw no;
  store.setWorkspaceRoot(dir && pathKey(real(dir)) !== pathKey(real(root().default)) ? real(dir) : null);
  return applyRoot();
}

// A folder as someone typed it: ~ and ~/… are the home folder (the demo's,
// in the demo); anything else has to be a full path. Null when it isn't one.
export function resolveFolder(input, home = config.homeDir) {
  const s = typeof input === "string" ? input.trim() : "";
  if (s === "~") return path.resolve(home);
  if (/^~[\\/]/.test(s)) return path.resolve(home, s.slice(2));
  return s && path.isAbsolute(s) ? path.resolve(s) : null;
}

const privacy = (shown) =>
  process.platform === "darwin" ? ` Let XenoNet read it in System Settings › Privacy & Security › Files and Folders (or Full Disk Access), then try again.` : ` Check the permissions on ${shown}.`;

// Whether XenoNet can read a folder as its projects folder: { ok, path, link,
// error }, with the folder's entries when it can. A link is judged by where
// it leads, home folder included, and `path` is that folder: `link` is the
// path given when it was a link, else null.
export function validate(input, { home = config.homeDir } = {}) {
  const dir = resolveFolder(input, home);
  const no = (error) => ({ ok: false, path: dir, link: null, error });
  if (!dir) return no("Give the folder's full path, or start it with ~.");
  const shown = tilde(dir, home);
  let st, target;
  try {
    st = fs.statSync(dir, { throwIfNoEntry: false });
    if (st) target = fs.realpathSync.native(dir);
  } catch (err) {
    return no(`XenoNet can't reach ${shown} (${err.code}).${privacy(shown)}`);
  }
  if (!st) return no(`${shown} doesn't exist.`);
  if (!st.isDirectory()) return no(`${shown} isn't a folder.`);
  const link = pathKey(target) !== pathKey(dir);
  const that = (what) => (link ? `${shown} is a link to ${what}.` : `That's ${what}.`);
  if (path.parse(target).root === target) return no(`${that("the whole disk")} Choose the folder that holds your projects.`);
  const key = pathKey(target), homeKey = pathKey(real(home));
  if (key === homeKey) return no(`${that("your home folder")} Choose the folder in it that holds your projects, such as ~/Projects.`);
  if (isUnder(homeKey, key)) return no(`${link ? `${shown} is a link to ${tilde(target, home)}, which` : shown} holds your home folder. Choose the folder that holds your projects.`);
  let names;
  try {
    names = fs.readdirSync(target);
  } catch (err) {
    return no(`XenoNet can't read ${shown} (${err.code}).${privacy(shown)}`);
  }
  return { ok: true, path: target, link: link ? dir : null, error: null, names };
}

// What XenoNet would find in a folder, without reading it for the map: why
// it can't use it, or how many projects and groups a scan finds there with
// these sessions' folders and rules, and anything worth knowing first. A
// scan that takes longer than `timeMs` isn't waited for, and is stopped,
// rather than left reading a big or slow folder for nothing. `scan` stands
// in for scanProjects in tests.
export async function check(input, { sessionDirs = [], rules = {}, limits = config.projectLimits, timeMs = 5000, home = config.homeDir, scan = scanProjects } = {}) {
  const v = validate(input, { home });
  const current = !!v.path && pathKey(v.ok ? v.path : real(v.path)) === pathKey(real(config.projectsDir));
  if (!v.ok) return { ok: false, path: v.path, link: null, error: v.error, current, warnings: [], scan: null };
  const shown = tilde(v.path, home);
  const warnings = [];
  if (v.link) warnings.push(`${tilde(v.link, home)} is a link, so XenoNet would read the folder it leads to, ${shown}.`);
  if (VCS_MARKERS.some((m) => v.names.includes(m))) warnings.push(`${shown} is a repo itself, so XenoNet would show the folders in it as projects. Choose the folder that holds your repos.`);
  let timer;
  const stop = new AbortController();
  // Given up on first, then stopped: a scan that rejects as it's stopped
  // mustn't win the race.
  const late = new Promise((resolve) => (timer = setTimeout(() => (resolve(null), stop.abort()), timeMs)));
  const found = await Promise.race([scan(v.path, { sessionDirs, rules, limits, signal: stop.signal }), late]).finally(() => clearTimeout(timer));
  const sc = found?.scan;
  if (sc && !sc.ok) return { ok: false, path: v.path, link: v.link, error: `XenoNet can't read ${shown}.`, current, warnings: [], scan: null };
  if (!sc) warnings.push(`Scanning ${shown} took more than ${timeMs / 1000} s: it's big, or slow to read, and XenoNet reads it again every few seconds.`);
  else if (!sc.projects && !sc.groups) warnings.push(`XenoNet finds no projects in ${shown}: no repos, and no folders in it.`);
  if (sc?.truncated) warnings.push(`The scan stopped at its ${sc.maxDirs}-folder limit, so some projects may be missing. XENONET_PROJECTS_MAX_DIRS raises it.`);
  return { ok: true, path: v.path, link: v.link, error: null, current, warnings, scan: sc ? { projects: sc.projects, groups: sc.groups, dirsRead: sc.dirsRead, truncated: sc.truncated, depth: sc.depth, maxDirs: sc.maxDirs, ms: sc.ms } : null };
}
