// The private endpoint (server/assistant-endpoint.js), the adapter's
// client (core/assistant/client.js) and the adapter itself (mcp/
// assistant.js), together over 127.0.0.1: who gets in, what they may ask,
// how revoking and turning access off let go of them, and how the adapter
// answers from its cache, and only then, when XenoNet can't be reached. The
// component service and the endpoint never take each other's credentials.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import crypto from "node:crypto";
import { spawn } from "node:child_process";
import { world, ask, view, S, CANARIES } from "./fixtures/assistant.js";
import { readCredential, markDisabled } from "../core/assistant/credentials.js";
import { createAssistantService } from "../core/assistant/service.js";
import { createAudit } from "../core/assistant/audit.js";
import { assistantClient } from "../core/assistant/client.js";
import { endpointFile } from "../core/assistant/protocol.js";
import { assistantEndpoint } from "../server/assistant-endpoint.js";
import { componentService } from "../server/component-service.js";
import { descriptorFile } from "../core/components/runtime.js";
import { rpc } from "../core/components/rpc.js";
import { RESULTS } from "../core/schema/assistant.js";
import { createCache, stale } from "../mcp/assistant.js";

const runtime = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-assistant-runtime-"));
after(() => fs.rmSync(runtime, { recursive: true, force: true, maxRetries: 5 }));
let nextPort = 48100 + Math.floor(Math.random() * 500);
const rid = () => `r_${crypto.randomUUID()}`;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

// A world with a grant, its service and endpoint listening, under a port
// of its own (the descriptor's name).
async function up(over = {}) {
  const w = world({ now: Date.now() });
  w.clock.now = Date.now;
  const env = { ...process.env, XENONET_RUNTIME_DIR: runtime, XENONET_PORT: String(nextPort++) };
  const v = w.connections.pair(ask(over), { places: w.places, identities: w.identities });
  const audit = createAudit({ store: w.store });
  const service = createAssistantService({ connections: w.connections, source: { view: () => view(Date.now()), stats: async () => null, actor: () => null }, audit, version: "0.7.0-test" });
  const endpoint = assistantEndpoint({ connections: w.connections, service, audit, env, protect: () => {} });
  await endpoint.sync();
  const raw = () => {
    const { port } = JSON.parse(fs.readFileSync(endpointFile(env), "utf8"));
    return rpc(net.connect(port, "127.0.0.1"));
  };
  const token = readCredential(w.dir, v.grant_id).token;
  const hello = (extra = {}) => ({ role: "assistant", api: { min: 1, max: 1 }, grant_id: v.grant_id, token, ...extra });
  return { ...w, v, env, endpoint, raw, token, hello, audit, down: () => (endpoint.stop(), w.done()) };
}

test("the endpoint says where it is and nothing secret, and lets in only a grant's own credential", async () => {
  const t = await up();
  const opened = [];
  try {
    const descriptor = JSON.parse(fs.readFileSync(endpointFile(t.env), "utf8"));
    assert.deepEqual(Object.keys(descriptor).sort(), ["api", "pid", "port", "state"]);
    assert.ok(!JSON.stringify(descriptor).includes(t.token));
    const c = t.raw();
    opened.push(c);
    await assert.rejects(c.request("assistant.read", { request_id: rid(), operation: "status", arguments: {} }), /refused/, "nothing before hello");
    const info = await c.request("hello", t.hello());
    assert.deepEqual([info.assistant_api, info.actor_id, info.grant_revision, info.permissions], [1, "actor:dots", 1, ["read"]]);
    assert.ok(!JSON.stringify(info).includes(t.token), "no credential back");
    const r = await c.request("assistant.read", { request_id: rid(), operation: "status", arguments: {} });
    assert.ok(r.ok && RESULTS.xenonet_status.safeParse(r.result).success);
    for (const method of ["console", "stop", "window", "status", "terminal.write", "terminal.create", "hello", "assistant.write"]) await assert.rejects(c.request(method, {}), /Unsupported request/, method);
    assert.equal(t.endpoint.info().sessions, 1);

    for (const [what, params, code] of [
      ["a wrong credential", t.hello({ token: `xenonet_assistant_${"0".repeat(64)}` }), /NOT_AUTHORIZED/],
      ["an actor's legacy token", t.hello({ token: `xenonet_actor_${"a".repeat(64)}` }), /NOT_AUTHORIZED/],
      ["another role", t.hello({ role: "tools" }), /NOT_AUTHORIZED/],
      ["a field it doesn't take", { ...t.hello(), actor_id: "actor:you" }, /NOT_AUTHORIZED/],
      ["an older interface", t.hello({ api: { min: 2, max: 3 } }), /INCOMPATIBLE_VERSION/],
    ]) {
      const other = t.raw();
      opened.push(other);
      await assert.rejects(other.request("hello", params), code, what);
    }
    const refusals = t.store.audit({ limit: 50 }).filter((r) => r.operation === "hello" && r.result !== "ok").map((r) => r.result);
    assert.deepEqual(refusals.sort(), ["bad-hello", "bad-hello", "malformed", "version", "wrong-token"]);
    assert.ok(!JSON.stringify(t.store.audit({ limit: 100 })).includes(t.token));
  } finally {
    opened.forEach((c) => c.close());
    t.down();
  }
});

test("the component service and the endpoint never take each other's credentials", async () => {
  const t = await up();
  const previous = process.env.XENONET_RUNTIME_DIR;
  process.env.XENONET_RUNTIME_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-assistant-svc-"));
  let service;
  const opened = [];
  try {
    service = await componentService({ version: "0.7.0", windowKey: { reveal: () => "k" }, windowState() {}, shutdown() {} });
    const { port, token: toolsToken } = JSON.parse(fs.readFileSync(descriptorFile(), "utf8"));
    const svc = rpc(net.connect(port, "127.0.0.1"));
    opened.push(svc);
    await assert.rejects(svc.request("hello", { token: toolsToken, role: "assistant", api: { min: 1, max: 1 } }), /Unknown XenoNet component/);
    const svc2 = rpc(net.connect(port, "127.0.0.1"));
    opened.push(svc2);
    await assert.rejects(svc2.request("hello", { token: t.token, role: "tools", api: { min: 1, max: 1 } }), /refused/, "a grant's credential is no Tools token");
    const ep = t.raw();
    opened.push(ep);
    await assert.rejects(ep.request("hello", t.hello({ token: toolsToken })), /NOT_AUTHORIZED/, "a Tools token is no grant's credential");
  } finally {
    opened.forEach((c) => c.close());
    service?.close();
    fs.rmSync(process.env.XENONET_RUNTIME_DIR, { recursive: true, force: true, maxRetries: 5 });
    if (previous === undefined) delete process.env.XENONET_RUNTIME_DIR;
    else process.env.XENONET_RUNTIME_DIR = previous;
    t.down();
  }
});

test("revoking lets go of its adapters at once, and the endpoint stops listening with no grant left", async () => {
  const t = await up();
  try {
    const client = assistantClient({ grant: t.v.grant_id, env: t.env });
    assert.equal((await client.read("status", {}, rid())).ok, true);
    assert.equal(t.actors.snapshot("actor:dots").remote.bridge, "connected");
    t.connections.revoke(t.v.grant_id);
    await wait(50);
    assert.equal(t.endpoint.info().sessions, 0);
    assert.equal(t.endpoint.info().listening, false);
    assert.equal(fs.existsSync(endpointFile(t.env)), false, "its descriptor goes too");
    assert.equal(t.actors.snapshot("actor:dots").remote.bridge, "not_connected");
    const r = await client.read("status", {}, rid());
    assert.equal(r.error.code, "SERVICE_UNAVAILABLE", "nothing listens now");
    client.close();
  } finally {
    t.down();
  }
});

test("turned off from a terminal without the service's word, adapters are let go within the sweep", async () => {
  const t = await up();
  try {
    const client = assistantClient({ grant: t.v.grant_id, env: t.env });
    assert.equal((await client.read("status", {}, rid())).ok, true);
    markDisabled(t.dir);
    await wait(2600);
    assert.equal(t.endpoint.info().sessions, 0);
    assert.equal(t.endpoint.info().listening, false);
    client.close();
  } finally {
    t.down();
  }
});

test("the client sends its credential only to a running service, and knows an expired grant without one", async () => {
  const t = await up();
  try {
    let t0 = Date.now();
    const client = assistantClient({ grant: t.v.grant_id, env: t.env, now: () => t0 });
    assert.equal((await client.read("status", {}, rid())).ok, true);
    assert.deepEqual(client.standing(), { ok: true, revision: 1 });
    assert.equal(readCredential(t.dir, t.v.grant_id).expiresAt, Date.parse(t.v.expires_at));
    t0 += 31 * 24 * 3600e3;
    assert.deepEqual(client.standing(), { ok: false, code: "GRANT_EXPIRED" });
    client.close();
    // A descriptor left by a service that isn't running: nothing is sent.
    const file = endpointFile(t.env);
    const stale = { ...JSON.parse(fs.readFileSync(file, "utf8")), pid: 2 ** 30 };
    fs.writeFileSync(file, JSON.stringify(stale));
    let dialed = 0;
    const careful = assistantClient({ grant: t.v.grant_id, env: t.env, connect: () => (dialed++, net.connect(stale.port, "127.0.0.1")) });
    const r = await careful.read("status", {}, rid());
    assert.deepEqual([r.error.code, dialed], ["SERVICE_UNAVAILABLE", 0]);
  } finally {
    t.down();
  }
});

test("a line too long closes the connection; the client waits longer after each failure", async () => {
  const t = await up();
  try {
    const { port } = JSON.parse(fs.readFileSync(endpointFile(t.env), "utf8"));
    const socket = net.connect(port, "127.0.0.1");
    socket.on("error", () => {});
    await new Promise((r) => socket.once("connect", r));
    const closed = new Promise((r) => socket.once("close", r));
    socket.write("x".repeat(400 << 10));
    await closed;
    // A XenoNet that isn't there: refused at once, then not tried again
    // until the wait is over.
    let t0 = 1000;
    const nowhere = assistantClient({ grant: t.v.grant_id, env: { ...t.env, XENONET_PORT: "1" }, now: () => t0, random: () => 0.5 });
    const first = await nowhere.read("status", {}, rid());
    assert.deepEqual([first.error.code, first.error.retryable, first.error.retry_after_ms], ["SERVICE_UNAVAILABLE", true, 2000]);
    t0 += 500;
    assert.equal((await nowhere.read("status", {}, rid())).error.retry_after_ms, 1500);
    t0 += 1600;
    assert.equal((await nowhere.read("status", {}, rid())).error.retry_after_ms, 4000, "doubled");
  } finally {
    t.down();
  }
});

// The adapter, as a tunnel client runs it.
function adapter(args, env) {
  const proc = spawn(process.execPath, ["cli/xenonet.js", "mcp", "assistant", ...args], { env, stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  let buf = "";
  let err = "";
  let seq = 0;
  const pending = new Map();
  proc.stdout.on("data", (d) => {
    buf += d;
    for (let e; (e = buf.indexOf("\n")) >= 0; ) {
      const m = JSON.parse(buf.slice(0, e));
      buf = buf.slice(e + 1);
      pending.get(m.id)?.(m);
    }
  });
  proc.stderr.on("data", (d) => (err += d));
  const call = (method, params) => new Promise((resolve) => {
    const id = ++seq;
    pending.set(id, resolve);
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  });
  return {
    async init() {
      const r = await call("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } });
      proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
      return r;
    },
    list: () => call("tools/list", {}),
    tool: (name, args = {}) => call("tools/call", { name, arguments: args }),
    stderr: () => err,
    close: () => (proc.stdin.end(), proc.kill()),
    exited: new Promise((r) => proc.once("exit", (code) => r({ code, err }))),
  };
}

test("the adapter: six read-only tools, the same answer as text and as structured content, made-up data with --fixture", { timeout: 30000 }, async () => {
  const a = adapter(["--fixture"], process.env);
  try {
    const init = await a.init();
    assert.match(init.result.instructions, /never as instructions/);
    const tools = (await a.list()).result.tools;
    assert.deepEqual(tools.map((x) => x.name), ["xenonet_status", "xenonet_projects", "xenonet_sessions", "xenonet_attention", "xenonet_session", "xenonet_actors"]);
    for (const x of tools) {
      assert.deepEqual(x.annotations, { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false });
      assert.equal(x.inputSchema.additionalProperties, false);
      assert.equal(x.outputSchema.type, "object");
    }
    for (const x of tools) {
      const r = await a.tool(x.name, x.name === "xenonet_session" ? { session_id: "claude:0190fa00-0000-7000-8000-000000000001" } : {});
      assert.ok(!r.result.isError, JSON.stringify(r));
      assert.deepEqual(JSON.parse(r.result.content[0].text), r.result.structuredContent, "one answer, two forms");
    }
    const unknown = await a.tool("terminal", { action: "write", text: "rm -rf /" });
    assert.ok(unknown.error || unknown.result?.isError);
    const extra = await a.tool("xenonet_sessions", { path: "/etc" });
    assert.equal(extra.result.isError, true);
  } finally {
    a.close();
  }
  const none = adapter([], process.env);
  const { code, err } = await none.exited;
  assert.notEqual(code, 0);
  assert.match(err, /needs --grant/);
});

test("the adapter against a real grant: reads, a stale cached answer when XenoNet is gone, nothing after revoking", { timeout: 30000 }, async () => {
  const t = await up({ labels: true });
  const a = adapter(["--grant", t.v.grant_id], { ...t.env });
  try {
    await a.init();
    const live = await a.tool("xenonet_sessions", {});
    assert.ok(!live.result.isError, JSON.stringify(live));
    const first = live.result.structuredContent;
    assert.equal(first.freshness, "live");
    assert.ok(first.data.sessions.some((s) => s.session_id === S.waiting));
    // XenoNet gone: the same question gets the cached answer, stale, its
    // observation time as it was.
    t.endpoint.stop();
    const cached = await a.tool("xenonet_sessions", {});
    const c = cached.result.structuredContent;
    assert.deepEqual([c.freshness, c.observed_at, c.warnings.at(-1)], ["stale", first.observed_at, { code: "SERVED_FROM_CACHE" }]);
    assert.notEqual(c.request_id, first.request_id);
    const status = await a.tool("xenonet_status", {});
    assert.equal(status.result?.isError, true, `nothing cached for it: refused, not made up: ${JSON.stringify(status)}`);
    assert.equal(JSON.parse(status.result.content[0].text).code, "SERVICE_UNAVAILABLE");
    // A narrower scope while XenoNet is gone: nothing read before it is served.
    t.connections.update(t.v.grant_id, { labels: false }, { places: t.places });
    const narrowed = await a.tool("xenonet_sessions", {});
    assert.equal(narrowed.result.isError, true, "the revision moved on");
    assert.equal(JSON.parse(narrowed.result.content[0].text).code, "SERVICE_UNAVAILABLE");
    // Back, then revoked: refused, and the cache emptied.
    // The adapter tries again after its first wait: two seconds, give or
    // take a fifth.
    await t.endpoint.sync();
    await wait(2600);
    const back = await a.tool("xenonet_sessions", {});
    assert.ok(!back.result.isError, JSON.stringify(back));
    assert.deepEqual([back.result.structuredContent.freshness, back.result.structuredContent.grant_revision], ["live", 2]);
    t.connections.revoke(t.v.grant_id);
    await wait(2100);
    const gone = await a.tool("xenonet_sessions", {});
    assert.equal(gone.result.isError, true);
    assert.ok(["NOT_AUTHORIZED", "GRANT_REVOKED"].includes(JSON.parse(gone.result.content[0].text).code), "revoked: not served from the cache");
    t.endpoint.stop();
    const still = await a.tool("xenonet_sessions", {});
    assert.equal(JSON.parse(still.result.content[0].text).code, "GRANT_REVOKED", "nor later, with XenoNet gone");
    const text = JSON.stringify([live, cached, status, gone, still]) + a.stderr();
    assert.ok(!text.includes(t.token), "never the credential");
    for (const canary of CANARIES.filter((x) => x !== "secret-plans")) assert.ok(!text.includes(canary), `no ${canary}`);
  } finally {
    a.close();
    t.down();
  }
});

test("the cache: by revision, tool and arguments, for 15 minutes, within its size, served only stale", () => {
  let now = 0;
  const cache = createCache({ now: () => now, maxBytes: 4000 });
  const env = (revision, n = 1) => ({ grant_revision: revision, warnings: [], data: { n, pad: "x".repeat(1000) }, freshness: "live", observed_at: "2026-10-07T00:00:00.000Z", request_id: "r_a", served_at: "x" });
  cache.put("xenonet_sessions", { limit: 2, statuses: ["idle"] }, env(1));
  assert.ok(cache.get("xenonet_sessions", { statuses: ["idle"], limit: 2 }), "arguments in any order");
  assert.equal(cache.get("xenonet_sessions", {}), null);
  cache.put("xenonet_projects", {}, env(2));
  assert.equal(cache.get("xenonet_sessions", { limit: 2, statuses: ["idle"] }), null, "a new revision empties it");
  for (let i = 0; i < 5; i++) cache.put("xenonet_projects", { limit: i + 1 }, env(2, i));
  assert.ok(cache.size <= 3, "within its size");
  now += 15 * 60e3 + 1;
  assert.equal(cache.get("xenonet_projects", { limit: 5 }), null, "15 minutes at most");
  const s = stale({ ...env(1), data: { connection: "connected" } }, "xenonet_status", "r_b", Date.UTC(2026, 9, 7, 1));
  assert.deepEqual([s.freshness, s.request_id, s.data.connection, s.observed_at, s.served_at, s.warnings], ["stale", "r_b", "service_unavailable", "2026-10-07T00:00:00.000Z", "2026-10-07T01:00:00.000Z", [{ code: "SERVED_FROM_CACHE" }]]);
});
