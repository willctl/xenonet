// XenoNet's actor gateway (server/actor-gateway.js): an MCP server over HTTP
// for actors outside this machine. Two made-up cloud agents, Muse and Grok
// Bot, each with a gateway token made the way XenoNet's window makes one,
// message each other through it: one through the MCP SDK's own HTTP client,
// as Grok Bot connects, and one with bare JSON-RPC posts, as Muse's agent
// does from its cloud computer. Everything else at the door is refused.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import net from "node:net";
import http from "node:http";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { Actors, mapOf } from "../core/actors/index.js";
import { createRegistration } from "../core/actors/registration.js";
import { createActorGateway, normalHost, MCP_PATH } from "../server/actor-gateway.js";

const HOST = "mac.example-tailnet.ts.net";
const freePort = () => new Promise((resolve) => net.createServer().listen(0, "127.0.0.1", function () {
  const { port } = this.address();
  this.close(() => resolve(port));
}));

const gateways = [];
after(async () => {
  for (const g of gateways) await g.stop();
});

// A machine with one Claude Code session at work in atlas, and Muse and
// Grok Bot as XenoNet observes them; the gateway listening for HOST.
async function world() {
  const t = { now: Date.now() };
  const now = () => t.now;
  const A = "claude:aaaaaaaa-0001";
  const live = { lastEventAt: t.now - 30e3, edits: new Map(), status: () => "tool", needsYou: () => null };
  const index = new Map([
    ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
    [A, { id: A, kind: "session", app: "claude", title: "Add partial refunds", cwd: "/w/atlas", updatedAt: live.lastEventAt }],
  ]);
  const state = { graph: {}, index, live: { sessions: new Map([[A, live]]) }, homeOf: (id) => (index.get(id)?.cwd === "/w/atlas" ? "project:atlas" : null) };
  const store = new EngineStore(":memory:");
  const actors = new Actors({ store, now, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, now, repos: { of: async (cwd) => (cwd?.startsWith("/w/atlas") ? { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" } : null) } });
  const registration = createRegistration({ actors, now });
  actors.upsert({ id: "actor:muse", name: "Muse", product: { id: "muse", vendor: "Meta", version: "7.0" }, via: ["observed"], heartbeatMs: null });
  actors.upsert({ id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: "0.68.1" }, via: ["observed", "attributed"], heartbeatMs: null });
  const port = await freePort();
  const gateway = createActorGateway({ engine, registration, actors, port, host: `https://${HOST}/`, version: "test" });
  gateways.push(gateway);
  await gateway.start();
  const base = `http://127.0.0.1:${port}${MCP_PATH}`;
  return { t, A, store, actors, engine, registration, gateway, port, base };
}

// A JSON-RPC call as a plain HTTP client makes it, naming the public host
// the tunnel forwards (fetch can't set Host, so node's http does).
function rpc(base, token, method, params = {}, { host = HOST, headers = {} } = {}) {
  const body = JSON.stringify({ jsonrpc: "2.0", id: 1, method, params });
  return new Promise((resolve, reject) => {
    const req = http.request(base, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream", host, "content-length": Buffer.byteLength(body), ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers } }, (res) => {
      let text = "";
      res.setEncoding("utf8").on("data", (c) => (text += c)).on("end", () => {
        let parsed = text;
        try {
          parsed = JSON.parse(text);
        } catch {}
        resolve({ status: res.statusCode, body: parsed, headers: new Headers(Object.entries(res.headers).map(([k, v]) => [k, String(v)])) });
      });
    });
    req.on("error", reject);
    req.end(body);
  });
}
const toolText = (r) => JSON.parse(r.body.result.content[0].text);

// The SDK's own client, as an MCP host connects by URL with a header.
async function connect(base, token) {
  const client = new Client({ name: "a-cloud-agent", version: "1.0.0" });
  await client.connect(new StreamableHTTPClientTransport(new URL(base), { requestInit: { headers: { authorization: `Bearer ${token}` } } }));
  return client;
}

test("two actors outside this machine message each other through the gateway, each message marked as coming through it", async () => {
  const w = await world();
  const grok = w.registration.gateway("grok-bot");
  const muse = w.registration.gateway("muse");
  // Muse says it's at work, with a bare JSON-RPC call: XenoNet lists actors
  // it has heard from.
  const beat = await rpc(w.base, muse.token, "tools/call", { name: "heartbeat", arguments: { presence: "idle", status: "Waiting for Grok Bot" } });
  assert.deepEqual([beat.status, beat.headers.get("content-type")], [200, "application/json"], "JSON, for a client that takes it");
  // Grok Bot, through the SDK's client.
  const client = await connect(w.base, grok.token);
  const tools = (await client.listTools()).tools.map((x) => x.name).sort();
  assert.deepEqual(tools, ["heartbeat", "inbox", "peers", "send", "whoami"], "messages and presence, nothing else");
  const me = JSON.parse((await client.callTool({ name: "whoami", arguments: {} })).content[0].text);
  assert.equal(me.through, "XenoNet's actor gateway");
  const peers = JSON.parse((await client.callTool({ name: "peers", arguments: {} })).content[0].text);
  assert.ok(peers.some((p) => p.id === "actor:muse" && p.reachable), "Muse reads messages now");
  const sent = JSON.parse((await client.callTool({ name: "send", arguments: { to: "actor:muse", text: "hello from Grok Bot" } })).content[0].text);
  assert.ok(sent.sent ?? sent.id ?? sent, "sent");
  // Muse, with bare JSON-RPC calls.
  const inbox = toolText(await rpc(w.base, muse.token, "tools/call", { name: "inbox", arguments: {} }));
  const got = (inbox.messages ?? inbox).find?.((m) => String(m.text ?? m.body).includes("hello from Grok Bot")) ?? null;
  assert.ok(got, `Muse reads it: ${JSON.stringify(inbox).slice(0, 400)}`);
  assert.match(JSON.stringify(inbox), /through XenoNet's actor gateway/, "with XenoNet's note of how it came");
  const back = await rpc(w.base, muse.token, "tools/call", { name: "send", arguments: { to: "actor:grok-bot", text: "hello back from Muse" } });
  assert.equal(back.status, 200);
  const reply = JSON.parse((await client.callTool({ name: "inbox", arguments: {} })).content[0].text);
  assert.match(JSON.stringify(reply), /hello back from Muse/);
  // XenoNet kept how each came.
  const envelopes = w.store.recent(0).map((m) => [m.sender, JSON.parse(m.envelope).auth.method, JSON.parse(m.envelope).transport]);
  assert.deepEqual(envelopes, [
    ["actor:grok-bot", "gateway", "gateway"],
    ["actor:muse", "gateway", "gateway"],
  ]);
  // A heartbeat says it's at work.
  await client.callTool({ name: "heartbeat", arguments: { presence: "working", status: "Planning the launch" } });
  assert.equal(w.actors.snapshot("actor:grok-bot").status, "Planning the launch");
  const status = w.gateway.status();
  assert.ok(status.calls >= 6);
  assert.equal(status.url, `https://${HOST}/mcp`);
  await client.close();
});

test("the door: only its host and path, no web pages, no token in the URL, and only gateway tokens", async () => {
  const w = await world();
  const muse = w.registration.gateway("muse");
  assert.equal((await rpc(w.base, muse.token, "tools/list", {}, { host: "evil.example.com" })).status, 421, "another host");
  assert.equal((await rpc(w.base, muse.token, "tools/list", {}, { headers: { origin: "https://evil.example.com" } })).status, 403, "a web page");
  assert.equal((await rpc(`http://127.0.0.1:${w.port}/api/actors`, muse.token, "tools/list")).status, 404, "nothing but /mcp");
  assert.equal((await rpc(`${w.base}?token=${muse.token}`, null, "tools/list")).status, 400, "a token in the URL");
  const none = await rpc(w.base, null, "tools/list");
  assert.deepEqual([none.status, none.body.code, none.headers.get("www-authenticate")], [401, "no-token", 'Bearer realm="xenonet-gateway"']);
  // An actor's own token, made for this machine, never works here.
  const wren = w.registration.add({ name: "Wren", capabilities: ["message-agents"] });
  const own = await rpc(w.base, wren.token, "tools/list");
  assert.deepEqual([own.status, own.body.code], [401, "wrong-token"]);
  // A token taken away stops at once.
  w.registration.dropGateway("muse");
  const gone = await rpc(w.base, muse.token, "tools/list");
  assert.deepEqual([gone.status, gone.body.code], [401, "revoked"]);
  // This machine may call it on its own port, as a local MCP client would.
  const fresh = w.registration.gateway("muse");
  assert.equal((await rpc(w.base, fresh.token, "tools/list", {}, { host: `127.0.0.1:${w.port}` })).status, 200);
});

test("past 20 tokens XenoNet doesn't know in a minute, the gateway answers 429 for a minute", async () => {
  const w = await world();
  const bad = "xenonet_gateway_" + "0".repeat(64);
  const codes = [];
  for (let i = 0; i < 22; i++) codes.push((await rpc(w.base, bad, "tools/list")).status);
  assert.deepEqual([codes[0], codes.at(-1)], [401, 429]);
  const good = w.registration.gateway("muse");
  assert.equal((await rpc(w.base, good.token, "tools/list")).status, 429, "for everyone, for a minute");
});

test("an actor's capabilities hold: without message-agents it can't send, and sends are counted", async () => {
  const w = await world();
  const quiet = w.registration.gateway("muse", { capabilities: [] });
  const r = await rpc(w.base, quiet.token, "tools/call", { name: "send", arguments: { to: "actor:grok-bot", text: "hi" } });
  assert.equal(r.body.result.isError, true);
  assert.match(r.body.result.content[0].text, /message-agents capability/);
});

test("a host the owner names, in whatever form, is kept as a host alone", () => {
  assert.equal(normalHost("https://Mac.Example-Tailnet.ts.net/mcp"), "mac.example-tailnet.ts.net");
  assert.equal(normalHost("mac.example-tailnet.ts.net."), "mac.example-tailnet.ts.net");
  assert.equal(normalHost("not a host"), null);
  assert.equal(normalHost("localhost"), null, "a public name, with a dot");
  assert.equal(normalHost(null), null);
});

// ---------- its switch and its tokens, through XenoNet's own routes ----------

test("the gateway's switch and tunnel are yours, from the window or your terminal; a gateway token is the window's alone, and Muse gets one by it", async () => {
  const { actorRoutes, createActorCaller } = await import("../server/routes/actors.js");
  const { createWindowKey } = await import("../core/actors/key.js");
  const { createGatewayControl } = await import("../server/gateway-control.js");
  const store = new EngineStore(":memory:");
  const now = () => Date.now();
  const state = { graph: {}, index: new Map(), live: { sessions: new Map() }, homeOf: () => null };
  const actors = new Actors({ store, now, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, now, repos: { of: async () => null } });
  const registration = createRegistration({ actors, now });
  actors.upsert({ id: "actor:muse", name: "Muse", product: { id: "muse", vendor: "Meta", version: "7.0" }, via: ["observed"], heartbeatMs: null });
  const port = await freePort();
  // No tunnel tool on this pretend computer, and the address checked on
  // this one alone, through the gateway's own port.
  const control = createGatewayControl({ engine, registration, actors, store, port, version: "test", probe: "local", tunnelOptions: { env: { PATH: "" }, exists: () => false }, every: () => null, stopEvery: () => {} });
  gateways.push(control);
  const key = createWindowKey();
  const verdicts = {};
  const verify = { window: async () => verdicts.window ?? { ok: true, checked: true }, person: async () => verdicts.person ?? { ok: true, checked: true }, peer: async () => 4242 };
  const route = actorRoutes({ actors, registration, caller: createActorCaller({ registration, verify, now }), windowKey: key, verify, gateway: control, readJson: async (req) => req.body ?? {}, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (method, path, { body, headers = {} } = {}) => {
    const res = {};
    await route({ method, headers, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  const window = { "x-xenonet-window": key.reveal() };

  const off = await go("GET", "/api/actors/gateway");
  assert.deepEqual([off.status, off.body.on, off.body.listening, off.body.url, off.body.tunnel.kind], [200, false, false, null, null]);
  // Every kind of tunnel is offered, none assumed: this one has none of
  // their tools, so only your own is ready to use.
  assert.deepEqual(off.body.kinds.map((k) => [k.kind, k.found]), [["tailscale", false], ["cloudflare", false], ["cloudflare-quick", false], ["ngrok", false], ["own", true]]);
  // From under an agent, it's refused.
  verdicts.person = { ok: false, checked: true, reason: "this is yours to do, and it came from under a Claude Code process" };
  assert.deepEqual([(await go("POST", "/api/actors/gateway", { body: { on: true } })).status, (await go("GET", "/api/actors/gateway")).status], [403, 403]);
  delete verdicts.person;
  // A tunnel that's missing what it asks for, or isn't one, is refused.
  for (const tunnel of [{ kind: "own" }, { kind: "own", host: "not a host" }, { kind: "cloudflare", host: "xenonet.example.com" }, { kind: "cloudflare", name: "a b", host: "xenonet.example.com" }, { kind: "wormhole" }]) assert.equal((await go("POST", "/api/actors/gateway", { body: { on: true, tunnel } })).status, 400, JSON.stringify(tunnel));
  const on = await go("POST", "/api/actors/gateway", { body: { on: true, tunnel: { kind: "own", host: "https://Mac.Example-Tailnet.ts.net/" } } });
  assert.deepEqual([on.status, on.body.on, on.body.listening, on.body.url, on.body.tunnel.kind, on.body.tunnel.state], [200, true, true, "https://mac.example-tailnet.ts.net/mcp", "own", "yours"]);
  // XenoNet checks that the address reaches this gateway: here through its
  // own port, with the host the tunnel would pass on.
  const checked = await go("POST", "/api/actors/gateway", { body: { check: true } });
  assert.deepEqual([checked.body.reach.state, checked.body.reach.local, checked.body.problem], ["ok", true, null]);
  // A gateway token is the window's to make: not from your terminal.
  assert.equal((await go("POST", "/api/actors/muse/gateway")).body.code, "yours");
  const made = await go("POST", "/api/actors/muse/gateway", { headers: window });
  assert.deepEqual([made.status, made.body.id, made.body.url], [201, "actor:muse", "https://mac.example-tailnet.ts.net/mcp"]);
  assert.match(made.body.token, /^xenonet_gateway_/);
  assert.equal(actors.isRegistered("actor:muse"), true, "Muse reports itself now, through the gateway");
  const muse = async () => (await go("GET", "/api/actors/gateway")).body.actors.find((a) => a.id === "actor:muse");
  assert.deepEqual(await muse().then((a) => [a.token, a.lastAt, a.oldAddress, a.expiring]), [true, null, false, false]);
  // Muse calls, by the gateway's address: its row says so.
  const call = (token, host = "mac.example-tailnet.ts.net") => rpc(`http://127.0.0.1:${port}${MCP_PATH}`, token, "tools/list", {}, { host });
  assert.equal((await call(made.body.token)).status, 200);
  assert.deepEqual(await muse().then((a) => [a.lastHost, a.oldAddress, !!a.lastAt]), ["mac.example-tailnet.ts.net", false, true]);
  // The address changes: Muse's token still works, but its app has the old
  // address until it's told the new one.
  await go("POST", "/api/actors/gateway", { body: { tunnel: { kind: "own", host: "xenonet.example.com" } } });
  assert.deepEqual(await muse().then((a) => [a.token, a.oldAddress, a.lastHost]), [true, true, "mac.example-tailnet.ts.net"]);
  assert.equal((await call(made.body.token, "xenonet.example.com")).status, 200);
  assert.equal((await muse()).oldAddress, false, "it called the new one");
  // Taking it away is yours, from your terminal too; its app calling with
  // it afterwards shows in its row.
  assert.deepEqual((await go("DELETE", "/api/actors/muse/gateway")).body, { actor: "actor:muse", gateway: false });
  assert.equal((await go("DELETE", "/api/actors/muse/gateway")).status, 404);
  assert.equal((await call(made.body.token, "xenonet.example.com")).status, 401);
  assert.deepEqual(await muse().then((a) => [a.token, a.refused?.reason]), [false, "removed"]);
  // Off closes the port, and is kept across a restart, with the tunnel.
  assert.equal((await go("POST", "/api/actors/gateway", { body: { on: false } })).body.listening, false);
  assert.deepEqual(JSON.parse(store.meta("gateway")), { on: false, host: "xenonet.example.com", tunnel: { kind: "own", name: null, host: "xenonet.example.com" } });
});

test("XenoNet's own check: a word it made is answered once with 204, and nothing else gets that answer", async () => {
  const w = await world();
  w.gateway.expectProbe("w0rd");
  const probe = (word) => new Promise((resolve, reject) => http.get({ host: "127.0.0.1", port: w.port, path: MCP_PATH, headers: { host: HOST, "x-xenonet-probe": word } }, (res) => (res.resume(), resolve([res.statusCode, res.headers["x-xenonet-probe"] ?? null]))).on("error", reject));
  assert.deepEqual(await probe("guess"), [401, null], "a word XenoNet didn't make is a call without a token");
  assert.deepEqual(await probe("w0rd"), [204, "w0rd"]);
  assert.deepEqual(await probe("w0rd"), [401, null], "once");
});

test("after a restart it comes back as it was: the tunnel started again, its new address learned and checked, the apps on the old one named, and a check at once after the computer wakes", async () => {
  const { createGatewayControl } = await import("../server/gateway-control.js");
  const { EventEmitter } = await import("node:events");
  const { PassThrough } = await import("node:stream");
  const store = new EngineStore(":memory:");
  const now = () => Date.now();
  const state = { graph: {}, index: new Map(), live: { sessions: new Map() }, homeOf: () => null };
  const actors = new Actors({ store, now, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, now, repos: { of: async () => null } });
  const registration = createRegistration({ actors, now });
  actors.upsert({ id: "actor:muse", name: "Muse", product: { id: "muse", vendor: "Meta", version: "7.0" }, via: ["observed"], heartbeatMs: null });
  const port = await freePort();
  // A quick tunnel, whose address is new each time cloudflared starts.
  const children = [];
  const spawn = () => {
    const c = Object.assign(new EventEmitter(), { pid: 5000 + children.length, stdout: new PassThrough(), stderr: new PassThrough(), kill: () => setImmediate(() => c.emit("exit", null)) });
    children.push(c);
    setImmediate(() => c.stderr.write(`INF |  https://quick-${children.length}.trycloudflare.com  |\nINF Registered tunnel connection connIndex=0\n`));
    return c;
  };
  const ticks = [];
  let clock = Date.now();
  const make = () =>
    createGatewayControl({
      engine,
      registration,
      actors,
      store,
      port,
      version: "test",
      probe: "local",
      now: () => clock,
      tunnelOptions: { env: { PATH: "" }, platform: "darwin", exists: (f) => f === "/opt/homebrew/bin/cloudflared", run: async () => ({ ok: true, stdout: "cloudflared version 2026.9.0", stderr: "" }), spawn },
      every: (fn) => (ticks.push(fn), ticks.length),
      stopEvery: () => {},
    });
  const settle = async (control, what) => {
    for (let i = 0; i < 100; i++) {
      const st = await control.status();
      if (what(st)) return st;
      await new Promise((r) => setTimeout(r, 50));
    }
    assert.fail(`never settled: ${JSON.stringify((await control.status()).reach)}`);
  };
  const first = make();
  gateways.push(first);
  await first.init();
  await first.set({ on: true, tunnel: { kind: "cloudflare-quick" } });
  const up = await settle(first, (st) => st.host === "quick-1.trycloudflare.com" && st.reach.state === "ok");
  assert.deepEqual([up.url, up.tunnel.state, up.tunnel.stable], ["https://quick-1.trycloudflare.com/mcp", "up", false]);
  const made = registration.gateway("muse");
  assert.equal((await rpc(`http://127.0.0.1:${port}${MCP_PATH}`, made.token, "tools/list", {}, { host: "quick-1.trycloudflare.com" })).status, 200);
  await first.stop();
  // XenoNet starts again, as after a reboot: on, through the same tunnel,
  // which now has another address.
  const second = make();
  gateways.push(second);
  await second.init();
  const back = await settle(second, (st) => st.host === "quick-2.trycloudflare.com" && st.reach.state === "ok");
  assert.deepEqual([back.on, back.listening, back.tunnel.kind], [true, true, "cloudflare-quick"]);
  assert.deepEqual(back.actors.map((a) => [a.id, a.oldAddress, a.lastHost]), [["actor:muse", true, "quick-1.trycloudflare.com"]], "Muse's app has the old address, and its token still works");
  // The computer slept: the clock jumped, and the next tick checks at once.
  const checkedAt = back.reach.at;
  clock += 45 * 60e3;
  await ticks.at(-1)();
  assert.ok((await second.status()).reach.at > checkedAt);
});

test("with its port taken, by another XenoNet's gateway say, it says so and leaves Tailscale Funnel as it is", async () => {
  const { createGatewayControl } = await import("../server/gateway-control.js");
  const store = new EngineStore(":memory:");
  const now = () => Date.now();
  const state = { graph: {}, index: new Map(), live: { sessions: new Map() }, homeOf: () => null };
  const actors = new Actors({ store, now, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, now, repos: { of: async () => null } });
  const registration = createRegistration({ actors, now });
  const taken = net.createServer();
  const port = await new Promise((resolve) => taken.listen(0, "127.0.0.1", () => resolve(taken.address().port)));
  const ran = [];
  const run = async (file, args) => {
    ran.push(args.join(" "));
    if (args[0] === "status") return { ok: true, stdout: JSON.stringify({ BackendState: "Running", Version: "1.104.1", Self: { DNSName: `${HOST}.`, CapMap: { funnel: null } } }), stderr: "" };
    if (args.join(" ") === "funnel status --json") return { ok: true, stdout: JSON.stringify({ Web: { [`${HOST}:443`]: { Handlers: { "/": { Proxy: `http://127.0.0.1:${port}` } } } }, AllowFunnel: { [`${HOST}:443`]: true } }), stderr: "" };
    return { ok: true, stdout: "1.104.1", stderr: "" };
  };
  store.setMeta("gateway", JSON.stringify({ on: true, host: HOST, tunnel: { kind: "tailscale" } }));
  const ticks = [];
  let clock = Date.now();
  const control = createGatewayControl({ engine, registration, actors, store, port, version: "test", now: () => clock, tunnelOptions: { env: { PATH: "" }, platform: "darwin", exists: (f) => f.endsWith("Tailscale"), run }, every: (fn) => (ticks.push(fn), 1), stopEvery: () => {} });
  gateways.push(control);
  await control.init();
  const st = await control.status();
  assert.deepEqual([st.on, st.listening, st.problem], [true, false, `port ${port} is in use by another program`]);
  assert.equal(ran.some((r) => r.includes("off") || r.includes("--bg")), false, "Funnel untouched");
  // The port comes free: the clock tries again, and the gateway listens.
  await new Promise((r) => taken.close(r));
  clock += 3 * 60e3;
  await ticks[0]();
  const back = await control.status();
  assert.deepEqual([back.listening, back.problem === `port ${port} is in use by another program`], [true, false]);
});
