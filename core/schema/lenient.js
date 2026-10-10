// Vendor data checked leniently against XenoNet's vendor schemas (contracts,
// section 1), as npm run schemas writes them in schemas/vendor/: JSON
// Schema, read with fs. zod isn't loaded at runtime, so nothing on the
// server's way up pays for it (docs/design/adr/0001-schemas.md); the files
// are what XenoNet checks against, and a unit test keeps them in step with
// the definitions in core/schema/vendor/.
//
// compile() turns a schema file into a spec, and inspect() walks a record
// against it, the way the record would be read: the fields the schema
// declares, each on its own. It finds
//   - a required field that isn't there (missing-field),
//   - a field of another type (type-change), which strip() takes out of
//     the record, so a reader sees it as missing and falls back the way it
//     does for a missing field,
//   - a discriminator's value the schema doesn't list (new-variant),
//   - a key a closed object doesn't list (new-field),
// and a record that isn't an object at all, which nothing can be read from
// (drop). Other keys pass: vendor objects are open unless the schema closes
// them (core/schema/vendor/common.js). Arrays and maps are checked a sample
// of elements at a time, since a record can hold hundreds.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isField } from "../drift.js";

export const SCHEMAS_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "schemas");

export const jsonType = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v === "object" ? "object" : typeof v);

const ANY = { kind: "any" };
const isEmpty = (node) => !node || typeof node !== "object" || Object.keys(node).every((k) => k === "description" || k === "title" || k === "$comment");

// ---------- compiling ----------

// A schema file, or a part of one, as the spec inspect() walks.
export function compile(root) {
  const seen = new Map();
  const resolve = (ref) => {
    const m = /^#\/\$defs\/(.+)$/.exec(ref);
    return m ? root.$defs?.[m[1].replaceAll("~1", "/").replaceAll("~0", "~")] : null;
  };
  const go = (node) => {
    if (isEmpty(node)) return ANY;
    if (seen.has(node)) return seen.get(node);
    if (node.$ref) {
      const target = resolve(node.$ref);
      return target ? go(target) : ANY;
    }
    const spec = {};
    seen.set(node, spec);
    const xs = node["x-xenonet"] ?? {};
    const options = node.oneOf ?? node.anyOf;
    if (options) {
      const compiled = options.map(go);
      const key = xs.discriminator ?? (node.oneOf ? discriminatorOf(options) : null);
      if (key) {
        const byValue = new Map();
        options.forEach((o, i) => {
          const value = o.properties?.[key]?.const;
          if (value !== undefined) byValue.set(value, compiled[i]);
        });
        return Object.assign(spec, { kind: "variants", key, byValue, valueTypes: new Set([...byValue.keys()].map(jsonType)) });
      }
      return Object.assign(spec, { kind: "union", options: compiled });
    }
    let types = node.type != null ? [].concat(node.type) : null;
    if (!types && node.const !== undefined) types = [jsonType(node.const)];
    if (!types && Array.isArray(node.enum)) types = [...new Set(node.enum.map(jsonType))];
    if (!types && node.properties) types = ["object"];
    if (!types) return Object.assign(spec, ANY);
    if (types.length === 1 && types[0] === "object") {
      if (node.properties) {
        const required = new Set(node.required ?? []);
        const props = new Map(Object.entries(node.properties).map(([k, v]) => [k, { spec: go(v), required: required.has(k) }]));
        return Object.assign(spec, { kind: "object", props, closed: !!xs.closed, known: new Set(xs.known ?? []) });
      }
      // A record (z.record): keyed by data, whatever its values are, so its
      // keys are never fields, nor any key inside it.
      if (node.propertyNames || !isEmpty(node.additionalProperties)) return Object.assign(spec, { kind: "record", value: go(node.additionalProperties) });
      return Object.assign(spec, { kind: "types", types: new Set(types) });
    }
    if (types.length === 1 && types[0] === "array") return Object.assign(spec, { kind: "array", items: go(node.items) });
    return Object.assign(spec, { kind: "types", types: new Set(types) });
  };
  return go(root);
}

// The key every option of a oneOf gives a const, as zod writes a
// discriminated union.
function discriminatorOf(options) {
  const first = options[0]?.properties;
  if (!first) return null;
  return Object.keys(first).find((k) => options.every((o) => o.properties?.[k]?.const !== undefined)) ?? null;
}

// The majors of a vendor source's schema in `dir`, newest first:
// [{ id, major, range, spec }]. None when the files aren't there.
export function loadVendor(source, dir = SCHEMAS_DIR) {
  const folder = path.join(dir, "vendor", ...source.split("/"));
  let names;
  try {
    names = fs.readdirSync(folder);
  } catch {
    return [];
  }
  return names
    .map((name) => /^v([1-9]\d*)\.json$/.exec(name))
    .filter(Boolean)
    .map((m) => {
      const json = JSON.parse(fs.readFileSync(path.join(folder, m[0]), "utf8"));
      return { id: `vendor/${source}/v${m[1]}`, major: Number(m[1]), range: json["x-xenonet"]?.vendorVersions ?? "*", spec: compile(json) };
    })
    .sort((a, b) => b.major - a.major);
}

// Every vendor source with a schema file in `dir`, as <vendor>/<product>/<source>.
export function vendorSources(dir = SCHEMAS_DIR) {
  const out = new Set();
  const walk = (rel) => {
    for (const e of fs.readdirSync(path.join(dir, "vendor", ...rel), { withFileTypes: true })) {
      if (e.isDirectory()) walk([...rel, e.name]);
      else if (/^v[1-9]\d*\.json$/.test(e.name)) out.add(rel.join("/"));
    }
  };
  try {
    walk([]);
  } catch {}
  return [...out].sort();
}

// ---------- checking ----------

// Where a walk is: a key in an object (`prop`), an element of an array
// (`item`) or an entry of a map (`entry`), linked to where it came from.
const at = (parent, key, kind = "prop") => ({ parent, key, kind });
const IDENT = /^[A-Za-z_$][\w$-]*$/;

// Where, as a drift record says it: message.content[].citations,
// changes.*.type, ["https://api.openai.com/auth"].chatgpt_plan_type. Null
// for the record itself.
export function pathOf(where) {
  if (!where) return null;
  const parts = [];
  for (let w = where; w; w = w.parent) parts.push(w);
  let s = "";
  for (const p of parts.reverse()) {
    if (p.kind === "item") s += "[]";
    else if (p.kind === "entry") s += s ? ".*" : "*";
    else s += IDENT.test(p.key) ? (s ? `.${p.key}` : p.key) : `[${JSON.stringify(p.key)}]`;
  }
  return s;
}

// Whether a place is inside an object keyed by data: a record's entry, or a
// key that reads as data.
const inData = (where) => {
  for (let w = where; w; w = w.parent) if (w.kind === "entry") return true;
  return false;
};

// The keys to it in this record, for strip().
const keysOf = (where) => {
  const keys = [];
  for (let w = where; w; w = w.parent) keys.push(w.key);
  return keys.reverse();
};

// The elements of an array worth a look: the first 16, then every 8th, and
// the last.
function* sampleOf(length) {
  for (let i = 0; i < length; i += i < 16 ? 1 : 8) yield i;
  if (length > 16 && (length - 1 - 16) % 8 !== 0) yield length - 1;
}
const MAX_ENTRIES = 32;

const fits = (types, v) => {
  const t = jsonType(v);
  return types.has(t) || (t === "number" && types.has("integer") && Number.isInteger(v));
};

// Whether `spec` takes a value of this sort at all (without looking inside).
export function accepts(spec, v) {
  switch (spec.kind) {
    case "any":
      return true;
    case "types":
      return fits(spec.types, v);
    case "object":
    case "record":
    case "variants":
      return jsonType(v) === "object";
    case "array":
      return Array.isArray(v);
    case "union":
      return spec.options.some((o) => accepts(o, v));
    default:
      return true;
  }
}

// Walks `value` against `spec`: { findings, strip, drop }. A finding is
// { kind, path, value } (new-field, type-change: the value found, for its
// shape), { kind: "new-variant", path, value } (the discriminator's) or
// { kind: "missing-field", path }; a type change also has `keys` (to strip
// it) and `segments` and `spec` (to look for it again cheaply, see
// recheck()). Each says the variant it's in: `variant`, the values of the
// discriminators the walk went through to get there, outermost first
// (["event_msg", "item_completed", "UserMessage"]), or null outside any.
// Those values are always ones the schema lists. A new field or type change
// inside an object keyed by data (a record's entry, or a key that reads as
// data) says `data: true`: its value's shape keeps no key's name. `drop`
// says nothing can be read: the record isn't an object.
export function inspect(spec, value) {
  const findings = [];
  const out = { findings, strip: [], drop: false };
  const changed = (where, v, s, variant) => {
    findings.push({ kind: "type-change", path: pathOf(where), variant, value: v, data: inData(where), keys: keysOf(where), segments: segmentsOf(where), spec: s });
    if (where) out.strip.push(keysOf(where));
    else out.drop = true;
  };
  // `variant` is null until the walk goes into a variant, then a list.
  const walk = (s, v, where, variant) => {
    switch (s.kind) {
      case "any":
        return;
      case "types":
        if (!fits(s.types, v)) changed(where, v, s, variant);
        return;
      case "array":
        if (!Array.isArray(v)) return changed(where, v, s, variant);
        for (const i of sampleOf(v.length)) walk(s.items, v[i], at(where, i, "item"), variant);
        return;
      case "record": {
        if (jsonType(v) !== "object") return changed(where, v, s, variant);
        let n = 0;
        for (const k of Object.keys(v)) {
          if (n++ >= MAX_ENTRIES) break;
          if (v[k] !== undefined) walk(s.value, v[k], at(where, k, "entry"), variant);
        }
        return;
      }
      case "object": {
        if (jsonType(v) !== "object") return changed(where, v, s, variant);
        for (const [k, p] of s.props) {
          const x = v[k];
          if (x === undefined) {
            if (p.required) findings.push({ kind: "missing-field", path: pathOf(at(where, k)), variant });
          } else walk(p.spec, x, at(where, k), variant);
        }
        // A key that reads as data rather than a field's name is never kept:
        // it shows as *.
        if (s.closed)
          for (const k of Object.keys(v)) {
            if (s.props.has(k) || s.known.has(k) || v[k] === undefined) continue;
            const here = isField(k) ? at(where, k) : at(where, k, "entry");
            findings.push({ kind: "new-field", path: pathOf(here), variant, value: v[k], data: inData(here) });
          }
        return;
      }
      case "variants": {
        if (jsonType(v) !== "object") return changed(where, v, s, variant);
        const d = v[s.key];
        const here = at(where, s.key);
        if (d === undefined) return void findings.push({ kind: "missing-field", path: pathOf(here), variant });
        const option = s.byValue.get(d);
        if (option) return walk(option, v, where, variant ? [...variant, d] : [d]);
        // A value of the sort the schema lists is a new one; another sort is
        // a type change. Either is in the variant around it, not one of its own.
        if (s.valueTypes.has(jsonType(d))) findings.push({ kind: "new-variant", path: pathOf(here), variant, value: d });
        else changed(here, d, { kind: "types", types: s.valueTypes }, variant);
        return;
      }
      case "union": {
        const option = s.options.find((o) => accepts(o, v));
        if (!option) return changed(where, v, s, variant);
        return walk(option, v, where, variant);
      }
    }
  };
  walk(spec, value, null, null);
  return out;
}

// The steps to a place, without the keys of this record: what recheck()
// looks for in the next ones.
const segmentsOf = (where) => {
  const segs = [];
  for (let w = where; w; w = w.parent) segs.push(w.kind === "prop" ? w.key : w.kind === "item" ? "[]" : "*");
  return segs.reverse();
};

// Looks at each place `segments` lead to in `value` (every element for
// [], every entry for *), and gives back the keys to each that `spec`
// doesn't take: a type change found before, looked for again in a record
// that isn't checked whole.
export function recheck(value, segments, spec) {
  const bad = [];
  const go = (v, i, keys) => {
    if (i === segments.length) {
      if (!accepts(spec, v)) bad.push(keys);
      return;
    }
    const seg = segments[i];
    if (seg === "[]") {
      if (Array.isArray(v)) v.forEach((x, j) => x !== undefined && go(x, i + 1, [...keys, j]));
    } else if (seg === "*") {
      if (jsonType(v) === "object") for (const k of Object.keys(v)) if (v[k] !== undefined) go(v[k], i + 1, [...keys, k]);
    } else if (jsonType(v) === "object" && v[seg] !== undefined) go(v[seg], i + 1, [...keys, seg]);
  };
  go(value, 0, []);
  return bad;
}

// `record` without what's at the end of each of `paths` (lists of keys): a
// key taken out of an object, an element out of an array. The record isn't
// changed; what's on the way is copied.
export function strip(record, paths) {
  if (!paths.length) return record;
  // Later elements first, so taking one out doesn't move the others.
  const order = [...paths].sort((a, b) => {
    for (let i = 0; i < Math.min(a.length, b.length); i++) {
      if (a[i] === b[i]) continue;
      return typeof a[i] === "number" && typeof b[i] === "number" ? b[i] - a[i] : String(b[i]).localeCompare(String(a[i]));
    }
    return a.length - b.length;
  });
  let out = record;
  for (const keys of order) out = without(out, keys, 0);
  return out;
}

function without(v, keys, i) {
  const k = keys[i];
  const last = i === keys.length - 1;
  if (Array.isArray(v)) {
    if (typeof k !== "number" || k >= v.length) return v;
    if (last) return v.filter((_, j) => j !== k);
    const copy = v.slice();
    copy[k] = without(v[k], keys, i + 1);
    return copy;
  }
  if (jsonType(v) === "object" && Object.hasOwn(v, k)) {
    const copy = { ...v };
    if (last) delete copy[k];
    else copy[k] = without(v[k], keys, i + 1);
    return copy;
  }
  return v;
}
