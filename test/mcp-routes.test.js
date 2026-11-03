// GET /api/mcp (server/routes/mcp.js): a shared system is looked up by its
// server id, which is not its name when the id is a uuid. Made-up ids.
import { test } from "node:test";
import assert from "node:assert/strict";
import { mcpRoutes } from "../server/routes/mcp.js";

const UUID = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";

async function ask(server) {
  const state = {
    index: new Map([
      [`resource:${UUID}`, { kind: "resource", name: "Tunebox", server: UUID }],
      ["resource:firefox", { kind: "resource", name: "firefox" }],
    ]),
    liveSnapshot: () => ({ sessions: [] }),
  };
  let reply;
  const route = mcpRoutes({ state, demo: false, send: (res, status, body) => (reply = { status, body }), catalog: { get: async (id) => ({ server: id, title: id }) } });
  const handled = await route({ method: "GET" }, {}, new URL(`http://x/api/mcp?server=${encodeURIComponent(server)}`));
  assert.equal(handled, true);
  return reply;
}

test("a shared system is found by its server id, whatever its name on the map", async () => {
  assert.deepEqual((await ask("firefox")).body, { server: "firefox", title: "firefox" });
  assert.equal((await ask(UUID)).status, 200, "a uuid server is found by its id");
  assert.equal((await ask(UUID)).body.server, UUID);
  assert.equal((await ask("Tunebox")).status, 404, "its name on the map is not an id");
  assert.equal((await ask("nowhere")).status, 404);
});
