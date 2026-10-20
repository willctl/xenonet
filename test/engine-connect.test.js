import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
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

test("an uninstall from Desktop, whose environment names no launcher, still takes out the console's server when asked", async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex"]) fs.writeFileSync(path.join(bin, name), "");
  const calls = [];
  const run = async (file, args) => (calls.push([path.basename(file), ...args]), "");
  const removed = () => calls.filter((c) => c[2] === "remove").map((c) => `${c[0]} ${c[3]}`).sort();
  await createConnect({ platform: "darwin", port: 47840, root: "/Applications/XenoNet.app/Contents/Resources/app", home, env: { PATH: bin }, run }).remove();
  assert.deepEqual(removed(), ["claude xenonet", "codex xenonet"], "without it, as before");
  calls.length = 0;
  await createConnect({ platform: "darwin", port: 47840, root: "/Applications/XenoNet.app/Contents/Resources/app", home, env: { PATH: bin }, run, consoleMcp: true }).remove();
  assert.deepEqual(removed(), ["claude xenonet", "claude xenonet-console", "codex xenonet", "codex xenonet-console"]);
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
    assert.ok(commands.length && commands.every((c) => c.startsWith(`XENONET_PORT=47840 '${path.join(old, "xenonet")}' hook `)), commands.join(" | "));
  }
  assert.ok(hooksOf(".claude/settings.json").some((h) => h.command === "my-own-stop-hook"), "your own hooks stay");
  for (const app of ["claude", "codex"]) assert.deepEqual([servers[app].xenonet.command, servers[app].xenonet.args], [path.join(old, "xenonet"), ["mcp"]]);

  // Disconnecting takes either form out: the old one, written again here,
  // goes as the binary's does.
  fs.writeFileSync(path.join(home, ".codex", "hooks.json"), JSON.stringify({ hooks: { Stop: [{ hooks: [{ type: "command", command: `${node} hook codex` }, { type: "command", command: `XENONET_PORT=47840 '${path.join(old, "xenonet")}' hook codex` }] }] } }));
  await connect.remove();
  assert.deepEqual(hooksOf(".codex/hooks.json"), []);
  assert.deepEqual(hooksOf(".claude/settings.json").map((h) => h.command), ["my-own-stop-hook"]);
});

// A home with Claude Code and Codex on PATH, and XenoNet's plugin for Claude
// Code installed and turned on as Claude Code records it (settings.json's
// enabledPlugins, plugins/installed_plugins.json) when `plugin` is given.
function pluginHome(t, { plugin = "xenonet@xenonet", enabled = true, installed = true, command = true } = {}) {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-connect-plugin-"));
  t.after(() => fs.rmSync(home, { recursive: true, force: true }));
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex", ...(command ? ["xenonet"] : [])]) fs.writeFileSync(path.join(bin, name), "");
  const launcher = path.join(home, "launcher", "xenonet");
  fs.mkdirSync(path.dirname(launcher), { recursive: true });
  fs.writeFileSync(launcher, "");
  const claude = path.join(home, ".claude");
  fs.mkdirSync(path.join(claude, "plugins"), { recursive: true });
  const settings = { model: "x", hooks: { Stop: [{ hooks: [{ type: "command", command: "my-own-stop-hook" }] }] } };
  if (plugin) settings.enabledPlugins = { "other@somewhere": true, [plugin]: enabled };
  fs.writeFileSync(path.join(claude, "settings.json"), JSON.stringify(settings));
  if (plugin && installed) fs.writeFileSync(path.join(claude, "plugins", "installed_plugins.json"), JSON.stringify({ version: 2, plugins: { [plugin]: [{ scope: "user", installPath: path.join(claude, "plugins", "cache", "xenonet", "xenonet", "0.3.0"), version: "0.3.0" }] } }));
  const servers = { claude: {}, codex: {} }, calls = [];
  const run = async (file, args) => {
    const app = path.basename(file);
    calls.push([app, ...args].join(" "));
    const [, op, name] = args;
    if (op === "get") {
      if (!servers[app][name]) throw new Error("not found");
      return JSON.stringify({ transport: servers[app][name] });
    }
    if (op === "remove") delete servers[app][name];
    if (op === "add-json") servers[app][name] = JSON.parse(args[3]);
    if (op === "add") servers[app][name] = { command: args[args.indexOf("--") + 1], args: args.slice(args.indexOf("--") + 2), env: { XENONET_PORT: "47840" } };
    if (app === "claude") fs.writeFileSync(path.join(home, ".claude.json"), JSON.stringify({ mcpServers: servers.claude }));
    return "";
  };
  const connect = (port = 47840) => createConnect({ platform: "darwin", port, bin: launcher, home, env: { PATH: bin }, run });
  const settingsFile = path.join(claude, "settings.json");
  const hooks = () => Object.values(JSON.parse(fs.readFileSync(settingsFile, "utf8")).hooks ?? {}).flat().flatMap((g) => g.hooks);
  const enable = () => {
    const s = JSON.parse(fs.readFileSync(settingsFile, "utf8"));
    fs.writeFileSync(settingsFile, JSON.stringify({ ...s, enabledPlugins: { "xenonet@xenonet": true } }));
    fs.writeFileSync(path.join(claude, "plugins", "installed_plugins.json"), JSON.stringify({ version: 2, plugins: { "xenonet@xenonet": [{ scope: "user", version: "0.3.0" }] } }));
  };
  return { home, launcher, servers, calls, connect, hooks, enable, settingsFile };
}

const claudeOf = (preview) => preview.apps.find((a) => a.app === "claude");

test("with XenoNet's plugin installed and turned on, connecting leaves Claude Code to it and adds nothing of its own there", async (t) => {
  const h = pluginHome(t);
  const connect = h.connect();
  const before = claudeOf(await connect.preview());
  assert.deepEqual(before.changes.map((c) => [c.id, c.state, c.via]), [["mcp", "present", "plugin"], ["hooks", "present", "plugin"]]);
  assert.ok(before.changes.every((c) => c.where === "XenoNet's plugin for Claude Code (xenonet@xenonet)"));
  assert.ok(before.notes.some((n) => /Claude Code has XenoNet's plugin \(xenonet@xenonet\), which carries the xenonet MCP server and the hooks/.test(n)), before.notes.join(" | "));
  assert.deepEqual(before.broken, []);

  const after = await connect.apply();
  assert.deepEqual(h.hooks().map((x) => x.command), ["my-own-stop-hook"], "no hook of connect's in Claude Code's settings");
  assert.deepEqual(Object.keys(h.servers.claude), [], "and no server of its own");
  assert.ok(!h.calls.some((c) => c.startsWith("claude mcp add-json")));
  assert.ok(claudeOf(after).changes.every((c) => c.state === "present" && c.via === "plugin"));
  // Codex has no plugin, so it's connected as before.
  assert.ok(after.apps.find((a) => a.app === "codex").changes.every((c) => c.state === "present" && !c.via));
  assert.equal(await connect.hookConnected("Stop"), false, "the plugin's hooks are the ones that count");
});

test("connect's own entries beside the plugin's show as to take out, the plugin's hooks stand by meanwhile, and connecting again keeps only the plugin's", async (t) => {
  const h = pluginHome(t, { plugin: null });
  const connect = h.connect();
  await connect.apply();
  assert.equal(await connect.hookConnected("Stop"), true);
  assert.equal(await connect.hookConnected("SessionStart"), true);

  // Then the plugin is installed: both are there.
  h.enable();
  const twice = claudeOf(await connect.preview());
  assert.deepEqual(twice.changes.map((c) => [c.id, c.state]), [["mcp", "remove"], ["hooks", "remove"]]);
  assert.ok(twice.notes.some((n) => /XenoNet ignores the plugin's copy of any hook connect added, so each event still arrives once/.test(n)), twice.notes.join(" | "));
  for (const event of ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "SessionEnd", "Notification"]) assert.equal(await connect.hookConnected(event), true, event);

  const after = await connect.apply(["claude"]);
  assert.deepEqual(h.hooks().map((x) => x.command), ["my-own-stop-hook"], "connect's own hooks out, yours kept");
  assert.deepEqual(Object.keys(h.servers.claude), []);
  assert.ok(h.calls.includes("claude mcp remove xenonet --scope user"));
  assert.ok(claudeOf(after).changes.every((c) => c.state === "present" && c.via === "plugin"));
  assert.equal(await connect.hookConnected("Stop"), false, "now the plugin's hooks count");
});

test("a turned-off or uninstalled plugin, or XenoNet Dev's own connect, isn't left to the plugin", async (t) => {
  for (const [why, options, port] of [["turned off", { enabled: false }, 47840], ["not installed", { installed: false }, 47840], ["another marketplace's plugin of another name", { plugin: "xenonet-tools@xenonet" }, 47840], ["XenoNet Dev", {}, 47842]]) {
    const h = pluginHome(t, options);
    const preview = claudeOf(await h.connect(port).preview());
    assert.deepEqual(preview.changes.map((c) => [c.id, c.state, c.via]), [["mcp", "missing", undefined], ["hooks", "missing", undefined]], why);
    await h.connect(port).apply();
    assert.ok(h.hooks().some((x) => x.type === "http" && x.url === `http://127.0.0.1:${port}/api/hook/claude`), why);
  }
});

test("the plugin runs xenonet from PATH, so a missing command is something broken, and a missing claude command doesn't matter", async (t) => {
  const h = pluginHome(t, { command: false });
  assert.deepEqual(claudeOf(await h.connect().preview()).broken, ["XenoNet's plugin runs the xenonet command, which isn't on your PATH"]);
  fs.rmSync(path.join(h.home, "bin", "claude"));
  assert.ok(!claudeOf(await h.connect().preview()).notes.some((n) => /can't find the claude command/.test(n)));
});

test("whether connect's own hook for an event is there: only this XenoNet's, only one that can run, and none in settings that can't be read", async (t) => {
  const h = pluginHome(t, { plugin: null });
  const connect = h.connect();
  assert.equal(await connect.hookConnected("Stop"), false, "nothing connected yet");
  await connect.apply();
  assert.equal(await connect.hookConnected("Stop"), true);
  assert.equal(await connect.hookConnected("SubagentStop"), false, "an event connect adds no hook for");
  assert.equal(await h.connect(47842).hookConnected("Stop"), false, "XenoNet Dev's own aren't there");
  // SessionStart's command runs the launcher; once that's gone it can't run.
  fs.rmSync(h.launcher);
  assert.equal(await connect.hookConnected("SessionStart"), false);
  assert.equal(await connect.hookConnected("Stop"), true, "an HTTP hook runs no file");
  fs.writeFileSync(h.settingsFile, "{ not json");
  assert.equal(await connect.hookConnected("Stop"), false);
});

test("XenoNet's plugin for Claude Code carries exactly connect's hooks and MCP server", async (t) => {
  const h = pluginHome(t, { plugin: null });
  await h.connect().apply(["claude"]);
  const plugin = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "plugins", "xenonet");
  const pluginHooks = JSON.parse(fs.readFileSync(path.join(plugin, "hooks", "hooks.json"), "utf8")).hooks;
  const connected = JSON.parse(fs.readFileSync(h.settingsFile, "utf8")).hooks;
  const ours = (groups) => groups.map((g) => ({ ...g, hooks: g.hooks.filter((x) => x.command !== "my-own-stop-hook") })).filter((g) => g.hooks.length);
  assert.deepEqual(Object.keys(pluginHooks).sort(), Object.keys(connected).sort(), "the same events");
  for (const [event, groups] of Object.entries(pluginHooks)) {
    const theirs = ours(connected[event]);
    assert.equal(groups.length, 1, event);
    assert.equal(theirs.length, 1, event);
    assert.equal(groups[0].matcher, theirs[0].matcher, event);
    const [p] = groups[0].hooks, [c] = theirs[0].hooks;
    assert.equal(p.type, c.type, event);
    assert.equal(p.timeout, c.timeout, event);
    if (c.type === "http") assert.equal(p.url, `${c.url}?via=plugin`, event);
    else assert.equal(p.command, "xenonet hook claude --port=47840 --via=plugin", event);
  }
  const server = JSON.parse(fs.readFileSync(path.join(plugin, ".mcp.json"), "utf8")).mcpServers.xenonet;
  assert.deepEqual(server, { ...h.servers.claude.xenonet, command: "xenonet" }, "connect's server, run from PATH");
});
