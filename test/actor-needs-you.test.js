// XenoNet Dev's made-up wait on you (simulate actor-needs-you, in
// core/actors): one of the demo's agents waits on you with a made-up
// question, which reaches the attention strip as its card (actorNeeds,
// src/actors-model.js), until clear takes it away. Outside the demo,
// nothing is made to wait. The actors are made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { EngineStore } from "../core/engine/store.js";
import { actorNeeds } from "../src/actors-model.js";
import { check, explain } from "../core/schema/index.js";

const T = 1791230000000;

function cast({ demo = true } = {}) {
  const clock = { t: T };
  const actors = new Actors({ store: new EngineStore(":memory:"), demo, now: () => clock.t });
  actors.upsert({ id: "actor:wren", name: "Wren", via: demo ? ["demo", "observed"] : ["observed"], heartbeatMs: 60000, at: T });
  actors.report("actor:wren", { instance: "gateway", presence: "working", at: T });
  actors.upsert({ id: "actor:moth", name: "Moth", via: demo ? ["demo", "registered"] : ["registered"], heartbeatMs: 30000, at: T });
  actors.report("actor:moth", { instance: "main", presence: "idle", at: T });
  return { actors, clock };
}

test("actor-needs-you: an agent of the demo waits on you, its card in the strip, and clear takes it away", () => {
  const { actors, clock } = cast();
  clock.t += 5000;
  assert.deepEqual(actors.simulate("actor-needs-you"), { ok: true, kind: "actor-needs-you", a: "actor:wren" }, "the first agent by default");
  const wren = actors.snapshot("actor:wren");
  assert.deepEqual(wren.needsYou, { kind: "question", text: "Should the refunds fix go out tonight, or wait for the review?", since: T + 5000 });
  const r = check("actor/snapshot/v1", wren, { strict: true });
  assert.ok(r.ok, explain(r));
  assert.deepEqual([wren.presence, wren.lastSeen], ["working", T], "as present as it was, and no heartbeat made up");
  assert.deepEqual(actorNeeds(actors.list()).map((n) => [n.id, n.name, n.sub]), [["actor:wren", "Wren", "asks"]], "a card in the strip");
  assert.equal(actors.simulate("actor-needs-you", { a: "actor:moth" }).a, "actor:moth", "any agent named");
  assert.equal(actorNeeds(actors.list()).length, 2);
  assert.equal(actors.simulate("actor-needs-you", { a: "actor:you" }), null, "never you");
  assert.equal(actors.simulate("actor-needs-you", { a: "actor:nobody" }), null);

  assert.deepEqual(actors.simulate("clear"), { ok: true, kind: "clear" });
  assert.deepEqual(actorNeeds(actors.list()), [], "clear takes every card away");
  assert.equal(actors.snapshot("actor:wren").needsYou, undefined);
});

test("actor-needs-you outside the demo: nothing is made to wait, and clear leaves real waits alone", () => {
  const { actors } = cast({ demo: false });
  assert.equal(actors.simulate("actor-needs-you", { a: "actor:wren" }), null);
  assert.equal(actors.snapshot("actor:wren").needsYou, undefined);
  // One that really waits (a registered actor's report) stays waiting.
  actors.report("actor:moth", { instance: "main", needsYou: { kind: "permission", text: "May I message the team?" } });
  assert.equal(actors.simulate("clear"), null);
  assert.equal(actors.snapshot("actor:moth").needsYou.kind, "permission");
});

test("actor-tunnel: a demo agent reaches XenoNet through a made-up tunnel, open or shut, which connections can't undo and clear takes away", () => {
  const { actors } = cast();
  assert.deepEqual(actors.simulate("actor-tunnel"), { ok: true, kind: "actor-tunnel", a: "actor:wren" });
  const wren = actors.snapshot("actor:wren");
  assert.deepEqual(wren.remote, { connections: 1, active: true, bridge: "connected", accountLink: "unverified", permissions: ["read"], transports: ["openai-secure-mcp-tunnel"] });
  const r = check("actor/snapshot/v1", wren, { strict: true });
  assert.ok(r.ok, explain(r));
  actors.setRemote("actor:wren", null);
  assert.equal(actors.snapshot("actor:wren").remote.bridge, "connected", "the connections' sync has none for it, and leaves it be");
  actors.simulate("actor-tunnel-down", { a: "actor:wren" });
  assert.equal(actors.snapshot("actor:wren").remote.bridge, "not_connected");
  assert.equal(actors.simulate("actor-tunnel", { a: "actor:you" }), null, "never you");
  actors.simulate("clear");
  assert.equal(actors.snapshot("actor:wren").remote, undefined);
  actors.setRemote("actor:wren", { active: true, connections: 1, transports: ["local-stdio"] });
  assert.deepEqual(actors.snapshot("actor:wren").remote.transports, ["local-stdio"], "after clear, the connections speak for it again");
  assert.equal(cast({ demo: false }).actors.simulate("actor-tunnel", { a: "actor:wren" }), null, "outside the demo, nothing");
});
