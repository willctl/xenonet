// Vendor data, read leniently (contracts, section 1). Each source XenoNet
// reads (a transcript's lines, the app-server's answers, Claude desktop's
// files, …) has a reader here, which checks its records against the
// source's schema (core/schema/lenient.js), picked by the version the data
// carries, and keeps what differs as drift (core/drift.js):
//
//   - a record that doesn't fit is used for the fields that do: a field of
//     another type is taken out, so the reader sees it as missing and falls
//     back the way it does for one that's missing;
//   - a record is dropped only when nothing can be read from it (it isn't
//     an object);
//   - every difference is a drift record, and so is a version outside every
//     range the source's schemas were made for (new-version).
//
// Records come by the hundred thousand in the tailer, so a whole check is
// made of a sample: the first of each kind of record in each file, and the
// 2nd, 4th, 8th… after it (transcriptHooks), or every so many of a list
// (sample). The rest are only guarded: a type change found once in a kind
// of record is looked for again in every record of that kind, along the
// path it was found at, which costs a few property reads.
import fs from "node:fs";
import { loadVendor, vendorSources, inspect, recheck, strip, jsonType } from "./schema/lenient.js";
import { drift as theLog, shapeOf, variantShape, cleanVersion, sourceName, describe, ADDITIONS } from "./drift.js";
import { tilde } from "./paths.js";
import { plural } from "./units.js";

// ---------- versions ----------

// "2.1.288" or "0.155.0-alpha.9.2" → [2, 1, 288, pre]; a missing part is 0.
export function parseVersion(v) {
  const m = /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(String(v ?? "").trim());
  return m ? [Number(m[1]), Number(m[2] ?? 0), Number(m[3] ?? 0), m[4] ?? null] : null;
}

function comparePre(a, b) {
  if (a === b) return 0;
  if (a == null) return 1;
  if (b == null) return -1;
  const [x, y] = [a.split("."), b.split(".")];
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if (x[i] === undefined) return -1;
    if (y[i] === undefined) return 1;
    const [nx, ny] = [/^\d+$/.test(x[i]), /^\d+$/.test(y[i])];
    const c = nx && ny ? Number(x[i]) - Number(y[i]) : nx ? -1 : ny ? 1 : x[i].localeCompare(y[i]);
    if (c) return Math.sign(c);
  }
  return 0;
}

export const compareVersions = (a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2] || comparePre(a[3], b[3]);

// A range as comparators to meet, one set per || alternative: "*",
// ">=1.0.0 <3.0.0", "<1.0.0", "2" (any 2.x.y), "2.1", "^0.3.0", "~2.1.0".
function parseRange(range) {
  const sets = [];
  for (const alt of String(range ?? "").split("||")) {
    const set = [];
    for (const part of alt.trim().split(/\s+/).filter(Boolean)) {
      if (part === "*" || /^x$/i.test(part)) continue;
      const m = /^(>=|<=|>|<|=|\^|~)?v?(\d+|x|\*)(?:\.(\d+|x|\*))?(?:\.(\d+|x|\*))?(-[0-9A-Za-z.-]+)?$/i.exec(part);
      if (!m) return null;
      const [, op = "", ...rest] = m;
      const nums = rest.slice(0, 3).map((n) => (n === undefined || /^[x*]$/i.test(n) ? null : Number(n)));
      const pre = rest[3] ? rest[3].slice(1) : null;
      const given = nums.findIndex((n) => n == null);
      const exact = given < 0;
      const v = [nums[0] ?? 0, nums[1] ?? 0, nums[2] ?? 0, pre];
      // What a partial version means: 2 is 2.0.0 up to 3.0.0, 2.1 up to 2.2.0.
      const bump = (i) => (i === 0 ? [v[0] + 1, 0, 0, null] : i === 1 ? [v[0], v[1] + 1, 0, null] : null);
      if (op === "^") {
        const i = v[0] > 0 || !exact && given <= 1 ? 0 : v[1] > 0 || (!exact && given === 2) ? 1 : 2;
        set.push([">=", v], ["<", i === 2 ? [v[0], v[1], v[2] + 1, null] : bump(i)]);
      } else if (op === "~") set.push([">=", v], ["<", bump(exact || given > 1 ? 1 : 0)]);
      else if (!exact && (op === "" || op === "=")) {
        if (given > 0) set.push([">=", v], ["<", bump(given - 1)]);
      } else if (!exact && op === ">") set.push([">=", bump(given - 1)]);
      else if (!exact && op === "<=") set.push(["<", bump(given - 1)]);
      else set.push([op || "=", v]);
    }
    sets.push(set);
  }
  return sets;
}

export const validRange = (range) => parseRange(range) != null;

// Whether `version` is in `range`. A prerelease counts as the version it
// comes before (0.155.0-alpha is in "<1.0.0"), unlike npm's rule, since
// vendors ship prereleases to the people XenoNet runs for.
export function satisfies(version, range) {
  const v = parseVersion(version);
  const sets = parseRange(range);
  if (!v || !sets) return false;
  const meets = ([op, w]) => {
    const c = compareVersions(v, w);
    return op === ">=" ? c >= 0 : op === ">" ? c > 0 : op === "<=" ? c <= 0 : op === "<" ? c < 0 : c === 0;
  };
  return sets.some((set) => set.every(meets));
}

// ---------- readers ----------

// A source's reader: its schemas (newest first) and what it has read.
export class VendorReader {
  constructor(source, { log = theLog, majors = null } = {}) {
    this.source = source;
    this.log = log;
    this.majorsGiven = majors;
    this.picks = new Map(); // version → the schema it reads with
    this.careful = new Map(); // a kind of record → the type changes found in it, to look for again
    this.calls = 0;
    // What it has read: records offered, checked whole, dropped, and with
    // fields taken out; and the versions seen (version → records checked).
    this.stats = { records: 0, checked: 0, dropped: 0, stripped: 0, versions: new Map() };
  }

  // The schemas, read from schemas/ the first time they're needed. None
  // when the files aren't there: then records pass as they are.
  get majors() {
    this.majorsGiven ??= loadVendor(this.source);
    return this.majorsGiven;
  }

  // The schema for data of `version`: the newest whose range has it, or,
  // for a version none has, the newest (and the version is drift). Data
  // that says no version is read with the newest.
  pick(version) {
    const v = cleanVersion(version);
    if (this.picks.has(v)) return this.picks.get(v);
    const majors = this.majors;
    let schema = majors[0] ?? null;
    if (v != null && majors.length) {
      schema = majors.find((m) => satisfies(v, m.range));
      if (!schema) {
        schema = majors[0];
        this.log.record({ source: this.source, vendorVersion: v, kind: "new-version" });
      }
    }
    this.picks.set(v, schema);
    return schema;
  }

  // A record read, checked whole: the record to use (the same, or without
  // the fields of another type), or null when nothing in it can be read.
  // `kind` sorts records for guard() (a line's type).
  check(record, opts) {
    this.stats.records++;
    return this.inspect(record, opts);
  }

  // The whole check, for a record already counted (sample(), the
  // transcript hooks).
  inspect(record, { version = null, kind = null } = {}) {
    this.stats.checked++;
    const v = cleanVersion(version);
    this.stats.versions.set(v, (this.stats.versions.get(v) ?? 0) + 1);
    const schema = this.pick(v);
    if (!schema) return record;
    const found = inspect(schema.spec, record);
    for (const f of found.findings) this.#keep(f, v);
    if (found.drop) {
      this.stats.dropped++;
      return null;
    }
    const changes = found.findings.filter((f) => f.kind === "type-change" && f.segments.length);
    if (changes.length) {
      const list = this.careful.get(kind) ?? [];
      for (const f of changes) if (!list.some((c) => c.path === f.path && String(c.variant) === String(f.variant))) list.push({ path: f.path, variant: f.variant, segments: f.segments, spec: f.spec });
      this.careful.set(kind, list);
    }
    // The walk looks at a sample of a long array's elements; the others are
    // looked at for the changes it found.
    return this.#fix(record, found.strip, kind, v);
  }

  // A record not checked whole: the type changes found before in its kind
  // are looked for, and taken out.
  guard(record, { version = null, kind = null } = {}) {
    if (!this.careful.has(kind)) return record;
    return this.#fix(record, [], kind, cleanVersion(version));
  }

  // Takes out `paths`, and each place a type change found before in this
  // kind of record is found again (each counted as drift, unless the whole
  // check counted it already, and said to be in the variant it was found
  // in, since only a whole check walks the variants).
  #fix(record, paths, kind, v) {
    const careful = this.careful.get(kind);
    let all = paths;
    if (careful) {
      const seen = new Set(paths.map((p) => JSON.stringify(p)));
      for (const c of careful)
        for (const keys of recheck(record, c.segments, c.spec)) {
          const id = JSON.stringify(keys);
          if (seen.has(id)) continue;
          seen.add(id);
          all = [...all, keys];
          const value = valueAt(record, keys);
          this.log.record({ source: this.source, vendorVersion: v, kind: "type-change", path: c.path, variant: c.variant, type: jsonType(value), shape: () => shapeOf(value, { names: !c.segments.includes("*") }) });
        }
    }
    if (!all.length) return record;
    this.stats.stripped++;
    return strip(record, all);
  }

  // Every `every`th record checked whole, in turn, the first included; the
  // rest guarded. For lists read again and again (sessions, files).
  sample(record, { version = null, kind = null, every = 16 } = {}) {
    this.stats.records++;
    return this.calls++ % every === 0 ? this.inspect(record, { version, kind }) : this.guard(record, { version, kind });
  }

  // Drift found outside a record: a line that isn't JSON, a column a query
  // didn't find.
  unparsable(version = null) {
    this.log.record({ source: this.source, vendorVersion: version, kind: "unparsable" });
  }
  missing(path, version = null) {
    this.log.record({ source: this.source, vendorVersion: version, kind: "missing-field", path });
  }

  // What a finding inside an object keyed by data found keeps no key's name
  // (see inspect() in lenient.js).
  #keep(f, v) {
    const base = { source: this.source, vendorVersion: v, kind: f.kind, path: f.path, variant: f.variant };
    if (f.kind === "new-field") this.log.record({ ...base, shape: () => shapeOf(f.value, { names: !f.data }) });
    else if (f.kind === "type-change") this.log.record({ ...base, type: jsonType(f.value), shape: () => shapeOf(f.value, { names: !f.data }) });
    else if (f.kind === "new-variant") this.log.record({ ...base, shape: variantShape(f.value) });
    else this.log.record(base);
  }

  // What it has read, for the report.
  summary() {
    const versions = [...this.stats.versions.keys()].filter((v) => v != null).sort((a, b) => compareVersions(parseVersion(a) ?? [0, 0, 0, null], parseVersion(b) ?? [0, 0, 0, null]));
    return {
      source: this.source,
      schemas: this.majors.map(({ id, major, range }) => ({ id, major, range })),
      records: this.stats.records,
      checked: this.stats.checked,
      dropped: this.stats.dropped,
      stripped: this.stats.stripped,
      versions,
    };
  }
}

const valueAt = (record, keys) => keys.reduce((v, k) => v?.[k], record);

// One reader per source, made the first time it's asked for.
const readers = new Map();
export function reader(source) {
  let r = readers.get(source);
  if (!r) readers.set(source, (r = new VendorReader(source)));
  return r;
}
export const allReaders = () => [...readers.values()];

// ---------- the report ----------

// What was read and found, by source, for the drift report (npm run
// contracts): every source with a schema, its schemas, what this process
// read of it (null when nothing), and its drift that's new since `since`
// (when you last looked), or all of it.
export function vendorReport({ log = theLog, since = log.lookedAt, all = false, readers: list = allReaders(), dir } = {}) {
  const records = log.list();
  return {
    since: since ?? null,
    sources: vendorSources(dir).map((source) => {
      // A reader made but never given a record read nothing.
      const r = list.find((x) => x.source === source && x.stats.records + x.stats.checked > 0);
      const mine = records.filter((x) => x.source === source);
      const { schemas, ...read } = r ? r.summary() : { schemas: loadVendor(source, dir).map(({ id, major, range }) => ({ id, major, range })) };
      return {
        source,
        name: sourceName(source),
        schemas,
        read: r ? { records: read.records, checked: read.checked, dropped: read.dropped, stripped: read.stripped, versions: read.versions } : null,
        drift: mine.filter((x) => all || x.firstSeen > (since ?? -Infinity)),
        known: mine.length,
      };
    }),
  };
}

// The drift report, as xenonet drift, GET /api/drift and npm run contracts
// give it: vendorReport's sources, with what other processes wrote to
// drift.json taken in, since you last looked (`since`), and
//   at       when it was made: marking it seen goes up to here, so drift
//            first seen after it stays new
//   file     where the drift is kept (under `home` as ~)
//   new      how many records were first seen since you last looked
//   changes  how many of those warn (see ADDITIONS)
//   known    how many are kept in all
// With `all`, each source lists every record kept, not only the new ones.
export function driftReport({ log = theLog, all = false, readers, dir, home = null, now = Date.now() } = {}) {
  const { file, lookedAt } = log.view();
  const report = vendorReport({ log, since: lookedAt, all, readers, dir });
  const fresh = report.sources.flatMap((s) => s.drift).filter((r) => r.firstSeen > (lookedAt ?? -Infinity));
  return {
    at: now,
    file: file && home ? tilde(file, home) : file,
    since: report.since,
    all,
    sources: report.sources,
    new: fresh.length,
    changes: fresh.filter((r) => !ADDITIONS.has(r.kind)).length,
    known: report.sources.reduce((n, s) => n + s.known, 0),
  };
}

// The versions a source's records said, briefly.
const versionsOf = (versions) => (!versions.length ? "no version in the data" : versions.length <= 3 ? `version${versions.length === 1 ? "" : "s"} ${versions.join(", ")}` : `versions ${versions[0]} to ${versions.at(-1)} (${versions.length})`);
const timesOf = (r) => [r.vendorVersion ? `since ${r.vendorVersion}` : null, plural(r.count, "time")].filter(Boolean).join("; ");

// The report as lines of text, by source: its schema, what was read of it
// (or `unread`, said of a source nothing was read of; null leaves the line
// out), its drift, each a change or an addition, and a summary. `paint`
// colors the parts ({ name, changed, added, dim }); `quiet` leaves out
// sources with nothing to say.
export function reportLines(report, { unread = "nothing this time", paint = {}, quiet = false } = {}) {
  const p = { name: String, changed: String, added: String, dim: String, ...paint };
  const lines = [];
  for (const s of report.sources) {
    if (quiet && !s.read && !s.drift.length && !s.known) continue;
    const schema = s.schemas[0];
    lines.push(`  ${p.name(s.name)}  ${p.dim(schema ? `${schema.id} (${schema.range})` : "no schema file")}`);
    if (s.read) lines.push(`    read     ${plural(s.read.records, "record")} (${s.read.checked.toLocaleString("en-US")} checked whole${s.read.dropped ? `, ${s.read.dropped} with nothing to read` : ""}), ${versionsOf(s.read.versions)}`);
    else if (unread) lines.push(`    read     ${unread}`);
    for (const r of s.drift) lines.push(`    ${ADDITIONS.has(r.kind) ? p.added("added   ") : p.changed("changed ")} ${describe(r)}  ${p.dim(`(${timesOf(r)})`)}`);
    if (!s.drift.length && s.known) lines.push(`    ${p.dim(`no drift ${report.all ? "kept" : "since you last looked"} (${s.known} known)`)}`);
  }
  const listed = report.sources.flatMap((s) => s.drift);
  const when = report.since ? `you last looked (${new Date(report.since).toLocaleString()})` : "XenoNet started keeping them";
  lines.push(
    report.all
      ? `  ${plural(report.known, "difference")} kept, ${plural(listed.filter((r) => !ADDITIONS.has(r.kind)).length, "change")} to what XenoNet reads`
      : `  ${plural(report.new, "difference")} new since ${when}, ${plural(report.changes, "change")} to what XenoNet reads; ${report.known} known in all`,
  );
  return lines;
}

// ---------- transcripts ----------

const STATE = Symbol("vendor");
const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

// The hooks a transcript's normalizer uses (lines.js): each line counted,
// the version each file's lines carry followed, a sample of each kind of
// line in each file checked whole (the 1st, 2nd, 4th, 8th…), the rest
// guarded, and lines that aren't JSON or aren't objects kept as drift.
// `kindOf(line)` sorts lines (Claude Code's by type, Codex's by type and
// payload type); `versionOf(line, current)` says the version a line
// carries, or the file's so far. `where` is an object the caller keeps per
// file (the tailer's entry); the sampling state rides on it.
export function transcriptHooks(source, { kindOf, versionOf, reader: r = reader(source) }) {
  const shared = { version: null, counts: new Map() };
  const stateOf = (where) => (where && typeof where === "object" ? (where[STATE] ??= { version: null, counts: new Map() }) : shared);
  return {
    reader: r,
    inspect(raw, where) {
      r.stats.records++;
      const st = stateOf(where);
      if (!isObject(raw)) return r.inspect(raw, { version: st.version });
      const version = versionOf(raw, st.version);
      if (version !== st.version) {
        st.version = version;
        // A version not seen before is checked against the schemas' ranges.
        r.pick(version);
      }
      const kind = kindOf(raw);
      const n = (st.counts.get(kind) ?? 0) + 1;
      st.counts.set(kind, n);
      // Powers of two: 1, 2, 4, 8, …
      if ((n & (n - 1)) === 0) return r.inspect(raw, { version, kind });
      return r.careful.size ? r.guard(raw, { version, kind }) : raw;
    },
    unparsable(text, where) {
      r.stats.records++;
      r.unparsable(stateOf(where).version);
    },
  };
}

// ---------- files that aren't JSON ----------

// Where JSON text first goes wrong, as { line, column } (both from 1), or
// null when it doesn't. V8's message for some errors quotes the text
// around the place ('Unexpected token 'x', "…" is not valid JSON'), and a
// sign-in file's text can hold a token or an email, so this finds the place
// itself, a value at a time, with no recursion for deep nesting to exhaust.
const NUMBER = /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/y;
const SPACE = new Set([" ", "\t", "\n", "\r"]);
export function jsonErrorAt(text) {
  const s = String(text);
  const n = s.length;
  const open = []; // the objects and lists the place is in: "{" or "["
  let i = 0;
  // What may come next: "value" (after "[" also the list's end:
  // "value-or-end"), "key" (after "{": "key-or-end"), "after" a value in an
  // object or list, or nothing but space at the "end".
  let want = "value";
  const where = () => {
    let line = 1;
    let start = 0;
    for (let nl = s.indexOf("\n"); nl !== -1 && nl < i; nl = s.indexOf("\n", nl + 1)) [line, start] = [line + 1, nl + 1];
    return { line, column: i - start + 1 };
  };
  const space = () => {
    while (i < n && SPACE.has(s[i])) i++;
  };
  // A string, from its opening quote; false where it goes wrong.
  const string = () => {
    for (i++; i < n; i++) {
      const c = s.charCodeAt(i);
      if (c === 0x22) return ++i > 0;
      if (c < 0x20) return false;
      if (c !== 0x5c) continue;
      i++;
      if (s[i] === "u") {
        if (!/^[0-9a-fA-F]{4}$/.test(s.slice(i + 1, i + 5))) return false;
        i += 4;
      } else if (!'"\\/bfnrt'.includes(s[i] ?? "x")) return false;
    }
    return false;
  };
  const done = () => (want = open.length ? "after" : "end");
  for (;;) {
    space();
    if (i >= n) return want === "end" ? null : where();
    const c = s[i];
    if (want === "end") return where();
    if (want === "after") {
      const close = open.at(-1) === "{" ? "}" : "]";
      if (c === ",") want = close === "}" ? "key" : "value";
      else if (c === close) open.pop(), done();
      else return where();
      i++;
    } else if (want === "key" || want === "key-or-end") {
      if (want === "key-or-end" && c === "}") {
        i++;
        open.pop();
        done();
        continue;
      }
      if (c !== '"' || !string()) return where();
      space();
      if (s[i] !== ":") return where();
      i++;
      want = "value";
    } else if (want === "value-or-end" && c === "]") {
      i++;
      open.pop();
      done();
    } else if (c === "{" || c === "[") {
      i++;
      open.push(c);
      want = c === "{" ? "key-or-end" : "value-or-end";
    } else if (c === '"') {
      if (!string()) return where();
      done();
    } else {
      NUMBER.lastIndex = i;
      const m = NUMBER.exec(s)?.[0] ?? ["true", "false", "null"].find((w) => s.startsWith(w, i));
      if (!m) return where();
      i += m.length;
      done();
    }
  }
}

// JSON.parse for another app's file, whose error says where the text goes
// wrong and never quotes it.
export function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch (err) {
    if (!(err instanceof SyntaxError)) throw err;
    const at = jsonErrorAt(text);
    throw new SyntaxError(at ? `isn't valid JSON (line ${at.line}, column ${at.column})` : "isn't valid JSON");
  }
}

// ---------- the Agent SDK's version ----------

// The installed SDK's version, from its package.json, or null.
let sdk;
export function agentSdkVersion() {
  if (sdk !== undefined) return sdk;
  try {
    const main = new URL(import.meta.resolve("@anthropic-ai/claude-agent-sdk"));
    sdk = cleanVersion(JSON.parse(fs.readFileSync(new URL("package.json", main), "utf8")).version);
  } catch {
    sdk = null;
  }
  return sdk;
}

// Codex's version, from the app-server's user agent: <client>/<version> (…).
export const codexVersionOf = (userAgent) => cleanVersion(/\/(\d+\.\d+\.\d+[\w.+-]*)/.exec(String(userAgent ?? ""))?.[1] ?? null);
