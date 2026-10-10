// Drift records (core/drift.js): shapes without values, records merged and
// counted, at most so many kept, and drift.json shared by the processes
// that write it.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { DriftLog, shapeOf, variantShape, keyOf, describe, readDriftFile, cleanVersion, sourceName, isField } from "../core/drift.js";
import { fakeClock } from "./harness/clock.js";
import { tempTree, cleanupAll } from "./harness/files.js";
import { check } from "../core/schema/index.js";

after(cleanupAll);

const SOURCE = "anthropic/claude-code/transcript-line";

test("a shape has types, never values: keys sorted, arrays as their elements' types, at most four levels", () => {
  assert.equal(shapeOf("secret"), "string");
  assert.equal(shapeOf(42), "number");
  assert.equal(shapeOf(null), "null");
  assert.equal(shapeOf({ url: "https://example.com/x", type: "web" }), "object{type:string,url:string}");
  assert.equal(shapeOf([{ type: "a", url: "u" }, { type: "b", url: "v" }]), "array<object{type:string,url:string}>");
  assert.equal(shapeOf([1, "two", null]), "array<null|number|string>");
  assert.equal(shapeOf([]), "array<>");
  assert.equal(shapeOf({ a: { b: { c: { d: { e: 1 } } } } }), "object{a:object{b:object{c:object{d:object}}}}");
  assert.equal(shapeOf({ a: [[[[1]]]] }), "object{a:array<array<array<array>>>}");
});

test("keys that are data make a record: paths, ids, emails, long numbers", () => {
  assert.equal(shapeOf({ "/Users/ada/atlas/app.py": { type: "update" } }), "record<object{type:string}>");
  assert.equal(shapeOf({ "ada@example.com": 1 }), "record<number>");
  assert.equal(shapeOf({ "4f1c0000a1b2c3d4": true }), "record<boolean>");
  assert.equal(shapeOf({ user12345: 1 }), "record<number>");
  const many = Object.fromEntries(Array.from({ length: 40 }, (_, i) => [`k${i}`, i]));
  assert.equal(shapeOf(many), "record<number>");
  // Nothing of a value gets through, however it's nested.
  const deep = shapeOf({ list: [{ "/private/x": { email: "ada@example.com", token: "sk-123" } }] });
  assert.equal(deep, "object{list:array<record<object{email:string,token:string}>>}");
  assert.ok(!/ada|sk-|private/.test(deep));
});

// Made-up tokens and ids, in the shapes real ones have.
const TOKEN = "ghp_" + "Zx9kQ2mN7pL4vB8cR1tY6wE3uI5oA0sDfGhJ";
const NOT_FIELDS = [
  TOKEN,
  "gho_madeUpForTestsOnlyNotRealAtAll",
  "github_pat_11MADEUP0ONLY_forTestsHere",
  "glpat-madeUpForTestsOnly",
  "sk-ant-api03-madeUpForTestsOnly",
  "sk_live_madeUpForTests",
  "xoxb-madeup-for-tests",
  "AKIAMADEUPFORTESTS12",
  "AIzaMadeUpForTestsOnlyNotARealKey",
  "eyJhbGciOiJIUzI1NiJ9",
  "3f9a2c1",
  "abc1234def",
  "session_3f9a2c1d",
  "4f1c0000a1b2c3d4",
  "deadbeefdeadbeef",
  "user12345",
  "ada@example.com",
  "src/app.py",
  "C:\\Users",
  "a b",
  "x".repeat(49),
];

test("keys that read as data: tokens by their prefixes, hex ids and long numbers anywhere in a key, @, paths, spaces", () => {
  for (const k of NOT_FIELDS) assert.equal(isField(k), false, k);
  const fields = ["type", "cache_creation_input_tokens", "ephemeral_5m_input_tokens", "internal_chat_message_metadata_passthrough", "sha256", "x86_64", "oauth2Token", "addedBefore", "feedback", "decaf", "e2e", "skip", "ghost", "github", "npmrc", "eyes", "ASIA_PACIFIC"];
  for (const k of fields) assert.equal(isField(k), true, k);
  // In a shape, such a key makes its object a record of its values' types.
  const shape = shapeOf({ [TOKEN]: { scopes: "repo" }, user: "" });
  assert.equal(shape, "record<object{scopes:string}|string>");
  for (const k of NOT_FIELDS) assert.ok(!shapeOf({ [k]: 1, type: "x" }).includes(k.slice(0, 6)), k);
});

test("inside an object keyed by data, a shape keeps no key's name", () => {
  const users = { "ada-dev": { oauth_token: "" }, orbitworks: null };
  assert.equal(shapeOf(users), "object{ada-dev:object{oauth_token:string},orbitworks:null}", "a login can look like a field's name");
  assert.equal(shapeOf(users, { names: false }), "record<null|record<string>>");
  assert.equal(shapeOf([{ main: 1 }], { names: false }), "array<record<number>>");
});

test("a version is kept only when it reads as one", () => {
  for (const v of ["2.1.288", "0.155.0-alpha.9.2", "31", "5", "v1.2.3", "1.0.0-rc.1", "2.0.0-dev.20261005", "1.2.3+build.7", "0.3.0-nightly.3f9a2c1", "0.22.0rc1", "0.22.0.dev0", "1.0.0.post2", "1.0.0b3"]) assert.equal(cleanVersion(v), v, v);
  for (const v of ["not a version, really", "ada-dev", "ghp_madeUpForTests", "1.2.3-feature-login", "2.1.288 (Claude Code)", "1.2.3+ada", "1.2.3--rc", "1.2.", "", "1".repeat(65), 5, null]) assert.equal(cleanVersion(v), null, String(v));
});

test("a shape stays short, and a discriminator's value is shown only when it's a short name", () => {
  const wide = Object.fromEntries(Array.from({ length: 30 }, (_, i) => [`field_${String.fromCharCode(97 + (i % 26))}${i}`, { nested: { more: [1, 2, 3] } }]));
  assert.ok(shapeOf(wide).length < 520, "about 400 characters at most");
  assert.ok(shapeOf(wide).includes("…"));
  // Key names count too: 32 long ones and small values make no long shape.
  const named = Object.fromEntries(Array.from({ length: 32 }, (_, i) => [`a_rather_long_field_name_for_tests_${String.fromCharCode(97 + (i % 26))}${String.fromCharCode(97 + Math.floor(i / 26))}`, i]));
  assert.ok(shapeOf(named).length <= 440 && shapeOf(named).endsWith(",…}"), shapeOf(named));
  assert.equal(variantShape("attachment"), '"attachment"');
  assert.equal(variantShape("has spaces / and slashes"), "string");
  assert.equal(variantShape("x".repeat(80)), "string");
  assert.equal(variantShape(3), "3");
  assert.equal(variantShape({}), "object");
  assert.equal(cleanVersion("2.1.288"), "2.1.288");
  assert.equal(cleanVersion("0.155.0-alpha.9.2"), "0.155.0-alpha.9.2");
  assert.equal(cleanVersion("not a version, really"), null);
  assert.equal(sourceName("openai/codex/rollout-line"), "Codex's rollouts");
});

test("the same drift is one record, counted, from the version it was first seen in", () => {
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const log = new DriftLog({ clock });
  const t0 = clock.now();
  log.record({ source: SOURCE, vendorVersion: "2.1.286", kind: "new-field", path: "message.content[].citations", shape: "array<object{type:string}>" });
  clock.set(t0 + 1000);
  const r = log.record({ source: SOURCE, vendorVersion: "2.1.288", kind: "new-field", path: "message.content[].citations", shape: () => assert.fail("not needed for a record that exists") });
  assert.deepEqual(r, { schema: "drift/record/v1", source: SOURCE, vendorVersion: "2.1.286", kind: "new-field", path: "message.content[].citations", shape: "array<object{type:string}>", firstSeen: t0, lastSeen: t0 + 1000, count: 2 });

  // A type change by the type found, a new variant by its value, a new
  // version by the version, lines that aren't JSON by source.
  log.record({ source: SOURCE, kind: "type-change", path: "message.content", shape: () => "object{a:string}", type: "object" });
  log.record({ source: SOURCE, kind: "type-change", path: "message.content", shape: () => "record<string>", type: "object" });
  log.record({ source: SOURCE, kind: "type-change", path: "message.content", shape: "number" });
  log.record({ source: SOURCE, kind: "new-variant", path: "type", shape: '"attachment"' });
  log.record({ source: SOURCE, kind: "new-variant", path: "type", shape: '"progress"' });
  log.record({ source: SOURCE, kind: "new-version", vendorVersion: "3.0.0" });
  log.record({ source: SOURCE, kind: "new-version", vendorVersion: "3.0.1" });
  log.record({ source: SOURCE, kind: "unparsable" });
  log.record({ source: SOURCE, kind: "unparsable" });
  log.record({ source: "openai/codex/rollout-line", kind: "unparsable" });
  const counts = log.list().map((x) => [x.kind, x.path, x.shape ?? x.vendorVersion, x.count]);
  assert.deepEqual(counts, [
    ["new-field", "message.content[].citations", "array<object{type:string}>", 2],
    ["type-change", "message.content", "object{a:string}", 2],
    ["type-change", "message.content", "number", 1],
    ["new-variant", "type", '"attachment"', 1],
    ["new-variant", "type", '"progress"', 1],
    ["new-version", null, "3.0.0", 1],
    ["new-version", null, "3.0.1", 1],
    ["unparsable", null, null, 2],
    ["unparsable", null, null, 1],
  ]);
  assert.equal(keyOf(log.list()[1]), keyOf({ source: SOURCE, kind: "type-change", path: "message.content", type: "object" }), "a stored record keys the same as a new one");
  assert.deepEqual(log.list().map(describe).slice(0, 4), ["message.content[].citations is new (array<object{type:string}>)", "message.content is object{a:string} now", "message.content is number now", 'type "attachment" isn\'t one the schema lists']);
});

test("drift in a kind of record says which, and is a record of its own there", () => {
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const log = new DriftLog({ clock });
  const CODEX = "openai/codex/rollout-line";
  const user = log.record({ source: CODEX, kind: "missing-field", path: "payload.message", variant: ["event_msg", "user_message"] });
  log.record({ source: CODEX, kind: "missing-field", path: "payload.message", variant: ["event_msg", "agent_message"] });
  log.record({ source: CODEX, kind: "missing-field", path: "payload.message", variant: ["event_msg", "user_message"] });
  log.record({ source: CODEX, kind: "type-change", path: "payload.item.content", variant: ["event_msg", "item_completed", "UserMessage"], type: "string", shape: () => "string" });
  log.record({ source: CODEX, kind: "new-variant", path: "type", variant: [], shape: '"world_view"' });
  assert.deepEqual(log.list().map((r) => [r.path, r.variant, r.count]), [
    ["payload.message", ["event_msg", "user_message"], 2],
    ["payload.message", ["event_msg", "agent_message"], 1],
    ["payload.item.content", ["event_msg", "item_completed", "UserMessage"], 1],
    ["type", undefined, 1],
  ]);
  assert.ok(!("variant" in log.list()[3]), "none where there's none");
  assert.equal(keyOf(user), keyOf({ source: CODEX, kind: "missing-field", path: "payload.message", variant: ["event_msg", "user_message"] }));
  assert.notEqual(keyOf(user), keyOf({ source: CODEX, kind: "missing-field", path: "payload.message" }));
  assert.deepEqual(log.list().map(describe), [
    "payload.message is missing, in event_msg › user_message",
    "payload.message is missing, in event_msg › agent_message",
    "payload.item.content is string now, in event_msg › item_completed › UserMessage",
    'type "world_view" isn\'t one the schema lists',
  ]);
});

test("a record from before records said their variant is taken over, history and all, so it isn't new again", async () => {
  const tree = tempTree();
  const file = tree.path("cache/drift.json");
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const t0 = clock.now();
  const CODEX = "openai/codex/rollout-line";
  // drift.json as an older XenoNet kept it, looked at since.
  const before = { schema: "drift/record/v1", source: CODEX, vendorVersion: "0.150.0", kind: "missing-field", path: "payload.cli_version", shape: null, firstSeen: t0 - 3600e3, lastSeen: t0 - 60e3, count: 7 };
  const changed = { ...before, kind: "type-change", path: "payload.info", shape: "string", count: 2 };
  fs.mkdirSync(tree.path("cache"));
  fs.writeFileSync(file, JSON.stringify({ version: 1, lookedAt: t0 - 1000, records: [before, changed] }));
  const server = new DriftLog({ clock }).open(file);
  const other = new DriftLog({ clock }).open(file);
  const r = server.record({ source: CODEX, vendorVersion: "0.152.0", kind: "missing-field", path: "payload.cli_version", variant: ["session_meta"] });
  assert.deepEqual(r, { ...before, variant: ["session_meta"], lastSeen: t0, count: 8 }, "the old one's history, in its variant");
  assert.deepEqual(server.since(server.lookedAt), [], "nothing new since you looked");
  // A type change keyed by the type found takes over the same way.
  server.record({ source: CODEX, kind: "type-change", path: "payload.info", variant: ["event_msg", "token_count"], type: "string", shape: () => "string" });
  assert.deepEqual(server.list().map((x) => [x.path, x.variant, x.count]), [["payload.cli_version", ["session_meta"], 8], ["payload.info", ["event_msg", "token_count"], 3]]);
  server.flush();
  assert.deepEqual(readDriftFile(file).records.map((x) => [x.path, x.variant, x.count]), [["payload.cli_version", ["session_meta"], 8], ["payload.info", ["event_msg", "token_count"], 3]], "the old ones are gone from the file");

  // Another process that read the old file drops its old records when it
  // sees the new ones, and its own counts add to theirs.
  clock.set(t0 + 10e3);
  other.record({ source: CODEX, kind: "missing-field", path: "payload.cli_version", variant: ["session_meta"] });
  other.flush();
  const byPath = (records) => records.map((x) => [x.path, x.variant, x.count, x.firstSeen]).sort(([a], [b]) => a.localeCompare(b));
  assert.deepEqual(byPath(readDriftFile(file).records), [["payload.cli_version", ["session_meta"], 9, before.firstSeen], ["payload.info", ["event_msg", "token_count"], 3, before.firstSeen]]);
  server.flush();
  assert.equal(server.view().records.length, 2, "and the first doesn't bring them back");
  // A variant that isn't a list of values is read as none.
  fs.writeFileSync(file, JSON.stringify({ version: 1, lookedAt: null, records: [{ ...before, variant: "session_meta" }, { ...before, path: "x", variant: [{}] }] }));
  assert.deepEqual(readDriftFile(file).records.map((x) => "variant" in x), [false, false]);
});

test("at most `max` records: the ones seen least lately go first", () => {
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const log = new DriftLog({ clock, max: 3 });
  for (const p of ["a", "b", "c"]) {
    log.record({ source: SOURCE, kind: "new-field", path: p, shape: "string" });
    clock.set(clock.now() + 1000);
  }
  log.record({ source: SOURCE, kind: "new-field", path: "a", shape: "string" }); // seen again
  clock.set(clock.now() + 1000);
  log.record({ source: SOURCE, kind: "new-field", path: "d", shape: "string" });
  assert.deepEqual(log.list().map((r) => r.path), ["a", "c", "d"]);
  assert.deepEqual(log.since(log.list()[1].firstSeen).map((r) => r.path), ["d"]);
});

test("drift.json: written a moment later, merged with what another process wrote, when you looked only moving forward", async () => {
  const tree = tempTree();
  const file = tree.path("cache/drift.json");
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const server = new DriftLog({ clock }).open(file);
  server.record({ source: SOURCE, kind: "new-field", path: "a", shape: "string" });
  server.record({ source: SOURCE, kind: "new-field", path: "a", shape: "string" });
  assert.ok(!fs.existsSync(file), "not yet");
  await clock.advance(5000);
  assert.deepEqual(readDriftFile(file).records.map((r) => [r.path, r.count]), [["a", 2]]);
  const raw = JSON.parse(fs.readFileSync(file, "utf8"));
  assert.deepEqual([raw.version, raw.lookedAt, raw.records[0].schema], [1, null, "drift/record/v1"]);
  assert.deepEqual(check("drift/log/v1", raw, { strict: true }).issues, [], "a drift/log/v1");

  // npm run contracts, meanwhile: its own reading, and you looked.
  const report = new DriftLog({ clock }).open(file);
  report.record({ source: SOURCE, kind: "new-field", path: "a", shape: "string" });
  report.record({ source: SOURCE, kind: "missing-field", path: "timestamp" });
  report.looked(clock.now());
  report.flush();

  // The server writes again: its new count adds to theirs, and the look
  // stays.
  clock.set(clock.now() + 60e3);
  server.record({ source: SOURCE, kind: "new-field", path: "a", shape: "string" });
  server.flush();
  const after = readDriftFile(file);
  assert.deepEqual(after.records.map((r) => [r.kind, r.path, r.count]), [["new-field", "a", 4], ["missing-field", "timestamp", 1]]);
  assert.equal(after.lookedAt, Date.parse("2026-10-05T12:00:05Z"));
  assert.ok(check("drift/log/v1", JSON.parse(fs.readFileSync(file, "utf8")), { strict: true }).ok, "looked at, still a drift/log/v1");
  assert.equal(server.view().lookedAt, after.lookedAt, "the server sees when you looked");
  assert.deepEqual(server.since(server.view().lookedAt), [], "nothing new since");
  server.record({ source: SOURCE, kind: "new-variant", path: "type", shape: '"attachment"' });
  assert.deepEqual(server.since(server.lookedAt).map((r) => r.kind), ["new-variant"]);
});

test("a drift.json that isn't one is no drift, and its odd records are left out", () => {
  const tree = tempTree();
  assert.equal(readDriftFile(tree.path("missing.json")), null);
  assert.equal(readDriftFile(tree.write("broken.json", "{ half")), null);
  assert.equal(readDriftFile(tree.write("array.json", "[]")), null);
  const good = { schema: "drift/record/v1", source: SOURCE, vendorVersion: null, kind: "unparsable", path: null, shape: null, firstSeen: 1, lastSeen: 2, count: 3 };
  const mixed = tree.write("mixed.json", JSON.stringify({ version: 1, lookedAt: "soon", records: [good, { ...good, kind: "mystery" }, { ...good, count: 0 }, null] }));
  assert.deepEqual(readDriftFile(mixed), { file: mixed, lookedAt: null, records: [good] });
  // A log opened on a broken file starts afresh and writes over it.
  const log = new DriftLog().open(tree.path("broken.json"));
  log.record({ source: SOURCE, kind: "unparsable" });
  log.flush();
  assert.equal(readDriftFile(tree.path("broken.json")).records.length, 1);
});

test("npm run contracts reports drift without marking it seen: only xenonet drift and the console do, since agents run it too", () => {
  const script = fs.readFileSync(new URL("../scripts/check-contracts.mjs", import.meta.url), "utf8");
  assert.doesNotMatch(script, /\.looked\(/, "it never says you've looked");
  assert.match(script, /driftReport\(/, "it still reports it");
  assert.match(script, /drift\.flush\(\)/, "and keeps what its run found");
});

test("npm run contracts keeps its drift where the XenoNet it runs as reads it, and builds a search index apart from that XenoNet's", () => {
  // Run from a checkout, that's XenoNet Dev: its drift.json is the one xenonet
  // drift and its doctor read. Its search index has its server writing it.
  const script = fs.readFileSync(new URL("../scripts/check-contracts.mjs", import.meta.url), "utf8");
  assert.match(script, /const cacheDir = own \? instanceDir\("cache", instance\.name\) : config\.cacheDir;\ndrift\.open\(path\.join\(cacheDir, DRIFT_FILE\)\);/);
  assert.match(script, /const scratch = fs\.mkdtempSync\(path\.join\(os\.tmpdir\(\), "xenonet-contracts-"\)\);[^]*dbFile: path\.join\(scratch, "search\.sqlite"\)[^]*fs\.rmSync\(scratch, \{ recursive: true, force: true \}\)/);
  assert.doesNotMatch(script, /config\.cacheDir, /, "nothing kept in another XenoNet's cache folder");
});
