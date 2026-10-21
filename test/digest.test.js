import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { summarize, dayLog, summaryText, askCodex, narrate } from "../core/digest.js";

// A stand-in for `codex exec`: it writes its answer where -o says, or fails
// or hangs when told to, and keeps what it was asked and the Codex home it
// ran in: what's there, and the sign-in it finds.
const bin = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-fake-codex-"));
after(() => fs.rmSync(bin, { recursive: true, force: true }));
const codex = path.join(bin, "codex");
fs.writeFileSync(
  codex,
  `#!${process.execPath}
if (process.env.FAKE_CODEX === "early") process.exit(3);
const fs = require("fs");
const path = require("path");
const args = process.argv.slice(2);
const out = args[args.indexOf("-o") + 1];
const home = process.env.CODEX_HOME;
const auth = path.join(home, "auth.json");
const seen = { home, files: fs.readdirSync(home), linked: fs.existsSync(auth) && fs.lstatSync(auth).isSymbolicLink(), auth: fs.existsSync(auth) ? fs.realpathSync(auth) : null };
let prompt = "";
process.stdin.on("data", (d) => (prompt += d));
process.stdin.on("end", () => {
  fs.writeFileSync(${JSON.stringify(path.join(bin, "asked.json"))}, JSON.stringify({ args, prompt, ...seen }));
  if (process.env.FAKE_CODEX === "fail") { process.stderr.write("no auth"); process.exit(3); }
  if (process.env.FAKE_CODEX === "hang") return setTimeout(() => {}, 60000);
  fs.writeFileSync(out, "  All quiet.\\n");
});
`,
  { mode: 0o755 },
);
const asked = () => JSON.parse(fs.readFileSync(path.join(bin, "asked.json"), "utf8"));
const unix = process.platform !== "win32";

// Your Codex's home, made up: signed in, with hooks, instructions and a
// plugin a run of XenoNet's own must not load.
const yours = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-your-codex-"));
after(() => fs.rmSync(yours, { recursive: true, force: true }));
fs.writeFileSync(path.join(yours, "auth.json"), JSON.stringify({ auth_mode: "chatgpt" }));
fs.writeFileSync(path.join(yours, "hooks.json"), "{}");
fs.writeFileSync(path.join(yours, "AGENTS.md"), "Always answer in French.");
fs.writeFileSync(path.join(yours, "config.toml"), "[mcp_servers.x]\ncommand = \"x\"\n");
fs.mkdirSync(path.join(yours, "plugins"));
const opts = { codexHome: yours };

const at = (h, m = 0) => new Date(2026, 9, 3, h, m).getTime();
const events = [
  { sessionId: "claude:a", app: "claude", kind: "prompt", ts: at(9) },
  { sessionId: "claude:a", app: "claude", kind: "tool", category: "edit", files: ["/p/x/a.js"], ts: at(9, 5) },
  { sessionId: "claude:a", app: "claude", kind: "tool", category: "shell", ts: at(9, 6) },
  { sessionId: "claude:a", app: "claude", kind: "tool-error", ts: at(9, 7) },
  { sessionId: "claude:a", app: "claude", kind: "message", detail: "Done.", ts: at(9, 8) },
  { sessionId: "claude:a", app: "claude", kind: "turn-end", ts: at(9, 9) },
  { sessionId: "codex:b", app: "codex", kind: "tool", category: "shell", ts: at(14) },
  { sessionId: "codex:b", app: "codex", kind: "subagent-start", ts: at(14, 1) },
];

test("the day's log: hours, busiest hour, top projects, waiting and uncommitted work", () => {
  const live = { today: 1234, sessions: [{ id: "codex:b", needsYou: { kind: "question", text: "Which one?" } }, { id: "claude:a", needsYou: { kind: "review" } }] };
  const log = dayLog(events, { now: at(18), titleOf: (id) => `T ${id}`, homeOf: (id) => (id === "claude:a" ? "project:x" : "zone:y"), nameOf: (id) => id.split(":")[1], live, repos: [{ name: "x", dirty: 2 }, { name: "y", dirty: 0 }] });
  assert.equal(log.hours[9], 2);
  assert.equal(log.hours[14], 1);
  assert.equal(log.busiestHour, 9);
  assert.deepEqual(log.topProjects, [{ id: "project:x", name: "x", toolCalls: 2 }, { id: "zone:y", name: "y", toolCalls: 1 }]);
  assert.deepEqual([log.firstAt, log.lastAt], [at(9), at(14, 1)]);
  assert.equal(log.tokens, 1234);
  assert.equal(log.waiting, 1, "a review isn't waiting");
  assert.deepEqual(log.uncommitted, [{ name: "x", dirty: 2 }]);
  assert.deepEqual(log.totals, { sessions: 2, turns: 1, toolCalls: 3, files: 1, errors: 1, subagents: 1 });

  const quiet = dayLog([], { now: at(18), titleOf: (id) => id });
  assert.equal(quiet.busiestHour, null);
  assert.equal(quiet.firstAt, null);
});

test("a summary reads as lines Codex can turn into prose", () => {
  const sum = summarize(events, { since: at(0), until: at(23), titleOf: (id) => (id === "claude:a" ? "Fix the parser" : "Tidy CI"), live: { sessions: [{ id: "codex:b", needsYou: { kind: "question", text: "Which one?" } }] } });
  const text = summaryText(sum, "Today so far");
  assert.match(text, /^Today so far: 2 sessions, 1 turns, 3 tool calls, 1 files edited, 1 errors\./);
  assert.match(text, /- claude "Fix the parser": 1 turns, 2 tool calls \(edit 1, shell 1\), edited a\.js, 1 errors; last said: Done\./);
  assert.match(text, /- codex "Tidy CI": 0 turns, 1 tool calls \(shell 1\); waiting for you: Which one\?/);
  assert.match(summaryText(summarize([], { since: 0 }), "Away"), /^Away: 0 sessions/);
});

test("narrate asks codex exec, read-only and ephemeral, for its last message", { skip: !unix }, async () => {
  assert.equal(await narrate(codex, "Today so far: 2 sessions", opts), "All quiet.");
  const { args, prompt } = asked();
  for (const a of ["exec", "--ephemeral", "--skip-git-repo-check", "read-only", "-"]) assert.ok(args.includes(a), a);
  assert.match(prompt, /Activity data:\nToday so far: 2 sessions$/);
  assert.equal(args.includes("--output-schema"), false);
});

test("codex exec runs isolated: a Codex home of its own with your sign-in, and no plugins, hooks, shell or web search", { skip: !unix }, async () => {
  assert.equal(await askCodex(codex, "Sort these", opts), "All quiet.");
  const { args, home, files, linked, auth } = asked();
  assert.notEqual(home, yours);
  assert.equal(path.basename(home), "home");
  assert.equal(fs.existsSync(home), false, "gone with the run's folder");
  assert.deepEqual(files, ["auth.json"], "none of your hooks, instructions, config or plugins");
  assert.equal(linked, true, "your sign-in is linked, never copied, so a refreshed token reaches yours");
  assert.equal(auth, fs.realpathSync(path.join(yours, "auth.json")));
  assert.equal(fs.readFileSync(path.join(yours, "auth.json"), "utf8"), JSON.stringify({ auth_mode: "chatgpt" }), "yours is as it was");
  const set = args.filter((_, i) => args[i - 1] === "-c");
  for (const kv of ["features.plugins=false", "features.hooks=false", "features.shell_tool=false", "features.unified_exec=false", 'web_search="disabled"', "project_doc_max_bytes=0", 'cli_auth_credentials_store="file"']) assert.ok(set.includes(kv), kv);
  // A Codex that doesn't know a flag, or a feature --disable names, fails
  // the run; the home of its own leaves nothing for the others to skip.
  for (const flag of ["--disable", "--ignore-user-config", "--ignore-rules", "--strict-config"]) assert.equal(args.includes(flag), false, flag);

  // Signed out of Codex: the run has no sign-in of yours to use.
  const out = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-signed-out-"));
  try {
    assert.equal(await askCodex(codex, "x", { codexHome: out }), "All quiet.");
    assert.deepEqual([asked().files, asked().auth], [[], null]);
  } finally {
    fs.rmSync(out, { recursive: true, force: true });
  }
});

test("askCodex hands Codex a schema, and says why when it fails or takes too long", { skip: !unix }, async () => {
  assert.equal(await askCodex(codex, "Sort these", { schema: { type: "object" }, ...opts }), "All quiet.");
  const schema = asked().args[asked().args.indexOf("--output-schema") + 1];
  assert.ok(schema.endsWith("schema.json"));
  assert.equal(fs.existsSync(schema), false, "its folder is gone afterwards");

  process.env.FAKE_CODEX = "fail";
  try {
    await assert.rejects(askCodex(codex, "x", opts), /codex exec exited 3: no auth/);
    process.env.FAKE_CODEX = "hang";
    await assert.rejects(askCodex(codex, "x", { timeoutMs: 300, ...opts }), /codex exec exited null/);
  } finally {
    delete process.env.FAKE_CODEX;
  }
});

test("a codex that quits without reading a long prompt is a failure, not a crash", { skip: !unix }, async () => {
  process.env.FAKE_CODEX = "early";
  try {
    // More than a pipe holds, as the outline of a big projects folder is.
    await assert.rejects(askCodex(codex, "x".repeat(2_000_000), opts), /codex exec exited 3/);
  } finally {
    delete process.env.FAKE_CODEX;
  }
});

test("a codex that can't be started leaves nothing behind", async () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-ask-"));
  const before = process.env.TMPDIR;
  process.env.TMPDIR = tmp;
  try {
    await assert.rejects(askCodex(path.join(bin, "no-such-codex"), "x", { timeoutMs: 1000, ...opts }), /ENOENT/);
  } finally {
    process.env.TMPDIR = before;
  }
  assert.deepEqual(fs.readdirSync(tmp), [], "no folder left in the temp folder");
  fs.rmSync(tmp, { recursive: true, force: true });
});
