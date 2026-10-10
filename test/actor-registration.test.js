// Registering actors and taking their reports (core/actors/registration.js
// and reports.js; docs/design/actors/interface.md, section 2): a token that
// waits for your Allow, requests that lapse or are replaced, adding,
// rotating and removing, tokens revoked with a reason and after 30 days
// unused, the limits, the products' own rules, observers switched off and
// on, the demo's made-up request, the processes actors name, and what
// heartbeats, events and targets may say. On made-up maps and clocks, in
// stores of their own.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Actors } from "../core/actors/index.js";
import { createRegistration, REVOKED } from "../core/actors/registration.js";
import { takeHeartbeat, takeEvents, takeTargets } from "../core/actors/reports.js";
import { hashToken, isToken } from "../core/actors/tokens.js";
import { hermesSlug } from "../core/actors/names.js";
import { profileSlug } from "../core/actors/hermes.js";
import { EngineStore } from "../core/engine/store.js";
import { check, explain } from "../core/schema/index.js";
import { fromFile } from "./fixtures/schema-files.js";
import { fakeMap, HOME, ATLAS, LEDGER, SCRATCH, S1, S2, ATLAS_DIR, LEDGER_DIR } from "./fixtures/actor-map.js";

const T = 1791230000000;
const MIN = 60e3;
const HOUR = 3600e3;
const DAY = 24 * HOUR;
const ALL = ["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents"];

function valid(id, data, what = id) {
  const d = JSON.parse(JSON.stringify(data));
  const r = check(id, d, { strict: true });
  assert.ok(r.ok, `${what} doesn't fit ${id}:\n${explain(r)}`);
  assert.ok(fromFile(id).safeParse(d).success, `${what} doesn't fit schemas/${id}.json`);
}

function setup({ t = T, demo = false, alive } = {}) {
  const clock = { t };
  const now = () => clock.t;
  const store = new EngineStore(":memory:");
  const actors = new Actors({ store, now, map: fakeMap(), demo, ...(alive ? { alive } : {}) });
  const reg = createRegistration({ actors, now, home: HOME });
  return { clock, store, actors, reg };
}

// What the process check saw of who asked: a program of nova's, run from a
// shell in Terminal.
const CHAIN = [
  { pid: 430, ppid: 420, program: "/usr/local/bin/node" },
  { pid: 420, ppid: 410, program: `${HOME}/bin/wren` },
  { pid: 410, ppid: 400, program: "/bin/zsh" },
  { pid: 400, ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" },
];
const SEEN = { checked: true, chain: CHAIN, cwd: `${HOME}/Projects/docs-site` };
const throwsCode = (fn, status, code, also = () => true, message) => assert.throws(fn, (e) => e.status === status && e.code === code && also(e), message);
const lastEvent = (actors, id) => actors.snapshot(id).recent.at(-1);

// ---------- registering ----------

test("registering answers at once with a token that waits for you, kept only as its hash", () => {
  const { reg, store, actors } = setup();
  const creds = reg.register({ schema: "actor/registration/v1", name: "Wren", capabilities: ["run-commands", "edit-repos", "message-agents", "fly"], heartbeatMs: 30000, pid: 420 }, SEEN);
  valid("actor/credentials/v1", creds);
  assert.deepEqual([creds.id, creds.state, creds.expiresAt, creds.ignored], ["actor:wren", "waiting", T + 10 * MIN, ["fly"]]);
  const dump = JSON.stringify([store.db.prepare("SELECT * FROM actor_requests").all(), store.db.prepare("SELECT * FROM actors").all()]);
  assert.ok(!dump.includes(creds.token), "the token itself is never kept");
  assert.ok(dump.includes(hashToken(creds.token)));

  // It waits: it may say who it is, and isn't on the list yet.
  const auth = reg.authenticate(creds.token);
  assert.deepEqual([auth.ok, auth.state, auth.id, auth.slug], [true, "waiting", "actor:wren", "wren"]);
  valid("actor/me/v1", reg.me(auth));
  assert.deepEqual(reg.me(auth), { schema: "actor/me/v1", id: "actor:wren", state: "waiting", presence: "unknown", heartbeatMs: 30000, unread: 0 });
  assert.equal(actors.has("actor:wren"), false);

  // The card: what it asks for, and who asked, with ~ for the home folder.
  const { requests } = reg.requests();
  assert.deepEqual(requests, [
    {
      id: requests[0].id,
      actor: "actor:wren",
      name: "Wren",
      product: null,
      capabilities: ["run-commands", "edit-repos", "message-agents"],
      heartbeatMs: 30000,
      pid: 420,
      asker: { program: "/usr/local/bin/node", parents: ["~/bin/wren", "/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: "~/Projects/docs-site" },
      joins: false,
      at: T,
      expiresAt: T + 10 * MIN,
    },
  ]);
  assert.match(requests[0].id, /^[A-Za-z0-9_-]{12}$/);
  assert.equal(reg.register({ name: "Kite" }).id, "actor:kite");
  assert.equal(reg.requests().requests.find((r) => r.actor === "actor:kite").asker, null, "where XenoNet couldn't look, it says so");
});

test("the card says who asked by their executables' real paths, not the names they gave themselves", () => {
  const { reg } = setup();
  // A program that ran exec -a to read as Wren run from Terminal, and a
  // parent whose executable couldn't be read, shown by its name.
  const chain = [
    { pid: 430, ppid: 420, program: `${HOME}/bin/wren`, exe: `${HOME}/Downloads/helper` },
    { pid: 420, ppid: 410, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", exe: "/bin/bash" },
    { pid: 410, ppid: 400, program: "/usr/bin/login", exe: null },
    { pid: 400, ppid: 1, program: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal", exe: "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal" },
  ];
  reg.register({ name: "Wren" }, { checked: true, chain, cwd: null });
  assert.deepEqual(reg.requests().requests[0].asker, { program: "~/Downloads/helper", parents: ["/bin/bash", "/usr/bin/login", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: null });
});

test("Allow makes the token work, and the actor joins the list, unknown until its first heartbeat", () => {
  const { reg, actors } = setup();
  const creds = reg.register({ name: "Wren", capabilities: ["run-commands", "message-agents"] }, SEEN);
  const [{ id }] = reg.requests().requests;
  const out = reg.decide(id, "allow");
  assert.deepEqual([out.request, out.decision], [id, "allow"]);
  valid("actor/snapshot/v1", out.actor);
  assert.deepEqual([out.actor.id, out.actor.presence, out.actor.via, out.actor.capabilities, out.actor.heartbeatMs], ["actor:wren", "unknown", ["registered"], ["run-commands", "message-agents"], 60000]);
  assert.deepEqual(reg.authenticate(creds.token), { ok: true, id: "actor:wren", slug: "wren", state: "allowed" });
  valid("actor/me/v1", reg.me(reg.authenticate(creds.token)));
  assert.deepEqual(reg.requests().requests, []);
  assert.ok(actors.list().actors.some((a) => a.id === "actor:wren"));
  throwsCode(() => reg.decide(id, "allow"), 404, "not-found");
  throwsCode(() => reg.decide("nope", "deny"), 404, "not-found");
  throwsCode(() => reg.decide(id, "maybe"), 400, "bad-request");
});

test("Deny, or ten minutes without an answer, revokes the token, and it hears why", () => {
  const { reg, clock } = setup();
  const a = reg.register({ name: "Wren" }, SEEN);
  reg.decide(reg.requests().requests[0].id, "deny");
  assert.deepEqual(reg.authenticate(a.token), { ok: false, status: 401, code: "revoked", reason: "denied", error: REVOKED.denied });
  const b = reg.register({ name: "Heron" }, SEEN);
  clock.t += 10 * MIN;
  reg.tick();
  assert.deepEqual([reg.authenticate(b.token).code, reg.authenticate(b.token).reason], ["revoked", "expired"]);
  assert.deepEqual(reg.requests().requests, []);
  const c = reg.register({ name: "Osprey" }, SEEN);
  clock.t += 10 * MIN;
  assert.equal(reg.authenticate(c.token).reason, "expired", "found out when it calls, too");
  assert.deepEqual(reg.authenticate(`xenonet_actor_${"0".repeat(64)}`), { ok: false, status: 401, code: "bad-token", error: "XenoNet doesn't know that token" });
  assert.equal(reg.authenticate("hunter2").code, "bad-token");
});

test("the same program asking again takes its old request's place; another program can't take the slug", () => {
  const { reg, actors } = setup();
  const first = reg.register({ name: "Wren" }, SEEN);
  const second = reg.register({ name: "Wren" }, SEEN);
  assert.equal(reg.requests().requests.length, 1);
  assert.equal(reg.authenticate(first.token).reason, "rotated");
  assert.equal(reg.authenticate(second.token).state, "waiting");
  const other = { ...SEEN, chain: [{ ...CHAIN[0], program: "/opt/homebrew/bin/python3" }, ...CHAIN.slice(1)] };
  throwsCode(() => reg.register({ name: "Wren" }, other), 409, "taken", (e) => e.extra.suggest === "wren-2");
  const cron = { ...SEEN, chain: [CHAIN[0], { pid: 1500, ppid: 1, program: "/usr/sbin/cron" }] };
  throwsCode(() => reg.register({ name: "Wren" }, cron), 409, "taken", () => true, "the xenonet command run from somewhere else is another program");
  throwsCode(() => reg.register({ name: "Wren!", slug: "wren" }, SEEN), 409, "taken", () => true, "another name, the same slug");
  // Where XenoNet can't look, the same name asking again replaces it.
  const blind = reg.register({ name: "Kite" });
  reg.register({ name: "Kite" });
  assert.equal(reg.authenticate(blind.token).reason, "rotated");
  // A slug an actor has is taken, with a free one suggested.
  actors.upsert({ id: "actor:moth", name: "Moth", via: ["observed"] });
  throwsCode(() => reg.register({ name: "Moth" }, SEEN), 409, "taken", (e) => e.extra.suggest === "moth-2");
  assert.equal(reg.register({ name: "Moth", slug: "moth-2" }, SEEN).id, "actor:moth-2");
});

test("adding from the window allows at once; a new token stops the old; removing takes the record and what it held", () => {
  const { reg, actors, store } = setup();
  const added = reg.add({ name: "Moth", capabilities: ["message-agents", "edit-repos"], heartbeatMs: 30000, pid: 999 });
  valid("actor/credentials/v1", added);
  assert.deepEqual([added.state, "expiresAt" in added], ["allowed", false]);
  assert.equal(reg.authenticate(added.token).state, "allowed");
  assert.equal(actors.snapshot("actor:moth").heartbeatMs, 30000);
  assert.equal(actors.pidsOf("actor:moth").size, 0, "a pid from the window means nothing");

  const fresh = reg.rotate("moth");
  valid("actor/credentials/v1", fresh);
  assert.equal(reg.authenticate(added.token).reason, "rotated", "the old one stops at once");
  assert.equal(reg.authenticate(fresh.token).state, "allowed");
  throwsCode(() => reg.rotate("you"), 404, "not-found");
  throwsCode(() => reg.rotate("nobody"), 404, "not-found");
  actors.upsert({ id: "actor:kestrel", name: "Kestrel", via: ["observed"] });
  throwsCode(() => reg.rotate("kestrel"), 404, "not-found", () => true, "an observed actor has no token");

  store.claim("actor:moth", `${ATLAS_DIR}/src/`, null, T + HOUR, T);
  store.setFocus("actor:moth", "tidying", T);
  actors.record("actor:moth", { kind: "said", target: S1, count: 1, via: "registered" });
  assert.deepEqual(reg.remove("moth"), { removed: "actor:moth" });
  assert.equal(reg.authenticate(fresh.token).reason, "removed");
  assert.deepEqual([actors.has("actor:moth"), store.claims(T).length, store.focus("actor:moth"), store.lastActorEvents("actor:moth").length], [false, 0, null, 0]);
  throwsCode(() => reg.remove("moth"), 404, "not-found");
  throwsCode(() => reg.remove("you"), 400, "bad-request");
  // One XenoNet only observes isn't XenoNet's to remove, whoever asks: its
  // observer's switch takes it off the map.
  throwsCode(() => reg.remove("kestrel"), 409, "observed", (e) => !("product" in e.extra) && /^XenoNet only observes Kestrel, so it can't be removed; turn off its observer instead/.test(e.message), "an observed actor's record stays");
  actors.upsert({ id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed", "attributed"] });
  throwsCode(() => reg.remove("grok-bot", { reason: "left" }), 409, "observed", (e) => e.extra.product === "grok-bot" && e.message === "XenoNet only observes Grok Bot, so it can't be removed; turn off its observer instead, with the Grok Bot switch under observers in XenoNet's Actors panel, or xenonet actor observe grok-bot off");
  assert.deepEqual([actors.has("actor:kestrel"), actors.has("actor:grok-bot")], [true, true]);

  const kite = reg.add({ name: "Kite" });
  reg.remove("kite", { reason: "left" });
  assert.equal(reg.authenticate(kite.token).reason, "left");
  throwsCode(() => reg.add({ name: "Kite", slug: "you" }), 400, "bad-request");
});

test("a token unused for 30 days is revoked, and its actor stays until you give it a new one; revoked ones are told why for a week", () => {
  const { reg, actors, store, clock } = setup();
  const a = reg.add({ name: "Kite" });
  const b = reg.add({ name: "Moth" });
  clock.t += 20 * DAY;
  assert.equal(reg.authenticate(a.token).state, "allowed", "a call keeps it alive");
  clock.t += 20 * DAY;
  reg.tick();
  assert.equal(reg.authenticate(a.token).state, "allowed");
  assert.equal(reg.authenticate(b.token).reason, "expired", "Moth went 40 days without a call");
  assert.ok(actors.list().actors.some((x) => x.id === "actor:moth"), "it stays, to be given a new token or removed");
  assert.equal(store.actorToken("moth"), null);
  assert.equal(reg.authenticate(reg.rotate("moth").token).state, "allowed");
  clock.t += 31 * DAY;
  assert.equal(reg.authenticate(a.token).reason, "expired", "found out when it calls, too");
  clock.t += 8 * DAY;
  actors.prune();
  assert.equal(reg.authenticate(a.token).code, "bad-token", "a week after, a revoked token is just unknown");
});

test("a token unused for exactly 30 days has expired, alike to a call with it, the sweep and the window's view", () => {
  const before = setup();
  const early = before.reg.add({ name: "Kite" });
  before.clock.t = T + 30 * DAY - 1;
  before.reg.tick();
  assert.deepEqual(before.actors.tokens().map((x) => x.state), ["works"], "a millisecond before, it works");
  assert.equal(before.reg.authenticate(early.token).state, "allowed");

  const at = setup();
  const kite = at.reg.add({ name: "Kite" });
  at.reg.add({ name: "Moth" });
  at.clock.t = T + 30 * DAY;
  assert.deepEqual(at.actors.tokens().map((x) => [x.actor, x.state]), [["actor:kite", "expired"], ["actor:moth", "expired"]], "the window's view");
  assert.equal(at.reg.authenticate(kite.token).reason, "expired", "a call with it");
  at.reg.tick();
  assert.equal(at.store.actorToken("moth"), null, "and the sweep, which takes Moth's");
});

test("the limits: 16 registered actors, 4 registrations waiting and 20 an hour", () => {
  const { reg, clock } = setup();
  for (let i = 0; i < 16; i++) reg.add({ name: `Bot ${i}` });
  throwsCode(() => reg.add({ name: "Bot 16" }), 409, "full");
  throwsCode(() => reg.register({ name: "Bot 17" }, SEEN), 409, "full");
  reg.remove("bot-3");
  const asked = reg.register({ name: "Late" }, SEEN);
  assert.equal(reg.add({ name: "Bot 18" }).state, "allowed", "a place freed");
  throwsCode(() => reg.decide(reg.requests().requests[0].id, "allow"), 409, "full", () => true, "and taken again before you allowed the one waiting");
  assert.equal(reg.authenticate(asked.token).state, "waiting", "which still waits, to be denied or to lapse");

  const w = setup();
  for (let i = 0; i < 4; i++) w.reg.register({ name: `Ask ${i}` }, SEEN);
  throwsCode(() => w.reg.register({ name: "Ask 4" }, SEEN), 409, "full");
  const deny = () => w.reg.requests().requests.forEach((r) => w.reg.decide(r.id, "deny"));
  deny();
  for (let i = 4; i < 20; i++) {
    w.reg.register({ name: `Ask ${i}` }, SEEN);
    deny();
  }
  throwsCode(() => w.reg.register({ name: "Ask 20" }, SEEN), 429, "too-many", (e) => e.extra.retryAfterMs === HOUR);
  w.clock.t += HOUR;
  assert.equal(w.reg.register({ name: "Ask 21" }, SEEN).state, "waiting", "an hour on");
  assert.ok(clock.t === T);
});

test("names, slugs and heartbeats asked for are checked", () => {
  const { reg } = setup();
  throwsCode(() => reg.register({ name: "XenoNet" }, SEEN), 400, "bad-request", (e) => /reads as you or as XenoNet/.test(e.message));
  throwsCode(() => reg.register({ name: "Claude Code" }, SEEN), 400, "bad-request");
  throwsCode(() => reg.register({ name: "Wren", slug: "events" }, SEEN), 400, "bad-request");
  throwsCode(() => reg.register({ name: "Wren", slug: "hermes" }, SEEN), 400, "bad-request", (e) => /Hermes Agent's/.test(e.message));
  throwsCode(() => reg.register({ name: "Wren", heartbeatMs: 1000 }, SEEN), 400, "bad-request", (e) => /5,000 to 3,600,000/.test(e.message));
  throwsCode(() => reg.register({ name: "Wren", heartbeatMs: HOUR + 1 }, SEEN), 400, "bad-request");
  assert.equal(reg.register({ name: "Café Bot" }, SEEN).id, "actor:cafe-bot");
});

test("products: dots don't register; Hermes's actor takes its own slug, from under Hermes, and joins the one XenoNet observes", () => {
  const { reg, actors } = setup();
  throwsCode(() => reg.register({ name: "Anything", product: { id: "dots" } }, SEEN), 403, "not-allowed");
  actors.upsert({ id: "actor:hermes", name: "Hermes", product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }, via: ["observed", "attributed"], heartbeatMs: 60000 });
  actors.record("actor:hermes", { kind: "ran", target: ATLAS, via: "attributed" });
  const h = reg.register({ name: "Hermes", product: { id: "hermes-agent", version: "0.22.0" }, capabilities: ["spawn-sessions", "run-commands"] }, SEEN);
  assert.equal(h.id, "actor:hermes");
  const card = reg.requests().requests[0];
  assert.deepEqual([card.joins, card.product], [true, { id: "hermes-agent", vendor: "Nous Research", version: "0.22.0" }]);
  reg.decide(card.id, "allow");
  const s = actors.snapshot("actor:hermes");
  assert.deepEqual([s.via, s.product.version, s.recent.length], [["observed", "attributed", "registered"], "0.21.5", 1], "the product as XenoNet observes it, and its history");
  assert.equal(reg.register({ name: "Hermes", slug: "hermes-work", product: { id: "hermes-agent" } }, SEEN).id, "actor:hermes-work", "a profile's actor");
  throwsCode(() => reg.register({ name: "Hermes", slug: "wren", product: { id: "hermes-agent" } }, SEEN), 400, "bad-request");
  const own = reg.register({ name: "Gizmo", product: { id: "gizmo-agent", vendor: "Gizmo Labs", version: "1.0" } }, SEEN);
  assert.deepEqual(reg.requests().requests.find((r) => r.actor === own.id).product, { id: "gizmo-agent", vendor: "Gizmo Labs", version: "1.0" }, "a product XenoNet doesn't know is the actor's own word");
  throwsCode(() => reg.register({ name: "Gadget", product: { id: "gadget", vendor: `sk-${"x".repeat(30)}` } }, SEEN), 400, "bad-request");
  // Leaving: a product's actor XenoNet observes stays, as observed.
  assert.deepEqual(reg.remove("hermes", { reason: "left" }), { removed: "actor:hermes", observed: true });
  assert.deepEqual([actors.snapshot("actor:hermes").via, actors.snapshot("actor:hermes").recent.length], [["observed", "attributed"], 1]);
  assert.equal(reg.authenticate(h.token).reason, "left");
});

test("a Hermes plugin registering for a profile takes the slug Hermes's observer gave that profile's actor, and joins it", () => {
  const { reg, actors } = setup();
  for (const profile of ["work", "Work", "my_work"]) {
    const id = `actor:${profileSlug(profile)}`;
    actors.upsert({ id, name: "Hermes (a profile)", product: { id: "hermes-agent", vendor: "Nous Research", version: null }, via: ["observed", "attributed"], heartbeatMs: 60000 });
    const asked = reg.register({ name: "Hermes", slug: hermesSlug(profile), product: { id: "hermes-agent" } }, SEEN);
    assert.equal(asked.id, id, profile);
    assert.equal(reg.requests().requests.find((r) => r.actor === id).joins, true, `${profile}: Allow would make it the actor XenoNet observes`);
  }
});

test("a product of an actor's own can't take the name of XenoNet, a known app or product, or a maker, by its id or its vendor", () => {
  const { reg } = setup();
  const cyrillicO = String.fromCodePoint(0x043e);
  for (const [product, reads] of [
    [{ id: "claude-code", vendor: "Anthropic", version: "2.1.0" }, /the product's id reads as Claude Code/],
    [{ id: "xenonet" }, /the product's id reads as you or as XenoNet/],
    [{ id: "hermes" }, /the product's id reads as Hermes Agent/],
    [{ id: "openai" }, /the product's id reads as OpenAI/],
    [{ id: "acme-sync", vendor: "Anthropic" }, /the product's vendor reads as Anthropic/],
    [{ id: "acme-sync", vendor: `Anthr${cyrillicO}pic` }, /the product's vendor reads as Anthropic/],
    [{ id: "acme-sync", vendor: "Nous Research" }, /the product's vendor reads as Nous Research/],
    [{ id: "acme-sync", vendor: "Anysphere" }, /the product's vendor reads as Anysphere, who doesn't make this actor/],
    [{ id: "acme-sync", vendor: "ANY-SPHERE" }, /the product's vendor reads as Anysphere/],
    [{ id: "acme-sync", vendor: "XenoNet" }, /the product's vendor reads as you or as XenoNet/],
    [{ id: "acme-sync", vendor: "Cursor" }, /the product's vendor reads as Cursor/],
    // Shown quoted, so a vendor or version can't close the quote.
    [{ id: "acme-sync", vendor: "Acme\u201d by \u201cAnthropic" }, /the product's vendor can't hold quotation marks/],
    [{ id: "acme-sync", vendor: 'Acme" by "Tools' }, /the product's vendor can't hold quotation marks/],
    [{ id: "acme-sync", version: "2.1\\0" }, /the product's version can't hold quotation marks or backslashes/],
  ]) throwsCode(() => reg.register({ name: "Helper", product }, SEEN), 400, "bad-request", (e) => reads.test(e.message), JSON.stringify(product));
  throwsCode(() => reg.add({ name: "Helper", product: { id: "codex" } }), 400, "bad-request", (e) => /reads as Codex/.test(e.message), "nor from the window");
  const fine = reg.register({ name: "Helper", product: { id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" } }, SEEN);
  assert.deepEqual(reg.requests().requests.find((r) => r.actor === fine.id).product, { id: "acme-sync", vendor: "Acme Tools", version: "2.1.0" });
});

test("a product id XenoNet doesn't know joins nothing: another program sending Wren's finds its slug taken", () => {
  const { reg, actors } = setup();
  const wren = reg.register({ name: "Wren", product: { id: "acme-sync", vendor: "Acme" }, capabilities: ["run-commands"] }, SEEN);
  reg.decide(reg.requests().requests[0].id, "allow");
  assert.equal(actors.info("actor:wren").product.id, "acme-sync", "its product, which anyone can read in its snapshot");
  const other = { ...SEEN, chain: [{ ...CHAIN[0], program: "/opt/homebrew/bin/python3" }, ...CHAIN.slice(1)] };
  const ask = { name: "Wren", slug: "wren", product: { id: "acme-sync" }, capabilities: ALL };
  throwsCode(() => reg.register(ask, other), 409, "taken", (e) => e.extra.suggest === "wren-2");
  throwsCode(() => reg.add(ask), 409, "taken", () => true, "nor does the window add it over Wren");
  assert.equal(reg.authenticate(wren.token).state, "allowed", "Wren's token still works");
  assert.deepEqual(actors.info("actor:wren").capabilities, ["run-commands"]);
  // A registration that waited while the slug was free can't take it
  // once an actor has it, whatever product that one gave.
  const heron = reg.register({ name: "Heron", product: { id: "acme-sync" } }, SEEN);
  actors.upsert({ id: "actor:heron", name: "Heron", product: { id: "acme-sync", vendor: null, version: null }, via: ["registered"], heartbeatMs: 60000, capabilities: [], allowedAt: T, at: T });
  const card = reg.requests().requests.find((r) => r.actor === heron.id);
  assert.equal(card.joins, false, "the card doesn't say it joins");
  throwsCode(() => reg.decide(card.id, "allow"), 409, "taken");
});

test("products: Grok Bot and Muse register from under their apps, and each joins the actor XenoNet observes, which can then be messaged", () => {
  const { reg, actors } = setup();
  for (const [id, slug, name, vendor] of [["grok-bot", "grok-bot", "Grok Bot", "SpaceXAI"], ["muse", "muse", "Muse", "Meta"]]) {
    actors.upsert({ id: `actor:${slug}`, name, product: { id, vendor, version: "1.0" }, via: ["observed"], heartbeatMs: null });
    const asked = reg.register({ name, product: { id }, capabilities: ["message-agents"], heartbeatMs: 3600e3 }, SEEN);
    assert.equal(asked.id, `actor:${slug}`, "its own slug");
    const card = reg.requests().requests.find((r) => r.actor === asked.id);
    assert.deepEqual([card.joins, card.product.id], [true, id], "it joins the actor XenoNet observes");
    reg.decide(card.id, "allow");
    const s = actors.snapshot(`actor:${slug}`);
    assert.deepEqual([s.via, s.product.version, s.capabilities], [["observed", "registered"], "1.0", ["message-agents"]]);
    assert.equal(actors.isRegistered(`actor:${slug}`), true);
    // Leaving puts it back as XenoNet observes it.
    assert.deepEqual(reg.remove(slug, { reason: "left" }), { removed: `actor:${slug}`, observed: true });
    assert.equal(actors.isRegistered(`actor:${slug}`), false);
  }
});

test("gateway tokens: made in the window, apart from an actor's own token; Muse, only observed, is registered by it; each works only where it belongs", () => {
  const { reg, store, actors, clock } = setup();
  actors.upsert({ id: "actor:muse", name: "Muse", product: { id: "muse", vendor: "Meta", version: "7.0" }, via: ["observed"], heartbeatMs: null });
  const creds = reg.gateway("muse");
  valid("actor/gateway-credentials/v1", creds, "a gateway token");
  assert.match(creds.token, /^xenonet_gateway_[0-9a-f]{64}$/);
  const s = actors.snapshot("actor:muse");
  assert.deepEqual([s.via, s.capabilities, actors.isRegistered("actor:muse")], [["observed", "registered"], ["message-agents"], true], "joins the actor XenoNet observes, and may message agents");
  assert.deepEqual(reg.authenticateGateway(creds.token), { ok: true, id: "actor:muse", slug: "muse", state: "allowed" });
  assert.equal(reg.authenticate(creds.token).code, "bad-token", "never on this machine's API");
  assert.ok(!JSON.stringify(store.db.prepare("SELECT * FROM actors").all()).includes(creds.token), "only its hash is kept");
  // An actor's own token is refused at the gateway, by name.
  const wren = reg.add({ name: "Wren", capabilities: ["message-agents"] });
  assert.equal(reg.authenticateGateway(wren.token).code, "wrong-token");
  assert.equal(reg.authenticateGateway("xenonet_gateway_" + "0".repeat(64)).code, "bad-token");
  // A registered actor gets one too, and keeps its own token.
  const wrenGate = reg.gateway("wren");
  assert.equal(reg.authenticate(wren.token).ok, true);
  assert.equal(reg.authenticateGateway(wrenGate.token).id, "actor:wren");
  // A new one replaces the old at once.
  const again = reg.gateway("muse");
  assert.deepEqual([reg.authenticateGateway(creds.token).reason, reg.authenticateGateway(again.token).ok], ["rotated", true]);
  // The window's view: Muse has no token of its own, and isn't expired for it.
  const view = Object.fromEntries(actors.tokens(clock.t).map((t) => [t.actor, t]));
  assert.deepEqual([view["actor:muse"].state, view["actor:muse"].gateway.state, view["actor:wren"].state], ["none", "works", "works"]);
  assert.deepEqual(store.expiredActorTokens(), []);
  // Taken away: it stops; its own token stands.
  assert.deepEqual(reg.dropGateway("wren"), { actor: "actor:wren", gateway: false });
  assert.deepEqual([reg.authenticateGateway(wrenGate.token).reason, reg.authenticate(wren.token).ok], ["removed", true]);
  throwsCode(() => reg.dropGateway("wren"), 404, "not-found");
  // Unused for 30 days, it expires.
  clock.t += 30 * DAY;
  reg.tick(clock.t);
  assert.equal(reg.authenticateGateway(again.token).reason, "expired");
  // Only a product that may register, or an actor already registered.
  throwsCode(() => reg.gateway("nobody"), 404, "not-found");
  throwsCode(() => reg.gateway("you"), 400, "bad-request");
});

test("gateway tokens go with their actor when you remove it; a product's actor stays, as observed", () => {
  const { reg, actors } = setup();
  actors.upsert({ id: "actor:muse", name: "Muse", product: { id: "muse", vendor: "Meta", version: "7.0" }, via: ["observed"], heartbeatMs: null });
  const creds = reg.gateway("muse");
  assert.deepEqual(reg.remove("muse"), { removed: "actor:muse", observed: true });
  assert.equal(reg.authenticateGateway(creds.token).reason, "removed");
  assert.equal(actors.isRegistered("actor:muse"), false);
});

test("observers: on until you turn one off, which hides an actor known only that way, before XenoNet has seen it too", () => {
  const { reg, actors, store, clock } = setup();
  assert.deepEqual(reg.observers().observers.map((o) => [o.product, o.name, o.on, o.actors]), [["hermes-agent", "Hermes Agent", true, []], ["grok-bot", "Grok Bot", true, []], ["muse", "Muse", true, []]]);
  assert.deepEqual(reg.observe("grok-bot", false), { product: "grok-bot", on: false, actors: ["actor:grok-bot"] });
  assert.equal(reg.observing("grok-bot"), false);
  assert.ok(!actors.list().actors.some((a) => a.id === "actor:grok-bot"), "the actor it would be isn't listed");
  actors.upsert({ id: "actor:hermes", name: "Hermes", product: { id: "hermes-agent", vendor: "Nous Research", version: null }, via: ["observed"], heartbeatMs: 60000 });
  actors.report("actor:hermes", { instance: "gateway", presence: "idle", via: "observed" });
  assert.ok(actors.list().actors.some((a) => a.id === "actor:hermes"));
  assert.ok(actors.engineIds(clock.t - HOUR).includes("actor:hermes"));
  reg.observe("hermes-agent", false);
  assert.ok(!actors.list().actors.some((a) => a.id === "actor:hermes"), "off, it leaves the list");
  assert.ok(!actors.engineIds(clock.t - HOUR).includes("actor:hermes"), "and the agents' peers");
  // Kept in engine.sqlite, across a restart.
  const again = new Actors({ store, now: () => clock.t, map: fakeMap() });
  assert.equal(createRegistration({ actors: again, now: () => clock.t }).observing("hermes-agent"), false);
  reg.observe("hermes-agent", true);
  assert.ok(actors.list().actors.some((a) => a.id === "actor:hermes"));
  throwsCode(() => reg.observe("dots", false), 404, "not-found");
  throwsCode(() => reg.observe("hermes-agent", "off"), 400, "bad-request");
});

test("actor-request: a made-up program asks, in the demo and test instances only", () => {
  assert.equal(setup().reg.simulate("actor-request"), null, "outside the demo, nothing is made up");
  const { reg } = setup({ demo: true });
  const one = reg.simulate("actor-request");
  assert.deepEqual([one.ok, one.kind, one.a, isToken(one.token), one.expiresAt], [true, "actor-request", "actor:heron", true, T + 10 * MIN]);
  const [card] = reg.requests().requests;
  assert.deepEqual([card.id, card.name, card.capabilities, card.asker], [one.request, "Heron", ["run-commands", "edit-repos", "message-agents"], { program: "~/bin/heron", parents: ["/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: "~/Projects/heron" }]);
  assert.equal(reg.authenticate(one.token).state, "waiting");
  assert.equal(reg.simulate("actor-request").a, "actor:heron-2", "another one is another program");
  assert.equal(reg.simulate("actor-request", { a: "Osprey" }).a, "actor:osprey");
  assert.deepEqual([reg.simulate("actor-request", { a: "XenoNet" }).ok, reg.simulate("actor-request", { a: "XenoNet" }).code], [false, "bad-request"]);
  reg.simulate("actor-request", { a: "Kite" });
  assert.deepEqual([reg.simulate("actor-request", { a: "Wren" }).ok, reg.simulate("actor-request", { a: "Wren" }).code], [false, "full"], "4 at most");
  assert.equal(reg.simulate("actor-spawn"), null);
});

test("the processes an actor names: one that ends takes its instance offline, and the one it registered as all of it", () => {
  const dead = new Set();
  const { reg, actors } = setup({ alive: (pid) => !dead.has(pid) });
  reg.register({ name: "Wren", pid: 420 }, SEEN);
  reg.decide(reg.requests().requests[0].id, "allow");
  assert.deepEqual([...actors.pidsOf("actor:wren")], [420]);
  actors.report("actor:wren", { instance: "main", presence: "working", pid: 430, via: "registered" });
  actors.report("actor:wren", { instance: "cli", presence: "working", pid: 431, via: "registered" });
  dead.add(431);
  actors.tick();
  assert.equal(actors.snapshot("actor:wren").presence, "working", "its main instance is still there");
  dead.add(420);
  actors.tick();
  assert.deepEqual([actors.snapshot("actor:wren").presence, lastEvent(actors, "actor:wren").detail], ["offline", "Went offline"]);
});

// ---------- what a registered actor reports ----------

function wren(capabilities = ALL) {
  const s = setup();
  s.actors.upsert({ id: "actor:wren", name: "Wren", via: ["registered"], heartbeatMs: 60000, capabilities, allowedAt: T });
  return s;
}

test("a heartbeat: presence, status, the folder it works in, and a status like a key refused", () => {
  const { actors } = wren();
  const id = "actor:wren";
  takeHeartbeat(actors, id, { instance: "gateway", presence: "working", status: "Rebuilding the docs", cwd: `${ATLAS_DIR}/docs`, pid: 4242 });
  const s = actors.snapshot(id);
  assert.deepEqual([s.presence, s.status], ["working", "Rebuilding the docs"]);
  assert.deepEqual([actors.engineView(id).cwd, actors.engineView(id).place], [`${ATLAS_DIR}/docs`, { id: ATLAS, name: "atlas-api", kind: "project", path: ATLAS_DIR }]);
  assert.deepEqual([...actors.pidsOf(id)], [4242]);
  throwsCode(() => takeHeartbeat(actors, id, { status: `token ghp_${"x".repeat(30)}` }), 400, "bad-request", (e) => /key or a token/.test(e.message));
  throwsCode(() => takeHeartbeat(actors, id, { cwd: "docs" }), 400, "bad-request");
  takeHeartbeat(actors, id, { instance: "gateway", cwd: "/tmp/elsewhere" });
  assert.deepEqual([actors.engineView(id).cwd, actors.engineView(id).place], [null, null], "a folder outside every place XenoNet knows is left out");
  takeHeartbeat(actors, id, { instance: "gateway", status: "" });
  assert.equal(actors.snapshot(id).status, null, "an empty status clears it");
  takeHeartbeat(actors, id, { instance: "gateway", presence: "offline" });
  assert.equal(actors.snapshot(id).presence, "offline", "goodbye");
  takeHeartbeat(actors, id, {});
  assert.equal(actors.snapshot(id).presence, "idle", "one without an instance is its main one, there");
});

test("events as facts, each taken or dropped on its own, in XenoNet's words", async () => {
  const { actors, clock } = wren();
  const id = "actor:wren";
  const r = await takeEvents(actors, id, [
    { kind: "ran", path: ATLAS_DIR, count: 3, ref: "run-1" },
    { kind: "edited", path: ATLAS_DIR, paths: [`${ATLAS_DIR}/src/app.ts`, `${ATLAS_DIR}/src/routes.ts`] },
    { kind: "spawned", target: S1 },
    { kind: "messaged", platform: "telegram" },
    { kind: "finished", subject: "job", at: T - 1000 },
    { kind: "said" },
    { kind: "ran", ref: "run-1" },
    { kind: "ran", at: T - DAY - 1 },
    { kind: "ran", path: "/tmp/elsewhere" },
    { kind: "edited", paths: ["/etc/hosts"] },
    { kind: "ran", target: "project:/users/nova/projects/nowhere" },
    { kind: "ran", target: "actor:moth" },
    { kind: "spawned", app: "claude", path: LEDGER_DIR },
    { kind: "messaged", platform: "carrier pigeon" },
    { kind: "ran", at: T + HOUR },
    { kind: "edited", paths: ["relative/x.ts"] },
    { kind: "spawned", target: "claude:x" },
    { kind: "edited", paths: [`${LEDGER_DIR}/close.md`] },
  ]);
  assert.equal(r.accepted, 9);
  assert.deepEqual(r.dropped.map((d) => d.index), [5, 6, 7, 8, 9, 10, 11, 15, 16]);
  for (const d of r.dropped) assert.ok(d.reason.length > 10, JSON.stringify(d));
  const recent = actors.snapshot(id).recent;
  assert.deepEqual(recent.map((e) => e.detail), [
    "A scheduled job ran",
    "Ran 3 commands in atlas-api",
    "Came online",
    "Edited app.ts and 1 more file in atlas-api",
    "Started a Claude Code session in atlas-api",
    "Sent a message on Telegram",
    "Started a Claude Code session in ledger",
    "Sent a message",
    "Ran a command",
    "Edited close.md in ledger",
  ]);
  assert.ok(recent.every((e) => e.via === "registered"));
  for (const e of recent) valid("actor/event/v1", e, e.detail);
  assert.equal(recent.filter((e) => e.kind !== "presence").length, 9);
  assert.equal(recent.find((e) => e.detail === "Ran a command").at, T, "a time ahead of XenoNet's is now");
  assert.deepEqual(actors.snapshot(id).targets.map((t) => [t.relation, t.id]).sort(), [["edited", ATLAS], ["edited", LEDGER], ["spawned", S1], ["works-on", ATLAS], ["works-on", LEDGER]]);
  assert.equal(actors.snapshot(id).presence, "working", "an action reported is working, for a while");
  // A retry never counts twice.
  const again = await takeEvents(actors, id, [{ kind: "ran", ref: "run-1" }]);
  assert.deepEqual([again.accepted, again.dropped], [0, [{ index: 0, reason: "an event with that ref was reported already" }]]);
  // The engine holds its edits for half an hour, as an agent's.
  assert.deepEqual(actors.engineView(id).edits.map((e) => e.path).sort(), [`${ATLAS_DIR}/src/app.ts`, `${ATLAS_DIR}/src/routes.ts`, `${LEDGER_DIR}/close.md`]);
  clock.t += 31 * MIN;
  assert.deepEqual(actors.engineView(id).edits, []);
});

test("each kind needs the capability you allowed", async () => {
  const { actors } = wren(["message-agents"]);
  const r = await takeEvents(actors, "actor:wren", [{ kind: "spawned", target: S1 }, { kind: "edited", path: ATLAS_DIR }, { kind: "ran" }, { kind: "messaged" }, { kind: "finished", subject: "task" }, { kind: "failed", subject: "turn" }]);
  assert.equal(r.accepted, 2);
  assert.deepEqual(r.dropped.map((d) => d.reason), [
    "spawned needs the spawn-sessions capability, which it wasn't allowed",
    "edited needs the edit-repos capability, which it wasn't allowed",
    "ran needs the run-commands capability, which it wasn't allowed",
    "messaged needs the message-people capability, which it wasn't allowed (messaged is a message to someone outside XenoNet, by email or a chat app; to write to an SI or another actor, use xenonet actor send)",
  ]);
});

test("a session it says it spawned must run under its own processes, where XenoNet can tell", async () => {
  const { actors, store } = wren();
  const id = "actor:wren";
  store.pin(S1, { pid: 900, start: 1 }, true, T);
  store.pin(S2, { pid: 901, start: 1 }, true, T);
  const lineage = async (pid) => ({ 900: [900, 800, 400], 901: [901, 420, 410, 400], 902: [] })[pid] ?? [];
  const own = new Set([420]);
  const theirs = await takeEvents(actors, id, [{ kind: "spawned", target: S1 }], { own, lineage });
  assert.deepEqual(theirs.dropped, [{ index: 0, reason: "that session runs under a process outside this actor's own" }]);
  assert.equal((await takeEvents(actors, id, [{ kind: "spawned", target: S2 }], { own, lineage })).accepted, 1, "its own");
  assert.equal((await takeEvents(actors, id, [{ kind: "spawned", target: S1 }], { own, lineage: null })).accepted, 1, "where XenoNet can't look");
  assert.equal((await takeEvents(actors, id, [{ kind: "spawned", target: S1 }], { own: null, lineage })).accepted, 1, "an actor whose processes XenoNet doesn't know");
  const gone = "claude:0190d3e0-0000-7000-8000-00000000cc03";
  store.pin(gone, { pid: 902, start: 1 }, true, T);
  assert.equal((await takeEvents(actors, id, [{ kind: "spawned", target: gone }], { own, lineage })).accepted, 1, "a session whose process ended");
  const t = await takeTargets(actors, id, { targets: [{ id: S1, relation: "spawned" }] }, { own, lineage });
  assert.equal(t.dropped[0].reason, "that session runs under a process outside this actor's own", "targets are held to it too");
});

test("targets: the relations an actor may set, pointing at what they point at, merged, removed or replaced", async () => {
  const { actors } = wren();
  const id = "actor:wren";
  const r = await takeTargets(actors, id, {
    targets: [
      { path: ATLAS_DIR, relation: "works-on" },
      { id: S1, relation: "spawned" },
      { id: S2, relation: "messaged" },
      { id: S2 },
      { id: "actor:moth", relation: "watches" },
      { path: "/tmp/x", relation: "watches" },
      { id: LEDGER, relation: "watches" },
      { id: S2, relation: "reviews" },
      { relation: "watches" },
      { id: SCRATCH, relation: "edited" },
      { id: SCRATCH, relation: "flies" },
    ],
  });
  assert.equal(r.accepted, 5);
  assert.deepEqual(r.dropped.map((d) => d.index), [2, 3, 4, 5, 8, 10]);
  assert.match(r.dropped[0].reason, /engine's/);
  assert.equal(r.dropped[1].reason, "works-on points at a project or a folder, not a session");
  const now = () => actors.targetsOf(id).map((t) => `${t.relation} ${t.id}`).sort();
  assert.deepEqual(now(), [`edited ${SCRATCH}`, `reviews ${S2}`, `spawned ${S1}`, `watches ${LEDGER}`, `works-on ${ATLAS}`]);
  await takeTargets(actors, id, { remove: [{ id: LEDGER }, { path: ATLAS_DIR, relation: "works-on" }, { id: "project:/nowhere" }] });
  assert.deepEqual(now(), [`edited ${SCRATCH}`, `reviews ${S2}`, `spawned ${S1}`]);
  actors.said({ from: id, to: [S2], ts: T });
  await takeTargets(actors, id, { targets: [{ id: ATLAS, relation: "watches" }], replace: true });
  assert.deepEqual(now(), [`messaged ${S2}`, `watches ${ATLAS}`], "replace is the whole list of what it sets; the engine's stay");
  const limited = wren(["message-agents"]);
  const d = await takeTargets(limited.actors, id, { targets: [{ id: S1, relation: "spawned" }, { id: ATLAS, relation: "edited" }, { id: ATLAS, relation: "works-on" }] });
  assert.deepEqual([d.accepted, d.dropped.map((x) => x.index)], [1, [0, 1]]);
});
