import { test } from "node:test";
import assert from "node:assert/strict";
import { LiveTelemetry, looksLikeQuestion, commandText } from "../core/live.js";

const CL = { app: "claude", sessionId: "claude:aaaaaaaa-0000-0000-0000-000000000001" };
const CX = { app: "codex", sessionId: "codex:bbbbbbbb-0000-0000-0000-000000000002" };
const iso = (ms) => new Date(ms).toISOString();

function fresh() {
  return new LiveTelemetry({ claudeRoot: null, codexRoot: null, claudeDesktopDir: null });
}
const claudeAssistant = (t, msgId, content, extra = {}) => ({ type: "assistant", timestamp: iso(t), message: { id: msgId, content, ...extra } });

test("question detection", () => {
  assert.ok(looksLikeQuestion("Done. Want me to push it?"));
  assert.ok(looksLikeQuestion("All set.\n\nLet me know which option you prefer."));
  assert.ok(!looksLikeQuestion("Finished the migration and all tests pass."));
});

test("needs-you: asked a question, a pending AskUserQuestion, or ready for review", () => {
  const live = fresh();
  const t = Date.now() - 60e3;
  live.applyClaude(CL, { type: "user", timestamp: iso(t), message: { content: "fix the build" } }, false);
  live.applyClaude(CL, claudeAssistant(t + 1000, "m1", [{ type: "text", text: "Fixed it. Should I also update the docs?" }], { stop_reason: "end_turn" }), false);
  let s = live.sessions.get(CL.sessionId).snapshot(Date.now());
  assert.equal(s.needsYou.kind, "question");

  live.applyClaude(CL, { type: "user", timestamp: iso(t + 2000), message: { content: "yes" } }, false);
  s = live.sessions.get(CL.sessionId).snapshot(Date.now());
  assert.equal(s.needsYou, null, "a new prompt clears it");

  live.applyClaude(CL, claudeAssistant(t + 3000, "m2", [{ type: "tool_use", id: "q1", name: "AskUserQuestion", input: { questions: [{ question: "Which database?" }] } }]), false);
  s = live.sessions.get(CL.sessionId).snapshot(Date.now());
  assert.deepEqual([s.needsYou.kind, s.needsYou.text], ["question", "Which database?"]);

  live.applyCodex(CX, { type: "event_msg", timestamp: iso(t), payload: { type: "task_started" } }, false);
  live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + 5000), payload: { type: "task_complete", last_agent_message: "Deployed and verified." } }, false);
  s = live.sessions.get(CX.sessionId).snapshot(Date.now());
  assert.equal(s.needsYou.kind, "review");
});

test("stuck: repeated commands and error streaks", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  live.applyClaude(CL, { type: "user", timestamp: iso(t), message: { content: "run tests" } }, false);
  for (let i = 0; i < 4; i++) {
    live.applyClaude(CL, claudeAssistant(t + i * 1000, `m${i}`, [{ type: "tool_use", id: `t${i}`, name: "Bash", input: { command: "npm test" } }]), false);
    live.applyClaude(CL, { type: "user", timestamp: iso(t + i * 1000 + 500), message: { content: [{ type: "tool_result", tool_use_id: `t${i}`, is_error: true }] } }, false);
  }
  const s = live.sessions.get(CL.sessionId).snapshot(Date.now());
  assert.ok(s.stuck.reasons.some((r) => r.startsWith("ran npm test 4 times in a row")), s.stuck.reasons.join(";"));
  assert.ok(s.stuck.reasons.includes("4 failed tool calls in a row"));
});

test("stuck ignores repeated edits and interleaved commands; Codex argv uses the script", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  live.applyClaude(CL, { type: "user", timestamp: iso(t), message: { content: "refactor" } }, false);
  for (let i = 0; i < 5; i++) live.applyClaude(CL, claudeAssistant(t + i, `e${i}`, [{ type: "tool_use", id: `e${i}`, name: "Edit", input: { file_path: "C:\\a.js", old_string: `${i}`, new_string: `${i + 1}` } }]), false);
  for (const [i, cmd] of ["npm test", "git diff", "npm test", "npm test"].entries()) live.applyClaude(CL, claudeAssistant(t + 10 + i, `b${i}`, [{ type: "tool_use", id: `b${i}`, name: "Bash", input: { command: cmd } }]), false);
  assert.equal(live.sessions.get(CL.sessionId).snapshot(Date.now()).stuck, null);
  assert.equal(commandText(["C:\\x\\pwsh.exe", "-Command", "Get-ChildItem"]), "Get-ChildItem");
  assert.equal(commandText(["git", "status"]), "git status");
});

test("context forecast from growth per turn", () => {
  const live = fresh();
  const t = Date.now() - 600e3;
  live.applyCodex(CX, { type: "event_msg", timestamp: iso(t), payload: { type: "task_started", model_context_window: 100000 } }, false);
  for (let i = 0; i < 4; i++) {
    live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 10e3), payload: { type: "token_count", info: { total_token_usage: { total_tokens: (i + 1) * 1000 }, last_token_usage: { total_tokens: 20000 + i * 10000 }, model_context_window: 100000 } } }, false);
    live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 10e3 + 5e3), payload: { type: "task_complete" } }, false);
  }
  const f = live.sessions.get(CX.sessionId).forecast();
  assert.equal(f.growthPerTurn, 10000);
  assert.equal(f.turnsLeft, 4, "(90k - 50k) / 10k");
});

test("collisions: both sessions edited the same file, with diffs", () => {
  const live = fresh();
  const t = Date.now() - 60e3;
  live.applyClaude(CL, claudeAssistant(t, "e1", [{ type: "tool_use", id: "x", name: "Edit", input: { file_path: "C:\\repo\\app.js", old_string: "a", new_string: "b" } }]), false);
  live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + 1000), payload: { type: "item_completed", item: { type: "FileChange", changes: { "C:/repo/app.js": { type: "update", unified_diff: "@@ -1 +1 @@\n-a\n+c" } } } } }, false);
  const c = live.collisions();
  assert.equal(c.length, 1);
  assert.deepEqual(c[0].files, ["app.js"]);
  const diffs = live.collisionDiffs(c[0].a, c[0].b);
  assert.equal(diffs.length, 1);
  assert.ok([diffs[0].a[0].diff, diffs[0].b[0].diff].some((d) => d.includes("+ b")));
  assert.ok([diffs[0].a[0].diff, diffs[0].b[0].diff].some((d) => d.includes("+c")));
});

test("history keeps events in order and limit forecast projects usage", () => {
  const live = fresh();
  const now = Date.now();
  for (let i = 0; i < 4; i++) {
    live.applyCodex(CX, { type: "event_msg", timestamp: iso(now - (4 - i) * 3600e3), payload: { type: "token_count", info: {}, rate_limits: { primary: { used_percent: 10 + i * 10, window_minutes: 10080, resets_at: Math.floor((now + 30 * 3600e3) / 1000) } } } }, false);
  }
  const f = live.limitForecast(now);
  assert.equal(Math.round(f.codex.perHour), 10);
  assert.ok(f.codex.hitsBeforeReset, "40% + 10%/h reaches 100% in 6h, before the reset in 30h");
  live.applyCodex(CX, { type: "event_msg", timestamp: iso(now - 1000), payload: { type: "task_started" } }, false);
  assert.equal(live.historySince(now - 5000).length, 1);
});

test("Codex's subagents: one per thread, labelled by its agent path, in the names rollouts use", () => {
  const live = fresh();
  const now = Date.now();
  const item = (i, item) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(now - (5 - i) * 1000), payload: { type: "item_completed", item } }, false);
  item(0, { type: "SubAgentActivity", id: "i1", kind: "started", agent_thread_id: "t-sub", agent_path: "/root/explorer" });
  item(1, { type: "SubAgentActivity", id: "i2", kind: "interacted", agent_thread_id: "t-sub", agent_path: "/root/explorer" });
  item(2, { type: "SubAgentActivity", id: "i3", agentThreadId: "t-old", prompt: "Profile the build" });
  const subs = live.snapshot().sessions.find((s) => s.id === CX.sessionId).subagents;
  assert.deepEqual(subs.map((a) => [a.id, a.label]).sort(), [["t-old", "Profile the build"], ["t-sub", "/root/explorer"]]);
});

test("a Codex subagent is done once it completes or is stopped, until it's given more to do", () => {
  const live = fresh();
  const now = Date.now();
  const item = (i, kind) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(now - (9 - i) * 1000), payload: { type: "item_completed", item: { type: "SubAgentActivity", id: `i${i}`, kind, agent_thread_id: "t-sub", agent_path: "/root/worker" } } }, false);
  const sub = () => live.snapshot().sessions.find((s) => s.id === CX.sessionId).subagents[0];
  item(0, "started");
  assert.deepEqual([sub().done, sub().active], [false, true]);
  item(1, "interrupted");
  assert.deepEqual([sub().done, sub().active], [true, false], "stopped");
  item(2, "interacted");
  assert.deepEqual([sub().done, sub().active], [false, true], "given more to do");
  item(3, "completed");
  item(4, "completed");
  assert.deepEqual([sub().done, sub().active], [true, false]);
  assert.deepEqual(live.historySince(0).map((e) => e.kind), ["subagent-start", "subagent-stop", "subagent-stop"], "a stop each time it stops");
});

test("a compacted summary and an interruption aren't prompts", () => {
  const live = fresh();
  const t = Date.now() - 60e3;
  live.applyClaude(CL, { type: "user", timestamp: iso(t), message: { content: "fix the build" } }, false);
  live.applyClaude(CL, claudeAssistant(t + 1000, "m1", [{ type: "text", text: "Fixed." }], { stop_reason: "end_turn" }), false);
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 2000), isCompactSummary: true, message: { content: "This session is being continued from a previous conversation." } }, false);
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 3000), message: { content: [{ type: "text", text: "[Request interrupted by user for tool use]" }] } }, false);
  assert.deepEqual(live.sessionNotes(CL.sessionId).prompts.map((p) => p.text), ["fix the build"]);
  assert.deepEqual(live.historySince(0).map((e) => e.kind), ["prompt", "message", "turn-end"]);
  assert.equal(live.sessions.get(CL.sessionId).snapshot(Date.now()).status, "idle", "neither starts a turn");
});

test("an interruption ends the turn you stopped, which says so and isn't ready for review", () => {
  const live = fresh();
  const t = Date.now() - 60e3;
  const snap = () => live.sessions.get(CL.sessionId).snapshot(Date.now());
  live.applyClaude(CL, { type: "user", timestamp: iso(t), message: { content: "migrate the database" } }, false);
  live.applyClaude(CL, claudeAssistant(t + 1000, "m1", [{ type: "text", text: "Should I back it up first?" }, { type: "tool_use", id: "b1", name: "Bash", input: { command: "npm run migrate" } }]), false);
  assert.equal(snap().status, "tool");
  // A tool use you turned down: its result, then Claude Code's note.
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 2000), message: { content: [{ type: "tool_result", tool_use_id: "b1", is_error: true, content: "The user doesn't want to proceed with this tool use." }] } }, false);
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 2500), message: { content: [{ type: "text", text: "[Request interrupted by user for tool use]" }] } }, false);
  let s = snap();
  assert.deepEqual([s.status, s.turnStartedAt, s.lastTurnEndAt, s.interruptedAt, s.needsYou], ["idle", null, t + 2500, t + 2500, null], "not a question, nor ready for review");
  assert.deepEqual(live.historySince(0).filter((e) => e.kind === "turn-end").map((e) => [e.duration, e.interrupted]), [[2500, true]]);
  assert.deepEqual(live.sessionNotes(CL.sessionId).prompts.map((p) => p.text), ["migrate the database"], "and no prompt");

  // The next turn starts afresh, and one that ends by itself is for review.
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 3000), message: { content: "back it up, then migrate" } }, false);
  assert.equal(snap().interruptedAt, null);
  live.applyClaude(CL, claudeAssistant(t + 4000, "m2", [{ type: "text", text: "Backed up and migrated." }], { stop_reason: "end_turn" }), false);
  s = snap();
  assert.deepEqual([s.interruptedAt, s.needsYou.kind], [null, "review"]);
  assert.equal(live.historySince(0).filter((e) => e.kind === "turn-end").at(-1).interrupted, undefined);

  // Esc while it thinks, the note in the same line as a result; and one in
  // a subagent's file, which leaves the session's turn alone.
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 5000), message: { content: "now the seed data" } }, false);
  live.applyClaude({ ...CL, agentId: "a1" }, { type: "user", timestamp: iso(t + 5500), isSidechain: true, message: { content: "[Request interrupted by user]" } }, false);
  assert.equal(snap().status, "thinking");
  live.applyClaude(CL, { type: "user", timestamp: iso(t + 6000), message: { content: [{ type: "tool_result", tool_use_id: "x9", content: "ok" }, { type: "text", text: "[Request interrupted by user]" }] } }, false);
  assert.deepEqual([snap().status, snap().interruptedAt], ["idle", t + 6000]);
});

test("a Codex turn you stop (turn_aborted) ends there, says so and isn't ready for review", () => {
  const live = fresh();
  const t = Date.now() - 60e3;
  const snap = () => live.sessions.get(CX.sessionId).snapshot(Date.now());
  const line = (ms, type, payload) => live.applyCodex(CX, { type, timestamp: iso(t + ms), payload }, false);
  const said = (ms, item, text) => line(ms, "event_msg", { type: "item_completed", item: { type: item, content: [{ type: "text", text }] } });
  const ends = () => live.historySince(0).filter((e) => e.kind === "turn-end").map((e) => [e.duration, e.interrupted]);
  line(0, "event_msg", { type: "task_started" });
  said(10, "UserMessage", "migrate the database");
  said(1000, "AgentMessage", "Should I back it up first?");
  line(1500, "response_item", { type: "function_call", call_id: "c1", name: "shell", arguments: "{}" });
  assert.equal(snap().status, "tool");
  // Esc while the call runs: Codex writes turn_aborted, and no task_complete.
  line(2500, "event_msg", { type: "turn_aborted", reason: "interrupted" });
  let s = snap();
  assert.deepEqual([s.status, s.turnStartedAt, s.lastTurnEndAt, s.interruptedAt, s.needsYou], ["idle", null, t + 2500, t + 2500, null], "not a question, nor ready for review");
  assert.deepEqual(ends(), [[2500, true]]);

  // The next turn starts afresh, and one that ends by itself is for review.
  line(3000, "event_msg", { type: "task_started" });
  said(3010, "UserMessage", "back it up, then migrate");
  assert.equal(snap().interruptedAt, null);
  line(4000, "event_msg", { type: "task_complete", last_agent_message: "Backed up and migrated." });
  s = snap();
  assert.deepEqual([s.status, s.interruptedAt, s.needsYou.kind], ["idle", null, "review"]);
  assert.deepEqual(ends(), [[2500, true], [null, undefined]]);
  // With no turn under way, there's nothing to end.
  line(5000, "event_msg", { type: "turn_aborted", reason: "interrupted" });
  assert.deepEqual([snap().interruptedAt, ends().length], [null, 2]);
  // A turn the next one replaces wasn't stopped by you, and neither is a
  // review that ends.
  line(6000, "event_msg", { type: "task_started" });
  said(6010, "UserMessage", "and the seed data");
  line(6500, "event_msg", { type: "turn_aborted", reason: "replaced" });
  line(6600, "event_msg", { type: "turn_aborted", reason: "review_ended" });
  assert.deepEqual([snap().interruptedAt, ends().length], [null, 2]);
});

test("a Codex call's output ends it, a shell call's too", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  const item = (i, payload) => live.applyCodex(CX, { type: "response_item", timestamp: iso(t + i * 1000), payload }, false);
  const status = () => live.sessions.get(CX.sessionId).snapshot(Date.now()).status;
  live.applyCodex(CX, { type: "event_msg", timestamp: iso(t), payload: { type: "task_started" } }, false);
  item(1, { type: "local_shell_call", call_id: "s1", status: "completed", action: { type: "exec", command: ["make"] } });
  assert.equal(status(), "tool");
  item(2, { type: "local_shell_call_output", call_id: "s1", output: "ok" });
  assert.equal(status(), "thinking", "not in flight for the ten minutes until the turn goes stale");
  // Matched by call_id, else id, as stats match them.
  item(3, { type: "function_call", id: "f1", name: "exec_command" });
  item(4, { type: "function_call_output", id: "f1" });
  item(5, { type: "custom_tool_call", call_id: "c1", name: "apply_patch", input: "" });
  item(6, { type: "custom_tool_call_output", call_id: "c1" });
  assert.equal(live.sessions.get(CX.sessionId).inflight.size, 0);
});

test("a blank Codex user message, an image alone, isn't a prompt, nor the first", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  const said = (i, content) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 1000), payload: { type: "item_completed", item: { type: "UserMessage", content } } }, false);
  said(0, [{ type: "local_image", path: "/tmp/screenshot.png" }]);
  said(1, [{ type: "text", text: "  " }, null]);
  said(2, [{ type: "text", text: "what's wrong in this screenshot?" }]);
  const notes = live.sessionNotes(CX.sessionId);
  assert.deepEqual([notes.firstPrompt.text, notes.prompts.map((p) => p.text)], ["what's wrong in this screenshot?", ["what's wrong in this screenshot?"]]);
  assert.deepEqual(live.historySince(0).map((e) => e.kind), ["prompt"]);
});

test("a Codex user message that starts with < (a delegation envelope) isn't a prompt", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  const said = (i, text) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 1000), payload: { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } } }, false);
  said(0, "<subagent_notification>the explorer finished</subagent_notification>");
  said(1, "add retries to the uploader");
  said(2, "<turn_aborted>");
  assert.deepEqual(live.sessionNotes(CX.sessionId).prompts.map((p) => p.text), ["add retries to the uploader"]);
  assert.equal(live.sessionNotes(CX.sessionId).firstPrompt.text, "add retries to the uploader");
});

test("a Codex rollout's prompts: its user_message events until it has a completed user message, then those alone", () => {
  const t = Date.now() - 60e3;
  const at = (i) => iso(t + i * 1000);
  const line = (live, i, payload) => live.applyCodex(CX, { type: "event_msg", timestamp: at(i), payload }, false);
  const event = (live, i, message) => line(live, i, { type: "user_message", message });
  const item = (live, i, text) => line(live, i, { type: "item_completed", item: { type: "UserMessage", content: [{ type: "text", text }] } });
  const prompts = (live) => live.sessionNotes(CX.sessionId).prompts.map((p) => p.text);

  // codex exec writes only events: they're its prompts, but for a blank one
  // and one that starts with "<".
  const exec = fresh();
  line(exec, 0, { type: "task_started" });
  event(exec, 0, "summarize the failing tests");
  event(exec, 1, "   ");
  event(exec, 2, "<environment_context>…</environment_context>");
  line(exec, 3, { type: "task_complete" });
  event(exec, 4, "and suggest fixes");
  assert.deepEqual(prompts(exec), ["summarize the failing tests", "and suggest fixes"]);
  assert.equal(exec.sessionNotes(CX.sessionId).firstPrompt.text, "summarize the failing tests");

  // A newer Codex may write the event, then the completed user message, in
  // the same turn: one prompt. From then on, events are passed by.
  const newer = fresh();
  line(newer, 0, { type: "task_started" });
  event(newer, 0, "why is the build slow?");
  item(newer, 0, "why is the build slow?");
  line(newer, 1, { type: "task_complete" });
  line(newer, 2, { type: "task_started" });
  event(newer, 2, "an event alone isn't a prompt now");
  item(newer, 3, "add -j8 to the makefile");
  assert.deepEqual(prompts(newer), ["why is the build slow?", "add -j8 to the makefile"]);
  assert.deepEqual(newer.historySince(0).filter((e) => e.kind === "prompt").map((e) => e.detail), ["why is the build slow?", "add -j8 to the makefile"]);

  // An older rollout picked up again: a turn stopped before it ended (no
  // task_complete), then a turn of the newer kind. Both prompts count.
  const old = fresh();
  line(old, 0, { type: "task_started" });
  event(old, 0, "rename the config keys");
  line(old, 1, { type: "turn_aborted" });
  line(old, 2, { type: "task_started" });
  item(old, 2, "now update the docs");
  assert.deepEqual(prompts(old), ["rename the config keys", "now update the docs"]);
});

test("a Codex web search's detail is its query, or with an empty one its action's", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  const search = (i, item) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 1000), payload: { type: "item_completed", item: { type: "WebSearch", ...item } } }, false);
  search(0, { query: "", action: { type: "search", query: "zod v4 coerce" } });
  search(1, { query: "node test runner mocks", action: { type: "search", query: "older" } });
  search(2, { action: { type: "open_page", url: "https://example.com" } });
  assert.deepEqual(live.historySince(0).map((e) => e.detail), ["zod v4 coerce", "node test runner mocks", ""]);
});

test("a Codex extension's web search is a web search; its other kinds aren't read", () => {
  const live = fresh();
  const t = Date.now() - 30e3;
  const extension = (i, item) => live.applyCodex(CX, { type: "event_msg", timestamp: iso(t + i * 1000), payload: { type: "item_completed", item: { type: "Extension", id: `x${i}`, ...item } } }, false);
  extension(0, { kind: "web.search", query: "", action: { type: "search", query: "zod v4 coerce" }, results: [] });
  extension(1, { kind: "image_gen.generation", status: "completed", revisedPrompt: "a ledger" });
  extension(2, { kind: "clock.sleep" });
  extension(3, { kind: "web.search", query: "node test runner mocks", action: { type: "search", query: null, queries: ["node test runner mocks"] } });
  assert.deepEqual(live.historySince(0).map((e) => [e.tool ?? e.kind, e.detail]), [["web_search", "zod v4 coerce"], ["web_search", "node test runner mocks"]]);
});
