import { test } from "node:test";
import assert from "node:assert/strict";
import { verifyComponentVersions, publishedComponents } from "../scripts/component-release.mjs";

test("Desktop releases cannot republish different component bytes under the same version", () => {
  const previous = { component: "tools", platform: "win32", arch: "x64", version: "0.6.0", sha256: "a" };
  verifyComponentVersions([previous], [previous]);
  assert.throws(() => verifyComponentVersions([{ ...previous, sha256: "b" }], [previous]), /Bump its version/);
  assert.throws(() => verifyComponentVersions([{ ...previous, version: "0.5.9" }], [previous]), /older version/);
  verifyComponentVersions([{ ...previous, version: "0.6.1", sha256: "b" }], [previous]);
  verifyComponentVersions([{ ...previous, arch: "arm64", sha256: "b" }], [previous]);
});

test("first component release accepts a missing feed but fails on an unavailable one", async () => {
  assert.deepEqual(await publishedComponents(async () => ({ status: 404 })), []);
  await assert.rejects(publishedComponents(async () => ({ status: 503 })), /Could not verify/);
});
