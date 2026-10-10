// Reading state.json safely: its version, moving an older one up to this
// one, and never losing a file XenoNet can't use (contracts, section 5).
//
//   - No file: a fresh state.
//   - Not JSON, or not an object: moved aside to state.broken-<time>.json,
//     where the doctor reports it, and XenoNet starts afresh. Never replaced
//     by an empty state and written over.
//   - An older version: copied to state.v<n>.json first, then migrated, and
//     saved as this version.
//   - A newer version: read as it is, and never written back, since this
//     XenoNet would write it in an older shape.
//   - A file XenoNet can't read, back up or move aside: a fresh or migrated
//     state in memory, and the file left alone.
//
// The store block (core/blocks/store.js) does the reading, for any versioned
// JSON file; this has state.json's version and migrations.
//
// No schema check here, which would put zod on the server's way up:
// inspectState() does that for the doctor.
import fs from "node:fs";
import path from "node:path";
import { migrate } from "./migrate.js";
import { pathKey } from "./paths.js";
import { readVersioned, versionOf } from "./blocks/store.js";

export const STATE_VERSION = 2;

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);

export const STATE_MIGRATIONS = [
  {
    from: 1,
    to: 2,
    // v2 adds the version, the workspace's root (null: XENONET_PROJECTS_DIR,
    // else ~/Projects) and Codex's picks for each root. v1's picks were made
    // for the projects folder of the day, so they go under its key.
    // A root or picks per root already there (written by code from before
    // the versions met) are kept.
    up(s, { rootKey = null } = {}) {
      const { version, workspace, projects, ...rest } = s;
      const { codex = null, codexByRoot, ...others } = isObject(projects) ? projects : {};
      return {
        version: 2,
        workspace: { root: null, ...(isObject(workspace) ? workspace : {}) },
        ...rest,
        projects: { ...others, codexByRoot: { ...(codex && rootKey ? { [rootKey]: codex } : {}), ...(isObject(codexByRoot) ? codexByRoot : {}) } },
      };
    },
  },
];

// A file without a version is v1, from before there were versions.
export { versionOf };

// `data` (at its version) as this version's shape. `projectsDir` is the
// projects folder Codex's picks in a v1 file were made for.
export function migrateState(data, { projectsDir = null } = {}) {
  return migrate(data, STATE_MIGRATIONS, { from: versionOf(data), to: STATE_VERSION, context: { rootKey: projectsDir ? pathKey(projectsDir) : null } }).doc;
}

// { state, report }, from the store block's reader (see readVersioned in
// core/blocks/store.js). The report says what was found and done:
// `version` (the file's), `migrated` ({ from, to }), `backup` and `broken`
// (files written or moved aside), `readOnly` (never write the file) and
// `problem` (why, in words).
export function readStateFile(file, { projectsDir = null, now = Date.now() } = {}) {
  const { doc, report } = readVersioned(file, { version: STATE_VERSION, migrations: STATE_MIGRATIONS, versionOf, context: { rootKey: projectsDir ? pathKey(projectsDir) : null }, now });
  return { state: doc, report };
}

// What's in the state folder, for the doctor: state.json's version and
// whether it fits its schema, and the files XenoNet moved aside.
export async function inspectState(dir) {
  const file = path.join(dir, "state.json");
  let names = [];
  try {
    names = fs.readdirSync(dir);
  } catch {}
  const out = { file, exists: names.includes("state.json"), known: STATE_VERSION, version: null, unreadable: null, issues: [], broken: [], backups: [] };
  for (const name of names.sort()) {
    if (/^state\.broken-.+\.json$/.test(name)) out.broken.push(path.join(dir, name));
    else if (/^state\.v\d+(-.+)?\.json$/.test(name)) out.backups.push(path.join(dir, name));
  }
  if (!out.exists) return out;
  let data;
  try {
    data = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (err) {
    out.unreadable = err instanceof SyntaxError ? "isn't valid JSON" : `can't be read (${err.code ?? err.message})`;
    return out;
  }
  if (!isObject(data)) {
    out.unreadable = "isn't a JSON object";
    return out;
  }
  out.version = versionOf(data);
  if (Number.isInteger(out.version) && out.version >= 1 && out.version <= STATE_VERSION) {
    const { check } = await import("./schema/index.js");
    out.issues = check(`state/v${out.version}`, data).issues;
  }
  return out;
}
