// Runs the JS tests that start XenoNet's processes (the service, the
// command, its MCP servers) against the Rust binary (docs/design/rust-core.md),
// through test/harness/xenonet-bin.js: each file once, with XENONET_CLI_BIN
// and XENONET_SERVICE_BIN naming target/debug/xenonet, and says how many of
// its tests pass.
//
//   npm run test:bin                 builds the binary, runs the files below
//   npm run test:bin -- <files>      these files
//   npm run test:bin -- --no-build   with the binary as it is
//   npm run test:bin -- --write      and keeps the counts in
//                                    crates/xenonet-cli/bin-tests.json
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { cargo } from "./cargo.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const named = args.filter((a) => !a.startsWith("--"));
// The tests that start the service, the command or an MCP server.
const FILES = [
  "test/server.test.js",
  "test/hosts-server.test.js",
  "test/control.test.js",
  "test/cli.test.js",
  "test/cli-actor.test.js",
  "test/cli-gateway.test.js",
  "test/cli-status-agents.test.js",
  "test/hosts-cli.test.js",
  "test/launcher.test.js",
  "test/mcp-actor.test.js",
  "test/engine.test.js",
  "test/assistant-endpoint.test.js",
];
const files = named.length ? named : FILES;
const bin = path.join(root, "target", "debug", process.platform === "win32" ? "xenonet.exe" : "xenonet");

if (!args.includes("--no-build")) cargo(["build", "-p", "xenonet-cli"], { stdio: "inherit", env: { ...process.env, CARGO_PROFILE_DEV_DEBUG: "0" } });

// node --test's TAP summary: # tests, # pass, # fail, # skipped.
const count = (out, key) => Number(new RegExp(`^# ${key} (\\d+)$`, "m").exec(out)?.[1] ?? 0);

function run(file) {
  return new Promise((resolve) => {
    const env = { ...process.env, XENONET_CLI_BIN: bin, XENONET_SERVICE_BIN: bin };
    const child = spawn(process.execPath, ["--test", "--test-reporter=tap", file], { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => {
      const failed = [...out.matchAll(/^\s*not ok \d+ - (.+)$/gm)].map((m) => m[1]);
      resolve({ file, code, tests: count(out, "tests"), pass: count(out, "pass"), fail: count(out, "fail"), skipped: count(out, "skipped"), failed });
    });
  });
}

const results = [];
for (const file of files) {
  const r = await run(file);
  results.push(r);
  console.log(`${r.fail ? "✗" : "✓"} ${path.basename(file).padEnd(32)} ${String(r.pass).padStart(3)}/${r.tests} pass${r.fail ? `, ${r.fail} fail` : ""}${r.skipped ? `, ${r.skipped} skipped` : ""}`);
  for (const name of r.failed.slice(0, 40)) console.log(`    not ok: ${name}`);
}
const pass = results.reduce((n, r) => n + r.pass, 0);
const tests = results.reduce((n, r) => n + r.tests, 0);
console.log(`\n${pass} of ${tests} tests pass against the binary, in ${results.length} files`);
if (args.includes("--write")) {
  const record = { binary: "target/debug/xenonet", at: new Date().toISOString().slice(0, 10), files: Object.fromEntries(results.map((r) => [r.file, { pass: r.pass, tests: r.tests, failed: r.failed }])) };
  fs.writeFileSync(path.join(root, "crates", "xenonet-cli", "bin-tests.json"), JSON.stringify(record, null, 2) + "\n");
}
process.exitCode = results.some((r) => r.fail) ? 1 : 0;
