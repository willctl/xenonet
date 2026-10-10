// Checking data against XenoNet's schemas.
//
// check() gives zod's issues as paths and messages, and with `strict`, the
// keys the data has that its schema doesn't declare. Every object is loose,
// so zod lets those through: tests check strictly to keep the schemas
// complete, and the lenient reader for vendor data will record them as
// drift.
import * as z from "zod/v4";
import { schemaFor } from "./registry.js";

const isPlain = (v) => v != null && typeof v === "object" && !Array.isArray(v);

// A path as it reads in a message: nodes[3].ranAs.id, layout["project:/x"].
export function pathText(path) {
  let out = "";
  for (const key of path) {
    if (typeof key === "number") out += `[${key}]`;
    else if (/^[A-Za-z_$][\w$-]*$/.test(String(key))) out += out ? `.${String(key)}` : String(key);
    else out += `[${JSON.stringify(String(key))}]`;
  }
  return out;
}

// zod's issues, flattened: a union that failed every option reports the
// option that came closest, since that's most likely the one meant.
function flatten(issues, at = []) {
  const out = [];
  for (const issue of issues) {
    const path = [...at, ...issue.path];
    if (issue.code === "invalid_union" && issue.errors?.length) {
      const closest = issue.errors.reduce((best, e) => (e.length < best.length ? e : best));
      out.push(...flatten(closest, path));
    } else out.push({ path: pathText(path), code: issue.code, message: issue.message });
  }
  return out;
}

// The fields an object schema declares (an intersection's from both sides),
// or null for a schema that isn't an object.
function fieldsOf(schema) {
  const def = schema._zod.def;
  if (def.type === "object") return new Map(Object.entries(def.shape));
  if (def.type !== "intersection") return null;
  const [left, right] = [fieldsOf(def.left), fieldsOf(def.right)];
  return left && right ? new Map([...left, ...right]) : (left ?? right);
}

const passesThrough = (schema) => {
  const def = schema._zod.def;
  if (z.globalRegistry.get(schema)?.["x-xenonet"]?.passthrough) return true;
  return def.type === "intersection" && (passesThrough(def.left) || passesThrough(def.right));
};

// The keys in `value` that `schema` doesn't declare, as paths.
function undeclared(schema, value, at, out) {
  const def = schema._zod.def;
  switch (def.type) {
    case "optional":
    case "nullable":
    case "default":
    case "prefault":
    case "nonoptional":
    case "readonly":
    case "catch":
      return value == null ? out : undeclared(def.innerType, value, at, out);
    case "pipe":
      return undeclared(def.in, value, at, out);
    case "lazy":
      return undeclared(def.getter(), value, at, out);
    case "array":
      if (Array.isArray(value)) value.forEach((v, i) => undeclared(def.element, v, [...at, i], out));
      return out;
    case "tuple":
      if (Array.isArray(value)) value.forEach((v, i) => (def.items[i] ?? def.rest) && undeclared(def.items[i] ?? def.rest, v, [...at, i], out));
      return out;
    case "record":
      if (isPlain(value)) for (const [k, v] of Object.entries(value)) undeclared(def.valueType, v, [...at, k], out);
      return out;
    case "union": {
      const option = def.options.find((o) => o.safeParse(value).success);
      return option ? undeclared(option, value, at, out) : out;
    }
    case "object":
    case "intersection": {
      if (!isPlain(value)) return out;
      const fields = fieldsOf(schema);
      const open = passesThrough(schema);
      for (const [k, v] of Object.entries(value)) {
        if (fields.has(k)) undeclared(fields.get(k), v, [...at, k], out);
        else if (!open && v !== undefined) out.push(pathText([...at, k]));
      }
      return out;
    }
    default:
      return out;
  }
}

// { ok, issues: [{ path, code, message }], undeclared: [path] } for `data`
// against a schema, or a schema's id.
export function check(schemaOrId, data, { strict = false } = {}) {
  const schema = typeof schemaOrId === "string" ? schemaFor(schemaOrId) : schemaOrId;
  const result = schema.safeParse(data);
  const issues = result.success ? [] : flatten(result.error.issues);
  const extra = strict && result.success ? undeclared(schema, data, [], []) : [];
  return { ok: !issues.length && !extra.length, issues, undeclared: extra };
}

// What check() found, as lines for an error message.
export const explain = (checked) => [...checked.issues.map((i) => `${i.path || "(the value)"}: ${i.message}`), ...checked.undeclared.map((p) => `${p}: not in the schema`)].join("\n");
