// The connectors Claude desktop lists in its session records
// (core/desktop-connectors.js): uuid to name, from the newest records.
// Made-up ids and names throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { cleanName, readConnectors, createConnectors } from "../core/desktop-connectors.js";
import { withDeclared } from "../core/mcp-names.js";

const A = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";
const B = "a9d3f0b2-6e41-4c57-8b12-77c0e5a49d36";
const C = "0f8e2d41-93ab-4c60-b7d5-1e6a0c9f3b72";

// A Claude desktop folder with one record per entry, oldest first, each a
// minute newer than the one before.
function desktop(records) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connectors-"));
  const folder = path.join(dir, "claude-code-sessions", "acct", "org");
  fs.mkdirSync(folder, { recursive: true });
  records.forEach((r, i) => {
    const file = path.join(folder, `local_${i}.json`);
    fs.writeFileSync(file, typeof r === "string" ? r : JSON.stringify(r));
    const t = new Date(Date.UTC(2026, 9, 1, 12, i));
    fs.utimesSync(file, t, t);
  });
  return dir;
}
const server = (uuid, name) => ({ uuid, name, url: "https://mcp.example.test/mcp", tools: [] });

test("a name is one line to show, and never a uuid", () => {
  assert.equal(cleanName("  Tunebox  "), "Tunebox");
  assert.equal(cleanName("Tune\nbox\t Pro\u0007"), "Tune box Pro");
  assert.equal(cleanName("x".repeat(80)), "x".repeat(48));
  for (const bad of ["", "   ", "\n", A, null, undefined, 7, {}]) assert.equal(cleanName(bad), null, String(bad));
});

test("the connectors are read from the records' remote servers, by uuid", () => {
  const dir = desktop([{ remoteMcpServersConfig: [server(A, "Tunebox"), server(B, "Brightdesk")] }]);
  assert.deepEqual([...readConnectors(dir)], [[A, "Tunebox"], [B, "Brightdesk"]]);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("where records differ the newest says, and only the newest few are read", () => {
  const records = Array.from({ length: 12 }, (_, i) => ({ remoteMcpServersConfig: [server(A, i === 11 ? "Tunebox 2" : "Tunebox")] }));
  records[0] = { remoteMcpServersConfig: [server(C, "Only in the oldest")] };
  const dir = desktop(records);
  const found = readConnectors(dir);
  assert.equal(found.get(A), "Tunebox 2");
  assert.equal(found.has(C), false, "a record past the newest eight isn't read");
  assert.equal(readConnectors(dir, { limit: 12 }).get(C), "Only in the oldest");
  fs.rmSync(dir, { recursive: true, force: true });
});

test("a record that isn't JSON, or lists nothing usable, is skipped", () => {
  const dir = desktop([
    { remoteMcpServersConfig: [server(A, "Tunebox")] },
    "{ not json",
    { remoteMcpServersConfig: "nope" },
    { remoteMcpServersConfig: [null, 3, { uuid: "not-a-uuid", name: "Bad id" }, { uuid: B }, { uuid: C, name: B }, server(B, "Brightdesk")] },
    { title: "a session with no connectors" },
  ]);
  assert.deepEqual([...readConnectors(dir)], [[B, "Brightdesk"], [A, "Tunebox"]]);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("no folder, or no records, is no connectors", () => {
  assert.equal(readConnectors(null).size, 0);
  assert.equal(readConnectors(path.join(os.tmpdir(), "xenonet-no-such-desktop")).size, 0);
});

test("the connectors are read again only after the ttl, and a failed read leaves none", () => {
  let now = 1000;
  let reads = 0;
  const get = createConnectors({ dir: "/d", now: () => now, ttl: 30000, read: () => (++reads, new Map([[A, `Tunebox ${reads}`]])) });
  assert.equal(get().get(A), "Tunebox 1");
  now += 29999;
  assert.equal(get().get(A), "Tunebox 1");
  now += 1;
  assert.equal(get().get(A), "Tunebox 2");
  assert.equal(reads, 2);
  now += 30000;
  const failing = createConnectors({ dir: "/d", now: () => now, read: () => { throw new Error("gone"); } });
  assert.equal(failing().size, 0);
  assert.equal(createConnectors({ dir: null, read: () => assert.fail("no folder, no read") })().size, 0);
});

test("a declared name replaces what the lines suggested, and leaves the other resources as they were", () => {
  const resources = [{ server: A, ts: 3, name: "Guess", nameSource: "results" }, { server: B, ts: 2 }, { server: "firefox", ts: 1 }];
  const out = withDeclared(resources, new Map([[A, "Tunebox"], [B, "Brightdesk"]]));
  assert.deepEqual(out, [{ server: A, ts: 3, name: "Tunebox", nameSource: "declared" }, { server: B, ts: 2, name: "Brightdesk", nameSource: "declared" }, { server: "firefox", ts: 1 }]);
  assert.deepEqual(Object.keys(out[0]), ["server", "ts", "name", "nameSource"], "in the keys' order");
  assert.deepEqual(resources[0].name, "Guess", "the input isn't changed");
  assert.deepEqual(withDeclared(resources, new Map()), resources);
});
