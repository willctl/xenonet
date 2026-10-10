// The projects folder over the API (server/routes/workspace.js): anyone
// may look and check, and changing it is yours (verify.person, stood in for
// here; test/verify.test.js has the real one on made-up process tables).
// A made-up state stands in for XenoNet's.
import { test } from "node:test";
import assert from "node:assert/strict";
import { workspaceRoutes } from "../server/routes/workspace.js";

function setup() {
  const asked = [];
  const verdict = { ok: true, checked: true };
  const state = {
    rootInfo: () => ({ path: "/Users/nova/Projects", source: "default" }),
    setRoot: async (p) => (asked.push(["setRoot", p]), { path: p ?? "/Users/nova/Projects", changed: true }),
    checkRoot: async (p) => (asked.push(["checkRoot", p]), { ok: true, path: p }),
  };
  const verify = { person: async () => (asked.push(["person"]), verdict) };
  const readJson = async (req) => (asked.push(["body"]), req.body ?? {});
  const route = workspaceRoutes({ state, verify, readJson, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (method, path, body) => {
    const res = {};
    await route({ method, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  return { go, asked, verdict };
}

test("changing the projects folder is yours: refused from under an agent before its body is read, and nothing changes", async () => {
  const r = setup();
  Object.assign(r.verdict, { ok: false, reason: "this is yours to do, and it came from under a Claude Code process; run it in your own terminal or in XenoNet" });
  for (const [method, body] of [["POST", { path: "~/Clients" }], ["DELETE"]]) {
    const res = await r.go(method, "/api/workspace/root", body);
    assert.deepEqual([res.status, res.body], [403, { error: r.verdict.reason, code: "yours" }], method);
  }
  assert.deepEqual(r.asked, [["person"], ["person"]], "no body read, no folder changed");
  // Looking and checking are anyone's.
  assert.equal((await r.go("GET", "/api/workspace/root")).status, 200);
  assert.deepEqual((await r.go("POST", "/api/workspace/check", { path: "~/Clients" })).body, { ok: true, path: "~/Clients" });
  assert.equal(r.asked.filter(([what]) => what === "person").length, 2);
});

test("from your terminal or XenoNet's window, the folder changes, and goes back", async () => {
  const r = setup();
  assert.deepEqual((await r.go("POST", "/api/workspace/root", { path: "~/Clients" })).body, { path: "~/Clients", changed: true });
  assert.equal((await r.go("DELETE", "/api/workspace/root")).status, 200);
  assert.equal((await r.go("POST", "/api/workspace/root", {})).status, 400, "no folder named");
  assert.deepEqual(r.asked.filter(([what]) => what === "setRoot"), [["setRoot", "~/Clients"], ["setRoot", null]]);
});
