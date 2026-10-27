import { test } from "node:test";
import assert from "node:assert/strict";
import { measure, parseShard, readTimings, shards, suiteFiles } from "../scripts/e2e-shard.mjs";

test("every end-to-end file runs in exactly one shard, whatever the count", () => {
  const files = suiteFiles();
  for (let total = 1; total <= 6; total++) {
    const all = shards(files, readTimings(), total).flat();
    assert.deepEqual([...all].sort(), files, `${total} shards`);
  }
});

test("the shards come out the same each time, and close in time with CI's timings", () => {
  const files = suiteFiles();
  const timings = readTimings();
  assert.deepEqual(shards(files, timings, 4), shards([...files].reverse(), timings, 4));
  const secs = shards(files, timings, 4).map((s) => s.reduce((sum, f) => sum + (timings[f] ?? 0), 0));
  const longest = Math.max(...Object.values(timings));
  // No shard runs past the average by more than its longest file.
  assert.ok(Math.max(...secs) <= secs.reduce((a, b) => a + b, 0) / 4 + longest, `${secs}`);
});

test("a file without a time counts as the median", () => {
  const timings = { "a.test.js": 10, "b.test.js": 20, "c.test.js": 30 };
  assert.deepEqual(shards(["a.test.js", "b.test.js", "c.test.js", "new.test.js"], timings, 2), [["a.test.js", "c.test.js"], ["b.test.js", "new.test.js"]]);
});

test("a shard is written <index>/<total>", () => {
  assert.deepEqual(parseShard("2/4"), { index: 2, total: 4 });
  for (const bad of ["0/4", "5/4", "2", "a/b", undefined]) assert.throws(() => parseShard(bad));
});

test("each file's time is read from the spec reporter's lines in a CI log", () => {
  const log = [
    "e2e-linux\tRun\t2026-10-27T01:23:30.1Z ✔ the map draws (1500.5ms)",
    "e2e-linux\tRun\t2026-10-27T01:23:31.1Z ✔ a theme switches (2500ms)",
    "e2e-linux\tRun\t2026-10-27T01:23:32.1Z ✔ the map zooms (1000ms)",
    "e2e-linux\tRun\t2026-10-27T01:23:33.1Z an unrelated line",
  ].join("\n");
  const source = { "map.test.js": 'test("the map draws", …); test("the map zooms", …)', "themes.test.js": 'test("a theme switches", …)' };
  assert.deepEqual(measure(log, Object.keys(source), (f) => source[f]), { "map.test.js": 3, "themes.test.js": 3 });
});
