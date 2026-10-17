// core/vendor.js in Rust (crates/xenonet-schema/src/vendor.rs): vendor data
// read leniently, each source by a reader that checks records against the
// source's schema, picked by the version the data carries, and keeps what
// differs as drift. Rust does the versions and ranges, the checks, the
// guards and the sampling. The report's text, the transcript hooks' glue
// between a normalizer and its reader, and the Agent SDK's version (found
// by Node's own module resolution) stay here.
import fs from "node:fs";
import { native } from "../index.js";
import { SCHEMAS_DIR, vendorSources, loadVendor } from "./schema/lenient.js";
import { drift as theLog, LOG, sourceName, describe, ADDITIONS, cleanVersion } from "./drift.js";
import { tilde } from "./paths.js";
import { plural } from "../../../../core/units.js";

// ---------- versions ----------

export const parseVersion = (v) => native.parseVersion(String(v ?? "")) ?? null;
export const compareVersions = (a, b) => native.compareVersions(a, b);
export const validRange = (range) => native.validRange(String(range ?? ""));
export const satisfies = (version, range) => native.satisfies(String(version ?? ""), String(range ?? ""));

// ---------- readers ----------

const str = (v) => (typeof v === "string" ? v : null);
const kindKey = (k) => (k == null ? null : String(k));

export class VendorReader {
  #core;
  #majors = null;

  constructor(source, { log = theLog, majors = null } = {}) {
    this.source = source;
    this.log = log;
    this.majorsGiven = majors;
    this.#core = new native.VendorReaderCore(String(source), SCHEMAS_DIR, majors != null);
    for (const m of majors ?? []) this.#core.addMajor(String(m.id), m.major, m.range, m.spec);
    // What it has read, kept in Rust; set through this as JS code did.
    const core = this.#core;
    this.stats = new Proxy(
      {},
      {
        get: (_, k) => (k === "versions" ? new Map(core.versionCounts()) : typeof k === "string" ? core.stat(k) : undefined),
        set: (_, k, v) => (core.setStat(String(k), Number(v)), true),
      },
    );
    // Whether it has found a type change in a kind of record.
    this.careful = { get size() { return core.anyCareful() ? 1 : 0; }, has: (kind) => core.carefulAbout(kindKey(kind)) };
  }

  get majors() {
    return this.majorsGiven ?? (this.#majors ??= this.#core.majors());
  }

  #now() {
    return this.log.clock?.now?.() ?? Date.now();
  }

  #out(r, record) {
    this.log.touch?.();
    return r.same ? record : r.dropped ? null : r.record;
  }

  pick(version) {
    const i = this.#core.pick(str(version), this.log[LOG], this.#now());
    this.log.touch?.();
    return i == null ? null : this.majors[i];
  }

  check(record, opts) {
    this.stats.records++;
    return this.inspect(record, opts);
  }

  inspect(record, { version = null, kind = null } = {}) {
    return this.#out(this.#core.inspect(record, str(version), kindKey(kind), this.log[LOG], this.#now()), record);
  }

  guard(record, { version = null, kind = null } = {}) {
    if (!this.#core.carefulAbout(kindKey(kind))) return record;
    return this.#out(this.#core.guard(record, str(version), kindKey(kind), this.log[LOG], this.#now()), record);
  }

  sample(record, { version = null, kind = null, every = 16 } = {}) {
    return this.#out(this.#core.sample(record, str(version), kindKey(kind), every, this.log[LOG], this.#now()), record);
  }

  unparsable(version = null) {
    this.log.record({ source: this.source, vendorVersion: version, kind: "unparsable" });
  }
  missing(path, version = null) {
    this.log.record({ source: this.source, vendorVersion: version, kind: "missing-field", path });
  }

  summary() {
    return {
      source: this.source,
      schemas: this.majors.map(({ id, major, range }) => ({ id, major, range })),
      records: this.stats.records,
      checked: this.stats.checked,
      dropped: this.stats.dropped,
      stripped: this.stats.stripped,
      versions: this.#core.versions(),
    };
  }
}

const readers = new Map();
export function reader(source) {
  let r = readers.get(source);
  if (!r) readers.set(source, (r = new VendorReader(source)));
  return r;
}
export const allReaders = () => [...readers.values()];

// ---------- the report ----------

export function vendorReport({ log = theLog, since = log.lookedAt, all = false, readers: list = allReaders(), dir } = {}) {
  const records = log.list();
  return {
    since: since ?? null,
    sources: vendorSources(dir).map((source) => {
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

const versionsOf = (versions) => (!versions.length ? "no version in the data" : versions.length <= 3 ? `version${versions.length === 1 ? "" : "s"} ${versions.join(", ")}` : `versions ${versions[0]} to ${versions.at(-1)} (${versions.length})`);
const timesOf = (r) => [r.vendorVersion ? `since ${r.vendorVersion}` : null, plural(r.count, "time")].filter(Boolean).join("; ");

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
// line in each file checked whole (Rust says which: the 1st, 2nd, 4th,
// 8th…), the rest guarded. The sampling state rides on `where`.
export function transcriptHooks(source, { kindOf, versionOf, reader: r = reader(source) }) {
  const shared = new native.Sampling();
  const stateOf = (where) => (where && typeof where === "object" ? (where[STATE] ??= new native.Sampling()) : shared);
  return {
    reader: r,
    inspect(raw, where) {
      r.stats.records++;
      const st = stateOf(where);
      if (!isObject(raw)) return r.inspect(raw, { version: st.version ?? null });
      const version = versionOf(raw, st.version ?? null);
      if (version !== (st.version ?? null)) {
        st.version = version;
        r.pick(version);
      }
      const kind = kindOf(raw);
      if (st.whole(String(kind))) return r.inspect(raw, { version, kind });
      return r.careful.size ? r.guard(raw, { version, kind }) : raw;
    },
    unparsable(text, where) {
      r.stats.records++;
      r.unparsable(stateOf(where).version ?? null);
    },
  };
}

// ---------- files that aren't JSON ----------

export function jsonErrorAt(text) {
  const at = native.jsonErrorAt(String(text));
  return at ? { line: at[0], column: at[1] } : null;
}

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

export const codexVersionOf = (userAgent) => native.codexVersionOf(String(userAgent ?? "")) ?? null;
