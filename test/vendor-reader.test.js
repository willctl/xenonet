// Vendor readers (core/vendor.js): the schema picked by the version the
// data carries, a sample checked whole and the rest guarded, records that
// don't fit used for the fields that do, and what differs kept as drift.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { VendorReader, transcriptHooks, vendorReport, driftReport, reportLines, parseVersion, satisfies, validRange, codexVersionOf, agentSdkVersion } from "../core/vendor.js";
import { compile, loadVendor, pathOf, vendorSources } from "../core/schema/lenient.js";
import { DriftLog } from "../core/drift.js";
import { tempTree, cleanupAll } from "./harness/files.js";

after(cleanupAll);

const T = "2026-10-05T12:00:00.000Z";

test("versions and ranges: partial versions, carets, tildes, alternatives, and prereleases as the version they lead to", () => {
  assert.deepEqual(parseVersion("2.1.288"), [2, 1, 288, null]);
  assert.deepEqual(parseVersion("0.155.0-alpha.9.2"), [0, 155, 0, "alpha.9.2"]);
  assert.deepEqual(parseVersion("5"), [5, 0, 0, null]);
  assert.equal(parseVersion("not one"), null);
  const cases = [
    ["*", "9.9.9", true],
    [">=1.0.0 <3.0.0", "2.1.288", true],
    [">=1.0.0 <3.0.0", "3.0.0", false],
    [">=1.0.0 <3.0.0", "0.9.0", false],
    ["<1.0.0", "0.155.0-alpha.9.2", true],
    ["<1.0.0", "1.0.0-rc.1", true],
    ["<1.0.0", "1.0.0", false],
    ["5", "5", true],
    ["5", "5.2.0", true],
    ["5", "6", false],
    ["2.1", "2.1.9", true],
    ["2.1", "2.2.0", false],
    ["^0.3.0", "0.3.287", true],
    ["^0.3.0", "0.4.0", false],
    ["^2.1.0", "2.9.0", true],
    ["~2.1.0", "2.1.5", true],
    ["~2.1.0", "2.2.0", false],
    [">=0.3.0 <0.4.0", "0.3.287", true],
    ["1.x || >=3", "1.4.0", true],
    ["1.x || >=3", "2.0.0", false],
    ["1.x || >=3", "3.1.0", true],
  ];
  for (const [range, version, want] of cases) assert.equal(satisfies(version, range), want, `${version} in ${range}`);
  assert.ok(validRange("*") && validRange(">=1.0.0 <3.0.0") && validRange("2"));
  assert.ok(!validRange("someday") && !validRange(">=one"));
  assert.equal(codexVersionOf("xenonet/0.155.0-alpha.9.2 (Mac OS 26.0.0; arm64) unknown"), "0.155.0-alpha.9.2");
  assert.equal(codexVersionOf("no version here"), null);
  assert.equal(agentSdkVersion(), "0.3.287", "the installed SDK's own");
});

// Two majors of a made-up source: v1 for 1.x, v2 for 2.x, where `n`
// became a string.
const majors = [
  { id: "vendor/test/v2", major: 2, range: ">=2.0.0", spec: compile({ type: "object", properties: { n: { type: "string" } }, required: ["n"] }) },
  { id: "vendor/test/v1", major: 1, range: ">=1.0.0 <2.0.0", spec: compile({ type: "object", properties: { n: { type: "number" } }, required: ["n"] }) },
];

test("the schema for the version the data carries, the newest for data that says none, and a version none was made for is drift", () => {
  const log = new DriftLog();
  const r = new VendorReader("test/source", { log, majors });
  assert.equal(r.pick("1.4.0").major, 1);
  assert.equal(r.pick("2.0.1").major, 2);
  assert.equal(r.pick(null).major, 2);
  assert.deepEqual(r.check({ n: 3 }, { version: "1.4.0" }), { n: 3 });
  assert.deepEqual(r.check({ n: "3" }, { version: "2.0.1" }), { n: "3" });
  assert.deepEqual(log.list(), []);
  assert.equal(r.pick("0.9.0").major, 2, "too old for any: the newest reads it");
  r.pick("0.9.0");
  assert.deepEqual(log.list().map((x) => [x.kind, x.vendorVersion]), [["new-version", "0.9.0"]], "once");
  assert.deepEqual(r.summary(), { source: "test/source", schemas: [{ id: "vendor/test/v2", major: 2, range: ">=2.0.0" }, { id: "vendor/test/v1", major: 1, range: ">=1.0.0 <2.0.0" }], records: 2, checked: 2, dropped: 0, stripped: 0, versions: ["1.4.0", "2.0.1"] });
  // No schema files: records pass as they are.
  const none = new VendorReader("test/none", { log, majors: [] });
  assert.deepEqual(none.check({ anything: 1 }), { anything: 1 });
});

test("a type change: drift with its shape, the field taken out, and looked for again in records not checked whole", () => {
  const log = new DriftLog();
  const [transcripts] = loadVendor("anthropic/claude-code/transcript-line");
  const r = new VendorReader("anthropic/claude-code/transcript-line", { log, majors: [transcripts] });
  const odd = (i) => ({ type: "user", timestamp: T, message: { role: "user", content: { text: `object ${i}` } } });
  const out = r.check(odd(1), { version: "2.1.280", kind: "user" });
  assert.deepEqual(out.message, { role: "user" });
  assert.deepEqual(log.list().map((x) => [x.kind, x.path, x.shape, x.vendorVersion, x.count]), [["type-change", "message.content", "object{text:string}", "2.1.280", 1]]);
  // The next user lines aren't checked whole, but the change is looked for.
  assert.deepEqual(r.guard(odd(2), { kind: "user" }).message, { role: "user" });
  assert.equal(log.list()[0].count, 2);
  const fine = { type: "user", timestamp: T, message: { role: "user", content: "fine" } };
  assert.equal(r.guard(fine, { kind: "user" }), fine, "a record that's fine is the same record");
  const other = odd(3);
  assert.equal(r.guard(other, { kind: "assistant" }), other, "another kind isn't looked at");
  assert.deepEqual([r.stats.checked, r.stats.stripped], [1, 2]);
});

test("drift says the variant it was found in, also where it's looked for again in records not checked whole", () => {
  const log = new DriftLog();
  const [transcripts] = loadVendor("anthropic/claude-code/transcript-line");
  const r = new VendorReader("anthropic/claude-code/transcript-line", { log, majors: [transcripts] });
  const reply = (text) => ({ type: "assistant", timestamp: T, message: { id: "m", model: "m", stop_reason: null, usage: { input_tokens: 1, output_tokens: 2 }, content: [{ type: "text", text }] } });
  r.check(reply(5), { version: "2.1.280", kind: "assistant" });
  assert.deepEqual(r.guard(reply(6), { kind: "assistant" }).message.content, [{ type: "text" }]);
  assert.deepEqual(log.list().map((x) => [x.kind, x.path, x.variant, x.count]), [["type-change", "message.content[].text", ["assistant", "text"], 2]]);
});

test("a list's records: every so many checked whole, in turn", () => {
  const log = new DriftLog();
  const r = new VendorReader("test/source", { log, majors });
  // Every 7th from the 3rd is odd: the 3rd and 10th go by unseen, the 17th
  // is checked whole, and the 24th, 31st and 38th are guarded.
  const out = [];
  for (let i = 0; i < 40; i++) out.push(r.sample({ n: i % 7 === 2 ? "odd" : i }, { version: "1.0.0", every: 16 }));
  assert.equal(r.stats.records, 40);
  assert.equal(r.stats.checked, 3, "the 1st, 17th and 33rd");
  assert.deepEqual(log.list().map((x) => [x.kind, x.path, x.count]), [["type-change", "n", 4]]);
  assert.equal(r.stats.stripped, 4);
  assert.deepEqual([out[2], out[16], out[23]], [{ n: "odd" }, {}, {}]);
});

test("transcript hooks: each file's lines sampled by kind, the version followed, and lines that aren't objects dropped", () => {
  const log = new DriftLog();
  const [transcripts] = loadVendor("anthropic/claude-code/transcript-line");
  const r = new VendorReader("anthropic/claude-code/transcript-line", { log, majors: [transcripts] });
  const hooks = transcriptHooks("anthropic/claude-code/transcript-line", { reader: r, kindOf: (o) => o.type, versionOf: (o, v) => (typeof o.version === "string" ? o.version : v) });
  const fileA = {};
  const line = (i, extra = {}) => ({ type: "user", timestamp: T, version: "2.1.280", message: { role: "user", content: `line ${i}` }, ...extra });
  for (let i = 1; i <= 20; i++) assert.ok(hooks.inspect(line(i), fileA));
  assert.equal(r.stats.checked, 5, "the 1st, 2nd, 4th, 8th and 16th");
  hooks.inspect({ type: "summary", summary: "s" }, fileA);
  assert.equal(r.stats.checked, 6, "the first of another kind");
  const fileB = {};
  hooks.inspect(line(1), fileB);
  assert.equal(r.stats.checked, 7, "and the first in another file");

  // Not an object: nothing to read, dropped and kept as drift; not JSON:
  // drift with the file's version.
  assert.equal(hooks.inspect(null, fileA), null);
  assert.equal(hooks.inspect(42, fileA), null);
  hooks.unparsable("{ half", fileA);
  assert.deepEqual(log.list().map((x) => [x.kind, x.path, x.shape, x.vendorVersion, x.count]), [
    ["type-change", null, "null", "2.1.280", 1],
    ["type-change", null, "number", "2.1.280", 1],
    ["unparsable", null, null, "2.1.280", 1],
  ]);
  // A version past the schema's range, mid-file.
  hooks.inspect(line(21, { version: "3.0.0" }), fileA);
  assert.deepEqual(log.list().at(-1).kind, "new-version");
  assert.equal(r.stats.records, 26);
});

test("paths say where in a record, never a key that's data", () => {
  const at = (parent, key, kind = "prop") => ({ parent, key, kind });
  assert.equal(pathOf(null), null);
  assert.equal(pathOf(at(at(at(null, "message"), 0, "item"), "text")), "message[].text");
  assert.equal(pathOf(at(at(null, "changes"), "/Users/ada/x.py", "entry")), "changes.*");
  assert.equal(pathOf(at(at(null, "https://api.openai.com/auth"), "chatgpt_plan_type")), '["https://api.openai.com/auth"].chatgpt_plan_type');
});

test("the report: every source with a schema, what was read of it, and its drift new since you last looked", () => {
  const log = new DriftLog();
  const [rollouts] = loadVendor("openai/codex/rollout-line");
  const r = new VendorReader("openai/codex/rollout-line", { log, majors: [rollouts] });
  r.check({ timestamp: T, type: "session_meta", payload: { cli_version: "0.150.0" } }, { version: "0.150.0" });
  log.record({ source: "openai/codex/rollout-line", kind: "new-variant", path: "type", shape: '"world_view"' }, 100);
  log.record({ source: "openai/codex/rollout-line", kind: "new-field", path: "ordinal", shape: "number" }, 200);
  const report = vendorReport({ log, since: 150, readers: [r] });
  // The contracts' twelve, Hermes Agent's seven, the native readers' four
  // and Grok Build's two.
  assert.equal(report.sources.length, 25);
  assert.equal(report.sources.find((s) => s.source === "nous/hermes-agent/state-db").name, "Hermes Agent's session database");
  const codex = report.sources.find((s) => s.source === "openai/codex/rollout-line");
  assert.deepEqual({ ...codex, drift: codex.drift.map((x) => x.path) }, {
    source: "openai/codex/rollout-line",
    name: "Codex's rollouts",
    schemas: [{ id: "vendor/openai/codex/rollout-line/v1", major: 1, range: "<1.0.0" }],
    read: { records: 1, checked: 1, dropped: 0, stripped: 0, versions: ["0.150.0"] },
    drift: ["ordinal"],
    known: 2,
  });
  assert.equal(report.sources.find((s) => s.source === "github/gh/hosts").read, null, "nothing read");
  assert.deepEqual(vendorReport({ log, since: 150, all: true, readers: [r] }).sources.find((s) => s.source === "openai/codex/rollout-line").drift.length, 2);
});

test("the drift report as xenonet drift and the API give it: since you last looked, in counts and in lines", () => {
  const tree = tempTree();
  const log = new DriftLog().open(tree.path("cache/drift.json"));
  const [rollouts] = loadVendor("openai/codex/rollout-line");
  const r = new VendorReader("openai/codex/rollout-line", { log, majors: [rollouts] });
  r.check({ timestamp: T, type: "session_meta", payload: { cli_version: "0.150.0" } }, { version: "0.150.0" });
  log.record({ source: "openai/codex/rollout-line", kind: "new-field", path: "ordinal", shape: "number" }, 100);
  log.looked(150);
  log.record({ source: "openai/codex/rollout-line", vendorVersion: "0.151.0", kind: "missing-field", path: "payload.cli_version" }, 200);
  log.record({ source: "anthropic/claude-code/transcript-line", kind: "new-variant", path: "type", shape: '"progress"' }, 250);
  const report = driftReport({ log, readers: [r], home: tree.root, now: 300 });
  assert.deepEqual([report.at, report.file.replace(/\\/g, "/"), report.since, report.all, report.new, report.changes, report.known], [300, "~/cache/drift.json", 150, false, 2, 1, 3]);
  assert.equal(report.sources.length, vendorSources().length, "every source with a schema");

  const lines = reportLines(report);
  const at = (name) => lines.indexOf(lines.find((l) => l.startsWith(`  ${name}  `)));
  const codex = at("Codex's rollouts");
  assert.deepEqual(lines.slice(codex, codex + 3), [
    "  Codex's rollouts  vendor/openai/codex/rollout-line/v1 (<1.0.0)",
    "    read     1 record (1 checked whole), version 0.150.0",
    "    changed  payload.cli_version is missing  (since 0.151.0; 1 time)",
  ]);
  assert.equal(lines[at("Claude Code's transcripts") + 2], `    added    type "progress" isn't one the schema lists  (1 time)`);
  assert.equal(lines.at(-1), `  2 differences new since you last looked (${new Date(150).toLocaleString()}), 1 change to what XenoNet reads; 3 known in all`);
  assert.ok(lines.includes("    read     nothing this time"));

  // With nothing read here (xenonet drift, no XenoNet running), the sources
  // with nothing to say stay out.
  const quiet = reportLines(driftReport({ log, readers: [], now: 300 }), { unread: null, quiet: true });
  assert.deepEqual(quiet.filter((l) => /^ {2}\S/.test(l)).map((l) => l.split("  ")[1]), ["Claude Code's transcripts", "Codex's rollouts", "2 differences new since you last looked (" + new Date(150).toLocaleString() + "), 1 change to what XenoNet reads; 3 known in all"]);
  assert.ok(!quiet.some((l) => l.includes("read ")), "no read lines");
  const all = driftReport({ log, all: true, readers: [], now: 300 });
  assert.equal(all.sources.find((s) => s.source === "openai/codex/rollout-line").drift.length, 2);
  assert.equal(reportLines(all).at(-1), "  3 differences kept, 1 change to what XenoNet reads");
  assert.equal(all.new, 2, "new is always since you last looked");
});
