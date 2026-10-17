// The scanner (core/blocks/scanner.js): a walk for the files wanted, within
// budgets of folders, depth and time, reading only the folders whose mtime
// changed since the last walk; depth first and synchronous, or breadth
// first and asynchronous, with each folder's entries handed to look.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { Scanner } from "../core/blocks/scanner.js";
import { tempTree, cleanupAll } from "./harness/files.js";

after(cleanupAll);

const T = Date.parse("2026-10-05T12:00:00Z");
const rel = (tree, files) => files.map((f) => path.relative(tree.root, f).split(path.sep).join("/"));
const jsonl = (name) => name.endsWith(".jsonl");

// projects/-a/{s1.jsonl, notes.txt, s1/subagents/agent-x.jsonl}, projects/-b/s2.jsonl
function world() {
  const tree = tempTree();
  tree.write("projects/-a/s1.jsonl", "{}\n");
  tree.write("projects/-a/notes.txt", "");
  tree.write("projects/-a/s1/subagents/agent-x.jsonl", "{}\n");
  tree.write("projects/-b/s2.jsonl", "{}\n");
  for (const d of ["projects/-a/s1/subagents", "projects/-a/s1", "projects/-a", "projects/-b", "projects"]) tree.touch(d, T);
  return tree;
}

test("the first walk finds every file wanted, a folder's own before its subfolders'", () => {
  const tree = world();
  const s = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl });
  const found = rel(tree, s.scan());
  assert.deepEqual([...found].sort(), ["projects/-a/s1.jsonl", "projects/-a/s1/subagents/agent-x.jsonl", "projects/-b/s2.jsonl"]);
  assert.ok(found.indexOf("projects/-a/s1.jsonl") < found.indexOf("projects/-a/s1/subagents/agent-x.jsonl"));
  assert.deepEqual(s.last, { files: 3, read: 5, seen: 5, truncated: null, ms: s.last.ms });
  assert.equal(s.state, "running");
  assert.deepEqual([s.counters.scans, s.counters.read], [1, 5]);
});

test("later walks read only the folders that changed, and forget the ones that went", () => {
  const tree = world();
  const s = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl });
  s.scan();
  assert.deepEqual(s.scan(), [], "nothing changed: nothing read");
  assert.deepEqual(s.last.read, 0);
  assert.equal(s.last.seen, 5, "but every folder is looked at");

  tree.write("projects/-b/s3.jsonl", "{}\n");
  tree.touch("projects/-b", T + 1000);
  assert.deepEqual(rel(tree, s.scan()).sort(), ["projects/-b/s2.jsonl", "projects/-b/s3.jsonl"], "the files of the folder that changed");

  tree.write("projects/-c/new/s4.jsonl", "{}\n");
  tree.touch("projects", T + 2000);
  assert.deepEqual(rel(tree, s.scan()), ["projects/-c/new/s4.jsonl"], "a new folder, all the way down");

  tree.remove("projects/-a/s1");
  tree.touch("projects/-a", T + 3000);
  s.scan();
  assert.ok(![...s.folders.keys()].some((k) => k.includes(`${path.sep}s1`)), "the folder that went is forgotten, and what was under it");
  s.forget();
  assert.equal(s.folders.size, 0);
  assert.equal(rel(tree, s.scan()).length, 4, "forgotten: everything read again");
});

// Linux's coarse timestamps: a file added in the same tick of the
// filesystem's clock as the folder's last change leaves its mtime as it was.
test("a folder read just after it changed is read again, so a file its mtime doesn't show is still found", async () => {
  for (const walk of [(s) => s.scan(), (s) => s.scanBreadth()]) {
    const tree = world();
    let now = T + 5000;
    const s = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl, clock: { now: () => now } });
    await walk(s);
    tree.write("projects/-b/s3.jsonl", "{}\n");
    tree.touch("projects/-b", T + 4500);
    assert.deepEqual(rel(tree, await walk(s)).sort(), ["projects/-b/s2.jsonl", "projects/-b/s3.jsonl"]);
    tree.write("projects/-b/s4.jsonl", "{}\n");
    tree.touch("projects/-b", T + 4500);
    assert.deepEqual(rel(tree, await walk(s)).sort(), ["projects/-b/s2.jsonl", "projects/-b/s3.jsonl", "projects/-b/s4.jsonl"], "the same mtime, read again");
    now = T + 9000;
    await walk(s);
    assert.deepEqual(await walk(s), [], "read long enough after its change: trusted again");
  }
});

test("depth, the folders it enters, and a walk without the cache", () => {
  const tree = world();
  const shallow = new Scanner({ name: "t", root: tree.path("projects"), depth: 1, pick: jsonl });
  assert.deepEqual(rel(tree, shallow.scan()).sort(), ["projects/-a/s1.jsonl", "projects/-b/s2.jsonl"]);
  const choosy = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl, enter: (name) => name !== "-b" });
  assert.ok(!rel(tree, choosy.scan()).includes("projects/-b/s2.jsonl"));
  const uncached = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl, cache: false });
  uncached.scan();
  assert.equal(uncached.scan().length, 3, "every folder, every time");
});

test("budgets: a walk stops at its folders or its time, says so, and the next walks go on", () => {
  const tree = world();
  const s = new Scanner({ name: "t", root: tree.path("projects"), folders: 2, pick: jsonl });
  const first = rel(tree, s.scan());
  assert.equal(s.last.truncated, "folders");
  assert.equal(s.last.read, 2);
  assert.deepEqual([s.state, s.lastError.message], ["degraded", "stopped at the 2-folder budget"]);
  assert.deepEqual(s.health().budget, { depth: Infinity, folders: 2, ms: Infinity, last: s.last });
  const all = new Set(first);
  for (let i = 0; i < 4; i++) for (const f of rel(tree, s.scan())) all.add(f);
  assert.equal(all.size, 3, "the walks after it read the rest");
  assert.equal(s.state, "running", "and a walk that finishes is fine again");

  // A clock that moves 10 ms each time it's read.
  let now = T;
  const slow = new Scanner({ name: "t", root: tree.path("projects"), ms: 25, pick: jsonl, clock: { now: () => (now += 10) } });
  slow.scan();
  assert.equal(slow.last.truncated, "time");
  assert.equal(slow.lastError.message, "stopped after 25 ms");
});

test("a folder that can't be read is passed over, and the scanner says so", { skip: process.platform === "win32" || process.getuid?.() === 0 }, () => {
  const tree = world();
  const locked = tree.path("projects/-b");
  fs.chmodSync(locked, 0o000);
  try {
    const s = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl });
    assert.ok(!rel(tree, s.scan()).includes("projects/-b/s2.jsonl"));
    assert.equal(s.state, "degraded");
    assert.match(s.lastError.message, /couldn't read .*-b \(EACCES\)/);
  } finally {
    fs.chmodSync(locked, 0o755);
  }
  const gone = new Scanner({ name: "t", root: tree.path("nowhere") });
  assert.deepEqual(gone.scan(), []);
  assert.equal(gone.state, "running", "a root that isn't there holds nothing, and that's no problem");
});

// ---------- breadth first ----------

// The file system, with each folder's entries in name order, so which
// folders a budget reaches doesn't depend on the disk's own order.
const inOrder = { ...fs, promises: { ...fs.promises, readdir: async (dir, opts) => (await fs.promises.readdir(dir, opts)).sort((a, b) => (a.name < b.name ? -1 : 1)) } };

// a/{x,y}, b/{x,y}, c/{x,y}, with a file in each folder.
function levels() {
  const tree = tempTree();
  for (const top of ["a", "b", "c"]) for (const sub of ["x", "y"]) tree.write(`${top}/${sub}/f.txt`, "");
  for (const top of ["a", "b", "c"]) tree.write(`${top}/f.txt`, "");
  return tree;
}

test("breadth first: the root, then a level at a time, each folder looked at before its subfolders are entered", async () => {
  const tree = world();
  const looked = [];
  const asked = [];
  const s = new Scanner({
    name: "t",
    root: tree.path("projects"),
    pick: jsonl,
    cache: false,
    look: async (dir, entries, level) => {
      await new Promise((r) => setImmediate(r));
      looked.push([rel(tree, [dir])[0], level, entries.map((e) => e.name).sort().join(",")]);
    },
    enter: (name, dir, level) => {
      asked.push([name, level, looked.some(([d]) => d === rel(tree, [dir])[0])]);
      return true;
    },
  });
  const found = rel(tree, await s.scanBreadth());
  assert.deepEqual(found.slice(0, 2).sort(), ["projects/-a/s1.jsonl", "projects/-b/s2.jsonl"], "the shallow files first");
  assert.equal(found[2], "projects/-a/s1/subagents/agent-x.jsonl");
  assert.deepEqual(looked.map(([d, level]) => [d, level]).sort((x, y) => x[1] - y[1] || (x[0] < y[0] ? -1 : 1)), [["projects", 0], ["projects/-a", 1], ["projects/-b", 1], ["projects/-a/s1", 2], ["projects/-a/s1/subagents", 3]]);
  assert.deepEqual(looked.map(([, level]) => level), [...looked.map(([, level]) => level)].sort(), "a level at a time");
  assert.ok(asked.every(([, , after]) => after), "enter is asked once look is done with the folder");
  assert.deepEqual(s.last, { files: 3, read: 4, seen: 4, truncated: null, ms: s.last.ms }, "the root is read, and not counted");
  assert.deepEqual([s.state, s.counters.scans, s.counters.read], ["running", 1, 4]);
});

test("breadth first: the budget takes each level's first folders, in order, and the walk stops after the level it ran out on", async () => {
  const tree = levels();
  const walk = async (opts) => {
    const looked = [];
    const s = new Scanner({ name: "t", root: tree.root, cache: false, fs: inOrder, pick: () => true, look: (dir) => looked.push(rel(tree, [dir])[0]), ...opts });
    const files = rel(tree, await s.scanBreadth());
    // Folders are read a few at once, so look is handed them as they come.
    return { s, files, looked: looked.filter(Boolean).sort() };
  };
  const inSecond = await walk({ folders: 4 });
  assert.deepEqual(inSecond.looked, ["a", "a/x", "b", "c"]);
  assert.deepEqual(inSecond.files, ["a/f.txt", "b/f.txt", "c/f.txt", "a/x/f.txt"]);
  assert.deepEqual([inSecond.s.last.read, inSecond.s.last.truncated, inSecond.s.state, inSecond.s.lastError.message], [4, "folders", "degraded", "stopped at the 4-folder budget"]);
  const inFirst = await walk({ folders: 2 });
  assert.deepEqual([inFirst.looked, inFirst.s.last.truncated], [["a", "b"], "folders"]);
  const atEnd = await walk({ folders: 3 });
  assert.deepEqual([atEnd.looked, atEnd.s.last.truncated], [["a", "b", "c"], "folders"], "the next level had folders to read");
  const shallow = await walk({ folders: 3, depth: 1 });
  assert.deepEqual([shallow.looked, shallow.s.last.truncated, shallow.s.state], [["a", "b", "c"], null, "running"], "and at the depth limit it had none");
  const all = await walk({ folders: 9 });
  assert.deepEqual([all.looked.length, all.s.last.read, all.s.last.truncated], [9, 9, null]);
});

test("breadth first: a few folders read at once, and no more", async () => {
  const tree = tempTree();
  for (let i = 0; i < 10; i++) tree.mkdir(`d${i}`);
  let open = 0;
  let most = 0;
  const slow = {
    ...fs,
    promises: {
      ...fs.promises,
      readdir: async (dir, opts) => {
        most = Math.max(most, ++open);
        await new Promise((r) => setTimeout(r, 5));
        open--;
        return fs.promises.readdir(dir, opts);
      },
    },
  };
  const s = new Scanner({ name: "t", root: tree.root, cache: false, fs: slow, concurrency: 3 });
  await s.scanBreadth();
  assert.equal(most, 3);
  assert.equal(s.last.read, 10);
});

test("breadth first: a walk's own root, limits and rules, in place of the scanner's", async () => {
  const one = levels();
  const other = world();
  const s = new Scanner({ name: "t", root: one.root, cache: false, pick: () => true });
  assert.equal((await s.scanBreadth()).length, 9);
  const found = rel(other, await s.scanBreadth({ root: other.path("projects"), depth: 1, pick: jsonl, enter: (name) => name !== "-b", folders: 5 }));
  assert.deepEqual(found, ["projects/-a/s1.jsonl"]);
  assert.deepEqual(s.health().budget, { depth: 1, folders: 5, ms: Infinity, last: s.last }, "health shows what the last walk was allowed");
});

test("breadth first, with the cache: only the folders that changed are read, and the ones that went are forgotten", async () => {
  const tree = world();
  const looked = [];
  const s = new Scanner({ name: "t", root: tree.path("projects"), pick: jsonl, look: (dir) => looked.push(rel(tree, [dir])[0]) });
  assert.equal((await s.scanBreadth()).length, 3);
  looked.length = 0;
  assert.deepEqual(await s.scanBreadth(), [], "nothing changed: nothing read");
  assert.deepEqual([s.last.read, s.last.seen, looked.length], [0, 4, 0]);
  tree.write("projects/-b/s3.jsonl", "{}\n");
  tree.touch("projects/-b", T + 1000);
  assert.deepEqual(rel(tree, await s.scanBreadth()).sort(), ["projects/-b/s2.jsonl", "projects/-b/s3.jsonl"]);
  assert.deepEqual(looked, ["projects/-b"]);
  tree.remove("projects/-a/s1");
  tree.touch("projects/-a", T + 2000);
  await s.scanBreadth();
  assert.ok(![...s.folders.keys()].some((k) => k.includes(`${path.sep}s1`)), "the folder that went is forgotten, and what was under it");
  const uncached = new Scanner({ name: "t", root: tree.path("projects"), cache: false });
  await uncached.scanBreadth();
  assert.equal(uncached.folders.size, 0, "without the cache, nothing is kept");
});

test("breadth first: a folder that can't be read, or that look can't take, is passed over, and the scanner says so", { skip: process.platform === "win32" || process.getuid?.() === 0 }, async () => {
  const tree = world();
  const locked = tree.path("projects/-b");
  fs.chmodSync(locked, 0o000);
  try {
    const looked = [];
    const s = new Scanner({ name: "t", root: tree.path("projects"), cache: false, pick: jsonl, look: (dir) => looked.push(rel(tree, [dir])[0]) });
    const found = rel(tree, await s.scanBreadth());
    assert.ok(!found.includes("projects/-b/s2.jsonl"));
    assert.ok(!looked.includes("projects/-b"), "look isn't handed what couldn't be read");
    assert.equal(s.last.read, 4, "it counts all the same");
    assert.equal(s.state, "degraded");
    assert.match(s.lastError.message, /couldn't read .*-b \(EACCES\)/);
  } finally {
    fs.chmodSync(locked, 0o755);
  }
  const picky = new Scanner({ name: "t", root: tree.path("projects"), cache: false, pick: jsonl, look: (dir) => { if (dir.endsWith("-a")) throw new Error("no"); } });
  assert.deepEqual(rel(tree, await picky.scanBreadth()), ["projects/-b/s2.jsonl"], "nothing under the folder look couldn't take");
  assert.match(picky.lastError.message, /couldn't look at .*-a \(no\)/);
  const gone = new Scanner({ name: "t", root: tree.path("nowhere") });
  assert.deepEqual(await gone.scanBreadth(), []);
  assert.equal(gone.state, "running", "a root that isn't there holds nothing");
});

test("breadth first: the time budget", async () => {
  const tree = levels();
  // A clock that moves 10 ms each time it's read.
  let now = T;
  const s = new Scanner({ name: "t", root: tree.root, cache: false, ms: 25, concurrency: 1, clock: { now: () => (now += 10) } });
  await s.scanBreadth();
  assert.equal(s.last.truncated, "time");
  assert.equal(s.lastError.message, "stopped after 25 ms");
  assert.ok(s.last.read < 9);
});

test("breadth first: a walk whose signal aborts reads no folder after it, and isn't the scanner's problem", async () => {
  const tree = levels();
  const stop = new AbortController();
  const looked = [];
  const s = new Scanner({ name: "t", root: tree.root, cache: false, concurrency: 1 });
  // No longer wanted once the second folder below the root is looked at.
  await s.scanBreadth({ signal: stop.signal, look: (dir, _entries, level) => (looked.push(dir), level === 1 && looked.length === 3 && stop.abort()) });
  assert.deepEqual([looked.length, s.last.read, s.last.truncated], [3, 2, "cancelled"]);
  assert.deepEqual([s.state, s.lastError], ["idle", null], "neither degraded nor running for it");
  // The next walk, with no signal, reads them all.
  await s.scanBreadth();
  assert.deepEqual([s.last.read, s.last.truncated, s.state], [9, null, "running"]);
});
