// xenonet gateway (cli/commands/gateway.js), against a stand-in XenoNet that
// records what reaches it: how the gateway stands in words, a tunnel picked
// by its kind (your own when only an address is given and the tunnel in use
// doesn't take one), check and repair, the tunnels this computer has, and
// what to tell an app whose address is old. Made-up addresses.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const T = Date.now();
const health = { ok: true, name: "XenoNet Dev", version: "9.9.9", demo: false, sources: [] };
const KINDS = [
  { kind: "tailscale", label: "Tailscale Funnel", through: "Tailscale Funnel", tool: "tailscale", found: false, path: null, version: null, stable: true, asks: {}, needs: "Tailscale on this computer.", note: "A note.", docs: "https://tailscale.com/kb/1223/funnel" },
  { kind: "ngrok", label: "ngrok", through: "ngrok", tool: "ngrok", found: true, path: "/opt/homebrew/bin/ngrok", version: "3.24.0", stable: true, asks: { host: "optional" }, needs: "ngrok, with your authtoken added.", note: null, docs: null },
  { kind: "own", label: "Your own", through: "a tunnel of your own", tool: null, found: true, path: null, version: null, stable: true, asks: { host: "required" }, needs: "Something you run.", note: null, docs: null },
];
const status = (over = {}) => ({
  on: true,
  listening: true,
  port: 47850,
  host: "xenonet-demo.ngrok-free.app",
  url: "https://xenonet-demo.ngrok-free.app/mcp",
  local: "http://127.0.0.1:47850/mcp",
  calls: 12,
  lastActor: "actor:muse",
  tunnel: { kind: "ngrok", label: "ngrok", through: "ngrok", state: "up", stable: true },
  reach: { state: "ok", at: T - 120e3, local: false },
  problem: null,
  kinds: KINDS,
  actors: [
    { id: "actor:muse", name: "Muse", product: "muse", token: true, lastAt: T - 3600e3, lastHost: "old.example.com", oldAddress: true, expiring: false, refused: null },
    { id: "actor:grok-bot", name: "Grok Bot", product: "grok-bot", token: true, lastAt: T - 60e3, lastHost: "xenonet-demo.ngrok-free.app", oldAddress: false, expiring: false, refused: null },
  ],
  ...over,
});

async function fakeXenoNet(routes) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const r = { method: req.method, path: new URL(req.url, "http://x").pathname, body: body ? JSON.parse(body) : null };
      seen.push(r);
      const route = routes[`${r.method} ${r.path}`];
      const [code, data] = route ? route(r) : [404, { error: "not found" }];
      res.writeHead(code, { "content-type": "application/json" }).end(JSON.stringify(data));
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => server.close() };
}

function xenonet(args) {
  return new Promise((resolve) => {
    const proc = spawn(process.execPath, [path.join(root, "cli", "xenonet.js"), ...args], { cwd: root, env: { ...process.env, NO_COLOR: "1", XENONET_PORT: "" }, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "", stderr = "";
    proc.stdout.on("data", (d) => (stdout += d));
    proc.stderr.on("data", (d) => (stderr += d));
    proc.on("exit", (code) => resolve({ code, stdout, stderr }));
  });
}

test("xenonet gateway: its tunnel, its address and that it's reachable, and each app's connection, with what to do for one on an old address", async () => {
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "GET /api/actors/gateway": () => [200, status()] });
  const r = await xenonet(["gateway", "--port", String(s.port)]);
  s.close();
  assert.equal(r.code, 0, r.stderr);
  assert.match(r.stdout, /the actor gateway is on/);
  assert.match(r.stdout, /tunnel\s+ngrok \(up\)/);
  assert.match(r.stdout, /address\s+https:\/\/xenonet-demo\.ngrok-free\.app\/mcp ✓ reachable, checked 2 min ago/);
  assert.match(r.stdout, /Muse\s+! last called old\.example\.com, an older address: xenonet gateway address muse/);
  assert.match(r.stdout, /Grok Bot  last call 1 min ago/);
  assert.match(r.stdout, /\n    Muse      ! last called/, "the apps' names line up");
});

test("xenonet gateway on: a tunnel by its kind, an address alone as your own tunnel, and check and repair", async () => {
  const s = await fakeXenoNet({
    "GET /api/health": () => [200, health],
    "GET /api/actors/gateway": () => [200, status({ tunnel: { kind: "tailscale", label: "Tailscale Funnel", state: "up" } })],
    "POST /api/actors/gateway": (req) => [200, status({ reach: { state: req.body.check || req.body.repair ? "ok" : "checking", at: T } })],
  });
  const port = ["--port", String(s.port)];
  assert.equal((await xenonet(["gateway", "on", "--tunnel", "ngrok", "--host", "xenonet-demo.ngrok-free.app", ...port])).code, 0);
  assert.equal((await xenonet(["gateway", "on", "--host", "xenonet.example.com", ...port])).code, 0);
  assert.equal((await xenonet(["gateway", "check", ...port])).code, 0);
  assert.equal((await xenonet(["gateway", "repair", ...port])).code, 0);
  assert.equal((await xenonet(["gateway", "off", ...port])).code, 0);
  s.close();
  assert.deepEqual(s.seen.filter((x) => x.method === "POST").map((x) => x.body), [
    { on: true, tunnel: { kind: "ngrok", host: "xenonet-demo.ngrok-free.app" } },
    { on: true, tunnel: { kind: "own", host: "xenonet.example.com" } },
    { check: true },
    { repair: true },
    { on: false },
  ]);
});

test("xenonet gateway tunnels lists every kind and which this computer has; address says what to tell an app", async () => {
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "GET /api/actors/gateway": () => [200, status()] });
  const port = ["--port", String(s.port)];
  const kinds = await xenonet(["gateway", "tunnels", ...port]);
  assert.match(kinds.stdout, /tailscale\s+Tailscale Funnel\s+tailscale isn't on this computer/);
  assert.match(kinds.stdout, /ngrok\s+ngrok\s+found at \/opt\/homebrew\/bin\/ngrok \(3\.24\.0\)\s+← in use/);
  assert.match(kinds.stdout, /own\s+Your own\s+nothing to install/);
  const told = await xenonet(["gateway", "address", "muse", ...port]);
  assert.match(told.stdout, /Muse: the gateway's address is https:\/\/xenonet-demo\.ngrok-free\.app\/mcp/);
  assert.match(told.stdout, /Change your "XenoNet" connector's host to xenonet-demo\.ngrok-free\.app and keep its key/);
  const none = await xenonet(["gateway", "address", "wren", ...port]);
  s.close();
  assert.equal(none.code, 1);
  assert.match(none.stderr, /actor:wren has no gateway token/);
});

test("xenonet gateway token asks for an actor's gateway token and prints none: it waits for Allow in the window", async () => {
  const ask = { schema: "actor/gateway-ask/v1", id: "actor:muse", state: "waiting", request: "gw-abc", expiresAt: T + 600e3, replaces: true };
  const s = await fakeXenoNet({ "GET /api/health": () => [200, health], "POST /api/actors/muse/gateway/ask": () => [202, ask] });
  const port = ["--port", String(s.port)];
  const said = await xenonet(["gateway", "token", "muse", ...port]);
  assert.equal(said.code, 0, said.stderr);
  assert.match(said.stdout, /asked: actor:muse's gateway token waits for you to click Allow in XenoNet's window, and replaces the one its app has/);
  assert.match(said.stdout, /this command never sees it/);
  assert.doesNotMatch(said.stdout + said.stderr, /xenonet_gateway_/);
  assert.deepEqual(s.seen.filter((x) => x.method === "POST").map((x) => [x.path, x.body]), [["/api/actors/muse/gateway/ask", {}]]);
  const asJson = await xenonet(["gateway", "token", "actor:muse", "--json", ...port]);
  assert.deepEqual(JSON.parse(asJson.stdout), ask, "the same, for a program, with the actor's id as well as its slug");
  const none = await xenonet(["gateway", "token", ...port]);
  s.close();
  assert.equal(none.code, 2);
  assert.match(none.stderr, /say whose: xenonet gateway token <slug>/);
});
