// What the JS actor gateway (server/actor-gateway.js) answers, for the Rust
// gateway's test to compare against: each request below, sent to a JS
// gateway whose cores are stand-ins, and its answer, kept in gateway.json.
// Run it again when the JS gateway or its MCP SDK changes:
//
//   node crates/xenonet-service/tests/fixtures/gateway.mjs
import fs from "node:fs";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, "..", "..", "..", "..");
const { createActorGateway } = await import(path.join(root, "server", "actor-gateway.js"));
const port = await new Promise((r) => net.createServer().listen(0, "127.0.0.1", function () { const { port } = this.address(); this.close(() => r(port)); }));

// The stand-ins, which the Rust test has too.
const T = `xenonet_gateway_${"a".repeat(64)}`;
const auth = { ok: true, id: "actor:muse", slug: "muse", state: "allowed" };
const registration = {
  authenticateGateway: (t) => (t === T ? auth : { ok: false, status: 401, code: "bad-token", error: "XenoNet doesn't know that token" }),
  me: () => ({ schema: "actor/me/v1", id: "actor:muse", unread: 2 }),
};
const actors = { info: () => ({ name: "Muse", product: { id: "muse" } }), can: () => true };
const engine = {
  whoami: async () => ({ you: "actor:muse" }),
  peers: async (id, o) => [{ id: "claude:x", scope: o.scope }],
  send: async (id, m, e) => ({ id: 7, to: m.to, method: e.envelope.auth.method }),
  inbox: async (id, o) => ({ messages: [], waitMs: o.waitMs }),
};
const runs = { report: (a, r) => ({ ok: true, id: `run:${r.run}` }) };
const g = createActorGateway({ engine, registration, actors, port, host: "https://mac.example.ts.net/", version: "test", runs });
await g.start();

function send({ method = "POST", path: p = "/mcp", headers = {}, body }) {
  return new Promise((resolve) => {
    const req = http.request({ host: "127.0.0.1", port, method, path: p, headers: { host: "mac.example.ts.net", ...headers } }, (res) => {
      let text = "";
      res.setEncoding("utf8").on("data", (c) => (text += c)).on("end", () => {
        const keep = ["content-type", "cache-control", "www-authenticate", "allow", "retry-after", "x-xenonet-probe"];
        resolve({ status: res.statusCode, headers: Object.fromEntries(keep.filter((k) => res.headers[k] !== undefined).map((k) => [k, res.headers[k]])), body: text });
      });
    });
    req.end(body);
  });
}

const J = { "content-type": "application/json", accept: "application/json, text/event-stream", authorization: `Bearer ${T}` };
const M = { "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}, "io.modelcontextprotocol/clientInfo": { name: "c", version: "1" } };
const N = (m, n) => ({ ...J, "mcp-protocol-version": "2026-07-28", "mcp-method": m, ...(n ? { "mcp-name": n } : {}) });
const rpc = (msg, headers = J) => ({ headers, body: JSON.stringify(msg) });
const call = (name, args, id = 3) => rpc({ jsonrpc: "2.0", id, method: "tools/call", params: { name, arguments: args } });
const init = (v) => rpc({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: v, capabilities: {}, clientInfo: { name: "c", version: "1" } } });

const requests = {
  "initialize 2025-06-18": init("2025-06-18"),
  "initialize 2025-11-25": init("2025-11-25"),
  "initialize 2024-11-05": init("2024-11-05"),
  "initialize 2026-07-28": init("2026-07-28"),
  "initialize unknown": init("1999-01-01"),
  "initialize without params": rpc({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} }),
  "initialized": rpc({ jsonrpc: "2.0", method: "notifications/initialized" }),
  "a response": rpc({ jsonrpc: "2.0", id: 2, result: {} }),
  "tools/list": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} }),
  "tools/list with a 2025 header": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list" }, { ...J, "mcp-protocol-version": "2025-06-18" }),
  "a version no one knows": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list" }, { ...J, "mcp-protocol-version": "1999-01-01" }),
  "ping": rpc({ jsonrpc: "2.0", id: 11, method: "ping" }),
  "ping by a string id": rpc({ jsonrpc: "2.0", id: "s", method: "ping" }),
  "a method it hasn't": rpc({ jsonrpc: "2.0", id: 10, method: "resources/list", params: {} }),
  "not JSON-RPC": rpc({ id: 2, method: "tools/list" }),
  "a batch": rpc([{ jsonrpc: "2.0", id: 1, method: "ping" }, { jsonrpc: "2.0", id: 2, method: "ping" }]),
  "whoami": call("whoami", {}),
  "whoami with more": call("whoami", { x: 1 }),
  "peers": call("peers", { scope: "repo" }),
  "peers without arguments": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "peers" } }),
  "peers out of scope": call("peers", { scope: "x" }),
  "send": call("send", { to: "all", text: "hi" }),
  "send empty": call("send", { text: "" }),
  "send nothing": call("send", {}),
  "send numbers": call("send", { text: 5, to: 3 }),
  "send too long, reply_to a fraction": call("send", { text: "x".repeat(4001), reply_to: 1.5 }),
  "send reply_to too big": call("send", { text: "hi", reply_to: 99999999999999999999 }),
  "inbox too long": call("inbox", { wait_seconds: 99 }),
  "inbox below zero": call("inbox", { wait_seconds: -1 }),
  "inbox a string": call("inbox", { wait_seconds: "5" }),
  "inbox a fraction": call("inbox", { wait_seconds: 2.5 }),
  "inbox": call("inbox", { wait_seconds: 3 }),
  "report_run": call("report_run", { run: "sb1", app: "claude" }),
  "report_run too long": call("report_run", { run: "x".repeat(65) }),
  "heartbeat away": call("heartbeat", { presence: "away" }),
  "heartbeat null status": call("heartbeat", { status: null }),
  "a tool it hasn't": call("nope", {}),
  "arguments not an object": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "whoami", arguments: 5 } }),
  "no name": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { arguments: {} } }),
  "2026 discover": rpc({ jsonrpc: "2.0", id: 9, method: "server/discover", params: { _meta: M } }, N("server/discover")),
  "2026 tools/list": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: { _meta: M } }, N("tools/list")),
  "2026 tools/call": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "whoami", arguments: {}, _meta: M } }, N("tools/call", "whoami")),
  "2026 tools/call bad input": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "inbox", arguments: { wait_seconds: 99 }, _meta: M } }, N("tools/call", "inbox")),
  "2026 a tool it hasn't": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "nope", arguments: {}, _meta: M } }, N("tools/call", "nope")),
  "2026 ping": rpc({ jsonrpc: "2.0", id: 4, method: "ping", params: { _meta: M } }, N("ping")),
  "2026 initialize": rpc({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2026-07-28", capabilities: {}, clientInfo: { name: "c", version: "1" }, _meta: M } }, N("initialize")),
  "2026 notification": rpc({ jsonrpc: "2.0", method: "notifications/initialized", params: { _meta: M } }, N("notifications/initialized")),
  "2026 without _meta": rpc({ jsonrpc: "2.0", id: 9, method: "server/discover", params: {} }, N("server/discover")),
  "2026 without client capabilities": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: { _meta: { "io.modelcontextprotocol/protocolVersion": "2026-07-28" } } }, N("tools/list")),
  "2026 without Mcp-Method": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: { _meta: M } }, { ...J, "mcp-protocol-version": "2026-07-28" }),
  "2026 another Mcp-Method": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: { _meta: M } }, N("tools/call")),
  "2026 without Mcp-Name": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "whoami", arguments: {}, _meta: M } }, N("tools/call")),
  "2026 another version in the body": rpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: { _meta: { ...M, "io.modelcontextprotocol/protocolVersion": "2025-11-25" } } }, N("tools/list")),
  "2026 in the body alone": rpc({ jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: "whoami", arguments: {}, _meta: M } }),
  "not JSON": { headers: J, body: "{nope" },
  "no Accept": { headers: { "content-type": "application/json", authorization: `Bearer ${T}` }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" }) },
  "not JSON's type": { headers: { "content-type": "text/plain", accept: "application/json, text/event-stream", authorization: `Bearer ${T}` }, body: "{}" },
  "neither": { headers: { "content-type": "text/plain", authorization: `Bearer ${T}` }, body: "{}" },
  "GET": { method: "GET", headers: { authorization: `Bearer ${T}`, accept: "text/event-stream" } },
  "DELETE": { method: "DELETE", headers: { authorization: `Bearer ${T}` } },
  "no token": { headers: { "content-type": "application/json" }, body: "{}" },
  "a token it doesn't know": { headers: { "content-type": "application/json", authorization: `Bearer xenonet_gateway_${"b".repeat(64)}` }, body: "{}" },
  "another host": { headers: { host: "evil.example" } },
  "this machine, by its address": { headers: { ...J, host: `127.0.0.1:${port}` }, body: JSON.stringify({ jsonrpc: "2.0", id: 11, method: "ping" }) },
  "from a page": { headers: { origin: "https://x" } },
  "another path": { path: "/other" },
  "PUT": { method: "PUT" },
  "a token in the URL": { path: "/mcp?token=x" },
  "a probe it didn't ask for": { method: "GET", headers: { "x-xenonet-probe": "nope" } },
};
const cases = [];
for (const [name, r] of Object.entries(requests)) cases.push({ name, request: { method: r.method ?? "POST", path: r.path ?? "/mcp", headers: { host: "mac.example.ts.net", ...(r.headers ?? {}) }, body: r.body ?? null }, answer: await send(r) });
await g.stop();
// The gateway's own port differs each run; the Rust test names its own.
const text = JSON.stringify({ port, cases }, null, 1).replaceAll(`127.0.0.1:${port}`, "127.0.0.1:PORT");
fs.writeFileSync(path.join(here, "gateway.json"), text + "\n");
console.log(`${cases.length} answers in ${path.relative(root, path.join(here, "gateway.json"))}`);
