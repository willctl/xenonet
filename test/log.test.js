// XenoNet's error log (core/log.js) and what xenonet diagnose makes of it
// (core/diagnostics.js): scrubbed as written, one line a code every ten
// seconds with how many meanwhile, moved aside past its size, read back by
// level, part and code; and a bundle that refuses to leave holding a home
// folder or a key. The home folder and keys are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createLog, readLogs, summarize, scrub, unsafe } from "../core/log.js";
import { makeBundle, bundleMarkdown, agentBrief, askCommand, writeLook } from "../core/diagnostics.js";

const HOME = "C:\\Users\\nova";
const KEY = "sk-proj-CANARY0123456789abcdefABCDEF";

test("scrubbed as written: the home folder as ~, in each of its forms, and keys out", () => {
  assert.equal(scrub(`C:\\Users\\nova\\Projects\\atlas and C:/Users/nova/.codex and C--Users-nova-Projects-atlas`, HOME), "~\\Projects\\atlas and ~/.codex and ~-Projects-atlas");
  assert.equal(scrub(`token ${KEY} and Bearer abcdefgh12345678 and ghp_ABCDEF1234567890`, HOME), "token [key] and [key] and [key]");
  assert.equal(scrub("bell\u0007 and tab\tkept\nand lines", HOME), "bell and tab\tkept\nand lines");
  assert.equal(unsafe("C:\\Users\\nova\\x", HOME), "the home folder");
  assert.equal(unsafe(`a ${KEY}`, HOME), "something that looks like a key");
  assert.equal(unsafe("~\\x [key]", HOME), null);
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

test("diagnostics: what's wrong by code, safe to share, refused if anything private is left", () => {
  const doctor = { at: 1, summary: { ok: 30, warn: 1, fail: 1 }, results: [{ id: "agents.codex", status: "fail", title: "Codex sessions are connected to XenoNet", detail: "connected, but C:\\Users\\nova\\old\\cli\\xenonet.js is gone", fix: "Run xenonet connect again." }, { id: "claude.cli", status: "ok", title: "x", detail: "y" }] };
  const errors = [{ schema: "log/entry/v1", at: Date.parse("2026-10-07T10:00:00Z"), component: "service", level: "error", code: "service.port-in-use", message: "port 47840 is already in use by XenoNet Dev (dev)", fix: "Quit the other XenoNet." }];
  const connect = { port: 47840, apps: [{ app: "codex", changes: [{ id: "hooks", state: "present" }], broken: [], notes: ["Another XenoNet (port 47842, XenoNet Dev) is connected here too; its entries stay as they are."] }] };
  const b = makeBundle({ xenonet: { version: "0.8.0", instance: "installed", running: true }, doctor, errors, connect, env: { XENONET_PORT: "47840", XENONET_STATE_DIR: "C:\\Users\\nova\\state", PATH: "x" }, home: HOME, now: Date.parse("2026-10-07T12:00:00Z") });
  assert.equal(b.schema, "diagnostics/bundle/v1");
  assert.deepEqual(b.doctor.problems.map((p) => [p.id, p.detail]), [["agents.codex", "connected, but ~\\old\\cli\\xenonet.js is gone"]]);
  assert.deepEqual(b.errors.map((e) => e.code), ["service.port-in-use"]);
  assert.deepEqual(b.settings, ["XENONET_PORT=47840", "XENONET_STATE_DIR"], "a path setting's name only");
  const md = bundleMarkdown(b);
  assert.match(md, /`agents\.codex`/);
  assert.match(md, /port 47840 is already in use by XenoNet Dev/);
  assert.ok(!md.includes("nova"));
  assert.throws(() => makeBundle({ xenonet: { instance: KEY }, env: {}, home: HOME }), /still held something that looks like a key/, "anything private left is refused");
});

test("asking an agent: a folder of its own with the bundle and a brief that rules out filing anything, and a read-only session", () => {
  const state = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-diag-"));
  try {
    const b = makeBundle({ xenonet: { version: "0.8.0" }, env: {}, home: HOME });
    const dir = writeLook(state, b, { sources: ["/opt/xenonet"], now: 1_800_000_000_000 });
    assert.deepEqual(fs.readdirSync(dir).sort(), ["AGENTS.md", "CLAUDE.md", "bundle.json", "bundle.md"]);
    const brief = agentBrief({ sources: ["/opt/xenonet"] });
    assert.match(brief, /Never file an issue, open a pull request, push, post or send anything anywhere, and don't run gh/);
    assert.match(brief, /issue-draft\.md/);
    // A look older than a week goes when the next is made.
    writeLook(state, b, { now: 1_800_000_000_000 + 8 * 86400e3 });
    assert.equal(fs.existsSync(dir), false);
    const codex = askCommand({ app: "codex", model: "gpt-5.1-codex", dir: "/d", sources: ["/opt/xenonet"] });
    assert.deepEqual(codex.args.slice(0, 8), ["-C", "/d", "-s", "read-only", "-a", "on-request", "-m", "gpt-5.1-codex"]);
    assert.ok(codex.args.includes("--add-dir"));
    const claude = askCommand({ app: "claude", dir: "/d" });
    assert.deepEqual(claude.args.slice(0, 2), ["--permission-mode", "plan"]);
    assert.equal(claude.cwd, "/d");
  } finally {
    fs.rmSync(state, { recursive: true, force: true });
  }
});
