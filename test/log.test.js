// XenoNet Desktop's error log (electron/lib/log.js): scrubbed as written,
// one line a code every ten seconds with how many meanwhile, and moved
// aside past its size. The home folder and keys are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createLog, readLogs, summarize, scrub, unsafe } from "../electron/lib/log.js";
const HOME = "C:\\Users\\nova";
const KEY = "sk-proj-CANARY0123456789abcdefABCDEF";

test("scrubbed as written: the home folder as ~, in each of its forms, and keys out", () => {
  assert.equal(scrub(`C:\\Users\\nova\\Projects\\atlas and C:/Users/nova/.codex and C--Users-nova-Projects-atlas`, HOME), "~\\Projects\\atlas and ~/.codex and ~-Projects-atlas");
  assert.equal(scrub(`token ${KEY} and Bearer abcdefgh12345678 and ghp_ABCDEF1234567890`, HOME), "token [key] and [key] and [key]");
  assert.equal(scrub("bell\u0007 and tab\tkept\nand lines", HOME), "bell and tab\tkept\nand lines");
  assert.equal(unsafe("C:\\Users\\nova\\x", HOME), "the home folder");
  assert.equal(unsafe(`a ${KEY}`, HOME), "something that looks like a key");
  assert.equal(unsafe("~\\x [key]", HOME), null);
  // In JSON, as a bundle is checked: backslashes doubled.
  assert.equal(unsafe(JSON.stringify({ x: "C:\\Users\\Nova\\x" }), HOME), "the home folder");
  assert.equal(unsafe(JSON.stringify({ x: "~\\x" }), HOME), null);
});

test("written as JSON lines, the same code with the same words once every ten seconds, and read back newest first", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-log-"));
  try {
    let t = 1_000_000;
    const log = createLog({ dir, component: "service", version: "0.8.0", home: HOME, now: () => t });
    const err = Object.assign(new Error(`couldn't read C:\\Users\\nova\\.claude with ${KEY}`), { code: "EACCES" });
    log.error("service.read", err.message, { err, fix: "Check C:\\Users\\nova\\.claude's permissions.", ctx: { file: "C:\\Users\\nova\\.claude\\x.jsonl" } });
    t += 1000;
    assert.equal(log.error("service.read", err.message, { err }), null, "again too soon");
    log.error("service.read", err.message, { err });
    t += 11_000;
    log.error("service.read", err.message, { err });
    log.warn("doctor.claude.auth", "Claude Code is signed in: not signed in", { fix: "Run claude and sign in with /login." });
    const text = fs.readFileSync(path.join(dir, "service.ndjson"), "utf8");
    assert.ok(!text.includes("nova") && !text.includes(KEY), "nothing of the home folder or the key");
    const all = readLogs(dir, { level: "info" });
    assert.deepEqual(all.map((e) => [e.code, e.level, e.repeats ?? 0]), [["service.read", "error", 2], ["doctor.claude.auth", "warn", 0], ["service.read", "error", 0]], "newest first, with how many it held back");
    const first = all.at(-1);
    assert.equal(first.message, "couldn't read ~\\.claude with [key]");
    assert.equal(first.fix, "Check ~\\.claude's permissions.");
    assert.equal(first.errno, "EACCES");
    assert.deepEqual(first.ctx, { file: "~\\.claude\\x.jsonl" });
    assert.match(first.stack, /^Error: couldn't read ~/);
    assert.deepEqual(readLogs(dir, { level: "error" }).map((e) => e.code), ["service.read", "service.read"]);
    assert.deepEqual(readLogs(dir, { level: "info", code: "doctor." }).map((e) => e.code), ["doctor.claude.auth"]);
    const rows = summarize(all);
    assert.deepEqual(rows.map((r) => [r.code, r.count]), [["service.read", 4], ["doctor.claude.auth", 1]], "each code once, counting what it held back");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("a log past its size moves aside, two are kept, and a log with no folder writes nothing", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-log-"));
  try {
    let t = 0;
    const log = createLog({ dir, component: "cli", home: HOME, now: () => (t += 20_000), maxBytes: 300 });
    for (let i = 0; i < 20; i++) log.warn("cli.x", `number ${i}`);
    assert.deepEqual(fs.readdirSync(dir).sort(), ["cli.1.ndjson", "cli.2.ndjson", "cli.ndjson"]);
    assert.ok(readLogs(dir, { level: "warn" }).length < 20, "the oldest went");
    assert.equal(readLogs(dir, { level: "warn" })[0].message, "number 19");
    const none = createLog({ dir: null, component: "service" });
    assert.equal(none.error("x", "y").message, "y", "it still makes the entry");
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

