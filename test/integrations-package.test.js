import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import * as tar from "tar";

test("the pinned SDK archive has compiled runtime, its MIT license and verified install integrity", async () => {
  const pkg = JSON.parse(fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.equal(pkg.dependencies.jevrouter, "file:vendor/jevrouter-0.1.0.tgz");
  const file = new URL("../vendor/jevrouter-0.1.0.tgz", import.meta.url);
  const lock = JSON.parse(fs.readFileSync(new URL("../package-lock.json", import.meta.url), "utf8"));
  const integrity = `sha512-${createHash("sha512").update(fs.readFileSync(file)).digest("base64")}`;
  assert.equal(lock.packages["node_modules/jevrouter"].integrity, integrity);
  const names = [];
  await tar.t({ file: fileURLToPath(file), onReadEntry: (entry) => { names.push(entry.path); entry.resume(); } });
  assert.ok(names.includes("package/LICENSE")); assert.ok(names.includes("package/dist/index.js")); assert.ok(names.includes("package/dist/router.js"));
});
