// Runs the JS tests against the Rust core (docs/design/rust-core.md), with
// each ported module's twin loaded in place of its JS.
//
//   npm run test:rust               the test files parity.json lists
//   npm run test:rust -- --all      every test file, one at a time: which
//                                   reach a twin, and pass or fail with it
//   npm run test:rust -- --all --write
//                                   and keep the passing ones in parity.json
//   npm run test:rust -- <files>    these files
//
// A file counts only if it reaches a twin. One that fails with twins is
// run again without them, to tell a difference in Rust from a test that
// fails here anyway.
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const parityFile = path.join(root, "crates", "xenonet-node", "parity.json");
const register = "./crates/xenonet-node/js/register.js";
const args = process.argv.slice(2);
const all = args.includes("--all");
const write = args.includes("--write");
const named = args.filter((a) => !a.startsWith("--"));
const parity = JSON.parse(fs.readFileSync(parityFile, "utf8"));

function run(file, { twins }) {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-parity-"));
  const logFile = path.join(tmp, "twins");
  const argv = [...(twins ? ["--import", register] : []), "--test", "--test-reporter=dot", file];
  return new Promise((resolve) => {
    const child = spawn(process.execPath, argv, { cwd: root, env: { ...process.env, XENONET_TWIN_LOG: twins ? logFile : "" }, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => {
      const reached = fs.existsSync(logFile) ? [...new Set(fs.readFileSync(logFile, "utf8").split("\n").filter(Boolean))].sort() : [];
      fs.rmSync(tmp, { recursive: true, force: true });
      resolve({ file, ok: code === 0, reached, out });
    });
  });
}

async function inTurn(items, n, fn) {
  const out = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    for (let i = next++; i < items.length; i = next++) out[i] = await fn(items[i]);
  }));
  return out;
}

const files = all ? fs.readdirSync(path.join(root, "test")).filter((f) => f.endsWith(".test.js")).sort().map((f) => `test/${f}`) : named.length ? named : parity.tests;
const n = Math.max(1, Math.floor(os.availableParallelism() / 2));
const results = await inTurn(files, n, (f) => run(f, { twins: true }));
const failed = results.filter((r) => !r.ok);
for (const r of failed) r.onJs = (await run(r.file, { twins: false })).ok;

const reaching = results.filter((r) => r.reached.length);
const width = Math.max(...files.map((f) => f.length));
for (const r of all ? reaching : results) {
  const how = r.ok ? "pass" : r.onJs ? "FAIL (passes on JS)" : "fail (fails on JS too)";
  console.log(`${r.file.padEnd(width)}  ${how.padEnd(22)} ${r.reached.join(" ")}`);
}
for (const r of failed.filter((r) => r.onJs)) console.log(`\n--- ${r.file}\n${r.out.slice(-4000)}`);
const passing = reaching.filter((r) => r.ok).map((r) => r.file);
console.log(`\n${passing.length} of ${files.length} test files pass with twins${all ? `; ${reaching.length} reach one` : ""}.`);
if (write) {
  parity.tests = passing;
  fs.writeFileSync(parityFile, JSON.stringify(parity, null, 2) + "\n");
  console.log(`wrote ${path.relative(root, parityFile)}`);
}
process.exitCode = failed.some((r) => r.onJs) || (!all && failed.length) ? 1 : 0;
