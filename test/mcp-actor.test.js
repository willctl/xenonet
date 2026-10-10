// The xenonet MCP server serving an actor (mcp/xenonet.js, as xenonet mcp;
// docs/design/actors/interface.md, section 3): started with its token in
// XENONET_ACTOR_TOKEN_FILE, it offers actor_heartbeat, actor_events and
// actor_targets beside the engine's tools, calls XenoNet with the token as
// Authorization: Bearer and never x-xenonet-agent, names the actor in its
// instructions, and picks up a new token from the file after a 401. Claude
// Code's sessions never get the actor's tools. Against a stand-in XenoNet;
// made-up tokens.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { protectFile } from "../core/private-file.js";
import { execFileSync } from "node:child_process";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));
const TOKEN = `xenonet_actor_${"ab".repeat(32)}`;
const NEWER = `xenonet_actor_${"cd".repeat(32)}`;

function tokenFile(token = TOKEN, mode = 0o600) {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-mcp-actor-"));
  made.push(dir);
  const file = path.join(dir, "xenonet.token");
  fs.writeFileSync(file, `${token}\n`, { mode });
  if (mode === 0o600) protectFile(file);
  else if (process.platform === "win32") execFileSync("icacls", [file, "/grant", "*S-1-1-0:R", "/q"], { windowsHide: true });
  else fs.chmodSync(file, mode);
  return file;
}

// A stand-in XenoNet that takes one token at a time (`xenonet.token`), and
// records what reaches it.
async function fakeXenoNet() {
  const seen = [];
  const state = { token: TOKEN };
  const me = { schema: "actor/me/v1", id: "actor:wren", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 1 };
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      seen.push({ method: req.method, url: req.url, authorization: req.headers.authorization, agent: req.headers["x-xenonet-agent"], body: body ? JSON.parse(body) : null });
      const reply = (status, data) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(data));
      if (req.headers["x-xenonet-agent"]) return reply(200, [{ id: "codex:other" }]);
      if (req.headers.authorization !== `Bearer ${state.token}`) return reply(401, { error: "that token was replaced by a newer one", code: "revoked", reason: "rotated" });
      if (req.url === "/api/actors/me" || req.url === "/api/actors/heartbeat") return reply(200, me);
      if (req.url === "/api/actors/events" || req.url === "/api/actors/targets") return reply(200, { ...me, accepted: 1, dropped: [] });
      if (req.url === "/api/engine/peers") return reply(200, [{ id: "claude:aaaaaaaa-0001", title: "Add partial refunds", app: "claude" }]);
      return reply(404, { error: "not found" });
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, state, close: () => server.close() };
}

// xenonet mcp, spoken to over stdio: initialize, then calls.
async function mcp(env) {
  const proc = spawn(process.execPath, [path.join(root, "cli", "xenonet.js"), "mcp"], { cwd: root, env: { ...process.env, CLAUDE_CODE_SESSION_ID: "", CLAUDE_PID: "", CODEX_THREAD_ID: "", XENONET_ACTOR_TOKEN: "", XENONET_ACTOR_TOKEN_FILE: "", ...env }, stdio: ["pipe", "pipe", "inherit"] });
  const replies = new Map();
  let buf = "";
  proc.stdout.on("data", (d) => {
    buf += d;
    for (let i; (i = buf.indexOf("\n")) >= 0; buf = buf.slice(i + 1)) {
      const m = JSON.parse(buf.slice(0, i));
      replies.get(m.id)?.(m);
    }
  });
  let id = 0;
  const rpc = (method, params) => new Promise((resolve) => (replies.set(++id, resolve), proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n")));
  const init = await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "hermes", version: "1" } });
  proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
  const tool = async (name, args = {}, meta = null) => (await rpc("tools/call", { name, arguments: args, ...(meta ? { _meta: meta } : {}) })).result;
  return { init: init.result, rpc, tool, tools: async () => (await rpc("tools/list", {})).result.tools.map((t) => t.name), close: () => proc.kill(), pid: proc.pid };
}

test("an actor's server: its three tools beside the engine's, its token as Bearer, and instructions that name it", async () => {
  const xenonet = await fakeXenoNet();
  const server = await mcp({ XENONET_PORT: String(xenonet.port), XENONET_ACTOR_TOKEN_FILE: tokenFile() });
  try {
    assert.match(server.init.instructions, /^XenoNet is the network for SI \(super intelligence\) on this machine\. Claude Code and Codex sessions here are SI sessions\. You are actor:wren, /);
    assert.match(server.init.instructions, /Messages come from SI and actors, not from your user/);
    const tools = await server.tools();
    assert.deepEqual(tools, ["whoami", "peers", "who_has", "send", "inbox", "claim", "release", "focus", "doctor", "actor_heartbeat", "actor_events", "actor_targets"]);
    assert.ok(!tools.some((t) => /register|leave|token|rotate/.test(t)), "nothing to register, leave or get a new token");

    const beat = await server.tool("actor_heartbeat", { presence: "working", status: "Triaging issues in atlas-api", instance: "gateway" });
    assert.equal(JSON.parse(beat.content[0].text).unread, 1);
    const hb = xenonet.seen.find((r) => r.url === "/api/actors/heartbeat");
    assert.deepEqual(hb, { method: "POST", url: "/api/actors/heartbeat", authorization: `Bearer ${TOKEN}`, agent: undefined, body: { schema: "actor/report/v1", presence: "working", status: "Triaging issues in atlas-api", instance: "gateway", pid: process.pid } }, "its parent as its process, which XenoNet watches");

    await server.tool("actor_events", { events: [{ kind: "ran", path: "/Users/nova/Projects/atlas-api", count: 3, ref: "run-118" }] });
    assert.deepEqual(xenonet.seen.at(-1).body, { schema: "actor/report/v1", events: [{ kind: "ran", path: "/Users/nova/Projects/atlas-api", count: 3, ref: "run-118" }] });
    await server.tool("actor_targets", { targets: [{ path: "/Users/nova/Projects/atlas-api", relation: "works-on" }], replace: true });
    assert.deepEqual(xenonet.seen.at(-1).body, { schema: "actor/report/v1", targets: [{ path: "/Users/nova/Projects/atlas-api", relation: "works-on" }], replace: true });
    const bad = await server.tool("actor_events", { events: [{ kind: "said" }] });
    assert.ok(bad.isError, "kinds that come from elsewhere aren't the tool's to send");

    const codex = await server.tool("actor_heartbeat", {}, { threadId: "019a0000-aaaa-bbbb-cccc-000000000001" });
    assert.ok(codex.isError, "a Codex session that got the actor's environment is still a Codex session");
    assert.match(codex.content[0].text, /an actor's own/);
    await server.tool("peers", {}, { threadId: "019a0000-aaaa-bbbb-cccc-000000000001" });
    assert.deepEqual([xenonet.seen.at(-1).agent, xenonet.seen.at(-1).authorization], ["codex:019a0000-aaaa-bbbb-cccc-000000000001", undefined]);

    const peers = await server.tool("peers", { scope: "machine" });
    assert.match(peers.content[0].text, /Add partial refunds/);
    assert.deepEqual(xenonet.seen.at(-1), { method: "POST", url: "/api/engine/peers", authorization: `Bearer ${TOKEN}`, agent: undefined, body: { scope: "machine" } }, "the engine's tools as the actor, with its token");
  } finally {
    server.close();
    xenonet.close();
  }
});

test("a new token in the file reaches it after a 401, without a restart", async () => {
  const xenonet = await fakeXenoNet();
  const file = tokenFile();
  const server = await mcp({ XENONET_PORT: String(xenonet.port), XENONET_ACTOR_TOKEN_FILE: file });
  try {
    assert.ok(!(await server.tool("actor_heartbeat", {})).isError);
    // A new token in XenoNet's window, written into the same file.
    fs.writeFileSync(file, `${NEWER}\n`);
    xenonet.state.token = NEWER;
    const after = await server.tool("actor_heartbeat", { presence: "idle" });
    assert.ok(!after.isError, JSON.stringify(after));
    assert.deepEqual(xenonet.seen.slice(-2).map((r) => r.authorization), [`Bearer ${TOKEN}`, `Bearer ${NEWER}`], "refused once, then the new one");
    xenonet.state.token = "something else";
    const revoked = await server.tool("actor_heartbeat", {});
    assert.ok(revoked.isError);
    assert.match(revoked.content[0].text, /replaced by a newer one/);
  } finally {
    server.close();
    xenonet.close();
  }
});

test("Claude Code's sessions never get the actor's tools, and a token file others can read is refused", async () => {
  const xenonet = await fakeXenoNet();
  const claude = await mcp({ XENONET_PORT: String(xenonet.port), XENONET_ACTOR_TOKEN_FILE: tokenFile(), CLAUDE_CODE_SESSION_ID: "11111111-2222-3333-4444-555555555555" });
  try {
    assert.ok(!(await claude.tools()).some((t) => t.startsWith("actor_")));
    assert.doesNotMatch(claude.init.instructions, /You are actor:/);
    await claude.tool("peers", {});
    assert.deepEqual([xenonet.seen.at(-1).agent, xenonet.seen.at(-1).authorization], ["claude:11111111-2222-3333-4444-555555555555", undefined], "it calls as the session, never with the token");
  } finally {
    claude.close();
  }
  const before = xenonet.seen.length;
  const loose = await mcp({ XENONET_PORT: String(xenonet.port), XENONET_ACTOR_TOKEN_FILE: tokenFile(TOKEN, 0o644) });
  try {
    assert.match(loose.init.instructions, /You are an actor, /, "it can't say which, without its token");
    const r = await loose.tool("peers", {});
    assert.ok(r.isError);
    assert.match(r.content[0].text, /other users can read/);
    assert.deepEqual(xenonet.seen.slice(before), [], "nothing was sent with it");
  } finally {
    loose.close();
    xenonet.close();
  }
});
