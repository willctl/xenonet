// Drift: where the vendor data XenoNet reads differs from XenoNet's schemas for
// it (contracts, section 1). The lenient readers (vendor.js) find it; this
// keeps it, as drift/record/v1 records, in drift.json in the cache folder
// (a drift/log/v1); npm run contracts reports it and the doctor's
// contracts.drift check warns about what's new since you last looked.
//
// A record holds shapes, never values: where in the record (`path`), and
// the types found there (`shape`): object keys and their value types, array
// element types, at most four levels deep. The only values ever kept are a
// declared discriminator's (a line's `type`) and vendor versions. Keys that
// read as data rather than field names (paths, ids, emails, numbers,
// tokens) make their object a record<…>, so they're never kept either, and
// nothing inside an object a schema declares keyed by data (a record: gh's
// hosts and logins, a change's paths) keeps a key's name.
//
//   kind           path                        shape
//   new-field      message.usage.ponder_ms     number             (a key a closed object doesn't list)
//   missing-field  payload.cli_version         null               (a required field that isn't there)
//   type-change    message.content             object{…}          (a field of another type)
//   new-variant    type                        "attachment"       (a discriminator's value the schema doesn't list)
//   new-version    null                        null               (a vendor version outside the schema's range)
//   unparsable     null                        null               (a line that isn't JSON)
//
// A record inside a kind of record says which (`variant`): the values of
// the discriminators around it, outermost first, such as ["assistant",
// "text"] for a text block in a reply; only ever values the schema lists.
// The same path can mean another field in another kind of record
// (payload.message in a user_message or an agent_message), so without it a
// record couldn't say where it is. It's left out where there's none.
//
// The same drift seen again is the same record, counted: a new or missing
// field by its path and variant, a type change by its path, variant and the
// type found, a new variant by its value, a new version by the version,
// lines that aren't JSON by source. `vendorVersion` is the version it was
// first seen in. Counts are of records the readers checked, which in the
// tailer is a sample (see vendor.js).
import fs from "node:fs";
import path from "node:path";
import { systemClock } from "./blocks/clock.js";

export const DRIFT_MAX = 500;
export const DRIFT_FILE = "drift.json";
// drift.json's own version; its records each say what they are.
const FILE_VERSION = 1;
const SAVE_AFTER_MS = 5000;

export const KINDS = ["new-field", "missing-field", "type-change", "new-variant", "new-version", "unparsable"];
// Kinds that can't break what XenoNet reads: something was added.
export const ADDITIONS = new Set(["new-field", "new-variant"]);

// What each source is, in words.
const SOURCES = {
  "anthropic/claude-code/transcript-line": "Claude Code's transcripts",
  "anthropic/claude-code/config": "Claude Code's ~/.claude.json",
  "anthropic/claude-desktop/session": "Claude desktop's session files",
  "anthropic/claude-desktop/plan-usage": "Claude desktop's plan usage",
  "anthropic/agent-sdk/session-listing": "the Agent SDK's session list",
  "anthropic/agent-sdk/session-message": "the Agent SDK's session messages",
  "anthropic/oauth/usage": "Anthropic's plan usage answer",
  "openai/codex/rollout-line": "Codex's rollouts",
  "openai/codex/app-server": "Codex's app-server",
  "openai/codex/state-db": "Codex's state database",
  "openai/codex/auth-claims": "Codex's sign-in",
  "github/gh/hosts": "gh's hosts.yml",
  "nous/hermes-agent/state-db": "Hermes Agent's session database",
  "nous/hermes-agent/spawn-ledger": "Hermes Agent's spawn ledger",
  "nous/hermes-agent/gateway-state": "Hermes Agent's gateway state",
  "nous/hermes-agent/processes": "Hermes Agent's background processes",
  "nous/hermes-agent/cron-jobs": "Hermes Agent's scheduled jobs",
  "nous/hermes-agent/cron-ticker": "Hermes Agent's scheduler ticker",
  "nous/hermes-agent/install-stamp": "Hermes Agent's build stamp",
  "anomaly/opencode/db": "OpenCode's database",
  "google/gemini-cli/chat-record": "Gemini CLI's chat records",
  "github/copilot-cli/session-event": "Copilot CLI's session events",
  "anysphere/cursor/hook-input": "Cursor's hook input",
  "spacexai/grok-build/session-summary": "Grok Build's session summaries",
  "spacexai/grok-build/update-line": "Grok Build's session updates",
  "spacexai/grok-build/subagent-meta": "Grok Build's subagents' metadata",
  "spacexai/grok-build/usage": "Grok Build's usage ledgers",
};
export const sourceName = (source) => SOURCES[source] ?? source;

// ---------- shapes ----------

export const jsonType = (v) => (v === null ? "null" : Array.isArray(v) ? "array" : typeof v === "object" ? "object" : typeof v);

// A key that reads as a field's name, not as data: no slashes, dots, @,
// colons or spaces, no runs of digits, no hex id (all hex, or seven hex
// characters in a row with a digit among them: a commit's short hash, an id
// inside a name) and no token's prefix. Logins, repos and branches can look
// like names, so an object keyed by them is declared a record in its schema,
// and what's inside a record keeps no key at all (see shapeOf's `names`).
const FIELD = /^[A-Za-z_$][\w$-]{0,47}$/;
const TOKEN = /^(?:gh[pousr]_|github_pat_|gl(?:pat|dt|rt|ptt|soat|ft)-|sk-|sk_(?:live|test)_|[pr]k_(?:live|test)_|whsec_|xox[a-z]-|xapp-|A[KS]IA[0-9A-Z]{12}|AIza|eyJ|npm_|pypi-|hf_|shp(?:at|ss|ca|pa)_|dop_v1_)/;
const hexId = (k) => /^[0-9a-f-]{12,}$/i.test(k) || (k.match(/[0-9a-f]{7,}/gi) ?? []).some((run) => /\d/.test(run));
export const isField = (k) => FIELD.test(k) && !/\d{4,}/.test(k) && !hexId(k) && !TOKEN.test(k);
const MAX_KEYS = 32;
const MAX_ITEMS = 8;
const MAX_ALTERNATIVES = 4;

const union = (shapes) => {
  const distinct = [...new Set(shapes)].sort();
  return distinct.length > MAX_ALTERNATIVES ? [...distinct.slice(0, MAX_ALTERNATIVES), "…"].join("|") : distinct.join("|");
};

// The types in `value`, at most `depth` levels of objects and arrays deep,
// and at most about `budget` characters long, key names included. With
// `names: false`, no key's name: every object is a record<…> of its values'
// types, for what's inside an object keyed by data.
export function shapeOf(value, { depth = 4, budget = 400, names = true } = {}) {
  let left = budget;
  const walk = (v, d) => {
    if (left <= 0) return "…";
    const t = jsonType(v);
    let s;
    if (t === "array") {
      if (d <= 0) s = "array";
      else s = `array<${union(v.slice(0, MAX_ITEMS).map((x) => walk(x, d - 1)))}>`;
    } else if (t === "object") {
      const keys = Object.keys(v).filter((k) => v[k] !== undefined);
      if (d <= 0) s = "object";
      else if (!names || keys.length > MAX_KEYS || keys.some((k) => !isField(k))) s = `record<${union(keys.slice(0, MAX_ITEMS).map((k) => walk(v[k], d - 1)))}>`;
      else {
        const parts = [];
        for (const k of keys.sort()) {
          if (left <= 0) {
            parts.push("…");
            break;
          }
          // The name, its colon and the comma after it.
          left -= k.length + 2;
          parts.push(`${k}:${walk(v[k], d - 1)}`);
        }
        s = `object{${parts.join(",")}}`;
      }
    } else s = t === "bigint" ? "number" : t;
    left -= s.length;
    return s;
  };
  return walk(value, depth);
}

// A discriminator's value as a shape: the value itself when it's a short
// name, else only its type.
export function variantShape(value) {
  if (typeof value === "string") return /^[A-Za-z0-9_.-]{1,64}$/.test(value) ? JSON.stringify(value) : "string";
  if (typeof value === "number" || typeof value === "boolean") return JSON.stringify(value);
  return jsonType(value);
}

// A vendor version as it's kept: one that reads as a version (2.1.288,
// 0.155.0-alpha.9.2, Python's 0.22.0rc1 and 0.22.0.dev0, 31), or null.
// What follows its numbers is numbers, the usual tags and commit hashes
// only, since anything else there (a branch's name, a person's) is data.
const CORE = /^v?\d+(?:\.\d+){0,3}/;
const TAG = /^(?:\d+|(?:alpha|beta|rc|pre|preview|dev|post|canary|nightly|next|snapshot|stable|build|sha|a|b|c)\d*|g?[0-9a-f]{7,40})$/i;
export function cleanVersion(v) {
  if (typeof v !== "string" || v.length > 64) return null;
  const core = CORE.exec(v)?.[0];
  if (!core) return null;
  // Split where a separator is: one may lead, as in -alpha, but none is empty.
  return v.slice(core.length).split(/[.+-]/).every((part, i) => (part ? TAG.test(part) : i === 0)) ? v : null;
}

// ---------- records ----------

// The type a shape starts with: object for object{…} and record<…>.
const rootType = (shape) => {
  const t = /^[a-z]+/.exec(shape ?? "")?.[0] ?? "";
  return t === "record" ? "object" : t;
};

// A variant as a record keeps it: a list of the discriminators' values, or
// nothing for none (an empty list, or anything else).
const isValue = (v) => typeof v === "string" || typeof v === "number" || typeof v === "boolean";
export const cleanVariant = (variant) => (Array.isArray(variant) && variant.length && variant.length <= 8 && variant.every(isValue) ? variant : null);
const variantKey = (variant) => (cleanVariant(variant) ? `|${JSON.stringify(variant)}` : "");

// Which record a piece of drift adds to. A type change is told apart by the
// type found (`type`, or the shape's). One in a variant (see above) adds to
// that variant's.
export function keyOf({ source, kind, path: at = null, shape = null, type = null, vendorVersion = null, variant = null }) {
  switch (kind) {
    case "new-version":
      return `${source}|${kind}|${vendorVersion}`;
    case "type-change":
      return `${source}|${kind}|${at}|${type ?? rootType(shape)}${variantKey(variant)}`;
    case "new-variant":
      return `${source}|${kind}|${at}|${shape}${variantKey(variant)}`;
    case "unparsable":
      return `${source}|${kind}`;
    default:
      return `${source}|${kind}|${at}${variantKey(variant)}`;
  }
}

// What a record says, in a few words.
export function describe(r) {
  const where = cleanVariant(r.variant) ? `, in ${r.variant.join(" › ")}` : "";
  switch (r.kind) {
    case "new-field":
      return `${r.path} is new (${r.shape})${where}`;
    case "missing-field":
      return `${r.path} is missing${where}`;
    case "type-change":
      return `${r.path ?? "a record"} is ${r.shape} now${where}`;
    case "new-variant":
      return `${r.path} ${r.shape} isn't one the schema lists${where}`;
    case "new-version":
      return `version ${r.vendorVersion} is outside the versions the schema was made for`;
    case "unparsable":
      return "lines that aren't JSON";
    default:
      return r.kind;
  }
}

const isRecord = (r) =>
  r && typeof r === "object" && typeof r.source === "string" && KINDS.includes(r.kind) && Number.isFinite(r.firstSeen) && Number.isFinite(r.lastSeen) && Number.isInteger(r.count) && r.count > 0;

// The key a record in a variant had in a drift.json from before records
// said their variant: the same, without it. Null for one in no variant.
const legacyKey = (r) => (cleanVariant(r.variant) ? keyOf({ ...r, variant: null }) : null);

// The drift XenoNet has seen, merged, at most `max` records (the ones seen
// least lately go first), and kept in a file once open() names one.
// Several processes can share the file (the server, npm run contracts):
// each adds the counts it saw since it last wrote, and `lookedAt` (when you
// last looked, through the report) only moves forward.
export class DriftLog {
  constructor({ max = DRIFT_MAX, clock = systemClock, saveAfterMs = SAVE_AFTER_MS } = {}) {
    this.max = max;
    this.clock = clock;
    this.saveAfterMs = saveAfterMs;
    this.records = new Map(); // key → { record, base, delta }
    // Keys of records from before variants that a record in its variant
    // took the place of (see #adopt): drift.json may still have them.
    this.retired = new Set();
    this.lookedAt = null;
    this.file = null;
    this.mtime = null;
    this.dirty = false;
    this.timer = null;
  }

  // Adds a piece of drift: { source, vendorVersion, kind, path, variant,
  // shape }. `shape` can be a function, called only for a record that's
  // new; a type change then says the type it found (`type`, jsonType's).
  // Returns the record.
  record({ source, vendorVersion = null, kind, path: at = null, variant = null, shape = null, type = null }, now = this.clock.now()) {
    const lazy = typeof shape === "function";
    const where = cleanVariant(variant);
    const key = keyOf({ source, kind, path: at, variant: where, vendorVersion, type, shape: lazy ? null : shape });
    let e = this.records.get(key);
    if (e) {
      e.record.lastSeen = Math.max(e.record.lastSeen, now);
      e.delta++;
      e.record.count = e.base + e.delta;
    } else {
      const record = { schema: "drift/record/v1", source, vendorVersion: cleanVersion(vendorVersion), kind, path: at, ...(where ? { variant: [...where] } : {}), shape: lazy ? shape() : shape, firstSeen: now, lastSeen: now, count: 1 };
      e = this.#adopt(record, { type }) ?? { record, base: 0, delta: 1 };
      this.records.set(key, e);
      this.#prune();
    }
    this.#changed();
    return e.record;
  }

  // A record that says its variant takes the place of the same drift kept
  // before records said theirs, with its history: when it was first seen,
  // in which version, and its count. So drift you've already looked at
  // isn't new again for saying where it is. Returns the entry, or null
  // when there's nothing to take over.
  #adopt(record, { type = null } = {}) {
    const old = legacyKey({ ...record, type });
    const e = old && this.records.get(old);
    if (!e || e.record.variant) return null;
    this.records.delete(old);
    this.retired.add(old);
    const { schema, source, vendorVersion, kind, path: at, shape, firstSeen, lastSeen } = e.record;
    const adopted = { schema, source, vendorVersion, kind, path: at, variant: record.variant, shape, firstSeen, lastSeen: Math.max(lastSeen, record.lastSeen), count: e.base + e.delta + 1 };
    return { record: adopted, base: e.base, delta: e.delta + 1 };
  }

  // Every record, oldest first.
  list() {
    return [...this.records.values()].map((e) => e.record).sort((a, b) => a.firstSeen - b.firstSeen || a.lastSeen - b.lastSeen);
  }

  // The records first seen after `t` (ms), oldest first.
  since(t) {
    return this.list().filter((r) => r.firstSeen > (t ?? -Infinity));
  }

  // You've looked at the drift there is: what comes after is new.
  looked(at = this.clock.now()) {
    this.lookedAt = Math.max(this.lookedAt ?? 0, at);
    this.#changed();
  }

  #changed() {
    this.dirty = true;
    if (!this.file || this.timer) return;
    this.timer = this.clock.setTimeout(() => {
      this.timer = null;
      this.flush();
    }, this.saveAfterMs);
    this.timer?.unref?.();
  }

  #prune() {
    if (this.records.size <= this.max) return;
    const oldest = [...this.records].sort(([, a], [, b]) => a.record.lastSeen - b.record.lastSeen || a.record.firstSeen - b.record.firstSeen);
    for (const [key] of oldest.slice(0, this.records.size - this.max)) this.records.delete(key);
  }

  // Keeps the drift in `file` from now on, with what's there already.
  open(file) {
    this.file = file;
    this.mtime = null;
    this.#sync();
    if (this.dirty) this.#changed();
    return this;
  }

  // What the file holds now, merged in: its counts are the base this
  // process's own add to.
  #sync({ force = false } = {}) {
    if (!this.file) return;
    let st = null;
    try {
      st = fs.statSync(this.file);
    } catch {}
    if (!st) return;
    if (!force && this.mtime === st.mtimeMs) return;
    const data = readDriftFile(this.file);
    this.mtime = st.mtimeMs;
    if (!data) return;
    if (data.lookedAt != null) this.lookedAt = Math.max(this.lookedAt ?? 0, data.lookedAt);
    for (const r of data.records) {
      const key = keyOf(r);
      // A record from before variants that one in its variant took the
      // place of, here or in another process: that one has its history.
      if (this.retired.has(key)) continue;
      const old = legacyKey(r);
      if (old) {
        this.retired.add(old);
        this.records.delete(old);
      }
      const e = this.records.get(key);
      if (!e) {
        this.records.set(key, { record: { ...r }, base: r.count, delta: 0 });
        continue;
      }
      const first = r.firstSeen < e.record.firstSeen;
      Object.assign(e.record, {
        firstSeen: Math.min(e.record.firstSeen, r.firstSeen),
        lastSeen: Math.max(e.record.lastSeen, r.lastSeen),
        vendorVersion: first ? r.vendorVersion : e.record.vendorVersion,
        shape: first ? r.shape : e.record.shape,
      });
      e.base = r.count;
      e.record.count = e.base + e.delta;
    }
    this.#prune();
  }

  // Writes what's changed, merged with what other processes wrote since.
  flush() {
    if (this.timer) {
      this.clock.clearTimeout(this.timer);
      this.timer = null;
    }
    if (!this.file || !this.dirty) return;
    this.#sync({ force: true });
    const records = this.list();
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      const tmp = `${this.file}.${process.pid}.tmp`;
      fs.writeFileSync(tmp, JSON.stringify({ version: FILE_VERSION, lookedAt: this.lookedAt, records }, null, 1) + "\n");
      fs.renameSync(tmp, this.file);
      this.mtime = fs.statSync(this.file).mtimeMs;
    } catch (err) {
      console.error(`[xenonet] couldn't save drift to ${this.file}: ${err.message}`);
      return;
    }
    for (const e of this.records.values()) {
      e.base = e.record.count;
      e.delta = 0;
    }
    this.dirty = false;
  }

  // The drift as the doctor and the report see it, with what other
  // processes wrote since.
  view() {
    this.#sync();
    return { file: this.file, lookedAt: this.lookedAt, records: this.list() };
  }

  // Forgets everything (tests).
  clear() {
    this.records.clear();
    this.retired.clear();
    this.lookedAt = null;
    this.dirty = false;
  }
}

// drift.json as it is: { file, lookedAt, records }, with what doesn't read
// as a record left out; null when there's no file or it isn't drift.json.
export function readDriftFile(file) {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch {
    return null;
  }
  if (!data || typeof data !== "object" || !Array.isArray(data.records)) return null;
  // A variant that isn't a list of values is no variant.
  const records = data.records.filter(isRecord).map(({ variant, ...r }) => (cleanVariant(variant) ? { ...r, variant } : r));
  return { file, lookedAt: Number.isFinite(data.lookedAt) ? data.lookedAt : null, records };
}

// This process's drift. The server keeps it in the cache folder; tests and
// tools that never open() it keep it in memory.
export const drift = new DriftLog();
