import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { patchExport } from "../electron/patch-export.js";

const encoded = (entries) => Object.entries(entries).map(([path, value]) => ({ path, content: Buffer.from(value).toString("base64") }));
const result = (entries, extra = {}) => ({ files: encoded(entries), problems: [], omissions: [], ...extra });
function tree(t, files = {}) {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-review-test-")));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  for (const [name, text] of Object.entries(files)) { const file = path.join(dir, name); fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text); }
  return dir;
}
function repo(t, files = { "file.txt": "original\n" }) {
  const dir = tree(t, files); execFileSync("git", ["init", "-q", dir]);
  // The fixture checks raw patch bytes without runner-level newline conversion.
  execFileSync("git", ["-C", dir, "config", "core.autocrlf", "false"]);
  execFileSync("git", ["-C", dir, "add", "."]); return dir;
}

test("native patch export rejects malformed filenames, modified bytes and unbounded receipts", () => {
  assert.equal(patchExport({ filename: "../../file", patch: "text" }), null);
  assert.equal(patchExport({ filename: "xenonet-12345678.patch", patch: "text", receipt: { patchHash: "wrong" } }), null);
});

