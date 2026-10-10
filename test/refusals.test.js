// Provider refusals (core/refusals): read from the lines Claude Code and
// Codex write, never guessed from what a model said; kept once each, with
// the transcript up to them when you keep context; captured later from a
// transcript still there; exported for a dataset, scrubbed when asked. The
// transcripts, ids and home folder are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { claudeSignal, codexSignal, keyOf } from "../core/refusals/detect.js";
import { createRefusals, contextOn, setContext, claudeContext, codexContext } from "../core/refusals/store.js";

const T = "2026-10-07T12:00:00.000Z";
const HOME = os.homedir();

// A Claude Code transcript: a compaction, then a prompt, a reply with a tool
// call and its result, a prompt that's refused, and Claude Code's own
// system line for it.
function claudeLines() {
  return [
    { type: "user", uuid: "u0", parentUuid: null, message: { role: "user", content: "before the compaction" }, timestamp: T },
    { type: "system", subtype: "compact_boundary", uuid: "c1", parentUuid: "u0", timestamp: T },
    { type: "user", uuid: "u1", parentUuid: "c1", message: { role: "user", content: "summarize the logs" }, cwd: path.join(HOME, "Projects", "atlas"), timestamp: T },
    { type: "assistant", uuid: "a1", parentUuid: "u1", message: { id: "msg_1", role: "assistant", model: "claude-opus-5-5", content: [{ type: "tool_use", id: "t1", name: "Read", input: { file_path: path.join(HOME, "Projects", "atlas", "log.txt") } }], stop_reason: "tool_use" }, timestamp: T },
    { type: "user", uuid: "u2", parentUuid: "a1", message: { role: "user", content: [{ type: "tool_result", tool_use_id: "t1", content: "x".repeat(30000) }] }, timestamp: T },
    { type: "user", uuid: "u3", parentUuid: "u2", message: { role: "user", content: "now do the thing" }, timestamp: T },
    { type: "assistant", uuid: "a3", parentUuid: "u3", isApiErrorMessage: true, error: "invalid_request", requestId: "req_011CANARY", message: { id: "msg_3", role: "assistant", model: "<synthetic>", content: [{ type: "text", text: "API Error: Claude Code's safeguards flagged this message." }], stop_reason: "refusal", stop_details: { category: "cyber", explanation: "This looks like an attempt to break into a system.", fallback_credit_token: "SECRET-CREDIT" } }, timestamp: "2026-10-07T12:00:05.000Z" },
    { type: "system", subtype: "model_refusal_no_fallback", uuid: "s3", parentUuid: "a3", requestId: "req_011CANARY", apiRefusalCategory: "cyber", apiRefusalExplanation: "This looks like an attempt to break into a system.", timestamp: "2026-10-07T12:00:05.000Z" },
  ];
}

test("Claude Code's refusals, as its lines mark them; other errors and replies aren't", () => {
  const [, , , reply, , , refused, system] = claudeLines();
  const sig = claudeSignal(refused);
  assert.deepEqual([sig.provider, sig.kind, sig.source, sig.category, sig.requestId, sig.messageId, sig.model, sig.code], ["anthropic", "refusal", "api-error", "cyber", "req_011CANARY", "msg_3", null, "invalid_request"]);
  assert.match(sig.text, /safeguards flagged/);
  assert.ok(!JSON.stringify(sig).includes("SECRET-CREDIT"), "a fallback's credit token is never kept");
  assert.equal(keyOf(claudeSignal(system), "claude:s"), keyOf(sig, "claude:s"), "its system line is the same refusal");
  assert.equal(claudeSignal(reply), null);
  assert.equal(claudeSignal({ type: "assistant", isApiErrorMessage: true, error: "rate_limit", message: { model: "<synthetic>", stop_reason: "stop_sequence", content: [] } }), null, "an API error that isn't a refusal");
  assert.equal(claudeSignal({ type: "assistant", message: { model: "claude-opus-5-5", stop_reason: "refusal", content: [{ type: "text", text: "I can't help with that." }] } }).source, "stop-reason", "a model's own reply that stops as a refusal");
  const recovered = claudeSignal({ type: "system", subtype: "model_refusal_fallback", requestId: "req_2", originalModel: "claude-opus-5-5", fallbackModel: "claude-sonnet-5-5", apiRefusalCategory: "bio" });
  assert.deepEqual([recovered.kind, recovered.model, recovered.fallbackModel], ["refusal-recovered", "claude-opus-5-5", "claude-sonnet-5-5"]);
  assert.equal(claudeSignal({ type: "assistant", message: { model: "claude-sonnet-5-5", stop_reason: "end_turn", content: [{ type: "fallback", trigger: { type: "refusal", category: "bio" }, original_model: "claude-opus-5-5" }] } }).kind, "refusal-recovered");
  assert.equal(claudeSignal(null), null);
});

test("Codex's refusals: a policy error event; a reroute and a rejected prompt told apart; other errors not", () => {
  const ev = (payload) => ({ type: "event_msg", timestamp: T, payload });
  const cyber = codexSignal(ev({ type: "error", message: "This request was flagged.", codex_error_info: "cyber_policy" }));
  assert.deepEqual([cyber.provider, cyber.kind, cyber.category, cyber.text], ["openai", "refusal", "cyber_policy", "This request was flagged."]);
  const mis = codexSignal(ev({ type: "error", message: "Stopped.", codex_error_info: { misalignment_policy_violation: { error_type: "x", detailed_explanation: "Why it stopped." } } }));
  assert.deepEqual([mis.category, mis.explanation], ["misalignment_policy_violation", "Why it stopped."]);
  assert.equal(codexSignal(ev({ type: "error", message: "?", codex_error_info: "invalid_prompt" })).kind, "possible-refusal");
  assert.equal(codexSignal(ev({ type: "error", message: "slow down", codex_error_info: "usage_limit_exceeded" })), null);
  const reroute = codexSignal(ev({ type: "model_reroute", from_model: "gpt-5.5-codex", to_model: "gpt-5.5-mini", reason: "high_risk_cyber_activity" }));
  assert.deepEqual([reroute.kind, reroute.model, reroute.fallbackModel, reroute.category], ["reroute", "gpt-5.5-codex", "gpt-5.5-mini", "high_risk_cyber_activity"]);
  assert.equal(codexSignal(ev({ type: "task_complete" })), null);
});

test("a Claude refusal's context: the chain back to the last compaction, the refused reply left out of the messages, long output cut", () => {
  const lines = claudeLines();
  const sig = claudeSignal(lines[6]);
  const got = claudeContext(lines, sig);
  assert.deepEqual(got.lines.map((l) => l.uuid), ["c1", "u1", "a1", "u2", "u3", "a3"], "from the compaction to the refusal");
  assert.deepEqual(got.request.messages.map((m) => m.role), ["user", "assistant", "user", "user"]);
  assert.equal(got.request.model, "claude-opus-5-5", "the model that was asked, not the synthetic one");
  assert.equal(got.request.system, null);
  assert.ok(got.missing.some((m) => /system prompt/.test(m)));
});

test("a Codex refusal's context: its settings, turn context and the items since its last compaction", () => {
  const lines = [
    { type: "session_meta", timestamp: T, payload: { id: "x", base_instructions: { text: "You are Codex." } } },
    { type: "response_item", timestamp: T, payload: { type: "message", role: "user", content: [{ type: "input_text", text: "old" }] } },
    { type: "compacted", timestamp: T, payload: { replacement_history: [{ type: "message", role: "user", content: [{ type: "input_text", text: "summary" }] }] } },
    { type: "turn_context", timestamp: T, payload: { model: "gpt-5.5-codex", effort: "high" } },
    { type: "response_item", timestamp: T, payload: { type: "message", role: "user", content: [{ type: "input_text", text: "do it" }] } },
    { type: "event_msg", timestamp: "2026-10-07T12:00:09.000Z", payload: { type: "error", message: "flagged", codex_error_info: "cyber_policy" } },
  ];
  const got = codexContext(lines, codexSignal(lines[5]));
  assert.deepEqual(got.lines.map((l) => l.type), ["session_meta", "compacted", "turn_context", "response_item", "event_msg"]);
  assert.deepEqual([got.request.model, got.request.effort, got.request.instructions], ["gpt-5.5-codex", "high", "You are Codex."]);
  assert.deepEqual(got.request.input.map((i) => i.content[0].text), ["summary", "do it"]);
});

test("kept once each, with context when it's on, captured later while the transcript is there, and exported scrubbed", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-refusals-"));
  try {
    const transcript = path.join(dir, "t", "0190d3e0-0000-7000-8000-000000000001.jsonl");
    fs.mkdirSync(path.dirname(transcript));
    fs.writeFileSync(transcript, claudeLines().map((l) => JSON.stringify(l)).join("\n") + "\n");
    const store = path.join(dir, "refusals");
    assert.equal(contextOn(store, {}), false, "off unless you turn it on");
    let on = false;
    const r = createRefusals({ dir: store, home: HOME, now: () => 1_800_000_000_000, context: () => on });
    const at = { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", file: transcript, cwd: path.join(HOME, "Projects", "atlas") };
    const rec = r.record(claudeSignal(claudeLines()[6]), at);
    assert.equal(rec.schema, "refusal/record/v1");
    assert.equal(rec.context, null);
    assert.equal(rec.session.cwd, path.join("~", "Projects", "atlas"));
    assert.equal(r.record(claudeSignal(claudeLines()[7]), at), null, "its system line is the same one, kept once");
    assert.equal(createRefusals({ dir: store, home: HOME, context: () => false }).record(claudeSignal(claudeLines()[6]), at), null, "once across starts too");
    // Captured afterwards, from the transcript still there.
    assert.deepEqual(r.capture(rec.id), { ok: true });
    const kept = r.get(rec.id);
    assert.equal(kept.context.lines, 6);
    assert.equal(kept.context.truncated, true, "the long tool output was cut");
    const ctx = zlib.gunzipSync(fs.readFileSync(r.contextFile(rec.id))).toString().trim().split("\n").map((l) => JSON.parse(l));
    assert.equal(ctx.at(-1).uuid, "a3");
    assert.match(ctx[3].message.content[0].content, /characters cut by XenoNet/);
    // A new one, with context on from the start.
    on = true;
    setContext(store, true);
    assert.equal(contextOn(store, {}), true);
    assert.equal(contextOn(store, { XENONET_REFUSAL_CONTEXT: "off" }), false, "the environment wins");
    const second = r.record({ ...claudeSignal(claudeLines()[6]), requestId: "req_other", at: 1_800_000_100_000 }, at);
    assert.ok(second.context, "kept with its context");
    assert.deepEqual(r.list().map((x) => x.id), [second.id, rec.id], "newest first");
    // Exported for a dataset: the request ready to tokenize, scrubbed.
    const lines = r.exportLines({ withContext: true, scrubbed: true });
    assert.equal(lines.length, 2);
    assert.ok(lines[0].request.messages.length >= 3);
    const text = JSON.stringify(lines);
    assert.ok(!text.includes(HOME) && !text.includes(HOME.replace(/\\/g, "\\\\")), "no home folder");
    assert.ok(!text.includes('"path"') && !text.includes('"key"'), "no transcript path or dedupe key");
    assert.equal(r.capture("20990101-deadbeef").ok, false);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
