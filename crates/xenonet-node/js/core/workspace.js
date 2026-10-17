// core/workspace.js in Rust (crates/xenonet-state/src/workspace.rs): the
// projects folder XenoNet reads, choosing another, and checking a folder
// before XenoNet reads it. Rust reads the folders and decides; the config,
// the store, the scan and its time limit stay here.
import path from "node:path";
import { config, appDir } from "../../../../core/config.js";
import { candidates, PORTS } from "./client.js";
import { INSTANCES } from "../../../../core/instances.js";
import { scanProjects, VCS_MARKERS } from "../../../../core/projects.js";
import * as store from "./store.js";
import { native } from "../index.js";

const fromEnv = () => (config.demo ? null : process.env.XENONET_PROJECTS_DIR || null);

export const savedRoot = (file = path.join(config.stateDir, "state.json")) => native.savedRoot(String(file)) ?? null;

export function instanceDir(kind, name, { env = process.env } = {}) {
  const set = env[native.instanceDirVar(String(kind))];
  return set ? path.resolve(set) : appDir(kind, { name });
}

export const standsFor = (opts = {}) => native.standsFor(candidates(opts), PORTS.dev, INSTANCES.dev.name) ?? null;

export function root({ peek = false, stateDir = config.stateDir } = {}) {
  const saved = peek ? savedRoot(path.join(stateDir, "state.json")) : store.getWorkspaceRoot();
  return native.workspaceRoot(fromEnv(), saved, String(config.homeDir));
}

export function applyRoot(opts) {
  const r = root(opts);
  config.projectsDir = r.path;
  return r;
}

export function locked() {
  const why = native.workspaceLocked(fromEnv(), String(config.homeDir));
  return why ? Object.assign(new Error(why), { status: 409 }) : null;
}

export function choose(dir) {
  const no = locked();
  if (no) throw no;
  store.setWorkspaceRoot(native.workspaceChosen(dir ? String(dir) : null, root().default) ?? null);
  return applyRoot();
}

export const resolveFolder = (input, home = config.homeDir) => native.resolveFolder(input, String(home)) ?? null;

export const validate = (input, { home = config.homeDir } = {}) => native.validateFolder(input, String(home), process.platform);

export async function check(input, { sessionDirs = [], rules = {}, limits = config.projectLimits, timeMs = 5000, home = config.homeDir, scan = scanProjects } = {}) {
  const start = native.checkStart(input, String(home), String(config.projectsDir), process.platform, VCS_MARKERS);
  if (start.done) return start.done;
  let timer;
  const stop = new AbortController();
  // Given up on first, then stopped: a scan that rejects as it's stopped
  // mustn't win the race.
  const late = new Promise((resolve) => (timer = setTimeout(() => (resolve(null), stop.abort()), timeMs)));
  const found = await Promise.race([scan(start.path, { sessionDirs, rules, limits, signal: stop.signal }), late]).finally(() => clearTimeout(timer));
  return native.checkFinish(start, found?.scan ?? null, Number(timeMs));
}
