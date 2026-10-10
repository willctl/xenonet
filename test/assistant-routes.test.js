// Remote connections over the API (server/routes/assistant.js): what's
// yours from a terminal (listing, revoking, turning access off), what's
// the window's alone with its key (previewing, pairing, widening, turning
// access back on), and that an actor's token, or a process under an agent,
// does none of it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { world, ask, view, ATLAS, LEDGER, SECRET, S } from "./fixtures/assistant.js";
import { assistantRoutes } from "../server/routes/assistant.js";
import { createWindowKey } from "../core/actors/key.js";
import { createAudit } from "../core/assistant/audit.js";
import { check, explain } from "../core/schema/index.js";

function setup({ person = true } = {}) {
  const w = world();
  const key = createWindowKey();
  const nodes = new Map([
    [ATLAS, { id: ATLAS, kind: "project", name: "atlas-api" }],
    [LEDGER, { id: LEDGER, kind: "project", name: "ledger" }],
    [SECRET, { id: SECRET, kind: "project", name: "secret-plans" }],
    [S.waiting, { id: S.waiting, kind: "session", updatedAt: w.clock.now() }],
  ]);
  const state = { index: nodes, homeOf: () => ATLAS, graph: { identities: [{ id: "chatgpt:demo-workspace", kind: "chatgpt", name: null, hidden: false }] } };
  const verify = { person: async () => (person ? { ok: true } : { ok: false, reason: "this is yours to do, and an agent can't" }), window: async () => ({ ok: true }) };
  const route = assistantRoutes({ connections: w.connections, audit: createAudit({ store: w.store }), source: { view: () => view(w.clock.now()) }, state, windowKey: key, verify, readJson: async (req) => req.body ?? {}, send: (res, status, body) => Object.assign(res, { status, body }), version: "0.7.0-test", now: w.clock.now });
  const call = async (method, path, { body, window = false, token = false } = {}) => {
    const res = {};
    const headers = { ...(window ? { "x-xenonet-window": key.reveal() } : {}), ...(token ? { authorization: "Bearer xenonet_actor_" + "a".repeat(64) } : {}) };
    await route({ method, headers, socket: {}, body }, res, new URL(`http://127.0.0.1${path}`));
    return res;
  };
  return { ...w, call };
}

test("pairing, previewing, widening and turning access on are the window's alone", async () => {
  const t = setup();
  try {
    const body = ask();
    assert.equal((await t.call("POST", "/api/assistant/pair", { body })).status, 403, "not from a terminal");
    assert.equal((await t.call("POST", "/api/assistant/preview", { body: { projects: [ATLAS] } })).status, 403);
    const paired = await t.call("POST", "/api/assistant/pair", { body, window: true });
    assert.equal(paired.status, 201, JSON.stringify(paired.body));
    const r = check("actor/connection/v1", paired.body, { strict: true });
    assert.ok(r.ok, explain(r));
    const grant = paired.body.grant_id;
    assert.equal((await t.call("POST", `/api/assistant/${grant}`, { body: { projects: [ATLAS, SECRET] } })).status, 403, "widening needs the window");
    assert.equal((await t.call("POST", `/api/assistant/${grant}/rotate`)).status, 403);
    const widened = await t.call("POST", `/api/assistant/${grant}`, { body: { projects: [ATLAS, SECRET], labels: true }, window: true });
    assert.deepEqual([widened.status, widened.body.revision, widened.body.projects.length], [200, 2, 2]);
    // Off from a terminal; on again only from the window.
    assert.equal((await t.call("POST", "/api/assistant/disable")).status, 200);
    assert.equal((await t.call("POST", "/api/assistant/enable")).status, 403);
    assert.equal((await t.call("POST", "/api/assistant/enable", { window: true })).status, 200);
    assert.equal((await t.call("POST", `/api/assistant/${grant}/revoke`)).status, 200, "revoking is yours from a terminal");
  } finally {
    t.done();
  }
});

test("an actor's token, or a process under an agent, does nothing here", async () => {
  const t = setup({ person: false });
  try {
    for (const [method, path] of [["GET", "/api/assistant"], ["GET", "/api/assistant/places"], ["GET", "/api/assistant/audit"], ["POST", "/api/assistant/disable"]]) assert.equal((await t.call(method, path)).status, 403, `${method} ${path}`);
    for (const [method, path] of [["GET", "/api/assistant"], ["POST", "/api/assistant/pair"], ["POST", "/api/assistant/enable"]]) assert.equal((await t.call(method, path, { window: true, token: true })).status, 403, `${method} ${path} with a token`);
  } finally {
    t.done();
  }
});

test("the list, the places and the preview: what you see, and exactly what would leave", async () => {
  const t = setup();
  try {
    await t.call("POST", "/api/assistant/pair", { body: ask(), window: true });
    const list = await t.call("GET", "/api/assistant");
    assert.equal(list.status, 200);
    assert.equal(list.body.connections.length, 1);
    assert.deepEqual(list.body.products.map((p) => p.key), ["dots", "grok-bot", "hermes-agent", "custom"]);
    assert.deepEqual(Object.keys(list.body.diagnostics).sort(), ["assistant_api", "endpoint", "service_version", "sources"]);
    assert.ok(!JSON.stringify(list.body).includes("xenonet_assistant_"), "never a credential");
    const places = await t.call("GET", "/api/assistant/places");
    assert.deepEqual(places.body.places.map((p) => p.name), ["atlas-api", "ledger", "secret-plans"]);
    assert.deepEqual(places.body.identities, [{ id: "chatgpt:demo-workspace", kind: "chatgpt", name: null }]);
    const preview = await t.call("POST", "/api/assistant/preview", { body: { projects: [ATLAS, "project:/not-on-the-map"], labels: false }, window: true });
    assert.equal(preview.status, 200);
    assert.equal(preview.body.real.projects.length, 1, "only places on the map");
    assert.ok(preview.body.real.projects.every((p) => p.label === null));
    assert.ok(preview.body.real.sessions.every((s) => s.label === null && !("place" in s)));
    assert.ok(!JSON.stringify(preview.body.real).includes("/users/nova"), "no paths in what would leave");
    assert.equal(preview.body.yours[0].place, ATLAS, "which is which, for the window");
    assert.ok(preview.body.synthetic.projects.projects.length > 0);
    const audit = await t.call("GET", "/api/assistant/audit");
    assert.equal(audit.body.records[0].operation, "pair");
    assert.equal((await t.call("GET", "/api/assistant/audit?grant=nope")).status, 400);
  } finally {
    t.done();
  }
});
