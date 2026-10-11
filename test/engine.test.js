// The engine (core/engine, mcp/xenonet.js, xenonet hook): agents, peers,
// the hooks' answers, connecting Claude Code and Codex, and the MCP server
// and hook bridge that agents run (as xenonet mcp and xenonet hook). XenoNet's state is stood in for, so these
// run without transcripts or a desktop app.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Engine, matches, editPaths, parseRepo } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { createConnect, mergeHooks } from "../core/engine/connect.js";
import * as hooks from "./fixtures/hook-payloads.js";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// Temp folders, taken away after the tests.
const made = [];
const stores = [];
const makeStore = (...args) => { const s = new EngineStore(...args); stores.push(s); return s; };
after(() => stores.forEach((s) => { if (s.db.isOpen) s.close(); }));
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync.native(os.tmpdir()), "xenonet-engine-"));
  made.push(dir);
  return dir;
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true, maxRetries: 10 })));

test("claims match files, folders and globs; edit tools name their files", () => {
  assert.ok(matches("/r/src/a.js", "/r/src/a.js"));
  assert.ok(matches("/r/src/", "/r/src/deep/b.js"));
  assert.ok(matches("/r/src/**", "/r/src/deep/b.js"));
  assert.ok(matches("/r/src/*.js", "/r/src/a.js"));
  assert.ok(!matches("/r/src/*.js", "/r/src/deep/a.js"));
  assert.ok(matches("/r/**/*.ts", "/r/a/b/c.ts"));
  assert.ok(!matches("/r/src/a.js", "/r/src/a.jsx"));
  assert.deepEqual(editPaths("Write", { file_path: "/r/a.md" }, "/r"), ["/r/a.md"]);
  assert.deepEqual(editPaths("NotebookEdit", { notebook_path: "n.ipynb" }, "/r"), ["/r/n.ipynb"]);
  const patch = "*** Begin Patch\n*** Update File: src/a.js\n@@\n-x\n+y\n*** Add File: /abs/b.js\n+z\n*** End Patch";
  assert.deepEqual(editPaths("apply_patch", { input: patch }, "/r"), ["/r/src/a.js", "/abs/b.js"]);
  assert.deepEqual(editPaths("Bash", { command: "ls" }, "/r"), []);
});

// A stand-in for XenoNet's state: sessions, their transcripts' live view, and
// where each lives. Repos are given, not read from git.
function world(now) {
  const t = { now };
  const live = (lastEventAt, edits = [], status = "tool") => ({
    lastEventAt,
    edits: new Map(edits.map(([p, ts]) => [p.toLowerCase(), [{ path: p, ts, kind: "edit" }]])),
    status: () => status,
    needsYou: () => null,
  });
  const sessions = {
    "claude:aaaaaaaa-0001": { title: "Add partial refunds", cwd: "/w/atlas", live: live(now - 30e3, [["/w/atlas/src/refunds.ts", now - 60e3]]) },
    "codex:bbbbbbbb-0002": { title: "Rate-limit the API", cwd: "/w/atlas", live: live(now - 20e3, [["/w/atlas/src/routes.ts", now - 120e3]]) },
    "claude:cccccccc-0003": { title: "Fix the ledger test", cwd: "/w/ledger", live: live(now - 10e3) },
    "codex:dddddddd-0004": { title: "Zoom on images", cwd: "/wt/atlas-zoom", branch: "feat/zoom", live: live(now - 40e3, [["/wt/atlas-zoom/src/refunds.ts", now - 300e3]]) },
    "claude:eeeeeeee-0005": { title: "Scratch in tmp", cwd: "/tmp/one", live: live(now - 50e3, [], "idle") },
    "claude:ffffffff-0006": { title: "Other scratch", cwd: "/tmp/two", live: live(now - 50e3, [], "idle") },
  };
  const repos = { "/w/atlas": { top: "/w/atlas", common: "/w/atlas/.git", branch: "main" }, "/wt/atlas-zoom": { top: "/wt/atlas-zoom", common: "/w/atlas/.git", branch: "feat/zoom" }, "/w/ledger": { top: "/w/ledger", common: "/w/ledger/.git", branch: "main" } };
  const index = new Map([
    ["project:atlas", { id: "project:atlas", kind: "project", name: "atlas", path: "/w/atlas" }],
    ["project:ledger", { id: "project:ledger", kind: "project", name: "ledger", path: "/w/ledger" }],
    ["zone:tmp", { id: "zone:tmp", kind: "zone", name: "Temp", path: "/tmp" }],
    ...Object.entries(sessions).map(([id, s]) => [id, { id, kind: "session", title: s.title, cwd: s.cwd, branch: s.branch, updatedAt: s.live.lastEventAt }]),
  ]);
  const homes = { "/w/atlas": "project:atlas", "/wt/atlas-zoom": "project:atlas", "/w/ledger": "project:ledger", "/tmp/one": "zone:tmp", "/tmp/two": "zone:tmp" };
  const state = {
    index,
    live: { sessions: new Map(Object.entries(sessions).map(([id, s]) => [id, s.live])) },
    homeOf: (id) => homes[index.get(id)?.cwd] ?? null,
  };
  const store = makeStore(path.join(tmp(), "engine.sqlite"));
  const engine = new Engine({ state, store, repos: { of: async (cwd) => repos[cwd] ?? null }, now: () => t.now });
  return { engine, store, t };
}

const A = "claude:aaaaaaaa-0001", B = "codex:bbbbbbbb-0002", C = "claude:cccccccc-0003", D = "codex:dddddddd-0004", E = "claude:eeeeeeee-0005", F = "claude:ffffffff-0006";

test("peers: the same repo, worktrees included, and outside git only the same folder", async () => {
  const { engine } = world(Date.now());
  const ids = (list) => list.map((a) => a.id).sort();
  assert.deepEqual(ids(await engine.peers(A)), [B, D].sort());
  assert.equal(ids(await engine.peers(A, { scope: "machine" })).length, 5);
  // Two scratch folders in the same catch-all group aren't one repo.
  assert.deepEqual(ids(await engine.peers(E)), []);
  const zoom = (await engine.peers(A)).find((a) => a.id === D);
  assert.deepEqual(zoom.overlap, ["src/refunds.ts"], "the same file in another worktree overlaps");
  assert.equal((await engine.whoami(A)).peersInRepo, 2);
});

test("conflicts: an edit in this tree, the same file in another worktree, and claims", async () => {
  const { engine } = world(Date.now());
  const routes = await engine.conflicts(A, ["/w/atlas/src/routes.ts"]);
  assert.deepEqual(routes.map((c) => [c.agent.id, c.how]), [[B, "editing"]]);
  const refunds = await engine.conflicts(B, ["/w/atlas/src/refunds.ts"]);
  assert.deepEqual(refunds.map((c) => [c.agent.id, c.how]).sort(), [[A, "editing"], [D, "worktree"]].sort());
  const claimed = await engine.claim(C, { paths: ["/w/atlas/docs/"], note: "rewriting the docs" });
  assert.equal(claimed.claimed[0], "/w/atlas/docs/");
  const docs = await engine.conflicts(A, ["/w/atlas/docs/guide.md"]);
  assert.deepEqual(docs.map((c) => [c.agent.id, c.how, c.note]), [[C, "claimed", "rewriting the docs"]]);
  // A relative claim is inside the claimer's own working tree.
  await engine.claim(B, { paths: ["src/*.ts"], note: "routing" });
  assert.ok((await engine.whoHas(A, "/w/atlas/src/index.ts")).some((c) => c.agent.id === B && c.how === "claimed"));
  assert.deepEqual(engine.release(B, {}), { released: 1 });
});

test("who has a file, asked by a person: next to the repo the file is in", async () => {
  const { engine } = world(Date.now());
  // The repo of a folder inside a working tree, as git would say.
  const tops = { "/w/atlas": { top: "/w/atlas", common: "/w/atlas/.git" }, "/wt/atlas-zoom": { top: "/wt/atlas-zoom", common: "/w/atlas/.git" } };
  engine.repos = { of: async (dir) => Object.entries(tops).find(([top]) => dir === top || dir.startsWith(top + "/"))?.[1] ?? null };
  const hands = await engine.whoHas(null, "/w/atlas/src/refunds.ts");
  assert.deepEqual(hands.map((h) => [h.agent.id, h.how]).sort(), [[A, "editing"], [D, "worktree"]].sort());
  assert.deepEqual(await engine.whoHas(null, "/elsewhere/notes.md"), []);
});

test("messages: to one agent, to the repo, replies, and an inbox that waits", async () => {
  const { engine } = world(Date.now());
  const toRepo = await engine.send(A, { to: "repo", text: "I'm moving refunds into its own module." });
  assert.deepEqual(toRepo.to.map((r) => r.id).sort(), [B, D].sort());
  assert.match(toRepo.to[0].delivery, /isn't connected/, "nobody has been in touch directly yet");
  const got = await engine.inbox(B);
  assert.equal(got.length, 1);
  assert.equal(got[0].from.id, A);
  assert.equal(got[0].to, "repo:atlas");
  assert.deepEqual(await engine.inbox(B), [], "read once");
  // A reply goes back to the sender without naming it.
  const reply = await engine.send(B, { replyTo: got[0].id, text: "OK, I'll stay out of src/refunds.ts." });
  assert.deepEqual(reply.to.map((r) => r.id), [A]);
  // By words from the title, and an inbox that waits for mail.
  const waiting = engine.inbox(C, { waitMs: 5000 });
  await engine.send(A, { to: "ledger test", text: "Is the ledger fixture stable now?" });
  const [m] = await waiting;
  assert.equal(m.text, "Is the ledger fixture stable now?");
  assert.equal((await engine.inbox(A)).length, 1, "the reply");
  await assert.rejects(engine.send(A, { to: "nobody here", text: "hi" }), /no session or actor matches/);
  await assert.rejects(engine.send(A, { to: B, text: "" }), /empty/);
});

test("hooks: a briefing at the start, a warning before an edit, mail between tool calls, and at Stop", async () => {
  const { engine, store, t } = world(Date.now());
  const raw = (id) => id.split(":")[1];
  // A new Claude session in the atlas repo hears who else is there.
  const start = await engine.hook("claude", "SessionStart", { session_id: "99999999-0007", cwd: "/w/atlas" });
  const brief = start.hookSpecificOutput.additionalContext;
  assert.equal(start.hookSpecificOutput.hookEventName, "SessionStart");
  assert.match(brief, /3 other agents/);
  assert.match(brief, /Add partial refunds/);
  assert.match(brief, /Zoom on images.*another worktree, \/wt\/atlas-zoom, on feat\/zoom/);
  assert.doesNotMatch(brief, /ledger/);
  // Codex is about to patch a file Claude just edited: a warning, once.
  const patch = { input: "*** Begin Patch\n*** Update File: src/refunds.ts\n*** End Patch" };
  const warn = await engine.hook("codex", "PreToolUse", { session_id: raw(B), cwd: "/w/atlas", tool_name: "apply_patch", tool_input: patch });
  assert.match(warn.hookSpecificOutput.additionalContext, /Add partial refunds.*edited refunds\.ts/s);
  assert.match(warn.hookSpecificOutput.additionalContext, /another worktree \(branch feat\/zoom\): expect a merge conflict/);
  assert.equal(await engine.hook("codex", "PreToolUse", { session_id: raw(B), cwd: "/w/atlas", tool_name: "apply_patch", tool_input: patch }), null);
  assert.equal(await engine.hook("claude", "PreToolUse", { session_id: raw(A), cwd: "/w/atlas", tool_name: "Read", tool_input: { file_path: "/w/atlas/x" } }), null);
  // Mail arrives between tool calls, framed as coming from another agent.
  await engine.send(A, { to: B, text: "Heads-up: refunds.ts is mid-refactor." });
  const mail = (await engine.hook("codex", "PostToolUse", { session_id: raw(B), cwd: "/w/atlas" })).hookSpecificOutput.additionalContext;
  assert.match(mail, /from "Add partial refunds"/);
  assert.match(mail, /not from your user/);
  assert.equal(await engine.hook("codex", "PostToolUse", { session_id: raw(B), cwd: "/w/atlas" }), null);
  // At Stop, unread mail keeps the agent going, once.
  await engine.send(A, { to: B, text: "One more thing." });
  assert.equal(await engine.hook("codex", "Stop", { session_id: raw(B), stop_hook_active: true }), null);
  const stop = await engine.hook("codex", "Stop", { session_id: raw(B), cwd: "/w/atlas" });
  assert.equal(stop.decision, "block");
  assert.match(stop.reason, /One more thing/);
  // Being in touch makes an agent reachable; SessionEnd takes it out.
  assert.match((await engine.send(A, { to: B, text: "ping" })).to[0].delivery, /current turn/);
  await engine.hook("codex", "SessionEnd", { session_id: raw(B) });
  t.now += 10e3;
  assert.ok(!(await engine.peers(A)).some((a) => a.id === B));
  assert.ok(store.contact(B).ended_at);
});

test("hooks: a subagent's events keep the session's mail for its main thread", async () => {
  const { engine, store, t } = world(Date.now());
  const raw = (id) => id.split(":")[1];
  const unread = (id) => engine.snapshot().then((s) => s.messages.find((m) => m.id === id).to.every((d) => !d.seen));
  // A Claude session's background subagent runs tools while a message for
  // the session comes in: the subagent's hooks don't take it, whatever the
  // event.
  assert.equal(await engine.hook("claude", "PostToolUse", hooks.claude.postToolUse(raw(A), "/w/atlas")), null);
  const { id } = await engine.send(B, { to: A, text: "Heads-up: routes.ts is mid-refactor." });
  t.now += 1000;
  assert.equal(await engine.hook("claude", "SubagentStart", hooks.claude.subagentStart(raw(A), "/w/atlas")), null);
  assert.equal(await engine.hook("claude", "PostToolUse", hooks.claude.postToolUse(raw(A), "/w/atlas", hooks.claude.inSubagent)), null);
  assert.equal(await engine.hook("claude", "SubagentStop", hooks.claude.subagentStop(raw(A), "/w/atlas")), null);
  assert.ok(await unread(id), "still waiting for the main thread");
  // They still show the session at work, without moving it to the
  // subagent's own worktree.
  assert.equal(store.contact(A).at, t.now);
  await engine.hook("claude", "PostToolUse", hooks.claude.postToolUse(raw(A), "/wt/atlas-agent-1", hooks.claude.inSubagent));
  assert.equal(store.contact(A).cwd, "/w/atlas");
  // The main thread's next tool call gets it, once; a session run with
  // --agent sends agent_type on its main thread, which is still the main
  // thread.
  const mail = await engine.hook("claude", "PostToolUse", hooks.claude.postToolUse(raw(A), "/w/atlas", hooks.claude.withAgent));
  assert.match(mail.hookSpecificOutput.additionalContext, /routes\.ts is mid-refactor/);
  assert.equal(await unread(id), false);
  assert.equal(await engine.hook("claude", "PostToolUse", hooks.claude.postToolUse(raw(A), "/w/atlas")), null);
  // Mail at Stop is the main thread's too.
  await engine.send(B, { to: A, text: "One more thing." });
  assert.equal(await engine.hook("claude", "Stop", { ...hooks.claude.stop(raw(A), "/w/atlas"), ...hooks.claude.inSubagent }), null);
  const stop = await engine.hook("claude", "Stop", hooks.claude.stop(raw(A), "/w/atlas"));
  assert.equal(stop.decision, "block");
  assert.match(stop.reason, /One more thing/);

  // A Codex session's spawned agent: its prompt and tool calls come under
  // the parent's id with agent_id set, and leave the mail too.
  const codexMail = await engine.send(A, { to: B, text: "I'm done with refunds.ts." });
  assert.equal(await engine.hook("codex", "UserPromptSubmit", hooks.codex.userPromptSubmit(raw(B), "/w/atlas", hooks.codex.inSubagent)), null);
  assert.equal(await engine.hook("codex", "PostToolUse", hooks.codex.postToolUse(raw(B), "/w/atlas", hooks.codex.inSubagent)), null);
  assert.ok(await unread(codexMail.id));
  const codexStop = await engine.hook("codex", "Stop", hooks.codex.stop(raw(B), "/w/atlas"));
  assert.match(codexStop.reason, /done with refunds\.ts/);
});

test("hooks: a subagent about to edit a file hears the warning, and so does its main thread", async () => {
  const { engine } = world(Date.now());
  const raw = (id) => id.split(":")[1];
  // B edited routes.ts two minutes ago. A's subagent is about to edit it.
  const file = "/w/atlas/src/routes.ts";
  const sub = await engine.hook("claude", "PreToolUse", hooks.claude.preToolUse(raw(A), "/w/atlas", file, hooks.claude.inSubagent));
  assert.match(sub.hookSpecificOutput.additionalContext, /before you edit routes\.ts.*"Rate-limit the API".*edited routes\.ts/s);
  assert.equal(await engine.hook("claude", "PreToolUse", hooks.claude.preToolUse(raw(A), "/w/atlas", file, hooks.claude.inSubagent)), null, "once for the subagent");
  // The main thread didn't see that, so it hears it too, once.
  const main = await engine.hook("claude", "PreToolUse", hooks.claude.preToolUse(raw(A), "/w/atlas", file));
  assert.match(main.hookSpecificOutput.additionalContext, /edited routes\.ts/);
  assert.equal(await engine.hook("claude", "PreToolUse", hooks.claude.preToolUse(raw(A), "/w/atlas", file)), null);
  // A warning before an edit doesn't take mail, from either thread.
  const { id } = await engine.send(B, { to: A, text: "Go ahead." });
  await engine.hook("claude", "PreToolUse", hooks.claude.preToolUse(raw(A), "/w/atlas", "/w/atlas/README.md", hooks.claude.inSubagent));
  assert.ok((await engine.snapshot()).messages.find((m) => m.id === id).to.every((d) => !d.seen));
});

test("an agent whose process is gone drops out; one joining later still hears the repo", async () => {
  const { engine } = world(Date.now());
  // A process that has already exited.
  const done = spawn(process.execPath, ["-e", ""]);
  await new Promise((r) => done.on("exit", r));
  engine.touch(B, { app: "codex", cwd: "/w/atlas", via: "mcp", pid: done.pid });
  engine.touch(A, { app: "claude", cwd: "/w/atlas", via: "mcp", pid: process.pid });
  const peers = (await engine.peers(C, { scope: "machine" })).map((a) => a.id);
  assert.ok(peers.includes(A), "alive");
  assert.ok(!peers.includes(B), "gone");
  // A message to the repo reaches agents that start there within half an hour.
  const sent = await engine.send(A, { to: "repo", text: "Refunds moved to src/refunds/." });
  assert.ok(sent.alsoTo);
  const late = await engine.hook("claude", "SessionStart", { session_id: "77777777-0008", cwd: "/w/atlas" });
  assert.match(late.hookSpecificOutput.additionalContext, /Refunds moved to src\/refunds\//);
  assert.deepEqual(await engine.inbox("claude:77777777-0008"), [], "and only once");
  const elsewhere = await engine.hook("claude", "SessionStart", { session_id: "66666666-0009", cwd: "/w/ledger" });
  assert.doesNotMatch(elsewhere?.hookSpecificOutput.additionalContext ?? "", /Refunds moved/);
});

test("connect: previews, adds and takes out only XenoNet's entries", async () => {
  const home = tmp();
  const bin = path.join(home, "bin");
  fs.mkdirSync(bin);
  for (const name of ["claude", "codex"]) fs.writeFileSync(path.join(bin, name), "");
  fs.mkdirSync(path.join(home, ".claude"));
  // Someone's own hooks, which must survive.
  const mine = { matcher: "Bash", hooks: [{ type: "command", command: "echo mine" }] };
  // And one an earlier XenoNet added, which gets replaced.
  const old = { hooks: [{ type: "command", command: "XENONET_PORT=47842 '/old/node' '/old/xenonet/cli/hook.js' claude", timeout: 10 }] };
  fs.writeFileSync(path.join(home, ".claude", "settings.json"), JSON.stringify({ model: "opus", hooks: { PreToolUse: [mine], SessionStart: [old] } }));
  const calls = [];
  const connect = createConnect({ platform: "darwin", port: 47842, root: "/app", exec: "/Apps/XenoNet Dev.app/Contents/MacOS/Electron", runAsNode: true, home, env: { PATH: bin }, run: async (file, args) => (calls.push([path.basename(file), ...args]), args[1] === "get" ? Promise.reject(new Error("not found")) : "") });

  const before = await connect.preview();
  assert.deepEqual(before.apps.map((a) => a.changes.map((c) => c.state)), [["missing", "missing"], ["missing", "missing"]]);

  await connect.apply();
  const settings = JSON.parse(fs.readFileSync(path.join(home, ".claude", "settings.json"), "utf8"));
  assert.equal(settings.model, "opus");
  assert.deepEqual(settings.hooks.PreToolUse[0], mine);
  assert.deepEqual(settings.hooks.PreToolUse[1], { matcher: "Edit|Write|MultiEdit|NotebookEdit", hooks: [{ type: "http", url: "http://127.0.0.1:47842/api/hook/claude", timeout: 5 }] });
  // SessionStart takes only commands in Claude Code.
  assert.equal(settings.hooks.SessionStart.length, 1);
  assert.match(settings.hooks.SessionStart[0].hooks[0].command, /^XENONET_PORT=47842 ELECTRON_RUN_AS_NODE=1 '\/Apps\/XenoNet Dev\.app\/Contents\/MacOS\/Electron' '\/app\/cli\/xenonet\.js' hook claude$/);
  assert.ok(fs.existsSync(path.join(home, ".claude", "settings.json.before-xenonet")));
  const add = calls.find((c) => c[0] === "claude" && c[2] === "add-json");
  assert.deepEqual(JSON.parse(add[4]), { type: "stdio", command: "/Apps/XenoNet Dev.app/Contents/MacOS/Electron", args: ["/app/cli/xenonet.js", "mcp"], env: { XENONET_PORT: "47842", ELECTRON_RUN_AS_NODE: "1" } });
  assert.deepEqual(add.slice(-2), ["--scope", "user"]);
  assert.ok(calls.some((c) => c[0] === "codex" && c[1] === "mcp" && c[2] === "add" && c.includes("ELECTRON_RUN_AS_NODE=1")));
  const codexHooks = JSON.parse(fs.readFileSync(path.join(home, ".codex", "hooks.json"), "utf8"));
  assert.deepEqual(Object.keys(codexHooks.hooks).sort(), ["PostToolUse", "PreToolUse", "SessionEnd", "SessionStart", "Stop", "UserPromptSubmit"]);
  assert.equal(codexHooks.hooks.PreToolUse[0].matcher, "apply_patch|Edit|Write");
  assert.equal(codexHooks.hooks.SessionEnd[0].hooks[0].timeout, 3, "Codex's most for SessionEnd");

  // Again: nothing doubles up.
  await connect.apply();
  assert.equal(JSON.parse(fs.readFileSync(path.join(home, ".claude", "settings.json"), "utf8")).hooks.PreToolUse.length, 2);

  await connect.remove();
  const after = JSON.parse(fs.readFileSync(path.join(home, ".claude", "settings.json"), "utf8"));
  assert.deepEqual(after, { model: "opus", hooks: { PreToolUse: [mine] } });
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(home, ".codex", "hooks.json"), "utf8")), {});
  assert.ok(calls.some((c) => c[0] === "claude" && c[2] === "remove"));
  assert.deepEqual(mergeHooks({ hooks: {} }, () => true), {});
});

test("connect: a settings file that isn't JSON, or can't be read, is left alone, and the error quotes none of it and says ~", async () => {
  const home = tmp();
  const connect = createConnect({ platform: "darwin", port: 47842, root: "/app", home, env: { PATH: path.join(home, "bin") }, run: async () => "" });
  const reason = (pending) => pending.then(() => assert.fail("it went ahead"), (err) => err.message);
  // Made-up tokens, in files cut short.
  const secret = "made-up-token-0000";
  fs.writeFileSync(path.join(home, ".claude.json"), `{\n  "primaryApiKey": "${secret}",\n  "mcpServers": {`);
  let message = await reason(connect.preview());
  assert.match(message, /^~[\\/]\.claude\.json isn't valid JSON \(line \d+, column \d+\), so XenoNet leaves it alone$/);
  assert.ok(!message.includes(secret) && !message.includes(home), message);
  fs.rmSync(path.join(home, ".claude.json"));

  const settings = path.join(home, ".claude", "settings.json");
  fs.mkdirSync(path.dirname(settings));
  fs.writeFileSync(settings, `{ "env": { "ANTHROPIC_API_KEY": "${secret}" } `);
  message = await reason(connect.apply());
  assert.match(message, /^~[\\/]\.claude[\\/]settings\.json isn't valid JSON \(line 1, column \d+\), so XenoNet leaves it alone$/);
  assert.ok(!message.includes(secret), message);
  assert.deepEqual([fs.readdirSync(path.dirname(settings)), fs.existsSync(path.join(home, ".codex"))], [["settings.json"], false], "nothing written, nor a copy made");

  fs.rmSync(settings);
  fs.mkdirSync(settings);
  assert.equal((await reason(connect.preview())).replace(/\\/g, "/"), "XenoNet can't read ~/.claude/settings.json (EISDIR), so it leaves it alone");
});

// A stand-in XenoNet on a spare port that records what reaches it.
async function fakeXenoNet(reply = () => [200, { ok: true }]) {
  const seen = [];
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      seen.push({ url: req.url, agent: req.headers["x-xenonet-agent"], body: body ? JSON.parse(body) : null });
      const [status, data] = reply(req);
      res.writeHead(status, { "content-type": "application/json" }).end(data ? JSON.stringify(data) : "");
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  return { port: server.address().port, seen, close: () => server.close() };
}

// xenonet mcp and xenonet hook, run as agents run them, each with a home folder
// of its own, an empty PATH and none of the variables that point at real
// folders, as test/cli.test.js runs xenonet: a tool that works here when it
// finds no XenoNet (the doctor, when a stand-in is slow to answer) reads
// nothing of this computer's. Names are matched in any case, as Windows
// spells PATH Path.
const agentHome = tmp();
const REAL = new Set(["PATH", "CLAUDE_CONFIG_DIR", "CODEX_HOME", "HERMES_HOME", "GH_CONFIG_DIR", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "APPDATA", "LOCALAPPDATA", "XENONET_STATE_DIR", "XENONET_CACHE_DIR", "XENONET_PROJECTS_DIR", "XENONET_WIKI_DIR", "XENONET_CLAUDE_DESKTOP_DIR"]);
function stdio(script, args, env, input = null) {
  const own = Object.fromEntries(Object.entries(process.env).filter(([name]) => !REAL.has(name.toUpperCase())));
  Object.assign(own, { HOME: agentHome, USERPROFILE: agentHome, PATH: path.join(agentHome, "bin"), XENONET_CODEX_BIN: path.join(agentHome, "bin", "codex"), CLAUDE_CODE_SESSION_ID: "", CLAUDE_PID: "", CODEX_THREAD_ID: "", XENONET_ACTOR_TOKEN: "", XENONET_ACTOR_TOKEN_FILE: "" });
  const proc = spawn(process.execPath, [script, ...args], { cwd: root, env: { ...own, ...env }, stdio: ["pipe", "pipe", "inherit"] });
  if (input !== null) proc.stdin.end(input);
  return proc;
}

test("mcp: says which Claude session or Codex thread is calling", async () => {
  const xenonet = await fakeXenoNet(() => [200, [{ id: "codex:other" }]]);
  const run = async (env, meta) => {
    const proc = stdio("cli/xenonet.js", ["mcp"], { XENONET_PORT: String(xenonet.port), ...env });
    const replies = new Map();
    let buf = "";
    proc.stdout.on("data", (d) => {
      buf += d;
      for (let i; (i = buf.indexOf("\n")) >= 0; buf = buf.slice(i + 1)) {
        const m = JSON.parse(buf.slice(0, i));
        replies.get(m.id)?.(m);
      }
    });
    const rpc = (id, method, params) => new Promise((resolve) => (replies.set(id, resolve), proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n")));
    await rpc(1, "initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } });
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
    const { result } = await rpc(2, "tools/list", {});
    const call = await rpc(3, "tools/call", { name: "peers", arguments: { scope: "machine" }, ...(meta ? { _meta: meta } : {}) });
    proc.kill();
    return { tools: result.tools.map((t) => t.name), call };
  };
  try {
    const claude = await run({ CLAUDE_CODE_SESSION_ID: "11111111-2222-3333-4444-555555555555", CLAUDE_PROJECT_DIR: "/w/atlas", CLAUDE_PID: "4242" });
    assert.deepEqual(claude.tools, ["whoami", "peers", "who_has", "send", "inbox", "claim", "release", "focus", "doctor"]);
    // The agent's process, so XenoNet can tell when it's gone.
    assert.deepEqual(xenonet.seen.at(-1), { url: "/api/engine/peers", agent: "claude:11111111-2222-3333-4444-555555555555", body: { scope: "machine", cwd: "/w/atlas", pid: 4242 } });
    assert.match(claude.call.result.content[0].text, /codex:other/);
    await run({}, { threadId: "019a0000-aaaa-bbbb-cccc-000000000001" });
    assert.equal(xenonet.seen.at(-1).agent, "codex:019a0000-aaaa-bbbb-cccc-000000000001");
    assert.equal(xenonet.seen.at(-1).body.pid, process.pid, "Codex: the server's parent");
    const nobody = await run({}, null);
    assert.ok(nobody.call.result.isError);
    assert.match(nobody.call.result.content[0].text, /can't tell which session/);
  } finally {
    xenonet.close();
  }
});

test("mcp doctor: what isn't fine, from the running XenoNet", async () => {
  const result = { at: 1, ms: 5, results: [{ id: "xenonet.server", group: "XenoNet", title: "XenoNet is running", status: "ok", detail: "XenoNet on port 1" }, { id: "codex.auth", group: "Codex", title: "Codex is signed in", status: "fail", detail: "not signed in", fix: "Run codex login." }], summary: {} };
  const xenonet = await fakeXenoNet((req) => (req.url === "/api/health" ? [200, { ok: true, name: "XenoNet Dev" }] : req.url === "/api/doctor" ? [200, result] : [404, {}]));
  try {
    const proc = stdio("cli/xenonet.js", ["mcp"], { XENONET_PORT: String(xenonet.port), CLAUDE_CODE_SESSION_ID: "11111111-2222-3333-4444-555555555555" });
    let buf = "";
    const reply = new Promise((resolve) =>
      proc.stdout.on("data", (d) => {
        buf += d;
        for (const line of buf.split("\n").slice(0, -1)) if (JSON.parse(line).id === 2) resolve(JSON.parse(line));
      }),
    );
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } } }) + "\n");
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
    proc.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "doctor", arguments: {} } }) + "\n");
    const out = JSON.parse((await reply).result.content[0].text);
    proc.kill();
    assert.equal(out.by, "XenoNet Dev");
    assert.deepEqual(out.notes, [{ title: "Codex is signed in", status: "fail", detail: "not signed in", fix: "Run codex login." }]);
    assert.equal(out.summary.fail, 1);
  } finally {
    xenonet.close();
  }
});

test("hook bridge: passes the event to XenoNet and prints its answer; silent when XenoNet is away", async () => {
  const answer = { hookSpecificOutput: { hookEventName: "SessionStart", additionalContext: "XenoNet: hello" } };
  const xenonet = await fakeXenoNet(() => [200, answer]);
  const out = async (port) => {
    const proc = stdio("cli/xenonet.js", ["hook", "codex"], { XENONET_PORT: String(port) }, JSON.stringify({ session_id: "t1", hook_event_name: "SessionStart" }));
    let text = "";
    proc.stdout.on("data", (d) => (text += d));
    const code = await new Promise((r) => proc.on("exit", r));
    return { code, text };
  };
  try {
    const got = await out(xenonet.port);
    assert.deepEqual([got.code, JSON.parse(got.text)], [0, answer]);
    assert.deepEqual(xenonet.seen[0], { url: "/api/hook/codex", agent: undefined, body: { session_id: "t1", hook_event_name: "SessionStart" } });
  } finally {
    xenonet.close();
  }
  const away = await out(xenonet.port);
  assert.deepEqual(away, { code: 0, text: "" });
});

test("a git answer with lines missing is no repo, not a crash", () => {
  assert.equal(parseRepo(""), null);
  assert.equal(parseRepo("C:/p\n"), null);
  assert.deepEqual(parseRepo("C:/p\nC:/p/.git\nmain\n"), { top: "C:/p", common: path.resolve("C:/p/.git"), branch: "main" });
});
