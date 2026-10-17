// Delivery's routes (server/routes/delivery.js, and the channel operation in
// server/routes/engine.js): the controls are yours only, never an actor's or
// an agent's; a Claude Code session's channel poll answers with its messages;
// and peers takes a selector as its scope.
import { test } from "node:test";
import assert from "node:assert/strict";
import { deliveryRoutes } from "../server/routes/delivery.js";
import { engineRoutes } from "../server/routes/engine.js";
import { DeliveryPolicy } from "../core/delivery/policy.js";
import { Channels } from "../core/delivery/channel.js";

const ME = "claude:11111111-2222-3333-4444-555555555555";
const go = async (route, method, path, { body = {}, headers = {} } = {}) => {
  const res = { writeHead: (status) => Object.assign(res, { status }) && { end: () => {} } };
  await route({ method, headers, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
  return res;
};
const send = (res, status, body) => Object.assign(res, { status, body });

test("delivery's controls: you pause, grant, revoke and allow; an actor's token or an agent can't", async () => {
  const policy = new DeliveryPolicy();
  let person = { ok: true };
  const route = deliveryRoutes({ policy, router: { list: () => [] }, verify: { person: async () => person }, readJson: async (r) => r.body, send });
  assert.equal((await go(route, "POST", "/api/delivery/pause", { body: { paused: true } })).body.paused, true);
  assert.equal(policy.paused, true);
  await go(route, "POST", "/api/delivery/grant", { body: { actor: "actor:wren", scope: "project:atlas", hours: 2 } });
  assert.equal(policy.grants()[0].selector, "project:atlas");
  await go(route, "POST", "/api/delivery/allow", { body: { session: ME, hours: 1 } });
  assert.ok(policy.consented(ME));
  assert.equal((await go(route, "POST", "/api/delivery/grant", { body: { actor: "actor:wren", scope: "fix the bug" } })).status, 400);
  assert.equal((await go(route, "POST", "/api/delivery/pause", { headers: { authorization: "Bearer x" } })).status, 403, "never an actor");
  person = { ok: false, reason: "this is yours to do" };
  assert.equal((await go(route, "POST", "/api/delivery/pause", { body: { paused: false } })).status, 403, "never an agent");
  assert.equal(policy.paused, true);
  const state = (await go(route, "GET", "/api/delivery")).body;
  assert.equal(state.paused, true);
  assert.deepEqual(state.audit.map((a) => a.kind), ["paused", "grant", "consent"]);
});

test("the channel operation: a Claude Code session's poll gets what's pushed; others and actors get none", async () => {
  const channels = new Channels();
  const calls = [];
  const engine = { touch: () => {}, peers: async (me, o) => (calls.push(o.scope), []) };
  const route = engineRoutes({ engine, connect: {}, verify: { check: async () => ({ ok: true, root: { pid: 1 } }) }, readJson: async (r) => r.body, send, channels });
  const pending = go(route, "POST", "/api/engine/channel", { headers: { "x-xenonet-agent": ME }, body: { waitSeconds: 5 } });
  await new Promise((r) => setTimeout(r, 10));
  assert.ok(channels.push(ME, { id: 1, text: "hi" }));
  assert.deepEqual((await pending).body, { items: [{ id: 1, text: "hi" }] });
  const codex = await go(route, "POST", "/api/engine/channel", { headers: { "x-xenonet-agent": "codex:bbbbbbbb-0002" } });
  assert.deepEqual(codex.body, { items: [], off: true });
  assert.equal((await go(route, "POST", "/api/engine/channel", { headers: { authorization: "Bearer x" } })).status, 404);
  await go(route, "POST", "/api/engine/peers", { headers: { "x-xenonet-agent": ME }, body: { scope: "app:opencode status:idle" } });
  await go(route, "POST", "/api/engine/peers", { headers: { "x-xenonet-agent": ME }, body: { scope: "nonsense here" } });
  assert.deepEqual(calls, ["app:opencode status:idle", "repo"]);
});
