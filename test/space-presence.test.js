// XenoNet presence from inside a sandbox (scripts/space/presence.mjs): the
// client a SPACE sandbox runs to show in XenoNet, against a real actor
// gateway with a gateway token made the way XenoNet's window makes one, and
// the SPACE API request that starts it, against a stand-in SPACE API. No
// Firecracker host is needed.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import http from "node:http";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { Actors, mapOf } from "../core/actors/index.js";
import { createRegistration } from "../core/actors/registration.js";
import { createActorGateway, MCP_PATH } from "../server/actor-gateway.js";
import { callTool, checkUrl, spaceCommand, startInSpace, tokenFrom } from "../scripts/space/presence.mjs";

const freePort = () => new Promise((resolve) => net.createServer().listen(0, "127.0.0.1", function () {
  const { port } = this.address();
  this.close(() => resolve(port));
}));
const closers = [];
after(async () => {
  for (const c of closers) await c();
});

async function gateway() {
  const state = { graph: {}, index: new Map(), live: { sessions: new Map() }, homeOf: () => null };
  const store = new EngineStore(":memory:");
  const actors = new Actors({ store, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, repos: { of: async () => null } });
  const registration = createRegistration({ actors });
  // A program of your own, added in the Actors panel, then Connect…
  registration.add({ name: "SPACE runner", slug: "space-runner", capabilities: ["message-agents"], heartbeatMs: 3600e3 });
  const { token } = registration.gateway("space-runner");
  const port = await freePort();
  const g = createActorGateway({ engine, registration, actors, port, host: "https://gateway.example.test/", version: "test" });
  await g.start();
  closers.push(() => g.stop());
  return { actors, token, url: `http://127.0.0.1:${port}${MCP_PATH}` };
}

test("a sandbox with its actor's gateway token says who it is and that it's at work, and its user sees the status", async () => {
  const g = await gateway();
  const me = await callTool(g.url, g.token, "whoami");
  assert.equal(me.you.id, "actor:space-runner");
  await callTool(g.url, g.token, "heartbeat", { presence: "working", status: "Running the test suite in a sandbox" });
  assert.equal(g.actors.presence("actor:space-runner"), "working");
  assert.equal(g.actors.engineView("actor:space-runner").status, "Running the test suite in a sandbox");
  await callTool(g.url, g.token, "heartbeat", { presence: "offline" });
  assert.equal(g.actors.presence("actor:space-runner"), "offline");
});

test("a wrong token is refused, and the error never carries a token", async () => {
  const g = await gateway();
  const bad = "xn_gw_" + "0".repeat(43);
  await assert.rejects(callTool(g.url, bad, "whoami"), (e) => !e.message.includes(bad) && /401/.test(e.message));
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
  const r = await startInSpace({ api, apiToken: "space-management", idempotencyKey: "k-1", sandbox: "sbx_abc", gatewayUrl: "https://gateway.example.test/mcp", gatewayToken: "gw-secret", status: "Building" });
  assert.equal(r.process, "proc_1");
  assert.equal(seen.length, 1);
  assert.deepEqual([seen[0].method, seen[0].url, seen[0].auth, seen[0].key], ["POST", "/v1/sandboxes/sbx_abc/commands", "Bearer space-management", "k-1"]);
  assert.deepEqual(seen[0].body.command, ["node", "/opt/xenonet/presence.mjs"]);
  assert.deepEqual(seen[0].body.env, { XENONET_GATEWAY_URL: "https://gateway.example.test/mcp", XENONET_GATEWAY_TOKEN: "gw-secret", XENONET_STATUS: "Building" });
  assert.equal(Object.hasOwn(seen[0].body, "metadata"), false);
  assert.throws(() => spaceCommand({ sandbox: "../x", gatewayUrl: "https://g.example.test/mcp", gatewayToken: "t" }), /sbx_/);
  assert.throws(() => spaceCommand({ sandbox: "sbx_a", gatewayUrl: "http://g.example.test/mcp", gatewayToken: "t" }), /https/);
});

test("the script itself, run as a sandbox runs it, says who it is and leaves no token in its output", async () => {
  const { execFile } = await import("node:child_process");
  const g = await gateway();
  const out = await new Promise((resolve, reject) =>
    execFile(process.execPath, ["scripts/space/presence.mjs", "--once"], { env: { ...process.env, XENONET_GATEWAY_TOKEN_FILE: "", XENONET_GATEWAY_URL: g.url, XENONET_GATEWAY_TOKEN: g.token, XENONET_STATUS: "Linting" }, timeout: 20000 }, (err, stdout, stderr) => (err ? reject(err) : resolve(stdout + stderr))),
  );
  assert.match(out, /actor:space-runner/);
  assert.equal(out.includes(g.token), false);
  assert.equal(g.actors.presence("actor:space-runner"), "working");
});
