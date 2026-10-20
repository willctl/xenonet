// core/hosts/declared.js in Rust (crates/xenonet-hosts/src/declared.rs):
// what a host sends, kept as XenoNet's schemas declare it. The schemas are
// zod's: each one's shape is read off it here once, as a table the Rust
// walks, and a union's options are tried with zod's safeParse.
import { native } from "../../index.js";

const WRAPS = new Set(["optional", "nullable", "default", "prefault", "nonoptional", "readonly", "catch"]);

// The fields an object schema declares, an intersection's from both sides.
function fieldsOf(schema) {
  const def = schema._zod.def;
  return def.type === "object" ? Object.entries(def.shape) : def.type === "intersection" ? [...new Map([...fieldsOf(def.left), ...fieldsOf(def.right)])] : [];
}

// A schema's table: its parts by index, the schema first.
function tableOf(root) {
  const index = new Map();
  const shapes = [];
  const schemas = [];
  const at = (schema) => {
    if (index.has(schema)) return index.get(schema);
    const i = shapes.length;
    index.set(schema, i);
    shapes.push(null);
    schemas.push(schema);
    const def = schema._zod.def;
    let shape;
    if (WRAPS.has(def.type)) shape = ["wrap", at(def.innerType)];
    else if (def.type === "pipe") shape = ["same", at(def.in)];
    else if (def.type === "lazy") shape = ["same", at(def.getter())];
    else if (def.type === "array") shape = ["array", at(def.element)];
    else if (def.type === "tuple") shape = ["tuple", def.items.map(at), def.rest ? at(def.rest) : null];
    else if (def.type === "record") shape = ["record", at(def.valueType)];
    else if (def.type === "union") shape = ["union", def.options.map(at)];
    else if (def.type === "object" || def.type === "intersection") shape = ["object", fieldsOf(schema).map(([k, field]) => [k, at(field)])];
    else if (def.type === "unknown" || def.type === "any") shape = ["any"];
    else shape = ["leaf"];
    shapes[i] = shape;
    return i;
  };
  at(root);
  return { shapes: new native.HostsShapes(shapes), schemas };
}

const TABLES = new WeakMap();

export function declared(schema, value) {
  let t = TABLES.get(schema);
  if (!t) TABLES.set(schema, (t = tableOf(schema)));
  return t.shapes.declared(0, value, (i, v) => t.schemas[i].safeParse(v).success);
}
