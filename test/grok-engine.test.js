// Grok Build and the engine: its hook envelope read in the engine's terms
// (core/engine/grok-hooks.js), what the engine answers each event given
// what Grok does with an answer, the route its hooks post to, the session a
// grok process holds for an MCP server that can't say (verify.holding), and
// a request in a grok:<id>'s name checked against its process.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Engine, isAgentId, editPaths } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { grokHookInput, fromGrok } from "../core/engine/grok-hooks.js";
import { createVerifier } from "../core/engine/verify.js";
import { engineRoutes } from "../server/routes/engine.js";
import { agentOf } from "../core/procs.js";

const made = [];
const stores = [];
after(() => {
  for (const s of stores) if (s.db.isOpen) s.close();
  for (const dir of made) fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 });
});
const store = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-grok-engine-"));
  made.push(dir);
  const s = new EngineStore(path.join(dir, "engine.sqlite"));
  stores.push(s);
  return s;
};

const G = "grok:019a3b5c-0000-7000-8000-00000000000a";
const RAW = G.slice("grok:".length);
const A = "claude:aaaaaaaa-0001";

// Grok's PreToolUse, as its guide shows it.
const PRE = { hookEventName: "pre_tool_use", hook_event_name: "PreToolUse", sessionId: RAW, cwd: "/w/atlas", workspaceRoot: "/w/atlas", permissionMode: "default", toolName: "search_replace", toolInput: { file_path: "/w/atlas/src/refunds.ts" }, toolUseId: "t1", toolInputTruncated: false, timestamp: "2026-04-14T12:00:00Z" };

test("Grok's envelope in the engine's terms: its PascalCase event, its camelCase fields, and the hook's environment where it says nothing", () => {
  assert.deepEqual(grokHookInput(PRE), { event: "PreToolUse", session_id: RAW, cwd: "/w/atlas", tool_name: "search_replace", tool_input: { file_path: "/w/atlas/src/refunds.ts" }, stop_hook_active: false, reason: null, subagent: null, notification: null, message: null });
  const stop = grokHookInput({ hookEventName: "stop", sessionId: RAW, stopHookActive: true, reason: "end_turn", lastAssistantMessage: "Done." });
  assert.deepEqual([stop.event, stop.stop_hook_active, stop.reason], ["Stop", true, "end_turn"]);
  assert.equal(grokHookInput({ hookEventName: "post_tool_use_failure" }).event, "PostToolUseFailure");
  assert.deepEqual(grokHookInput({ notificationType: "permission_prompt", message: "Allow npm install?", subagentType: "explore" }, { GROK_SESSION_ID: RAW, GROK_HOOK_EVENT: "notification" }), { event: "Notification", session_id: RAW, cwd: null, tool_name: null, tool_input: null, stop_hook_active: false, reason: null, subagent: "explore", notification: "permission_prompt", message: "Allow npm install?" });
  assert.deepEqual(grokHookInput(null), { event: null, session_id: null, cwd: null, tool_name: null, tool_input: null, stop_hook_active: false, reason: null, subagent: null, notification: null, message: null });
  // Grok's events, told from Claude Code's: Grok runs Claude Code's hooks too.
  assert.equal(fromGrok(PRE), true);
  assert.equal(fromGrok({ session_id: "11111111-2222-3333-4444-555555555555", hook_event_name: "PreToolUse", tool_name: "Edit" }), false);
  assert.equal(fromGrok({}, { GROK_HOOK_EVENT: "session_start" }), true);
  assert.ok(isAgentId(G));
  assert.deepEqual(editPaths("search_replace", { target_file: "src/a.ts" }, "/w/atlas"), ["/w/atlas/src/a.ts"]);
});

// A stand-in for XenoNet's state, with a Claude Code session that just
// edited refunds.ts in atlas, and Grok's session beside it.
function world() {
  const t = { now: Date.now() };
  const live = (edits = []) => ({ lastEventAt: t.now - 10e3, edits: new Map(edits.map(([p, ts]) => [p.toLowerCase(), [{ path: p, ts, kind: "edit" }]])), status: () => "tool", needsYou: () => null });
  const sessions = { [A]: { title: "Add partial refunds", cwd: "/w/atlas", live: live([["/w/atlas/src/refunds.ts", t.now - 60e3]]) }, [G]: { title: "Rate-limit the API", cwd: "/w/atlas", live: live() } };
  const state = {
    index: new Map(Object.entries(sessions).map(([id, s]) => [id, { id, kind: "session", title: s.title, cwd: s.cwd, updatedAt: t.now - 10e3 }])),
    live: { sessions: new Map(Object.entries(sessions).map(([id, s]) => [id, s.live])) },
    homeOf: () => null,
  };
  const s = store();
  const repos = { of: async (cwd) => (cwd?.startsWith("/w/atlas") ? { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" } : null) };
  return { engine: new Engine({ state, store: s, repos, now: () => t.now }), store: s, t };
}

test("Grok's hooks: nothing at the start or with a prompt, which Grok wouldn't read; a warning after an edit; who's here and mail with the next tool result; mail at a turn's end", async () => {
  const { engine, store: s, t } = world();
  const hook = (event, extra = {}) => engine.hook("grok", event, grokHookInput({ hook_event_name: event, sessionId: RAW, cwd: "/w/atlas", ...extra }));
  assert.equal(await hook("SessionStart"), null, "Grok ignores what SessionStart says");
  await engine.send(A, { to: G, text: "refunds.ts is mid-refactor." });
  assert.equal(await hook("UserPromptSubmit", { prompt: "go" }), null, "Grok throws away what UserPromptSubmit says");
  assert.equal(s.unseen(G).length, 1, "so the mail waits");
  // About to edit the file the Claude session just did: told, with the result.
  const warn = await hook("PreToolUse", { toolName: "search_replace", toolInput: { file_path: "/w/atlas/src/refunds.ts" } });
  assert.equal(warn.hookSpecificOutput.hookEventName, "PreToolUse");
  assert.match(warn.hookSpecificOutput.additionalContext, /"Add partial refunds" \(claude, .*edited refunds\.ts/);
  // The first tool result brings who's here and the mail, once.
  const first = (await hook("PostToolUse", { toolName: "read_file" })).hookSpecificOutput;
  assert.equal(first.hookEventName, "PostToolUse");
  assert.match(first.additionalContext, /another agent has been working in this repo/);
  assert.match(first.additionalContext, /You are grok:019a3b5c/);
  assert.match(first.additionalContext, /refunds\.ts is mid-refactor/);
  assert.equal(await hook("PostToolUse"), null, "nothing more to say");
  // Mail at a turn's end keeps it going; the end of a session's Stop, and a
  // Stop already kept going, don't.
  await engine.send(A, { to: G, text: "One more thing." });
  assert.equal(await hook("Stop", { reason: "channel_closed" }), null);
  assert.equal(await hook("Stop", { stopHookActive: true, reason: "end_turn" }), null);
  const stop = await hook("Stop", { reason: "end_turn" });
  assert.equal(stop.decision, "block");
  assert.match(stop.reason, /One more thing/);
  assert.equal(await hook("PostToolUseFailure"), null);
  // A subagent's own session is its parent's to speak for: no mail taken.
  await engine.send(A, { to: G, text: "For the parent." });
  assert.equal(await hook("PostToolUse", { subagentType: "explore" }), null);
  assert.equal(await hook("Stop", { subagentType: "explore", reason: "end_turn" }), null);
  assert.equal(s.unseen(G).length, 1);
  assert.equal(await hook("PreToolUse", { subagentType: "explore", toolName: "search_replace", toolInput: { file_path: "/w/atlas/src/refunds.ts" } }), null);
  await hook("SessionEnd", { subagentType: "explore" });
  assert.equal(s.contact(G).ended_at, null, "a subagent's end isn't the session's");
  // The session's own next event gets what the subagent's passed over.
  assert.match((await hook("PostToolUse")).hookSpecificOutput.additionalContext, /For the parent/);
  assert.equal(s.unseen(G).length, 0);
  // In touch, so reachable; its SessionEnd takes it out.
  assert.equal(s.contact(G).app, "grok");
  assert.equal((await engine.view(G)).app, "grok");
  await hook("SessionEnd", { reason: "exit" });
  t.now += 10e3;
  assert.ok(s.contact(G).ended_at);
});

test("Grok's hooks reach the engine through their own route, checked against the grok that sends them, and the tailer hears them", async () => {
  const calls = [];
  const note = (name, value = null) => (...args) => (calls.push([name, ...args]), value);
  const live = { noteGrokHook: note("noteGrokHook") };
  const engine = { hook: note("hook", { hookSpecificOutput: { additionalContext: "hi" } }), touch: note("touch"), whoami: note("whoami", { you: G }), state: { live, refresh: async () => calls.push(["refresh"]) } };
  let held = G;
  const verify = { check: note("check", { ok: true, root: { pid: 7 } }), person: note("person", { ok: true }), holding: async () => held };
  const route = engineRoutes({ engine, connect: {}, verify, readJson: async (req) => req.body ?? {}, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (method, url, { body, headers = {} } = {}) => {
    const res = { writeHead: (status) => Object.assign(res, { status }) && { end: () => {} } };
    await route({ method, headers, body, socket: {} }, res, new URL(url, "http://127.0.0.1"));
    return res;
  };
  const res = await go("POST", "/api/hook/grok", { body: PRE });
  assert.deepEqual([res.status, res.body.hookSpecificOutput.additionalContext], [200, "hi"]);
  assert.equal(calls.find((c) => c[0] === "check")[2], G);
  assert.deepEqual(calls.find((c) => c[0] === "noteGrokHook").slice(1, 3), [G, "PreToolUse"]);
  const [, app, event, input] = calls.find((c) => c[0] === "hook");
  assert.deepEqual([app, event, input.tool_name, input.cwd], ["grok", "PreToolUse", "search_replace", "/w/atlas"]);
  await go("POST", "/api/hook/grok", { body: { hookEventName: "session_start", sessionId: RAW } });
  assert.ok(calls.some((c) => c[0] === "refresh"), "a new session: the map looks again");
  assert.equal((await go("POST", "/api/hook/grok", { body: { hookEventName: "stop" } })).status, 204, "no session, nothing to say");
  // An MCP server that can say only that a grok calls: the session that
  // grok holds, by its hooks.
  const who = await go("POST", "/api/engine/whoami", { headers: { "x-xenonet-agent": "grok" }, body: {} });
  assert.deepEqual([who.status, who.body], [200, { you: G }]);
  assert.equal(calls.filter((c) => c[0] === "check").at(-1)[2], G);
  held = null;
  const lost = await go("POST", "/api/engine/whoami", { headers: { "x-xenonet-agent": "grok" }, body: {} });
  assert.equal(lost.status, 400);
  assert.match(lost.body.error, /xenonet connect grok/);
});

test("a grok's session for its MCP server: the one its hooks pinned to that grok last; a request in a Grok session's name from elsewhere is refused", async () => {
  const table = new Map(
    [
      [1, 0, "/sbin/launchd"],
      [400, 1, "/bin/zsh"],
      [500, 400, "/Users/me/.grok/bin/grok"],
      [510, 500, "/bin/sh"], // a hook
      [520, 500, "/usr/local/bin/node"], // the xenonet MCP server
      [600, 400, "agent", "/Users/me/.grok/downloads/grok-macos-aarch64"], // grok, as its installer's agent link
      [610, 600, "/usr/local/bin/node"],
      [700, 400, "/Users/me/.local/bin/claude"],
    ].map(([pid, ppid, program, exe]) => [pid, { pid, ppid, start: 1000 + pid, program, exe }]),
  );
  const procs = {
    supported: true,
    peer: async (socket) => socket.pid,
    chain: async (pid) => {
      const out = [];
      for (let p = table.get(pid); p; p = p.ppid > 1 ? table.get(p.ppid) : null) out.push({ ...p, agent: agentOf(p.exe ?? p.program), product: null });
      return out;
    },
    alive: async ({ pid, start }) => table.get(pid)?.start === start,
    signed: async () => false,
  };
  const s = store();
  const verify = createVerifier({ procs, store: s, now: () => 5000 });
  assert.equal(await verify.holding({ pid: 520 }, "grok"), null, "no hook has said yet");
  // Its hooks, from under it, pin two sessions in turn (a /new).
  const other = "grok:019a3b5c-0000-7000-8000-00000000000b";
  assert.ok((await verify.check({ pid: 510 }, other, { hook: true })).ok);
  s.pin(G, { pid: 500, start: 1500 }, false, 9000);
  assert.equal(await verify.holding({ pid: 520 }, "grok"), G, "the one pinned last");
  assert.equal(await verify.holding({ pid: 610 }, "grok"), null, "another grok holds none");
  assert.equal(await verify.holding({ pid: 700 }, "grok"), null, "not from under a grok");
  // In its name from under Claude Code, or another grok: refused.
  assert.match((await verify.check({ pid: 700 }, G)).reason, /have to come from a Grok Build process/);
  assert.match((await verify.check({ pid: 610 }, G)).reason, /belongs to another Grok Build process/);
  assert.ok((await verify.check({ pid: 520 }, G)).ok);
});
