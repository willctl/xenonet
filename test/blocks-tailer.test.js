// The tailer (core/blocks/tailer.js): files read a line at a time as they
// grow, learned of by watch events, sweeps and walks of the folders, with
// the clock and the watch the test's own.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { Tailer } from "../core/blocks/tailer.js";
import { tempTree, fakeWatch, cleanupAll } from "./harness/files.js";
import { fakeClock } from "./harness/clock.js";

after(cleanupAll);

// A tailer over a tree's `logs` folder, with what it hands on kept.
function setup({ files = {}, now = Date.now() } = {}) {
  const tree = tempTree();
  for (const [rel, [text, mtime]] of Object.entries(files)) tree.write(rel, text, { mtime });
  tree.mkdir("logs");
  const clock = fakeClock(now);
  const watcher = fakeWatch();
  const got = [];
  const fresh = [];
  const order = [];
  const tailer = new Tailer({
    name: "logs",
    roots: [{ dir: tree.path("logs") }],
    classify: (file) => (/\.jsonl$/.test(file) && !file.includes("skip") ? { name: path.basename(file) } : null),
    onLine: (line, entry, prime) => got.push(`${entry.name}:${prime ? "history" : "live"}:${line}`),
    onNewFile: (file, entry) => fresh.push(entry.name),
    onPrimed: () => order.push("primed"),
    watch: (...args) => (order.push("watch"), watcher.watch(...args)),
    clock,
  });
  const settle = async () => {
    await clock.flush();
    await tailer.idle();
  };
  return { tree, clock, watcher, tailer, got, fresh, order, settle, take: () => got.splice(0) };
}

test("start reads what was written lately as history, leaves older files dormant, then watches", async () => {
  const now = Date.parse("2026-10-05T12:00:00Z");
  const t = setup({
    now,
    files: {
      "logs/a/one.jsonl": ['{"n":1}\n\n{"n":2}\n', now - 3600e3],
      "logs/b/old.jsonl": ['{"n":"old"}\n', now - 3 * 86400e3],
      "logs/b/skip.jsonl": ["ignored\n", now],
      "logs/c.txt": ["not picked\n", now],
    },
  });
  await t.tailer.start();
  assert.deepEqual(t.take(), ['one.jsonl:history:{"n":1}', 'one.jsonl:history:{"n":2}'], "blank lines are passed over");
  assert.deepEqual(t.order, ["primed", "watch"]);
  assert.deepEqual([...t.tailer.dormant.keys()].map((f) => path.basename(f)), ["old.jsonl"]);
  assert.deepEqual([t.tailer.status.watching, t.tailer.status.files, t.tailer.state], [true, 1, "running"]);

  // The old one is written to again: what it held is history, then what's new.
  t.tree.append("logs/b/old.jsonl", '{"n":"again"}\n');
  t.watcher.emit(t.tree.path("logs/b/old.jsonl"));
  await t.clock.advance(30);
  await t.settle();
  assert.deepEqual(t.take(), ['old.jsonl:history:{"n":"old"}', 'old.jsonl:live:{"n":"again"}']);
  assert.deepEqual(t.fresh, ["old.jsonl"]);
  t.tailer.stop();
});

test("watch events: a burst is one read, after the debounce, of whole lines only", async () => {
  const t = setup({ files: { "logs/s.jsonl": ['{"start":1}\n'] } });
  await t.tailer.start();
  t.take();
  const file = t.tree.path("logs/s.jsonl");
  // Half a line, split inside a three-byte character.
  const bytes = Buffer.from('{"text":"日本語 ✓"}\n');
  const cut = bytes.indexOf(Buffer.from("本")) + 1;
  t.tree.append("logs/s.jsonl", bytes.subarray(0, cut));
  t.watcher.emit(file);
  await t.clock.advance(10);
  t.watcher.emit(file);
  await t.clock.advance(29);
  assert.equal(t.tailer.reads.size + t.got.length, 0, "still settling");
  await t.clock.advance(1);
  await t.settle();
  assert.deepEqual(t.take(), [], "half a line waits");
  t.tree.append("logs/s.jsonl", Buffer.concat([bytes.subarray(cut), Buffer.from('{"next":2}\n{"partial')]));
  t.watcher.emit(file);
  await t.clock.advance(30);
  await t.settle();
  assert.deepEqual(t.take(), ['s.jsonl:live:{"text":"日本語 ✓"}', 's.jsonl:live:{"next":2}']);

  // Rewritten from the top: read from the start, without the half line.
  t.tree.write("logs/s.jsonl", '{"over":"again"}\n');
  await t.tailer.track(file, { prime: false });
  assert.deepEqual(t.take(), ['s.jsonl:live:{"over":"again"}']);
  t.tailer.stop();
});

test("sweeps catch up on what no event announced, and every so often find new files", async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'] } });
  t.tree.touch("logs", t.clock.now());
  await t.tailer.start();
  t.take();
  t.tree.append("logs/a.jsonl", '{"a":1}\n');
  t.tree.write("logs/new/b.jsonl", '{"b":1}\n');
  // Real writes can share a timestamp on Windows; the scanner's cache
  // needs the same passage of time as the clock this test advances.
  t.tree.touch("logs", t.clock.now() + 1000);
  // The first sweep, 2 seconds in, also walks the folders.
  await t.clock.advance(2000);
  await t.settle();
  assert.deepEqual(t.take().sort(), ['a.jsonl:live:{"a":1}', 'b.jsonl:live:{"b":1}']);
  assert.deepEqual(t.fresh, ["b.jsonl"]);

  // The next walk is 15 sweeps on; the sweeps between only look at the
  // files already read. (A sweep at a time: one that comes while another
  // is under way joins it.)
  t.tree.write("logs/later/c.jsonl", '{"c":1}\n');
  t.tree.touch("logs", t.clock.now() + 1000);
  for (let i = 0; i < 14; i++) {
    await t.clock.advance(2000);
    await t.settle();
  }
  assert.deepEqual(t.take(), []);
  await t.clock.advance(2000);
  await t.settle();
  assert.deepEqual(t.take(), ['c.jsonl:live:{"c":1}']);
  assert.deepEqual([t.tailer.counters.sweeps, t.tailer.counters.walks], [16, 2]);

  // A file that went is let go.
  t.tree.remove("logs/a.jsonl");
  await t.tailer.sweep({ discover: false });
  assert.equal([...t.tailer.files.keys()].some((f) => f.endsWith("a.jsonl")), false);
  assert.equal(t.tailer.status.files, 2);
  t.tailer.stop();
  await t.clock.advance(10e3);
  assert.equal(t.tailer.counters.sweeps, 17, "stopped: no more sweeps");
});

test("two reads at once take each line once", async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'] } });
  await t.tailer.start();
  t.take();
  t.tree.append("logs/a.jsonl", Array.from({ length: 300 }, (_, i) => `{"i":${i}}\n`).join(""));
  const file = t.tree.path("logs/a.jsonl");
  await Promise.all([t.tailer.track(file, { prime: false }), t.tailer.track(file, { prime: false }), t.tailer.sweep()]);
  assert.equal(new Set(t.take()).size, 300);
  t.tailer.stop();
});

test("watches that fail are counted; the sweep still reads", async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'] } });
  t.watcher.refuse(t.tree.path("logs"));
  await t.tailer.start();
  assert.deepEqual([t.tailer.status.watching, t.tailer.status.errors, t.tailer.state], [false, 1, "degraded"]);
  assert.match(t.tailer.lastError.message, /EMFILE/);
  t.tree.append("logs/a.jsonl", '{"a":1}\n');
  await t.clock.advance(2000);
  await t.settle();
  assert.deepEqual(t.take().slice(-1), ['a.jsonl:live:{"a":1}']);
  assert.equal(t.tailer.state, "degraded", "without a watch, new lines wait for the sweep");
  const h = t.tailer.health();
  assert.deepEqual([h.counters.files, h.counters.dormant, h.counters.watchers], [1, 0, 0]);
  t.tailer.stop();

  // A watch that reports an error, and reads that work after it.
  const u = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'] } });
  await u.tailer.start();
  u.watcher.error(new Error("the folder went away"));
  assert.deepEqual([u.tailer.state, u.tailer.status.errors], ["degraded", 1]);
  u.tree.append("logs/a.jsonl", '{"a":1}\n');
  await u.clock.advance(2000);
  await u.settle();
  assert.equal(u.tailer.state, "running", "reading again is fine again");
  u.tailer.stop();
});

// A file mode stops its owner reading it, except on Windows or as root.
const modesBind = process.platform !== "win32" && process.getuid?.() !== 0;

test("a file it can't read at start waits, and the rest are read and followed; once it can be read, what it held is history", { skip: !modesBind && "file modes don't stop this user reading" }, async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'], "logs/locked.jsonl": ['{"l":0}\n'], "logs/b.jsonl": ['{"b":0}\n'] } });
  const locked = t.tree.path("logs/locked.jsonl");
  // Another user's file, say: it can be written to here, not read.
  fs.chmodSync(locked, 0o200);
  try {
    await t.tailer.start();
    assert.deepEqual(t.take().sort(), ['a.jsonl:history:{"a":0}', 'b.jsonl:history:{"b":0}']);
    assert.deepEqual([t.tailer.state, t.tailer.status.errors, t.tailer.status.watching, t.tailer.status.files], ["degraded", 1, true, 2]);
    assert.match(t.tailer.lastError.message, /EACCES|EPERM/);
    assert.deepEqual([...t.tailer.dormant.keys()].map((f) => path.basename(f)), ["locked.jsonl"]);

    // The sweeps go on, and don't try it again each time.
    t.tree.append("logs/a.jsonl", '{"a":1}\n');
    for (let i = 0; i < 3; i++) {
      await t.clock.advance(2000);
      await t.settle();
    }
    assert.deepEqual(t.take(), ['a.jsonl:live:{"a":1}']);
    assert.deepEqual([t.tailer.status.errors, t.tailer.state], [1, "running"], "a read that works is fine again");

    // Written to while it still can't be read: it waits again.
    t.tree.append("logs/locked.jsonl", '{"l":1}\n');
    t.watcher.emit(locked);
    await t.clock.advance(30);
    await t.settle();
    assert.deepEqual([t.take(), t.tailer.status.errors, t.tailer.dormant.has(locked), t.tailer.files.has(locked)], [[], 2, true, false]);

    // Once it can be read: what it held at start is history, what came
    // after is live.
    fs.chmodSync(locked, 0o600);
    t.tree.append("logs/locked.jsonl", '{"l":2}\n');
    t.watcher.emit(locked);
    await t.clock.advance(30);
    await t.settle();
    assert.deepEqual(t.take(), ['locked.jsonl:history:{"l":0}', 'locked.jsonl:live:{"l":1}', 'locked.jsonl:live:{"l":2}']);
  } finally {
    fs.chmodSync(locked, 0o600);
    t.tailer.stop();
  }
});

test("stopped while it starts, it reads no more, and neither watches nor sweeps", async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'], "logs/b.jsonl": ['{"b":0}\n'] } });
  const starting = t.tailer.start();
  t.tailer.stop();
  await starting;
  await t.settle();
  assert.deepEqual([t.tailer.state, t.tailer.started, t.watcher.watching(), t.tailer.watchers.length, t.tailer.status.watching], ["stopped", false, [], 0, false]);
  assert.ok(t.got.length < 2, "the history isn't read on after the stop");
  await t.clock.advance(10e3);
  assert.equal(t.tailer.counters.sweeps ?? 0, 0, "no sweeps");
  assert.equal(t.clock.pending(), 0, "nothing left waiting");
});

test("stopped, it lets go of its watches, so a start after it says when a watch fails", async () => {
  const t = setup({ files: { "logs/a.jsonl": ['{"a":0}\n'] } });
  await t.tailer.start();
  // A watch event waits out its debounce when the tailer stops.
  t.tree.append("logs/a.jsonl", '{"a":1}\n');
  t.watcher.emit(t.tree.path("logs/a.jsonl"));
  t.tailer.stop();
  assert.deepEqual([t.tailer.watchers.length, t.tailer.status.watching, t.tailer.pending.size, t.clock.pending()], [0, false, 0, 0]);
  await t.clock.advance(5000);
  await t.settle();
  t.take();

  // Started again, with the folder's watch refused this time.
  t.watcher.refuse(t.tree.path("logs"));
  await t.tailer.start();
  assert.deepEqual([t.tailer.state, t.tailer.status.watching, t.tailer.watchers.length], ["degraded", false, 0]);
  assert.match(t.tailer.lastError.message, /EMFILE/);
  // The sweep still reads: what was written while it was stopped, then
  // what's new.
  t.tree.append("logs/a.jsonl", '{"a":2}\n');
  await t.clock.advance(2000);
  await t.settle();
  assert.deepEqual(t.take(), ['a.jsonl:live:{"a":1}', 'a.jsonl:live:{"a":2}']);
  t.tailer.stop();
});
