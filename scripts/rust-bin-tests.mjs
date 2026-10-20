// Runs the JS tests that start XenoNet's processes (the service, the
// command, its MCP servers) against the Rust binary (docs/design/rust-core.md),
// through test/harness/xenonet-bin.js: each file once, with XENONET_CLI_BIN
// and XENONET_SERVICE_BIN naming target/debug/xenonet, and says how many of
// its tests pass.
//
//   npm run test:bin                 builds the binary, runs the files below
//   npm run test:bin -- <files>      these files
//   npm run test:bin -- --no-build   with the binary as it is
//   npm run test:bin -- --bin=<file> against that binary, without building
//                                    (coverage:rust -- --js gives its own)
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
const given = args.find((a) => a.startsWith("--bin="))?.slice("--bin=".length);
const bin = given ? path.resolve(given) : path.join(root, "target", "debug", process.platform === "win32" ? "xenonet.exe" : "xenonet");

if (!given && !args.includes("--no-build")) cargo(["build", "-p", "xenonet-cli"], { stdio: "inherit", env: { ...process.env, CARGO_PROFILE_DEV_DEBUG: "0" } });

// node --test's TAP summary: # tests, # pass, # fail, # skipped.
const count = (out, key) => Number(new RegExp(`^# ${key} (\\d+)$`, "m").exec(out)?.[1] ?? 0);

// A test the binary leaves waiting fails after this, as one that times out,
// unless it sets its own; a file still running after FILE_LIMIT (a hook
// that never returns) is stopped, so a hang can't hold up the rest.
const TEST_LIMIT = 180000;
const FILE_LIMIT = 15 * 60000;

// What a failed test's TAP block says went wrong: its error's first lines
// and the line of the test file it was thrown from, so a failure on a CI
// runner can be read from the log alone.
function why(block) {
  const lines = block.split("\n");
  const at = lines.findIndex((l) => /^\s+error: /.test(l));
  const error = [];
  if (at >= 0) {
    const indent = lines[at].match(/^\s*/)[0].length;
    const first = lines[at].trim().slice("error: ".length);
    if (/^[|>][-+]?$/.test(first)) {
      for (let i = at + 1; i < lines.length && (!lines[i].trim() || lines[i].match(/^\s*/)[0].length > indent); i++) error.push(lines[i].trim());
    } else {
      error.push(first.replace(/^'(.*)'$/, "$1"));
    }
  }
  const where = /[\\/](test[\\/][^\s:)]+:\d+):\d+/.exec(block)?.[1].replace(/\\/g, "/");
  const text = error.filter(Boolean).slice(0, 3).join(" / ");
  return [text, where && `(${where})`].filter(Boolean).join(" ");
}

// Each failed test in node --test's TAP output, with why it failed.
function failures(out) {
  const lines = out.split("\n");
  const found = [];
  for (let i = 0; i < lines.length; i++) {
    const m = /^\s*not ok \d+ - (.+)$/.exec(lines[i]);
    if (!m) continue;
    let end = i + 1;
    while (end < lines.length && !/^\s*\.\.\.$/.test(lines[end])) end++;
    found.push({ name: m[1], why: why(lines.slice(i + 1, end).join("\n")) });
  }
  return found;
}

function run(file) {
  return new Promise((resolve) => {
    const env = { ...process.env, XENONET_CLI_BIN: bin, XENONET_SERVICE_BIN: bin };
    const child = spawn(process.execPath, ["--test", "--test-reporter=tap", `--test-timeout=${TEST_LIMIT}`, file], { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let stopped = false;
    const limit = setTimeout(() => ((stopped = true), child.kill("SIGKILL")), FILE_LIMIT);
    child.stdout.on("data", (d) => (out += d));
    child.stderr.on("data", (d) => (out += d));
    child.on("close", (code) => {
      clearTimeout(limit);
      const failed = failures(out);
      if (stopped) failed.push({ name: `(stopped after ${FILE_LIMIT / 60000} minutes)`, why: "" });
      resolve({ file, code, tests: count(out, "tests"), pass: count(out, "pass"), fail: count(out, "fail") + (stopped ? 1 : 0), skipped: count(out, "skipped"), failed });
    });
  });
}

const results = [];
for (const file of files) {
  const r = await run(file);
  results.push(r);
  console.log(`${r.fail ? "✗" : "✓"} ${path.basename(file).padEnd(32)} ${String(r.pass).padStart(3)}/${r.tests} pass${r.fail ? `, ${r.fail} fail` : ""}${r.skipped ? `, ${r.skipped} skipped` : ""}`);
  for (const f of r.failed.slice(0, 40)) {
    console.log(`    not ok: ${f.name}`);
    if (f.why) console.log(`        ${f.why.slice(0, 400)}`);
  }
}
const pass = results.reduce((n, r) => n + r.pass, 0);
const tests = results.reduce((n, r) => n + r.tests, 0);
console.log(`\n${pass} of ${tests} tests pass against the binary, in ${results.length} files`);
if (args.includes("--write")) {
  const record = { binary: "target/debug/xenonet", at: new Date().toISOString().slice(0, 10), files: Object.fromEntries(results.map((r) => [r.file, { pass: r.pass, tests: r.tests, failed: r.failed.map((f) => f.name) }])) };
  fs.writeFileSync(path.join(root, "crates", "xenonet-cli", "bin-tests.json"), JSON.stringify(record, null, 2) + "\n");
}
process.exitCode = results.some((r) => r.fail) ? 1 : 0;
