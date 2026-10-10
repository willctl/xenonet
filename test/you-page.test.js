// The page's reports of you (src/you.js), in a made-up page: a clock and
// timers the test moves, a window and document with input, focus and
// visibility, the desktop bridge's key, and a server that answers as told.
// What goes out is checked against actor/console/v1 strictly: node ids and
// action names, never text.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createYou, SWITCH } from "../src/you.js";
import { check, explain } from "../core/schema/index.js";

const T = 1791230000000;
const MIN = 60e3;
const KEY = "a1".repeat(32);
const ATLAS = "project:/users/nova/projects/atlas-api";
const LEDGER = "project:/users/nova/projects/ledger";
const S1 = "claude:0190d3e0-0000-7000-8000-00000000aa01";
const settle = async () => {
  for (let i = 0; i < 6; i++) await new Promise((r) => setImmediate(r));
};

// `window`: the desktop app's word on its window ("shown" or "hidden"), for
// a bridge that has it; setWindow() changes it as the app would say.
// `replies`: answers to the window's other calls by "METHOD path", each a
// list taken in turn; a route with none answers 404, as one XenoNet doesn't
// have. `hold`: reports wait on the way until release() lets them land.
function page({ keys = null, bridge = true, focused = true, visible = true, off = false, answers = [], window = null, replies = {}, hold = false, onRecord } = {}) {
  const clock = { t: T };
  const timers = [];
  const intervals = [];
  let ids = 0;
  const later = (fn, ms) => {
    const id = ++ids;
    timers.push({ id, at: clock.t + ms, fn });
    return id;
  };
  const cancel = (id) => {
    const i = timers.findIndex((t) => t.id === id);
    if (i >= 0) timers.splice(i, 1);
  };
  const every = (fn, ms) => intervals.push({ fn, ms, next: clock.t + ms });
  const listeners = {};
  const on = (type, fn) => (listeners[type] ??= []).push(fn);
  const doc = { visibilityState: visible ? "visible" : "hidden", focused, hasFocus: () => doc.focused, addEventListener: on };
  const stored = new Map(off ? [[SWITCH, "off"]] : []);
  const storage = { getItem: (k) => stored.get(k) ?? null, setItem: (k, v) => stored.set(k, v) };
  const sent = [];
  // Every call in the order it reached the server: [method, path].
  const reached = [];
  const calls = [];
  const held = [];
  const release = () => held.splice(0).forEach((go) => go());
  const request = async (url, init) => {
    const method = init.method ?? "GET";
    if (method !== "POST") {
      calls.push({ method, url, key: init.headers?.["x-xenonet-window"], at: clock.t });
      reached.push([method, url]);
      const a = (replies[`${method} ${url}`] ?? []).shift() ?? { status: 404, body: { error: "XenoNet has no such route for actors", code: "not-found" } };
      return { ok: a.status < 300, status: a.status, headers: { get: () => null }, json: async () => a.body ?? {} };
    }
    if (hold) await new Promise((go) => held.push(go));
    sent.push({ url, key: init.headers["x-xenonet-window"], body: JSON.parse(init.body), at: clock.t });
    reached.push([method, url]);
    const a = answers.shift() ?? { status: 200 };
    return { ok: a.status < 300, status: a.status, headers: { get: (h) => a.headers?.[h] ?? null } };
  };
  const asked = { n: 0 };
  const desktop = bridge ? { windowKey: async () => (asked.n++, keys ? (keys.shift() ?? null) : KEY) } : null;
  let windowSays = null;
  if (desktop && window) Object.assign(desktop, { windowState: async () => window, onWindowState: (fn) => (windowSays = fn) });
  const setWindow = (state) => ((window = state), windowSays?.(state));
  const you = createYou({ desktop, doc, win: { addEventListener: on }, storage, request, onRecord, now: () => clock.t, later, cancel, every });
  // Moves the clock on, running what falls due on the way.
  async function run(ms) {
    const until = clock.t + ms;
    for (;;) {
      await settle();
      const due = [...timers.map((t) => ({ at: t.at, go: () => (timers.splice(timers.indexOf(t), 1), t.fn()) })), ...intervals.map((iv) => ({ at: iv.next, go: () => ((iv.next += iv.ms), iv.fn()) }))].filter((d) => d.at <= until).sort((a, b) => a.at - b.at)[0];
      if (!due) break;
      clock.t = due.at;
      due.go();
    }
    clock.t = until;
    await settle();
  }
  const fire = (type, e = {}) => listeners[type]?.forEach((fn) => fn(e));
  return { you, sent, clock, run, fire, doc, asked, stored, setWindow, calls, reached, release };
}

function fits(body) {
  const r = check("actor/console/v1", body, { strict: true });
  assert.ok(r.ok, explain(r));
}

test("you're reported at once, with the window's key, and with nothing the schema leaves out", async () => {
  const p = page();
  await p.run(0);
  assert.equal(p.sent.length, 1);
  assert.deepEqual(p.sent[0], { url: "/api/actors/you", key: KEY, body: { schema: "actor/console/v1", presence: "working" }, at: T });
  fits(p.sent[0].body);
  assert.equal(p.you.reporting(), true);
});

test("a selection counts once it has held 3 s, so stepping through reports only where you stop", async () => {
  const p = page();
  await p.run(0);
  p.you.looked(ATLAS);
  await p.run(1000);
  p.you.looked(S1);
  await p.run(1000);
  p.you.looked(LEDGER);
  await p.run(2999);
  assert.equal(p.sent.length, 1, "nothing yet");
  await p.run(1);
  assert.equal(p.sent.length, 2);
  assert.deepEqual(p.sent[1].body.did, [{ what: "looked", node: LEDGER, at: T + 5000 }]);
  p.you.looked(ATLAS);
  p.you.looked(null);
  await p.run(5000);
  assert.equal(p.sent.length, 2, "nothing selected, nothing to say");
});

test("what you do travels together, at most once a second", async () => {
  const p = page();
  await p.run(0);
  p.you.did("read", S1);
  p.you.did("handed-off", S1, { to: "codex" });
  await p.run(0);
  p.you.did("started", ATLAS, { app: "claude" });
  p.you.did("terminal", LEDGER);
  await p.run(999);
  assert.equal(p.sent.length, 1, "the first report went at once; the rest wait their second");
  await p.run(1);
  assert.equal(p.sent.length, 2);
  assert.deepEqual(p.sent[1].body.did.map((d) => [d.what, d.node, d.to ?? d.app ?? null]), [["read", S1, null], ["handed-off", S1, "codex"], ["started", ATLAS, "claude"], ["terminal", LEDGER, null]]);
  fits(p.sent[1].body);
  p.you.did("read", null);
  p.you.did("read", 42);
  await p.run(5000);
  assert.equal(p.sent.length, 2, "only node ids");
});

test("presence: idle after two minutes without input or without focus, away after ten or hidden, each change told, and a word every minute", async () => {
  const p = page();
  await p.run(0);
  await p.run(2 * MIN + 5000);
  const presences = () => p.sent.map((s) => s.body.presence);
  assert.equal(presences().at(-1), "idle");
  assert.ok(p.sent.some((s) => s.at === T + MIN && s.body.presence === "working"), "a word after a minute");
  await p.run(8 * MIN);
  assert.equal(presences().at(-1), "away");
  p.fire("pointerdown");
  await p.run(1000);
  assert.equal(presences().at(-1), "working", "back with input");
  p.doc.focused = false;
  p.fire("blur");
  await p.run(1000);
  assert.equal(presences().at(-1), "idle", "the window shows, but you're elsewhere");
  p.doc.focused = true;
  p.doc.visibilityState = "hidden";
  p.fire("visibilitychange");
  await p.run(1000);
  assert.equal(presences().at(-1), "away");
  const before = p.sent.length;
  await p.run(3 * MIN);
  assert.equal(p.sent.length, before, "no minute's word while hidden");
});

test("the desktop window hidden: away is said once and then nothing, so XenoNet has you offline ten minutes on; shown again, you're back", async () => {
  const p = page({ window: "shown" });
  await p.run(0);
  assert.equal(p.sent.at(-1).body.presence, "working");
  // A look in progress when the window hides isn't one: it never held in view.
  p.you.looked(ATLAS);
  await p.run(1000);
  p.setWindow("hidden");
  await p.run(1000);
  assert.deepEqual(p.sent.at(-1).body, { schema: "actor/console/v1", presence: "away" });
  assert.equal(p.you.hiddenMs(), 1000);
  const said = p.sent.length;
  await p.run(9 * MIN);
  assert.equal(p.sent.length, said, "no minute's word while hidden");
  // Ten minutes on, nothing goes at all, even what's done meanwhile (a
  // script driving the page, say), until the window shows.
  await p.run(MIN);
  p.you.did("read", S1);
  await p.run(5 * MIN);
  assert.equal(p.sent.length, said, "nothing more after ten minutes hidden");
  p.setWindow("shown");
  await p.run(1000);
  assert.equal(p.you.hiddenMs(), null);
  assert.equal(p.sent.length, said + 1, "shown again, a word at once");
  assert.equal(p.sent.at(-1).body.presence, "away", "still away: no input for ten minutes");
  assert.deepEqual(p.sent.at(-1).body.did?.map((d) => d.what), ["read"], "with what waited");
  p.fire("pointerdown");
  await p.run(1000);
  assert.equal(p.sent.at(-1).body.presence, "working");
  for (const s of p.sent) fits(s.body);
});

test("XenoNet started hidden (at login): its first word is away, and a minimized window is hidden too", async () => {
  const p = page({ window: "hidden" });
  await p.run(0);
  assert.deepEqual(p.sent.map((s) => s.body.presence), ["away"], "never working, not even for a moment");
  p.you.looked(ATLAS);
  await p.run(20 * MIN);
  assert.equal(p.sent.length, 1, "a selection made while hidden is no look, and nothing else goes");
  p.setWindow("shown");
  p.fire("keydown");
  await p.run(1000);
  assert.equal(p.sent.at(-1).body.presence, "working");
  p.setWindow("minimized");
  await p.run(1000);
  assert.equal(p.sent.at(-1).body.presence, "working", "only shown and hidden are words; the desktop app says hidden for minimized");
});

test("nothing you type goes out: keys only say you're there", async () => {
  const p = page();
  await p.run(0);
  for (const key of "zebra-secret") p.fire("keydown", { key, target: { value: "zebra-secret" } });
  p.you.did("read", S1);
  await p.run(10 * MIN);
  assert.ok(p.sent.length > 3);
  for (const s of p.sent) {
    fits(s.body);
    assert.doesNotMatch(JSON.stringify(s.body), /zebra|secret/);
  }
});

test("without a key (a browser tab, or a server the app didn't start), nothing goes out", async () => {
  const tab = page({ bridge: false });
  tab.you.did("read", S1);
  tab.you.looked(ATLAS);
  await tab.run(10 * MIN);
  assert.deepEqual([tab.sent.length, tab.you.reporting()], [0, false]);
  const reused = page({ keys: [] });
  for (let i = 0; i < 50; i++) reused.fire("keydown");
  await reused.run(2 * MIN + 1000);
  assert.equal(reused.sent.length, 0);
  assert.ok(reused.asked.n >= 2 && reused.asked.n <= 4, `it asks again a minute later, not on every keystroke: ${reused.asked.n}`);
});

test("a refused key is asked for again once; actors off stop it; too many waits as told", async () => {
  const p = page({ keys: ["old", "new", "newer"], answers: [{ status: 403 }, { status: 200 }] });
  p.you.did("read", S1);
  await p.run(0);
  assert.deepEqual(p.sent.map((s) => [s.key, s.body.did?.length ?? 0]), [["old", 1], ["new", 1]], "what was refused goes again with the new key");

  const refused = page({ answers: [{ status: 403 }, { status: 403 }] });
  refused.you.did("read", S1);
  await refused.run(0);
  await refused.run(MIN);
  assert.ok(refused.sent.length >= 3);
  assert.equal(refused.sent[2].body.did, undefined, "refused twice, what it held is dropped; your presence is tried again");

  const off = page({ answers: [{ status: 503 }] });
  await off.run(10 * MIN);
  assert.deepEqual([off.sent.length, off.you.reporting()], [1, false]);

  const busy = page({ answers: [{ status: 429, headers: { "retry-after": "3" } }] });
  busy.you.did("read", S1);
  await busy.run(2999);
  assert.equal(busy.sent.length, 1);
  await busy.run(1);
  assert.deepEqual([busy.sent.length, busy.sent[1].body.did?.length], [2, 1], "after three seconds, with what it held");
});

test("off the map: nothing more is sent, and on again it starts over; kept for the window", async () => {
  const p = page();
  await p.run(0);
  p.you.setShown(false);
  p.you.did("read", S1);
  p.you.looked(ATLAS);
  await p.run(5 * MIN);
  assert.equal(p.sent.length, 1);
  assert.deepEqual([p.you.shown(), p.stored.get(SWITCH)], [false, "off"]);
  p.you.setShown(true);
  await p.run(0);
  assert.equal(p.sent.length, 2);
  const later = page({ off: true });
  await later.run(5 * MIN);
  assert.deepEqual([later.sent.length, later.you.shown()], [0, false], "a page that starts off stays off");
});

test("turning yourself off the map deletes your record at once, with the window's key; a XenoNet without the route keeps it, quietly", async () => {
  const p = page({ replies: { "DELETE /api/actors/you": [{ status: 200, body: { deleted: "actor:you" } }] } });
  await p.run(0);
  p.you.looked(ATLAS);
  const done = p.you.setShown(false);
  await p.run(5 * MIN);
  assert.deepEqual(await done, { deleted: true });
  const deletes = (q) => q.calls.filter((c) => c.method === "DELETE").map((c) => [c.url, c.key]);
  assert.deepEqual(deletes(p), [["/api/actors/you", KEY]]);
  assert.equal(p.sent.length, 1, "and nothing more is reported, not even the look in progress");
  // On again: reported again, and nothing deleted.
  assert.deepEqual(await p.you.setShown(true), { deleted: false });
  await p.run(0);
  assert.equal(p.sent.length, 2);
  assert.equal(deletes(p).length, 1);

  // Before the route exists (a XenoNet from before it), off is still off.
  const older = page();
  await older.run(0);
  assert.deepEqual(await older.you.setShown(false), { deleted: false });
  await older.run(5 * MIN);
  assert.deepEqual([deletes(older).length, older.sent.length, older.you.shown()], [1, 1, false]);
  // A browser tab has no key to ask with.
  const tab = page({ bridge: false });
  assert.deepEqual(await tab.you.setShown(false), { deleted: false });
  assert.equal(tab.calls.length, 0);
});

test("the deletion waits for a report on its way, so that report can't bring your record back", async () => {
  const p = page({ hold: true, replies: { "DELETE /api/actors/you": [{ status: 200, body: { deleted: "actor:you" } }] } });
  p.you.did("read", S1);
  await p.run(0);
  const done = p.you.setShown(false);
  await p.run(2000);
  assert.deepEqual(p.reached, [], "the report hasn't landed, and the deletion waits");
  p.release();
  await p.run(1000);
  assert.deepEqual(p.reached.filter(([method]) => method !== "GET"), [["POST", "/api/actors/you"], ["DELETE", "/api/actors/you"]]);
  assert.deepEqual(await done, { deleted: true });
});

test("your record is read back with the window's key once you're reported, after a reload too, and again after what you did is taken", async () => {
  const yours = (status) => ({ status: 200, body: { schema: "actor/snapshot/v1", id: "actor:you", kind: "human", name: "You", presence: "working", status, targets: [{ id: ATLAS, relation: "watches", since: T, lastAt: T }], recent: [] } });
  const p = page({ replies: { "GET /api/actors/you": [yours("Looking at atlas-api"), yours("Reading a Claude Code session in atlas-api")] } });
  await p.run(0);
  assert.deepEqual(p.calls.map((c) => [c.method, c.url, c.key]), [["GET", "/api/actors/you", KEY]], "read once the first report is taken");
  assert.equal(p.you.record().status, "Looking at atlas-api");
  await p.run(3 * MIN);
  assert.equal(p.calls.length, 1, "a heartbeat isn't news");
  p.you.did("read", S1);
  await p.run(1000);
  assert.equal(p.calls.length, 2, "what you did is, once it's taken");
  assert.equal(p.you.record().status, "Reading a Claude Code session in atlas-api");
  // Off the map, it's let go.
  p.you.setShown(false);
  assert.equal(p.you.record(), null);
});

test("reading your record: one at a time, onRecord hears each, and a XenoNet that won't give it isn't asked again until you act", async () => {
  const heard = [];
  const p = page({ onRecord: (r) => heard.push(r?.status ?? null), replies: { "GET /api/actors/you": [{ status: 403, body: { code: "yours" } }, { status: 403, body: { code: "yours" } }] } });
  await p.run(0);
  assert.equal(p.calls.length, 2, "a refused key is asked for again once");
  await p.run(5 * MIN);
  assert.deepEqual([p.calls.length, p.you.record(), heard], [2, null, []], "and then left alone");
  p.you.did("read", S1);
  await p.run(1000);
  assert.equal(p.calls.length, 3, "until you act");
  const r = page({ onRecord: (rec) => heard.push(rec?.status ?? null), replies: { "GET /api/actors/you": [{ status: 200, body: { id: "actor:you", status: "Looking at ledger" } }] } });
  await r.run(0);
  r.you.setShown(false);
  assert.deepEqual(heard, ["Looking at ledger", null]);
});
