import { test } from "node:test";
import assert from "node:assert/strict";
import { linkWatcher } from "../src/link-resync.js";

test("the console reads everything again when the stream comes back, not on the first open", () => {
  const seen = [];
  const link = linkWatcher({ state: (s) => seen.push(s), resync: () => seen.push("resync") });
  link.open();
  assert.deepEqual(seen, ["live"], "the first open has nothing to catch up on");
  link.error();
  link.error(); // EventSource errors again while it retries
  assert.deepEqual(seen, ["live", "down", "down"]);
  link.open();
  assert.deepEqual(seen, ["live", "down", "down", "live", "resync"], "events missed while the service was away");
  link.open();
  assert.equal(seen.filter((s) => s === "resync").length, 1, "once per outage");
});
