// Small persistent state in state.json in the state dir (see config.js):
// dragged node positions, dismissed "needs you" alerts, once-a-day markers,
// how to read the projects folder (your rules, and Codex's picks), and
// identities (your names and pins, and the sign-ins XenoNet has seen). The
// file has a version: an older one is moved up to this one, and one XenoNet
// can't use is never written over (see statefile.js). The store block
// (core/blocks/store.js) reads it, saves it and reports on it.
import path from "node:path";
import { config } from "./config.js";
import { pathKey, isUnder } from "./paths.js";
import { JsonStore } from "./blocks/store.js";
import { STATE_VERSION, STATE_MIGRATIONS, versionOf } from "./statefile.js";

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);

export const stateStore = new JsonStore({
  name: "state",
  // The folder is looked up each time: it can change while XenoNet runs (in
  // tests).
  file: () => path.join(config.stateDir, "state.json"),
  version: STATE_VERSION,
  migrations: STATE_MIGRATIONS,
  versionOf,
  // v1's Codex picks were made for the projects folder of the day.
  context: () => ({ rootKey: config.projectsDir ? pathKey(config.projectsDir) : null }),
  // Every part in place. One that isn't an object (a hand edit that left
  // "identities": "x") can't hold what XenoNet keeps there, and would stop
  // the server as it starts, so it starts afresh and the rest is kept; the
  // doctor names it until XenoNet next saves.
  prepare(state) {
    const repaired = [];
    const part = (parent, key, at, fresh = () => ({})) => {
      if (parent[key] === undefined || parent[key] === null) parent[key] = fresh();
      else if (!isObject(parent[key])) {
        parent[key] = fresh();
        repaired.push(at);
      }
      return parent[key];
    };
    part(state, "workspace", "workspace", () => ({ root: null }));
    for (const k of ["layout", "acks", "marks"]) part(state, k, k);
    const projects = part(state, "projects", "projects");
    for (const k of ["rules", "codexByRoot"]) part(projects, k, `projects.${k}`);
    const identities = part(state, "identities", "identities");
    for (const k of ["names", "hidden", "pins", "known", "signins"]) part(identities, k, `identities.${k}`);
    return repaired;
  },
  what: "its state",
});

export const load = () => stateStore.load();

// What reading the file found and did (see readVersioned).
export const loadReport = () => stateStore.loadReport();

// Changes are saved a moment later, several in one write.
const save = () => stateStore.save();

// Writes what's unsaved now, through a temporary file so a crash can't
// leave half of one. A write that fails (a full disk, a folder XenoNet can't
// write to) is reported and tried again with the next change, or at exit.
export const flush = () => stateStore.flush();

export const getLayout = () => load().layout;

export function setLayout(positions) {
  const s = load();
  for (const [id, p] of Object.entries(positions ?? {})) {
    if (p === null) delete s.layout[id];
    else if ([p.x, p.y, p.z].every(Number.isFinite)) s.layout[id] = { x: +p.x.toFixed(2), y: +p.y.toFixed(2), z: +p.z.toFixed(2) };
  }
  save();
  return s.layout;
}

export function clearLayout() {
  load().layout = {};
  save();
}

export const getAcks = () => load().acks;

export function ack(sessionId, ts = Date.now()) {
  load().acks[sessionId] = ts;
  save();
}

export function mark(name, value) {
  const s = load();
  if (value === undefined) return s.marks[name];
  s.marks[name] = value;
  save();
}

// Your rules for the projects folder, by path key: { path, as, at }, where
// `as` is "project", "group" or "ignore" (see scanProjects).
export const getProjectRules = () => load().projects.rules;

export function setProjectRule(key, rule) {
  const rules = load().projects.rules;
  if (rule) rules[key] = rule;
  else delete rules[key];
  save();
  return rules;
}

// Every rule, or with a projects folder's path key, only the rules for the
// folders inside it: rules stay when the folder changes, so another
// folder's are kept.
export function clearProjectRules(underKey = null) {
  const rules = load().projects.rules;
  for (const key of Object.keys(rules)) if (!underKey || (isUnder(key, underKey) && key !== underKey)) delete rules[key];
  save();
}

// The last Codex pass over each projects folder (see classify.js), by the
// folder's path key: when it ran, a fingerprint of what it was sent, and
// what it decided. A folder's picks are never another's.
export const getCodexPicks = (rootKey) => load().projects.codexByRoot[folderKey(rootKey)] ?? null;

export function setCodexPicks(rootKey, picks) {
  const all = load().projects.codexByRoot;
  if (picks) all[folderKey(rootKey)] = picks;
  else delete all[folderKey(rootKey)];
  save();
}

function folderKey(rootKey) {
  if (typeof rootKey !== "string" || !rootKey) throw new Error("say which projects folder the picks are for");
  return rootKey;
}

// The projects folder you chose (see workspace.js), or null for
// XENONET_PROJECTS_DIR's or ~/Projects.
export const getWorkspaceRoot = () => load().workspace.root;

export function setWorkspaceRoot(dir) {
  load().workspace.root = dir ?? null;
  save();
}

// Identities (see identities.js). Yours: `names` (id → the name you gave it;
// identities with the same name show as one), `hidden` (id → true: no chip
// in the filters, no ring on the map) and `pins` (session or project id →
// { identity, at }: "this ran as" or "this belongs to"). XenoNet's: `known`
// (id → what it last saw and when it first saw it) and `signins` (app → the
// timeline of accounts signed in).
export const getIdentities = () => load().identities;
export const saveIdentities = () => save();

export function setIdentityName(id, name) {
  const names = load().identities.names;
  if (name) names[id] = name;
  else delete names[id];
  save();
}

export function setIdentityHidden(id, hidden) {
  const h = load().identities.hidden;
  if (hidden) h[id] = true;
  else delete h[id];
  save();
}

export function setIdentityPin(nodeId, identity) {
  const pins = load().identities.pins;
  if (identity) pins[nodeId] = { identity, at: Date.now() };
  else delete pins[nodeId];
  save();
}

// The console's theme (see themes.js), by id.
export const getTheme = () => load().theme ?? null;

export function setTheme(id) {
  load().theme = id;
  save();
}

export const getEnvironment = () => load().environment ?? null;
export function setEnvironment(id) { load().environment = id; save(); }
