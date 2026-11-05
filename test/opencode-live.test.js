// OpenCode live: what its XenoNet plugin forwards (scripts/demo/opencode-
// plugin.js), what XenoNet makes of it (core/native/opencode-events.js, the
// engine's view, the live snapshot) and that the database stays the
// fallback. Fake bus events, a fake fetch and made-up sessions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { XenoNet } from "../scripts/demo/opencode-plugin.js";
import { OpenCodeEvents, WORKING_MS, WAIT_MS } from "../core/native/opencode-events.js";
import { Engine } from "../core/engine/engine.js";
import { EngineStore } from "../core/engine/store.js";
import { engineRoutes } from "../server/routes/engine.js";

const ID = "opencode:ses-abc123";

// The plugin run against a fake fetch: what it posted, and how it answered.
async function plugin({ status = 200, fail = false, directory = "/w/atlas" } = {}) {
  const posts = [];
  const real = globalThis.fetch;
  globalThis.fetch = async (url, opts) => {
    posts.push({ url: String(url), headers: opts.headers, body: JSON.parse(opts.body) });
    if (fail) throw new Error("ECONNREFUSED");
    return { status, ok: status < 400 };
  };
  const hooks = await XenoNet({ directory });
  const settle = () => new Promise((r) => setTimeout(r, 5));
  return { hooks, posts, settle, done: () => (globalThis.fetch = real) };
}

test("the plugin forwards a session's status, permission, edits and finished messages, as metadata", async () => {
  const p = await plugin();
  try {
    await p.hooks.event({ event: { type: "session.status", properties: { sessionID: "ses_abc123", status: { type: "busy" } } } });
    await p.hooks.event({ event: { type: "permission.asked", properties: { id: "per_1", sessionID: "ses_abc123", permission: "bash", patterns: ["rm -rf /secret"], metadata: { command: "cat ~/.ssh/id_rsa" } } } });
    await p.hooks.event({ event: { type: "permission.replied", properties: { sessionID: "ses_abc123", requestID: "per_1", reply: "once" } } });
    await p.hooks["tool.execute.after"]({ tool: "edit", sessionID: "ses_abc123", callID: "c1", args: { filePath: "/w/atlas/a.js", oldString: "SECRET OLD", newString: "SECRET NEW" } });
    await p.hooks["tool.execute.after"]({ tool: "apply_patch", sessionID: "ses_abc123", callID: "c2", args: { patchText: "*** Begin Patch\n*** Update File: src/b.js\n@@\n-secret\n+secret2\n*** Add File: src/c.js\n+body\n*** End Patch" } });
    await p.hooks["tool.execute.after"]({ tool: "bash", sessionID: "ses_abc123", callID: "c3", args: { command: "echo hi" } });
    await p.hooks.event({ event: { type: "message.updated", properties: { info: { id: "msg_1", sessionID: "ses_abc123", role: "assistant", cost: 0.012, tokens: { input: 10, output: 20, reasoning: 0, cache: { read: 5, write: 1 } }, modelID: "big-pickle", providerID: "opencode", time: { completed: 5 }, summary: "SECRET REPLY" } } } });
    await p.hooks.event({ event: { type: "message.updated", properties: { info: { id: "msg_2", sessionID: "ses_abc123", role: "assistant", time: {} } } } });
    await p.hooks.event({ event: { type: "message.updated", properties: { info: { id: "msg_3", sessionID: "ses_abc123", role: "user", time: { completed: 1 } } } } });
    await p.hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_abc123" } } });
    await p.hooks.event({ event: { type: "something.new", properties: { sessionID: "ses_abc123" } } });
    await p.settle();
    const events = p.posts.flatMap((x) => x.body.events);
    assert.deepEqual(events.map((e) => e.type + (e.state ? `:${e.state}` : "")), ["status:busy", "permission:asked", "permission:replied", "edit", "edit", "edit", "message", "status:idle"]);
    assert.deepEqual(events.filter((e) => e.type === "edit").map((e) => e.path), ["/w/atlas/a.js", "src/b.js", "src/c.js"]);
    assert.deepEqual(events.find((e) => e.type === "permission").tool, "bash");
    assert.deepEqual(events.find((e) => e.type === "message"), { type: "message", id: "msg_1", cost: 0.012, tokens: { input: 10, output: 20, reasoning: 0, cacheRead: 5, cacheWrite: 1 }, provider: "opencode", model: "big-pickle" });
    const sent = JSON.stringify(p.posts);
    for (const secret of ["SECRET", "rm -rf", "id_rsa", "echo hi"]) assert.ok(!sent.includes(secret), `${secret} stays in OpenCode`);
    assert.equal(p.posts[0].url, "http://127.0.0.1:47840/api/engine/opencode-event");
    assert.equal(p.posts[0].headers["x-xenonet-agent"], ID, "as the session, by the id XenoNet keeps it under");
    assert.equal(p.posts[0].body.cwd, "/w/atlas");
  } finally {
    p.done();
  }
});

test("the plugin fails open: a XenoNet that's down costs a hook nothing, and is left alone for a while", async () => {
  const p = await plugin({ fail: true });
  try {
    const t = Date.now();
    await p.hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_abc123" } } });
    assert.ok(Date.now() - t < 100, "hooks return at once");
    await p.settle();
    for (let i = 0; i < 5; i++) await p.hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_abc123" } } });
    await p.settle();
    assert.equal(p.posts.length, 1, "one try, then quiet");
    await p.hooks.event({ event: { type: "session.idle", properties: {} } });
    await assert.doesNotReject(p.hooks["tool.execute.after"]({}));
  } finally {
    p.done();
  }
  const old = await plugin({ status: 404 });
  try {
    await old.hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_abc123" } } });
    await old.settle();
    await old.hooks.event({ event: { type: "session.idle", properties: { sessionID: "ses_abc123" } } });
    await old.settle();
    assert.equal(old.posts.length, 1, "a XenoNet without the route is left alone too");
  } finally {
    old.done();
  }
});

test("working, idle and a permission wait, and how each lapses when the plugin goes quiet", () => {
  const ev = new OpenCodeEvents();
  const t0 = 1_000_000;
  assert.equal(ev.state(ID, t0), null, "nothing heard, nothing said: the database decides");
  ev.ingest(ID, [{ type: "status", state: "busy" }], { now: t0 });
  assert.equal(ev.state(ID, t0 + 1000).status, "working");
  ev.ingest(ID, [{ type: "permission", state: "asked", id: "per_1", tool: "edit" }], { now: t0 + 2000 });
  assert.deepEqual(ev.state(ID, t0 + 3000).needsYou, { kind: "permission", text: "Permission to use edit", since: t0 + 2000 });
  ev.ingest(ID, [{ type: "permission", state: "replied", id: "per_1" }], { now: t0 + 4000 });
  assert.equal(ev.state(ID, t0 + 5000).needsYou, null);
  ev.ingest(ID, [{ type: "permission", state: "asked", id: "per_2" }], { now: t0 + 6000 });
  assert.equal(ev.state(ID, t0 + 7000).needsYou.text, "Permission needed");
  ev.ingest(ID, [{ type: "status", state: "idle" }], { now: t0 + 8000 });
  assert.equal(ev.state(ID, t0 + 9000).status, "idle");
  assert.equal(ev.state(ID, t0 + 9000).needsYou, null, "a turn that ended asks nothing");
  ev.ingest(ID, [{ type: "status", state: "busy" }], { now: t0 + 10_000 });
  assert.equal(ev.state(ID, t0 + 10_000 + WORKING_MS - 1).status, "working");
  assert.equal(ev.state(ID, t0 + 10_000 + WORKING_MS + 1), null, "gone quiet: back to the database");
  ev.ingest(ID, [{ type: "permission", state: "asked", id: "per_3" }], { now: t0 + 20_000 });
  assert.ok(ev.state(ID, t0 + 20_000 + WORKING_MS + 1)?.needsYou, "a wait outlives a quiet plugin, since nothing else will say it was answered");
  assert.equal(ev.state(ID, t0 + 20_000 + WAIT_MS + 1), null);
});

test("edits resolve against the session's folder and go in a session's edits; messages sum once each", () => {
  const ev = new OpenCodeEvents();
  const now = Date.now();
  ev.ingest(ID, [{ type: "edit", path: "src/a.js" }, { type: "edit", path: "/w/other/b.js" }, { type: "edit", path: "" }], { cwd: "/w/atlas", now });
  assert.deepEqual([...ev.state(ID, now).edits.keys()], ["/w/atlas/src/a.js", "/w/other/b.js"]);
  const m = { type: "message", id: "msg_1", cost: 0.5, tokens: { input: 10, output: 2 }, provider: "opencode", model: "big-pickle" };
  ev.ingest(ID, [m, m, { ...m, id: "msg_2", cost: 0.25 }], { now });
  const u = ev.state(ID, now).usage;
  assert.equal(u.costUsd, 0.75);
  assert.deepEqual(u.tokens, { input: 20, output: 4, reasoning: null, cacheRead: null, cacheWrite: null });
  assert.deepEqual([u.provider, u.model], ["opencode", "big-pickle"]);
});

test("what the store doesn't know, or can't use, is ignored", () => {
  const ev = new OpenCodeEvents();
  const bad = [null, 7, "x", {}, { type: "nope" }, { type: "status", state: "sideways" }, { type: "permission", state: "asked" }, { type: "permission", state: "asked", id: "has space" }, { type: "edit", path: 5 }, { type: "message", id: "m", cost: -1, tokens: { input: "x" }, model: "bad model!" }];
  assert.equal(ev.ingest(ID, bad, { now: 1 }), 1, "only the message with an id counts, and it has nothing in it to keep");
  assert.equal(ev.ingest(ID, "not a list"), 0);
  assert.equal(ev.state(ID, 1)?.usage ?? null, null);
  assert.equal(ev.ingest(ID, Array.from({ length: 50 }, () => ({ type: "status", state: "busy" })), { now: 1 }), 20, "a request is read to its first 20 events");
});

// The engine with a state that holds the events, as the real one does.
function machine() {
  const t = { now: Date.now() };
  const now = () => t.now;
  const opencodeEvents = new OpenCodeEvents();
  const index = new Map([[ID, { id: ID, kind: "session", app: "opencode", title: "Atlas: billing API", cwd: "/w/atlas", updatedAt: t.now - 3600e3 }]]);
  const state = { index, live: { sessions: new Map() }, homeOf: () => null, graph: { identities: [] }, opencodeEvents };
  const store = new EngineStore(":memory:");
  const repos = { of: async (cwd) => (cwd ? { top: cwd, common: `${cwd}/.git`, branch: "main" } : null) };
  return { t, engine: new Engine({ state, store, now, repos }), state };
}

test("the engine's view of an OpenCode session goes by the plugin: working, waiting, the files it is editing", async () => {
  const { t, engine } = machine();
  assert.deepEqual(await engine.opencodeEvent(ID, { events: [{ type: "status", state: "busy" }, { type: "permission", state: "asked", id: "per_1", tool: "bash" }, { type: "edit", path: "src/a.js" }], cwd: "/w/atlas" }), { used: 3 });
  const v = await engine.view(ID);
  assert.equal(v.status, "working");
  assert.equal(v.needsYou, "permission");
  assert.deepEqual(v.editing.map((e) => e.path), ["/w/atlas/src/a.js"]);
  await engine.opencodeEvent(ID, { events: [{ type: "permission", state: "replied", id: "per_1" }, { type: "status", state: "idle" }] });
  t.now += 100e3;
  const idle = await engine.view(ID);
  assert.equal(idle.needsYou, null);
  assert.equal(idle.status, "idle", "the database's 'last at work an hour ago' doesn't hold it working");
  assert.throws(() => engine.opencodeEvent("claude:abc", { events: [] }), /only an OpenCode session/);
});

test("without the plugin's word, the view is what the database gave", async () => {
  const { engine } = machine();
  const v = await engine.view(ID);
  assert.equal(v.status, "idle");
  assert.equal(v.needsYou, null);
});

test("the live snapshot has an OpenCode session that waits, for the needs-you strip, once it's on the map", () => {
  const ev = new OpenCodeEvents();
  const now = Date.now();
  ev.ingest(ID, [{ type: "status", state: "busy" }, { type: "permission", state: "asked", id: "per_1", tool: "edit" }, { type: "message", id: "m1", model: "big-pickle" }], { now });
  assert.deepEqual(ev.snapshot(now, () => false), [], "a session the map doesn't have isn't shown");
  const [s] = ev.snapshot(now, (id) => id === ID);
  assert.deepEqual([s.id, s.app, s.status, s.model], [ID, "opencode", "thinking", "big-pickle"]);
  assert.deepEqual(s.needsYou, { kind: "permission", text: "Permission to use edit", since: now });
  for (const k of ["tokens", "context", "toolCounts", "subagents", "files", "mcp", "lastEventAt"]) assert.ok(k in s, `${k} is there for the page`);
});

test("the route takes the plugin's events as the session, and answers a kind it doesn't know with a count", async () => {
  const { engine } = machine();
  const reply = {};
  const route = engineRoutes({
    engine,
    connect: {},
    verify: { check: async () => ({ ok: true }) },
    readJson: async () => ({ events: [{ type: "status", state: "busy" }, { type: "future-kind" }], cwd: "/w/atlas" }),
    send: (res, status, body) => Object.assign(reply, { status, body }),
  });
  await route({ method: "POST", headers: { "x-xenonet-agent": ID }, socket: {} }, {}, new URL("http://x/api/engine/opencode-event"));
  assert.deepEqual([reply.status, reply.body], [200, { used: 1 }]);
});
