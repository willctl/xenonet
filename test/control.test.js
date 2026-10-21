// The control channel's pieces that run without a desktop window: key
// combos, instances, the server's control route and the MCP server's
// protocol. test/e2e drives a real XenoNet Dev.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseKey, parseSize, modifierBits } from "../electron/keys.js";
import { INSTANCES, instanceFrom } from "../core/instances.js";
import { command, service } from "./harness/xenonet-bin.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("keys: combos become DevTools Protocol key events", () => {
  assert.deepEqual(parseKey("Mod+K", "darwin"), { key: "k", code: "KeyK", keyCode: 75, modifiers: 4, text: undefined });
  assert.equal(parseKey("Mod+K", "win32").modifiers, 2);
  assert.deepEqual(parseKey("W"), { key: "w", code: "KeyW", keyCode: 87, modifiers: 0, text: "w" });
  assert.deepEqual(parseKey("Shift+W"), { key: "W", code: "KeyW", keyCode: 87, modifiers: 8, text: "W" });
  assert.deepEqual(parseKey("Escape"), { key: "Escape", code: "Escape", keyCode: 27, modifiers: 0, text: undefined });
  assert.deepEqual(parseKey("Control+`"), { key: "`", code: "Backquote", keyCode: 192, modifiers: 2, text: undefined });
  assert.equal(parseKey("Enter").text, "\r");
  assert.deepEqual(parseKey("Control++"), { key: "+", code: "Equal", keyCode: 187, modifiers: 2, text: undefined });
  assert.throws(() => parseKey("Hyper+K"), /unknown modifier/);
  assert.throws(() => parseKey("Mod+Banana"), /unknown key/);
  assert.equal(modifierBits(["Alt", "Shift"]), 9);
  assert.deepEqual(parseSize("1600x900@2"), { width: 1600, height: 900, deviceScaleFactor: 2 });
  assert.deepEqual(parseSize("800x600"), { width: 800, height: 600, deviceScaleFactor: 0 });
  assert.throws(() => parseSize("big"), /1600x900/);
});

test("instances: each has its own port and name, picked from the environment", () => {
  assert.equal(new Set(Object.values(INSTANCES).map((i) => i.port)).size, Object.keys(INSTANCES).length);
  assert.equal(new Set(Object.values(INSTANCES).map((i) => i.name)).size, Object.keys(INSTANCES).length);
  assert.equal(instanceFrom({}).id, "dev");
  assert.equal(instanceFrom({ XENONET_DEMO: "1" }).id, "demo");
  assert.deepEqual(instanceFrom({ XENONET_INSTANCE: "test" }), { id: "test", ...INSTANCES.test });
  assert.equal(instanceFrom({ XENONET_INSTANCE: "dev", XENONET_PORT: "5000" }).port, 5000);
  assert.throws(() => instanceFrom({ XENONET_INSTANCE: "prod" }), /dev, demo, test/);
});

// Starts a demo server on a spare port, outside the desktop app.
async function demoServer(port) {
  const proc = spawn(...service(["--demo"]), { cwd: root, env: { ...process.env, XENONET_PORT: String(port), XENONET_CONTROL: "1" }, stdio: ["ignore", "pipe", "pipe"] });
  let timer, stderr = "";
  proc.stderr.on("data", (chunk) => { stderr = (stderr + chunk).slice(-4000); });
  try {
    await new Promise((resolve, reject) => {
      proc.stdout.on("data", (d) => /http:\/\/127\.0\.0\.1/.test(d) && resolve());
      proc.on("exit", (code) => reject(new Error(`server exited (${code}): ${stderr}`)));
      timer = setTimeout(() => reject(new Error(`server didn't start: ${stderr}`)), 60000);
    });
  } catch (err) { proc.kill(); throw err; }
  finally { clearTimeout(timer); }
  return proc;
}

test("launch: a window mode it doesn't know is refused before anything starts, never shown in front", async () => {
  const { xenonet } = await import("../core/control.js");
  await assert.rejects(xenonet("demo", { port: 1 }).launch({ show: "restart:hidden" }), /a window shows inactive, front or hidden, not "restart:hidden"/);
});

test("server: the control route needs XenoNet Dev's window, JSON and the right host", async () => {
  const port = 47900 + Math.floor(Math.random() * 90);
  const proc = await demoServer(port);
  try {
    const post = (headers, body = '{"op":"ping"}') => fetch(`http://127.0.0.1:${port}/api/dev/control`, { method: "POST", headers, body });
    // XENONET_CONTROL alone isn't enough: only a server inside the desktop app
    // has a window to relay to.
    const res = await post({ "content-type": "application/json" });
    assert.equal(res.status, 503);
    assert.match((await res.json()).error, /no window to control/);
    // A page on another site can't reach it.
    assert.equal((await post({ "content-type": "application/json", origin: "https://example.com" })).status, 403);
  } finally {
    proc.kill();
    await new Promise((r) => (proc.exitCode !== null ? r() : proc.once("exit", r)));
    fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
  }
});

// Talks MCP over stdio to mcp/console.js, as a client of either era would.
function mcpClient() {
  const proc = spawn(...command(["mcp", "console"]), { cwd: root, stdio: ["pipe", "pipe", "inherit"] });
  const waiting = new Map();
  let buf = "", id = 0, gone = null;
  // A server that exits, or never starts, fails the calls still waiting
  // rather than leaving the test waiting for an answer that can't come.
  const fail = (err) => {
    gone ??= err;
    for (const { reject } of waiting.values()) reject(gone);
    waiting.clear();
  };
  proc.on("error", fail);
  proc.on("close", (code, signal) => fail(new Error(`xenonet mcp console exited (${signal ?? code}) before it answered`)));
  // Writing to a server that has exited fails with EPIPE; its exit says why.
  proc.stdin.on("error", () => {});
  proc.stdout.on("data", (d) => {
    buf += d;
    for (let i; (i = buf.indexOf("\n")) >= 0; buf = buf.slice(i + 1)) {
      const msg = JSON.parse(buf.slice(0, i));
      waiting.get(msg.id)?.resolve(msg);
      waiting.delete(msg.id);
    }
  });
  return {
    rpc: (method, params = {}) =>
      new Promise((resolve, reject) => {
        if (gone) return reject(gone);
        waiting.set(++id, { resolve, reject });
        proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
      }),
    notify: (method) => proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method }) + "\n"),
    close: () => proc.kill(),
  };
}

const TOOLS = ["terminal", "updates", "background", "status", "launch", "quit", "find", "select", "navigate", "stats", "needs_you", "refusals", "integrations", "spend", "rename", "command", "poster", "labels", "filters", "identities", "hosts", "root", "simulate", "actors", "feed", "screenshot", "input", "window", "logs", "eval", "theme", "mcp_info", "environment"];

test("mcp: answers a 2025-era client's handshake and lists the tools", async () => {
  const c = mcpClient();
  try {
    const init = await c.rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } });
    assert.equal(init.result.serverInfo.name, "xenonet-console");
    assert.ok(init.result.capabilities.tools);
    assert.match(init.result.instructions, /launch/);
    c.notify("notifications/initialized");
    const { result } = await c.rpc("tools/list");
    assert.deepEqual(result.tools.map((t) => t.name), TOOLS);
    for (const t of result.tools) {
      assert.ok(t.description.length > 40, `${t.name} explains itself`);
      assert.equal(t.inputSchema.type, "object");
    }
    // A tool that fails says why, as a tool error rather than a protocol one.
    const call = await c.rpc("tools/call", { name: "select", arguments: { instance: "nope" } });
    assert.ok(call.result?.isError || call.error);
  } finally {
    c.close();
  }
});

test("mcp: answers a 2026-07-28 client, which sends no handshake", async () => {
  const c = mcpClient();
  try {
    const _meta = { "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": { name: "test", version: "1" }, "io.modelcontextprotocol/clientCapabilities": {} };
    const discover = await c.rpc("server/discover", { _meta });
    assert.ok(discover.result.supportedVersions.includes("2026-07-28"));
    const { result } = await c.rpc("tools/list", { _meta });
    assert.deepEqual(result.tools.map((t) => t.name), TOOLS);
  } finally {
    c.close();
  }
});
