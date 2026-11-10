// The Rust core's built-in themes (crates/xenonet-console/src/themes.json) are
// made from src/shared/themes.js, which the console page keeps: a change to one
// is a change to both (node crates/xenonet-console/themes.mjs).
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("the Rust's built-in themes say what src/shared/themes.js says", () => {
  execFileSync(process.execPath, ["crates/xenonet-console/themes.mjs", "--check"], { cwd: root, stdio: "pipe" });
});
