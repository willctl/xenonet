// Waking actors when a message arrives (core/actors/wake.js) and an
// actor's own stream (GET /api/actors/me/stream): a webhook's request as
// Standard Webhooks has it, signed or with a bearer key, saying only how
// many messages wait; messages in a burst make one wake, 30 seconds apart at
// most, and a failing webhook waits longer each time; a URL that isn't
// https (or this computer), carries a password or is link-local is
// refused; the URL and key go to the credential store and never to a log;
// an actor nothing wakes is a notification after two minutes, hourly at
// most; and a program's stream hears of each message at once, until its
// token stops working. Made-up actors, URLs and keys.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import http from "node:http";
import { createWakes, checkWakeUrl, wakeSignature, wakeRequest, wakeTarget, linkLocal, MIN_GAP_MS, COALESCE_MS, BACKOFF_MS, NOTIFY_AFTER_MS, NOTIFY_EVERY_MS, STREAMS_PER_ACTOR, UNREAD_HOLD_MS } from "../core/actors/wake.js";
import { createMemorySecrets } from "../core/secret-store.js";
import { EngineStore } from "../core/engine/store.js";
import { Engine } from "../core/engine/engine.js";
import { Actors, mapOf } from "../core/actors/index.js";
import { createRegistration } from "../core/actors/registration.js";
import { actorRoutes, createActorCaller } from "../server/routes/actors.js";
import { createWindowKey } from "../core/actors/key.js";

const KEY = "rk_live_made_up_key_0001";
const servers = [];
after(() => servers.forEach((s) => s.close()));

test("a webhook's request: Standard Webhooks' signature (its own test vector), its headers and body, and a bearer key otherwise", () => {
  // The vector the Standard Webhooks libraries test against.
  assert.equal(wakeSignature("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw", "msg_p5jXN8AQM9LWM0D4loKWxJek", 1614265330, '{"test": 2432232314}'), "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");
  const now = Date.UTC(2026, 9, 9, 12);
  const signed = wakeRequest({ key: "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw", actor: "actor:kite", unread: 2, version: "9.9.9", now, id: "msg_1" });
  assert.equal(signed.method, "POST");
  assert.equal(signed.redirect, "manual", "never follows a redirect");
  assert.deepEqual(JSON.parse(signed.body), { type: "xenonet.inbox", timestamp: "2026-10-09T12:00:00.000Z", data: { actor: "actor:kite", unread: 2 } });
  assert.deepEqual([signed.headers["webhook-id"], signed.headers["webhook-timestamp"], signed.headers["user-agent"]], ["msg_1", String(now / 1000), "XenoNet/9.9.9"]);
  assert.equal(signed.headers["webhook-signature"], wakeSignature("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw", "msg_1", now / 1000, signed.body));
  assert.equal(signed.headers.authorization, undefined, "a signed one sends no key");
  const bearer = wakeRequest({ key: KEY, actor: "actor:kite", unread: 1, now });
  assert.equal(bearer.headers.authorization, `Bearer ${KEY}`);
  assert.equal(bearer.headers["webhook-signature"], undefined);
  assert.match(wakeRequest({ key: KEY, actor: "a", unread: 0 }).headers["webhook-id"], /^msg_[0-9a-f]{32}$/, "a new id each time");
});

test("a webhook URL: https, or http on this computer; no password in it, nothing link-local, nothing too long", () => {
  assert.deepEqual(checkWakeUrl(" https://api.example.com/automations/webhook/abc#x "), { url: "https://api.example.com/automations/webhook/abc", host: "api.example.com" });
  assert.equal(checkWakeUrl("http://127.0.0.1:9000/wake").host, "127.0.0.1:9000");
  for (const [bad, why] of [
    ["http://api.example.com/hook", /https:\/\//],
    ["https://user:pass@api.example.com/hook", /user name or password/],
    ["https://169.254.169.254/latest", /link-local/],
    ["https://[::ffff:169.254.169.254]/latest", /link-local/],
    ["https://[fe9f::1]/x", /link-local/],
    ["ftp://api.example.com/x", /https:\/\//],
    ["not a url", /isn't a URL/],
    [`https://api.example.com/${"a".repeat(2050)}`, /2,048/],
  ])
    assert.throws(() => checkWakeUrl(bad), (err) => err.status === 400 && why.test(err.message), bad);
});

// A world with an engine, a registered actor Kite, and a webhook that
// records each call; the clock and the timers are the test's.
async function world({ answer = () => ({ status: 200 }), slowSecrets = false, addresses = [{ address: "203.0.113.9", family: 4 }] } = {}) {
  const clock = { t: Date.UTC(2026, 9, 9, 12) };
  const now = () => clock.t;
  const store = new EngineStore(":memory:");
  const state = { graph: {}, index: new Map(), live: { sessions: new Map() }, homeOf: () => null };
  const actors = new Actors({ store, now, map: mapOf(state) });
  const engine = new Engine({ state, store, actors, now, repos: { of: async () => null } });
  const registration = createRegistration({ actors, now });
  const kite = registration.add({ schema: "actor/registration/v1", name: "Kite", capabilities: ["message-agents"], heartbeatMs: 60e3 });
  const muse = registration.add({ schema: "actor/registration/v1", name: "Wren", capabilities: ["message-agents"], heartbeatMs: 60e3 });
  const calls = [];
  const logs = [];
  const notes = [];
  const timers = [];
  const secrets = createMemorySecrets();
  // A credential store that answers a moment later, as a real one does.
  if (slowSecrets) {
    const get = secrets.get;
    secrets.get = async (t) => (await new Promise((r) => setTimeout(r, 30)), get(t));
  }
  const fetch = async (url, init) => {
    calls.push({ url, init });
    const a = answer(calls.length);
    if (a.throws) throw a.throws;
    return { status: a.status, body: { cancel: async () => {} } };
  };
  const wakes = createWakes({
    engine,
    actors,
    store,
    secrets,
    fetch,
    resolve: async () => addresses,
    now,
    version: "9.9.9",
    log: (code, d) => logs.push(`${code} ${d}`),
    notify: (n) => notes.push(n),
    setTimer: (fn, ms) => {
      const t = { fn, at: clock.t + ms, unref() {} };
      timers.push(t);
      return t;
    },
    clearTimer: (t) => t && (t.cleared = true),
  });
  // Runs the timers due by now, as time passes.
  const pass = async (ms) => {
    clock.t += ms;
    for (const t of timers.filter((x) => !x.cleared && !x.ran && x.at <= clock.t)) {
      t.ran = true;
      await t.fn();
    }
    await new Promise((r) => setImmediate(r));
  };
  const say = (to, text = "hello") => engine.send("actor:wren", { to, text });
  return { clock, store, actors, engine, registration, wakes, secrets, calls, logs, notes, pass, say, kite, muse };
}

test("a webhook wake: messages in a burst make one call, saying only how many wait; none once they're read; 30 seconds apart at most", async () => {
  const w = await world();
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/wake/k1", key: KEY });
  // Kept in the credential store, not in XenoNet's own data.
  assert.equal(await w.secrets.get(wakeTarget("kite", "url")), "https://hooks.example.com/wake/k1");
  assert.equal(await w.secrets.get(wakeTarget("kite", "key")), KEY);
  assert.doesNotMatch(w.store.meta("wakes"), /k1|rk_live/);
  await w.say("actor:kite", "the build is green");
  await w.say("actor:kite", "and deployed");
  await w.pass(COALESCE_MS);
  assert.equal(w.calls.length, 1, "one wake for the burst");
  const body = JSON.parse(w.calls[0].init.body);
  assert.deepEqual(body.data, { actor: "actor:kite", unread: 2 });
  assert.doesNotMatch(w.calls[0].init.body, /green|deployed|wren/, "never what they say or who sent them");
  assert.equal(w.calls[0].init.headers.authorization, `Bearer ${KEY}`);
  assert.deepEqual([w.wakes.view("actor:kite").last.ok, w.wakes.view("actor:kite").host], [true, "hooks.example.com"]);
  // It reads them; another soon after waits for 30 seconds from the last.
  await w.engine.inbox("actor:kite");
  await w.say("actor:kite");
  await w.pass(COALESCE_MS);
  assert.equal(w.calls.length, 1);
  await w.pass(MIN_GAP_MS);
  assert.equal(w.calls.length, 2);
  // Read before its wake: no call.
  await w.engine.inbox("actor:kite");
  await w.say("actor:kite");
  await w.engine.inbox("actor:kite");
  await w.pass(MIN_GAP_MS + COALESCE_MS);
  assert.equal(w.calls.length, 2);
});

test("a failing webhook: said in words without its URL or key, and tried again later each time, up to half an hour", async () => {
  const w = await world({ answer: (n) => (n === 1 ? { status: 401 } : n === 2 ? { status: 302 } : n === 3 ? { throws: Object.assign(new Error("fetch failed"), { cause: { code: "ECONNREFUSED" } }) } : { status: 204 }) });
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/wake/secret-path?token=abc", key: KEY });
  await w.say("actor:kite");
  await w.pass(COALESCE_MS);
  assert.deepEqual([w.wakes.view("actor:kite").last.why, w.wakes.view("actor:kite").failures], ["hooks.example.com answered HTTP 401", 1]);
  await w.say("actor:kite");
  await w.pass(BACKOFF_MS[0] - 1000);
  assert.equal(w.calls.length, 1, "it waits its backoff");
  await w.pass(1000);
  assert.match(w.wakes.view("actor:kite").last.why, /answered with a redirect \(HTTP 302\), which XenoNet doesn't follow/);
  await w.say("actor:kite");
  await w.pass(BACKOFF_MS[1]);
  assert.equal(w.wakes.view("actor:kite").last.why, "XenoNet couldn't reach hooks.example.com (ECONNREFUSED)");
  await w.say("actor:kite");
  await w.pass(BACKOFF_MS[2]);
  assert.deepEqual([w.wakes.view("actor:kite").last.ok, w.wakes.view("actor:kite").failures], [true, 0]);
  const said = w.logs.join("\n");
  assert.doesNotMatch(said, /secret-path|token=abc|rk_live/, "no URL path, query or key in the log");
});

test("setting a wake: a webhook needs a URL and a key; Notify me and Nothing forget them; gone with the actor", async () => {
  const w = await world();
  await assert.rejects(w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: "short" }), /8 to 1,024 printable characters/);
  await assert.rejects(w.wakes.set("kite", { kind: "pigeon" }), /a wake is webhook, notify, none/);
  await assert.rejects(w.wakes.set("nobody", { kind: "none" }), (err) => err.status === 404);
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw" });
  assert.deepEqual([w.wakes.view("actor:kite").kind, w.wakes.view("actor:kite").signed], ["webhook", true]);
  await w.wakes.set("kite", { kind: "notify" });
  assert.equal(await w.secrets.get(wakeTarget("kite", "key")), null, "its key is gone");
  assert.equal(w.wakes.view("actor:kite").kind, "notify");
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  await w.wakes.forget("kite");
  assert.deepEqual([w.wakes.view("actor:kite").kind, await w.secrets.get(wakeTarget("kite", "url"))], ["none", null]);
  // A test: called now, even with nothing waiting, once every 5 seconds.
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  assert.equal((await w.wakes.test("kite")).ok, true);
  assert.equal(JSON.parse(w.calls.at(-1).init.body).data.unread, 0);
  await assert.rejects(w.wakes.test("kite"), (err) => err.status === 429);
  await w.wakes.set("kite", { kind: "none" });
  await w.pass(10e3);
  await assert.rejects(w.wakes.test("kite"), (err) => err.code === "no-webhook");
});

test("any agent can write to an actor, so its wakes are held: not again while what it was woken for waits unread, and 20 an hour at most", async () => {
  const w = await world();
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  await w.say("actor:kite");
  await w.pass(COALESCE_MS);
  assert.equal(w.calls.length, 1);
  // Unread, so more messages wait their ten minutes.
  for (let i = 0; i < 10; i++) {
    await w.say("actor:kite");
    await w.pass(MIN_GAP_MS);
  }
  assert.equal(w.calls.length, 1, "held while it hasn't read");
  await w.pass(UNREAD_HOLD_MS);
  assert.equal(w.calls.length, 2, "then woken again");
  // Reading each time, an hour of messages every 31 seconds: 20 an hour,
  // after the five it may have at once.
  const before = w.calls.length;
  for (let i = 0; i < 116; i++) {
    await w.engine.inbox("actor:kite");
    await w.say("actor:kite");
    await w.pass(31e3);
  }
  const woken = w.calls.length - before;
  assert.ok(woken >= 20 && woken <= 25, `${woken} wakes in an hour`);
});

test("one wake at a time: a message that arrives while a wake is on its way waits for the next; a test doesn't get in the way", async () => {
  const w = await world({ slowSecrets: true });
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  await w.say("actor:kite");
  w.clock.t += COALESCE_MS;
  const due = w.pass(0);
  await w.say("actor:kite");
  await w.pass(COALESCE_MS);
  await due;
  await new Promise((r) => setTimeout(r, 80));
  assert.equal(w.calls.length, 1, "one call, not two at once");
  await w.pass(10e3);
  assert.equal((await w.wakes.test("kite")).ok, true);
  assert.equal(w.calls.length, 2);
});

test("a webhook changed outside XenoNet, or one whose name turns out link-local, isn't called", async () => {
  const w = await world();
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  // Another program, running as you, puts its own URL in the store.
  await w.secrets.set(wakeTarget("kite", "url"), "http://elsewhere.example.net/collect");
  const swapped = await w.wakes.test("kite");
  assert.deepEqual([swapped.ok, swapped.reason, w.calls.length], [false, "its webhook changed outside XenoNet", 0]);
  const meta = await world({ addresses: [{ address: "169.254.169.254", family: 4 }] });
  await meta.wakes.set("kite", { kind: "webhook", url: "https://metadata.example.com/x", key: KEY });
  const local = await meta.wakes.test("kite");
  assert.deepEqual([local.ok, local.reason, meta.calls.length], [false, "a link-local address", 0]);
  assert.deepEqual(["169.254.1.1", "fe80::1", "::ffff:a9fe:a9fe", "10.0.0.1"].map(linkLocal), [true, true, true, false]);
});

test("an actor nothing wakes: a notification once a message has waited two minutes, how many and never what, hourly at most", async () => {
  const w = await world();
  await w.wakes.set("kite", { kind: "notify" });
  await w.say("actor:kite", "the secret plan");
  await w.pass(NOTIFY_AFTER_MS);
  assert.deepEqual(w.notes, [{ actor: "actor:kite", name: "Kite", unread: 1 }]);
  await w.say("actor:kite");
  await w.pass(NOTIFY_AFTER_MS);
  assert.equal(w.notes.length, 1, "not again within the hour");
  await w.pass(NOTIFY_EVERY_MS);
  await w.say("actor:kite");
  await w.pass(NOTIFY_AFTER_MS);
  assert.deepEqual(w.notes.at(-1), { actor: "actor:kite", name: "Kite", unread: 3 });
  // Read in time: nothing.
  const before = w.notes.length;
  await w.engine.inbox("actor:kite");
  await w.say("actor:kite");
  await w.engine.inbox("actor:kite");
  await w.pass(NOTIFY_AFTER_MS + NOTIFY_EVERY_MS);
  assert.equal(w.notes.length, before);
  // Nothing set: Nothing, unless it reaches XenoNet through the gateway.
  assert.equal(w.wakes.view("actor:wren").kind, "none");
});

test("a program listening hears of each message at once, instead of a webhook; at most four streams an actor", async () => {
  const w = await world();
  await w.wakes.set("kite", { kind: "webhook", url: "https://hooks.example.com/x", key: KEY });
  const heard = [];
  const off = w.wakes.subscribe("actor:kite", { send: (e, d) => heard.push([e, d]), end: () => {} });
  await w.say("actor:kite");
  assert.deepEqual(heard, [["inbox", { unread: 1 }]]);
  await w.pass(MIN_GAP_MS);
  assert.equal(w.calls.length, 0, "listening, it needs no webhook");
  assert.equal(w.wakes.view("actor:kite").listening, 1);
  const more = [1, 2, 3].map(() => w.wakes.subscribe("actor:kite", { send() {}, end() {} }));
  assert.throws(() => w.wakes.subscribe("actor:kite", { send() {}, end() {} }), (err) => err.status === 429 && err.message.includes(String(STREAMS_PER_ACTOR)));
  more.forEach((f) => f());
  off();
  assert.equal(w.wakes.view("actor:kite").listening, 0);
});

test("the stream route: an actor's token opens it, it hears each message as it arrives, and nothing else opens it", async () => {
  const w = await world();
  const key = createWindowKey();
  const verify = { window: async () => ({ ok: true }), person: async () => ({ ok: true }), actor: async () => ({ ok: true, checked: true, chain: [] }), peer: async () => 4242 };
  const caller = createActorCaller({ registration: w.registration, verify, now: () => w.clock.t });
  const readJson = (req) => new Promise((resolve) => { let b = ""; req.on("data", (d) => (b += d)).on("end", () => resolve(b ? JSON.parse(b) : {})); });
  const route = actorRoutes({ actors: w.actors, registration: w.registration, caller, windowKey: key, verify, wakes: w.wakes, readJson, send: (res, status, body) => (res.writeHead(status, { "content-type": "application/json" }), res.end(JSON.stringify(body))), now: () => w.clock.t });
  const server = http.createServer((req, res) => route(req, res, new URL(req.url, "http://127.0.0.1")));
  servers.push(server);
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  const base = `http://127.0.0.1:${server.address().port}`;
  assert.equal((await fetch(`${base}/api/actors/me/stream`)).status, 401, "no token, no stream");
  const res = await fetch(`${base}/api/actors/me/stream`, { headers: { authorization: `Bearer ${w.kite.token}` } });
  assert.equal(res.status, 200);
  assert.match(res.headers.get("content-type"), /^text\/event-stream/);
  const reader = res.body.getReader();
  let text = "";
  const until = async (re) => {
    while (!re.test(text)) text += new TextDecoder().decode((await reader.read()).value);
  };
  await until(/retry: 5000/);
  await w.say("actor:kite", "ping");
  await until(/event: inbox\ndata: \{"unread":1\}/);
  assert.doesNotMatch(text, /ping/, "how many, not what");
  await reader.cancel();
  // Setting a webhook is the window's, with its key; seeing them is yours.
  const put = await fetch(`${base}/api/actors/kite/wake`, { method: "PUT", body: JSON.stringify({ kind: "webhook", url: "https://hooks.example.com/x", key: KEY }) });
  assert.equal(put.status, 403);
  const ok = await fetch(`${base}/api/actors/kite/wake`, { method: "PUT", headers: { "x-xenonet-window": key.reveal() }, body: JSON.stringify({ kind: "webhook", url: "https://hooks.example.com/x", key: KEY }) });
  assert.deepEqual([ok.status, (await ok.json()).host], [200, "hooks.example.com"]);
  const listed = await (await fetch(`${base}/api/actors/wakes`)).json();
  assert.doesNotMatch(JSON.stringify(listed), /rk_live|hooks\.example\.com\/x/, "never the URL or the key");
  assert.equal((await fetch(`${base}/api/actors/kite/wake`, { method: "PUT", headers: { authorization: `Bearer ${w.kite.token}`, "x-xenonet-window": key.reveal() }, body: "{}" })).status, 403, "never with an actor's token");
  assert.equal((await fetch(`${base}/api/actors/kite/wake`, { method: "DELETE" })).status, 200);
});
