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
  const launcher = "/xenonet/runtime/launcher/xenonet";
  const connect = createConnect({ platform: "darwin", port: 47840, bin: launcher, home, env: { PATH: bin, XENONET_LAUNCHER_BIN: launcher }, run: async (file, args) => {
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
    assert.equal(server.command, launcher);
    assert.deepEqual(server.args, ["mcp", ...(name === "xenonet-console" ? ["console"] : [])]);
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
  const launcher = path.join(home, "launcher", "xenonet");
  fs.mkdirSync(path.dirname(launcher), { recursive: true });
  fs.writeFileSync(launcher, "");
  const checkout = path.join(home, "checkout");
  const built = path.join(checkout, "target", "debug", "xenonet");
  fs.mkdirSync(path.dirname(built), { recursive: true });
  fs.writeFileSync(built, "");
  const servers = { claude: new Set(), codex: new Set() };
  const run = async (file, args) => {
    const app = path.basename(file);
    if (args[1] === "get") throw new Error("not found");
    if (args[1] === "remove") servers[app].delete(args[2]);
    if (args[1] === "add" || args[1] === "add-json") servers[app].add(args[2]);
    return "";
  };
  const installed = createConnect({ platform: "darwin", port: 47840, bin: launcher, home, env: { PATH: bin }, run });
  const dev = createConnect({ platform: "darwin", port: 47842, bin: built, home, env: { PATH: bin }, run });
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
  await installed.apply();
  assert.deepEqual(ports(codexFile), ["47840"], "the gone checkout's command hooks are cleared");
  assert.deepEqual(hooksOf(claudeFile).filter((h) => h.type === "command").map((h) => /XENONET_PORT=(\d+)/.exec(h.command)[1]), ["47840"]);
});

test("on Windows, a connected hook's quoted port and app aren't files gone, and another XenoNet's hooks stay its own", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-win-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const launcher = path.join(home, "launcher", "xenonet.exe");
  fs.mkdirSync(path.dirname(launcher), { recursive: true });
  fs.writeFileSync(launcher, "");
  const built = path.join(home, "checkout", "target", "debug", "xenonet.exe");
  fs.mkdirSync(path.dirname(built), { recursive: true });
  fs.writeFileSync(built, "");
  const run = async (_file, args) => (args[1] === "get" ? Promise.reject(new Error("not found")) : "");
  const installed = createConnect({ platform: "win32", port: 47840, bin: launcher, home, env: { PATH: "" }, run });
  const dev = createConnect({ platform: "win32", port: 47842, bin: built, home, env: { PATH: "" }, run });
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

test("an earlier XenoNet's entries, which ran Node and cli/xenonet.js, show as different, and connecting again has them run the binary", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-earlier-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex"]) fs.writeFileSync(path.join(bin, name), "");
  const old = path.join(home, "launcher");
  fs.mkdirSync(path.join(old, "cli"), { recursive: true });
  for (const f of ["node", "cli/xenonet.js", "xenonet"]) fs.writeFileSync(path.join(old, f), "");
  const node = `XENONET_PORT=47840 '${old}/node' '${old}/cli/xenonet.js'`;
  const nodeServer = { type: "stdio", command: path.join(old, "node"), args: [path.join(old, "cli", "xenonet.js"), "mcp"], env: { XENONET_PORT: "47840" } };
  const servers = { claude: { xenonet: nodeServer }, codex: { xenonet: nodeServer } };
  const writeClaudeState = () => fs.writeFileSync(path.join(home, ".claude.json"), JSON.stringify({ mcpServers: servers.claude }));
  writeClaudeState();
  fs.mkdirSync(path.join(home, ".claude"));
  fs.writeFileSync(path.join(home, ".claude", "settings.json"), JSON.stringify({ hooks: {
    SessionStart: [{ hooks: [{ type: "command", command: `${node} hook claude`, timeout: 10 }] }],
    Stop: [{ hooks: [{ type: "http", url: "http://127.0.0.1:47840/api/hook/claude", timeout: 5 }, { type: "command", command: "my-own-stop-hook" }] }],
  } }));
  fs.mkdirSync(path.join(home, ".codex"));
  fs.writeFileSync(path.join(home, ".codex", "hooks.json"), JSON.stringify({ hooks: { SessionStart: [{ hooks: [{ type: "command", command: `${node} hook codex`, timeout: 10 }] }] } }));
  const run = async (file, args) => {
    const app = path.basename(file);
    const [, op, name] = args;
    if (op === "get") {
      if (!servers[app][name]) throw new Error("not found");
      return JSON.stringify({ transport: servers[app][name] });
    }
    if (op === "remove") delete servers[app][name];
    if (op === "add-json") servers[app][name] = JSON.parse(args[3]);
    if (op === "add") servers[app][name] = { command: args[args.indexOf("--") + 1], args: args.slice(args.indexOf("--") + 2), env: { XENONET_PORT: "47840" } };
    if (app === "claude") writeClaudeState();
    return "";
  };
  const connect = createConnect({ platform: "darwin", port: 47840, bin: path.join(old, "xenonet"), home, env: { PATH: bin }, run });
  const hooksOf = (file) => Object.values(JSON.parse(fs.readFileSync(path.join(home, file), "utf8")).hooks ?? {}).flat().flatMap((g) => g.hooks);

  const before = await connect.preview();
  for (const app of before.apps) {
    assert.deepEqual(app.changes.map((c) => [c.id, c.state]), [["mcp", "different"], ["hooks", "different"]], app.app);
    assert.ok(app.notes.some((n) => /run cli\/xenonet\.js with Node, as an earlier XenoNet wrote them\. Connecting again has them run the xenonet binary/.test(n)), app.notes.join(" | "));
  }

  const after = await connect.apply();
  assert.ok(after.apps.every((app) => app.changes.every((c) => c.state === "present")));
  assert.ok(after.apps.every((app) => !app.notes.some((n) => /earlier XenoNet/.test(n))));
  for (const file of [".claude/settings.json", ".codex/hooks.json"]) {
    const commands = hooksOf(file).filter((h) => h.type === "command" && h.command !== "my-own-stop-hook").map((h) => h.command);
    assert.ok(commands.length && commands.every((c) => c.startsWith(`XENONET_PORT=47840 '${old}/xenonet' hook `)), commands.join(" | "));
  }
  assert.ok(hooksOf(".claude/settings.json").some((h) => h.command === "my-own-stop-hook"), "your own hooks stay");
  for (const app of ["claude", "codex"]) assert.deepEqual([servers[app].xenonet.command, servers[app].xenonet.args], [path.join(old, "xenonet"), ["mcp"]]);

  // Disconnecting takes either form out: the old one, written again here,
  // goes as the binary's does.
  fs.writeFileSync(path.join(home, ".codex", "hooks.json"), JSON.stringify({ hooks: { Stop: [{ hooks: [{ type: "command", command: `${node} hook codex` }, { type: "command", command: `XENONET_PORT=47840 '${old}/xenonet' hook codex` }] }] } }));
  await connect.remove();
  assert.deepEqual(hooksOf(".codex/hooks.json"), []);
  assert.deepEqual(hooksOf(".claude/settings.json").map((h) => h.command), ["my-own-stop-hook"]);
});
