import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import net from "node:net";
import { componentService } from "../server/component-service.js";
import { descriptorFile } from "../core/components/runtime.js";
import { rpc } from "../core/components/rpc.js";

test("a refused Desktop handshake never grants terminal access", async () => {
  const previous = process.env.XENONET_RUNTIME_DIR;
  process.env.XENONET_RUNTIME_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-auth-"));
  let service, first, refused;
  try {
    service = await componentService({ version: "0.6.0", windowKey: { reveal: () => "test-window-key" }, windowState() {}, shutdown() {} });
    const { port, token } = JSON.parse(fs.readFileSync(descriptorFile(), "utf8"));
    first = rpc(net.connect(port, "127.0.0.1"));
    refused = rpc(net.connect(port, "127.0.0.1"));
    const hello = { token, role: "desktop", api: { min: 1, max: 1 } };
    assert.equal((await first.request("hello", hello)).windowKey, "test-window-key");
    await assert.rejects(refused.request("hello", hello), /already has/);
    await assert.rejects(refused.request("terminal.list"), /refused/);
    assert.deepEqual(await first.request("terminal.list"), []);
  } finally {
    first?.close(); refused?.close(); service?.close();
    fs.rmSync(process.env.XENONET_RUNTIME_DIR, { recursive: true, force: true, maxRetries: 5 });
    if (previous === undefined) delete process.env.XENONET_RUNTIME_DIR;
    else process.env.XENONET_RUNTIME_DIR = previous;
  }
});

test("the window key goes only to a Desktop the service can tell is XenoNet's app, which it forgets when it goes", async () => {
  const previous = process.env.XENONET_RUNTIME_DIR;
  process.env.XENONET_RUNTIME_DIR = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-auth-"));
  const told = [];
  let accept = false;
  let service, desktop, tools;
  try {
    service = await componentService({ version: "0.6.0", windowKey: { reveal: () => "test-window-key" }, windowState() {}, shutdown() {}, onDesktop: async (socket) => (told.push(socket ? "connected" : "gone"), { ok: accept }) });
    const { port, token } = JSON.parse(fs.readFileSync(descriptorFile(), "utf8"));
    const hello = (role) => ({ token, role, api: { min: 1, max: 1 } });
    tools = rpc(net.connect(port, "127.0.0.1"));
    assert.equal((await tools.request("hello", hello("tools"))).windowKey, undefined);
    assert.deepEqual(told, [], "the tools aren't the Desktop");
    desktop = rpc(net.connect(port, "127.0.0.1"));
    const refused = await desktop.request("hello", hello("desktop"));
    assert.equal(refused.windowKey, undefined, "no key for a Desktop that isn't XenoNet's app");
    assert.equal(refused.desktop, true, "though it still connects, for its terminals");
    desktop.close();
    await new Promise((r) => setTimeout(r, 50));
    assert.deepEqual(told, ["connected", "gone"]);
    accept = true;
    desktop = rpc(net.connect(port, "127.0.0.1"));
    assert.equal((await desktop.request("hello", hello("desktop"))).windowKey, "test-window-key");
  } finally {
    desktop?.close(); tools?.close(); service?.close();
    fs.rmSync(process.env.XENONET_RUNTIME_DIR, { recursive: true, force: true, maxRetries: 5 });
    if (previous === undefined) delete process.env.XENONET_RUNTIME_DIR;
    else process.env.XENONET_RUNTIME_DIR = previous;
  }
});
