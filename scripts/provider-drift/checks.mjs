// What a provider's real output looks like to XenoNet: each record read
// through XenoNet's own lenient reader for its source (core/vendor.js), with
// a drift log of its own, so the drift found is this run's alone and never
// goes near a drift.json. Every record is checked whole, not sampled: the
// check reads a few dozen.
import { VendorReader } from "../../core/vendor.js";
import { DriftLog, ADDITIONS } from "../../core/drift.js";

// What a drift record says, without its path and variant, which the
// report shows beside it (core/drift.js's describe() says them all).
function detailOf(r) {
  switch (r.kind) {
    case "new-field":
      return `new (${r.shape})`;
    case "missing-field":
      return "missing";
    case "type-change":
      return `${r.shape} now`;
    case "new-variant":
      return `${r.shape}: a kind the schema doesn't list`;
    case "new-version":
      return `version ${r.vendorVersion} is outside the versions the schema was made for`;
    case "unparsable":
      return "lines that aren't JSON";
    default:
      return r.kind;
  }
}

// A new variant's value, from its shape ("\"token_usage_record\"").
const valueOf = (shape) => {
  try {
    const v = JSON.parse(shape);
    return typeof v === "string" || typeof v === "number" || typeof v === "boolean" ? v : undefined;
  } catch {
    return undefined;
  }
};

// A drift record as a finding: an addition (a new field or kind) or a
// change (anything else), from data.
export const findingOfDrift = (r) => ({
  source: r.source,
  kind: r.kind,
  severity: ADDITIONS.has(r.kind) ? "addition" : "change",
  path: r.path ?? null,
  variant: r.variant ?? null,
  shape: r.shape ?? null,
  ...(r.kind === "new-variant" && valueOf(r.shape) !== undefined ? { value: valueOf(r.shape) } : {}),
  vendorVersion: r.vendorVersion ?? null,
  count: r.count,
  detail: detailOf(r),
  origin: "data",
});

// Readers for a run: reader(source) gives one per source, all keeping
// drift in one log; findings() gives what they found.
export function readers() {
  const log = new DriftLog({ saveAfterMs: Infinity });
  const made = new Map();
  const get = (source) => {
    if (!made.has(source)) made.set(source, new VendorReader(source, { log }));
    return made.get(source);
  };
  return {
    reader: get,
    // The records read, by source.
    read: () => Object.fromEntries([...made].map(([s, r]) => [s, r.stats.checked])),
    findings: () => log.list().map(findingOfDrift),
  };
}

// The lines of a transcript or rollout, read as XenoNet's tailer reads them:
// each line checked against `source`'s schema, with the version the lines
// carry (`versionOf(line, current)`) and their kind (`kindOf(line)`).
// Lines that aren't JSON are drift too.
export function checkLines(reader, text, { versionOf, kindOf }) {
  let version = null;
  const lines = [];
  for (const raw of String(text).split("\n")) {
    if (!raw.trim()) continue;
    let line;
    try {
      line = JSON.parse(raw);
    } catch {
      reader.stats.records++;
      reader.unparsable(version);
      continue;
    }
    if (line && typeof line === "object" && !Array.isArray(line)) version = versionOf(line, version);
    reader.check(line, { version, kind: line && typeof line === "object" ? kindOf(line) : null });
    lines.push(line);
  }
  return { lines, version };
}

// As core/lines.js tells them apart (claudeHooks, codexHooks).
const kindPart = (v) => (typeof v === "string" ? v : typeof v);
export const CLAUDE_LINES = {
  kindOf: (o) => kindPart(o.type),
  versionOf: (o, current) => (typeof o.version === "string" ? o.version : current),
};
export const CODEX_LINES = {
  kindOf: (o) => {
    const t = kindPart(o.type);
    if (t !== "event_msg" && t !== "response_item") return t;
    const p = kindPart(o.payload?.type);
    return p === "item_completed" ? `${t}:${p}:${kindPart(o.payload.item?.type)}` : `${t}:${p}`;
  },
  versionOf: (o, current) => (o.type === "session_meta" && typeof o.payload?.cli_version === "string" ? o.payload.cli_version : current),
};

// An expectation: something the stand-in turn put there that XenoNet's
// reading should find. A failed one is a change: the data fits the schema,
// but not where XenoNet looks. One XenoNet doesn't depend on yet (`required`
// false: a call it doesn't make) is only a note.
export const expect = (source, ok, what, { required = true } = {}) => ({
  source,
  kind: "expectation",
  severity: ok ? "ok" : required ? "change" : "info",
  path: null,
  variant: null,
  detail: what,
  origin: "expectation",
  ok: !!ok,
});
