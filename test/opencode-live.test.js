// OpenCode live: what its XenoNet plugin forwards (scripts/demo/opencode-
// plugin.js), and that it fails open. Fake bus events, a fake fetch and
// made-up sessions.
import { test } from "node:test";
import assert from "node:assert/strict";
import { XenoNet } from "../scripts/demo/opencode-plugin.js";
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

// The engine with a state that holds the events, as the real one does.
