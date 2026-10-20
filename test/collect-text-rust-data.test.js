// The program the Rust binary hands a sandbox to collect its text
// (crates/xenonet-integrations/src/collect_text.js) is the one
// core/integrations/reviews.js makes from its own functions: a change to
// one is a change to both (node crates/xenonet-integrations/collect-text.mjs).
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("the Rust's text collector is what core/integrations/reviews.js makes", () => {
  execFileSync(process.execPath, ["crates/xenonet-integrations/collect-text.mjs", "--check"], { cwd: root, stdio: "pipe" });
});
