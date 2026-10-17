// core/actors/hermes-files.js in Rust (crates/xenonet-actors/src/hermes_files.rs):
// Hermes Agent's state on this machine, read for metadata only. What's read
// and what it means are Rust's; the files are read through `files`, the
// vendor readers check them, and Hermes's state.db is opened here (with
// node:sqlite, or the `open` a test gives) and queried with Rust's SQL.
import fs from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { reader } from "../vendor.js";
import { native } from "../../index.js";

const C = native.hermesConstants();
export const SOURCE = C.source;
export const EDIT_TOOLS = C.editTools;

const vendor = {
  check: (source, value, version) => reader(source).check(value, { version }),
  unparsable: (source, version) => reader(source).unparsable(version),
  missing: (source, path, version) => reader(source).missing(path, version),
};
const depsOf = (files) => ({
  files,
  vendor,
  dirs: (dir) => files.readdirSync(dir, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name),
  dateParse: (s) => Date.parse(s),
});

export const msOf = (v) => native.hermesMsOf(depsOf(fs), v) ?? null;
export const sameStart = (recorded, tableStart, slackMs = 2000) => native.hermesSameStart(recorded, tableStart, slackMs);

export const hermesRoot = (home, env = process.env) => native.hermesRoot(String(home), env.HERMES_HOME ? String(env.HERMES_HOME) : undefined);
export const listProfiles = (root, { files = fs } = {}) => native.hermesListProfiles(depsOf(files), String(root));
export const installed = (root, { files = fs } = {}) => native.hermesInstalled(depsOf(files), String(root));
export const readStamp = (root, { files = fs } = {}) => native.hermesReadStamp(depsOf(files), String(root));
export const readLedger = (root, { version = null, files = fs } = {}) => native.hermesRead(depsOf(files), "ledger", String(root), version);
export const readGateway = (home, { version = null, files = fs } = {}) => native.hermesRead(depsOf(files), "gateway", String(home), version);
export const readTicker = (home, { version = null, files = fs } = {}) => native.hermesRead(depsOf(files), "ticker", String(home), version);
export const readProcesses = (home, { version = null, files = fs } = {}) => native.hermesRead(depsOf(files), "processes", String(home), version);
export const readJobs = (home, { version = null, files = fs } = {}) => native.hermesRead(depsOf(files), "jobs", String(home), version);

export function readStateDb(file, { afterId = null, sinceMs, sessionIds = [], open = (f) => new DatabaseSync(f, { readOnly: true }), files = fs } = {}) {
  let db = null;
  const deps = {
    ...depsOf(files),
    open: () => void (db = open(file)),
    all: (sql, args) => db.prepare(sql).all(...args),
    get: (sql, args) => db.prepare(sql).get(...args) ?? null,
    close: () => db?.close(),
  };
  return native.hermesReadStateDb(deps, String(file), afterId, sinceMs, [...sessionIds]);
}
