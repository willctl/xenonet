// XenoNet's schemas as a whole: the ones the contracts list, and the files
// they're written out as (docs/design/contracts.md, section 1).
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { schemaIds, jsonSchemas, schemaFiles } from "../core/schema/index.js";
import { schemaUri } from "../core/schema/ids.js";
import { schemaFilesState, writeSchemaFiles } from "../core/schema/files.js";

const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

// Contracts, section 1: the internal schemas, by path (state has v1 and v2).
const CONTRACT = [
  "session/v1", "project/v1", "graph/v1",
  "live/snapshot/v1", "live/session/v1", "live/activity/v1",
  "identity/v1",
  "engine/message/v1", "engine/claim/v1", "engine/contact/v1", "engine/agent/v1",
  "doctor/result/v1", "doctor/run/v1",
  "drift/log/v1",
  "theme/v1",
  "state/v1", "state/v2",
  "actor/snapshot/v1", "actor/event/v1", "actor/list/v1", "actor/registration/v1",
  "actor/credentials/v1", "actor/gateway-credentials/v1", "actor/gateway-ask/v1", "actor/report/v1", "actor/me/v1", "actor/events/v1", "actor/console/v1", "actor/requests/v1", "actor/tokens/v1",
  "actor/connection/v1", "assistant/result/v1", "assistant/error/v1",
  "drift/record/v1",
  "host/list/v1", "host/hello/v1",
  "network/frame/v1", "network/presence/v1", "network/message/v1", "network/receipt/v1", "network/claims/v1",
  "network/handoff/v1", "network/operator/v1", "network/relay/v1", "network/roster-entry/v1", "network/state/v1",
  "app/v1",
];

test("every internal schema the contracts list is defined", () => {
  const ids = schemaIds();
  assert.deepEqual(CONTRACT.filter((id) => !ids.includes(id)), []);
  assert.deepEqual(ids.filter((id) => !CONTRACT.includes(id) && !id.startsWith("vendor/")), [], "a new internal schema goes in the contracts first");
});

test("each is written out with its $id, a title and a description, and its $refs lead somewhere", () => {
  const all = jsonSchemas();
  const ids = new Set(Object.values(all).map((s) => s.$id));
  for (const [id, json] of Object.entries(all)) {
    assert.equal(json.$schema, "https://json-schema.org/draft/2020-12/schema", id);
    assert.equal(json.$id, schemaUri(id), id);
    assert.ok(json.title && json.description, `${id} has a title and a description`);
    const refs = [];
    const walk = (v) => {
      if (Array.isArray(v)) v.forEach(walk);
      else if (v && typeof v === "object") for (const [k, x] of Object.entries(v)) k === "$ref" ? refs.push(x) : walk(x);
    };
    walk(json);
    for (const ref of refs) assert.ok(ids.has(ref), `${id} refers to ${ref}, which isn't a schema`);
  }
  // The ones that use others refer to them rather than copying them.
  const refsOf = (id) => JSON.stringify(all[id]).match(/"\$ref":"[^"]+"/g)?.map((r) => r.slice(8, -1)) ?? [];
  assert.deepEqual([...new Set(refsOf("graph/v1"))].sort(), [schemaUri("identity/v1"), schemaUri("project/v1"), schemaUri("session/v1")]);
  assert.deepEqual([...new Set(refsOf("live/snapshot/v1"))], [schemaUri("live/session/v1")]);
  assert.deepEqual([...new Set(refsOf("doctor/run/v1"))], [schemaUri("doctor/result/v1")]);
  assert.deepEqual([...new Set(refsOf("drift/log/v1"))], [schemaUri("drift/record/v1")]);
  assert.deepEqual([...new Set(refsOf("actor/list/v1"))], [schemaUri("actor/snapshot/v1")]);
  assert.deepEqual([...new Set(refsOf("actor/snapshot/v1"))], [schemaUri("actor/event/v1")]);
  assert.deepEqual([...new Set(refsOf("actor/events/v1"))], [schemaUri("actor/event/v1")]);
});

test("the committed files in schemas/ are current: npm run schemas writes them", () => {
  const state = schemaFilesState();
  assert.ok(state.current, `the schema files are out of date; run npm run schemas.\n${JSON.stringify(state, null, 2)}`);
});

test("writing the files: what's missing or changed, and none left over", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-schemas-"));
  made.push(dir);
  const all = schemaFiles().map((f) => f.file);
  assert.deepEqual(writeSchemaFiles(dir), { written: [...all].sort(), removed: [] });
  assert.ok(schemaFilesState(dir).current);
  assert.deepEqual(writeSchemaFiles(dir), { written: [], removed: [] }, "nothing to do");

  fs.writeFileSync(path.join(dir, "graph", "v1.json"), "{}\n");
  fs.mkdirSync(path.join(dir, "gone", "deep"), { recursive: true });
  fs.writeFileSync(path.join(dir, "gone", "deep", "v1.json"), "{}\n");
  fs.writeFileSync(path.join(dir, "notes.txt"), "not a schema");
  fs.rmSync(path.join(dir, "theme", "v1.json"));
  assert.deepEqual(schemaFilesState(dir), { current: false, missing: ["theme/v1.json"], changed: ["graph/v1.json"], extra: ["gone/deep/v1.json"] });
  assert.deepEqual(writeSchemaFiles(dir), { written: ["graph/v1.json", "theme/v1.json"], removed: ["gone/deep/v1.json"] });
  assert.ok(!fs.existsSync(path.join(dir, "gone")), "empty folders go too");
  assert.ok(fs.existsSync(path.join(dir, "notes.txt")), "other files stay");
  assert.ok(schemaFilesState(dir).current);
  assert.equal(JSON.parse(fs.readFileSync(path.join(dir, "graph", "v1.json"), "utf8")).$id, schemaUri("graph/v1"));
});

test("the app ships the schema files, which the lenient readers read at runtime", () => {
  const pkg = JSON.parse(fs.readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  assert.ok(pkg.build.files.includes("schemas/**"), "package.json build.files has schemas/**");
});
