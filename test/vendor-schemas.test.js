// The vendor schemas (core/schema/vendor), and the lenient check that
// reads vendor data against their files (core/schema/lenient.js): every
// source the contracts list, made-up records that fit and odd ones with the
// drift each should show, and zod and the files agreeing on which fit.
import { test } from "node:test";
import assert from "node:assert/strict";
import { schemaIds, schemaFor } from "../core/schema/index.js";
import { loadVendor, inspect, strip } from "../core/schema/lenient.js";
import { satisfies, validRange } from "../core/vendor.js";
import { claudeLines, codexLines } from "../core/lines.js";
import { drift } from "../core/drift.js";
import { VENDOR } from "./fixtures/vendor.js";

// Contracts, section 1: the vendor sources, and the ones actors added in
// Wave 2 (Hermes Agent's, docs/design/actors/hermes.md).
const SOURCES = [
  "anthropic/claude-code/transcript-line",
  "anthropic/claude-code/config",
  "anthropic/claude-desktop/session",
  "anthropic/claude-desktop/plan-usage",
  "anthropic/agent-sdk/session-listing",
  "anthropic/agent-sdk/session-message",
  "anthropic/oauth/usage",
  "openai/codex/rollout-line",
  "openai/codex/app-server",
  "openai/codex/state-db",
  "openai/codex/auth-claims",
  "github/gh/hosts",
  "nous/hermes-agent/state-db",
  "nous/hermes-agent/spawn-ledger",
  "nous/hermes-agent/gateway-state",
  "nous/hermes-agent/processes",
  "nous/hermes-agent/cron-jobs",
  "nous/hermes-agent/cron-ticker",
  "nous/hermes-agent/install-stamp",
  "anomaly/opencode/db",
  "google/gemini-cli/chat-record",
];

test("every vendor source the contracts list has a schema, read leniently, with the versions it was made for", () => {
  const vendorIds = schemaIds().filter((id) => id.startsWith("vendor/"));
  assert.deepEqual(vendorIds, SOURCES.map((s) => `vendor/${s}/v1`).sort());
  for (const source of SOURCES) {
    const [newest, ...older] = loadVendor(source);
    assert.ok(newest, `schemas/vendor/${source}/v1.json is there`);
    assert.deepEqual(older, []);
    assert.equal(newest.id, `vendor/${source}/v1`);
    assert.ok(validRange(newest.range), `${source}: ${newest.range} is a range`);
    const meta = schemaFor(newest.id).meta()["x-xenonet"];
    assert.deepEqual([meta.lenient, meta.vendorVersions], [true, newest.range]);
    assert.ok(VENDOR[source], `${source} has made-up records`);
  }
  assert.ok(satisfies("2.1.288", loadVendor("anthropic/claude-code/transcript-line")[0].range));
  assert.ok(satisfies("0.155.0-alpha.9", loadVendor("openai/codex/rollout-line")[0].range), "a prerelease is a version too");
  assert.ok(!satisfies("1.0.0", loadVendor("openai/codex/rollout-line")[0].range));
});

const kinds = (findings) => findings.map((f) => [f.kind, f.path]);

for (const source of SOURCES) {
  test(`${source}: made-up records that fit show no drift, and odd ones show theirs`, () => {
    const [{ id, spec }] = loadVendor(source);
    const schema = schemaFor(id);
    for (const record of VENDOR[source].fits) {
      assert.deepEqual(kinds(inspect(spec, record).findings), [], `${JSON.stringify(record).slice(0, 120)} fits`);
      const parsed = schema.safeParse(record);
      assert.ok(parsed.success, `zod agrees: ${parsed.error?.message}`);
    }
    for (const { record, drift, strip: paths = [], drop = false } of VENDOR[source].odd) {
      const what = JSON.stringify(record)?.slice(0, 120);
      const found = inspect(spec, record);
      assert.deepEqual(kinds(found.findings), drift, what);
      assert.deepEqual(found.strip, paths, `${what}: what's taken out`);
      assert.equal(found.drop, drop, `${what}: dropped`);
      // zod takes a record whose only drift is new fields, as the files'
      // loose objects do; a new variant fits none of a union's options.
      assert.equal(schema.safeParse(record).success, drift.every(([k]) => k === "new-field"), `zod on ${what}`);
    }
  });
}

test("each difference says the kind of record it's in: the discriminators' values around it", () => {
  const T = "2026-10-05T12:00:00.000Z";
  const where = (source, record) => inspect(loadVendor(source)[0].spec, record).findings.map((f) => [f.kind, f.path, f.variant]);
  const rollout = (type, payload) => ({ timestamp: T, type, payload });
  assert.deepEqual(where("openai/codex/rollout-line", rollout("session_meta", { id: "t1" })), [["missing-field", "payload.cli_version", ["session_meta"]]]);
  // The same path in two kinds of event is two places.
  assert.deepEqual(where("openai/codex/rollout-line", rollout("event_msg", { type: "user_message" })), [["missing-field", "payload.message", ["event_msg", "user_message"]]]);
  assert.deepEqual(where("openai/codex/rollout-line", rollout("event_msg", { type: "agent_message" })), [["missing-field", "payload.message", ["event_msg", "agent_message"]]]);
  // A new kind is in the kind around it, not one of its own.
  assert.deepEqual(where("openai/codex/rollout-line", rollout("event_msg", { type: "item_completed", item: { type: "Hologram" } })), [["new-variant", "payload.item.type", ["event_msg", "item_completed"]]]);
  assert.deepEqual(where("openai/codex/rollout-line", rollout("world_view", {})), [["new-variant", "type", null]]);
  assert.deepEqual(where("openai/codex/rollout-line", rollout("event_msg", { type: "item_completed", item: { type: "UserMessage", content: "a string" } })), [["type-change", "payload.item.content", ["event_msg", "item_completed", "UserMessage"]]]);
  const reply = { type: "assistant", timestamp: T, message: { id: "m", model: "m", stop_reason: null, usage: { input_tokens: 1, output_tokens: 2 }, content: [{ type: "text", text: 5 }, { type: "tool_use", id: "t", name: "Bash", input: {}, extra: 1 }] } };
  assert.deepEqual(where("anthropic/claude-code/transcript-line", reply), [
    ["type-change", "message.content[].text", ["assistant", "text"]],
    ["new-field", "message.content[].extra", ["assistant", "tool_use"]],
  ]);
  assert.deepEqual(where("anthropic/claude-code/transcript-line", { type: 5 }), [["type-change", "type", null]]);
  assert.deepEqual(where("openai/codex/app-server", { "thread/items/list": { data: [{ item: { type: "agentMessage", text: 5 } }] } }), [["type-change", '["thread/items/list"].data[].item.text', ["agentMessage"]]]);
  // Where there are no kinds of record, there's no variant.
  assert.deepEqual(where("anthropic/oauth/usage", { five_hour: { utilization: "10%" } }), [["type-change", "five_hour.utilization", null]]);
});

test("the kinds of line and the fields newer Claude Code and Codex write are known, and make no drift", () => {
  const T = "2026-10-05T12:00:00.000Z";
  const newerKinds = ["queue-operation", "last-prompt", "attachment", "file-history-snapshot", "file-history-delta", "agent-name", "custom-title", "ai-title", "atis-latch", "cost-state", "frame-link", "mode", "pr-link", "relocated", "permission-mode"];
  const usage = { input_tokens: 12, output_tokens: 150, output_tokens_details: { thinking_tokens: 40 }, iterations: [{ input_tokens: 12, output_tokens: 150 }], inference_geo: "us", fallback_credit: null };
  const claude = [
    ...newerKinds.map((type) => ({ type, timestamp: T, version: "2.1.300", sessionId: "0190d3e0-0000-7000-8000-000000000001" })),
    { type: "assistant", timestamp: T, version: "2.1.300", message: { id: "msg_01", type: "message", role: "assistant", model: "claude-opus-5-5", content: [{ type: "text", text: "Done." }], stop_reason: "end_turn", stop_sequence: null, stop_details: null, diagnostics: null, input_transformations: [], usage } },
  ];
  const codex = [
    { timestamp: T, type: "session_meta", payload: { id: "t1", cli_version: "0.160.0" } },
    { timestamp: T, type: "world_state", payload: {} },
    { timestamp: T, type: "event_msg", payload: { type: "mcp_tool_call_end", call_id: "c1" } },
  ];
  for (const [source, records] of [["anthropic/claude-code/transcript-line", claude], ["openai/codex/rollout-line", codex]]) {
    const [{ spec }] = loadVendor(source);
    for (const record of records) assert.deepEqual(kinds(inspect(spec, record).findings), [], JSON.stringify(record).slice(0, 120));
  }
  // Read as the tailer, stats and search read them, each the first of its
  // kind in its file, so checked whole: no drift record. A kind neither
  // schema lists still is one.
  drift.clear();
  const claudeFile = { file: "/made/up/session.jsonl" };
  const codexFile = { file: "/made/up/rollout.jsonl" };
  for (const raw of claude) claudeLines.record(raw, claudeFile);
  for (const raw of codex) codexLines.record(raw, codexFile);
  assert.deepEqual(drift.list(), []);
  claudeLines.record({ type: "progress", timestamp: T, version: "2.1.300" }, claudeFile);
  codexLines.record({ timestamp: T, type: "event_msg", payload: { type: "hologram_shown" } }, codexFile);
  assert.deepEqual(drift.list().map((r) => [r.source, r.kind, r.path, r.shape]), [
    ["anthropic/claude-code/transcript-line", "new-variant", "type", '"progress"'],
    ["openai/codex/rollout-line", "new-variant", "payload.type", '"hologram_shown"'],
  ]);
  drift.clear();
});

test("a record with a field of another type is read without it, and isn't changed", () => {
  const [{ spec }] = loadVendor("anthropic/claude-code/transcript-line");
  const line = { type: "assistant", timestamp: "2026-10-05T12:00:00Z", message: { id: "m", model: "m", stop_reason: null, usage: { input_tokens: 1, output_tokens: "2" }, content: [{ type: "text", text: 1 }, { type: "text", text: "ok" }, { type: "text", text: 2 }] } };
  const before = JSON.stringify(line);
  const found = inspect(spec, line);
  assert.deepEqual(kinds(found.findings), [["type-change", "message.content[].text"], ["type-change", "message.content[].text"], ["type-change", "message.usage.output_tokens"]]);
  const kept = strip(line, found.strip);
  assert.deepEqual(kept.message.content, [{ type: "text" }, { type: "text", text: "ok" }, { type: "text" }]);
  assert.deepEqual(kept.message.usage, { input_tokens: 1 });
  assert.equal(JSON.stringify(line), before, "the record read is left as it was");
  // An element of another type comes out of its array.
  const odd = { ...line, message: { ...line.message, usage: { input_tokens: 1, output_tokens: 2 }, content: ["a string", { type: "text", text: "ok" }, 7] } };
  const f2 = inspect(spec, odd);
  assert.deepEqual(kinds(f2.findings), [["type-change", "message.content[]"], ["type-change", "message.content[]"]]);
  assert.deepEqual(strip(odd, f2.strip).message.content, [{ type: "text", text: "ok" }]);
});

test("a key that reads as data is never kept, even where the schema closes the object", () => {
  const [{ spec }] = loadVendor("anthropic/claude-code/transcript-line");
  const line = { type: "assistant", timestamp: "2026-10-05T12:00:00Z", message: { id: "m", model: "m", stop_reason: null, content: [], usage: { input_tokens: 1, output_tokens: 2, "/Users/ada/secret.txt": 3, "ada@example.com": { n: 1 } } } };
  const paths = inspect(spec, line).findings.map((f) => f.path);
  assert.deepEqual(paths, ["message.usage.*", "message.usage.*"]);
});
