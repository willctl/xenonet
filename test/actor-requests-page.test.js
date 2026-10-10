// The page's side of registrations waiting for you (src/actor-requests.js)
// and what the Actors panel offers for each actor (src/actors-model.js), in
// a made-up page: a desktop bridge that gives a key or none, a server that
// answers as told, and a clock the test moves. Requests and actors are
// made up.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createWindowCalls, createActorRequests, createActorTokens, tokenParts, tokenWords, askedBy, asksFor, leftWords, ARM_MS } from "../src/actor-requests.js";
import { actorActions, actorOrder, CAPABILITY_WORDS } from "../src/actors-model.js";

const T = 1791230000000;
const KEY = "b2".repeat(32);
const HERON = { id: "rq1", actor: "actor:heron", name: "Heron", product: null, capabilities: ["run-commands", "edit-repos"], heartbeatMs: 60e3, pid: null, asker: { program: "~/bin/heron", parents: ["/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: "~/Projects/heron" }, joins: false, at: T, expiresAt: T + 10 * 60e3 };

// A server that answers each call with the next of `answers` (a status and
// a body), and keeps what was sent.
function server(answers = []) {
  const sent = [];
  const request = async (url, init) => {
    sent.push({ url, method: init.method, key: init.headers["x-xenonet-window"], type: init.headers["content-type"] ?? null, body: init.body === undefined ? undefined : JSON.parse(init.body) });
    const a = answers.shift() ?? { status: 200, body: {} };
    if (a.throws) throw new Error("connection refused");
    return { ok: a.status < 300, status: a.status, json: async () => a.body ?? {} };
  };
  return { sent, request };
}
const bridge = (keys = [KEY]) => {
  const asked = { n: 0 };
  return { asked, desktop: { windowKey: async () => (asked.n++, keys.length > 1 ? keys.shift() : keys[0]) } };
};

test("the window's calls carry its key, and ask for a new one once when the server started again", async () => {
  const { desktop, asked } = bridge(["k-old", "k-new"]);
  const srv = server([{ status: 403, body: { code: "yours", error: "no key" } }, { status: 201, body: { id: "actor:kite", token: "t" } }]);
  const calls = createWindowCalls({ desktop, request: srv.request });
  const r = await calls.call("POST", "/api/actors", { name: "Kite" });
  assert.deepEqual([r.ok, r.status, r.data.id], [true, 201, "actor:kite"]);
  assert.deepEqual(srv.sent.map((s) => [s.method, s.url, s.key, s.type, s.body?.name]), [["POST", "/api/actors", "k-old", "application/json", "Kite"], ["POST", "/api/actors", "k-new", "application/json", "Kite"]]);
  assert.equal(asked.n, 2);
  // A GET sends no body and no content type.
  await calls.call("GET", "/api/actors/requests");
  assert.deepEqual([srv.sent[2].type, srv.sent[2].body], [null, undefined]);
});

test("refused twice, the refusal stands; with no key, or no bridge, nothing is sent", async () => {
  const srv = server([{ status: 403, body: { code: "yours", error: "not from XenoNet's app" } }, { status: 403, body: { code: "yours", error: "not from XenoNet's app" } }]);
  const r = await createWindowCalls({ desktop: bridge().desktop, request: srv.request }).call("GET", "/api/actors/requests");
  assert.deepEqual([r.ok, r.status, r.code, r.error], [false, 403, "yours", "not from XenoNet's app"]);
  assert.equal(srv.sent.length, 2);

  const quiet = server();
  for (const desktop of [null, { windowKey: async () => null }, { windowKey: async () => Promise.reject(new Error("no")) }]) {
    const calls = createWindowCalls({ desktop, request: quiet.request });
    assert.deepEqual(await calls.call("POST", "/api/actors/requests/rq1", { decision: "allow" }), { ok: false, status: 0, code: "no-key", error: "Only XenoNet's own window can do that", data: {} });
    assert.equal(await calls.ready(), false);
  }
  assert.equal(quiet.sent.length, 0);
  assert.equal(await createWindowCalls({ desktop: bridge().desktop, request: quiet.request }).ready(), true);
  // A server that doesn't answer.
  const down = server([{ throws: true }]);
  assert.equal((await createWindowCalls({ desktop: bridge().desktop, request: down.request }).call("GET", "/api/actors/requests")).code, "unreachable");
});

test("requests: read with the key at the start and when the count changes; Allow waits a moment after a card shows", async () => {
  const clock = { t: T };
  const srv = server([{ status: 200, body: { at: T, requests: [HERON] } }, { status: 200, body: { request: "rq1", decision: "allow" } }, { status: 200, body: { at: T, requests: [] } }]);
  const views = [];
  const requests = createActorRequests({ calls: createWindowCalls({ desktop: bridge().desktop, request: srv.request }), onChange: (v) => views.push(v), now: () => clock.t });
  const v = await requests.refresh();
  assert.deepEqual([v.canDecide, v.waiting, v.requests.map((r) => r.id)], [true, 1, ["rq1"]]);
  assert.equal(views.length, 1);
  assert.equal(requests.armed("rq1"), false, "not at once");
  assert.equal(requests.armsIn("rq1"), ARM_MS);
  clock.t += ARM_MS;
  assert.equal(requests.armed("rq1"), true);
  assert.equal(requests.armed("rq-unknown"), false);

  const r = await requests.decide("rq1", "allow");
  assert.equal(r.ok, true);
  assert.deepEqual(srv.sent.map((s) => [s.method, s.url, s.body?.decision]), [["GET", "/api/actors/requests", undefined], ["POST", "/api/actors/requests/rq1", "allow"], ["GET", "/api/actors/requests", undefined]]);
  assert.deepEqual(requests.view(), { requests: [], waiting: 0, canDecide: true, off: false });
  // Answered, it's forgotten: one that came back would wait again.
  assert.equal(requests.armed("rq1"), false);
});

test("requests: the newest read wins, a window without a key knows only how many wait, and actors off stop it all", async () => {
  // Two reads in flight: the second answers first, and the first's
  // (older) answer is dropped.
  let release;
  const slow = new Promise((r) => (release = r));
  let n = 0;
  const calls = { call: async () => (++n === 1 ? (await slow, { ok: true, status: 200, data: { requests: [HERON] } }) : { ok: true, status: 200, data: { requests: [] } }) };
  const requests = createActorRequests({ calls });
  const first = requests.refresh();
  await requests.refresh();
  release();
  await first;
  assert.deepEqual(requests.view().requests, []);

  const views = [];
  const keyless = createActorRequests({ calls: createWindowCalls({ desktop: null }), onChange: (v) => views.push(v) });
  await keyless.refresh();
  keyless.heard({ waiting: 2 });
  assert.deepEqual(keyless.view(), { requests: [], waiting: 2, canDecide: false, off: false });
  keyless.heard({ waiting: "lots" });
  assert.equal(keyless.view().waiting, 0, "a count is a count");

  const srv = server([{ status: 503, body: { code: "off", error: "Actors are off here (XENONET_ACTORS=off)" } }]);
  const off = createActorRequests({ calls: createWindowCalls({ desktop: bridge().desktop, request: srv.request }) });
  assert.equal((await off.refresh()).off, true);
  await off.refresh();
  assert.equal(srv.sent.length, 1, "once off, it doesn't ask again");
});

test("tokens: read with the key, the newest read winning; a window without it sees none, and actors off stop it", async () => {
  const DAY = 86400e3;
  const kite = { actor: "actor:kite", state: "works", expiresAt: T + 30 * DAY, usedAt: T };
  const srv = server([{ status: 200, body: { at: T, tokens: [kite, { actor: "actor:moth", state: "none", expiresAt: null, usedAt: null }, { state: "works" }] } }]);
  const tokens = createActorTokens({ calls: createWindowCalls({ desktop: bridge().desktop, request: srv.request }) });
  const v = await tokens.refresh();
  assert.deepEqual([v.canSee, v.off, v.at, v.tokens.map((t) => t.actor)], [true, false, T, ["actor:kite", "actor:moth"]], "one without an actor is no one's");
  assert.deepEqual([srv.sent[0].method, srv.sent[0].url, srv.sent[0].key], ["GET", "/api/actors/tokens", KEY]);
  assert.deepEqual(tokens.of("actor:kite"), kite);
  assert.equal(tokens.of("actor:wren"), null, "an actor XenoNet only observes has no token");

  // Two reads in flight: the newer one's answer stands.
  let release;
  const slow = new Promise((r) => (release = r));
  let n = 0;
  const racing = createActorTokens({ calls: { call: async () => (++n === 1 ? (await slow, { ok: true, status: 200, data: { tokens: [kite] } }) : { ok: true, status: 200, data: { tokens: [] } }) } });
  const first = racing.refresh();
  await racing.refresh();
  release();
  await first;
  assert.deepEqual(racing.view().tokens, []);

  const keyless = createActorTokens({ calls: createWindowCalls({ desktop: null }) });
  assert.deepEqual(await keyless.refresh(), { canSee: false, off: false, at: null, tokens: [] });
  const refused = createActorTokens({ calls: createWindowCalls({ desktop: bridge().desktop, request: server([{ status: 403, body: { code: "yours" } }, { status: 403, body: { code: "yours" } }]).request }) });
  assert.equal((await refused.refresh()).canSee, false);
  const offSrv = server([{ status: 503, body: { code: "off" } }]);
  const off = createActorTokens({ calls: createWindowCalls({ desktop: bridge().desktop, request: offSrv.request }) });
  assert.equal((await off.refresh()).off, true);
  await off.refresh();
  assert.equal(offSrv.sent.length, 1, "once off, it doesn't ask again");
});

test("what a token's line says: works, expired or none, when it expires, and when it was last used or that it never was", () => {
  const DAY = 86400e3, H = 3600e3;
  assert.equal(tokenWords({ actor: "actor:kite", state: "works", expiresAt: T + 30 * DAY, usedAt: T - 2 * H }, T), "token works · expires in 30 days · last used 2h ago");
  assert.equal(tokenWords({ actor: "actor:kite", state: "works", expiresAt: T + 30 * DAY, usedAt: null }, T), "token works · expires in 30 days · never used");
  assert.equal(tokenWords({ actor: "actor:kite", state: "works", expiresAt: T + 1.5 * DAY, usedAt: T }, T), "token works · expires in 2 days · last used just now");
  assert.equal(tokenWords({ actor: "actor:kite", state: "works", expiresAt: T + DAY, usedAt: T }, T), "token works · expires in 1 day · last used just now");
  assert.equal(tokenWords({ actor: "actor:kite", state: "works", expiresAt: T + 5 * H, usedAt: T }, T), "token works · expires in under a day · last used just now");
  // Expired: its last use while XenoNet still has it, then nothing more.
  assert.equal(tokenWords({ actor: "actor:kite", state: "expired", expiresAt: null, usedAt: T - 31 * DAY }, T), "token expired, unused for 30 days · last used 31d ago");
  assert.equal(tokenWords({ actor: "actor:kite", state: "expired", expiresAt: null, usedAt: null }, T), "token expired, unused for 30 days");
  assert.equal(tokenWords({ actor: "actor:moth", state: "none", expiresAt: null, usedAt: null }, T), "no token");
  // A state XenoNet's page doesn't know yet says itself; a time that isn't a
  // number says nothing.
  assert.equal(tokenWords({ actor: "actor:kite", state: "paused", expiresAt: "soon", usedAt: "today" }, T), "token paused");
  assert.deepEqual(tokenParts({ actor: "actor:kite", state: "works", expiresAt: T + DAY * 3, usedAt: T - H }, T), { state: "token works", expires: "expires in 3 days", used: T - H, usedWords: "last used 1h ago" });
  assert.equal(tokenWords(null), null);
});

test("what a card says: what it asks, who asked by name and from where, and how long it waits", () => {
  assert.deepEqual(askedBy(HERON.asker), { program: "~/bin/heron", from: ["zsh", "Terminal"], parents: HERON.asker.parents, folder: "~/Projects/heron" });
  assert.equal(askedBy(null), null, "where XenoNet couldn't look");
  assert.deepEqual(askedBy({ program: "/usr/local/bin/node", parents: ["/Applications/iTerm.app/Contents/MacOS/iTerm2"], folder: null }).from, ["iTerm"]);
  // A number keeps its unit on its line.
  assert.equal(asksFor(HERON, CAPABILITY_WORDS), "asks to run commands, edit repos · a heartbeat every 60\u00a0s");
  assert.equal(asksFor({ capabilities: [], heartbeatMs: 300e3 }, CAPABILITY_WORDS), "asks for nothing but to report · a heartbeat every 5\u00a0min");
  assert.equal(asksFor({ capabilities: ["teleport"], heartbeatMs: null }, CAPABILITY_WORDS), "asks to teleport");
  assert.equal(leftWords(T + 10 * 60e3, T), "10\u00a0min left");
  assert.equal(leftWords(T + 61e3, T), "2\u00a0min left");
  assert.equal(leftWords(T + 59e3, T), "under a minute left");
  assert.equal(leftWords(T - 1, T), "lapsing");
});

test("the Actors panel offers New token and Remove for registered actors, an observer's switch for observed ones, Gateway… for those that may use the gateway, and nothing that hands out a token without the key", () => {
  const observed = new Set(["hermes-agent", "grok-bot"]);
  const you = { id: "actor:you", kind: "human", via: ["observed"] };
  const wren = { id: "actor:wren", kind: "agent", via: ["demo", "observed", "attributed"], product: { id: "hermes-agent" } };
  const kestrel = { id: "actor:kestrel", kind: "agent", via: ["observed"], product: { id: "grok-bot" } };
  const moth = { id: "actor:moth", kind: "agent", via: ["demo", "registered"], product: null };
  // Hermes with its plugin in: observed and registered.
  const hermes = { id: "actor:hermes", kind: "agent", via: ["observed", "attributed", "registered"], product: { id: "hermes-agent" } };
  const own = { id: "actor:kite", kind: "agent", via: ["registered"], product: { id: "kite-tools" } };
  const a = (s, canDecide = true) => actorActions(s, { canDecide, observed });
  // Muse, as XenoNet observes it: a gateway token registers it.
  const muse = { id: "actor:muse", kind: "agent", via: ["observed"], product: { id: "muse" } };
  assert.deepEqual(a(you), { you: true, token: false, remove: false, observer: null, gateway: false });
  assert.deepEqual(a(wren), { you: false, token: false, remove: false, observer: "hermes-agent", gateway: false }, "the demo's are made up");
  assert.deepEqual(a(kestrel), { you: false, token: false, remove: false, observer: "grok-bot", gateway: true }, "a Grok Bot works in the cloud");
  assert.deepEqual(a(muse), { you: false, token: false, remove: true, observer: null, gateway: true });
  assert.deepEqual(a(moth), { you: false, token: true, remove: true, observer: null, gateway: false });
  assert.deepEqual(a(hermes), { you: false, token: true, remove: true, observer: "hermes-agent", gateway: true });
  assert.deepEqual(a(own), { you: false, token: true, remove: true, observer: null, gateway: true }, "a product id XenoNet doesn't observe is the actor's own word");
  for (const s of [moth, hermes, own, muse, kestrel]) assert.deepEqual([a(s, false).token, a(s, false).remove, a(s, false).gateway], [false, false, false], `${s.id}: not without the key`);
  // You first, then by name, whatever their presence.
  const list = [{ ...moth, name: "Moth" }, { ...wren, name: "Wren" }, { ...you, name: "You" }, { ...kestrel, name: "Kestrel" }];
  assert.deepEqual(list.sort(actorOrder).map((x) => x.id), ["actor:you", "actor:kestrel", "actor:moth", "actor:wren"]);
});
