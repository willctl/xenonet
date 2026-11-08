// A run in a sandbox, shown in XenoNet (scripts/space/presence.mjs): the
// addresses its client accepts, and the SPACE API request that starts it,
// against a stand-in SPACE API. No Firecracker host is needed.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import http from "node:http";
import { callTool, checkUrl, spaceCommand, startInSpace, tokenFrom } from "../scripts/space/presence.mjs";

const freePort = () => new Promise((resolve) => net.createServer().listen(0, "127.0.0.1", function () {
  const { port } = this.address();
  this.close(() => resolve(port));
}));
const closers = [];
after(async () => {
  for (const c of closers) await c();
});

test("only https, or http on loopback; no user in the address; redirects aren't followed", async () => {
  assert.throws(() => checkUrl("http://gateway.example.test/mcp"), /https/);
  assert.throws(() => checkUrl("https://u:p@gateway.example.test/mcp"), /user/);
  assert.equal(checkUrl("http://127.0.0.1:47850/mcp").port, "47850");
  const redirect = async () => new Response(null, { status: 307, headers: { location: "https://elsewhere.example.test/" } });
  await assert.rejects(callTool("https://gateway.example.test/mcp", "t", "whoami", {}, { fetch: redirect }), /redirect/);
  assert.throws(() => tokenFrom({}), /no gateway token/);
  assert.equal(tokenFrom({ XENONET_GATEWAY_TOKEN: " abc \n" }), "abc");
});

test("the SPACE API is asked to start it in the sandbox, with the gateway token in that command's env alone", async () => {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (c) => (body += c)).on("end", () => {
      seen.push({ method: req.method, url: req.url, auth: req.headers.authorization, key: req.headers["idempotency-key"], body: JSON.parse(body) });
      res.writeHead(201, { "content-type": "application/json" }).end(JSON.stringify({ session_id: "proc_1", process_id: "proc_1", attach: { events_url: "/v1/sessions/proc_1/events", token: "attach" } }));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  closers.push(() => new Promise((r) => server.close(r)));
  const api = `http://127.0.0.1:${server.address().port}`;
  const r = await startInSpace({ api, apiToken: "space-management", idempotencyKey: "k-1", sandbox: "sbx_abc", app: "claude", gatewayUrl: "https://gateway.example.test/mcp", gatewayToken: "gw-secret", status: "Building" });
  assert.equal(r.process, "proc_1");
  assert.equal(seen.length, 1);
  assert.deepEqual([seen[0].method, seen[0].url, seen[0].auth, seen[0].key], ["POST", "/v1/sandboxes/sbx_abc/commands", "Bearer space-management", "k-1"]);
  assert.deepEqual(seen[0].body.command, ["node", "/opt/xenonet/presence.mjs"]);
  assert.deepEqual(seen[0].body.env, { XENONET_GATEWAY_URL: "https://gateway.example.test/mcp", XENONET_GATEWAY_TOKEN: "gw-secret", XENONET_RUN_ID: "sbx_abc", XENONET_RUN_APP: "claude", XENONET_STATUS: "Building" });
  assert.equal(Object.hasOwn(seen[0].body, "metadata"), false);
  assert.throws(() => spaceCommand({ sandbox: "../x", app: "claude", gatewayUrl: "https://g.example.test/mcp", gatewayToken: "t" }), /sbx_/);
  assert.throws(() => spaceCommand({ sandbox: "sbx_a", app: "claude", gatewayUrl: "http://g.example.test/mcp", gatewayToken: "t" }), /https/);
});

