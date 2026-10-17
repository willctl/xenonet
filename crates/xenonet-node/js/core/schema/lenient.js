// core/schema/lenient.js in Rust (crates/xenonet-schema/src/lenient.rs):
// vendor data checked leniently against XenoNet's vendor schemas. A
// compiled schema is a Rust handle; inspect, accepts, recheck and strip
// walk records in Rust.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { native } from "../../index.js";

export const SCHEMAS_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..", "..", "..", "schemas");

// A value's type as JSON sees it (JS's own typeof for anything else).
export const jsonType = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v === "object" ? "object" : typeof v);

export const compile = (root) => native.compileSchema(root);

export const loadVendor = (source, dir = SCHEMAS_DIR) => native.loadVendor(String(source), String(dir));

export const vendorSources = (dir = SCHEMAS_DIR) => native.vendorSources(String(dir));

// Where, as a drift record says it, from a place ({ parent, key, kind }).
export function pathOf(where) {
  const steps = [];
  for (let w = where; w; w = w.parent) steps.push([w.kind ?? "prop", w.key]);
  return native.pathOf(steps.reverse()) ?? null;
}

export const accepts = (spec, v) => spec.accepts(v);

export const inspect = (spec, value) => spec.inspect(value);

export const recheck = (value, segments, spec) => native.recheck(value, segments, spec);

export const strip = (record, paths) => (paths.length ? native.stripRecord(record, paths) : record);
