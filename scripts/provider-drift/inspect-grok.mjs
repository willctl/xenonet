// Grok Build's files, checked against its source. Nothing here runs grok
// (its files are written only by a signed-in grok with a model behind it),
// so the Rust structs it writes them from are read instead:
// session/persistence.rs's Summary, session/usage_file.rs's usage ledger and
// agent/subagent/mod.rs's SubagentMeta, in xai-org/grok-build (a source
// mirror with no tags, so `main`). Each field XenoNet's schema declares
// must still be there.
import { githubFile } from "./inspect-native.mjs";
import { stripComments } from "./types.mjs";
import { steps, xenonetSchema } from "./common.mjs";

export const GROK_REPO = "xai-org/grok-build";
const SHELL = "crates/codegen/xai-grok-shell/src";
export const GROK_FILES = {
  persistence: `${SHELL}/session/persistence.rs`,
  info: "crates/codegen/xai-grok-shared/src/session/info.rs",
  usage: `${SHELL}/session/usage_file.rs`,
  subagent: `${SHELL}/agent/subagent/mod.rs`,
};

const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

// The serde attributes among a list of attribute lines, joined.
const serdeOf = (attrs) => attrs.filter((a) => a.startsWith("#[serde")).join(" ");

// The fields serde writes for `struct <name>`, as wire name → { optional },
// and the types it flattens in. Handles rename_all = "camelCase", rename,
// alias, skip and flatten, which is all Grok's structs use.
export function rustStruct(source, name) {
  const text = stripComments(String(source));
  const at = new RegExp(`\\bstruct\\s+${name}\\b[^{;]*\\{`).exec(text);
  if (!at) return null;
  const above = text.slice(0, at.index).split("\n");
  const structAttrs = [];
  for (let i = above.length - 1; i >= 0; i--) {
    const l = above[i].trim();
    if (l.startsWith("#[")) structAttrs.unshift(l);
    else if (l && !l.startsWith("pub")) break;
  }
  const renameAll = /rename_all\s*=\s*"(\w+)"/.exec(serdeOf(structAttrs))?.[1] ?? null;
  let depth = 1;
  let end = at.index + at[0].length;
  const start = end;
  for (; end < text.length && depth > 0; end++) depth += text[end] === "{" ? 1 : text[end] === "}" ? -1 : 0;
  const fields = new Map();
  const flatten = [];
  let attrs = [];
  for (const raw of text.slice(start, end - 1).split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith("#[")) {
      attrs.push(line);
      continue;
    }
    const m = /^(?:pub(?:\([^)]*\))?\s+)?([a-z_][a-z0-9_]*)\s*:\s*(.+?),?$/.exec(line);
    if (!m) continue;
    const serde = serdeOf(attrs);
    attrs = [];
    if (/[(,\s]skip[,)\s]/.test(serde)) continue;
    if (/\bflatten\b/.test(serde)) {
      flatten.push(m[2].replace(/[<>].*$/, "").trim());
      continue;
    }
    const wire = /\brename\s*=\s*"([^"]+)"/.exec(serde)?.[1] ?? (renameAll === "camelCase" ? camel(m[1]) : m[1]);
    const optional = /^Option</.test(m[2]) || /\bdefault\b/.test(serde);
    fields.set(wire, { optional });
    for (const a of serde.matchAll(/\balias\s*=\s*"([^"]+)"/g)) fields.set(a[1], { optional });
  }
  return { fields, flatten };
}

// The object a schema node describes, through the anyOf a nullable makes.
function objectOf(node) {
  if (!node) return null;
  if (node.properties) return node;
  for (const alt of node.anyOf ?? []) {
    const o = objectOf(alt);
    if (o) return o;
  }
  return objectOf(node.items);
}

// What XenoNet's schema declares at `node` that `struct` doesn't write.
// `also` are structs flattened into it; `camelToo` accepts a field's
// camelCase name too (a subagent's cloud copy).
function missing(source, node, at, struct, { also = [], camelToo = false } = {}) {
  const obj = objectOf(node);
  if (!obj || !struct) return [];
  const names = new Set();
  for (const s of [struct, ...also]) {
    for (const k of s.fields.keys()) {
      names.add(k);
      if (camelToo) names.add(camel(k));
    }
  }
  const required = new Set(obj.required ?? []);
  return Object.keys(obj.properties)
    .filter((p) => !names.has(p))
    .map((p) => ({
      source,
      kind: "removed",
      severity: required.has(p) ? "change" : "info",
      path: `${at}${at ? "." : ""}${p}`,
      variant: null,
      detail: required.has(p) ? "XenoNet needs this field, and Grok Build's source doesn't write it" : "XenoNet reads this field when it is there; Grok Build's source doesn't write it, so only older versions do",
      origin: "types",
    }));
}

// Every finding, given the four source files.
export function grokFindings(files, schemaOf = xenonetSchema) {
  const out = [];
  const need = (what, struct) => {
    if (!struct) out.push({ source: "spacexai/grok-build", kind: "removed", severity: "change", path: what, variant: null, detail: `Grok Build's source has no struct ${what} any more`, origin: "types" });
    return struct;
  };
  const summary = need("Summary", rustStruct(files.persistence, "Summary"));
  const info = need("Info", rustStruct(files.info, "Info"));
  const usageFile = need("SessionUsageFile", rustStruct(files.usage, "SessionUsageFile"));
  const usage = need("UsageSummary", rustStruct(files.usage, "UsageSummary"));
  const turn = need("TurnUsage", rustStruct(files.usage, "TurnUsage"));
  const meta = need("SubagentMeta", rustStruct(files.subagent, "SubagentMeta"));

  const sum = "spacexai/grok-build/session-summary";
  let s = schemaOf(sum);
  out.push(...missing(sum, s, "", summary), ...missing(sum, s?.properties?.info, "info", info));
  const use = "spacexai/grok-build/usage";
  s = schemaOf(use);
  out.push(...missing(use, s, "", usageFile), ...missing(use, s?.properties?.session, "session", usage), ...missing(use, s?.properties?.turns, "turns[]", turn, { also: usage ? [usage] : [] }));
  const sub = "spacexai/grok-build/subagent-meta";
  out.push(...missing(sub, schemaOf(sub), "", meta, { camelToo: true }));
  return out;
}

export async function inspectGrok() {
  const s = steps();
  const files = {};
  for (const [key, file] of Object.entries(GROK_FILES)) {
    files[key] = await s.run(`read ${file} from ${GROK_REPO}`, () => githubFile(GROK_REPO, file, "main"), (t) => `${t.length} bytes`);
  }
  const ok = Object.values(files).every((t) => t != null);
  return { steps: s.list, findings: ok ? grokFindings(files) : [], read: {}, info: ok ? { typesFrom: `${GROK_REPO} main` } : {} };
}
