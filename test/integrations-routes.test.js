import { test } from "node:test";
import assert from "node:assert/strict";
import { integrationRoutes } from "../server/routes/integrations.js";
import { IntegrationError } from "../core/integrations/common.js";

function fixture() {
  const calls = [];
  const fn = (name) => (...args) => { calls.push([name, ...args]); return { name }; };
  const integrations = { status: fn("status"), history: fn("history"), pi: fn("pi"), enablePi: fn("enablePi"), importPi: fn("importPi"), routing: { preview: fn("routePreview"), route: fn("route"), outcome: fn("outcome") }, shell: { status: fn("shellStatus"), preview: fn("shellPreview"), launch: fn("launch"), inspect: fn("inspect"), remove: fn("delete"), review: fn("review"), retrieve: fn("retrieve"), exportReview: fn("export"), forgetReview: fn("forget") } };
  async function call(route, { method = "GET", body = {}, owner = true, yours = true, headers = {}, bodyError = null } = {}) {
    let result = null, reads = 0, ownerChecks = 0;
    const handled = await integrationRoutes({ req: { method, headers }, res: {}, url: new URL(route, "http://localhost"), integrations, yours: async (_, res) => { if (!yours) result = { status: 403, body: {} }; return yours; }, owner: async () => { ownerChecks++; if (!owner) result = { status: 403, body: {} }; return owner; }, readJson: async (_, limit) => { reads++; if (bodyError) throw bodyError; return body; }, send: (_, status, body) => { result = { status, body }; } });
    return { handled, result, reads, ownerChecks };
  }
  return { call, calls, integrations };
}
test("every integration read requires the owner, and bearer actors cannot read or write", async () => {
  const { call, calls } = fixture();
  for (const route of ["/api/integrations", "/api/integrations/history", "/api/integrations/pi", "/api/integrations/sandbox/review?id=job"]) {
    assert.equal((await call(route, { yours: false })).result.status, 403);
    assert.equal((await call(route, { headers: { authorization: "Bearer actor" } })).result.status, 403);
  }
  assert.equal(calls.length, 0);
  assert.equal((await call("/different")).handled, false);
});
test("every mutation requires the verified window before the body is read or a backend is called", async () => {
  const { call, calls } = fixture();
  for (const suffix of ["route/preview", "route/send", "route/outcome", "sandbox/check", "sandbox/preview", "sandbox/launch", "sandbox/inspect", "sandbox/delete", "sandbox/retrieve", "sandbox/export", "sandbox/forget-review", "pi/enable", "pi/import"]) {
    const denied = await call(`/api/integrations/${suffix}`, { method: "POST", owner: false });
    assert.equal(denied.result.status, 403); assert.equal(denied.reads, 0);
  }
  assert.equal(calls.length, 0);
});
test("cached review reads and privileged retrieve/export/forget dispatch through the same owner gates", async () => {
  const { call, calls } = fixture();
  assert.equal((await call("/api/integrations/sandbox/review?id=job")).result.body.name, "review");
  assert.deepEqual(calls.at(-1), ["review", "job"]);
  for (const [route, name] of [["retrieve", "retrieve"], ["export", "export"], ["forget-review", "forget"]]) {
    assert.equal((await call(`/api/integrations/sandbox/${route}`, { headers: { authorization: "Bearer actor" }, method: "POST" })).result.status, 403);
    assert.equal((await call(`/api/integrations/sandbox/${route}`, { method: "POST", body: { id: "job", revision: "r" } })).result.body.name, name);
    assert.deepEqual(calls.at(-1), name === "export" ? [name, "job", "r"] : [name, "job"]);
  }
});
test("the complete route surface dispatches documented bodies and never executes on GET", async () => {
  const { call, calls } = fixture();
  for (const [suffix, name, body] of [["route/preview", "routePreview", { request: "task" }], ["route/send", "route", { ticket: "t" }], ["route/outcome", "outcome", { id: "d", outcome: "useful" }], ["sandbox/check", "shellStatus", {}], ["sandbox/preview", "shellPreview", {}], ["sandbox/launch", "launch", { ticket: "t" }], ["sandbox/inspect", "inspect", { id: "d" }], ["sandbox/delete", "delete", { id: "d" }], ["pi/enable", "enablePi", { on: true }], ["pi/import", "importPi", { run: {} }]]) {
    assert.equal((await call(`/api/integrations/${suffix}`)).result.status, 404);
    const result = await call(`/api/integrations/${suffix}`, { method: "POST", body });
    assert.equal(result.result.body.name, name); assert.equal(result.reads, 1); assert.equal(result.ownerChecks, 1);
  }
  assert.equal(calls.length, 10);
  assert.equal((await call("/api/integrations/history?q=test&kind=decision")).result.status, 200);
  assert.deepEqual(calls.at(-1), ["history", { q: "test", kind: "decision" }]);
});
test("integration errors expose controlled messages; unexpected backend errors omit secrets", async () => {
  const f = fixture(); f.integrations.routing.route = () => { throw new IntegrationError(409, "expired", "Preview again."); };
  assert.deepEqual((await f.call("/api/integrations/route/send", { method: "POST" })).result, { status: 409, body: { error: "Preview again.", code: "expired" } });
  f.integrations.importPi = () => { throw new Error("PRIVATE_TOKEN"); };
  const failed = await f.call("/api/integrations/pi/import", { method: "POST" });
  assert.equal(failed.result.status, 502); assert.equal(JSON.stringify(failed).includes("PRIVATE_TOKEN"), false);
});
test("invalid JSON and oversized imports preserve body-validation HTTP status", async () => {
  const f = fixture();
  for (const [status, message] of [[400, "the body isn't JSON"], [413, "the body is too big"]]) {
    const result = await f.call("/api/integrations/pi/import", { method: "POST", bodyError: Object.assign(new Error(message), { status }) });
    assert.equal(result.result.status, status); assert.equal(f.calls.length, 0);
  }
});
