// Registering and reporting over HTTP (server/routes/actors.js;
// docs/design/actors/interface.md, section 2): a program asks and waits,
// the window allows, adds and rotates with its key, you remove actors and
// switch observers, and an actor reports with its token, held to the
// process checks (stood in for here, but for the real check on a made-up
// machine at the end; test/verify.test.js has the rest), the limits and the
// errors' codes. Made-up actors, paths and processes throughout.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { createRegistration } from "../core/actors/registration.js";
import { createWindowKey } from "../core/actors/key.js";
import { EngineStore } from "../core/engine/store.js";
import { actorRoutes, createActorCaller, RATES } from "../server/routes/actors.js";
import { createVerifier } from "../core/engine/verify.js";
import { agentOf, productOf } from "../core/procs.js";
import { check, explain } from "../core/schema/index.js";
import { fromFile } from "./fixtures/schema-files.js";
import { fakeMap, HOME, ATLAS, LEDGER, ATLAS_DIR, LEDGER_DIR, S1 } from "./fixtures/actor-map.js";

const T = 1791230000000;
const ALL = ["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents"];
const CHAIN = [
  { pid: 430, ppid: 420, program: "/usr/local/bin/node" },
  { pid: 420, ppid: 400, program: `${HOME}/bin/wren` },
  { pid: 400, ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" },
];

function valid(id, data, what = id) {
  const d = JSON.parse(JSON.stringify(data));
  const r = check(id, d, { strict: true });
  assert.ok(r.ok, `${what} doesn't fit ${id}:\n${explain(r)}`);
  assert.ok(fromFile(id).safeParse(d).success, `${what} doesn't fit schemas/${id}.json`);
}

async function readJson(req, limit = 1 << 20) {
  if (req.body === "not json") throw Object.assign(new Error("the body isn't JSON"), { status: 400 });
  if (JSON.stringify(req.body ?? {}).length > limit) throw Object.assign(new Error("the body is too big"), { status: 413 });
  // What the client sent, null included; nothing at all reads as {}.
  return req.body === undefined ? {} : req.body;
}

// The routes on a clock the test moves, with the process checks stood in
// for: what each says is in `verdicts`, and `calls` records what each was
// asked.
function routes({ demo = false, off = false } = {}) {
  const clock = { t: T };
  const now = () => clock.t;
  const store = new EngineStore(":memory:");
  const actors = off ? null : new Actors({ store, now, map: fakeMap(), demo });
  const registration = off ? null : createRegistration({ actors, now, home: HOME });
  const key = createWindowKey();
  const calls = [];
  const verdicts = {};
  const verify = {
    window: async () => (calls.push(["window"]), verdicts.window ?? { ok: true, checked: true }),
    person: async () => (calls.push(["person"]), verdicts.person ?? { ok: true, checked: true }),
    actor: async (_socket, who, opts) => (calls.push(["actor", who, opts]), verdicts.actor ?? { ok: true, checked: true, chain: CHAIN }),
    register: async (_socket, opts) => (calls.push(["register", opts]), verdicts.register ?? { ok: true, checked: true, chain: CHAIN, cwd: `${HOME}/Projects/docs-site` }),
    asker: async () => (calls.push(["asker"]), verdicts.asker ?? { ok: true, checked: true, chain: CHAIN, cwd: `${HOME}/Projects/docs-site` }),
    peer: async (socket) => socket.pid ?? 4242,
    lineage: async () => null,
  };
  const caller = off ? null : createActorCaller({ registration, verify, now });
  const route = actorRoutes({ actors, registration, caller, windowKey: key, verify, readJson, send: (res, status, body, headers = {}) => Object.assign(res, { status, body, headers }), now });
  const go = async (method, path, { body, headers = {}, pid } = {}) => {
    const res = {};
    await route({ method, headers, body, socket: { pid } }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  const window = (extra = {}) => ({ "x-xenonet-window": key.reveal(), ...extra });
  const bearer = (token, extra = {}) => ({ authorization: `Bearer ${token}`, ...extra });
  // A program that asked, and that you allowed: its token.
  async function allowed(body = { name: "Wren", capabilities: ALL }) {
    const asked = await go("POST", "/api/actors/register", { body });
    assert.equal(asked.status, 202, JSON.stringify(asked.body));
    const { requests } = (await go("GET", "/api/actors/requests", { headers: window() })).body;
    const answer = await go("POST", `/api/actors/requests/${requests.find((r) => r.actor === asked.body.id).id}`, { body: { decision: "allow" }, headers: window() });
    assert.equal(answer.status, 200, JSON.stringify(answer.body));
    return asked.body.token;
  }
  return { go, window, bearer, allowed, actors, registration, caller, store, clock, calls, verdicts };
}
const codeOf = (res) => [res.status, res.body.code];

test("registering: 202 with a token that waits, after the process check, never with a token, and bodies checked", async () => {
  const r = routes();
  const res = await r.go("POST", "/api/actors/register", { body: { schema: "actor/registration/v1", name: "Wren", capabilities: ["run-commands"], pid: 420 } });
  assert.equal(res.status, 202);
  valid("actor/credentials/v1", res.body);
  assert.deepEqual([res.body.id, res.body.state], ["actor:wren", "waiting"]);
  assert.deepEqual(r.calls.find((c) => c[0] === "register")[1], { product: null, pid: 420 });
  const card = (await r.go("GET", "/api/actors/requests", { headers: r.window() })).body.requests[0];
  assert.deepEqual(card.asker, { program: "/usr/local/bin/node", parents: ["~/bin/wren", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: "~/Projects/docs-site" });

  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body: { name: "Kite" }, headers: r.bearer(res.body.token) })), [403, "not-allowed"], "an actor can't register actors");
  r.verdicts.register = { ok: false, checked: true, reason: "a session can't register an actor, only ask you to, and this came from under a Claude Code process" };
  const agent = await r.go("POST", "/api/actors/register", { body: { name: "Kite" } });
  assert.deepEqual([...codeOf(agent), agent.body.error], [403, "wrong-process", r.verdicts.register.reason]);
  delete r.verdicts.register;
  for (const body of [{}, { name: "" }, { name: "Kite", capabilities: "all" }, { name: "Kite", slug: "Kite!" }, { name: "Kite", pid: -1 }]) assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body })), [400, "bad-request"], JSON.stringify(body));
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body: "not json" })), [400, "bad-request"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body: { name: "Kite", pad: "x".repeat(5000) } })), [413, "too-big"]);
  const taken = await r.go("POST", "/api/actors/register", { body: { name: "Wren" }, headers: {} });
  assert.deepEqual(codeOf(taken), [202, undefined], "the same program again takes its old request's place");
  r.verdicts.register = { ok: true, checked: true, chain: [{ ...CHAIN[0], program: "/opt/homebrew/bin/python3" }, ...CHAIN.slice(1)], cwd: null };
  const other = await r.go("POST", "/api/actors/register", { body: { name: "Wren" } });
  assert.deepEqual([...codeOf(other), other.body.suggest], [409, "taken", "wren-2"], "another program can't take its slug");
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body: { name: "Dot", product: { id: "dots" } } })), [403, "not-allowed"]);
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/register")), [404, "not-found"]);
});

test("a token comes in the Authorization header only, and every refusal says why with a code", async () => {
  const r = routes();
  const none = await r.go("GET", "/api/actors/me");
  assert.deepEqual([...codeOf(none), none.headers["www-authenticate"]], [401, "no-token", 'Bearer realm="xenonet"']);
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/me", { headers: { authorization: "Basic d3Jlbjp4" } })), [401, "no-token"]);
  const unknown = await r.go("GET", "/api/actors/me", { headers: r.bearer(`xenonet_actor_${"0".repeat(64)}`) });
  assert.deepEqual([...codeOf(unknown), unknown.headers["www-authenticate"]], [401, "bad-token", 'Bearer realm="xenonet", error="invalid_token"']);
  const asked = (await r.go("POST", "/api/actors/register", { body: { name: "Wren" } })).body;
  assert.deepEqual(codeOf(await r.go("GET", `/api/actors/me?token=${asked.token}`)), [400, "bad-request"], "never in a URL");
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors?access_token=x")), [400, "bad-request"]);
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/me", { headers: r.bearer(asked.token, { "x-xenonet-agent": "claude:aaaaaaaa-0001" }) })), [400, "bad-request"], "an agent or an actor, never both");
  const [{ id }] = (await r.go("GET", "/api/actors/requests", { headers: r.window() })).body.requests;
  await r.go("POST", `/api/actors/requests/${id}`, { body: { decision: "deny" }, headers: r.window() });
  const denied = await r.go("GET", "/api/actors/me", { headers: r.bearer(asked.token) });
  assert.deepEqual([...codeOf(denied), denied.body.reason], [401, "revoked", "denied"]);
  for (const res of [none, unknown, denied]) assert.ok(!JSON.stringify(res).includes(asked.token), "a token is never in an answer");
});

test("while it waits, a token opens only the actor's own record", async () => {
  const r = routes();
  const asked = (await r.go("POST", "/api/actors/register", { body: { name: "Wren", heartbeatMs: 30000 } })).body;
  const me = await r.go("GET", "/api/actors/me", { headers: r.bearer(asked.token) });
  assert.equal(me.status, 200);
  valid("actor/me/v1", me.body);
  assert.deepEqual(me.body, { schema: "actor/me/v1", id: "actor:wren", state: "waiting", presence: "unknown", heartbeatMs: 30000, unread: 0 });
  for (const [method, path, body] of [["POST", "/api/actors/heartbeat", {}], ["POST", "/api/actors/events", { events: [] }], ["POST", "/api/actors/targets", { targets: [] }], ["DELETE", "/api/actors/me"]]) {
    const res = await r.go(method, path, { body, headers: r.bearer(asked.token) });
    assert.deepEqual(codeOf(res), [403, "waiting"], `${method} ${path}`);
  }
});

test("allowed: heartbeats, events and targets, each answered with what it is and what was taken", async () => {
  const r = routes();
  const token = await r.allowed();
  const headers = r.bearer(token);
  const hb = await r.go("POST", "/api/actors/heartbeat", { body: { schema: "actor/report/v1", instance: "gateway", presence: "working", status: "Rebuilding the docs", cwd: `${ATLAS_DIR}/docs`, pid: 4242 }, headers });
  assert.equal(hb.status, 200);
  valid("actor/me/v1", hb.body);
  assert.deepEqual(hb.body, { schema: "actor/me/v1", id: "actor:wren", state: "allowed", presence: "working", heartbeatMs: 60000, unread: 0 });
  assert.deepEqual(r.calls.filter((c) => c[0] === "actor").at(-1).slice(1), [{ slug: "wren", product: null }, { pid: 4242 }], "the pid it names is checked");
  assert.equal(r.actors.snapshot("actor:wren").status, "Rebuilding the docs");

  const ev = await r.go("POST", "/api/actors/events", { body: { schema: "actor/report/v1", events: [{ kind: "ran", path: ATLAS_DIR, count: 2 }, { kind: "ran", count: 0 }, { kind: "said" }, { kind: "edited", paths: [`${ATLAS_DIR}/src/a.ts`] }] }, headers });
  assert.equal(ev.status, 200);
  valid("actor/me/v1", ev.body);
  assert.deepEqual([ev.body.accepted, ev.body.dropped.map((d) => d.index)], [2, [1, 2]]);
  assert.match(ev.body.dropped[0].reason, /^that isn't an event/);
  assert.deepEqual(r.actors.snapshot("actor:wren").recent.filter((e) => e.kind !== "presence" && e.kind !== "status").map((e) => e.detail), ["Ran 2 commands in atlas-api", "Edited a.ts in atlas-api"]);

  const tg = await r.go("POST", "/api/actors/targets", { body: { targets: [{ path: LEDGER_DIR, relation: "watches" }, { id: "actor:moth" }, 7], remove: [{ id: ATLAS, relation: "edited" }] }, headers });
  assert.deepEqual([tg.status, tg.body.accepted, tg.body.dropped.map((d) => d.index)], [200, 1, [1, 2]]);
  assert.deepEqual(r.actors.targetsOf("actor:wren").map((t) => `${t.relation} ${t.id}`).sort(), [`watches ${LEDGER}`, `works-on ${ATLAS}`]);

  for (const [path, body, why] of [
    ["/api/actors/heartbeat", { status: `token ghp_${"x".repeat(30)}` }, /key or a token/],
    ["/api/actors/heartbeat", { presence: "away" }, /actor\/report\/v1/],
    ["/api/actors/heartbeat", { instance: "Gate way" }, /actor\/report\/v1/],
    ["/api/actors/heartbeat", { events: [] }, /POST \/api\/actors\/events/],
    ["/api/actors/heartbeat", { cwd: "docs" }, /absolute/],
    ["/api/actors/events", {}, /list of what it did/],
    ["/api/actors/targets", { targets: "all" }, /is a list/],
  ]) {
    r.clock.t += 2000;
    const res = await r.go("POST", path, { body, headers });
    assert.deepEqual(codeOf(res), [400, "bad-request"], `${path} ${JSON.stringify(body)}`);
    assert.match(res.body.error, why);
  }
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/events", { body: { events: Array.from({ length: 51 }, () => ({ kind: "ran" })) }, headers })), [413, "too-big"], "50 events a call");
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/heartbeat", { body: { pad: "x".repeat(5000) }, headers })), [413, "too-big"]);
});

test("the process check holds a token to where it may be used", async () => {
  const r = routes();
  const token = await r.allowed();
  r.verdicts.actor = { ok: false, checked: true, reason: "an actor's token doesn't work from under a session, and this came from under a Claude Code process" };
  for (const [method, path, body] of [["GET", "/api/actors/me"], ["POST", "/api/actors/heartbeat", {}], ["POST", "/api/actors/events", { events: [] }], ["POST", "/api/actors/targets", {}], ["DELETE", "/api/actors/me"]]) {
    const res = await r.go(method, path, { body, headers: r.bearer(token) });
    assert.deepEqual([...codeOf(res), res.body.error], [403, "wrong-process", r.verdicts.actor.reason], `${method} ${path}`);
  }
  assert.ok(r.actors.has("actor:wren"), "and it changed nothing");
});

test("rates: a heartbeat a second per instance, 60 events a minute in bursts of 120, 30 target calls a minute", async () => {
  const r = routes();
  const headers = r.bearer(await r.allowed());
  const beat = (instance = "main") => r.go("POST", "/api/actors/heartbeat", { body: { instance }, headers });
  assert.deepEqual([(await beat()).status, (await beat()).status], [200, 200]);
  const fast = await beat();
  assert.deepEqual([...codeOf(fast), fast.headers["retry-after"]], [429, "too-many", "1"]);
  assert.equal((await beat("cli")).status, 200, "another instance has its own");
  r.clock.t += 1000;
  assert.equal((await beat()).status, 200);

  const events = (n) => r.go("POST", "/api/actors/events", { body: { events: Array.from({ length: n }, () => ({ kind: "ran" })) }, headers });
  for (const n of [50, 50, 20]) assert.equal((await events(n)).status, 200);
  const flood = await events(1);
  assert.deepEqual([...codeOf(flood), flood.headers["retry-after"]], [429, "too-many", "1"]);
  r.clock.t += 10e3;
  assert.equal((await events(10)).status, 200, "ten seconds on, ten more");

  const target = () => r.go("POST", "/api/actors/targets", { body: { targets: [] }, headers });
  for (let i = 0; i < 10; i++) assert.equal((await target()).status, 200);
  assert.deepEqual(codeOf(await target()), [429, "too-many"]);
});

test("an actor's own instance names don't escape its rates: eight heartbeats a second at most, and its other buckets stay as they were", async () => {
  const r = routes();
  const headers = r.bearer(await r.allowed());
  const beat = (instance, extra = {}) => r.go("POST", "/api/actors/heartbeat", { body: { instance, ...extra }, headers });
  const codes = [];
  for (let i = 0; i < 20; i++) codes.push((await beat(`made-up-${i}`)).status);
  assert.deepEqual([codes.filter((c) => c === 200).length, codes.at(-1)], [16, 429], "a burst of 16, however many names");
  r.clock.t += 1000;
  const next = [];
  for (let i = 20; i < 30; i++) next.push((await beat(`made-up-${i}`)).status);
  assert.equal(next.filter((c) => c === 200).length, 8, "then eight a second");
  // Its messages to all (the engine's), spent, aren't given back by a
  // thousand new names pushing their bucket out.
  for (let i = 0; i < 6; i++) assert.equal(r.caller.take("a:wren|wide", RATES.wide), 0);
  assert.ok(r.caller.take("a:wren|wide", RATES.wide) > 0);
  for (let i = 0; i < 1100; i++) {
    r.clock.t += 130;
    assert.equal((await beat(`churn-${i}`)).status, 200, String(i));
  }
  assert.ok(r.caller.take("a:wren|wide", RATES.wide) > 0, "still spent");
});

test("a new status is an event: it counts against the actor's events, and the same status again doesn't", async () => {
  const r = routes();
  const headers = r.bearer(await r.allowed());
  const status = (text, instance = "main") => r.go("POST", "/api/actors/heartbeat", { body: { instance, status: text }, headers });
  for (const n of [50, 50, 19]) assert.equal((await r.go("POST", "/api/actors/events", { body: { events: Array.from({ length: n }, () => ({ kind: "ran" })) }, headers })).status, 200);
  assert.equal((await status("Rebuilding the docs")).status, 200, "the 120th");
  r.clock.t += 1000;
  assert.equal((await status("Rebuilding the docs")).status, 200, "the same again takes none");
  assert.equal((await r.go("POST", "/api/actors/events", { body: { events: [{ kind: "ran" }] }, headers })).status, 200, "so the second's event is still there");
  const flood = await status("Deploying to production", "cli");
  assert.deepEqual([...codeOf(flood), flood.headers["retry-after"]], [429, "too-many", "1"]);
  assert.match(flood.body.error, /a new status is an event/);
  assert.equal(r.actors.snapshot("actor:wren").status, "Rebuilding the docs");
});

test("a caller the process check refuses spends none of the actor's rates", async () => {
  const r = routes();
  const headers = r.bearer(await r.allowed());
  r.verdicts.actor = { ok: false, checked: true, reason: "an actor's token doesn't work from under a session, and this came from under a Claude Code process" };
  const fifty = { events: Array.from({ length: 50 }, () => ({ kind: "ran" })) };
  for (let i = 0; i < 3; i++) assert.deepEqual(codeOf(await r.go("POST", "/api/actors/events", { body: fifty, headers })), [403, "wrong-process"]);
  for (let i = 0; i < 4; i++) assert.deepEqual(codeOf(await r.go("POST", "/api/actors/heartbeat", { body: {}, headers })), [403, "wrong-process"]);
  for (let i = 0; i < 12; i++) assert.deepEqual(codeOf(await r.go("POST", "/api/actors/targets", { body: { targets: [] }, headers })), [403, "wrong-process"]);
  delete r.verdicts.actor;
  for (const n of [50, 50, 20]) assert.equal((await r.go("POST", "/api/actors/events", { body: { events: Array.from({ length: n }, () => ({ kind: "ran" })) }, headers })).status, 200, "Wren's own, all 120 of its burst");
  assert.equal((await r.go("POST", "/api/actors/heartbeat", { body: {}, headers })).status, 200);
  assert.equal((await r.go("POST", "/api/actors/targets", { body: { targets: [] }, headers })).status, 200);
});

test("bad tokens: 20 a minute from one process, then 429 for a minute, while good tokens still work", async () => {
  const r = routes();
  const good = r.bearer(await r.allowed());
  const bad = r.bearer(`xenonet_actor_${"f".repeat(64)}`);
  for (let i = 0; i < 20; i++) assert.deepEqual(codeOf(await r.go("GET", "/api/actors/me", { headers: bad, pid: 7000 })), [401, "bad-token"], String(i));
  const blocked = await r.go("GET", "/api/actors/me", { headers: bad, pid: 7000 });
  assert.deepEqual([...codeOf(blocked), blocked.headers["retry-after"]], [429, "too-many", "60"]);
  assert.equal((await r.go("GET", "/api/actors/me", { headers: good, pid: 7000 })).status, 200, "a good token isn't blocked");
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/me", { headers: bad, pid: 7001 })), [401, "bad-token"], "another process has its own count");
  r.clock.t += 61e3;
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/me", { headers: bad, pid: 7000 })), [401, "bad-token"], "a minute on");
});

test("leaving revokes its token and takes its record", async () => {
  const r = routes();
  const token = await r.allowed();
  const out = await r.go("DELETE", "/api/actors/me", { headers: r.bearer(token) });
  assert.deepEqual([out.status, out.body], [200, { removed: "actor:wren" }]);
  assert.equal(r.actors.has("actor:wren"), false);
  const after = await r.go("GET", "/api/actors/me", { headers: r.bearer(token) });
  assert.deepEqual([...codeOf(after), after.body.reason], [401, "revoked", "left"]);
});

test("a body that isn't a JSON object is a 400, not a crash: Allow and Deny, observers, and an actor's reports", async () => {
  const r = routes();
  const token = await r.allowed();
  const kite = (await r.go("POST", "/api/actors/register", { body: { name: "Kite" } })).body;
  const waiting = () => r.go("GET", "/api/actors/requests", { headers: r.window() }).then((res) => res.body.requests);
  const id = (await waiting()).find((q) => q.actor === kite.id).id;
  for (const body of [null, [], 7, "allow"]) {
    const what = JSON.stringify(body);
    assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${id}`, { body, headers: r.window() })), [400, "bad-request"], `an answer of ${what}`);
    assert.deepEqual(codeOf(await r.go("POST", "/api/actors/observers", { body })), [400, "bad-request"], `observers ${what}`);
    for (const p of ["heartbeat", "events", "targets"]) assert.deepEqual(codeOf(await r.go("POST", `/api/actors/${p}`, { body, headers: r.bearer(token) })), [400, "bad-request"], `${p} ${what}`);
  }
  assert.deepEqual((await waiting()).map((q) => q.actor), ["actor:kite"], "the registration still waits for a real answer");
  assert.equal((await r.go("POST", "/api/actors/heartbeat", { body: { presence: "working" }, headers: r.bearer(token) })).status, 200, "and the actor's rates weren't spent");
});

test("the window's routes: requests, Allow and Deny, adding, a new token; refused without its key, with a token, or from elsewhere", async () => {
  const r = routes();
  await r.go("POST", "/api/actors/register", { body: { name: "Wren" } });
  await r.go("POST", "/api/actors/register", { body: { name: "Heron" } });
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/requests")), [403, "yours"]);
  const list = await r.go("GET", "/api/actors/requests", { headers: r.window() });
  assert.deepEqual([list.status, list.body.requests.map((q) => q.actor)], [200, ["actor:wren", "actor:heron"]]);
  const [wren, heron] = list.body.requests;
  const yes = await r.go("POST", `/api/actors/requests/${wren.id}`, { body: { decision: "allow" }, headers: r.window() });
  assert.equal(yes.status, 200);
  valid("actor/snapshot/v1", yes.body.actor);
  assert.deepEqual((await r.go("POST", `/api/actors/requests/${heron.id}`, { body: { decision: "deny" }, headers: r.window() })).body, { request: heron.id, decision: "deny", actor: "actor:heron" });
  assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${heron.id}`, { body: { decision: "allow" }, headers: r.window() })), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/requests/%E0%A4%A", { body: { decision: "allow" }, headers: r.window() })), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${wren.id}`, { body: { decision: "allow" } })), [403, "yours"]);

  const added = await r.go("POST", "/api/actors", { body: { name: "Moth", capabilities: ["message-agents"] }, headers: r.window() });
  assert.equal(added.status, 201);
  valid("actor/credentials/v1", added.body);
  assert.deepEqual([added.body.state, added.body.id], ["allowed", "actor:moth"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors", { body: { name: "Kite" } })), [403, "yours"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors", { body: { name: "Kite" }, headers: r.window(r.bearer(added.body.token)) })), [403, "yours"], "never with a token, key or not");
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors", { body: { name: "" }, headers: r.window() })), [400, "bad-request"]);

  const fresh = await r.go("POST", "/api/actors/moth/token", { headers: r.window() });
  assert.equal(fresh.status, 200);
  valid("actor/credentials/v1", fresh.body);
  assert.deepEqual((await r.go("GET", "/api/actors/me", { headers: r.bearer(added.body.token) })).body.reason, "rotated");
  assert.equal((await r.go("GET", "/api/actors/me", { headers: r.bearer(fresh.body.token) })).status, 200);
  for (const slug of ["nobody", "you", "Moth", "%E0%A4%A"]) assert.deepEqual(codeOf(await r.go("POST", `/api/actors/${slug}/token`, { headers: r.window() })), [404, "not-found"], slug);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/moth/token")), [403, "yours"]);

  r.verdicts.window = { ok: false, checked: true, reason: "this is for XenoNet's own window, and it came from a program run in XenoNet" };
  const outside = await r.go("GET", "/api/actors/requests", { headers: r.window() });
  assert.deepEqual([...codeOf(outside), outside.body.error], [403, "yours", r.verdicts.window.reason], "the key isn't enough from the wrong process");
});

test("the registrations waiting for you answer an actor/requests/v1: what each asks, who asked, and when it lapses", async () => {
  const r = routes();
  await r.go("POST", "/api/actors/register", { body: { name: "Wren", capabilities: ["run-commands", "fly"], heartbeatMs: 30000, pid: 420 } });
  // A product of its own word, asked from where XenoNet couldn't look.
  r.verdicts.register = { ok: true, checked: false };
  await r.go("POST", "/api/actors/register", { body: { name: "Kite", product: { id: "kite-tools", vendor: "Made-up Co", version: "1.2" } } });
  const list = await r.go("GET", "/api/actors/requests", { headers: r.window() });
  valid("actor/requests/v1", list.body);
  assert.equal(list.body.schema, "actor/requests/v1", "it says what it is, as the other actor answers do");
  const [wren, kite] = list.body.requests;
  assert.deepEqual([wren.actor, wren.capabilities, wren.heartbeatMs, wren.pid, wren.joins, wren.expiresAt - wren.at], ["actor:wren", ["run-commands"], 30000, 420, false, 600e3]);
  assert.deepEqual(wren.asker, { program: "/usr/local/bin/node", parents: ["~/bin/wren", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: "~/Projects/docs-site" });
  assert.deepEqual([kite.product, kite.asker, kite.pid], [{ id: "kite-tools", vendor: "Made-up Co", version: "1.2" }, null, null]);
  valid("actor/requests/v1", (await r.go("GET", "/api/actors/requests", { headers: r.window() })).body);
  r.clock.t += 600e3;
  valid("actor/requests/v1", (await r.go("GET", "/api/actors/requests", { headers: r.window() })).body, "with none waiting");
});

test("a session's ask for a gateway token: any program may ask, nothing it gets back holds a token, and only the window's Allow makes one", async () => {
  const r = routes();
  await r.allowed({ name: "Wren", capabilities: ALL });
  const none = (res) => assert.ok(!/xenonet_(gateway|actor)_[0-9a-f]{8}/.test(JSON.stringify(res.body)), "no token in the answer");

  const asked = await r.go("POST", "/api/actors/wren/gateway/ask", { body: {} });
  assert.equal(asked.status, 202, JSON.stringify(asked.body));
  valid("actor/gateway-ask/v1", asked.body);
  assert.deepEqual([asked.body.id, asked.body.state, asked.body.replaces], ["actor:wren", "waiting", false]);
  none(asked);
  assert.ok(r.calls.some(([what]) => what === "asker"), "after the process check");
  assert.equal(r.registration.authenticateGateway("xenonet_gateway_" + "0".repeat(64)).code, "bad-token", "and nothing was made");

  // Never an actor's own token, a stranger, you, or a program the check refuses.
  const token = (await r.go("POST", "/api/actors/wren/token", { headers: r.window() })).body.token;
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/wren/gateway/ask", { body: {}, headers: r.bearer(token) })), [403, "not-allowed"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/nobody/gateway/ask", { body: {} })), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/you/gateway/ask", { body: {} })), [400, "bad-request"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/%E0%A4%A/gateway/ask", { body: {} })), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/wren/gateway/ask")), [404, "not-found"]);
  r.verdicts.asker = { ok: false, checked: true, reason: "a program under Muse can't ask for a gateway token: it could be relaying for its cloud" };
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/wren/gateway/ask", { body: {} })), [403, "wrong-process"]);
  delete r.verdicts.asker;

  // The window sees the card, with who asked, and no token.
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/requests")), [403, "yours"], "only with the window's key");
  const list = await r.go("GET", "/api/actors/requests", { headers: r.window() });
  valid("actor/requests/v1", list.body);
  none(list);
  const [card] = list.body.gatewayAsks;
  assert.deepEqual([card.id, card.actor, card.name, card.replaces, card.asker.folder], [asked.body.request, "actor:wren", "Wren", false, "~/Projects/docs-site"]);

  // Only the window answers it, and Allow answers with the token, once.
  const yours = { body: { decision: "allow" } };
  assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${card.id}`, yours)), [403, "yours"]);
  assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${card.id}`, { ...yours, headers: r.window(r.bearer(token)) })), [403, "yours"], "never with an actor's token");
  const allowed = await r.go("POST", `/api/actors/requests/${card.id}`, { ...yours, headers: r.window() });
  assert.equal(allowed.status, 200, JSON.stringify(allowed.body));
  valid("actor/gateway-credentials/v1", allowed.body.credentials);
  assert.equal(allowed.body.credentials.url, null, "no gateway here to give an address");
  assert.equal(r.registration.authenticateGateway(allowed.body.credentials.token).id, "actor:wren");
  assert.deepEqual(codeOf(await r.go("POST", `/api/actors/requests/${card.id}`, { ...yours, headers: r.window() })), [404, "not-found"]);
  assert.deepEqual((await r.go("GET", "/api/actors/requests", { headers: r.window() })).body.gatewayAsks, []);

  // Asking again: Allow replaces the token, and Deny keeps it.
  const second = await r.go("POST", "/api/actors/wren/gateway/ask", { body: {} });
  assert.equal(second.body.replaces, true);
  const denied = await r.go("POST", `/api/actors/requests/${second.body.request}`, { body: { decision: "deny" }, headers: r.window() });
  assert.deepEqual(denied.body, { request: second.body.request, decision: "deny", actor: "actor:wren" });
  assert.equal(r.registration.authenticateGateway(allowed.body.credentials.token).ok, true);
});

test("the window sees whether each registered actor's token works, when it expires and when it was last used, and never the token", async () => {
  const DAY = 24 * 3600e3;
  const r = routes();
  const token = await r.allowed();
  r.clock.t += 1000;
  const added = await r.go("POST", "/api/actors", { body: { name: "Moth", capabilities: ["message-agents"] }, headers: r.window() });
  r.actors.upsert({ id: "actor:kestrel", name: "Kestrel", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed"] });
  const tokens = async () => {
    const res = await r.go("GET", "/api/actors/tokens", { headers: r.window() });
    assert.equal(res.status, 200);
    valid("actor/tokens/v1", res.body);
    assert.deepEqual([res.body.schema, res.body.at], ["actor/tokens/v1", r.clock.t]);
    assert.ok(!JSON.stringify(res.body).includes("xenonet_actor_") && !JSON.stringify(res.body).includes(r.store.actorToken("wren").token_hash), "never a token or its hash");
    return res.body.tokens;
  };
  assert.deepEqual(await tokens(), [
    { actor: "actor:wren", state: "works", expiresAt: T + 30 * DAY, usedAt: null },
    { actor: "actor:moth", state: "works", expiresAt: T + 1000 + 30 * DAY, usedAt: null },
  ], "only the actors that report themselves, neither token used yet");
  // Used a minute on: its first use is when it was used, exactly, and its
  // 30 days start again.
  r.clock.t = T + 60e3;
  assert.equal((await r.go("POST", "/api/actors/heartbeat", { body: { presence: "idle" }, headers: r.bearer(token) })).status, 200);
  assert.deepEqual((await tokens())[0], { actor: "actor:wren", state: "works", expiresAt: T + 60e3 + 30 * DAY, usedAt: T + 60e3 });
  // Used again within the hour: still then, to within the hour.
  r.clock.t = T + 30 * 60e3;
  await r.go("GET", "/api/actors/me", { headers: r.bearer(token) });
  assert.equal((await tokens())[0].usedAt, T + 60e3);
  // Two hours on: written again.
  r.clock.t = T + 2 * 3600e3;
  await r.go("GET", "/api/actors/me", { headers: r.bearer(token) });
  assert.deepEqual((await tokens())[0], { actor: "actor:wren", state: "works", expiresAt: r.clock.t + 30 * DAY, usedAt: r.clock.t });
  // A new token is one never used.
  const fresh = await r.go("POST", "/api/actors/wren/token", { headers: r.window() });
  assert.equal(fresh.status, 200);
  assert.deepEqual((await tokens())[0], { actor: "actor:wren", state: "works", expiresAt: r.clock.t + 30 * DAY, usedAt: null });
  r.clock.t += 1000;
  await r.go("GET", "/api/actors/me", { headers: r.bearer(fresh.body.token) });
  assert.equal((await tokens())[0].usedAt, r.clock.t, "until its first call");
  // Moth's goes unused for 30 days.
  r.clock.t = T + 1000 + 30 * DAY;
  assert.deepEqual((await tokens())[1], { actor: "actor:moth", state: "expired", expiresAt: null, usedAt: null });
  r.clock.t += 60e3;
  r.registration.tick(r.clock.t);
  assert.deepEqual((await tokens())[1], { actor: "actor:moth", state: "expired", expiresAt: null, usedAt: null }, "and once it's revoked, there's nothing to use");
  // The window's alone.
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/tokens")), [403, "yours"]);
  assert.deepEqual(codeOf(await r.go("GET", "/api/actors/tokens", { headers: r.window(r.bearer(added.body.token)) })), [403, "yours"]);
  // The demo's made-up actor that reports itself has none.
  const demo = routes({ demo: true });
  demo.actors.upsert({ id: "actor:moth", name: "Moth", via: ["demo", "registered"], heartbeatMs: 30000 });
  demo.actors.upsert({ id: "actor:wren", name: "Wren", via: ["demo", "observed", "attributed"] });
  const made = await demo.go("GET", "/api/actors/tokens", { headers: demo.window() });
  valid("actor/tokens/v1", made.body);
  assert.deepEqual(made.body.tokens.map((t) => [t.actor, t.state]), [["actor:moth", "none"]]);
});

test("your routes: removing an actor and switching observers, from the window or your terminal, never from an agent, a product or a token", async () => {
  const r = routes();
  const token = await r.allowed();
  r.verdicts.person = { ok: false, checked: true, reason: "this is yours to do, and it came from under Grok Bot; run it in your own terminal or in XenoNet" };
  const product = await r.go("DELETE", "/api/actors/wren");
  assert.deepEqual([...codeOf(product), product.body.error], [403, "yours", r.verdicts.person.reason]);
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/wren", { headers: r.bearer(token) })), [403, "yours"], "an actor can't remove itself this way, or any other");
  assert.equal((await r.go("DELETE", "/api/actors/wren", { headers: r.window() })).status, 200, "from the window");
  delete r.verdicts.person;
  await r.go("POST", "/api/actors", { body: { name: "Moth" }, headers: r.window() });
  const removed = await r.go("DELETE", "/api/actors/moth");
  assert.deepEqual([removed.status, removed.body], [200, { removed: "actor:moth" }], "from your own terminal");
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/moth")), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/you")), [403, "yours"], "you're never removed: Show me on the map deletes your record, from the window alone");

  const observers = await r.go("GET", "/api/actors/observers");
  assert.deepEqual([observers.status, observers.body.observers.map((o) => [o.product, o.on])], [200, [["hermes-agent", true], ["grok-bot", true], ["muse", true]]]);
  assert.deepEqual((await r.go("POST", "/api/actors/observers", { body: { product: "grok-bot", on: false } })).body, { product: "grok-bot", on: false, actors: ["actor:grok-bot"] });
  assert.deepEqual((await r.go("GET", "/api/actors/observers")).body.observers.map((o) => o.on), [true, false, true]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/observers", { body: { product: "dots", on: false } })), [404, "not-found"]);
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/observers", { body: { product: "grok-bot", on: "no" } })), [400, "bad-request"]);
  r.verdicts.person = { ok: false, checked: true, reason: "this is yours to do, and it came from under a Codex process; run it in your own terminal or in XenoNet" };
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/observers", { body: { product: "grok-bot", on: true } })), [403, "yours"]);
});

test("an actor XenoNet only observes can't be removed, from the window or a program alike: 409, and its observer's switch takes it off the map", async () => {
  const r = routes();
  r.actors.upsert({ id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed", "attributed"] });
  for (const headers of [{}, r.window()]) {
    const refused = await r.go("DELETE", "/api/actors/grok-bot", { headers });
    assert.deepEqual([...codeOf(refused), refused.body.product], [409, "observed", "grok-bot"]);
    assert.equal(refused.body.error, "XenoNet only observes Grok Bot, so it can't be removed; turn off its observer instead, with the Grok Bot switch under observers in XenoNet's Actors panel, or xenonet actor observe grok-bot off");
  }
  assert.ok(r.actors.has("actor:grok-bot"), "still there");
  r.verdicts.person = { ok: false, checked: true, reason: "this is yours to do, and it came from under a Codex process; run it in your own terminal or in XenoNet" };
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/grok-bot")), [403, "yours"], "from under an agent, refused before anything is said of it");
  delete r.verdicts.person;
  // Hermes registered through its plugin reports itself: removing it takes
  // its registration, and XenoNet keeps observing it.
  r.actors.upsert({ id: "actor:hermes", name: "Hermes", product: { id: "hermes-agent", vendor: "Nous Research", version: null }, via: ["observed", "attributed", "registered"], heartbeatMs: 60000, allowedAt: T });
  const hermes = await r.go("DELETE", "/api/actors/hermes");
  assert.deepEqual([hermes.status, hermes.body], [200, { removed: "actor:hermes", observed: true }]);
  assert.deepEqual(r.actors.info("actor:hermes").via, ["observed", "attributed"]);
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/hermes")), [409, "observed"], "after which XenoNet only observes it");
  // The observer's switch is what takes it off.
  assert.equal((await r.go("POST", "/api/actors/observers", { body: { product: "grok-bot", on: false } })).status, 200);
  assert.ok(!r.actors.list().actors.some((a) => a.id === "actor:grok-bot"));
});

test("the demo's observed actors play the part: 409, while its registered one can be removed", async () => {
  const r = routes({ demo: true });
  for (const def of [
    { id: "actor:wren", name: "Wren", product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }, via: ["demo", "observed", "attributed"] },
    { id: "actor:moth", name: "Moth", via: ["demo", "registered"], heartbeatMs: 30000 },
  ])
    r.actors.upsert(def);
  assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/wren", { headers: r.window() })), [409, "observed"]);
  assert.equal((await r.go("DELETE", "/api/actors/moth", { headers: r.window() })).status, 200);
});

test("with actors off, every new route says so too", async () => {
  const r = routes({ off: true });
  for (const [method, path] of [["POST", "/api/actors/register"], ["GET", "/api/actors/me"], ["POST", "/api/actors/heartbeat"], ["GET", "/api/actors/requests"], ["GET", "/api/actors/tokens"], ["POST", "/api/actors/observers"], ["DELETE", "/api/actors/wren"]]) assert.deepEqual(codeOf(await r.go(method, path, { body: {} })), [503, "off"], `${method} ${path}`);
});

// The routes with the real process check (core/engine/verify.js) on a
// made-up machine: Wren, a program of nova's, runs from a shell in
// Terminal; a Claude Code session has a shell of its own, and another runs
// in a second Terminal window, holding S1. A pid the table
// doesn't hold stands for a process that sent its request and exited
// before XenoNet looked, and no pid for a connection netstat had no line for.
function checkedRoutes() {
  const rows = [
    [100, 1, "/Applications/Claude.app/Contents/MacOS/Claude"],
    [200, 100, `${HOME}/Library/Application Support/Claude/claude.app/Contents/MacOS/claude`],
    [220, 200, "/bin/zsh"],
    [400, 1, "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"],
    [410, 400, "/bin/zsh"],
    [420, 410, `${HOME}/bin/wren`],
    [411, 400, "/bin/zsh"],
    [440, 411, `${HOME}/.local/bin/claude`],
    [500, 1, "/Applications/Muse.app/Contents/MacOS/Muse"],
    [510, 500, "/bin/zsh"],
  ];
  const table = new Map(rows.map(([pid, ppid, program]) => [pid, { pid, ppid, start: 1000 + pid, program }]));
  const procs = {
    supported: true,
    peer: async (socket) => socket.pid ?? null,
    chain: async (pid) => {
      const out = [];
      for (let p = table.get(pid); p; p = p.ppid > 1 ? table.get(p.ppid) : null) out.push({ ...p, agent: agentOf(p.program), product: agentOf(p.program) ? null : productOf(p.program) });
      return out;
    },
    alive: async ({ pid, start }) => table.get(pid)?.start === start,
    signed: async () => false,
    cwd: async () => null,
  };
  const clock = { t: T };
  const now = () => clock.t;
  const store = new EngineStore(":memory:");
  const actors = new Actors({ store, now, map: fakeMap() });
  const registration = createRegistration({ actors, now, home: HOME });
  const verify = createVerifier({ procs, store, home: null, now });
  const route = actorRoutes({ actors, registration, windowKey: createWindowKey(), verify, readJson, send: (res, status, body, headers = {}) => Object.assign(res, { status, body, headers }), now });
  const go = async (method, path, { body, headers = {}, pid } = {}) => {
    const res = {};
    await route({ method, headers, body, socket: { pid } }, res, new URL(path, "http://127.0.0.1"));
    return res;
  };
  store.pin(S1, { pid: 440, start: 1440 }, true, T);
  return { go, actors, registration, verify, clock };
}

test("with the real process check: a caller that's gone, or that netstat can't place, can't use a token or remove an actor", async () => {
  const r = checkedRoutes();
  const { token } = r.registration.add({ name: "Wren", capabilities: ALL });
  const headers = { authorization: `Bearer ${token}` };
  assert.equal((await r.go("POST", "/api/actors/heartbeat", { body: { status: "Rebuilding the docs" }, headers, pid: 420 })).status, 200, "Wren, from your terminal");
  for (const pid of [4999, undefined]) {
    r.clock.t += 2000;
    const beat = await r.go("POST", "/api/actors/heartbeat", { body: { status: "Deploying to production" }, headers, pid });
    assert.deepEqual([...codeOf(beat), beat.body.error], [403, "wrong-process", "XenoNet couldn't see which program sent this, so it doesn't take the token from it"], String(pid));
    assert.deepEqual(codeOf(await r.go("DELETE", "/api/actors/me", { headers, pid })), [403, "wrong-process"]);
    const removed = await r.go("DELETE", "/api/actors/wren", { pid });
    assert.deepEqual([...codeOf(removed), removed.body.error], [403, "yours", "this is yours to do, and XenoNet couldn't see which program asked; run it in your own terminal or in XenoNet"]);
    assert.deepEqual(codeOf(await r.go("POST", "/api/actors/observers", { body: { product: "grok-bot", on: false }, pid })), [403, "yours"]);
    assert.deepEqual(codeOf(await r.go("POST", "/api/actors/register", { body: { name: "Kite" }, pid })), [403, "wrong-process"]);
  }
  assert.equal(r.actors.snapshot("actor:wren").status, "Rebuilding the docs", "nothing it sent was taken");
  assert.deepEqual(codeOf(await r.go("POST", "/api/actors/heartbeat", { body: {}, headers, pid: 220 })), [403, "wrong-process"], "nor from under an agent");
  assert.equal((await r.go("DELETE", "/api/actors/wren", { pid: 410 })).status, 200, "you, from your own terminal");
  assert.equal(r.verify.state.unchecked, 0);
});

test("with the real process check: a session or your terminal may ask for a gateway token, a program under a product's app may not", async () => {
  const r = checkedRoutes();
  r.registration.add({ name: "Wren", capabilities: ALL });
  const ask = (pid) => r.go("POST", "/api/actors/wren/gateway/ask", { body: {}, pid });
  const fromSession = await ask(440);
  assert.deepEqual([fromSession.status, fromSession.body.state], [202, "waiting"], "a Claude Code session asks");
  const fromTerminal = await ask(420);
  assert.equal(fromTerminal.status, 202, "so does a program in your terminal");
  assert.equal(r.registration.requests().gatewayAsks.length, 1, "the same actor asked again takes its place");
  assert.deepEqual(r.registration.requests().gatewayAsks[0].asker.parents.at(-1), "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", "and the card says where it came from");
  const relayed = await ask(510);
  assert.deepEqual([...codeOf(relayed), relayed.body.error], [403, "wrong-process", "a program under Muse can't ask for a gateway token: it could be relaying for its cloud"], "Muse's own app could be relaying for its cloud");
  for (const pid of [4999, undefined]) assert.deepEqual(codeOf(await ask(pid)), [403, "wrong-process"], `${pid}: a caller XenoNet can't place can't say who asked`);
  assert.equal(r.verify.state.unchecked, 0);
});

test("with the real process check: naming Terminal as its process doesn't make another window's session the actor's", async () => {
  const r = checkedRoutes();
  const { token } = r.registration.add({ name: "Wren", capabilities: ALL });
  const headers = { authorization: `Bearer ${token}` };
  const spawned = async () => (await r.go("POST", "/api/actors/events", { body: { events: [{ kind: "spawned", target: S1 }] }, headers, pid: 420 })).body;
  const before = await spawned();
  assert.deepEqual([before.accepted, before.dropped.map((d) => d.reason)], [0, ["that session runs under a process outside this actor's own"]]);
  const app = await r.go("POST", "/api/actors/heartbeat", { body: { pid: 400 }, headers, pid: 420 });
  assert.deepEqual([...codeOf(app), app.body.error], [403, "wrong-process", "the pid it names is an app's, which other programs run under too; name the actor's own process, or a parent of it below the app it runs in"]);
  assert.equal((await r.go("POST", "/api/actors/heartbeat", { body: { pid: 410 }, headers, pid: 420 })).status, 200, "the shell it runs in is fine");
  const after = await spawned();
  assert.deepEqual([after.accepted, r.actors.targetsOf("actor:wren")], [0, []], "S1 still isn't its");
  const asked = await r.go("POST", "/api/actors/register", { body: { name: "Kite", pid: 400 }, pid: 420 });
  assert.deepEqual(codeOf(asked), [403, "wrong-process"], "nor can a registration name it");
});
