import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

// Temp folders, taken away after the tests.
const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-test-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

test("palette fuzzy match ranks substrings and word starts first", async () => {
  const { fuzzy } = await import("../src/palette.js");
  assert.ok(fuzzy("led", "ledger") > fuzzy("led", "build local ledger"));
  assert.ok(fuzzy("blam", "Build local Ledger with mock") > 0, "subsequence");
  assert.equal(fuzzy("zzz", "ledger"), 0);
  assert.equal(fuzzy("", "anything"), 1);
});

