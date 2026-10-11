// The live tailer on Grok Build's sessions (core/live.js): updates.jsonl
// lines as they're written, turned into prompts, messages, tool calls,
// edits and context fill; turns that end by going quiet or by Grok's
// hooks; and tokens and cost from the usage ledger. Made-up sessions,
// written as test/fixtures/grok.js writes them, at today's times.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { LiveTelemetry, categorize } from "../core/live.js";

const A = "019a3b5c-0000-7000-8000-0000000000a1";
const B = "019a3b5c-0000-7000-8000-0000000000b2";
const CWD = "/Users/ada/atlas";
const line = (o) => JSON.stringify(o) + "\n";
const up = (ts, update) => line({ timestamp: new Date(ts).toISOString(), update });
const text = (kind, t) => ({ sessionUpdate: kind, content: { type: "text", text: t } });

function setup() {
  const root = fs.realpathSync.native(fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-grok-live-")));
  const grokRoot = path.join(root, ".grok", "sessions");
  const dir = (id) => path.join(grokRoot, encodeURIComponent(CWD), id);
  for (const id of [A, B]) fs.mkdirSync(dir(id), { recursive: true });
  return { root, grokRoot, dir, file: (id) => path.join(dir(id), "updates.jsonl") };
}

test("Grok Build's tool names fall in XenoNet's families", () => {
  assert.equal(categorize("run_terminal_command"), "shell");
  assert.equal(categorize("search_replace"), "edit");
  assert.equal(categorize("write_file"), "edit");
  assert.equal(categorize("read_file"), "read");
  assert.equal(categorize("list_dir"), "read");
  assert.equal(categorize("web_fetch"), "web");
  assert.equal(categorize("spawn_subagent"), "agent");
});

test("only a session's updates.jsonl is read, in its own folder", () => {
  const f = setup();
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: f.grokRoot, claudeDesktopDir: null });
  assert.deepEqual(live.classify(f.file(A)), { app: "grok", sessionId: `grok:${A}` });
  assert.equal(live.classify(path.join(f.dir(A), "chat_history.jsonl")), null);
  assert.equal(live.classify(path.join(f.dir(A), "compaction_checkpoints", "updates.jsonl")), null);
  assert.equal(live.classify(path.join(f.grokRoot, "updates.jsonl")), null);
});

test("a session's updates: its prompt, its tools and edits, its context, and its message once the turn goes quiet", async () => {
  const f = setup();
  const t = Date.now() - 5 * 60e3;
  fs.writeFileSync(
    f.file(A),
    up(t, text("user_message_chunk", "Add partial ")) +
      up(t, text("user_message_chunk", "refunds.")) +
      up(t + 1000, text("agent_message_chunk", "Running ")) +
      up(t + 1000, text("agent_message_chunk", "the tests.")) +
      up(t + 2000, { sessionUpdate: "tool_call", toolCallId: "c1", title: "npm test", kind: "execute", status: "pending", rawInput: { command: "npm test" } }) +
      up(t + 9000, { sessionUpdate: "tool_call_update", toolCallId: "c1", status: "completed" }) +
      up(t + 10000, { sessionUpdate: "tool_call", toolCallId: "c2", title: "Edit refunds.ts", kind: "edit", status: "in_progress", locations: [{ path: `${CWD}/src/refunds.ts` }] }) +
      up(t + 11000, { sessionUpdate: "tool_call_update", toolCallId: "c2", status: "completed", content: [{ type: "diff", path: `${CWD}/src/refunds.ts`, oldText: "refund()", newText: "refund(amount)" }] }) +
      up(t + 12000, { sessionUpdate: "tool_call", toolCallId: "c3", title: "linear", kind: "other", status: "failed", _meta: { toolName: "linear__save_issue" } }) +
      up(t + 13000, { sessionUpdate: "usage_update", used: 41000, size: 500000 }) +
      up(t + 14000, text("agent_message_chunk", "Done. Should I open a PR?")),
  );
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: f.grokRoot, claudeDesktopDir: null });
  await live.start();
  try {
    const s = live.snapshot().sessions.find((x) => x.id === `grok:${A}`);
    assert.ok(s, "the session is in the snapshot");
    assert.equal(s.app, "grok");
    assert.deepEqual(s.toolCounts, { shell: 1, edit: 1, mcp: 1 });
    assert.deepEqual(s.files.map((x) => x.path), [`${CWD}/src/refunds.ts`]);
    assert.deepEqual(s.context, { used: 41000, window: 500000 });
    assert.deepEqual(s.mcp.map((m) => m.server), ["linear"]);
    // Its last word was a question, and it's been quiet: the turn ended,
    // and it waits on you.
    assert.equal(s.status, "idle");
    assert.equal(s.lastMessage, "Done. Should I open a PR?");
    assert.equal(s.needsYou?.kind, "question");
    const kinds = live.history.filter((e) => e.sessionId === `grok:${A}`).map((e) => e.kind);
    assert.deepEqual(kinds.slice(0, 2), ["prompt", "message"]);
    assert.ok(kinds.includes("tool-error"), "the failed call");
    assert.equal(live.history.find((e) => e.kind === "prompt" && e.sessionId === `grok:${A}`).detail, "Add partial");
    assert.equal(live.sessionNotes(`grok:${A}`).prompts[0].text, "Add partial refunds.", "the prompt's chunks, joined");
  } finally {
    live.stop();
  }
});

test("hooks say when a turn starts and ends, and when Grok waits for a permission", async () => {
  const f = setup();
  fs.writeFileSync(f.file(B), up(Date.now() - 1000, text("user_message_chunk", "Fix the build")));
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: f.grokRoot, claudeDesktopDir: null });
  await live.start();
  try {
    const id = `grok:${B}`;
    live.noteGrokHook(id, "UserPromptSubmit", { prompt: "Fix the build" });
    live.noteGrokHook(id, "Notification", { notificationType: "permission_prompt", message: "Allow npm install?" });
    let s = live.snapshot().sessions.find((x) => x.id === id);
    assert.deepEqual([s.status, s.needsYou?.kind, s.needsYou?.text], ["thinking", "permission", "Allow npm install?"]);
    // A turn whose last word was the agent's stays open while hooks report
    // this session's turns: Stop ends it.
    fs.appendFileSync(f.file(B), up(Date.now() - 60e3, text("agent_message_chunk", "Fixed.")));
    await live.track(f.file(B), { prime: false });
    assert.equal(live.snapshot().sessions.find((x) => x.id === id).status, "thinking");
    // A subagent's Stop isn't the session's.
    live.noteGrokHook(id, "Stop", { reason: "end_turn", subagentType: "explore" });
    assert.equal(live.snapshot().sessions.find((x) => x.id === id).status, "thinking");
    live.noteGrokHook(id, "Stop", { reason: "end_turn", lastAssistantMessage: "Fixed." });
    s = live.snapshot().sessions.find((x) => x.id === id);
    assert.deepEqual([s.status, s.needsYou?.kind], ["idle", "review"]);
    // The session's end fires a Stop too, which ends nothing more.
    const ends = () => live.history.filter((e) => e.sessionId === id && e.kind === "turn-end").length;
    const before = ends();
    live.noteGrokHook(id, "Stop", { reason: "channel_closed" });
    assert.equal(ends(), before);
    // You stopping a turn.
    live.noteGrokHook(id, "UserPromptSubmit", {});
    live.noteGrokHook(id, "StopCancelled", { reason: "user_interrupt", cancelledBy: "user" });
    assert.equal(live.snapshot().sessions.find((x) => x.id === id).interruptedAt > 0, true);
  } finally {
    live.stop();
  }
});

test("tokens and cost from the usage ledger: the day's turns when first read, what's new after", async () => {
  const f = setup();
  const now = Date.now();
  fs.writeFileSync(f.file(A), up(now - 120e3, text("user_message_chunk", "Go")));
  const ledger = (turns) =>
    fs.writeFileSync(
      path.join(f.dir(A), "usage.json"),
      JSON.stringify({ sessionId: A, session: { totalTokens: turns.reduce((a, t) => a + t.totalTokens, 0) }, turns: turns.map((t, i) => ({ turnNumber: i + 1, ...t })) }),
    );
  ledger([
    { endedAt: new Date(now - 100e3).toISOString(), model: "grok-4.6", inputTokens: 10000, outputTokens: 1000, totalTokens: 11000, costUsdTicks: 2e8 },
    // Long ago: not today's.
    { endedAt: new Date(now - 3 * 86400e3).toISOString(), model: "grok-4.6", inputTokens: 5000, outputTokens: 0, totalTokens: 5000 },
  ]);
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: f.grokRoot, claudeDesktopDir: null });
  await live.start();
  try {
    const id = `grok:${A}`;
    let snap = live.snapshot();
    let s = snap.sessions.find((x) => x.id === id);
    assert.equal(s.tokens.processed, 11000);
    assert.equal(s.model, "grok-4.6");
    assert.ok(Math.abs(snap.costs.byApp.grok - 0.02) < 1e-9, "Grok's own cost");
    assert.equal(snap.series.grok.length, 300);
    // A third turn, priced by XenoNet: Grok gave no cost.
    ledger([
      { endedAt: new Date(now - 100e3).toISOString(), model: "grok-4.6", inputTokens: 10000, outputTokens: 1000, totalTokens: 11000, costUsdTicks: 2e8 },
      { endedAt: new Date(now - 3 * 86400e3).toISOString(), model: "grok-4.6", inputTokens: 5000, outputTokens: 0, totalTokens: 5000 },
      { endedAt: new Date(now).toISOString(), model: "grok-build-0.1", inputTokens: 100000, outputTokens: 0, totalTokens: 100000 },
    ]);
    const tokens = [];
    live.on("tokens", (ev) => tokens.push(ev));
    live.readGrokUsage(now + 31e3);
    snap = live.snapshot();
    s = snap.sessions.find((x) => x.id === id);
    assert.deepEqual(tokens.map((t) => [t.app, t.n]), [["grok", 100000]]);
    assert.equal(s.model, "grok-build-0.1");
    assert.ok(Math.abs(snap.costs.byApp.grok - (0.02 + 0.1)) < 1e-9, "100K grok-build-0.1 input tokens at $1 a million");
    assert.ok(snap.costs.today >= snap.costs.byApp.grok);
  } finally {
    live.stop();
  }
});

test("two Grok sessions editing one file collide", async () => {
  const f = setup();
  const t = Date.now() - 60e3;
  const edit = (id) => up(t, { sessionUpdate: "tool_call", toolCallId: `e-${id}`, title: "Edit", kind: "edit", status: "completed", content: [{ type: "diff", path: `${CWD}/a.ts`, oldText: "a", newText: "b" }] });
  fs.writeFileSync(f.file(A), edit(A));
  fs.writeFileSync(f.file(B), edit(B));
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, grokRoot: f.grokRoot, claudeDesktopDir: null });
  await live.start();
  try {
    const [c] = live.snapshot().collisions;
    assert.deepEqual([c.a, c.b].sort(), [`grok:${A}`, `grok:${B}`].sort());
    assert.deepEqual(c.files, ["a.ts"]);
  } finally {
    live.stop();
  }
});
