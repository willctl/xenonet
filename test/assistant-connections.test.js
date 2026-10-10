// Remote actors' connections and grants (core/actors/connections.js): what
// pairing makes and refuses, the actor it joins or makes without touching
// what else XenoNet knows of it, revoking, rotating, changing scope, turning
// all of it off, the tombstones that outlast the database, a state folder
// that moves to another machine, and how account links stay apart from
// authentication.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { world, ask, T, DAY, MIN, ATLAS, LEDGER, SECRET } from "./fixtures/assistant.js";
import { readCredential, isDisabled, markDisabled, markRevoked } from "../core/assistant/credentials.js";
import { createRegistration } from "../core/actors/registration.js";
import { linkState, recordVerified, addExternalRef } from "../core/actors/account-links.js";
import { REMOTE_LEASE_MS } from "../core/actors/index.js";
import { createRemotePresence } from "../core/actors/remote-presence.js";
import { check, explain } from "../core/schema/index.js";

const pairIn = (w, over = {}) => w.connections.pair(ask(over), { places: w.places, identities: w.identities });
const tokenOf = (w, grant) => readCredential(w.dir, grant)?.token;

test("pairing a dot makes actor:dots, a connection and a grant, and writes its credential for the adapter only", () => {
  const w = world();
  try {
    const v = pairIn(w);
    assert.ok(check("actor/connection/v1", v).ok, explain(check("actor/connection/v1", v)));
    assert.equal(v.actor_id, "actor:dots");
    assert.deepEqual([v.state, v.data_mode, v.labels, v.lookback_days, v.revision, v.bridge], ["active", "real", false, 30, 1, "not_connected"]);
    assert.equal(Date.parse(v.expires_at) - Date.parse(v.created_at), 30 * DAY, "30 days unless you choose");
    assert.deepEqual(v.projects.map((p) => [p.place, p.name]), [[ATLAS, "atlas-api"]]);
    assert.match(v.projects[0].project_id, /^prj_[0-9a-f-]{36}$/);
    assert.deepEqual(v.launch, { command: "xenonet", args: ["mcp", "assistant", "--grant", v.grant_id] });
    assert.doesNotMatch(JSON.stringify(v), /xenonet_assistant_/, "never the credential");
    const token = tokenOf(w, v.grant_id);
    assert.match(token, /^xenonet_assistant_[0-9a-f]{64}$/);
    assert.equal(w.store.grant(v.grant_id).token_hash.length, 64, "only its hash");
    assert.ok(!JSON.stringify(w.store.grants()).includes(token));
    const snap = w.actors.snapshot("actor:dots");
    assert.deepEqual(snap.via, ["remote"]);
    assert.deepEqual(snap.remote, { connections: 1, active: true, bridge: "not_connected", accountLink: "unverified", permissions: ["read"], transports: ["openai-secure-mcp-tunnel"] });
    assert.deepEqual(snap.capabilities, [], "read permission isn't a capability");
    assert.equal(snap.identity, null);
    assert.ok(check("actor/snapshot/v1", JSON.parse(JSON.stringify(snap)), { strict: true }).ok);
    assert.ok(w.actors.list().actors.some((a) => a.id === "actor:dots"), "listed while active");
    assert.equal(w.actors.isRegistered("actor:dots"), false, "it doesn't report itself");
    assert.equal(w.actors.registeredCount(), 0);
    assert.deepEqual(w.actors.tokens(), [], "no legacy token to show");
    assert.deepEqual(w.store.expiredActorTokens(), [], "never shown as an expired token");
    assert.equal(w.store.audit({}).find((r) => r.operation === "pair")?.kind, "owner");
  } finally {
    w.done();
  }
});

test("a second dot is another actor; a grant without verified audience reads only made-up data", () => {
  const w = world();
  try {
    pairIn(w);
    const second = pairIn(w, { label: "Work dot", name: "Work dot", audience: { description: "Anyone in my workspace", verification: "unverified", scope: "authenticated_user" } });
    assert.match(second.actor_id, /^actor:dots-[0-9a-f]{4}$/);
    assert.equal(second.data_mode, "fixture");
    assert.equal(w.actors.snapshot(second.actor_id).name, "Work dot");
    const third = pairIn(w, { actor: "actor:dots", label: "Laptop" });
    assert.equal(third.actor_id, "actor:dots", "one actor, two connections");
    assert.equal(w.actors.snapshot("actor:dots").remote.connections, 2);
  } finally {
    w.done();
  }
});

test("pairing refuses what it can't stand behind", () => {
  const w = world();
  try {
    const refused = (over, code, re) => assert.throws(() => pairIn(w, over), (err) => err.code === code && re.test(err.message));
    refused({ label: "" }, "bad-request", /needs a name/);
    refused({ label: "sk-live-0123456789abcdef0123456789abcdef" }, "bad-request", /key or a token/);
    refused({ transport: "carrier-pigeon" }, "bad-request", /reaches XenoNet through/);
    refused({ product: "muse" }, "bad-request", /can't be paired/);
    refused({ product: "grok-bot", transport: "openai-secure-mcp-tunnel" }, "bad-request", /reaches XenoNet through/);
    refused({ projects: ["project:/nowhere"] }, "bad-request", /from the map/);
    refused({ days: 365 }, "bad-request", /lasts/);
    refused({ audience: { description: "x", verification: "trust-me", scope: "owner_endpoint" } }, "bad-request", /verified who can call/);
    refused({ accountLink: { state: "verified" } }, "bad-request", /only its provider can verify/);
    refused({ accountLink: { state: "owner_asserted", identity: "chatgpt:someone-else" } }, "bad-request", /identities XenoNet knows/);
    refused({ actor: "actor:you" }, "not-found", /no such actor/);
    refused({ product: null, name: "Claude" }, "bad-request", /reads as Claude/);
    assert.equal(w.store.grants().length, 0, "nothing made");
    assert.equal(fs.existsSync(path.join(w.dir, "credentials")) ? fs.readdirSync(path.join(w.dir, "credentials")).length : 0, 0, "and no credential left behind");
  } finally {
    w.done();
  }
});

test("a credential that can't be kept private makes nothing", () => {
  const w = world();
  try {
    const leaky = w.make({ credentialIo: { protect: () => {}, readers: () => ["S-1-1-0"] } });
    assert.throws(() => leaky.pair(ask(), { places: w.places, identities: w.identities }), /couldn't keep the grant's credential private/);
    assert.equal(w.store.grants().length, 0);
    assert.equal(w.actors.has("actor:dots"), false);
  } finally {
    w.done();
  }
});

test("authenticating: the right credential hears how its grant stands; a wrong one is refused like an unknown grant", () => {
  const w = world();
  try {
    const v = pairIn(w);
    const token = tokenOf(w, v.grant_id);
    const ok = w.connections.authenticate({ grant: v.grant_id, token });
    assert.deepEqual([ok.ok, ok.actorId, ok.revision], [true, "actor:dots", 1]);
    const wrong = `xenonet_assistant_${"0".repeat(64)}`;
    assert.deepEqual(w.connections.authenticate({ grant: v.grant_id, token: wrong }), { ok: false, code: "NOT_AUTHORIZED", reason: "wrong-token" });
    assert.equal(w.connections.authenticate({ grant: "gr_" + "1".repeat(24), token }).code, "NOT_AUTHORIZED");
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token: "xenonet_actor_" + "a".repeat(64) }).code, "NOT_AUTHORIZED", "an actor's legacy token is no grant's credential");
    w.clock.add(31 * DAY);
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token }).code, "GRANT_EXPIRED");
  } finally {
    w.done();
  }
});

test("revoking ends one grant for good, leaves the actor and its other connections, and a tombstone outlasts the database", () => {
  const w = world();
  try {
    const a = pairIn(w);
    const b = pairIn(w, { actor: "actor:dots", label: "Laptop" });
    const heard = [];
    w.connections.on("revoked", (e) => heard.push(e.grant));
    const tokenA = tokenOf(w, a.grant_id);
    assert.deepEqual(w.connections.revoke(a.grant_id), { revoked: a.grant_id, already: false });
    assert.deepEqual(heard, [a.grant_id]);
    assert.equal(readCredential(w.dir, a.grant_id), null, "its credential is gone");
    assert.equal(w.connections.authenticate({ grant: a.grant_id, token: tokenA }).code, "GRANT_REVOKED", "the right credential hears why");
    assert.equal(w.connections.authenticate({ grant: a.grant_id, token: `xenonet_assistant_${"0".repeat(64)}` }).code, "NOT_AUTHORIZED", "a wrong one hears nothing more");
    assert.equal(w.store.grant(a.grant_id).revision, 2);
    assert.equal(w.connections.authenticate({ grant: b.grant_id, token: tokenOf(w, b.grant_id) }).ok, true, "the other connection still works");
    assert.ok(w.actors.has("actor:dots"));
    assert.deepEqual(w.connections.revoke(a.grant_id), { revoked: a.grant_id, already: true });
    // A restored database that still has it active: the tombstone wins.
    w.store.db.prepare("UPDATE actor_grants SET revoked_at = NULL WHERE grant_id = ?").run(a.grant_id);
    const again = w.make();
    assert.ok(w.store.grant(a.grant_id).revoked_at != null);
    assert.equal(w.store.grant(a.grant_id).revoked_reason, "revoked-offline");
    assert.equal(again.list().connections.find((c) => c.grant_id === a.grant_id).state, "revoked");
  } finally {
    w.done();
  }
});

test("a grant revoked or everything turned off without the service (xenonet assistant) stops at once", () => {
  const w = world();
  try {
    const v = pairIn(w);
    const token = tokenOf(w, v.grant_id);
    markDisabled(w.dir);
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token }).code, "DISABLED");
    assert.equal(w.connections.anyActive(), false);
    w.connections.enable();
    assert.equal(isDisabled(w.dir), false);
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token }).ok, true);
    markRevoked(w.dir, v.grant_id);
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token }).code, "GRANT_REVOKED");
    assert.equal(w.connections.recheck(v.grant_id, 1).ok, false);
  } finally {
    w.done();
  }
});

test("rotating makes a new credential and stops the old one; changing scope moves the revision on", () => {
  const w = world();
  try {
    const v = pairIn(w);
    const old = tokenOf(w, v.grant_id);
    const rotated = w.connections.rotate(v.grant_id);
    const fresh = tokenOf(w, v.grant_id);
    assert.notEqual(fresh, old);
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token: old }).code, "NOT_AUTHORIZED");
    assert.equal(w.connections.authenticate({ grant: v.grant_id, token: fresh }).ok, true);
    assert.equal(rotated.revision, 2);
    const narrowed = w.connections.update(v.grant_id, { projects: [ATLAS, LEDGER], labels: true }, { places: w.places });
    assert.deepEqual([narrowed.revision, narrowed.labels, narrowed.projects.map((p) => p.name)], [3, true, ["atlas-api", "ledger"]]);
    assert.equal(narrowed.projects[0].project_id, v.projects[0].project_id, "a place keeps its id");
    assert.deepEqual(w.connections.recheck(v.grant_id, 2).changed, true);
    // A place that left the map can stay, but a new one has to be on it.
    w.places.delete(LEDGER);
    assert.equal(w.connections.update(v.grant_id, { projects: [ATLAS, LEDGER] }, { places: w.places }).projects.length, 2);
    assert.throws(() => w.connections.update(v.grant_id, { projects: ["project:/new"] }, { places: w.places }), /from the map/);
  } finally {
    w.done();
  }
});

test("the state folder on another machine or account stops every grant, and the device gets a new id", () => {
  const w = world();
  try {
    const v = pairIn(w);
    const before = w.connections.device();
    const moved = w.make({ fingerprint: "another-machine" });
    assert.notEqual(moved.device(), before);
    assert.equal(w.store.grant(v.grant_id).revoked_reason, "device-changed");
    assert.equal(moved.authenticate({ grant: v.grant_id, token: tokenOf(w, v.grant_id) ?? `xenonet_assistant_${"0".repeat(64)}` }).ok, false);
  } finally {
    w.done();
  }
});

test("a remote connection joins Hermes and Grok Bot without touching their tokens, observation or presence", () => {
  const w = world();
  try {
    const reg = createRegistration({ actors: w.actors, now: w.clock.now });
    // Hermes, registered with its own token, and observed too.
    w.actors.upsert({ id: "actor:hermes", name: "Hermes", product: { id: "hermes-agent", vendor: "Nous Research", version: "1.2" }, via: ["observed", "registered"], heartbeatMs: 60e3, capabilities: ["run-commands", "message-agents"], allowedAt: T });
    w.store.setActorToken("hermes", "a".repeat(64), T);
    w.actors.report("actor:hermes", { instance: "gateway", presence: "working", at: T });
    // Grok Bot, observed only.
    w.actors.upsert({ id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed"], heartbeatMs: 5000, capabilities: ["browse"] });
    w.actors.report("actor:grok-bot", { instance: "app", presence: "working", at: T });

    const hermes = pairIn(w, { product: "hermes-agent", transport: "local-stdio", actor: "actor:hermes" });
    const grok = pairIn(w, { product: "grok-bot", transport: "local-stdio", actor: "actor:grok-bot" });
    assert.deepEqual(w.actors.snapshot("actor:hermes").via, ["observed", "registered", "remote"]);
    assert.deepEqual(w.actors.snapshot("actor:hermes").capabilities, ["run-commands", "message-agents"], "its capabilities stand");
    assert.equal(w.store.actorToken("hermes").token_hash, "a".repeat(64), "its own token untouched");
    assert.equal(w.actors.isRegistered("actor:hermes"), true);
    assert.throws(() => pairIn(w, { product: "grok-bot", transport: "local-stdio", actor: "actor:hermes" }), /isn't Grok Bot/, "a product picks only its own actor");

    // A read through the connection: a working lease, which never outranks
    // or replaces the observer's word.
    const presence = createRemotePresence({ actors: w.actors, now: w.clock.now });
    presence.read("actor:grok-bot", grok.connection_id, { targets: [ATLAS] });
    assert.equal(w.actors.snapshot("actor:grok-bot").presence, "working");
    w.clock.add(REMOTE_LEASE_MS + 1);
    w.actors.report("actor:grok-bot", { instance: "app", presence: "idle" });
    w.actors.tick();
    assert.equal(w.actors.snapshot("actor:grok-bot").presence, "idle", "the observer's idle wins over a lapsed lease");

    // Revoking leaves both actors as they were; Hermes leaving keeps its
    // remote actor; removing Grok Bot's observed actor isn't XenoNet's to do.
    w.connections.revoke(grok.grant_id);
    assert.deepEqual(w.actors.snapshot("actor:grok-bot").via, ["observed", "remote"]);
    reg.remove("hermes", { reason: "left" });
    assert.ok(w.actors.has("actor:hermes"), "it still has a remote connection");
    assert.deepEqual(w.actors.snapshot("actor:hermes").via, ["observed", "remote"]);
    assert.equal(w.connections.authenticate({ grant: hermes.grant_id, token: tokenOf(w, hermes.grant_id) }).ok, true, "its remote connection outlives its registration");
    assert.throws(() => reg.remove("grok-bot"), /only observes/);
  } finally {
    w.done();
  }
});

test("removing a registered actor you also paired revokes its connections; its leaving doesn't", () => {
  const w = world();
  try {
    const reg = createRegistration({ actors: w.actors, now: w.clock.now });
    const made = (slug) => {
      w.actors.upsert({ id: `actor:${slug}`, name: slug, via: ["registered"], heartbeatMs: 60e3, capabilities: ["run-commands"], allowedAt: T });
      w.store.setActorToken(slug, slug.padEnd(64, "0"), T);
      return pairIn(w, { product: null, transport: "local-stdio", actor: `actor:${slug}`, label: `${slug} remote` });
    };
    const removed = made("wren");
    assert.deepEqual(reg.remove("wren"), { removed: "actor:wren" }, "gone, as the panel says");
    assert.equal(w.actors.has("actor:wren"), false);
    assert.equal(w.store.grants().find((g) => g.grant_id === removed.grant_id), undefined);
    assert.equal(readCredential(w.dir, removed.grant_id), null);
    const left = made("moth");
    reg.remove("moth", { reason: "left" });
    assert.ok(w.actors.has("actor:moth"), "leaving keeps the connection");
    assert.equal(w.connections.authenticate({ grant: left.grant_id, token: tokenOf(w, left.grant_id) }).ok, true);
  } finally {
    w.done();
  }
});

test("a lapsed lease says nothing, so an observer's offline shows", () => {
  const w = world();
  try {
    w.actors.upsert({ id: "actor:grok-bot", name: "Grok Bot", product: { id: "grok-bot", vendor: "SpaceXAI", version: null }, via: ["observed"], heartbeatMs: 5000 });
    const v = pairIn(w, { product: "grok-bot", transport: "local-stdio", actor: "actor:grok-bot" });
    createRemotePresence({ actors: w.actors, now: w.clock.now }).read("actor:grok-bot", v.connection_id);
    w.actors.report("actor:grok-bot", { instance: "app", presence: "offline" });
    assert.equal(w.actors.snapshot("actor:grok-bot").presence, "working", "while the read's lease holds");
    w.clock.add(REMOTE_LEASE_MS + 1);
    w.actors.tick();
    assert.equal(w.actors.snapshot("actor:grok-bot").presence, "offline");
  } finally {
    w.done();
  }
});

test("removing a remote actor revokes its connections first, and its rows go with it", () => {
  const w = world();
  try {
    const reg = createRegistration({ actors: w.actors, now: w.clock.now });
    const v = pairIn(w);
    reg.remove("dots");
    assert.equal(w.actors.has("actor:dots"), false);
    assert.equal(readCredential(w.dir, v.grant_id), null);
    assert.ok(fs.existsSync(path.join(w.dir, "revoked", v.grant_id)), "its tombstone stays");
    assert.equal(w.store.connections().length, 0, "its connection went with its row");
    assert.equal(w.store.grants().length, 0);
    assert.equal(w.store.audit({}).some((r) => r.operation === "revoke" && r.result === "actor-removed"), true);
  } finally {
    w.done();
  }
});

test("an account link is your word or a provider's proof, never both, and never authentication", () => {
  const w = world();
  try {
    const v = pairIn(w, { accountLink: { state: "owner_asserted", identity: "chatgpt:demo-workspace" } });
    assert.deepEqual(linkState(w.store, "dots"), { state: "owner_asserted", identity: "chatgpt:demo-workspace" });
    assert.equal(w.actors.snapshot("actor:dots").identity, null, "a link that's only your word leaves identity null");
    assert.equal(w.actors.snapshot("actor:dots").remote.accountLink, "owner_asserted");
    assert.equal(v.account_link.state, "owner_asserted");
    assert.throws(() => recordVerified(w.store, { slug: "dots", facts: { state: "unavailable" } }), /only a verified assertion/);
    // A verified assertion maps to an identity only when it names exactly
    // that one; a later one under another workspace lapses the first.
    recordVerified(w.store, { slug: "dots", facts: { state: "verified", issuer: "https://auth.example", provider: "openai", workspace: "demo-workspace", subject: "user-1" }, known: w.identities, now: T });
    assert.deepEqual(linkState(w.store, "dots", T), { state: "verified", identity: "chatgpt:demo-workspace" });
    assert.ok(!JSON.stringify(w.store.accountLinks("dots")).includes("user-1"), "the subject kept as a hash");
    const switched = recordVerified(w.store, { slug: "dots", facts: { state: "verified", issuer: "https://auth.example", provider: "openai", workspace: "other", subject: "user-1" }, known: w.identities, now: T + 1 });
    assert.equal(switched.switched, true);
    assert.deepEqual(linkState(w.store, "dots", T + 2), { state: "verified", identity: null });
    // External ids: once proven, under their namespace; never merged.
    pairIn(w, { label: "Second", name: "Second dot" });
    const facts = { state: "verified", issuer: "https://auth.example", accountBinding: "acct-1", externalActorId: "dot-42" };
    addExternalRef(w.store, { slug: "dots", adapterId: "openai-dots", facts });
    const second = w.actors.ids().find((id) => /^actor:dots-/.test(id)).slice("actor:".length);
    assert.throws(() => addExternalRef(w.store, { slug: second, adapterId: "openai-dots", facts }), (err) => err.code === "conflict");
    assert.throws(() => addExternalRef(w.store, { slug: second, adapterId: "openai-dots", facts: { ...facts, state: "unavailable" } }), (err) => err.code === "unverified");
  } finally {
    w.done();
  }
});

test("limits: remote actors and active connections are capped apart from registered actors", () => {
  const w = world();
  try {
    for (let i = 0; i < 8; i++) pairIn(w, { label: `Dot ${i}`, name: `Dot ${i}` });
    assert.throws(() => pairIn(w, { label: "Ninth", name: "Ninth" }), (err) => err.code === "full");
    for (let i = 0; i < 8; i++) pairIn(w, { label: `Extra ${i}`, actor: "actor:dots" });
    assert.throws(() => pairIn(w, { label: "Seventeenth", actor: "actor:dots" }), (err) => err.code === "full" && /connections/.test(err.message));
  } finally {
    w.done();
  }
});

test("a connection's presence lease ends out of sight, never offline, and a restart keeps it from standing forever", () => {
  const w = world();
  try {
    const v = pairIn(w);
    const presence = createRemotePresence({ actors: w.actors, now: w.clock.now });
    presence.read("actor:dots", v.connection_id, { targets: [ATLAS] });
    assert.equal(w.actors.snapshot("actor:dots").presence, "working");
    assert.deepEqual(w.actors.targetsOf("actor:dots"), [{ id: ATLAS, relation: "watches" }]);
    // Polling doesn't flood: the same read again is one word.
    const events = w.store.lastActorEvents("actor:dots", 50).length;
    for (let i = 0; i < 20; i++) (w.clock.add(1000), presence.read("actor:dots", v.connection_id, { targets: [ATLAS] }));
    assert.equal(w.store.lastActorEvents("actor:dots", 50).length, events);
    w.clock.add(REMOTE_LEASE_MS + 1);
    w.actors.tick();
    assert.equal(w.actors.snapshot("actor:dots").presence, "unknown");
    // Saved working, restored with a lease rather than for good.
    presence.read("actor:dots", v.connection_id, { at: w.clock.now() });
    w.actors.stop();
    const { Actors } = { Actors: w.actors.constructor };
    const restored = new Actors({ store: w.store, now: w.clock.now, alive: () => true });
    w.clock.add(REMOTE_LEASE_MS + 1);
    restored.tick();
    assert.equal(restored.snapshot("actor:dots").presence, "unknown");
    restored.stop();
  } finally {
    w.done();
  }
});

test("removed from the map, a place's sessions stop counting; the grant's own words never name another", () => {
  const w = world();
  try {
    const v = pairIn(w, { projects: [ATLAS, SECRET] });
    const s = w.connections.authenticate({ grant: v.grant_id, token: tokenOf(w, v.grant_id) });
    const policy = w.connections.policy(s);
    assert.deepEqual([...policy.places.values()].sort(), [ATLAS, SECRET].sort());
    w.connections.update(v.grant_id, { projects: [ATLAS] }, { places: w.places });
    const after = w.connections.policy(w.connections.recheck(v.grant_id, -1));
    assert.deepEqual([...after.places.values()], [ATLAS]);
    assert.equal(after.revision, 2);
    assert.ok(MIN > 0);
  } finally {
    w.done();
  }
});

test("a remote actor's snapshot names the routes its active connections take, and reads reach the map at most once a second", () => {
  const w = world();
  try {
    const dot = pairIn(w);
    const local = pairIn(w, { actor: "actor:dots", transport: "local-stdio", label: "On this computer" });
    assert.deepEqual(w.actors.snapshot("actor:dots").remote.transports, ["local-stdio", "openai-secure-mcp-tunnel"]);
    w.connections.revoke(local.grant_id);
    assert.deepEqual(w.actors.snapshot("actor:dots").remote.transports, ["openai-secure-mcp-tunnel"], "only active ones");

    // Each read is light on the map: the places it read, at most one an
    // actor a second, and nothing kept.
    const heard = [];
    const presence = createRemotePresence({ actors: w.actors, now: w.clock.now, onRead: (r) => heard.push(r) });
    presence.read("actor:dots", dot.connection_id, { targets: [], reach: [ATLAS, LEDGER, ATLAS] });
    presence.read("actor:dots", dot.connection_id, { targets: [ATLAS] });
    assert.deepEqual(heard, [{ actor: "actor:dots", connection: dot.connection_id, reach: [ATLAS, LEDGER], at: T }]);
    w.clock.add(1000);
    presence.read("actor:dots", dot.connection_id, { targets: [ATLAS] });
    assert.deepEqual(heard.at(-1).reach, [ATLAS], "reach is its targets unless it says otherwise");
    presence.read("actor:nobody", dot.connection_id, { reach: [ATLAS] });
    assert.equal(heard.length, 2, "an actor XenoNet doesn't have reads nothing onto the map");
    const broken = createRemotePresence({ actors: w.actors, now: w.clock.now, onRead: () => { throw new Error("page gone"); } });
    assert.doesNotThrow(() => broken.read("actor:dots", dot.connection_id), "a listener that fails never fails the read");
  } finally {
    w.done();
  }
});
