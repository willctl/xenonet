import { test } from "node:test";
import assert from "node:assert/strict";
import { engineRoutes } from "../server/routes/engine.js";

const ME = "claude:11111111-2222-3333-4444-555555555555";

// The routes with stand-ins for the engine, connecting agents and the
// checks on who's asking; `calls` records what each was asked.
function routes({ check = { ok: true, root: { pid: 4242 } }, person = { ok: true }, hookOut = null } = {}) {
  const calls = [];
  const note = (name, value = null) => (...args) => (calls.push([name, ...args]), value);
  const engine = {
    hook: note("hook", hookOut),
    snapshot: note("snapshot", { agents: [] }),
    agents: note("agents", [{ id: ME }]),
    whoHas: note("whoHas", []),
    whoami: note("whoami", { you: ME }),
    peers: note("peers", []),
    send: note("send", { id: 1 }),
    inbox: note("inbox", []),
    claim: note("claim", { claimed: [] }),
    release: note("release", { released: 0 }),
    setFocus: note("setFocus", { focus: "x" }),
    touch: note("touch"),
  };
  const connect = { preview: note("preview", { apps: [] }), apply: note("apply", { applied: true }), remove: note("remove", { removed: true }) };
  const verify = { check: note("check", check), person: note("person", person) };
  const route = engineRoutes({ engine, connect, verify, readJson: async (req) => req.body ?? {}, send: (res, status, body) => Object.assign(res, { status, body }) });
  const go = async (method, path, { body, headers = {} } = {}) => {
    const res = { writeHead: (status) => Object.assign(res, { status }) && { end: () => {} } };
    await route({ method, headers, body, socket: {} }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  return { go, calls, was: (name) => calls.filter((c) => c[0] === name) };
}

test("Codex's hooks go to the engine, checked against the asking process", async () => {
  let r = routes();
  assert.equal((await r.go("POST", "/api/hook/codex", { body: {} })).status, 204, "nothing to say");
  assert.deepEqual(r.was("check"), [], "no session, nothing to check");
  r = routes({ hookOut: { hookSpecificOutput: { additionalContext: "hi" } } });
  const res = await r.go("POST", "/api/hook/codex", { body: { session_id: "t1", hook_event_name: "SessionStart" } });
  assert.deepEqual([res.status, res.body.hookSpecificOutput.additionalContext], [200, "hi"]);
  assert.equal(r.was("check")[0][2], "codex:t1");
  assert.deepEqual(r.was("hook")[0].slice(1, 3), ["codex", "SessionStart"]);
  r = routes({ check: { ok: false, reason: "that's another process's session" } });
  const refused = await r.go("POST", "/api/hook/codex", { body: { session_id: "t1" } });
  assert.deepEqual([refused.status, refused.body.error], [403, "that's another process's session"]);
  assert.deepEqual(r.was("hook"), []);
});

test("the console's and the xenonet command's reads", async () => {
  const r = routes();
  assert.deepEqual((await r.go("GET", "/api/engine/state")).body, { agents: [] });
  assert.deepEqual((await r.go("GET", "/api/engine/agents")).body, [{ id: ME }]);
  assert.equal((await r.go("GET", "/api/engine/who?path=src/a.js")).status, 400, "a relative path");
  assert.equal((await r.go("GET", "/api/engine/who?path=/repo/src/a.js")).status, 200);
  assert.deepEqual(r.was("whoHas")[0].slice(1), [null, "/repo/src/a.js"]);
  assert.equal((await r.go("GET", "/api/engine/whoami")).status, 404, "agents' calls are posts");
  assert.equal((await r.go("POST", "/api/engine/nope", { headers: { "x-xenonet-agent": ME } })).status, 404);
});

test("connecting agents is for people only", async () => {
  let r = routes();
  assert.deepEqual((await r.go("GET", "/api/engine/connect")).body, { apps: [] });
  assert.deepEqual((await r.go("POST", "/api/engine/connect", { body: { apps: ["claude"] } })).body, { applied: true });
  assert.deepEqual(r.was("apply")[0].slice(1), [["claude"]]);
  assert.deepEqual((await r.go("DELETE", "/api/engine/connect")).body, { removed: true });
  r = routes({ person: { ok: false, reason: "an agent is asking" } });
  const refused = await r.go("POST", "/api/engine/connect", { body: { apps: ["claude"] } });
  assert.deepEqual([refused.status, refused.body.error], [403, "an agent is asking"]);
  assert.deepEqual(r.was("apply"), []);
});

test("an agent's calls say who it is, are checked, and note that it's in touch", async () => {
  let r = routes();
  assert.equal((await r.go("POST", "/api/engine/whoami", { body: {} })).status, 400, "no agent named");
  assert.equal((await r.go("POST", "/api/engine/whoami", { headers: { "x-xenonet-agent": "someone" } })).status, 400);
  const ok = await r.go("POST", "/api/engine/whoami", { headers: { "x-xenonet-agent": ME }, body: { cwd: "/repo", pid: 7 } });
  assert.deepEqual(ok.body, { you: ME });
  assert.deepEqual(r.was("touch")[0], ["touch", ME, { app: "claude", cwd: "/repo", via: "mcp", pid: 4242 }], "the pid XenoNet saw, not the one claimed");

  r = routes({ check: { ok: true } });
  await r.go("POST", "/api/engine/focus", { headers: { "x-xenonet-agent": ME }, body: { text: "tests", cwd: 3, pid: "7" } });
  assert.deepEqual(r.was("touch")[0][2], { app: "claude", cwd: null, via: "mcp", pid: 7 });
  assert.deepEqual(r.was("setFocus")[0].slice(1), [ME, "tests"]);

  r = routes({ check: { ok: false, reason: "not you" } });
  const refused = await r.go("POST", "/api/engine/send", { headers: { "x-xenonet-agent": ME }, body: { to: "all", text: "hi" } });
  assert.deepEqual([refused.status, refused.body.error], [403, "not you"]);
  assert.deepEqual(r.was("send"), []);
});

test("each of an agent's tools reaches the engine with what it needs", async () => {
  const r = routes();
  const as = (op, body) => r.go("POST", `/api/engine/${op}`, { headers: { "x-xenonet-agent": ME }, body });
  await as("peers", { scope: "machine" });
  await as("peers", { scope: "galaxy" });
  await as("who-has", { path: "src/a.js" });
  await as("send", { to: "repo", text: "hi", replyTo: 3 });
  await as("inbox", { waitSeconds: 500 });
  await as("claim", { paths: ["src/"], note: "refactor", minutes: 30 });
  await as("release", { paths: [] });
  await as("release", { paths: ["src/"] });
  assert.deepEqual(r.was("peers").map((c) => c[2]), [{ scope: "machine" }, { scope: "repo" }]);
  assert.deepEqual(r.was("whoHas")[0].slice(1), [ME, "src/a.js"]);
  assert.deepEqual(r.was("send")[0][2], { to: "repo", text: "hi", replyTo: 3 });
  assert.deepEqual(r.was("inbox")[0][2], { waitMs: 120000 }, "two minutes at most");
  assert.deepEqual(r.was("claim")[0][2], { paths: ["src/"], note: "refactor", minutes: 30 });
  assert.deepEqual(r.was("release").map((c) => c[2]), [{ paths: null }, { paths: ["src/"] }]);
});

test("an error from the engine is the caller's to read", async () => {
  const r = routes();
  const route = engineRoutes({
    engine: { touch() {}, send: async () => { throw new Error("the message is empty"); } },
    connect: {},
    verify: { check: async () => ({ ok: true }) },
    readJson: async () => ({}),
    send: (res, status, body) => Object.assign(res, { status, body }),
  });
  const res = {};
  await route({ method: "POST", headers: { "x-xenonet-agent": ME }, socket: {} }, res, new URL("/api/engine/send", "http://127.0.0.1"));
  assert.deepEqual([res.status, res.body.error], [400, "the message is empty"]);
  assert.equal(r.calls.length, 0);
});
