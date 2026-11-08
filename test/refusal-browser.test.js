import { test } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { queryRefusals } from "../src/shared/refusals-query.js";
test("refusal search and facets cover records beyond the former 500-record cap", () => {
  const records = Array.from({ length: 620 }, (_, i) => ({ id: String(i), at: i + 1, provider: i === 0 ? "rare-provider" : "openai", kind: "reroute", session: { model: i === 0 ? "rare-model" : "default" }, signal: { text: i === 0 ? "Needle <script>" : "other" }, source: { file: "~/transcript", path: "private path" }, key: "private key" }));
  const found = queryRefusals(records, { q: "needle SCRIPT", limit: 50 });
  assert.equal(found.matched, 1);
  assert.equal(found.records[0].id, "0");
  assert.deepEqual(found.records[0].source, { file: "~/transcript" });
  assert.equal("key" in found.records[0], false);
  assert.ok(found.facets.model.includes("rare-model"));
  const page = queryRefusals(records, { offset: 600, limit: 50 });
  assert.equal(page.records.length, 20);
  assert.equal(page.records.at(-1).id, "0");
  assert.equal(queryRefusals(records).records.length, 500, "existing API default");
});

