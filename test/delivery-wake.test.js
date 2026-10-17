// Waking sessions that have no live way in (core/delivery): a Stop hook that
// listens after a turn, and the plan readings that hold wakes at 90%.
// Made-up sessions.
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
import { planUsage } from "../core/delivery/usage.js";
import { listenSettings, writeListenSettings, liveSetup } from "../core/delivery/setup.js";

const A = "claude:aaaaaaaa-0001", B = "claude:bbbbbbbb-0002";
const ID = "0b6f1c2e-3d4a-4b5c-8d9e-0f1a2b3c4d5e";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Two Claude Code sessions in one folder, a policy and a router.
function world({ limits = {} } = {}) {
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
  const router = new DeliveryRouter({ engine, policy, now, ackMs: 50 });
  engine.delivery = router;
  return { t, engine, store, policy, router };
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

test("the listen settings: a Stop hook to XenoNet's listen route, given longer than its window, written once", async () => {
  assert.deepEqual(listenSettings({ port: 47842, seconds: 120 }), { hooks: { Stop: [{ hooks: [{ type: "http", url: "http://127.0.0.1:47842/api/hook/claude/listen?seconds=120", timeout: 150 }] }] } });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-listen-"));
  const file = writeListenSettings({ dir, port: 47840 });
  assert.equal(path.basename(file), "listen-47840.json");
  const mtime = fs.statSync(file).mtimeMs;
  writeListenSettings({ dir, port: 47840 });
  assert.equal(fs.statSync(file).mtimeMs, mtime, "unchanged, not written again");
  const r = await liveSetup({ exec: async () => null, opencode: { servers: async () => [] }, listenFile: path.join(dir, "listen-47840.json"), home: path.dirname(dir) });
  assert.equal(r.listen, `claude --settings "$HOME/${path.basename(dir)}/listen-47840.json"`);
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

