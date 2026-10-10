import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { LiveTelemetry, categorize, mcpServerOf, dayKey } from "../core/live.js";

const CLAUDE_ID = "11111111-2222-3333-4444-555555555555";
const CODEX_ID = "0190ffff-0000-7000-8000-000000000001";
const line = (o) => JSON.stringify(o) + "\n";
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// Real paths, as the tailer keeps them (on macOS the temp folder is a link).
function setup() {
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-live-")));
  const claudeRoot = path.join(root, "claude", "projects");
  const codexRoot = path.join(root, ".codex", "sessions");
  fs.mkdirSync(path.join(claudeRoot, "C--proj", CLAUDE_ID, "subagents"), { recursive: true });
  fs.mkdirSync(path.join(codexRoot, "2026", "10", "02"), { recursive: true });
  return {
    root,
    claudeRoot,
    codexRoot,
    claudeFile: path.join(claudeRoot, "C--proj", `${CLAUDE_ID}.jsonl`),
    subFile: path.join(claudeRoot, "C--proj", CLAUDE_ID, "subagents", "agent-abc123.jsonl"),
    codexFile: path.join(codexRoot, "2026", "10", "02", `rollout-2026-10-02T14-00-00-${CODEX_ID}.jsonl`),
  };
}

test("tool names map to families and MCP servers", () => {
  assert.equal(categorize("Bash"), "shell");
  assert.equal(categorize("Edit"), "edit");
  assert.equal(categorize("mcp__agent-wiki__wiki_log"), "mcp");
  assert.equal(mcpServerOf("mcp__plugin_agent-wiki_agent-wiki__wiki_log"), "agent-wiki");
  assert.equal(mcpServerOf("mcp__Claude_Browser__computer"), "Claude_Browser");
  assert.equal(mcpServerOf("Bash"), null);
});

// dayKey keeps the last day it worked out; it must still say what
// toLocaleDateString says for every time, in order or not.
test("a time's day is the local day toLocaleDateString gives, whatever order times come in", () => {
  const day = (ts) => new Date(ts).toLocaleDateString("en-CA");
  const t0 = Date.parse("2026-03-01T00:00:00Z");
  const times = [];
  for (let t = t0; t < t0 + 40 * 86400e3; t += 37 * 60e3 + 11) times.push(t);
  const local = new Date(t0 + 20 * 86400e3);
  const midnight = new Date(local.getFullYear(), local.getMonth(), local.getDate()).getTime();
  times.push(midnight, midnight - 1, midnight + 1, midnight + 86400e3 - 1, t0 - 400 * 86400e3, t0, t0 + 3 * 86400e3, t0);
  for (const t of times) assert.equal(dayKey(t), day(t), new Date(t).toISOString());
  for (const t of [...times].reverse()) assert.equal(dayKey(t), day(t), new Date(t).toISOString());
  for (const odd of [NaN, Infinity, "2026-10-03T12:00:00Z", String(t0), new Date(t0), null]) assert.equal(dayKey(odd), day(odd), String(odd));
});

test("tails appended Claude and Codex lines into live events and state", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  fs.writeFileSync(f.codexFile, line({ type: "session_meta", timestamp: ts, payload: { id: CODEX_ID } }));

  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  assert.equal(events.length, 0, "history is primed silently");

  fs.appendFileSync(
    f.claudeFile,
    line({
      type: "assistant",
      timestamp: ts,
      message: {
        id: "msg_1",
        model: "claude-x",
        stop_reason: "tool_use",
        usage: { input_tokens: 10, cache_read_input_tokens: 1000, cache_creation_input_tokens: 90, output_tokens: 50 },
        content: [
          { type: "tool_use", id: "tu1", name: "Edit", input: { file_path: "C:\\proj\\shared.js" } },
          { type: "tool_use", id: "tu2", name: "mcp__agent-wiki__wiki_log", input: {} },
        ],
      },
    }),
  );
  fs.appendFileSync(f.subFile, line({ type: "user", timestamp: ts, isSidechain: true, agentId: "abc123", message: { role: "user", content: "Investigate the parser" } }));
  fs.appendFileSync(
    f.codexFile,
    line({ type: "event_msg", timestamp: ts, payload: { type: "task_started", model_context_window: 258400 } }) +
      line({ type: "event_msg", timestamp: ts, payload: { type: "token_count", info: { total_token_usage: { total_tokens: 5000, output_tokens: 300 }, last_token_usage: { total_tokens: 4000 }, model_context_window: 258400 }, rate_limits: { primary: { used_percent: 12, window_minutes: 300 } } } }) +
      line({ type: "event_msg", timestamp: ts, payload: { type: "item_completed", item: { type: "FileChange", changes: { "C:\\proj\\shared.js": {} } } } }),
  );

  // Wait for the events themselves, not a count: under load the files'
  // watch events can arrive in any order.
  const expected = ["claude:tool:edit", "claude:tool:mcp", "claude:subagent-start:Investigate the parser", "codex:turn-start:", "codex:tool:edit"];
  const kinds = () => events.map((e) => `${e.sessionId.split(":")[0]}:${e.kind}:${e.category ?? e.label ?? ""}`);
  for (let i = 0; i < 100 && !expected.every((k) => kinds().includes(k)); i++) await wait(100);
  live.stop();

  for (const k of expected) assert.ok(kinds().includes(k), `${k} missing from ${kinds().join(" ")}`);

  const snap = live.snapshot();
  const claude = snap.sessions.find((s) => s.id === `claude:${CLAUDE_ID}`);
  const codex = snap.sessions.find((s) => s.id === `codex:${CODEX_ID}`);
  assert.equal(claude.context.used, 1150);
  assert.equal(claude.tokens.output, 50);
  assert.equal(claude.status, "tool");
  assert.equal(codex.tokens.processed, 5000);
  assert.equal(codex.context.used, 4000);
  assert.deepEqual(snap.limits.codex.primary, { pct: 12, windowMinutes: 300, resetsAt: null }, "in XenoNet's shape");
  assert.deepEqual(snap.collab.map((e) => e.files), [["shared.js"]], "both sessions edited shared.js");
  assert.deepEqual(snap.resources.map((r) => r.server), ["agent-wiki"]);
  fs.rmSync(f.root, { recursive: true, force: true });
});

// The watcher misses events: macOS starts a watch a moment after fs.watch
// returns, and drops events under load. A sweep has to catch up, new
// transcripts included.
test("a sweep finds what the file watcher missed, new transcripts too", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  live.stop(); // from here on, every watch event is missed

  fs.appendFileSync(f.claudeFile, line({ type: "assistant", timestamp: ts, message: { id: "m1", content: [{ type: "tool_use", id: "tu1", name: "Bash", input: { command: "npm test" } }] } }));
  fs.appendFileSync(f.subFile, line({ type: "user", timestamp: ts, isSidechain: true, message: { role: "user", content: "Investigate the parser" } }));
  fs.writeFileSync(f.codexFile, line({ type: "event_msg", timestamp: ts, payload: { type: "task_started" } }));
  await live.sweep();

  const kinds = events.map((e) => `${e.sessionId.split(":")[0]}:${e.kind}:${e.detail ?? e.label ?? ""}`);
  assert.deepEqual(kinds.sort(), ["claude:subagent-start:Investigate the parser", "claude:tool:npm test", "codex:turn-start:"]);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a sweep finds transcripts in folders made since start, and forgets folders that went", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  live.stop();

  // A new project folder, and the subagents folder taken away.
  const other = "22222222-3333-4444-5555-666666666666";
  fs.mkdirSync(path.join(f.claudeRoot, "D--other"));
  fs.writeFileSync(path.join(f.claudeRoot, "D--other", `${other}.jsonl`), line({ type: "user", timestamp: ts, message: { role: "user", content: "new project" } }));
  fs.rmSync(path.join(f.claudeRoot, "C--proj", CLAUDE_ID), { recursive: true });
  await live.sweep();

  assert.deepEqual(events.map((e) => `${e.sessionId}:${e.detail}`), [`claude:${other}:new project`]);
  assert.ok(![...live.folders.keys()].some((k) => k.includes(CLAUDE_ID)), "the folders that went are forgotten");
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a transcript from days ago that's written to again replays nothing from before", async () => {
  const f = setup();
  const old = new Date(Date.now() - 3 * 86400e3);
  fs.writeFileSync(
    f.claudeFile,
    line({ type: "user", timestamp: old.toISOString(), message: { role: "user", content: "an old prompt" } }) +
      line({ type: "assistant", timestamp: old.toISOString(), message: { id: "m0", stop_reason: "end_turn", content: [{ type: "tool_use", id: "tu0", name: "Bash", input: { command: "an old command" } }] } }),
  );
  fs.utimesSync(f.claudeFile, old, old);
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  const events = [];
  live.on("activity", (e) => events.push(e));
  await live.start();
  live.stop();

  // The session is picked up again: a watch event says the file changed.
  fs.appendFileSync(f.claudeFile, line({ type: "user", timestamp: new Date().toISOString(), message: { role: "user", content: "picking this up again" } }));
  await live.track(f.claudeFile, { prime: false });

  assert.deepEqual(events.map((e) => `${e.kind}:${e.detail}`), ["prompt:picking this up again"]);
  // What it held before still counts as its history.
  assert.equal(live.sessionNotes(`claude:${CLAUDE_ID}`).firstPrompt.text, "an old prompt");
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("two reads of a transcript at once take each new line once", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  await live.start();
  live.stop();
  const events = [];
  live.on("activity", (e) => events.push(e));

  // A watch event and a sweep can both start a read before the other ends.
  const calls = Array.from({ length: 200 }, (_, i) => line({ type: "assistant", timestamp: ts, message: { id: `m${i}`, content: [{ type: "tool_use", id: `tu${i}`, name: "Bash", input: { command: `step ${i}` } }] } }));
  fs.appendFileSync(f.claudeFile, calls.join(""));
  await Promise.all([live.track(f.claudeFile, { prime: false }), live.track(f.claudeFile, { prime: false })]);

  assert.equal(events.length, 200);
  assert.equal(new Set(events.map((e) => e.detail)).size, 200);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a transcript rewritten from the top doesn't keep the half line from before", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  const prompt = (text) => line({ type: "user", timestamp: ts, message: { role: "user", content: text } });
  fs.writeFileSync(f.claudeFile, prompt("a first prompt, long enough to outlast the rewrite") + prompt("half written").slice(0, 40));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  await live.start();
  live.stop();
  const events = [];
  live.on("activity", (e) => events.push(e));

  fs.writeFileSync(f.claudeFile, prompt("over again"));
  await live.track(f.claudeFile, { prime: false });
  assert.deepEqual(events.map((e) => e.detail), ["over again"]);
  fs.rmSync(f.root, { recursive: true, force: true });
});

test("a line that two reads split inside a character reads whole", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  await live.start();
  live.stop();
  const events = [];
  live.on("activity", (e) => events.push(e));

  const bytes = Buffer.from(line({ type: "user", timestamp: ts, message: { role: "user", content: "Déjà vu: 日本語 ✓" } }));
  const cut = bytes.indexOf(Buffer.from("日")) + 1; // inside a three-byte character
  fs.appendFileSync(f.claudeFile, bytes.subarray(0, cut));
  await live.track(f.claudeFile, { prime: false });
  fs.appendFileSync(f.claudeFile, bytes.subarray(cut));
  await live.track(f.claudeFile, { prime: false });
  assert.deepEqual(events.map((e) => e.detail), ["Déjà vu: 日本語 ✓"]);
  fs.rmSync(f.root, { recursive: true, force: true });
});

// A file mode stops its owner reading it, except on Windows or as root.
const modesBind = process.platform !== "win32" && process.getuid?.() !== 0;

test("a transcript XenoNet can't read, or a line its normalizer can't take, costs only that: start goes on", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", timestamp: ts, message: { role: "user", content: "start" } }));
  // A rollout with a line whose payload type turns into no string, and a
  // line after it.
  fs.writeFileSync(
    f.codexFile,
    line({ type: "session_meta", timestamp: ts, payload: { id: CODEX_ID, cli_version: "0.150.0" } }) +
      line({ type: "event_msg", timestamp: ts, payload: { type: { toString: 1 } } }) +
      line({ type: "turn_context", timestamp: ts, payload: { model: "gpt-made-up" } }),
  );
  // A subagent's transcript another user wrote, say.
  fs.writeFileSync(f.subFile, line({ type: "user", timestamp: ts, isSidechain: true, message: { role: "user", content: "a subagent's task" } }));
  if (modesBind) fs.chmodSync(f.subFile, 0o200);
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  try {
    await live.start();
    assert.equal(live.sessions.get(`codex:${CODEX_ID}`).model, "gpt-made-up", "the line after the odd one is read");
    assert.ok(live.sessions.has(`claude:${CLAUDE_ID}`));
    assert.equal(live.status.watching, true);
    assert.ok(live.limitTimer, "plan limits are read too");
    if (modesBind) assert.deepEqual([live.tailer.state, [...live.dormant.keys()]], ["degraded", [f.subFile]]);
  } finally {
    live.stop();
    fs.rmSync(f.root, { recursive: true, force: true });
  }
});

test("an event that arrives late takes its place in the history", () => {
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  const s = live.session(`claude:${CLAUDE_ID}`, "claude");
  const now = Date.now();
  // Arriving in this order: a transcript the sweep found late brings one
  // from 40 seconds ago.
  for (const ago of [5, 4, 40, 3]) live.activity(s, { kind: "tool", detail: `${ago}s ago`, ts: now - ago * 1000 }, true);
  assert.deepEqual(live.historySince(now - 10e3).map((e) => e.detail), ["5s ago", "4s ago", "3s ago"]);
  assert.deepEqual(live.historySince(0).map((e) => e.detail), ["40s ago", "5s ago", "4s ago", "3s ago"]);
});

test("an old Claude usage reading reads as missing, with when it was taken", () => {
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: null });
  const now = Date.parse("2026-10-03T12:00:00Z");
  const sample = (minutesAgo, fh, sd) => ({ ts: now - minutesAgo * 60e3, fiveHour: fh, sevenDay: sd });
  live.limits.claude = { fiveHour: 9, sevenDay: 19, observedAt: now - 10 * 60e3 };
  live.limitSamples.claude = [sample(40, 4, 18), sample(10, 9, 19)];
  assert.deepEqual(live.claudeLimits(now), live.limits.claude);
  assert.equal(live.limitForecast(now).claudeFiveHour.pct, 9);
  const old = now - 2 * 86400e3;
  live.limits.claude = { fiveHour: 0, sevenDay: 0, observedAt: old };
  live.limitSamples.claude = [{ ts: old, fiveHour: 0, sevenDay: 0 }];
  assert.deepEqual(live.claudeLimits(now), { fiveHour: null, sevenDay: null, observedAt: old, stale: true, reason: null });
  assert.equal(live.limitForecast(now).claudeFiveHour, undefined);
});

test("Claude usage comes from Anthropic every 5 minutes, backs off when asked, and the newer reading wins", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-usage-"));
  let now = Date.parse("2026-10-03T12:00:00Z");
  const answers = [];
  let asks = 0;
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: dir, claudeUsage: async () => (asks++, answers.shift()) });
  const reading = (fh, sd) => ({ usage: { fiveHour: fh, sevenDay: sd, resets: { fiveHour: now + 2 * 3600e3, sevenDay: now + 3 * 86400e3 }, observedAt: now, source: "anthropic" } });

  answers.push(reading(10, 6));
  await live.readClaudeLimits(now);
  assert.equal(live.claudeLimits(now).fiveHour, 10);
  assert.equal(live.limitForecast(now).claudeFiveHour.resetsAt, now + 2 * 3600e3);

  now += 60e3;
  await live.readClaudeLimits(now);
  assert.equal(asks, 1, "not asked again within 5 minutes");

  now += 5 * 60e3;
  answers.push({ reason: "Anthropic asked to wait (429)", retryAfterMs: 20 * 60e3 });
  await live.readClaudeLimits(now);
  assert.equal(asks, 2);
  now += 10 * 60e3;
  await live.readClaudeLimits(now);
  assert.equal(asks, 2, "waits as long as Anthropic asked");

  // Claude desktop's own sample, newer than Anthropic's last answer, wins.
  fs.writeFileSync(path.join(dir, "plan-usage-history.json"), JSON.stringify({ version: 2, samples: [{ t: now - 60e3, org: null, u: { fh: 14, sd: 7 } }] }));
  await live.readClaudeLimits(now);
  assert.equal(live.claudeLimits(now).fiveHour, 14);

  // Nothing fresh for an hour: missing, with when and why.
  now += 60 * 60e3;
  answers.push({ reason: "no Claude Code sign-in found" });
  await live.readClaudeLimits(now);
  assert.deepEqual(live.claudeLimits(now), { fiveHour: null, sevenDay: null, observedAt: now - 60 * 60e3 - 60e3, stale: true, reason: "no Claude Code sign-in found" });
  fs.rmSync(dir, { recursive: true, force: true });
});

test("Claude desktop's samples: a file that goes bad or away keeps the samples there were; a reading that throws gives its reason", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-usage-"));
  const now = Date.parse("2026-10-03T12:00:00Z");
  const file = path.join(dir, "plan-usage-history.json");
  const live = new LiveTelemetry({ claudeRoot: "/nowhere", codexRoot: "/nowhere", claudeDesktopDir: dir, claudeUsage: async () => { throw new Error("socket hang up"); } });
  await live.readClaudeLimits(now);
  assert.deepEqual(live.limitSamples.claude, [], "no file yet: no samples");
  assert.deepEqual(live.claudeLimits(now), { fiveHour: null, sevenDay: null, observedAt: null, stale: true, reason: "socket hang up" });

  fs.writeFileSync(file, JSON.stringify({ samples: [{ t: now - 60e3, u: { fh: 20, sd: 5 } }, { t: now - 30e3 }] }));
  await live.readClaudeLimits(now);
  assert.deepEqual(live.limitSamples.claude.map((s) => [s.fiveHour, s.source]), [[20, "claude-desktop"]], "samples without usage are left out");
  for (const bad of ["{ half", "null", JSON.stringify({ samples: { not: "a list" } })]) {
    fs.writeFileSync(file, bad);
    await live.readClaudeLimits(now);
    assert.deepEqual(live.limitSamples.claude.map((s) => s.fiveHour), [20], bad);
  }
  fs.rmSync(file);
  await live.readClaudeLimits(now);
  assert.deepEqual(live.limitSamples.claude.map((s) => s.fiveHour), [20], "gone");
  assert.equal(live.claudeLimits(now).fiveHour, 20);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("a provider's refusal ends the turn, is in the feed as one, and is told once, with where it was written", async () => {
  const f = setup();
  const ts = new Date().toISOString();
  fs.writeFileSync(f.claudeFile, line({ type: "user", uuid: "u0", timestamp: ts, message: { role: "user", content: "start" } }));
  fs.writeFileSync(f.codexFile, line({ type: "session_meta", timestamp: ts, payload: { id: CODEX_ID } }));
  const live = new LiveTelemetry({ claudeRoot: f.claudeRoot, codexRoot: f.codexRoot, claudeDesktopDir: null });
  const events = [], refusals = [];
  live.on("activity", (e) => events.push(e));
  live.on("refusal", (e) => refusals.push(e));
  await live.start();
  fs.appendFileSync(f.claudeFile, line({ type: "user", uuid: "u1", parentUuid: "u0", timestamp: ts, message: { role: "user", content: "do the thing" } }));
  fs.appendFileSync(f.claudeFile, line({ type: "assistant", uuid: "a1", parentUuid: "u1", isApiErrorMessage: true, requestId: "req_1", timestamp: ts, message: { id: "msg_r", model: "<synthetic>", role: "assistant", content: [{ type: "text", text: "API Error: refused" }], stop_reason: "refusal", stop_details: { category: "cyber", explanation: "It looks like an attack." } } }));
  fs.appendFileSync(f.codexFile, line({ type: "event_msg", timestamp: ts, payload: { type: "task_started" } }) + line({ type: "event_msg", timestamp: ts, payload: { type: "error", message: "flagged", codex_error_info: "cyber_policy" } }));
  for (let i = 0; i < 100 && (refusals.length < 2 || events.filter((e) => e.kind === "refusal").length < 2); i++) await wait(100);
  live.stop();
  const claude = refusals.find((r) => r.app === "claude");
  assert.deepEqual([claude.signal.kind, claude.signal.category, claude.live, claude.file], ["refusal", "cyber", true, f.claudeFile]);
  assert.equal(refusals.find((r) => r.app === "codex").signal.category, "cyber_policy");
  assert.deepEqual(events.filter((e) => e.kind === "refusal").map((e) => [e.app, e.detail]).sort(), [["claude", "It looks like an attack."], ["codex", "flagged"]]);
  assert.equal(live.sessions.get(`claude:${CLAUDE_ID}`).turn.active, false, "the refused turn is over, not thinking for ten minutes");
  assert.equal(live.sessions.get(`codex:${CODEX_ID}`).turn.active, false);
  fs.rmSync(f.root, { recursive: true, force: true });
});
