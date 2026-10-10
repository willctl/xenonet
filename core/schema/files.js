// The JSON Schema files in schemas/, against the definitions: which are
// missing, out of date or left over, and writing them. npm run schemas
// (scripts/schemas.mjs) writes them, and a unit test fails until they're
// current.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { schemaFiles } from "./index.js";

export const SCHEMAS_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "schemas");

// The .json files under `dir`, as paths inside it with forward slashes.
function listJson(dir, at = "") {
  let ents = [];
  try {
    ents = fs.readdirSync(path.join(dir, at), { withFileTypes: true });
  } catch {}
  return ents.flatMap((e) => {
    const rel = at ? `${at}/${e.name}` : e.name;
    if (e.isDirectory()) return listJson(dir, rel);
    return e.isFile() && e.name.endsWith(".json") ? [rel] : [];
  });
}

const read = (file) => {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
};

export function schemaFilesState(dir = SCHEMAS_DIR) {
  const want = schemaFiles();
  const have = new Set(listJson(dir));
  const missing = want.filter((f) => !have.has(f.file)).map((f) => f.file);
  const changed = want.filter((f) => have.has(f.file) && read(path.join(dir, f.file)) !== f.text).map((f) => f.file);
  const extra = [...have].filter((f) => !want.some((w) => w.file === f)).sort();
  return { current: !missing.length && !changed.length && !extra.length, missing, changed, extra };
}

// Writes what's missing or out of date, and removes files for schemas that
// are no longer defined, with folders left empty. Says what it did.
export function writeSchemaFiles(dir = SCHEMAS_DIR) {
  const { missing, changed, extra } = schemaFilesState(dir);
  const todo = new Set([...missing, ...changed]);
  for (const { file, text } of schemaFiles()) {
    if (!todo.has(file)) continue;
    fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
    fs.writeFileSync(path.join(dir, file), text);
  }
  for (const file of extra) {
    fs.rmSync(path.join(dir, file));
    for (let d = path.dirname(path.join(dir, file)); d.startsWith(dir + path.sep) && !fs.readdirSync(d).length; d = path.dirname(d)) fs.rmdirSync(d);
  }
  return { written: [...todo].sort(), removed: extra };
}
