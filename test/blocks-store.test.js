// The stores (core/blocks/store.js and sqlite.js): versioned files that move
// up a step at a time, are written so a crash can't leave half of one, and
// never lose what they can't use.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { JsonStore, readVersioned } from "../core/blocks/store.js";
import { SqliteStore, migrateSqlite } from "../core/blocks/sqlite.js";
import { tempTree, cleanupAll } from "./harness/files.js";
import { fakeClock } from "./harness/clock.js";

after(cleanupAll);

// prefs.json: v1 had `colour`, v2 calls it `color`.
const MIGRATIONS = [{ from: 1, to: 2, up: ({ colour, ...rest }, { by }) => ({ ...rest, version: 2, color: colour ?? null, movedBy: by }) }];
const prefs = (tree, extra = {}) =>
  new JsonStore({ name: "prefs", file: tree.path("state/prefs.json"), version: 2, migrations: MIGRATIONS, context: () => ({ by: "test" }), prepare: (d) => (d.seen ??= {}), what: "your prefs", ...extra });

test("a new file: a fresh document, filled in, and saved a moment after a change", async () => {
  const tree = tempTree();
  const clock = fakeClock();
  const s = prefs(tree, { clock });
  assert.deepEqual(s.load(), { version: 2, seen: {} });
  assert.equal(s.load(), s.load(), "read once");
  assert.deepEqual(s.loadReport(), { file: tree.path("state/prefs.json"), version: 2, migrated: null, backup: null, broken: null, readOnly: false, problem: null });
  s.doc.seen.a = 1;
  s.save();
  s.doc.seen.b = 2;
  s.save();
  assert.equal(tree.exists("state/prefs.json"), false, "not yet");
  await clock.advance(200);
  assert.deepEqual(JSON.parse(tree.read("state/prefs.json")), { version: 2, seen: { a: 1, b: 2 } });
  assert.equal(tree.exists("state/prefs.json.tmp"), false);
  assert.deepEqual([s.counters.saves, s.state], [1, "running"]);
});

test("an older file is kept as it was, moved up, and saved; a newer one is read and never written", async () => {
  const tree = tempTree();
  const clock = fakeClock();
  tree.write("state/prefs.json", JSON.stringify({ colour: "teal", other: [1] }));
  const s = prefs(tree, { clock });
  assert.deepEqual(s.load(), { version: 2, other: [1], color: "teal", movedBy: "test", seen: {} });
  assert.deepEqual(s.report.migrated, { from: 1, to: 2 });
  assert.equal(tree.read("state/prefs.v1.json"), JSON.stringify({ colour: "teal", other: [1] }));
  await clock.advance(200);
  assert.equal(JSON.parse(tree.read("state/prefs.json")).version, 2, "saved without waiting for a change");

  const newer = tempTree();
  const v3 = JSON.stringify({ version: 3, color: "red" });
  newer.write("state/prefs.json", v3);
  const n = prefs(newer);
  assert.equal(n.load().color, "red");
  n.save();
  n.flush();
  assert.equal(newer.read("state/prefs.json"), v3, "untouched");
  assert.deepEqual([n.state, n.lastError.message], ["degraded", "prefs.json is from a newer XenoNet (version 3), so this one reads it but doesn't save changes to it"]);
});

test("a file that isn't JSON is moved aside, never written over; a failed save is said and tried again", () => {
  const tree = tempTree();
  tree.write("state/prefs.json", "{ not json");
  const s = prefs(tree);
  assert.deepEqual(s.load(), { version: 2, seen: {} });
  assert.match(path.basename(s.report.broken), /^prefs\.broken-.+\.json$/);
  assert.equal(fs.readFileSync(s.report.broken, "utf8"), "{ not json");
  assert.equal(s.report.problem, "prefs.json wasn't valid JSON, so XenoNet moved it aside and started afresh");
  assert.equal(s.state, "degraded", "until it has saved");

  // The folder moves to where a file is in the way.
  let dir = tree.path("state");
  const moving = new JsonStore({ name: "prefs", file: () => path.join(dir, "prefs.json"), version: 2, what: "your prefs" });
  moving.load();
  tree.write("blocker", "");
  dir = tree.path("blocker/state");
  const errors = [];
  const error = console.error;
  console.error = (...a) => errors.push(a.join(" "));
  try {
    moving.save();
    moving.flush();
  } finally {
    console.error = error;
  }
  assert.match(errors[0], /^\[xenonet\] couldn't save your prefs to .*blocker.state.prefs\.json: /);
  assert.equal(moving.state, "degraded");
  dir = tree.path("state");
  moving.flush();
  assert.equal(moving.state, "running", "saved on the next try");
  assert.equal(JSON.parse(tree.read("state/prefs.json")).version, 2);
});

test("a document prepare can't fill in isn't used or copied, at any start; one it mends says which parts", () => {
  const tree = tempTree();
  // A v1 file a hand edit left `seen` a string, which this prepare can't
  // fill in.
  const v1 = JSON.stringify({ colour: "teal", seen: "x" });
  tree.write("state/prefs.json", v1);
  const strict = () => prefs(tree, { prepare: (d) => ((d.seen ??= {}), (d.seen.count ??= 0)) });
  for (let start = 0; start < 2; start++) {
    const s = strict();
    assert.deepEqual(s.load(), { version: 2, seen: { count: 0 } }, "a fresh one in its place");
    assert.deepEqual([s.report.readOnly, s.report.migrated, s.report.backup, s.state], [true, null, null, "degraded"]);
    assert.equal(s.report.problem, "prefs.json has a part XenoNet can't use, so it leaves the file alone");
    s.save();
    s.flush();
  }
  assert.deepEqual([tree.files(), tree.read("state/prefs.json")], [["state/prefs.json"], v1], "no copy, and the file as it was");

  // A prepare that mends what it can: the parts it started afresh.
  const mended = tempTree();
  mended.write("state/prefs.json", JSON.stringify({ version: 2, seen: [1], color: "red" }));
  const s = prefs(mended, { prepare: (d) => (d.seen && typeof d.seen === "object" && !Array.isArray(d.seen) ? [] : ((d.seen = {}), ["seen"])) });
  assert.deepEqual(s.load(), { version: 2, seen: {}, color: "red" });
  assert.deepEqual([s.report.repaired, s.report.readOnly], [["seen"], false]);
});

test("readVersioned names what it reads by the file's own name", () => {
  const tree = tempTree();
  tree.write("layout.json", "[1, 2]");
  const r = readVersioned(tree.path("layout.json"), { version: 1 });
  assert.equal(r.report.problem, "layout.json wasn't a JSON object, so XenoNet moved it aside and started afresh");
  assert.deepEqual(r.doc, { version: 1 });
});

test("a SQLite database: steps taken and counted, a copy kept first when asked, and a newer one left alone", () => {
  const tree = tempTree();
  const steps = [(db) => db.exec("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"), (db) => db.exec("ALTER TABLE notes ADD COLUMN at INTEGER")];
  const file = tree.path("state/notes.sqlite");
  const fresh = new SqliteStore({ name: "notes", file, migrations: steps.slice(0, 1), pragmas: ["journal_mode = WAL"], backup: true });
  const db = fresh.open();
  assert.deepEqual(fresh.version, { from: 0, version: 1, newer: false });
  assert.equal(fresh.backupFile, null, "nothing to copy in a new database");
  db.prepare("INSERT INTO notes (body) VALUES (?)").run("hello");
  assert.equal(db.prepare("PRAGMA journal_mode").get().journal_mode, "wal");
  fresh.close();
  assert.equal(fresh.state, "stopped");

  const later = new SqliteStore({ name: "notes", file, migrations: steps, backup: true });
  later.open();
  assert.deepEqual(later.version, { from: 1, version: 2, newer: false });
  assert.equal(path.basename(later.backupFile), "notes.v1.sqlite");
  const copy = new DatabaseSync(later.backupFile, { readOnly: true });
  assert.deepEqual(copy.prepare("SELECT body FROM notes").all().map((r) => r.body), ["hello"]);
  assert.deepEqual(copy.prepare("PRAGMA table_info(notes)").all().map((c) => c.name), ["id", "body"], "as it was before the step");
  copy.close();
  later.close();

  const raw = new DatabaseSync(file);
  raw.exec("PRAGMA user_version = 9");
  raw.close();
  const newer = new SqliteStore({ name: "notes", file, migrations: steps });
  newer.open();
  assert.deepEqual(newer.version, { from: 9, version: 9, newer: true });
  assert.equal(newer.state, "degraded");
  newer.close();
});

test("a copy made before a migration is deleted once it's a week old, as the store opens or prunes", () => {
  const tree = tempTree();
  const clock = fakeClock("2026-10-05T12:00:00Z");
  const steps = [(db) => db.exec("CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"), (db) => db.exec("ALTER TABLE notes ADD COLUMN at INTEGER")];
  const file = tree.path("state/notes.sqlite");
  const first = new SqliteStore({ name: "notes", file, migrations: steps.slice(0, 1), clock });
  first.open().prepare("INSERT INTO notes (body) VALUES (?)").run("a made-up message");
  first.close();
  const later = new SqliteStore({ name: "notes", file, migrations: steps, backup: true, clock });
  later.open();
  const copy = later.backupFile;
  assert.equal(path.basename(copy), "notes.v1.sqlite");
  // A copy from another migration (its name was taken), and files that
  // aren't copies.
  const older = tree.write("state/notes.v1-2026-09-01T00-00-00.000Z.sqlite", "x");
  const others = ["state/notes.sqlite.v1", "state/other.v1.sqlite", "state/notes.v1.sqlite.txt"].map((rel) => tree.write(rel, "x"));
  const age = (f, days) => fs.utimesSync(f, new Date(clock.now() - days * 86400e3), new Date(clock.now() - days * 86400e3));
  age(copy, 6);
  age(older, 8);
  for (const f of others) age(f, 30);
  assert.deepEqual(later.pruneBackups(), [older]);
  assert.equal(later.backupFile, copy, "six days on, it's kept");
  later.close();

  age(copy, 7);
  const next = new SqliteStore({ name: "notes", file, migrations: steps, backup: true, clock });
  next.open();
  assert.equal(fs.existsSync(copy), false, "a week on, the next open deletes it");
  assert.deepEqual(tree.files(), ["state/notes.sqlite", "state/notes.sqlite.v1", "state/notes.v1.sqlite.txt", "state/other.v1.sqlite"]);
  next.close();
});

test("a step that fails takes nothing with it, and the store that tried has failed", () => {
  const tree = tempTree();
  const steps = [(db) => db.exec("CREATE TABLE a (x INTEGER)"), (db) => (db.exec("CREATE TABLE b (y INTEGER)"), db.exec("INSERT INTO nowhere VALUES (1)"))];
  const s = new SqliteStore({ name: "broken", file: tree.path("broken.sqlite"), migrations: steps });
  assert.throws(() => s.open(), /no such table: nowhere/);
  assert.deepEqual([s.state, s.lastError.message], ["failed", "no such table: nowhere"]);
  const db = new DatabaseSync(tree.path("broken.sqlite"));
  assert.equal(db.prepare("PRAGMA user_version").get().user_version, 1);
  assert.deepEqual(db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all().map((r) => r.name), ["a"]);
  assert.deepEqual(migrateSqlite(db, steps.slice(0, 1)), { from: 1, version: 1, newer: false });
  db.close();
});
