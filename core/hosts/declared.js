// What a host sends, kept as XenoNet's schemas declare it (docs/design/
// hosts.md, "Security"). Every object in the schemas is loose, so a value
// that passes its check still carries keys the schema never names. From a
// host, such a key is a field the page may read as one of its own and
// write into its markup without escaping it, since nothing of XenoNet's ever
// puts that field there.
//
// declared() copies a value that has passed its schema with the declared
// fields alone, at every level. A record keeps its keys, which its schema
// declares as a kind of key; an object where the schema takes anything
// keeps none of its own. The value must have passed the schema first: a
// field of the wrong type is the check's to refuse.
const isPlain = (v) => v != null && typeof v === "object" && !Array.isArray(v);

// The fields an object schema declares, an intersection's from both sides
// (as check.js reads them), worked out once per schema.
const FIELDS = new WeakMap();
function fieldsOf(schema) {
  let fields = FIELDS.get(schema);
  if (fields) return fields;
  const def = schema._zod.def;
  fields = def.type === "object" ? Object.entries(def.shape) : def.type === "intersection" ? [...new Map([...fieldsOf(def.left), ...fieldsOf(def.right)])] : [];
  FIELDS.set(schema, fields);
  return fields;
}

export function declared(schema, value) {
  const def = schema._zod.def;
  switch (def.type) {
    case "optional":
    case "nullable":
    case "default":
    case "prefault":
    case "nonoptional":
    case "readonly":
    case "catch":
      return value == null ? value : declared(def.innerType, value);
    case "pipe":
      return declared(def.in, value);
    case "lazy":
      return declared(def.getter(), value);
    case "array":
      return Array.isArray(value) ? value.map((v) => declared(def.element, v)) : value;
    case "tuple":
      return Array.isArray(value) ? value.slice(0, def.rest ? value.length : def.items.length).map((v, i) => declared(def.items[i] ?? def.rest, v)) : value;
    case "record":
      return isPlain(value) ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, declared(def.valueType, v)])) : value;
    case "union": {
      const option = def.options.find((o) => o.safeParse(value).success);
      return option ? declared(option, value) : value;
    }
    case "object":
    case "intersection": {
      if (!isPlain(value)) return value;
      const out = {};
      for (const [k, field] of fieldsOf(schema)) if (Object.hasOwn(value, k) && value[k] !== undefined) out[k] = declared(field, value[k]);
      return out;
    }
    case "unknown":
    case "any":
      return Array.isArray(value) ? value.map((v) => declared(schema, v)) : isPlain(value) ? {} : value;
    default:
      return value;
  }
}
