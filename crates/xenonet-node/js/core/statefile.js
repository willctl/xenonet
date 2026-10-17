// core/statefile.js in Rust (crates/xenonet-state/src/statefile.rs):
// state.json's version and the step from version 1. Reading the file is
// the store's (blocks/store.js), and the doctor's look at the folder,
// inspectState, is still the JS's.
import { native } from "../index.js";
import { migrate } from "./migrate.js";
import { pathKey } from "./paths.js";
import { readVersioned, versionOf } from "./blocks/store.js";
export { inspectState } from "../../../../core/statefile.js";

export const STATE_VERSION = native.stateVersion();

export const STATE_MIGRATIONS = [{ from: 1, to: 2, up: (s, { rootKey = null } = {}) => native.stateUp1To2(s, rootKey ?? null) }];

export { versionOf };

export function migrateState(data, { projectsDir = null } = {}) {
  return migrate(data, STATE_MIGRATIONS, { from: versionOf(data), to: STATE_VERSION, context: { rootKey: projectsDir ? pathKey(projectsDir) : null } }).doc;
}

export function readStateFile(file, { projectsDir = null, now = Date.now() } = {}) {
  const { doc, report } = readVersioned(file, { version: STATE_VERSION, migrations: STATE_MIGRATIONS, versionOf, context: { rootKey: projectsDir ? pathKey(projectsDir) : null }, now });
  return { state: doc, report };
}
