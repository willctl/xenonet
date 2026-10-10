import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createConnect } from "../core/engine/connect.js";

test("installed connections register both MCP interfaces through the stable launcher", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex"]) fs.writeFileSync(path.join(bin, name), "");
  const calls = [], entries = { claude: {}, codex: {} };
  const runtime = "/xenonet/runtime/launcher/node", root = "/xenonet/runtime/launcher";
  const connect = createConnect({ platform: "darwin", port: 47840, root, exec: runtime, home, env: { PATH: bin, XENONET_LAUNCHER_EXEC: runtime }, run: async (file, args) => {
    const app = path.basename(file);
    calls.push({ app, args });
    const [, op, name] = args;
    if (op === "get") {
      if (!entries[app][name]) throw new Error("not found");
      return JSON.stringify({ transport: entries[app][name] });
    }
    if (op === "remove") delete entries[app][name];
    if (op === "add-json") entries[app][name] = JSON.parse(args[3]);
    if (op === "add") {
      const split = args.indexOf("--");
      entries[app][name] = { command: args[split + 1], args: args.slice(split + 2), env: { XENONET_PORT: "47840" } };
    }
    if (app === "claude") fs.writeFileSync(path.join(home, ".claude.json"), JSON.stringify({ mcpServers: entries.claude }));
    return "";
  } });
  const before = await connect.preview();
  assert.deepEqual(before.apps.map((app) => app.changes.map((change) => change.id)), [["console", "mcp", "hooks"], ["console", "mcp", "hooks"]]);
  const after = await connect.apply();
  assert.ok(after.apps.every((app) => app.changes.every((change) => change.state === "present")));
  for (const app of ["claude", "codex"]) for (const name of ["xenonet", "xenonet-console"]) {
    const server = entries[app][name];
    assert.equal(server.command, runtime);
    assert.deepEqual(server.args, [`${root}/cli/xenonet.js`, "mcp", ...(name === "xenonet-console" ? ["console"] : [])]);
    assert.deepEqual(server.env, { XENONET_PORT: "47840" });
  }
  await connect.remove();
  assert.deepEqual(entries, { claude: {}, codex: {} });
  for (const app of ["claude", "codex"]) assert.ok(calls.some((call) => call.app === app && call.args[1] === "remove" && call.args[2] === "xenonet-console"));
});

test("installed XenoNet and XenoNet Dev connect side by side: each changes its own entries and leaves the other's, and a gone checkout's are anyone's to clear", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex"]) fs.writeFileSync(path.join(bin, name), "");
  const launcher = path.join(home, "launcher");
  fs.mkdirSync(path.join(launcher, "cli"), { recursive: true });
  fs.writeFileSync(path.join(launcher, "node"), "");
  fs.writeFileSync(path.join(launcher, "cli", "xenonet.js"), "");
  const checkout = path.join(home, "checkout");
  fs.mkdirSync(path.join(checkout, "cli"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "cli", "xenonet.js"), "");
  const electron = path.join(home, "electron");
  fs.writeFileSync(electron, "");
  const servers = { claude: new Set(), codex: new Set() };
  const run = async (file, args) => {
    const app = path.basename(file);
    if (args[1] === "get") throw new Error("not found");
    if (args[1] === "remove") servers[app].delete(args[2]);
    if (args[1] === "add" || args[1] === "add-json") servers[app].add(args[2]);
    return "";
  };
  const installed = createConnect({ platform: "darwin", port: 47840, root: launcher, exec: path.join(launcher, "node"), home, env: { PATH: bin }, run });
  const dev = createConnect({ platform: "darwin", port: 47842, root: checkout, exec: electron, runAsNode: true, home, env: { PATH: bin }, run });
  const hooksOf = (file) => Object.values(JSON.parse(fs.readFileSync(file, "utf8")).hooks ?? {}).flat().flatMap((g) => g.hooks);
  const claudeFile = path.join(home, ".claude", "settings.json"), codexFile = path.join(home, ".codex", "hooks.json");

  await installed.apply();
  await dev.apply();
  assert.deepEqual([...servers.claude].sort(), ["xenonet", "xenonet-dev"], "each its own MCP server");
  const ports = (file) => [...new Set(hooksOf(file).map((h) => /(?:127\.0\.0\.1:|XENONET_PORT=)(\d+)/.exec(h.url ?? h.command)?.[1]))].sort();
  assert.deepEqual(ports(claudeFile), ["47840", "47842"], "both XenoNets hear Claude Code");
  assert.deepEqual(ports(codexFile), ["47840", "47842"]);
  const notes = (await installed.preview()).apps.flatMap((a) => a.notes);
  assert.ok(notes.some((n) => /Another XenoNet \(port 47842, XenoNet Dev\) is connected here too/.test(n)), notes.join(" | "));

  // Connecting the installed one again changes nothing of XenoNet Dev's.
  const devHooks = hooksOf(codexFile).filter((h) => /XENONET_PORT=47842/.test(h.command));
  await installed.apply();
  assert.deepEqual(hooksOf(codexFile).filter((h) => /XENONET_PORT=47842/.test(h.command)), devHooks);

  // Disconnecting XenoNet Dev takes its own away and leaves the installed one's.
  await dev.remove();
  assert.deepEqual(ports(claudeFile), ["47840"]);
  assert.deepEqual(ports(codexFile), ["47840"]);
  assert.deepEqual([...servers.claude], ["xenonet"]);

  // XenoNet Dev connected from a checkout that's since been deleted: the
  // installed XenoNet clears its broken entries when it connects.
  await dev.apply();
  fs.rmSync(checkout, { recursive: true, force: true });
  fs.rmSync(electron, { force: true });
  await installed.apply();
  assert.deepEqual(ports(codexFile), ["47840"], "the gone checkout's command hooks are cleared");
  assert.deepEqual(hooksOf(claudeFile).filter((h) => h.type === "command").map((h) => /XENONET_PORT=(\d+)/.exec(h.command)[1]), ["47840"]);
});

test("on Windows, a connected hook's quoted port and app aren't files gone, and another XenoNet's hooks stay its own", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-win-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const launcher = path.join(home, "launcher");
  fs.mkdirSync(path.join(launcher, "cli"), { recursive: true });
  fs.writeFileSync(path.join(launcher, "node.exe"), "");
  fs.writeFileSync(path.join(launcher, "cli", "xenonet.js"), "");
  const checkout = path.join(home, "checkout");
  fs.mkdirSync(path.join(checkout, "cli"), { recursive: true });
  fs.writeFileSync(path.join(checkout, "cli", "xenonet.js"), "");
  fs.writeFileSync(path.join(home, "electron.exe"), "");
  const run = async (_file, args) => (args[1] === "get" ? Promise.reject(new Error("not found")) : "");
  const installed = createConnect({ platform: "win32", port: 47840, root: launcher, exec: path.join(launcher, "node.exe"), home, env: { PATH: "" }, run });
  const dev = createConnect({ platform: "win32", port: 47842, root: checkout, exec: path.join(home, "electron.exe"), runAsNode: true, home, env: { PATH: "" }, run });
  await installed.apply();
  await dev.apply();
  const preview = await installed.preview();
  assert.deepEqual(preview.apps.map((a) => a.broken), [[], []], "nothing reported gone");
  const codex = JSON.parse(fs.readFileSync(path.join(home, ".codex", "hooks.json"), "utf8"));
  const ports = (hooks) => [...new Set(Object.values(hooks).flat().flatMap((g) => g.hooks).map((h) => /XENONET_PORT='(\d+)'/.exec(Buffer.from(/-EncodedCommand ([A-Za-z0-9+/=]+)$/.exec(h.commandWindows)[1], "base64").toString("utf16le"))[1]))].sort();
  assert.deepEqual(ports(codex.hooks), ["47840", "47842"], "both XenoNets' hooks, side by side");
  await installed.apply();
  assert.deepEqual(ports(JSON.parse(fs.readFileSync(path.join(home, ".codex", "hooks.json"), "utf8")).hooks), ["47840", "47842"], "connecting the installed one again leaves XenoNet Dev's");
});
