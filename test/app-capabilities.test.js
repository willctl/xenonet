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
import { PROCESSES, agentApps, agentOf } from "../core/processes.js";
import { createVerifier } from "../core/engine/verify.js";
import { EngineStore } from "../core/engine/store.js";
import { CALLERS, callingApps, callingSession } from "../mcp/xenonet.js";
import { actionsFor, urlFor, handoffUrl } from "../core/open.js";
import { DELIVERY, deliveryApps } from "../core/delivery/apps.js";
import { resumeCommand, resumeEnv } from "../core/delivery/resume.js";
import { planUsage } from "../core/delivery/usage.js";

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

test("processes: the apps whose processes XenoNet tells apart, and so the apps verify checks, are exactly Claude Code, Codex and OpenCode", () => {
  assert.deepEqual(agentApps().map((a) => [a.id, a.name]), [["claude", "Claude Code"], ["codex", "Codex"], ["opencode", "OpenCode"]]);
  assert.deepEqual(Object.keys(PROCESSES), ["claude", "codex", "opencode"]);
  assert.deepEqual(agentApps().filter((a) => a.pinsByHook).map((a) => a.id), ["claude"], "only Claude Code's hooks pin a session");
  assert.deepEqual(agentApps().filter((a) => a.anyProcess).map((a) => a.id), ["opencode"], "only OpenCode's sessions are any of its processes'");
  for (const [program, args, agent] of [
    ["/Users/me/.local/bin/claude", "", "claude"],
    ["/Users/me/.local/share/claude/versions/2.1.288", "", "claude"],
    ["C:\\Users\\me\\.local\\bin\\claude", "", "claude"],
    ["/path/codex-aarch64-apple-darwin", "", "codex"],
    ["/usr/local/bin/opencode", "", "opencode"],
    ["C:\\tools\\opencode.exe", "", "opencode"],
    ["/usr/local/bin/node", "node /x/@anthropic-ai/claude-code/cli.js", "claude"],
    ["/usr/local/bin/bun", "bun /x/@openai/codex/bin/codex.js", "codex"],
    ["/usr/local/bin/node", "node /x/opencode-ai/bin/opencode", null],
    ["/usr/local/bin/python3", "python3 /x/@openai/codex/x", null],
    ["/tmp/claude-decoy/run", "", null],
    ["/usr/bin/codexterm", "", null],
    ["", "", null],
  ]) assert.equal(agentOf(program, args), agent, `${program} ${args}`);
});

test("processes: a made-up app in a registry of its own is told by its own rules, with no change to agentOf", () => {
  const registry = [
    { id: "wren", name: "Wren", numbat: ["wren"], has: ["processes"] },
    { id: "claude", name: "Claude Code", numbat: ["claude-code"], has: ["processes"] },
    { id: "finch", name: "Finch", numbat: ["finch"] },
    { id: "lark", name: "Lark", numbat: ["lark"], has: ["processes"] },
  ];
  const rules = { ...PROCESSES, wren: { program: (_p, base) => base === "wren", script: (args) => /@wren\/cli\//.test(args) } };
  const agents = agentApps(registry, rules);
  assert.deepEqual(agents.map((a) => a.id), ["wren", "claude"], "finch has no processes, lark no rules");
  assert.equal(agentOf("/usr/local/bin/wren", "", agents), "wren");
  assert.equal(agentOf("/usr/local/bin/node", "node /x/@wren/cli/main.js", agents), "wren");
  assert.equal(agentOf("/usr/local/bin/claude", "", agents), "claude");
  assert.equal(agentOf("/usr/local/bin/codex", "", agents), null, "not in this registry");
});

test("processes: verify checks a session's app by its processes, and passes an app outside them unchecked, as before", async (t) => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-capabilities-"));
  const stores = [];
  t.after(() => {
    for (const s of stores) s.close();
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 });
  });
  // One process, whichever app the test says it is, and a store of its own.
  const verifier = (agent) => {
    const procs = { supported: true, peer: async () => 50, chain: async () => [{ pid: 50, start: 1, program: "/x", agent, product: null }], alive: async () => true, signed: async () => true };
    const store = new EngineStore(path.join(dir, `engine-${stores.length}.sqlite`));
    stores.push(store);
    return createVerifier({ procs, store });
  };
  const socket = { remotePort: 1, localPort: 2 };
  assert.deepEqual(await verifier("claude").check(socket, "gemini-cli:x"), { ok: true, checked: false }, "an app whose processes XenoNet doesn't tell apart");
  assert.deepEqual(await verifier("claude").check(socket, "wren:x"), { ok: true, checked: false });
  assert.deepEqual(await verifier("opencode").check(socket, "opencode:x"), { ok: true, checked: true, root: { pid: 50, start: 1 } });
  assert.deepEqual(await verifier("claude").check(socket, "opencode:x"), { ok: false, checked: true, reason: "requests in opencode:x's name have to come from an OpenCode process, and this one doesn't" });
  assert.deepEqual(await verifier("opencode").check(socket, "codex:x"), { ok: false, checked: true, reason: "requests in codex:x's name have to come from a Codex process, and this one doesn't" });
  assert.deepEqual(await verifier("codex").check(socket, "claude:x"), { ok: false, checked: true, reason: "requests in claude:x's name have to come from a Claude Code process, and this one doesn't" });
  assert.deepEqual(await verifier("claude").check(socket, "claude:x", { hook: true }), { ok: true, checked: true, root: { pid: 50, start: 1 }, sure: true }, "Claude Code's own hook pins its session");
  assert.deepEqual(await verifier("codex").check(socket, "codex:x", { hook: true }), { ok: true, checked: true, root: { pid: 50, start: 1 } }, "Codex's hooks don't");
});

// crates/xenonet-mcp/src/xenonet.rs has the same cases.
test("caller: the calling session is the first app's that says one is, Claude Code's then Codex's", () => {
  assert.deepEqual(callingApps().map(([app]) => app), ["claude", "codex"]);
  const ask = (env, meta = {}) => callingSession({ env, meta })?.id ?? null;
  assert.equal(ask({}), null);
  assert.equal(ask({ CLAUDE_CODE_SESSION_ID: "abc", CODEX_THREAD_ID: "t" }, { threadId: "u" }), "claude:abc");
  assert.equal(ask({ CLAUDE_CODE_SESSION_ID: "" }, { threadId: "u" }), "codex:u", "an empty session id is none");
  assert.equal(ask({ CODEX_THREAD_ID: "w" }, { "x-codex-turn-metadata": { thread_id: "v" } }), "codex:v");
  assert.equal(ask({}, { threadId: null, "x-codex-turn-metadata": { thread_id: "v" } }), "codex:v");
  assert.equal(ask({ CODEX_THREAD_ID: "w" }), "codex:w");
  assert.equal(ask({ CODEX_THREAD_ID: "w" }, { threadId: "" }), null, "an empty thread id is none, and the variable isn't asked");
  assert.equal(ask({}, { threadId: 7 }), "codex:7");
  assert.deepEqual(callingSession({ env: { CLAUDE_CODE_SESSION_ID: "abc" } }), { id: "claude:abc", app: "claude" });
  assert.equal(CALLERS.claude.started({ CLAUDE_CODE_SESSION_ID: "abc" }), true);
  assert.equal(CALLERS.codex.started, undefined, "Codex's thread doesn't keep an actor from this server");
  // A made-up app's own way, in a registry of its own.
  const registry = [{ id: "wren", name: "Wren", numbat: ["wren"], has: ["caller"] }, { id: "codex", name: "Codex", numbat: ["codex"], has: ["caller"] }, { id: "lark", name: "Lark", numbat: ["lark"] }];
  const apps = callingApps(registry, { ...CALLERS, wren: { session: (env) => env.WREN_SESSION }, lark: { session: () => "never" } });
  assert.deepEqual(apps.map(([app]) => app), ["wren", "codex"]);
  assert.deepEqual(callingSession({ env: { WREN_SESSION: "s1" }, meta: { threadId: "u" }, apps }), { id: "wren:s1", app: "wren" });
  assert.deepEqual(callingSession({ env: {}, meta: { threadId: "u" }, apps }), { id: "codex:u", app: "codex" });
});

test("open: each app opens its own sessions, starts its own in a project, and takes a handoff, as before", () => {
  const uuid = "0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b";
  const session = (fields) => ({ kind: "session", cwd: "/p", ...fields });
  const opens = (node) => actionsFor(node).filter((a) => a.id === "open-app").map((a) => a.label);
  assert.deepEqual(opens(session({ app: "claude", desktopId: "local_abc", rawId: uuid })), ["Open in Claude desktop"]);
  assert.deepEqual(opens(session({ app: "claude", rawId: uuid })), ["Import into Claude desktop"]);
  assert.deepEqual(opens(session({ app: "claude", desktopId: "nope", rawId: uuid })), [], "a desktop id that isn't one offers nothing");
  assert.deepEqual(opens(session({ app: "codex", rawId: uuid })), ["Open in ChatGPT · Codex"]);
  assert.deepEqual(opens(session({ app: "codex", rawId: "x&calc.exe" })), []);
  assert.deepEqual(opens(session({ app: "opencode", rawId: uuid })), [], "an app without open");
  assert.equal(urlFor(session({ app: "claude", desktopId: "local_a b" }), "open-app"), null);
  assert.equal(urlFor(session({ app: "claude", desktopId: "local_abc", rawId: uuid }), "open-app"), "claude://code/continue?session=local_abc");
  assert.equal(urlFor(session({ app: "claude", desktopId: "nope", rawId: uuid }), "open-app"), `claude://resume?session=${uuid}`, "the URL still resumes it");
  assert.equal(urlFor(session({ app: "codex", rawId: uuid }), "open-app"), `codex://threads/${uuid}`);
  assert.equal(urlFor(session({ app: "opencode", rawId: uuid }), "open-app"), null);
  const project = { kind: "project", path: "/Users/me/a b" };
  assert.deepEqual(actionsFor(project).map((a) => a.id), ["open-folder", "open-vscode", "new-claude"]);
  assert.equal(urlFor(project, "new-claude"), "claude://code/new?folder=%2FUsers%2Fme%2Fa%20b");
  assert.equal(urlFor({ kind: "project" }, "new-claude"), null, "no folder");
  assert.equal(urlFor(project, "new-codex"), null, "Codex starts none here");
  assert.equal(handoffUrl("claude", "/p", "go"), "claude://code/new?folder=%2Fp&q=go");
  assert.equal(handoffUrl("codex", "", "go"), "codex://threads/new?prompt=go");
  assert.equal(handoffUrl("opencode", "/p", "go"), null);
});

test("delivery: each app's resume, the environment a resumed turn keeps, its plan's usage, as before", () => {
  assert.deepEqual(deliveryApps().map((a) => [a.id, !!a.resume, !!a.channel, !!a.server]), [["claude", true, true, false], ["codex", true, false, false], ["opencode", false, false, true]]);
  const id = "0190d3e0-0000-7000-8000-00000000ab01";
  assert.deepEqual(resumeCommand(`claude:${id}`), ["claude", ["-p", "--resume", id]]);
  assert.deepEqual(resumeCommand(`claude:${id}`, { settings: "/s/listen.json" }), ["claude", ["-p", "--resume", id, "--settings", "/s/listen.json"]]);
  assert.deepEqual(resumeCommand(`codex:${id}`, { settings: "/s/listen.json" }), ["codex", ["exec", "resume", id, "-"]]);
  for (const agent of [`opencode:${id}`, `gemini-cli:${id}`, "claude:abc", "claude", `Claude:${id}`]) assert.equal(resumeCommand(agent), null, agent);
  const env = { PATH: "/bin", CLAUDECODE: "1", CLAUDE_CONFIG_DIR: "/c", CODEX_HOME: "/x", CODEX_THREAD_ID: "t", CODEXX: "1", XENONET_PORT: "47840", OPENCODE_X: "1" };
  assert.deepEqual(Object.keys(resumeEnv(env)), ["PATH", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "CODEXX", "OPENCODE_X"]);
  const now = Date.UTC(2026, 9, 20);
  const limits = { claude: { fiveHour: 40, sevenDay: 70, resets: {} }, codex: { observedAt: now, primary: { pct: 20 }, secondary: { pct: 90, resetsAt: now - 1 } } };
  assert.equal(planUsage(limits, "claude", now), 0.7);
  assert.equal(planUsage(limits, "codex", now), 0.2, "a window that's reset doesn't count");
  for (const app of ["opencode", "gemini-cli", "toString", ""]) assert.equal(planUsage(limits, app, now), null, app);
  // A made-up app's own ways, in a registry of its own.
  const registry = [{ id: "wren", name: "Wren", numbat: ["wren"], roots: { env: "WREN_HOME", default: "~/.wren" }, has: ["delivery"] }, { id: "lark", name: "Lark", numbat: ["lark"] }];
  const wren = deliveryApps(registry, { ...DELIVERY, wren: { resume: (sid) => ["wren", ["resume", sid]], marks: "WREN_" }, lark: { server: true } });
  assert.deepEqual(wren.map((a) => [a.id, a.roots.env]), [["wren", "WREN_HOME"]]);
  assert.deepEqual(wren[0].resume(id), ["wren", ["resume", id]]);
});
