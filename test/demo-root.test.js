import { test } from "node:test";
import assert from "node:assert/strict";
import { freshDemoRoot } from "../core/demo-root.js";

test("a Windows demo reload uses a fresh sibling when the old workspace is held open", () => {
  const calls = [];
  const root = freshDemoRoot("/temporary/xenonet-demo-123", { platform: "win32", remove: (dir, opts) => { calls.push([dir, opts]); throw Object.assign(new Error("held open"), { code: "EPERM" }); }, allocate: (prefix) => { calls.push(prefix); return `${prefix}fresh`; } });
  assert.equal(root, "/temporary/xenonet-demo-123-fresh");
  assert.equal(calls.length, 2);
  assert.equal(calls[0][1].maxRetries, 4);
  assert.equal(calls[1], "/temporary/xenonet-demo-123-");
});
test("an ordinary demo reset keeps its root; other filesystem errors are reported", () => {
  assert.equal(freshDemoRoot("/temporary/demo", { remove: () => {}, allocate: () => { throw new Error("not needed"); } }), "/temporary/demo");
  assert.throws(() => freshDemoRoot("/temporary/demo", { platform: "win32", remove: () => { throw Object.assign(new Error("disk error"), { code: "EIO" }); } }), /disk error/);
  assert.throws(() => freshDemoRoot("/temporary/demo", { platform: "linux", remove: () => { throw Object.assign(new Error("permissions"), { code: "EPERM" }); } }), /permissions/);
});
