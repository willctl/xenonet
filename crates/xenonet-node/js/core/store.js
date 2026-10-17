// core/store.js in Rust (crates/xenonet-state/src/store.rs): the small
// state XenoNet keeps in state.json, read and saved by the JSON store's
// twin. Rust says what a loaded document must hold and what each change
// does; the document stays the object the store loaded, changed in place,
// since its callers keep hold of its parts.
import path from "node:path";
import { config } from "../../../../core/config.js";
import { pathKey } from "./paths.js";
import { JsonStore } from "./blocks/store.js";
import { STATE_VERSION, STATE_MIGRATIONS, versionOf } from "./statefile.js";
import { native } from "../index.js";

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
  prepare(state) {
    const { parts, repaired } = native.prepareState(state);
    for (const [k, v] of Object.entries(parts)) state[k] = v;
    return repaired;
  },
  what: "its state",
});

export const load = () => stateStore.load();
export const loadReport = () => stateStore.loadReport();
const save = () => stateStore.save();
export const flush = () => stateStore.flush();

export const getLayout = () => load().layout;

export function setLayout(positions) {
  const s = load();
  for (const [id, p] of native.layoutChanges(positions ?? {})) {
    if (p === null) delete s.layout[id];
    else s.layout[id] = p;
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

export const getProjectRules = () => load().projects.rules;

export function setProjectRule(key, rule) {
  const rules = load().projects.rules;
  if (rule) rules[key] = rule;
  else delete rules[key];
  save();
  return rules;
}

export function clearProjectRules(underKey = null) {
  const rules = load().projects.rules;
  for (const key of native.rulesToClear(Object.keys(rules), underKey ? String(underKey) : null)) delete rules[key];
  save();
}

export const getCodexPicks = (rootKey) => load().projects.codexByRoot[native.picksFolderKey(rootKey)] ?? null;

export function setCodexPicks(rootKey, picks) {
  const all = load().projects.codexByRoot;
  const key = native.picksFolderKey(rootKey);
  if (picks) all[key] = picks;
  else delete all[key];
  save();
}

export const getWorkspaceRoot = () => load().workspace.root;

export function setWorkspaceRoot(dir) {
  load().workspace.root = dir ?? null;
  save();
}

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

export const getTheme = () => load().theme ?? null;

export function setTheme(id) {
  load().theme = id;
  save();
}

export const getEnvironment = () => load().environment ?? null;
export function setEnvironment(id) {
  load().environment = id;
  save();
}
