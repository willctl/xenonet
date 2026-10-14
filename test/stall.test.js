import { test } from "node:test";
import assert from "node:assert/strict";
import { LiveTelemetry } from "../core/live.js";

const t = Date.now() - 120e3;
const iso = (at) => new Date(at).toISOString();
const cx = { app: "codex", sessionId: "codex:bbbbbbbb-0000-0000-0000-000000000002" };
const cl = { app: "claude", sessionId: "claude:aaaaaaaa-0000-0000-0000-000000000001" };
function fresh() {
  const live = new LiveTelemetry({ claudeRoot: null, codexRoot: null, claudeDesktopDir: null });
  live.applyCodex(cx, { type: "event_msg", timestamp: iso(t), payload: { type: "task_started" } }, false);
  return live;
}
const event = (live, at, payload) => live.applyCodex(cx, { type: "event_msg", timestamp: iso(at), payload }, false);
const call = (live, at, args, ok = true) => event(live, at, { type: "item_completed", item: { type: "McpToolCall", server: "example", tool: "list_channel_messages", arguments: args, status: ok ? "completed" : "failed", ...(ok ? {} : { error: "temporary failure" }) } });
const snapshot = (live, at) => live.sessions.get(cx.sessionId).snapshot(at);

test("successful channel searches, pagination and identical polling calls are progress", () => {
  const live = fresh();
  for (let i = 0; i < 20; i++) {
    call(live, t + i * 1000, { channel_id: `channel-${i}`, cursor: `page-${i}` });
    assert.equal(snapshot(live, t + i * 1000).stuck, null);
  }
  for (let i = 0; i < 8; i++) call(live, t + 30000 + i * 1000, { channel_id: "one-channel" });
  assert.equal(snapshot(live, t + 40000).stuck, null, "identical successful calls aren't evidence of a stall");
});

test("a repeated failing call warns, then clears on success, completion and a new turn", () => {
  const live = fresh();
  for (let i = 0; i < 3; i++) call(live, t + i * 1000, { channel_id: "one", top: 50 }, false);
  assert.ok(snapshot(live, t + 3000).stuck.reasons.some((r) => r.includes("3 times in a row")));
  call(live, t + 4000, { channel_id: "one", top: 50 });
  assert.equal(snapshot(live, t + 5000).stuck, null);
  for (let i = 0; i < 3; i++) call(live, t + 6000 + i * 1000, { channel_id: "one" }, false);
  event(live, t + 10000, { type: "task_complete" });
  assert.equal(snapshot(live, t + 11000).stuck, null);
  event(live, t + 12000, { type: "task_started" });
  call(live, t + 13000, { channel_id: "one" }, false);
  assert.equal(snapshot(live, t + 14000).stuck, null, "the previous turn's failures don't carry over");
});

test("MCP identity includes full arguments and ignores object key order", () => {
  const live = fresh();
  const prefix = "x".repeat(600);
  for (let i = 0; i < 4; i++) call(live, t + i * 1000, { prefix, channel_id: `channel-${i}` }, false);
  assert.equal(snapshot(live, t + 4000).stuck, null, "different calls aren't an identical retry loop");
  for (let i = 0; i < 3; i++) call(live, t + 5000 + i * 1000, i % 2 ? { top: 50, channel_id: "one" } : { channel_id: "one", top: 50 }, false);
  assert.ok(snapshot(live, t + 9000).stuck);
  assert.ok(live.sessions.get(cx.sessionId).recentTools.every((r) => !r.key.includes(prefix)), "only digests are kept for comparison");
});

test("different failed calls need sustained failure; a quick parallel failure burst is not a stall", () => {
  const live = fresh();
  for (let i = 0; i < 5; i++) call(live, t + i * 1000, { channel_id: `channel-${i}` }, false);
  assert.equal(snapshot(live, t + 5000).stuck, null);
  call(live, t + 65000, { channel_id: "another" }, false);
  assert.deepEqual(snapshot(live, t + 66000).stuck.reasons, ["6 failed tool calls in a row"]);
  call(live, t + 67000, { channel_id: "another" });
  assert.equal(snapshot(live, t + 68000).stuck, null);
});

test("Claude retries require completed failures; a pending retry isn't a failed call", () => {
  const live = fresh();
  live.applyClaude(cl, { type: "user", timestamp: iso(t), message: { content: "Search channel messages" } }, false);
  for (let i = 0; i < 3; i++) {
    live.applyClaude(cl, { type: "assistant", timestamp: iso(t + i * 1000), message: { id: `m${i}`, content: [{ type: "tool_use", id: `c${i}`, name: "mcp__example__search", input: { query: "one" } }] } }, false);
    live.applyClaude(cl, { type: "user", timestamp: iso(t + i * 1000 + 500), message: { content: [{ type: "tool_result", tool_use_id: `c${i}`, is_error: true }] } }, false);
  }
  const s = live.sessions.get(cl.sessionId);
  assert.ok(s.snapshot(t + 3000).stuck);
  live.applyClaude(cl, { type: "assistant", timestamp: iso(t + 4000), message: { id: "next", content: [{ type: "tool_use", id: "next", name: "mcp__example__search", input: { query: "one" } }] } }, false);
  assert.equal(s.snapshot(t + 5000).stuck, null);
  live.applyClaude(cl, { type: "user", timestamp: iso(t + 6000), message: { content: [{ type: "tool_result", tool_use_id: "next", is_error: false }] } }, false);
  assert.equal(s.errorStreak, 0);
});

test("a successful parallel call clears a warning even when it started before the failed calls", () => {
  const live = fresh();
  live.applyClaude(cl, { type: "user", timestamp: iso(t), message: { content: "Search and read" } }, false);
  live.applyClaude(cl, { type: "assistant", timestamp: iso(t), message: { id: "read", content: [{ type: "tool_use", id: "read", name: "Read", input: { file_path: "~/example.md" } }] } }, false);
  for (let i = 0; i < 3; i++) {
    live.applyClaude(cl, { type: "assistant", timestamp: iso(t + 1000 + i * 1000), message: { id: `m${i}`, content: [{ type: "tool_use", id: `c${i}`, name: "mcp__example__search", input: { query: "one" } }] } }, false);
    live.applyClaude(cl, { type: "user", timestamp: iso(t + 1500 + i * 1000), message: { content: [{ type: "tool_result", tool_use_id: `c${i}`, is_error: true }] } }, false);
  }
  const s = live.sessions.get(cl.sessionId);
  assert.ok(s.snapshot(t + 4000).stuck);
  live.applyClaude(cl, { type: "user", timestamp: iso(t + 5000), message: { content: [{ type: "tool_result", tool_use_id: "read", is_error: false }] } }, false);
  assert.equal(s.snapshot(t + 6000).stuck, null);
});

test("Codex raw tool replies refresh activity, while actual mid-turn silence still warns", () => {
  const live = fresh();
  live.applyCodex(cx, { type: "response_item", timestamp: iso(t + 1000), payload: { type: "function_call", call_id: "call", name: "exec" } }, false);
  assert.equal(snapshot(live, t + 5 * 60e3).stuck, null, "an in-flight tool gets its longer silence allowance");
  live.applyCodex(cx, { type: "response_item", timestamp: iso(t + 5 * 60e3), payload: { type: "function_call_output", call_id: "call", output: "results" } }, false);
  assert.equal(snapshot(live, t + 5 * 60e3 + 1000).stuck, null);
  assert.equal(snapshot(live, t + 5 * 60e3).lastEventAt, t + 5 * 60e3);
  assert.ok(snapshot(live, t + 10 * 60e3).stuck.reasons.some((r) => r.startsWith("silent for")));
});

test("waiting for a person's answer or approval is not stalled work", () => {
  const live = fresh();
  live.applyClaude(cl, { type: "user", timestamp: iso(t), message: { content: "Prepare a plan" } }, false);
  live.applyClaude(cl, { type: "assistant", timestamp: iso(t + 1000), message: { id: "ask", content: [{ type: "tool_use", id: "q", name: "AskUserQuestion", input: { questions: [{ question: "Which option?" }] } }] } }, false);
  assert.equal(live.sessions.get(cl.sessionId).snapshot(t + 16 * 60e3).stuck, null);
});
