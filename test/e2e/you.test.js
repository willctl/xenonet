// End to end: the page reports you (src/you.js) to XenoNet Dev's test
// instance, with the window key only its window holds (core/actors/key.js,
// through the preload bridge). The test instance runs the demo, where the
// made-up you is shown like any actor, so the list says what the window
// reported; outside the demo your record is the window's alone, which the
// unit tests cover (test/actors.test.js). The window stays hidden;
// XENONET_E2E_SHOW=inactive shows it to watch.
//
//   npm run test:e2e
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { xenonet } from "../../core/control.js";

const port = await new Promise((resolve) => {
  const probe = net.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address();
    probe.close(() => resolve(port));
  });
});
const s = xenonet("test", { port });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Runs a test body, and on failure saves what the window looked like.
const shoot = (name, body) => async () => {
  try {
    await body();
  } catch (err) {
    const shot = await s.screenshot({ name: `e2e-${name}`, inline: false }).catch(() => null);
    if (shot) err.message += `\n(screenshot: ${shot.file})`;
    throw err;
  }
};

// Polls `fn` until `ok` holds of what it returns, or the time's up.
async function until(fn, ok, ms = 15000) {
  for (const end = Date.now() + ms; ; await sleep(250)) {
    const v = await fn().catch(() => null);
    if ((v != null && ok(v)) || Date.now() > end) return v;
  }
}

// Your record as anyone may read it: on the demo, all of it.
const you = () => s.api("/api/actors/you");

async function home() {
  await s.input({ keys: "Escape", move: { x: 4, y: 4 } });
  const st = await s.driver("state");
  if (st.selection) await s.driver("select", null);
}

const findOne = async (query, kind) => {
  const { nodes } = await s.driver("find", { query, kind, limit: 1 });
  assert.ok(nodes.length, `the demo has a ${kind ?? "node"} matching "${query}"`);
  return nodes[0];
};

before(async () => {
  // One test instance at a time (it has a single-instance lock).
  await xenonet("test").quit();
  await s.launch({ show: process.env.XENONET_E2E_SHOW ?? "hidden" });
}, { timeout: 180000 });

after(async () => {
  if (process.env.XENONET_E2E_KEEP === "1") return;
  await s.quit().catch(() => {});
  fs.rmSync(path.join(fs.realpathSync.native(os.tmpdir()), `xenonet-demo-${port}`), { recursive: true, force: true, maxRetries: 10 });
});

test("a selection that holds becomes what you're doing, in XenoNet's words", { timeout: 60000 }, shoot("you-looked", async () => {
  await home();
  const atlas = await findOne("atlas-api", "project");
  const since = Date.now();
  await s.driver("select", atlas.id);
  const record = await until(you, (r) => r.recent?.some((e) => e.kind === "status" && e.target === atlas.id && e.at >= since));
  const looked = record?.recent?.find((e) => e.kind === "status" && e.target === atlas.id && e.at >= since);
  assert.ok(looked, `your record says you looked at atlas-api: ${JSON.stringify(record?.recent?.slice(-3))}`);
  assert.equal(looked.detail, "Looking at atlas-api");
  assert.equal(looked.via, "observed");
  assert.ok(record.targets.some((t) => t.relation === "watches" && t.id === atlas.id), "and it watches the project");
  assert.equal((await s.driver("state")).errors, 0, JSON.stringify(await s.driver("errors")));
}));

test("reading a session's stats is reading the session, and its title never leaves it", { timeout: 60000 }, shoot("you-read", async () => {
  await home();
  const session = await findOne("partial refunds", "session");
  const since = Date.now();
  await s.driver("stats", session.id);
  const record = await until(you, (r) => r.recent?.some((e) => e.target === session.id && e.at >= since && /^Reading /.test(e.detail ?? "")));
  const read = record?.recent?.find((e) => e.target === session.id && e.at >= since && /^Reading /.test(e.detail ?? ""));
  assert.ok(read, JSON.stringify(record?.recent?.slice(-3)));
  assert.match(read.detail, /^Reading a (Claude Code|Codex) session in \S+$/);
  assert.doesNotMatch(JSON.stringify(record), /partial refunds/i, "a session is never named by its title");
  await home();
}));

test("what you type into the palette never reaches XenoNet", { timeout: 60000 }, shoot("you-typed", async () => {
  await home();
  const since = Date.now();
  await s.input({ keys: "Mod+K" });
  await s.input({ text: "zebra quartz secret" });
  await sleep(2500);
  await s.input({ keys: "Escape" });
  await sleep(1500);
  const record = await you();
  const events = await s.api(`/api/actors/events?since=${since}`);
  assert.doesNotMatch(JSON.stringify(record), /zebra|quartz|secret/);
  assert.doesNotMatch(JSON.stringify(events), /zebra|quartz|secret/);
}));

test("your reports take the window's key, which only the window has, and never in a URL", { timeout: 30000 }, shoot("you-key", async () => {
  const outside = await fetch(`${s.base}/api/actors/you`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ presence: "working" }) });
  assert.equal(outside.status, 403, "a program without the key is refused");
  assert.equal((await outside.json()).code, "yours");
  // The page has it from the desktop app, reads your record with it, and
  // keeps it out of its address, its storage and the page itself.
  const inside = await s.eval(`return (async () => {
    const key = await window.xenonetDesktop.windowKey();
    const r = await fetch("/api/actors/you", { headers: { "x-xenonet-window": key } });
    const body = await r.json();
    return { has: typeof key === "string" && key.length === 64, status: r.status, name: body.name, heartbeatMs: body.heartbeatMs, inUrl: location.href.includes(key), stored: JSON.stringify({ ...localStorage, ...sessionStorage }).includes(key), inPage: document.documentElement.outerHTML.includes(key) };
  })()`);
  assert.deepEqual(inside, { has: true, status: 200, name: "You", heartbeatMs: 60000, inUrl: false, stored: false, inPage: false });
}));
