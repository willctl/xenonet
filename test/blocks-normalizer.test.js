// The normalizer (core/blocks/normalizer.js): a vendor's record to the one
// XenoNet works with, through the hooks core-3 uses to check records
// leniently and record drift.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Normalizer } from "../core/blocks/normalizer.js";

test("lines parse and normalize; what isn't JSON is counted and passed by", () => {
  const n = new Normalizer({ name: "lines", source: "anthropic/claude-code/transcript-line", normalize: (o, where) => ({ kind: o.type, from: where?.file ?? null }) });
  assert.deepEqual(n.line('{"type":"user"}', { file: "a.jsonl" }), { kind: "user", from: "a.jsonl" });
  assert.equal(n.line("{ half a line"), null);
  assert.deepEqual(n.record({ type: "assistant" }), { kind: "assistant", from: null });
  assert.deepEqual(n.counters, { errors: 0, records: 2, unparsable: 1, dropped: 0 });
  assert.equal(n.source, "anthropic/claude-code/transcript-line");
});

test("the hooks: inspect can trim a record or drop it, and unparsable lines are reported", () => {
  const seen = [];
  const n = new Normalizer({ name: "lines" }).use({
    inspect: (o, where) => (o.type === "mystery" ? (seen.push(["drift", where.file, o.type]), null) : { ...o, extra: undefined }),
    unparsable: (text, where, err) => seen.push(["unparsable", where.file, text, err.name]),
  });
  assert.deepEqual(n.line('{"type":"user","extra":1}', { file: "a" }), { type: "user", extra: undefined });
  assert.equal(n.line('{"type":"mystery"}', { file: "b" }), null);
  assert.equal(n.line("not json", { file: "c" }), null);
  assert.deepEqual(seen, [["drift", "b", "mystery"], ["unparsable", "c", "not json", "SyntaxError"]]);
  assert.deepEqual([n.counters.records, n.counters.dropped, n.counters.unparsable], [2, 1, 1]);
});

test("its health says records went through since the last look", () => {
  const n = new Normalizer({ name: "lines" });
  assert.equal(n.health().state, "idle");
  n.line("{}");
  const h = n.health();
  assert.equal(h.state, "running");
  assert.ok(h.lastOk > 0);
  assert.equal(h.counters.records, 1);
});

test("a record the hooks or normalize throw on is left out and counted, and the next goes through", () => {
  const n = new Normalizer({ name: "lines", normalize: (o) => (o.bad ? o.missing.field : o) }).use({ inspect: (o) => (o.worse ? `${o.worse}` : o) });
  assert.equal(n.line('{"bad":1}'), null);
  assert.equal(n.record({ worse: { toString: 1 } }), null, "an object that won't be a string");
  assert.deepEqual(n.line('{"type":"user"}'), { type: "user" });
  assert.deepEqual([n.counters.records, n.counters.errors, n.counters.dropped], [3, 2, 0]);
  assert.match(n.lastError.message, /Cannot convert object to primitive value/);
  assert.equal(n.health().state, "running", "one record isn't its health");
});
