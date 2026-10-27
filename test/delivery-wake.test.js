// Waking sessions that have no live way in (core/delivery): a Stop hook that
// listens after a turn, a headless resume of a session whose process has
// gone, and the plan readings that hold wakes at 90%. Made-up sessions; the
// commands a resume would run are checked, never run.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { EventEmitter } from "node:events";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { DeliveryPolicy } from "../core/delivery/policy.js";
import { DeliveryRouter } from "../core/delivery/router.js";
import { createResume, resumeCommand, resumeEnv } from "../core/delivery/resume.js";
import { planUsage } from "../core/delivery/usage.js";
import { listenSettings, writeListenSettings, liveSetup } from "../core/delivery/setup.js";

const A = "claude:aaaaaaaa-0001", B = "claude:bbbbbbbb-0002";
const ID = "0b6f1c2e-3d4a-4b5c-8d9e-0f1a2b3c4d5e";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Two Claude Code sessions in one folder, a policy, and a router whose
// resume is a stand-in.
function world({ limits = {}, resumed = [] } = {}) {
  const t = { now: Date.now() };
  const now = () => t.now;
  const live = (lastEventAt) => ({ lastEventAt, edits: new Map(), status: () => "idle", needsYou: () => null });
  const sessions = { [A]: "First", [B]: "Second" };
  const index = new Map(Object.entries(sessions).map(([id, title]) => [id, { id, kind: "session", app: "claude", title, cwd: "/w/atlas", updatedAt: t.now - 200e3 }]));
  const state = { graph: { identities: [] }, index, live: { sessions: new Map(Object.keys(sessions).map((id) => [id, live(t.now - 200e3)])) }, homeOf: () => null };
  const store = new EngineStore(":memory:");
  for (const id of Object.keys(sessions)) store.touch(id, { app: "claude", cwd: "/w/atlas", via: "hook" }, t.now - 200e3);
  const policy = new DeliveryPolicy({ now, limits });
  const repos = { of: async () => null };
  const engine = new Engine({ state, store, now, repos, policy });
  const resume = { can: () => true, running: () => false, deliver: async (x) => (resumed.push(x), { ok: true, where: "pid 7" }) };
  const router = new DeliveryRouter({ engine, policy, resume, now, ackMs: 50 });
  engine.delivery = router;
  return { t, engine, store, policy, router, resumed };
}

test("plan usage for the 90% rule: the fuller window that hasn't reset, from a fresh reading", () => {
  const now = Date.parse("2026-10-16T12:00:00Z");
  const codex = { primary: { pct: 40, resetsAt: now + 3600e3 }, secondary: { pct: 92, resetsAt: null }, observedAt: now - 60e3 };
  assert.equal(planUsage({ codex }, "codex", now), 0.92);
  assert.equal(planUsage({ codex: { ...codex, secondary: { pct: 99, resetsAt: now - 1 } } }, "codex", now), 0.4, "a window that reset says nothing");
  assert.equal(planUsage({ codex: { ...codex, observedAt: now - 7 * 3600e3 } }, "codex", now), null, "too old");
  assert.equal(planUsage({ claude: { fiveHour: 30, sevenDay: 91, resets: {} } }, "claude", now), 0.91);
  assert.equal(planUsage({ claude: { fiveHour: null, sevenDay: null, stale: true } }, "claude", now), null);
  assert.equal(planUsage({ claude: { fiveHour: 95 } }, "opencode", now), null, "no readings for it");
});

test("resume runs the session's own CLI on the same session, the message on stdin, without another session's variables", async () => {
  assert.deepEqual(resumeCommand(`claude:${ID}`, { settings: "/s/listen.json" }), ["claude", ["-p", "--resume", ID, "--settings", "/s/listen.json"]]);
  assert.deepEqual(resumeCommand(`codex:${ID}`), ["codex", ["exec", "resume", ID, "-"]]);
  assert.equal(resumeCommand("opencode:ses-abc"), null);
  assert.equal(resumeCommand("claude:--help"), null, "only a session id");
  assert.deepEqual(Object.keys(resumeEnv({ PATH: "/bin", CLAUDE_PID: "1", CLAUDECODE: "1", CLAUDE_CODE_SESSION_ID: "x", CODEX_THREAD_ID: "y", XENONET_ACTOR_TOKEN: "z", CLAUDE_CONFIG_DIR: "/c", CODEX_HOME: "/h" })).sort(), ["CLAUDE_CONFIG_DIR", "CODEX_HOME", "PATH"]);

  const runs = [];
  const spawn = (cmd, args, opts) => {
    const child = new EventEmitter();
    child.pid = 99;
    child.stdin = new EventEmitter();
    child.stdin.end = (text) => runs.push({ cmd, args, cwd: opts.cwd, env: opts.env, text });
    child.kill = () => child.emit("exit", null);
    setImmediate(() => child.emit("spawn"));
    return child;
  };
  // The runner finds the program (runs.js); here it's the app's name.
  const find = (app) => app;
  const r = createResume({ spawn, find, env: { PATH: "/bin", CLAUDE_PID: "1" }, settings: () => "/s/listen.json", exists: () => true });
  const out = await r.deliver({ agent: `claude:${ID}`, cwd: "/w/atlas", text: "-rf [XenoNet from Muse] hi" });
  assert.deepEqual(out, { ok: true, where: "pid 99" });
  assert.deepEqual(runs[0], { cmd: "claude", args: ["-p", "--resume", ID, "--settings", "/s/listen.json"], cwd: "/w/atlas", env: { PATH: "/bin" }, text: "-rf [XenoNet from Muse] hi" });
  assert.ok(r.running(`claude:${ID}`));
  assert.match((await r.deliver({ agent: `claude:${ID}`, cwd: "/w/atlas", text: "again" })).why, /under way/, "one resumed turn at a time");
  assert.match((await createResume({ spawn, find, exists: () => false }).deliver({ agent: `codex:${ID}`, cwd: "/gone", text: "x" })).why, /folder/);
  const missing = () => {
    const child = new EventEmitter();
    setImmediate(() => child.emit("error", Object.assign(new Error("spawn claude ENOENT"), { code: "ENOENT" })));
    return child;
  };
  assert.match((await createResume({ spawn: missing, find, exists: () => true }).deliver({ agent: `claude:${ID}`, cwd: "/w", text: "x" })).why, /isn't installed/);
  assert.equal((await createResume({ spawn, find: () => null, exists: () => true }).deliver({ agent: `claude:${ID}`, cwd: "/w", text: "x" })).why, "claude isn't installed here", "found nowhere");
});

test("the listen settings: a Stop hook to XenoNet's listen route, given longer than its window, written once", async () => {
  assert.deepEqual(listenSettings({ port: 47842, seconds: 120 }), { hooks: { Stop: [{ hooks: [{ type: "http", url: "http://127.0.0.1:47842/api/hook/claude/listen?seconds=120", timeout: 150 }] }] } });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-listen-"));
  const file = writeListenSettings({ dir, port: 47840 });
  assert.equal(path.basename(file), "listen-47840.json");
  const mtime = fs.statSync(file).mtimeMs;
  writeListenSettings({ dir, port: 47840 });
  assert.equal(fs.statSync(file).mtimeMs, mtime, "unchanged, not written again");
  const r = await liveSetup({ exec: async () => null, opencode: { servers: async () => [] }, listenFile: path.join(dir, "listen-47840.json"), home: path.dirname(dir) });
  // Your home folder as $HOME, but on Windows, whose shells spell it
  // otherwise, the path as it is.
  const listen = path.join(dir, "listen-47840.json");
  assert.equal(r.listen, process.platform === "win32" ? `claude --settings "${listen}"` : `claude --settings "$HOME/${path.basename(dir)}/listen-47840.json"`);
  fs.rmSync(dir, { recursive: true, force: true });
});

test("a listening Stop hook hands over the message that comes during its window, and the router leaves it to the hook", async () => {
  const w = world();
  const hook = w.engine.listen(B, { ms: 2000 });
  await sleep(10);
  assert.ok(w.engine.listening.has(B));
  assert.equal(w.router.way(A, await w.engine.view(B)), "right away: it's listening for messages");
  const sent = await w.engine.send(A, { to: B, text: "your turn" });
  const out = await hook;
  assert.equal(out.decision, "block");
  assert.match(out.reason, /your turn/);
  assert.deepEqual(out.ids, [sent.id]);
  assert.equal(w.store.unseen(B).length, 0);
  assert.ok(!w.engine.listening.has(B));
  const rec = w.router.list().find((r) => r.message === sent.id);
  assert.equal(rec.how, "listen");
  assert.ok(rec.ackedAt, "acked when the hook took it");
});

test("a listening Stop hook lets go: at the end of its window, when its request goes, when the main Stop hook handed mail over, and when the wake budget says no", async () => {
  let w = world();
  assert.equal(await w.engine.listen(B, { ms: 30 }), null, "nothing came");

  w = world();
  const gone = new AbortController();
  const hook = w.engine.listen(B, { ms: 5000, signal: gone.signal });
  await sleep(10);
  gone.abort();
  assert.equal(await hook, null);

  w = world();
  await w.engine.send(A, { to: B, text: "at the stop" });
  const main = await w.engine.hook("claude", "Stop", { session_id: B.slice("claude:".length) });
  assert.equal(main.decision, "block");
  assert.equal(await w.engine.listen(B, { ms: 5000 }), null, "the session goes on anyway");

  w = world({ limits: { wakesPerTargetPerHour: 0 } });
  await w.engine.send(A, { to: B, text: "too many" });
  assert.equal(await w.engine.listen(B, { ms: 5000 }), null);
  assert.equal(w.store.unseen(B).length, 1, "it waits for the next turn");
});

test("a session that ended is resumed for a message only with your consent, and one nothing acked goes back to the hooks", async () => {
  const w = world();
  w.store.ended(B, w.t.now);
  assert.ok(!(await w.engine.agents()).some((a) => a.id === B), "ended, and not allowed: not among the agents");
  await assert.rejects(w.engine.send(A, { to: "second", text: "x" }), /no session or actor matches/, "nor found by its title");
  const waits = await w.engine.send(A, { to: B, text: "for later" });
  await sleep(10);
  const held = w.router.list().find((r) => r.message === waits.id);
  assert.equal(held.state, "next-turn", "by its id it waits in its inbox, as before");
  assert.match(held.why, /resume: no consent/);
  assert.equal(w.resumed.length, 0);
  w.store.markSeen(B, [waits.id], w.t.now);
  w.policy.allow(B);
  assert.ok((await w.engine.agents()).some((a) => a.id === B && a.status === "ended"));
  const sent = await w.engine.send(A, { to: "second", text: "pick this up" });
  assert.equal(sent.to[0].delivery, "right away, in a turn XenoNet resumes it for");
  await sleep(10);
  const rec = w.router.list().find((r) => r.message === sent.id);
  assert.equal(rec.how, "resume");
  assert.equal(w.resumed[0].agent, B);
  assert.match(w.resumed[0].text, /pick this up/);
  assert.equal(w.store.unseen(B).length, 0);
  await sleep(80);
  assert.equal(rec.state, "notified");
  assert.equal(w.store.unseen(B).length, 1, "unacked: the hooks hand it over next turn");
});
