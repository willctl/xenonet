// Checks a value against one of XenoNet's JSON Schemas (schemas/<name>.json),
// the same files the Rust service checks its answers against
// (crates/xenonet-schema). `strict` also refuses keys a schema doesn't
// declare, as the service does when it checks what it writes.
import fs from "node:fs";
import Ajv2020 from "ajv/dist/2020.js";
import addFormats from "ajv-formats";

const dir = new URL("../../schemas/", import.meta.url);
const compiled = new Map();

// The schema with every object that lists its properties closed to others.
function closed(schema) {
  if (Array.isArray(schema)) return schema.map(closed);
  if (!schema || typeof schema !== "object") return schema;
  const out = Object.fromEntries(Object.entries(schema).map(([k, v]) => [k, closed(v)]));
  if (out.type === "object" && out.properties && (out.additionalProperties === undefined || (typeof out.additionalProperties === "object" && !Object.keys(out.additionalProperties).length))) out.additionalProperties = false;
  return out;
}

function validator(name, strict) {
  const key = `${name}${strict ? ":strict" : ""}`;
  if (!compiled.has(key)) {
    const schema = JSON.parse(fs.readFileSync(new URL(`${name}.json`, dir), "utf8"));
    const ajv = new Ajv2020({ strict: false, allErrors: true });
    addFormats(ajv);
    compiled.set(key, ajv.compile(strict ? closed(schema) : schema));
  }
  return compiled.get(key);
}

// { ok, errors }: errors as "<path> <what's wrong>" lines.
export function check(name, value, { strict = false } = {}) {
  const v = validator(name, strict);
  const ok = v(value);
  return { ok, errors: ok ? [] : v.errors.map((e) => `${e.instancePath || "/"} ${e.message}${e.params?.additionalProperty ? ` (${e.params.additionalProperty})` : ""}`) };
}

export const explain = (r) => r.errors.join("\n");

// Throws, as a schema's parse did, when the value doesn't fit.
export function parse(name, value, options) {
  const r = check(name, value, options);
  if (!r.ok) throw new Error(`not ${name}:\n${explain(r)}`);
  return value;
}
