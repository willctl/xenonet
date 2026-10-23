// The hosts routes (server/routes/hosts.js) with stand-ins for hosts.json,
// the hub and the check on who's asking: being a host needs no flag,
// showing hosts does, changes are yours alone, and remote sessions are
// read through their host's relay with nothing else let through.
import { test } from "node:test";
import assert from "node:assert/strict";
import { hostRoutes } from "../server/routes/hosts.js";

const S = "claude:0190d3e0-0000-7000-8000-000000000001";

function routes({ person = { ok: true }, hub = true, answers = {} } = {}) {
  const calls = [];
  const note = (name, value) => (...args) => (calls.push([name, ...args]), typeof value === "function" ? value(...args) : value);
  const hosts = { self: note("self", { id: "studio", key: "7c1e0000000000000000000000000001", createdAt: 1 }), setSelf: note("setSelf", (id) => ({ id, key: "7c1e0000000000000000000000000001", createdAt: 1 })), sshConfig: "/Users/nova/Library/Application Support/XenoNet/ssh_config" };
  const fake = hub && {
    has: (id) => id === "vm1",
    list: note("list", [{ id: "vm1", state: "live" }]),
    add: note("add", (body) => ({ id: body.id, state: "connecting" })),
    detail: note("detail", (id) => ({ id, engine: null })),
    remove: note("remove", (id, opts) => ({ id, removed: true, ...opts })),
    reconnect: note("reconnect", (id) => ({ id, state: "connecting" })),
    test: note("test", async (id) => ({ id, ok: true })),
    get: note("get", async (id, path) => answers[path] ?? { status: 200, body: { id: S, path } }),
  };
  const verify = { person: note("person", person) };
  // The window's own requests carry its key (x-xenonet-window), which
  // server/index.js checks; here, a header saying so.
  const windowOnly = (req) => req.headers?.window === true;
  const route = hostRoutes({ hosts, hub: fake, verify, readJson: async (req) => req.body ?? {}, send: (res, status, body) => Object.assign(res, { status, body }), home: "/Users/nova", windowOnly });
  const go = async (method, path, body, window = true) => {
    const res = {};
    const handled = await route({ method, body, socket: {}, headers: { window } }, res, new URL(path, "http://127.0.0.1"));
    return { handled, ...res };
  };
  return { go, calls, was: (name) => calls.filter((c) => c[0] === name).map((c) => c.slice(1)) };
}

test("this XenoNet as a host: who it is for anyone here, naming it yours alone, and no flag needed", async () => {
  for (const hub of [true, false]) {
    const r = routes({ hub });
    assert.deepEqual((await r.go("GET", "/api/hosts/self")).body, { id: "studio", key: "7c1e0000000000000000000000000001", createdAt: 1, home: "/Users/nova" });
    assert.deepEqual((await r.go("POST", "/api/hosts/self", { id: "lab" })).body.id, "lab");
    assert.equal((await r.go("POST", "/api/hosts/self", {})).status, 400);
  }
  const refused = routes({ person: { ok: false, reason: "this is yours to do, and it came from under a Claude Code process" } });
  const res = await refused.go("POST", "/api/hosts/self", { id: "lab" });
  assert.deepEqual([res.status, res.body.error], [403, "this is yours to do, and it came from under a Claude Code process"]);
  assert.deepEqual(refused.was("setSelf"), []);
});

test("without the flag, showing hosts answers 404, and remote ids are left to the routes that don't know them", async () => {
  const r = routes({ hub: false });
  for (const [m, p] of [["GET", "/api/hosts"], ["POST", "/api/hosts"], ["GET", "/api/hosts/vm1"], ["DELETE", "/api/hosts/vm1"], ["POST", "/api/hosts/vm1/test"]]) {
    const res = await r.go(m, p, { id: "vm1" });
    assert.equal(res.status, 404, `${m} ${p}`);
    assert.match(res.body.error, /XENONET_HOSTS=1/);
  }
  assert.equal((await r.go("GET", `/api/session/${encodeURIComponent(`@vm1:${S}`)}/transcript`)).handled, false, "the server's own route says it doesn't know the session");
  assert.equal((await r.go("GET", "/api/graph")).handled, false);
});

test("adding and removing are yours; the rest of what's on the list", async () => {
  const r = routes();
  assert.deepEqual((await r.go("GET", "/api/hosts")).body, { self: { id: "studio", key: "7c1e0000000000000000000000000001", createdAt: 1 }, sshConfig: "/Users/nova/Library/Application Support/XenoNet/ssh_config", hosts: [{ id: "vm1", state: "live" }] });
  assert.deepEqual((await r.go("POST", "/api/hosts", { id: "vm2", target: "lab@vm2.local" })).body, { id: "vm2", state: "connecting" });
  assert.deepEqual(r.was("add")[0], [{ id: "vm2", target: "lab@vm2.local" }, { replace: false }]);
  await r.go("POST", "/api/hosts", { id: "vm2", replace: true });
  assert.deepEqual(r.was("add")[1][1], { replace: true });
  assert.deepEqual((await r.go("DELETE", "/api/hosts/vm1?forget=1")).body, { id: "vm1", removed: true, forget: true });
  assert.deepEqual((await r.go("POST", "/api/hosts/vm1/reconnect")).body, { id: "vm1", state: "connecting" });
  assert.deepEqual((await r.go("POST", "/api/hosts/vm1/test")).body, { id: "vm1", ok: true });
  assert.equal((await r.go("POST", "/api/hosts/vm9/test")).status, 404);
  assert.deepEqual((await r.go("GET", "/api/hosts/vm1")).body, { id: "vm1", engine: null });
  assert.equal((await r.go("GET", "/api/hosts/Not-A-Host")).status, 404);
  assert.equal((await r.go("PUT", "/api/hosts/vm1")).status, 405);

  const refused = routes({ person: { ok: false, reason: "yours to do" } });
  assert.equal((await refused.go("POST", "/api/hosts", { id: "vm2" })).status, 403);
  assert.equal((await refused.go("DELETE", "/api/hosts/vm1")).status, 403);
  assert.deepEqual([refused.was("add"), refused.was("remove")], [[], []]);
});

test("a remote session's transcript and stats come through its relay, as plain reads with checked queries", async () => {
  const r = routes();
  const id = `@vm1:${S}`;
  // Only the window reads another machine's: never an agent here.
  for (const path of [`/api/session/${encodeURIComponent(id)}/transcript`, `/api/session/${encodeURIComponent(id)}/stats`, `/api/collision?a=${encodeURIComponent(id)}&b=${encodeURIComponent("@vm1:claude:2")}`]) {
    const refused = await r.go("GET", path, undefined, false);
    assert.deepEqual([refused.status, refused.body.code], [403, "window"], path);
  }
  assert.equal(r.was("get").length, 0, "nothing reached a relay");
  const t = await r.go("GET", `/api/session/${encodeURIComponent(id)}/transcript?limit=50&cursor=abc_1`);
  assert.equal(t.status, 200);
  assert.deepEqual(r.was("get")[0], ["vm1", `/api/session/${S}/transcript?limit=50&cursor=abc_1`]);
  assert.deepEqual(t.body, { id, path: `/api/session/${S}/transcript?limit=50&cursor=abc_1`, host: "vm1" }, "its ids as this XenoNet writes them");
  await r.go("GET", `/api/session/${id}/stats?limit=9`);
  assert.deepEqual(r.was("get")[1], ["vm1", `/api/session/${S}/stats`], "stats take no query");
  for (const bad of [`/api/session/${encodeURIComponent(id)}/transcript?limit=all`, `/api/session/${encodeURIComponent(id)}/transcript?cursor=${encodeURIComponent("a b")}`]) assert.equal((await r.go("GET", bad)).status, 400, bad);
  for (const unknown of [`/api/session/${encodeURIComponent(`@vm9:${S}`)}/stats`, `/api/session/${encodeURIComponent("@vm1:project:~/x")}/stats`, `/api/session/${encodeURIComponent("@vm1:@vm2:claude:x")}/stats`, `/api/session/${encodeURIComponent("@vm1:claude:../../x")}/stats`]) assert.equal((await r.go("GET", unknown)).status, 404, unknown);
  assert.equal(r.was("get").length, 2, "nothing else reached a relay");
  assert.equal((await r.go("GET", `/api/session/${S}/stats`)).handled, false, "this machine's own sessions are the server's");

  const relayed = routes({ answers: { [`/api/collision?a=${S}&b=claude:2`]: { status: 200, body: { a: { id: S, title: "A" }, b: { id: "claude:2", title: "B" }, files: [] } } } });
  const c = await relayed.go("GET", `/api/collision?a=${encodeURIComponent(id)}&b=${encodeURIComponent("@vm1:claude:2")}`);
  assert.deepEqual(c.body, { a: { id, title: "A" }, b: { id: "@vm1:claude:2", title: "B" }, files: [], host: "vm1" });
  assert.equal((await relayed.go("GET", `/api/collision?a=${encodeURIComponent(id)}&b=${S}`)).status, 400, "across machines comes later");
  assert.equal((await relayed.go("GET", `/api/collision?a=${S}&b=claude:2`)).handled, false, "this machine's own");
});
