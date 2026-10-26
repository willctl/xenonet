import { test } from "node:test";
import assert from "node:assert/strict";
import { openRouterView } from "../src/openrouter-view.js";

test("OpenRouter missing, rejected and unavailable usage remain visible without invented spend", () => {
  assert.deepEqual(openRouterView({ key: "missing" }).value, "Set key");
  assert.equal(openRouterView({ key: "missing" }).setup, true);
  assert.match(openRouterView({ key: "missing" }).hint, /not other apps or the whole account/);
  assert.equal(openRouterView(null).value, "Unavailable");
  assert.equal(openRouterView({ key: "rejected" }).value, "Check key");
  assert.equal(openRouterView({ key: "stored", error: "OpenRouter didn't answer" }).setup, false);
  assert.equal(openRouterView({ key: "stored", today: 0, month: 0 }), null);
  // The words don't fit the value column, so each state shows an icon.
  assert.equal(openRouterView({ key: "missing" }).icon, "key");
  assert.equal(openRouterView({ key: "rejected" }).icon, "key-bad");
  assert.equal(openRouterView(null).icon, "none");
});
