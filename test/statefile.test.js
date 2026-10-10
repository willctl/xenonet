// state.json's versions (core/statefile.js and core/migrate.js): moving a
// made-up v1 file up to v2, and every way a file can't be used, none of
// which loses it.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { migrate } from "../core/migrate.js";
import { STATE_VERSION, STATE_MIGRATIONS, migrateState, readStateFile, inspectState, versionOf } from "../core/statefile.js";
import { check, explain } from "../core/schema/index.js";

const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-statefile-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => (fs.chmodSync(dir, 0o755), fs.rmSync(dir, { recursive: true, force: true }))));

const T = 1791230000000;
const ROOT = "/Users/nova/Projects";
const PICKS = { ranAt: T, ms: 9000, fingerprint: "0123456789abcdef", sent: 12, picks: [{ path: "labs/snippets", as: "project", reason: "Its own pyproject.toml." }] };

// A v1 file as an earlier XenoNet wrote it, with every part.
const V1 = {
  layout: { core: { x: 1, y: 2, z: 3 } },
  acks: { "claude:0190d3e0-0000-7000-8000-000000000003": T },
  marks: {},
  projects: { rules: { "/users/nova/projects/labs": { path: `${ROOT}/labs`, as: "group", at: T } }, codex: PICKS },
  identities: { names: { "github:github.com/atlas-co": "work" }, hidden: {}, pins: {}, known: {}, signins: { claude: [{ id: "claude:a:b", from: T - 1000, to: T }] } },
  theme: "phosphor",
};

const write = (dir, data) => {
  const file = path.join(dir, "state.json");
  fs.writeFileSync(file, typeof data === "string" ? data : JSON.stringify(data, null, 2));
  return file;
};

test("migrations run in order from a document's version, and never backwards", () => {
  const steps = [
    { from: 1, to: 2, up: (d) => ({ ...d, two: true }) },
    { from: 2, to: 3, up: (d, ctx) => ({ ...d, three: ctx.n }) },
  ];
  const doc = { one: true };
  assert.deepEqual(migrate(doc, steps, { from: 1, to: 3, context: { n: 3 } }), { doc: { one: true, two: true, three: 3 }, applied: ["1→2", "2→3"] });
  assert.deepEqual(migrate(doc, steps, { from: 2, to: 2 }), { doc: { one: true }, applied: [] });
  assert.throws(() => migrate(doc, steps, { from: 3, to: 1 }), /can't go back/);
  assert.throws(() => migrate(doc, steps, { from: 1, to: 4 }), /no way from version 3 to 4/);
  assert.throws(() => migrate(doc, [{ from: 1, to: 3, up: (d) => d }], { from: 1, to: 2 }), /no way from version 1 to 2/, "a step that would overshoot");
  assert.throws(() => migrate(doc, steps, { from: "1", to: 3 }), /whole numbers/);
  // A step can't reach the caller's document.
  const mutating = [{ from: 1, to: 2, up: (d) => ((d.one = "changed"), d) }];
  migrate(doc, mutating, { from: 1, to: 2 });
  assert.equal(doc.one, true);
});

test("v1 to v2: a version, the workspace's root, and Codex's picks under the root they were made for", () => {
  assert.equal(STATE_VERSION, 2);
  assert.deepEqual(STATE_MIGRATIONS.map((m) => [m.from, m.to]), [[1, 2]]);
  const before = structuredClone(V1);
  const v2 = migrateState(V1, { projectsDir: ROOT });
  assert.deepEqual(V1, before, "the v1 document is left as it was");
  assert.equal(v2.version, 2);
  assert.deepEqual(v2.workspace, { root: null });
  assert.deepEqual(v2.projects, { rules: V1.projects.rules, codexByRoot: { "/users/nova/projects": PICKS } });
  for (const k of ["layout", "acks", "marks", "identities", "theme"]) assert.deepEqual(v2[k], V1[k], k);
  assert.deepEqual(Object.keys(v2).slice(0, 2), ["version", "workspace"], "the version first, for anyone reading the file");
  const fits = check("state/v2", v2, { strict: true });
  assert.ok(fits.ok, explain(fits));

  // An empty v1 file, one without picks, and parts v2 doesn't know.
  assert.deepEqual(migrateState({}, { projectsDir: ROOT }), { version: 2, workspace: { root: null }, projects: { codexByRoot: {} } });
  assert.deepEqual(migrateState({ projects: { codex: null, rules: {} }, extra: 1 }, { projectsDir: ROOT }), { version: 2, workspace: { root: null }, extra: 1, projects: { rules: {}, codexByRoot: {} } });
  // No projects folder to file the picks under: they can't be placed.
  assert.deepEqual(migrateState(V1).projects.codexByRoot, {});
  // A root, or picks per root, already there are kept.
  const early = migrateState({ workspace: { root: "/Users/nova/Code" }, projects: { codex: PICKS, codexByRoot: { "/users/nova/code": PICKS } } }, { projectsDir: ROOT });
  assert.deepEqual(early.workspace, { root: "/Users/nova/Code" });
  assert.deepEqual(Object.keys(early.projects.codexByRoot).sort(), ["/users/nova/code", "/users/nova/projects"]);
  assert.deepEqual(migrateState(v2), v2, "v2 stays as it is");
  assert.deepEqual([versionOf({}), versionOf({ version: 2 })], [1, 2]);
});

test("reading: a v1 file is kept beside it as it was, then moved up", () => {
  const dir = tmp();
  const text = JSON.stringify(V1, null, 2);
  const file = write(dir, text);
  const { state, report } = readStateFile(file, { projectsDir: ROOT, now: T });
  assert.equal(state.version, 2);
  assert.deepEqual(state.projects.codexByRoot, { "/users/nova/projects": PICKS });
  assert.deepEqual(report, { file, version: 1, migrated: { from: 1, to: 2 }, backup: path.join(dir, "state.v1.json"), broken: null, readOnly: false, problem: null });
  assert.equal(fs.readFileSync(report.backup, "utf8"), text, "byte for byte");
  assert.equal(fs.readFileSync(file, "utf8"), text, "reading writes nothing over the file; the store saves v2");

  // A second migration (an old file put back, say) never writes over the first copy.
  const again = readStateFile(file, { projectsDir: ROOT, now: T + 1000 });
  assert.equal(again.report.backup, path.join(dir, `state.v1-${new Date(T + 1000).toISOString().replace(/:/g, "-")}.json`));
  assert.equal(fs.readFileSync(path.join(dir, "state.v1.json"), "utf8"), text);
});

test("reading: no file, this version, and a version from a newer XenoNet", () => {
  const dir = tmp();
  const file = path.join(dir, "state.json");
  assert.deepEqual(readStateFile(file), { state: { version: 2 }, report: { file, version: 2, migrated: null, backup: null, broken: null, readOnly: false, problem: null } });

  const v2 = migrateState(V1, { projectsDir: ROOT });
  write(dir, v2);
  const current = readStateFile(file);
  assert.deepEqual(current.state, v2);
  assert.equal(current.report.readOnly, false);

  const v3 = { version: 3, layout: { a: [1, 2, 3] }, somethingNew: true };
  write(dir, v3);
  const newer = readStateFile(file);
  assert.deepEqual(newer.state, v3, "read as it is");
  assert.equal(newer.report.version, 3);
  assert.equal(newer.report.readOnly, true, "and never written back");
  assert.match(newer.report.problem, /from a newer XenoNet \(version 3\)/);
  assert.deepEqual(fs.readdirSync(dir), ["state.json"], "no copies of a newer file");
});

test("reading: a file that isn't JSON, isn't an object or has a version XenoNet doesn't know is moved aside", () => {
  for (const [text, why] of [
    ["{ not json", /wasn't valid JSON/],
    ["", /wasn't valid JSON/],
    ["[1, 2]", /wasn't a JSON object/],
    ["null", /wasn't a JSON object/],
    ['{"version": "2"}', /a version XenoNet doesn't know \("2"\)/],
    ['{"version": 0}', /a version XenoNet doesn't know \(0\)/],
  ]) {
    const dir = tmp();
    const file = write(dir, text);
    const { state, report } = readStateFile(file, { now: T });
    const aside = path.join(dir, `state.broken-${new Date(T).toISOString().replace(/:/g, "-")}.json`);
    assert.deepEqual(state, { version: 2 }, "afresh");
    assert.equal(report.broken, aside, text);
    assert.match(report.problem, why);
    assert.equal(report.readOnly, false, "a new state.json can be written");
    assert.equal(fs.readFileSync(aside, "utf8"), text, "kept as it was");
    assert.ok(!fs.existsSync(file));
  }
});

test("a file XenoNet can't read, move aside or keep a copy of is left alone", { skip: process.platform === "win32" || process.getuid?.() === 0 }, () => {
  // Can't be read.
  let dir = tmp();
  let file = write(dir, V1);
  fs.chmodSync(file, 0o000);
  let r = readStateFile(file);
  assert.deepEqual(r.state, { version: 2 });
  assert.equal(r.report.readOnly, true);
  assert.match(r.report.problem, /couldn't read state.json \(EACCES\)/);
  fs.chmodSync(file, 0o644);

  // Broken, in a folder XenoNet can't change.
  dir = tmp();
  file = write(dir, "{ not json");
  fs.chmodSync(dir, 0o555);
  r = readStateFile(file);
  fs.chmodSync(dir, 0o755);
  assert.equal(r.report.readOnly, true);
  assert.match(r.report.problem, /couldn't move it aside \(EACCES\)/);
  assert.equal(fs.readFileSync(file, "utf8"), "{ not json");

  // A v1 file with no room for its copy: migrated in memory only.
  dir = tmp();
  file = write(dir, V1);
  fs.chmodSync(dir, 0o555);
  r = readStateFile(file, { projectsDir: ROOT });
  fs.chmodSync(dir, 0o755);
  assert.equal(r.state.version, 2, "usable");
  assert.equal(r.report.readOnly, true, "but never saved over the only copy");
  assert.match(r.report.problem, /couldn't keep a copy of state.json \(EACCES\)/);
  assert.deepEqual(fs.readdirSync(dir), ["state.json"]);
});

test("what the doctor sees in the state folder", async () => {
  const dir = tmp();
  assert.deepEqual(await inspectState(dir), { file: path.join(dir, "state.json"), exists: false, known: 2, version: null, unreadable: null, issues: [], broken: [], backups: [] });
  write(dir, V1);
  readStateFile(path.join(dir, "state.json"), { projectsDir: ROOT, now: T });
  write(dir, "{ not json");
  readStateFile(path.join(dir, "state.json"), { now: T });
  write(dir, migrateState(V1, { projectsDir: ROOT }));
  let seen = await inspectState(dir);
  assert.deepEqual([seen.exists, seen.version, seen.unreadable, seen.issues], [true, 2, null, []]);
  assert.deepEqual(seen.backups.map((f) => path.basename(f)), ["state.v1.json"]);
  assert.deepEqual(seen.broken.map((f) => path.basename(f)), [`state.broken-${new Date(T).toISOString().replace(/:/g, "-")}.json`]);

  write(dir, { ...V1, layout: [1, 2], acks: { s: "yesterday" } });
  seen = await inspectState(dir);
  assert.equal(seen.version, 1);
  assert.deepEqual(seen.issues.map((i) => i.path), ["layout", "acks.s"], "checked against its own version's schema");
  write(dir, { version: 7 });
  assert.deepEqual([(await inspectState(dir)).version, (await inspectState(dir)).issues], [7, []], "a newer version isn't XenoNet's to check");
  write(dir, "nope");
  assert.equal((await inspectState(dir)).unreadable, "isn't valid JSON");
  write(dir, "[]");
  assert.equal((await inspectState(dir)).unreadable, "isn't a JSON object");
});
