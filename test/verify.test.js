// Whether a request made in an agent's name comes from that agent
// (core/engine/verify.js), against a made-up process table: which process
// is on the other end of a connection, its ancestors, and which agent
// processes are signed. The pins live in a real engine store in a temp dir.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createVerifier, bundleOf, actorRefusal, ownOf } from "../core/engine/verify.js";
import { EngineStore } from "../core/engine/store.js";
import { agentOf, productOf, createProcs } from "../core/procs.js";
import { fakeProcesses } from "./harness/procs.js";

// Temp folders, taken away after the tests.
const made = [];
const stores = [];
const makeStore = (...args) => { const s = new EngineStore(...args); stores.push(s); return s; };
after(() => stores.forEach((s) => { if (s.db.isOpen) s.close(); }));
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-verify-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

// A small machine: the Claude app runs two Claude Code sessions, A and B,
// each its own signed process. A has an MCP server and a shell under it;
// there's also a plain terminal, and XenoNet's own app with a window (its
// helper) and a terminal with Claude Code and XenoNet's own binary run as
// Node (the xenonet command) in it.
const XENONET = 600;
function machine() {
  const rows = [
    [1, 0, "/sbin/launchd"],
    [100, 1, "/Applications/Claude.app/Contents/MacOS/Claude"],
    [200, 100, "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude"],
    [210, 200, "/usr/local/bin/node"],
    [220, 200, "/bin/zsh"],
    [300, 100, "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude"],
    [400, 1, "/bin/zsh"],
    [XENONET, 1, "/Applications/XenoNet.app/Contents/MacOS/XenoNet"],
    [610, XENONET, "/Applications/XenoNet.app/Contents/Frameworks/XenoNet Helper.app/Contents/MacOS/XenoNet Helper"],
    [620, XENONET, "/bin/zsh"],
    [630, 620, "/Users/me/.local/bin/claude"],
    [640, 620, "/Applications/XenoNet.app/Contents/MacOS/XenoNet"],
  ];
  const table = new Map(rows.map(([pid, ppid, program]) => [pid, { pid, ppid, start: 1000 + pid, program }]));
  const signed = new Set([200, 300, 630]);
  return { table, signed };
}

function fakeProcs({ table, signed }, { supported = true, calls = { chain: 0 } } = {}) {
  return {
    supported,
    peer: async (socket) => socket.pid,
    chain: async (pid) => {
      calls.chain++;
      const out = [];
      for (let p = table.get(pid); p; p = p.ppid > 1 ? table.get(p.ppid) : null) {
        const agent = agentOf(p.program, p.args ?? "");
        out.push({ ...p, agent, product: agent ? null : productOf(p.program, p.args ?? "") });
      }
      return out;
    },
    alive: async ({ pid, start }) => table.get(pid)?.start === start,
    signed: async (p) => signed.has(p.pid),
    cwd: async (pid) => table.get(pid)?.cwd ?? null,
  };
}

// A machine with actors on it, made up: Claude Code (session A and its
// shell); a terminal where Wren, a program of your own, runs as a daemon
// and calls XenoNet through the xenonet command; Grok Bot's app, with a helper
// and a command it runs (the xenonet command, and a claude it started);
// Hermes's terminal UI and agent host, with a shell its terminal tool runs
// and the xenonet MCP server it started; Hermes.app's own processes; Muse's
// app, with a program it runs (the xenonet command); a bundle that calls
// itself Grok Bot.app but isn't signed by Cursor, running the xenonet command
// too; and XenoNet with its window and its terminal. Claude Code, Grok Bot and
// Muse are signed by their makers.
const HERMES_PY = "/Users/me/.hermes/tools/python-3.14.7/bin/python3.14";
const HERMES_APP = "/Users/me/.hermes/hermes-agent/apps/desktop/release/mac-arm64/Hermes.app/Contents";
const GROK = "/Applications/Grok Bot.app/Contents";
const MUSE = "/Applications/Muse.app/Contents";
function actorMachine() {
  const rows = [
    [1, 0, "/sbin/launchd"],
    [100, 1, "/Applications/Claude.app/Contents/MacOS/Claude"],
    [200, 100, "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude"],
    [220, 200, "/bin/zsh"],
    [400, 1, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"],
    [410, 400, "/bin/zsh"],
    [420, 410, "/usr/bin/python3", "/usr/bin/python3 /Users/me/bin/wren"],
    [430, 420, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js actor heartbeat", "/Users/me/Projects/docs-site"],
    [500, 1, `${GROK}/MacOS/Grok Bot`],
    [510, 500, `${GROK}/Frameworks/Grok Bot Helper.app/Contents/MacOS/Grok Bot Helper`],
    [520, 500, "/bin/zsh", "/bin/zsh -c xenonet actor heartbeat", "/Users/me/Projects/pathfinder"],
    [530, 520, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js actor heartbeat"],
    [540, 520, "/Users/me/.local/bin/claude"],
    [700, 410, "/usr/local/bin/node", "node /Users/me/.hermes/hermes-agent/ui-tui/dist/entry.js"],
    [710, 700, HERMES_PY, `${HERMES_PY} -m tui_gateway.entry`],
    [720, 710, "/bin/bash", "/bin/bash -c xenonet actor event ran"],
    [730, 720, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js actor event ran"],
    [740, 710, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js mcp"],
    [800, 1, `${HERMES_APP}/MacOS/Hermes`],
    [810, 800, `${HERMES_APP}/Frameworks/Hermes Helper.app/Contents/MacOS/Hermes Helper`],
    [900, 1, `${MUSE}/MacOS/Muse`],
    [910, 900, "/bin/zsh", "/bin/zsh -c xenonet actor send actor:grok-bot hello"],
    [920, 910, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js actor send actor:grok-bot hello"],
    [950, 1, "/Users/me/Downloads/Grok Bot.app/Contents/MacOS/Grok Bot"],
    [960, 950, "/bin/zsh", "/bin/zsh -c xenonet actor send actor:muse hello"],
    [970, 960, "/usr/local/bin/node", "node /Users/me/xenonet/cli/xenonet.js actor send actor:muse hello"],
    [XENONET, 1, "/Applications/XenoNet.app/Contents/MacOS/XenoNet"],
    [610, XENONET, "/Applications/XenoNet.app/Contents/Frameworks/XenoNet Helper.app/Contents/MacOS/XenoNet Helper"],
    [620, XENONET, "/bin/zsh"],
  ];
  const table = new Map(rows.map(([pid, ppid, program, args = "", cwd = null]) => [pid, { pid, ppid, start: 1000 + pid, program, args, cwd }]));
  return { table, signed: new Set([200, 500, 510, 900]) };
}

function setup({ home = XENONET, clock = { t: 1, step: 1 }, machine: make = machine, ...opts } = {}) {
  const m = make();
  const store = makeStore(path.join(tmp(), "engine.sqlite"));
  const calls = { chain: 0 };
  const verify = createVerifier({ procs: fakeProcs(m, { ...opts, calls }), store, home, now: () => (clock.t += clock.step) });
  return { m, store, verify, calls, clock, from: (pid) => ({ pid }) };
}

test("Claude Code's own hook pins its session for sure, and its MCP server and shell may act as it", async () => {
  const { verify, store, from } = setup();
  const hook = await verify.check(from(200), "claude:A", { hook: true });
  assert.equal(hook.ok, true);
  assert.equal(hook.sure, true);
  assert.deepEqual(store.pins("claude:A").map((p) => [p.pid, p.sure]), [[200, 1]]);
  assert.equal((await verify.check(from(210), "claude:A")).ok, true, "its MCP server");
  assert.equal((await verify.check(from(220), "claude:A")).ok, true, "its shell");
});

test("another session can't act in a pinned session's name", async () => {
  const { verify, from } = setup();
  await verify.check(from(200), "claude:A", { hook: true });
  const r = await verify.check(from(300), "claude:A");
  assert.equal(r.ok, false);
  assert.match(r.reason, /belongs to another Claude Code process/);
  assert.equal(verify.state.refused, 1);
});

test("requests in an agent's name must come from that kind of agent", async () => {
  const { verify, from } = setup();
  const plain = await verify.check(from(400), "claude:A");
  assert.equal(plain.ok, false, "a plain terminal isn't Claude Code");
  assert.match(plain.reason, /have to come from a Claude Code process/);
  const codex = await verify.check(from(220), "codex:T");
  assert.equal(codex.ok, false, "a Claude Code shell isn't Codex");
  assert.match(codex.reason, /have to come from a Codex process/);
});

test("the first process to name an unpinned session holds it until Claude Code's hook says otherwise", async () => {
  const { verify, store, from } = setup();
  // B names A before A's own hook arrives: a guess.
  assert.equal((await verify.check(from(300), "claude:A")).ok, true);
  assert.deepEqual(store.pins("claude:A").map((p) => [p.pid, p.sure]), [[300, 0]]);
  // A's own hook is the word: the guess goes.
  assert.equal((await verify.check(from(200), "claude:A", { hook: true })).sure, true);
  assert.deepEqual(store.pins("claude:A").map((p) => [p.pid, p.sure]), [[200, 1]]);
  assert.equal((await verify.check(from(300), "claude:A")).ok, false);
});

test("a hook from a Claude Code process that isn't signed pins nothing for sure", async () => {
  const { m, verify, store, from } = setup();
  await verify.check(from(200), "claude:A", { hook: true });
  // A program named claude, but not Anthropic's, under A's shell.
  m.table.set(700, { pid: 700, ppid: 220, start: 1700, program: "/tmp/somewhere/claude" });
  const r = await verify.check(from(700), "claude:A", { hook: true });
  assert.equal(r.ok, false, "it's another process than the one holding A");
  assert.deepEqual(store.pins("claude:A").map((p) => [p.pid, p.sure]), [[200, 1]]);
});

test("a session whose process ended can be taken up, and a reused pid isn't the same process", async () => {
  const { m, verify, store, from } = setup();
  await verify.check(from(200), "claude:A", { hook: true });
  // A's process ends, and its pid is reused by a new Claude Code process.
  m.table.set(200, { ...m.table.get(200), start: 9999 });
  assert.equal((await verify.check(from(200), "claude:A", { hook: true })).ok, true);
  assert.deepEqual(store.pins("claude:A").map((p) => [p.pid, p.start]), [[200, 9999]], "the old pin went with the old process");
  // It ends for good: B may resume the session.
  m.table.delete(200);
  m.table.delete(210);
  m.table.delete(220);
  assert.equal((await verify.check(from(300), "claude:A", { hook: true })).ok, true);
  assert.deepEqual(store.pins("claude:A").map((p) => p.pid), [300]);
});

test("where XenoNet can't look, requests go through unchecked", async () => {
  const { verify, from } = setup({ supported: false });
  assert.deepEqual(await verify.check(from(400), "claude:A"), { ok: true, checked: false });
  assert.deepEqual(await verify.person(from(220)), { ok: true, checked: false });
  assert.equal(verify.state.unchecked, 2);
});

test("a request in the name of a session whose app XenoNet doesn't check is refused, never passed unchecked", async () => {
  const gemini = "requests in gemini-cli:G's name are refused: XenoNet can't tell Gemini CLI's processes apart, so it can't check that the request comes from that session";
  for (const supported of [true, false]) {
    const { verify, from } = setup({ supported });
    // From anywhere, Claude Code's own process too, and as a hook.
    for (const pid of [200, 220, 400]) assert.deepEqual(await verify.check(from(pid), "gemini-cli:G"), { ok: false, checked: true, reason: gemini }, `from ${pid}`);
    assert.equal((await verify.check(from(220), "gemini-cli:G", { hook: true })).reason, gemini);
    assert.deepEqual(await verify.check({ pid: null }, "gemini-cli:G"), { ok: false, checked: true, reason: gemini }, "with no process to look at");
    assert.match((await verify.check(from(220), "copilot:C")).reason, /XenoNet can't tell GitHub Copilot CLI's processes apart/);
    // A name that isn't an app's at all, a property every object has
    // included.
    assert.equal((await verify.check(from(220), "nobody:N")).reason, 'requests in nobody:N\'s name are refused: XenoNet doesn\'t know an app called "nobody"');
    assert.equal((await verify.check(from(220), "toString:N")).reason, 'requests in toString:N\'s name are refused: XenoNet doesn\'t know an app called "toString"');
    assert.deepEqual([verify.state.unchecked, verify.state.refused, verify.state.checked], [0, 8, 0], "each counted as refused, none as checked or unchecked");
    assert.deepEqual(verify.state.lastRefusal.agent, "toString:N");
    // The apps XenoNet checks are as before: unchecked where it can't look,
    // and where it can, an OpenCode session from any OpenCode process.
    assert.equal((await verify.check(from(220), "claude:A")).ok, true);
    assert.deepEqual(await verify.check(from(300), "claude:A"), supported ? { ok: false, checked: true, reason: "claude:A belongs to another Claude Code process" } : { ok: true, checked: false });
    assert.deepEqual(await verify.check({ pid: null }, "codex:T"), { ok: true, checked: false }, "netstat had no line: taken at its word, as before");
  }
});

test("connecting agents is for people: refused under an agent, fine in a terminal or XenoNet's window", async () => {
  const { verify, from } = setup();
  const agent = await verify.person(from(220));
  assert.equal(agent.ok, false);
  assert.match(agent.reason, /came from under a Claude Code process/);
  assert.equal((await verify.person(from(400))).ok, true, "your own terminal");
  assert.equal((await verify.person(from(610))).ok, true, "XenoNet's own window");
  assert.equal((await verify.person(from(630))).ok, false, "claude run in XenoNet's terminal is still an agent");
});

test("the window's own routes: only from XenoNet's own app, with no agent nearer, never from its terminal", async () => {
  const { verify, from } = setup();
  assert.deepEqual(await verify.window(from(610)), { ok: true, checked: true }, "the page, through XenoNet's helper");
  assert.deepEqual(await verify.window(from(XENONET)), { ok: true, checked: true }, "XenoNet itself");
  for (const [pid, why, what] of [
    [620, /a program run in XenoNet/, "a shell in XenoNet's terminal"],
    [640, /a program run in XenoNet/, "XenoNet's own binary run as Node in its terminal (the xenonet command)"],
    [630, /under a Claude Code process/, "Claude Code in XenoNet's terminal"],
    [220, /under a Claude Code process/, "a Claude Code session's shell"],
    [400, /outside XenoNet/, "a plain terminal"],
    [100, /outside XenoNet/, "another app"],
  ]) {
    const r = await verify.window(from(pid));
    assert.equal(r.ok, false, what);
    assert.match(r.reason, why, what);
  }
});

test("a verdict on the window is kept a minute for its process", async () => {
  const { verify, calls, clock, from } = setup({ clock: { t: 0, step: 0 } });
  await verify.window(from(610));
  await verify.window(from(610));
  assert.equal(calls.chain, 1, "the table is read once");
  clock.t += 61e3;
  await verify.window(from(610));
  assert.equal(calls.chain, 2, "and again a minute later");
  assert.equal((await verify.window(from(620))).ok, false, "another process has its own");
});

test("without a window of its own (a server the desktop app didn't start), nothing passes as the window", async () => {
  const { verify, from } = setup({ home: null });
  const r = await verify.window(from(610));
  assert.equal(r.ok, false);
  assert.match(r.reason, /outside XenoNet/);
  assert.deepEqual(await setup({ home: null, supported: false }).verify.window(from(610)), { ok: true, checked: false }, "where XenoNet can't look, the key alone decides");
});

test("the bundle a program lies in", () => {
  assert.equal(bundleOf("/Applications/XenoNet.app/Contents/Frameworks/XenoNet Helper.app/Contents/MacOS/XenoNet Helper"), "/Applications/XenoNet.app/");
  assert.equal(bundleOf("/Users/me/xenonet/node_modules/electron/dist/Electron.app/Contents/MacOS/Electron"), "/Users/me/xenonet/node_modules/electron/dist/Electron.app/");
  assert.equal(bundleOf("/usr/local/bin/node"), "/usr/local/bin/");
});

// ---------- actors (interface section 5) ----------

const actors = (opts = {}) => setup({ machine: actorMachine, ...opts });
const WREN = { slug: "wren", product: null };
const HERMES = { slug: "hermes", product: "hermes-agent" };

test("an actor's token never works from under an agent, nor from an actor app's own process", async () => {
  const { verify, from } = actors();
  const agent = await verify.actor(from(220), WREN);
  assert.equal(agent.ok, false);
  assert.match(agent.reason, /an actor's token doesn't work from under a session, and this came from under a Claude Code process/);
  assert.equal((await verify.actor(from(540), WREN)).ok, false, "a claude a Bot started is an agent too");
  for (const [pid, who, why] of [
    [510, WREN, /Grok Bot's own processes can't use an actor's token: they could be relaying for its cloud/],
    [500, WREN, /Grok Bot's own processes/],
    [810, HERMES, /Hermes Agent's own processes/],
  ]) {
    const r = await verify.actor(from(pid), who);
    assert.equal(r.ok, false, String(pid));
    assert.match(r.reason, why);
  }
});

test("a product's actor only from under that product; an actor of your own never from under one", async () => {
  const { verify, from } = actors();
  assert.equal((await verify.actor(from(730), HERMES)).ok, true, "a command Hermes's terminal tool runs");
  assert.equal((await verify.actor(from(740), HERMES)).ok, true, "the xenonet MCP server Hermes started");
  for (const pid of [430, 530, 700, 620]) {
    const r = await verify.actor(from(pid), HERMES);
    assert.equal(r.ok, false, `${pid} isn't under a Hermes agent host`);
    assert.match(r.reason, /Hermes Agent's actor works only from under Hermes Agent's own processes/);
  }
  const ok = await verify.actor(from(430), WREN);
  assert.deepEqual([ok.ok, ok.checked, ok.root, ok.chain.map((p) => p.pid)], [true, true, { pid: 430, start: 1430 }, [430, 420, 410, 400]], "from your own terminal, with where it came from");
  const grok = await verify.actor(from(530), WREN);
  assert.match(grok.reason, /an actor of your own can't use its token from under Grok Bot/);
  assert.match((await verify.actor(from(730), WREN)).reason, /from under Hermes Agent/);
  assert.equal((await verify.actor(from(530), { slug: "grok-bot", product: "grok-bot" })).ok, true, "Grok Bot's own actor, from a command its app runs");
  assert.equal((await verify.actor(from(920), { slug: "muse", product: "muse" })).ok, true, "Muse's, from a program it runs");
  assert.match((await verify.actor(from(900), { slug: "muse", product: "muse" })).reason, /Muse's own processes can't use an actor's token/, "never from Muse's app itself");
  assert.match((await verify.actor(from(530), { slug: "muse", product: "muse" })).reason, /Muse's actor works only from under Muse's own processes/, "nor from under Grok Bot");
  assert.match((await verify.actor(from(920), { slug: "grok-bot", product: "grok-bot" })).reason, /Grok Bot's actor works only from under Grok Bot's own processes/, "nor Grok Bot's from under Muse");
  assert.match((await verify.actor(from(430), { slug: "muse", product: "muse" })).reason, /only from under Muse's own processes/, "nor from your terminal");
  const fake = await verify.actor(from(970), { slug: "grok-bot", product: "grok-bot" });
  assert.match(fake.reason, /Grok Bot's actor works only from under Grok Bot's app as Cursor signed it/, "nor from under a bundle that only calls itself Grok Bot.app");
  assert.match((await verify.register(from(970), { product: "grok-bot" })).reason, /registers only from under Grok Bot's app as Cursor signed it/);
  assert.equal((await verify.register(from(920), { product: "muse" })).ok, true, "Muse's registers from a program it runs");
  assert.equal((await verify.actor(from(430), { slug: "x", product: "something-else" })).ok, true, "a product XenoNet doesn't know is the actor's own word, held as yours are");
});

test("the pid an actor names is the caller or one of its parents", async () => {
  const { verify, from } = actors();
  assert.equal((await verify.actor(from(430), WREN, { pid: 430 })).ok, true, "itself");
  assert.equal((await verify.actor(from(430), WREN, { pid: 420 })).ok, true, "the daemon it runs under");
  for (const pid of [999, 510, 220]) {
    const r = await verify.actor(from(430), WREN, { pid });
    assert.equal(r.ok, false, String(pid));
    assert.match(r.reason, /the pid it names has to be the calling process or one of its parents/);
  }
  // Terminal is one of its parents, but other windows' sessions run under
  // it too: never an actor's own.
  assert.equal((await verify.actor(from(430), WREN, { pid: 410 })).ok, true, "the shell it runs in");
  const app = await verify.actor(from(430), WREN, { pid: 400 });
  assert.equal(app.ok, false);
  assert.match(app.reason, /the pid it names is an app's, which other programs run under too/);
  assert.match((await verify.register(from(430), { pid: 400 })).reason, /an app's/, "registering names a pid by the same rule");
  assert.equal((await verify.actor(from(530), { slug: "grok-bot", product: "grok-bot" }, { pid: 500 })).ok, false, "nor Grok Bot's own app, for a command it runs");
});

test("ownOf: the caller and its parents below the first app; a product's host inside its app is the product's", () => {
  const p = (pid, program, product = null) => ({ pid, start: pid, program, agent: null, product });
  const terminal = p(400, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal");
  assert.deepEqual(ownOf([p(430, "/usr/local/bin/node"), p(420, "/Users/me/bin/wren"), p(410, "/bin/zsh"), terminal]).map((x) => x.pid), [430, 420, 410]);
  assert.deepEqual(ownOf([p(430, "/usr/local/bin/node"), p(420, "/Users/me/bin/wren")]).map((x) => x.pid), [430, 420], "no app above: the whole chain");
  assert.deepEqual(ownOf([p(900, "/Applications/Wren.app/Contents/MacOS/Wren"), terminal]).map((x) => x.pid), [900], "a caller that's an app's process is still its own");
  const host = p(710, `${HERMES_APP}/Resources/python/bin/python3`, { product: "hermes-agent", role: "host" });
  const hermesApp = p(800, `${HERMES_APP}/MacOS/Hermes`, { product: "hermes-agent", role: "app" });
  assert.deepEqual(ownOf([p(740, "/usr/local/bin/node"), host, hermesApp]).map((x) => x.pid), [740, 710], "Hermes's agent host inside Hermes.app, not the app");
  assert.equal(actorRefusal([p(740, "/usr/local/bin/node"), host, hermesApp], { product: "hermes-agent", pid: 710 }), null, "so Hermes's actor may name its host");
  assert.deepEqual(ownOf([]), []);
});

test("a verdict on an actor is kept a minute per actor, process and pid, and where XenoNet can't look the token decides", async () => {
  const { verify, calls, clock, from } = actors({ clock: { t: 0, step: 0 } });
  await verify.actor(from(430), WREN);
  await verify.actor(from(430), WREN);
  assert.equal(calls.chain, 1, "the table is read once");
  await verify.actor(from(430), WREN, { pid: 420 });
  await verify.actor(from(430), HERMES);
  assert.equal(calls.chain, 3, "another pid, another actor: their own");
  clock.t += 61e3;
  await verify.actor(from(430), WREN);
  assert.equal(calls.chain, 4, "and again a minute later");
  const blind = actors({ supported: false });
  assert.deepEqual(await blind.verify.actor(blind.from(220), WREN), { ok: true, checked: false });
  assert.deepEqual(await blind.verify.register(blind.from(220), {}), { ok: true, checked: false });
  assert.equal(await blind.verify.peer(blind.from(220)), null);
  assert.equal(await blind.verify.lineage(220), null);
});

test("registering: refused from under an agent, an actor app or a product it isn't; the chain and folder of who asked otherwise", async () => {
  const { verify, from } = actors();
  assert.match((await verify.register(from(220), {})).reason, /a session can't register an actor, only ask you to, and this came from under a Claude Code process/);
  assert.match((await verify.register(from(510), {})).reason, /Grok Bot's own processes can't register an actor/);
  assert.match((await verify.register(from(730), {})).reason, /a program under Hermes Agent can't register an actor of its own/);
  assert.match((await verify.register(from(430), { product: "hermes-agent" })).reason, /Hermes Agent's actor registers only from under Hermes Agent's own processes/);
  assert.match((await verify.register(from(430), { pid: 999 })).reason, /pid/);
  const ok = await verify.register(from(430), { pid: 420 });
  assert.deepEqual([ok.ok, ok.checked, ok.chain.map((p) => p.pid), ok.cwd], [true, true, [430, 420, 410, 400], "/Users/me/Projects/docs-site"]);
  const hermes = await verify.register(from(730), { product: "hermes-agent" });
  assert.deepEqual([hermes.ok, hermes.cwd], [true, null]);
});

test("your own routes are refused from under an actor product, as from under an agent", async () => {
  const { verify, from } = actors();
  assert.match((await verify.person(from(530))).reason, /it came from under Grok Bot; run it in your own terminal or in XenoNet/);
  assert.match((await verify.person(from(730))).reason, /it came from under Hermes Agent/);
  assert.match((await verify.person(from(510))).reason, /under Grok Bot/);
  assert.equal((await verify.person(from(430))).ok, true, "your own terminal, and a program of yours in it");
  assert.equal((await verify.person(from(610))).ok, true, "XenoNet's own window");
  assert.deepEqual(await verify.lineage(730), [730, 720, 710, 700, 410, 400]);
  assert.deepEqual(await verify.lineage(4242), [], "a process that's gone");
  assert.equal(await verify.peer(from(430)), 430);
});

test("where XenoNet can look, a process it can't see is refused, never passed: tokens, your routes, registering and the window", async () => {
  const { m, verify, from } = actors();
  // Gone before the table was read (it sent and exited), and one netstat
  // had no line for: on a made-up table, a pid it doesn't hold and none.
  const gone = from(4999);
  const blind = { pid: null };
  for (const socket of [gone, blind]) {
    const r = await verify.actor(socket, WREN);
    assert.deepEqual([r.ok, r.unseen], [false, true]);
    assert.match(r.reason, /XenoNet couldn't see which program sent this, so it doesn't take the token from it/);
    assert.match((await verify.person(socket)).reason, /this is yours to do, and XenoNet couldn't see which program asked/);
    assert.match((await verify.register(socket, {})).reason, /XenoNet couldn't see which program asked to register/);
    assert.match((await verify.window(socket)).reason, /this is for XenoNet's own window, and XenoNet couldn't see which program asked/);
  }
  assert.deepEqual([verify.state.unseen, verify.state.unchecked], [8, 0], "counted, and none went through unchecked");
  // An agent's request in its own session's name: refused from a process
  // that's gone, and taken at its word only when netstat has no line at all.
  assert.match((await verify.check(gone, "claude:A")).reason, /had gone before XenoNet could look/);
  assert.deepEqual(await verify.check(blind, "claude:A"), { ok: true, checked: false });
  // Nothing of it is kept: once the process is there, it's looked at.
  m.table.set(4999, { pid: 4999, ppid: 410, start: 5999, program: "/usr/bin/curl", args: "", cwd: null });
  assert.deepEqual([(await verify.actor(gone, WREN)).ok, (await verify.window(gone)).reason], [true, "this is for XenoNet's own window, and it came from outside XenoNet"]);
});

test("actorRefusal: the rules on a chain alone", () => {
  const p = (pid, extra = {}) => ({ pid, start: pid, program: "/x", agent: null, product: null, ...extra });
  assert.equal(actorRefusal([p(1), p(2)]), null);
  assert.match(actorRefusal([p(1), p(2, { agent: "codex" })]), /Codex process/);
  assert.match(actorRefusal([p(1, { product: { product: "muse", role: "app" } })]), /Muse's own processes/);
  assert.match(actorRefusal([p(1), p(2, { product: { product: "dots", role: "app" } })]), /the ChatGPT app/, "under the ChatGPT app, an actor of your own is refused");
  assert.equal(actorRefusal([p(1), p(2, { product: { product: "grok-bot", role: "app" } })], { product: "grok-bot" }), null);
  assert.match(actorRefusal([p(1), p(2, { product: { product: "hermes-agent", role: "tui" } })], { product: "hermes-agent" }), /only from under/, "the terminal UI isn't the host");
  assert.match(actorRefusal([p(1)], { pid: 3 }), /pid/);
});

// The real lookups (core/procs.js) on a made-up machine (test/harness),
// where a process's executable can differ from the argv[0] ps shows.
function onMachine(home) {
  const m = fakeProcesses();
  const store = makeStore(path.join(tmp(), "engine.sqlite"));
  const verify = createVerifier({ procs: createProcs({ platform: "darwin", exec: m.exec }), store, home: typeof home === "function" ? home(m) : home, now: () => 1 });
  let port = 50000;
  // A connection from `pid` to XenoNet, as netstat lists it.
  const from = (pid) => {
    port++;
    m.connect({ pid, clientPort: port, serverPort: 47840 });
    return { remotePort: port, localPort: 47840 };
  };
  return { m, verify, from };
}

test("the window's check goes by the executable's real path: a program can't pass for XenoNet's by naming itself", async () => {
  let xenonet = null;
  const { m, verify, from } = onMachine((m) => (xenonet = m.add({ ppid: 1, program: "/Applications/XenoNet.app/Contents/MacOS/XenoNet" })));
  const HELPER = "/Applications/XenoNet.app/Contents/Frameworks/XenoNet Helper.app/Contents/MacOS/XenoNet Helper";
  const helper = m.add({ ppid: xenonet, program: HELPER });
  // A shell in XenoNet's terminal that ran exec -a with the helper's path, and
  // one whose executable lsof couldn't read.
  const renamed = m.add({ ppid: xenonet, program: HELPER, exe: "/Users/me/bin/wren" });
  const unread = m.add({ ppid: xenonet, program: HELPER, exe: null });
  assert.deepEqual(await verify.window(from(helper)), { ok: true, checked: true });
  for (const pid of [renamed, unread]) assert.match((await verify.window(from(pid))).reason, /it came from a program run in XenoNet/, String(pid));
});

test("a service no app started takes the Desktop connected to it as XenoNet's app, and only an app's own executable", async () => {
  const { m, verify, from } = onMachine(null);
  const xenonet = m.add({ ppid: 1, program: "/Applications/XenoNet.app/Contents/MacOS/XenoNet" });
  // The page's requests reach the service through the Desktop, from its
  // own process.
  assert.match((await verify.window(from(xenonet))).reason, /outside XenoNet/, "before the Desktop connects");
  assert.deepEqual(await verify.desktop(from(xenonet)), { ok: true, checked: true });
  assert.deepEqual(await verify.window(from(xenonet)), { ok: true, checked: true }, "the page, through the Desktop");
  assert.equal((await verify.person(from(xenonet))).ok, true);
  await verify.desktop(null);
  assert.match((await verify.window(from(xenonet))).reason, /outside XenoNet/, "once the Desktop goes");

  // What holds the service's token can say it's the Desktop, but only an
  // app's own executable, by its real path, becomes XenoNet's app.
  const shell = m.add({ ppid: 1, program: "/bin/zsh" });
  const claude = m.add({ ppid: shell, program: "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude" });
  for (const [pid, why, what] of [
    [m.add({ ppid: claude, program: "/usr/local/bin/node" }), /an app's own executable/, "a script an agent runs"],
    [m.add({ ppid: shell, program: "/Users/me/.local/bin/xenonet" }), /an app's own executable/, "the xenonet command"],
    [m.add({ ppid: shell, program: "/Applications/XenoNet.app/Contents/MacOS/XenoNet", exe: "/Users/me/bin/wren" }), /an app's own executable/, "a program naming itself XenoNet"],
    [claude, /a Claude Code process/, "Claude Code itself"],
  ]) {
    const r = await verify.desktop(from(pid));
    assert.equal(r.ok, false, what);
    assert.match(r.reason, why, what);
    assert.match((await verify.window(from(pid))).reason, /outside XenoNet|under a Claude Code process/, `${what} isn't the window`);
  }

  // A Desktop that goes while it's being looked at never becomes home.
  const looking = verify.desktop(from(xenonet));
  await verify.desktop(null);
  assert.equal((await looking).ok, true);
  assert.match((await verify.window(from(xenonet))).reason, /outside XenoNet/);
});

test("the tray is XenoNet Tray.app's own executable in an Applications folder, never under an agent, never unchecked", async () => {
  const { m, verify, from } = onMachine(null);
  const apps = ["/Applications", "/Users/me/Applications/"];
  const TRAY = "/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray";
  const tray = m.add({ ppid: 1, program: TRAY });
  const mine = m.add({ ppid: 1, program: "/Users/me/Applications/XenoNet Tray.app/Contents/MacOS/XenoNet Tray" });
  assert.deepEqual(await verify.tray(from(tray), apps), { ok: true, checked: true });
  assert.deepEqual(await verify.tray(from(mine), apps), { ok: true, checked: true }, "the home folder's Applications");

  const elsewhere = /only XenoNet Tray\.app's own executable, in \/Applications or your own Applications folder/;
  const shell = m.add({ ppid: 1, program: "/bin/zsh" });
  const claude = m.add({ ppid: shell, program: "/Users/me/Library/Application Support/Claude/claude.app/Contents/MacOS/claude" });
  for (const [pid, why, what] of [
    [m.add({ ppid: 1, program: "/Users/me/src/xenonet/tray/macos/.build/XenoNet Tray.app/Contents/MacOS/XenoNet Tray" }), elsewhere, "where it was built"],
    [m.add({ ppid: 1, program: "/Applications/XenoNet Tray.app/Contents/MacOS/tools/helper" }), elsewhere, "a helper inside the bundle"],
    [m.add({ ppid: 1, program: "/Applications/XenoNet.app/Contents/MacOS/XenoNet" }), elsewhere, "Desktop"],
    [m.add({ ppid: shell, program: TRAY, exe: "/Users/me/bin/wren" }), elsewhere, "a program naming itself the tray"],
    [m.add({ ppid: shell, program: TRAY, exe: null }), elsewhere, "one whose executable couldn't be read"],
    [m.add({ ppid: claude, program: TRAY }), /can't run under a Claude Code process/, "the tray, started by an agent"],
  ]) {
    const r = await verify.tray(from(pid), apps);
    assert.equal(r.ok, false, what);
    assert.match(r.reason, why, what);
  }
  assert.match((await verify.tray(from(tray), [])).reason, elsewhere, "no Applications folder named");

  // Where XenoNet can't look, nothing passes.
  const blind = createVerifier({ procs: { supported: false }, store: makeStore(path.join(tmp(), "engine.sqlite")), home: null, now: () => 1 });
  assert.deepEqual(await blind.tray({ remotePort: 1, localPort: 2 }, apps), { ok: false, checked: true, reason: "XenoNet can't see which program connects as its tray on this system" });
});

test("Hermes's actor works only from under a Hermes agent host by its real path, not one that names itself as one", async () => {
  const { m, verify, from } = onMachine(null);
  const HOST = "/Users/me/.hermes/tools/python-3.14.7/bin/python3.14";
  const terminal = m.add({ ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" });
  const shell = m.add({ ppid: terminal, program: "/bin/zsh" });
  const posing = m.add({ ppid: shell, program: HOST, exe: "/usr/bin/python3", args: ["-c", "import hermes_cli"] });
  const host = m.add({ ppid: shell, program: HOST, args: ["-m", "tui_gateway.entry"] });
  const cli = (parent) => m.add({ ppid: parent, program: "/usr/local/bin/node", args: ["/Users/me/xenonet/cli/xenonet.js", "actor", "heartbeat"] });
  const HERMES = { slug: "hermes", product: "hermes-agent" };
  assert.match((await verify.actor(from(cli(posing)), HERMES)).reason, /Hermes Agent's actor works only from under Hermes Agent's own processes/);
  assert.match((await verify.register(from(cli(posing)), { product: "hermes-agent" })).reason, /registers only from under Hermes Agent's own processes/);
  assert.equal((await verify.actor(from(cli(host)), HERMES)).ok, true);
  const asked = await verify.register(from(cli(host)), { product: "hermes-agent" });
  assert.deepEqual(asked.chain.map((p) => p.exe).slice(0, 2), ["/usr/local/bin/node", HOST], "the chain a card is drawn from has the real paths");
});
