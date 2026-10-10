import { test } from "node:test";
import assert from "node:assert/strict";
import { ENVIRONMENTS } from "../core/environments.js";
import { environmentRoutes } from "../server/routes/environments.js";

test("environment settings persist independently; invalid or unauthorized choices are refused", async () => {
  let saved = "trench", allowed = true, response;
  const events = [];
  const route = environmentRoutes({ store: { getEnvironment: () => saved, setEnvironment: (id) => { saved = id; } }, yours: async () => allowed, readJson: async (r) => r.body, send: (_, status, body) => { response = { status, body }; }, broadcast: (...args) => events.push(args) });
  await route({ method: "GET" }, {}, new URL("http://localhost/api/environment"));
  assert.equal(response.body.current, "trench");
  assert.deepEqual(response.body.environments.map((e) => e.id), ["nebula", "aurora", "trench", "vault"]);
  await route({ method: "POST", body: { id: "vault" } }, {}, new URL("http://localhost/api/environment"));
  assert.equal(saved, "vault");
  assert.deepEqual(events, [["environment", { id: "vault" }]]);
  await route({ method: "POST", body: { id: "unknown" } }, {}, new URL("http://localhost/api/environment"));
  assert.equal(response.status, 404);
  assert.equal(saved, "vault");
  allowed = false;
  await route({ method: "POST", body: { id: "aurora" } }, {}, new URL("http://localhost/api/environment"));
  assert.equal(saved, "vault");
  assert.equal(ENVIRONMENTS.length, 4);
});
