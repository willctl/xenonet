// A provider's published types against XenoNet's vendor schemas: the fields
// XenoNet reads, and for the objects its schemas close, every field. The
// types come as JSON Schema, from Codex's generator (`codex app-server
// generate-json-schema`) or made here from a TypeScript declaration file
// (the Agent SDK's sdk.d.ts), and XenoNet's are its schema files
// (schemas/vendor/**). Nothing here reads data: it compares declarations.
//
// A comparison gives findings, each { kind, severity, path, variant,
// detail }:
//
//   kind           severity   when
//   removed        change     a field XenoNet reads isn't in the types any more
//   retyped        change     a field XenoNet reads has no type in common with XenoNet's
//   now-optional   change     a field XenoNet needs may now be left out
//   added          addition   a field the types have that a closed object doesn't list
//   new-variant    addition   a kind of record the types have that XenoNet doesn't list
//   variant-gone   info       a kind XenoNet lists that the types no longer have (kept, for older versions)
//
// An addition can't break what XenoNet reads, but it is drift once data
// carries it; a change can.

// ---------- JSON Schema nodes ----------

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const jsonType = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v === "object" ? "object" : typeof v);

// The node a $ref leads to in `root` (#/definitions/X or #/$defs/X).
function resolveRef(ref, root) {
  const m = /^#\/(definitions|\$defs)\/(.+)$/.exec(ref ?? "");
  if (!m) return null;
  return root?.[m[1]]?.[m[2].replaceAll("~1", "/").replaceAll("~0", "~")] ?? null;
}

// A node with its $refs and one-element allOfs followed.
export function deref(node, root, depth = 0) {
  let n = node;
  for (let i = 0; i < 32 && isObject(n); i++) {
    if (n.$ref) {
      const next = resolveRef(n.$ref, root);
      if (!next) return {};
      n = next;
    } else if (Array.isArray(n.allOf) && n.allOf.length === 1 && !n.properties && !n.type) n = n.allOf[0];
    else break;
  }
  return isObject(n) ? n : {};
}

const options = (n) => (Array.isArray(n.oneOf) ? n.oneOf : Array.isArray(n.anyOf) ? n.anyOf : null);
const isNull = (n, root) => {
  const d = deref(n, root);
  return d.type === "null" || (Array.isArray(d.enum) && d.enum.length === 1 && d.enum[0] === null);
};

// A node without a null alternative (maybe(), Option<T>): { node, nullable }.
export function unwrap(node, root) {
  let n = deref(node, root);
  let nullable = false;
  for (let i = 0; i < 8; i++) {
    const opts = options(n);
    if (!opts) break;
    const rest = opts.filter((o) => !isNull(o, root));
    if (rest.length === opts.length || rest.length !== 1) break;
    nullable = true;
    n = deref(rest[0], root);
  }
  if (Array.isArray(n.type) && n.type.includes("null")) nullable = true;
  return { node: n, nullable };
}

// The JSON types a node allows, as a Set ("integer" counts as "number"),
// or null when it allows anything.
export function typesOf(node, root, seen = new Set()) {
  const n = deref(node, root);
  if (seen.has(n)) return null;
  seen.add(n);
  const norm = (t) => (t === "integer" ? "number" : t);
  if (n.type != null) return new Set([].concat(n.type).map(norm));
  if (n.const !== undefined) return new Set([jsonType(n.const)]);
  if (Array.isArray(n.enum)) return new Set(n.enum.map(jsonType));
  const opts = options(n);
  if (opts) {
    const out = new Set();
    for (const o of opts) {
      const t = typesOf(o, root, seen);
      if (!t) return null;
      for (const x of t) out.add(x);
    }
    return out;
  }
  if (n.properties) return new Set(["object"]);
  if (n.items) return new Set(["array"]);
  return null;
}

// The value a variant's discriminator has: its const, or an enum of one.
const literalOf = (n, root) => {
  const d = deref(n, root);
  if (d.const !== undefined) return d.const;
  if (Array.isArray(d.enum) && d.enum.length === 1) return d.enum[0];
  return undefined;
};

// What a node is, for comparing:
//   { kind: "object", fields: Map(name → { node, required }), closed, known }
//   { kind: "variants", key, options: Map(value → object shape) }
//   { kind: "other" }
export function shapeOf(node, root) {
  const { node: n } = unwrap(node, root);
  const opts = options(n);
  if (opts) {
    const objects = opts.map((o) => unwrap(o, root).node).filter((o) => isObject(o.properties));
    if (objects.length && objects.length === opts.filter((o) => !isNull(o, root)).length) {
      const declared = n["x-xenonet"]?.discriminator;
      const keys = declared ? [declared] : Object.keys(objects[0].properties).filter((k) => objects.every((o) => literalOf(o.properties[k], root) !== undefined));
      if (keys.length) {
        const key = keys.includes("type") ? "type" : keys[0];
        return { kind: "variants", key, options: new Map(objects.map((o) => [literalOf(o.properties[key], root), objectShape(o)])) };
      }
      // A union of objects without a discriminator: every field of any.
      return flatten(objects.map(objectShape));
    }
    return { kind: "other" };
  }
  if (isObject(n.properties)) return objectShape(n);
  return { kind: "other" };
}

function objectShape(n) {
  const required = new Set(n.required ?? []);
  const xs = n["x-xenonet"] ?? {};
  return {
    kind: "object",
    fields: new Map(Object.entries(n.properties ?? {}).map(([k, v]) => [k, { node: v, required: required.has(k) }])),
    closed: !!xs.closed,
    known: new Set(xs.known ?? []),
  };
}

// Several objects as one: every field any has, required only where all
// require it.
function flatten(objects) {
  const fields = new Map();
  for (const o of objects)
    for (const [k, f] of o.fields) {
      const had = fields.get(k);
      fields.set(k, { node: had?.node ?? f.node, required: (had ? had.required : true) && f.required, count: (had?.count ?? 0) + 1 });
    }
  for (const f of fields.values()) {
    if (f.count < objects.length) f.required = false;
    delete f.count;
  }
  return { kind: "object", fields, closed: false, known: new Set() };
}

// The node at `segments` under `node`: a field's name goes into an
// object, "[]" into an array's items, and "<value>" into the variant whose
// discriminator has that value. Null when there's nothing there.
export function nodeAt(node, segments, root) {
  let n = node;
  for (const seg of segments) {
    const { node: u } = unwrap(n, root);
    const m = /^<(.+)>$/.exec(seg);
    if (m) {
      const opts = options(u);
      if (!opts) return null;
      const shape = shapeOf(u, root);
      if (shape.kind !== "variants") return null;
      const found = opts.map((o) => unwrap(o, root).node).find((o) => String(literalOf(o.properties?.[shape.key], root)) === m[1]);
      if (!found) return null;
      n = found;
    } else if (seg === "[]") {
      if (!u.items) return null;
      n = u.items;
    } else {
      const next = u.properties?.[seg];
      if (next === undefined) return null;
      n = next;
    }
  }
  return n;
}

// ---------- comparing ----------

const IDENT = /^[A-Za-z_$][\w$-]*$/;
const join = (base, key) => (IDENT.test(key) ? (base ? `${base}.${key}` : key) : `${base}[${JSON.stringify(key)}]`);

// A path as findings say it, from the segments a pair names: a variant's
// selection says nothing in the path (it goes in `variant`).
export function pathOf(segments) {
  let s = "";
  for (const seg of segments) {
    if (/^<.+>$/.test(seg)) continue;
    s = seg === "[]" ? `${s}[]` : join(s, seg);
  }
  return s;
}
export const variantOf = (segments) => segments.filter((s) => /^<.+>$/.test(s)).map((s) => s.slice(1, -1));

const describeTypes = (t) => (t ? [...t].sort().join("|") : "anything");
const withoutNull = (t) => (t ? new Set([...t].filter((x) => x !== "null")) : null);

// Compares XenoNet's node with the provider's: findings, as above. `opts`:
//   path, variant   where the pair is, for the findings
//   legacy          fields XenoNet reads only from older versions, which the
//                   types needn't have
//   untyped         fields the provider sends but its types don't declare
//   shared          fields XenoNet reads from every kind of record in a
//                   union (a tool call's name, input or arguments), which
//                   each kind may or may not have: gone only when no kind
//                   has them
//   ignoreVariants  kinds the types list that data never carries (a
//                   catch-all such as Codex's "other")
export function compare(xenonetNode, xenonetRoot, providerNode, providerRoot, opts = {}) {
  const { path = "", variant = [], legacy = [], untyped = [], shared = [], ignoreVariants = [] } = opts;
  const findings = [];
  const add = (kind, severity, at, v, detail, extra = {}) => findings.push({ kind, severity, path: at || null, variant: v.length ? v : null, detail, ...extra });
  const skip = new Set([...legacy, ...untyped]);
  // Every field any kind of record has, for `shared`.
  let anywhere = null;

  const objects = (s, p, at, v) => {
    for (const [name, f] of s.fields) {
      if (skip.has(name)) continue;
      const here = join(at, name);
      const theirs = p.fields.get(name);
      if (!theirs) {
        if (!shared.includes(name) || !anywhere?.has(name)) add("removed", "change", here, v, "XenoNet reads it, and the types no longer declare it");
        continue;
      }
      const mine = withoutNull(typesOf(f.node, xenonetRoot));
      const types = withoutNull(typesOf(theirs.node, providerRoot));
      if (mine && types && mine.size && types.size && ![...mine].some((t) => types.has(t))) add("retyped", "change", here, v, `XenoNet reads it as ${describeTypes(mine)}; the types say ${describeTypes(types)}`);
      if (f.required && !theirs.required && !unwrap(f.node, xenonetRoot).nullable) add("now-optional", "change", here, v, "XenoNet needs it; the types say it may be left out");
    }
    if (s.closed)
      for (const name of p.fields.keys())
        if (!s.fields.has(name) && !s.known.has(name) && !untyped.includes(name)) add("added", "addition", join(at, name), v, "the types declare it, and XenoNet's closed object doesn't list it");
  };

  let mine = shapeOf(xenonetNode, xenonetRoot);
  let theirs = shapeOf(providerNode, providerRoot);
  // A provider object whose discriminator is a union of literals (the
  // SDK's SessionMessage: type: 'user' | 'assistant' | 'system') is that
  // many variants of one shape.
  if (mine.kind === "variants" && theirs.kind === "object") {
    const d = theirs.fields.get(mine.key);
    const values = d && deref(d.node, providerRoot).enum;
    if (Array.isArray(values)) theirs = { kind: "variants", key: mine.key, options: new Map(values.map((v) => [v, theirs])) };
  }
  if (mine.kind === "object" && theirs.kind === "variants") theirs = flatten([...theirs.options.values()]);

  if (mine.kind === "object" && theirs.kind === "object") objects(mine, theirs, path, variant);
  else if (mine.kind === "variants" && theirs.kind === "variants") {
    if (mine.key !== theirs.key) add("retyped", "change", join(path, mine.key), variant, `XenoNet tells these records apart by ${mine.key}; the types by ${theirs.key}`);
    else {
      anywhere = new Set([...theirs.options.values()].flatMap((o) => [...o.fields.keys()]));
      for (const value of theirs.options.keys())
        if (!mine.options.has(value) && !ignoreVariants.includes(value)) add("new-variant", "addition", join(path, mine.key), variant, `${JSON.stringify(value)}: the types declare this kind, and XenoNet doesn't list it`, { value });
      for (const [value, option] of mine.options) {
        const other = theirs.options.get(value);
        // A kind XenoNet passes over (skipped) declares nothing but its key.
        const passedOver = option.fields.size <= 1 && !option.closed;
        if (!other) {
          if (!passedOver) add("variant-gone", "info", join(path, mine.key), variant, `${JSON.stringify(value)} isn't in the types any more; XenoNet still reads it from older versions`, { value });
          continue;
        }
        if (!passedOver) objects(option, other, path, [...variant, value]);
      }
    }
  } else if (mine.kind !== "other" && theirs.kind === "other") add("retyped", "change", path, variant, `XenoNet reads it as ${mine.kind === "variants" ? "kinds of record" : "an object"}; the types say otherwise`);
  return findings;
}

// Every pair a provider's mapping names, compared: the findings, each
// with the pair's source, and the pairs that couldn't be found (a type
// gone from the provider's output, or a path gone from XenoNet's schema),
// which are changes too.
export function compareAll(pairs, { xenonet, provider, source }) {
  const findings = [];
  for (const pair of pairs) {
    const xenonetRoot = xenonet(pair);
    const providerRoot = provider(pair);
    const where = { path: pathOf(pair.xenonet), variant: variantOf(pair.xenonet) };
    const mine = xenonetRoot && nodeAt(xenonetRoot, pair.xenonet, xenonetRoot);
    const theirs = providerRoot && (providerRoot.definitions?.[pair.provider] ?? providerRoot.$defs?.[pair.provider] ?? (providerRoot.title === pair.provider ? providerRoot : null));
    if (!mine) {
      findings.push({ source, kind: "unmapped", severity: "info", path: where.path || null, variant: where.variant.length ? where.variant : null, detail: `XenoNet's schema has nothing at ${where.path || "its root"} to compare with ${pair.provider}` });
      continue;
    }
    if (!theirs) {
      findings.push({ source, kind: "removed", severity: "change", path: where.path || null, variant: where.variant.length ? where.variant : null, detail: `the types no longer declare ${pair.provider}, which XenoNet reads at ${where.path || "the root"}` });
      continue;
    }
    for (const f of compare(mine, xenonetRoot, theirs, providerRoot, { ...pair, ...where })) findings.push({ source, type: pair.provider, ...f });
  }
  return findings;
}

// ---------- methods and exports ----------

// The JSON-RPC methods a client-request schema (Codex's ClientRequest.json)
// declares, with the definition each one's params refer to.
export function methodsOf(clientRequest) {
  const out = new Map();
  for (const o of options(clientRequest ?? {}) ?? []) {
    const method = literalOf(o.properties?.method, clientRequest);
    if (typeof method !== "string") continue;
    const ref = /^#\/(?:definitions|\$defs)\/(.+)$/.exec(o.properties?.params?.$ref ?? "")?.[1] ?? null;
    out.set(method, ref);
  }
  return out;
}

// The app-server methods XenoNet's code calls: request("x/y", …) (any
// method with a slash; XenoNet's own service calls have none) and
// initialize, from the text of its source files.
export function codexMethodsIn(sources) {
  const out = new Set();
  for (const text of sources) {
    for (const m of String(text).matchAll(/(?:request|#call)\(\s*["'`]([A-Za-z][\w]*(?:\/[A-Za-z][\w]*)+)["'`]/g)) out.add(m[1]);
    if (/#call\(\s*["'`]initialize["'`]/.test(text)) out.add("initialize");
  }
  return [...out].sort();
}

// The names a module's source imports from `pkg`.
export function importsFrom(sources, pkg) {
  const out = new Set();
  const escaped = pkg.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
  const re = new RegExp(`import\\s*\\{([^}]*)\\}\\s*from\\s*["']${escaped}["']`, "g");
  for (const text of sources)
    for (const m of String(text).matchAll(re))
      for (const part of m[1].split(",")) {
        const name = part.trim().split(/\s+as\s+/)[0].replace(/^type\s+/, "").trim();
        if (name) out.add(name);
      }
  return [...out].sort();
}

// ---------- TypeScript declarations ----------

// Comments out, strings kept.
function stripComments(text) {
  let out = "";
  for (let i = 0; i < text.length; ) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? text.length : end + 2;
    } else if (c === "/" && text[i + 1] === "/") {
      const end = text.indexOf("\n", i);
      i = end < 0 ? text.length : end;
    } else if (c === "'" || c === '"' || c === "`") {
      let j = i + 1;
      while (j < text.length && text[j] !== c) j += text[j] === "\\" ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j + 1;
    } else {
      out += c;
      i++;
    }
  }
  return out;
}

// `text` split at `sep` where it isn't inside brackets or a string.
function splitTop(text, sep) {
  const parts = [];
  let depth = 0;
  let quote = null;
  let start = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quote) {
      if (c === "\\") i++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === "'" || c === '"' || c === "`") quote = c;
    else if ("({[<".includes(c)) depth++;
    else if (")}]>".includes(c) && !(c === ">" && text[i - 1] === "=")) depth--;
    else if (c === sep && depth === 0) {
      parts.push(text.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(text.slice(start));
  return parts.map((p) => p.trim()).filter(Boolean);
}

// The text of each `type Name = …;` and `interface Name {…}` declared.
export function declarationsOf(dts) {
  const text = stripComments(dts);
  const out = new Map();
  const re = /(?:^|\n)\s*(?:export\s+)?(?:declare\s+)?(type|interface)\s+([A-Za-z_$][\w$]*)\s*(?:<[^=>{]*>)?\s*(=|extends[^{]*\{|\{)/g;
  let m;
  while ((m = re.exec(text))) {
    const [, kind, name, opener] = m;
    let i = re.lastIndex;
    if (kind === "interface" || opener.endsWith("{")) i--;
    // Read to the end of the declaration: the `;` (or `}` of an interface)
    // outside brackets.
    let depth = 0;
    let j = i;
    let quote = null;
    for (; j < text.length; j++) {
      const c = text[j];
      if (quote) {
        if (c === "\\") j++;
        else if (c === quote) quote = null;
        continue;
      }
      if (c === "'" || c === '"' || c === "`") quote = c;
      else if ("({[".includes(c)) depth++;
      else if (")}]".includes(c)) {
        depth--;
        if (depth === 0 && kind === "interface") break;
      } else if (c === ";" && depth === 0 && kind === "type") break;
    }
    const body = text.slice(i, kind === "interface" ? j + 1 : j).trim();
    if (!out.has(name)) out.set(name, body);
  }
  return out;
}

// A TypeScript type as a JSON Schema node, as far as the comparison needs:
// primitives, literals, unions, arrays, object literals (one level of
// fields) and the declarations it names. What it can't tell is {} (anything).
export function tsToSchema(type, decls, seen = new Set()) {
  const t = String(type ?? "").trim().replace(/^\|/, "").trim();
  const union = splitTop(t, "|");
  if (union.length > 1) {
    const parts = union.map((u) => tsToSchema(u, decls, seen));
    const literals = parts.every((p) => p.const !== undefined) ? parts.map((p) => p.const) : null;
    return literals ? { enum: literals } : { anyOf: parts };
  }
  if (/^\(.*\)$/s.test(t)) return tsToSchema(t.slice(1, -1), decls, seen);
  if (/^'([^']*)'$|^"([^"]*)"$/.test(t)) return { const: t.slice(1, -1) };
  if (/^-?\d+(\.\d+)?$/.test(t)) return { const: Number(t) };
  if (t === "true" || t === "false") return { const: t === "true" };
  if (t === "string" || t.startsWith("`")) return { type: "string" };
  if (t === "number" || t === "bigint") return { type: "number" };
  if (t === "boolean") return { type: "boolean" };
  if (t === "null") return { type: "null" };
  if (t === "undefined" || t === "void") return { type: "null" };
  if (/\[\]$/.test(t) || /^(Readonly)?Array</.test(t) || /^readonly\s/.test(t)) return { type: "array", items: {} };
  if (t.startsWith("{")) return objectSchema(t, decls, seen);
  if (/^(Record|Partial|Required|Pick|Omit)</.test(t)) return { type: "object" };
  const name = /^([A-Za-z_$][\w$.]*)/.exec(t)?.[1];
  if (name && decls.has(name) && !seen.has(name)) return tsToSchema(decls.get(name), decls, new Set([...seen, name]));
  return {};
}

function objectSchema(t, decls, seen) {
  const inner = t.replace(/^\{/, "").replace(/\}$/, "");
  const properties = {};
  const required = [];
  for (const member of [...splitTop(inner, ";")].flatMap((m) => splitTop(m, ","))) {
    const m = /^(?:readonly\s+)?(['"]?)([A-Za-z_$][\w$-]*)\1(\?)?\s*:\s*([\s\S]+)$/.exec(member);
    if (!m) continue;
    const [, , name, optional, type] = m;
    properties[name] = tsToSchema(type, decls, seen);
    if (!optional) required.push(name);
  }
  return { type: "object", properties, required };
}

// The named declarations of a .d.ts as one JSON Schema document, so they
// compare the way Codex's do: { definitions: { Name: {...} } }.
export function schemaFromDts(dts, names) {
  const decls = declarationsOf(dts);
  return { definitions: Object.fromEntries(names.filter((n) => decls.has(n)).map((n) => [n, tsToSchema(decls.get(n), decls, new Set([n]))])) };
}

// The functions a .d.ts exports: `export declare function name(`.
export function functionsOf(dts) {
  return [...new Set([...stripComments(dts).matchAll(/export\s+(?:declare\s+)?function\s+([A-Za-z_$][\w$]*)/g)].map((m) => m[1]))].sort();
}
