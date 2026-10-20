// What XenoNet does with an app sits behind its capabilities (connect,
// delivery, processes, caller, open), each with one implementation per app
// or config format, chosen through the app registry. A made-up app in a
// registry of its own goes through the same code as Claude Code and Codex.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createConnect, mergeHooks, FORMATS } from "../core/engine/connect.js";

// A made-up app, Wren, whose config format keeps command hooks in
// ~/.wren/hooks.json and its MCP server through `wren mcp`, built from what
// every format shares.
const wren = { id: "wren", name: "Wren", numbat: ["wren"], connect: { format: "wren-hooks", events: [["Start"], ["Edit", "write"]] }, has: ["connect"] };
function wrenHooks(app, k) {
  const file = path.join(k.home, ".wren", "hooks.json");
  const isXenoNet = k.xenonetCommand(app.id);
  const ours = k.mine(isXenoNet);
  const set = k.hookSet(app, () => ({ type: "command", command: k.hookCommand(app.id) }));
  const read = () => (fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, "utf8")) : {});
  const write = (data) => (fs.mkdirSync(path.dirname(file), { recursive: true }), fs.writeFileSync(file, JSON.stringify(data)));
  return {
    cli: "wren",
    server: async (cli, name) => (cli ? (await k.run(cli, ["mcp", "get", name]), null) : null),
    entry: async ({ cli }) => ({ app: app.id, name: app.name, cli, changes: [{ id: "hooks", state: k.hooksState(read(), ours, set, isXenoNet) }], notes: [], broken: [] }),
    apply: async () => write(mergeHooks(read(), ours, set)),
    removeHooks: async () => write(mergeHooks(read(), ours)),
    removeServer: async (cli, name) => {
      if (cli) await k.run(cli, ["mcp", "remove", name]);
    },
  };
}

test("connect: a made-up app with a made-up format, in a registry of its own, is previewed, applied and removed by connect's own loop", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-capabilities-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  fs.writeFileSync(path.join(bin, "wren"), "");
  const asked = [];
  const run = async (file, args) => (asked.push([path.basename(file), ...args].join(" ")), "");
  // An app without connect, and one whose format no code reads, are left
  // out.
  const registry = [{ id: "amp", name: "Amp", numbat: ["amp"] }, wren, { id: "kiro", name: "Kiro", numbat: ["kiro"], connect: { format: "kiro-toml", events: [] }, has: ["connect"] }];
  const connect = createConnect({ platform: "darwin", port: 47840, bin: "/x/xenonet", home, env: { PATH: bin }, run, registry, formats: { ...FORMATS, "wren-hooks": wrenHooks } });
  const file = path.join(home, ".wren", "hooks.json");

  const before = await connect.preview();
  assert.deepEqual(before, { port: 47840, demo: false, apps: [{ app: "wren", name: "Wren", cli: path.join(bin, "wren"), changes: [{ id: "hooks", state: "missing" }], notes: [], broken: [] }] });
  assert.deepEqual(asked, ["wren mcp get xenonet"]);

  const after = await connect.apply();
  assert.equal(after.apps[0].changes[0].state, "present");
  const hook = { type: "command", command: "XENONET_PORT=47840 '/x/xenonet' hook wren" };
  assert.deepEqual(JSON.parse(fs.readFileSync(file, "utf8")), { hooks: { Start: [{ hooks: [hook] }], Edit: [{ matcher: "write", hooks: [hook] }] } });
  assert.deepEqual(await connect.apply(["claude"]), after, "an app it doesn't name stays as it is");

  // Your own hook stays when XenoNet's go.
  const mixed = JSON.parse(fs.readFileSync(file, "utf8"));
  mixed.hooks.Start[0].hooks.push({ type: "command", command: "my-own" });
  fs.writeFileSync(file, JSON.stringify(mixed));
  asked.length = 0;
  const removed = await connect.remove();
  assert.equal(removed.apps[0].changes[0].state, "missing");
  assert.deepEqual(JSON.parse(fs.readFileSync(file, "utf8")), { hooks: { Start: [{ hooks: [{ type: "command", command: "my-own" }] }] } });
  assert.deepEqual(asked, ["wren mcp remove xenonet", "wren mcp get xenonet"], "its hooks, then its server, then the preview");
  assert.equal(await connect.hookConnected("Start"), false, "no format here says");
});

test("connect: the registry connects Claude Code and Codex, through their own formats", async () => {
  const connect = createConnect({ platform: "darwin", port: 47840, bin: "/x/xenonet", home: "/nowhere", env: { PATH: "" }, demo: true });
  await assert.rejects(connect.apply(), { message: "this is the demo, so connect Claude Code and Codex from XenoNet, with your own sessions" });
  assert.deepEqual(Object.keys(FORMATS), ["claude-settings", "codex-hooks"]);
});
