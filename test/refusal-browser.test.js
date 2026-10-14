import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { queryRefusals } from "../core/refusals/query.js";
import { demoRefusals } from "../core/refusals/demo.js";
import { refusalRoutes } from "../server/routes/refusals.js";

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

test("refusal filters preserve outcome distinctions, date boundaries and stable sort", () => {
  const records = demoRefusals(100000000);
  assert.equal(queryRefusals(records, { provider: "codex" }).matched, 2);
  for (const r of records) assert.equal(queryRefusals(records, { kind: r.kind }).matched, 1);
  assert.equal(queryRefusals(records, { context: "with" }).matched, 1);
  assert.equal(queryRefusals(records, { context: "without" }).matched, 3);
  assert.equal(queryRefusals(records, { model: "claude-opus", category: "policy" }).matched, 1);
  assert.equal(queryRefusals(records, { since: records[1].at, until: records[1].at }).records[0].id, records[1].id);
  assert.equal(queryRefusals(records, { sort: "oldest" }).records[0].id, records[3].id);
  assert.equal(queryRefusals(records, { sort: "model" }).records[0].signal.model, "claude-opus");
  const ties = [{ id: "b", at: 1 }, { id: "a", at: 1 }];
  assert.deepEqual(queryRefusals(ties).records.map((r) => r.id), ["a", "b"]);
  assert.deepEqual(ties.map((r) => r.id), ["b", "a"], "sorting does not mutate the collection");
  assert.equal(queryRefusals([null, { id: "bad", at: NaN }]).total, 0);
  assert.equal(queryRefusals(records, { q: "no such record" }).matched, 0);
});

test("refusal routes require owner access before reading records or context", async () => {
  let reads = 0;
  for (const route of ["/api/refusals", "/api/refusals/20261001-00000001/request", "/api/refusals/context"]) {
    const result = await refusalRoutes({ req: { method: "GET" }, res: {}, url: new URL(route, "http://localhost"), refusals: { list() { reads++; } }, yours: async () => false, send() { throw new Error("owner gate already responded"); } });
    assert.equal(result, true);
  }
  assert.equal(reads, 0);
});

test("saved request reads are explicit, ID-checked, bounded and report missing files", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-refusal-route-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const record = demoRefusals(100000000)[0];
  const folder = path.join(dir, record.id);
  fs.mkdirSync(folder);
  const file = path.join(folder, "request.json");
  fs.writeFileSync(file, JSON.stringify({ input: "private prompt" }));
  let reads = 0;
  const refusals = { dir, get(id) { reads++; return id === record.id ? record : null; }, list: () => [record] };
  const call = async (route) => {
    let result;
    await refusalRoutes({ req: { method: "GET" }, res: {}, url: new URL(route, "http://localhost"), refusals, yours: async () => true, send: (_, status, body) => { result = { status, body }; } });
    return result;
  };
  const list = await call("/api/refusals?limit=50");
  assert.equal(list.body.matched, 1);
  assert.equal(JSON.stringify(list).includes("private prompt"), false);
  assert.equal(reads, 0, "browsing metadata never opens requests");
  assert.equal((await call(`/api/refusals/${record.id}/request`)).body.request.input, "private prompt");
  reads = 0;
  assert.equal((await call("/api/refusals/invalid-id/request")).status, 404);
  assert.equal(reads, 0);
  fs.writeFileSync(file, "x".repeat(1024 * 1024 + 1));
  assert.equal((await call(`/api/refusals/${record.id}/request`)).status, 413);
  fs.writeFileSync(file, "invalid json");
  assert.equal((await call(`/api/refusals/${record.id}/request`)).status, 404);
  fs.unlinkSync(file);
  assert.equal((await call(`/api/refusals/${record.id}/request`)).status, 404);
});
