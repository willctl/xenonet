// The Python side of the registered mode for Hermes
// (plugins/hermes/xenonet): its own unittest suite, run when python3 is here.
import test from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../plugins/hermes");
const probe = spawnSync("python3", ["--version"]);

test("the Hermes plugin's unit tests pass", { skip: probe.error ? "python3 isn't here" : false }, () => {
  const r = spawnSync("python3", ["-I", "-B", "-m", "unittest", "discover", "-s", dir, "-p", "test_*.py"], { encoding: "utf8", cwd: dir });
  assert.equal(r.status, 0, r.stderr);
});
