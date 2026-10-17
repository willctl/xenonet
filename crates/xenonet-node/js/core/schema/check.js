// core/schema/check.js in Rust (crates/xenonet-schema/src/check.rs): data
// checked against a schema's JSON Schema document, with zod's issues and,
// strictly, the keys the schema doesn't declare. A registered schema is
// checked against the documents of every registered schema, which refer to
// each other by $id as the files in schemas/ do; any other zod schema
// against what z.toJSONSchema writes for it. Either is compiled once, the
// registered ones again when the registry grows.
//
// The documents are written for input (io: "input"), as data is read:
// for XenoNet's schemas that's the files themselves, and for z.object() it
// leaves unknown keys open, as zod does, where the files' form would close
// them.
import * as z from "zod/v4";
import { native } from "../../index.js";
import { schemaFor, schemaIds } from "./registry.js";
import { schemaUri } from "./ids.js";

export const pathText = (path) => native.schemaPathText(Array.from(path, (key) => (typeof key === "number" ? key : String(key))));

// The registered schemas, compiled, and which schema has which id.
let registered = null;
const others = new WeakMap();

function compileRegistered(ids) {
  const registry = z.registry();
  for (const id of ids) registry.add(schemaFor(id), { id });
  try {
    return native.schemaCheckers(Object.values(z.toJSONSchema(registry, { uri: schemaUri, io: "input" }).schemas));
  } catch {
    // A schema the registry can't write out (or Rust can't check) is
    // checked on its own below, and says why there.
    return null;
  }
}

function checkerOf(schema) {
  const ids = schemaIds();
  if (registered?.count !== ids.length) registered = { count: ids.length, checker: compileRegistered(ids), ids: new Map(ids.map((id) => [schemaFor(id), id])) };
  const id = registered.ids.get(schema);
  if (id && registered.checker) return [registered.checker, id];
  let own = others.get(schema);
  if (!own) {
    own = native.schemaChecker(z.toJSONSchema(schema, { io: "input" }));
    others.set(schema, own);
  }
  return [own, null];
}

export function check(schemaOrId, data, { strict = false } = {}) {
  const schema = typeof schemaOrId === "string" ? schemaFor(schemaOrId) : schemaOrId;
  const [checker, id] = checkerOf(schema);
  return checker.check(id, data, Boolean(strict));
}

export const explain = (checked) => native.schemaExplain(checked.issues, checked.undeclared);
