// engine.sqlite's versions (core/engine/store.js): numbered migrations, with
// PRAGMA user_version counting the ones taken, from a new file, from files
// made before there were versions, and from a newer XenoNet's. Then the
// actors' tables: what's kept and for how long, and how they're read.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { EngineStore, ENGINE_MIGRATIONS, ENGINE_VERSION, ACTOR_EVENTS_KEPT, migrateEngine } from "../core/engine/store.js";

const made = [];
const tmp = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-engine-store-"));
  made.push(dir);
  return path.join(dir, "engine.sqlite");
};
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));

const T = Date.now();
const columns = (db, table) => db.prepare(`PRAGMA table_info(${table})`).all().map((c) => c.name);
const version = (db) => db.prepare("PRAGMA user_version").get().user_version;

// The first engine.sqlite, as XenoNet wrote it before messages had a scope
// and contacts a pid, with a message and a contact in it.
function firstVersion(file, { scope = false, pid = false } = {}) {
  const db = new DatabaseSync(file);
  db.exec(`
    CREATE TABLE messages (id INTEGER PRIMARY KEY, sender TEXT NOT NULL, target TEXT NOT NULL, body TEXT NOT NULL, reply_to INTEGER, created_at INTEGER NOT NULL${scope ? ", scope TEXT" : ""});
    CREATE TABLE deliveries (message_id INTEGER NOT NULL, agent TEXT NOT NULL, seen_at INTEGER, PRIMARY KEY (message_id, agent));
    CREATE TABLE claims (id INTEGER PRIMARY KEY, agent TEXT NOT NULL, pattern TEXT NOT NULL, note TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
    CREATE TABLE focus (agent TEXT PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);
    CREATE TABLE contacts (agent TEXT PRIMARY KEY, app TEXT NOT NULL, cwd TEXT, via TEXT NOT NULL, at INTEGER NOT NULL, ended_at INTEGER${pid ? ", pid INTEGER" : ""});
    CREATE TABLE pins (agent TEXT NOT NULL, pid INTEGER NOT NULL, start INTEGER NOT NULL, sure INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (agent, pid, start));
  `);
  db.prepare("INSERT INTO messages (sender, target, body, created_at) VALUES (?, ?, ?, ?)").run("claude:aaaaaaaa-0001", "codex:bbbbbbbb-0002", "hello", T);
  db.prepare("INSERT INTO contacts (agent, app, via, at) VALUES (?, ?, ?, ?)").run("claude:aaaaaaaa-0001", "claude", "hook", T);
  db.close();
}

test("a new engine.sqlite takes every step, and says so in its version", () => {
  const file = tmp();
  const store = new EngineStore(file);
  assert.equal(ENGINE_VERSION, ENGINE_MIGRATIONS.length);
  assert.deepEqual(store.version, { from: 0, version: ENGINE_VERSION, newer: false });
  assert.equal(version(store.db), ENGINE_VERSION);
  assert.ok(columns(store.db, "messages").includes("scope") && columns(store.db, "contacts").includes("pid"));
  store.close();
  const again = new EngineStore(file);
  assert.deepEqual(again.version, { from: ENGINE_VERSION, version: ENGINE_VERSION, newer: false }, "nothing to do the second time");
  again.close();
});

test("one from before there were versions is brought up, whatever it had, and keeps what's in it", () => {
  for (const had of [{}, { scope: true }, { scope: true, pid: true }]) {
    const file = tmp();
    firstVersion(file, had);
    const store = new EngineStore(file);
    assert.deepEqual(store.version, { from: 0, version: ENGINE_VERSION, newer: false }, JSON.stringify(had));
    assert.equal(columns(store.db, "messages").filter((c) => c === "scope").length, 1);
    assert.equal(columns(store.db, "contacts").filter((c) => c === "pid").length, 1);
    assert.deepEqual(store.recent(0).map((m) => [m.body, m.scope]), [["hello", null]]);
    assert.equal(store.contact("claude:aaaaaaaa-0001").pid, null);
    // And it works as one made now.
    store.touch("claude:aaaaaaaa-0001", { app: "claude", via: "mcp", pid: 4242 });
    assert.equal(store.contact("claude:aaaaaaaa-0001").pid, 4242);
    store.close();
  }
});

test("one with data in it is copied before it's migrated, and a new or current one isn't", () => {
  const file = tmp();
  firstVersion(file);
  const store = new EngineStore(file);
  assert.equal(path.basename(store.block.backupFile), "engine.v0.sqlite");
  const copy = new DatabaseSync(store.block.backupFile, { readOnly: true });
  assert.equal(version(copy), 0, "as it was");
  assert.deepEqual(copy.prepare("SELECT body FROM messages").all().map((r) => r.body), ["hello"]);
  copy.close();
  store.close();
  const again = new EngineStore(file);
  assert.equal(again.block.backupFile, null, "nothing to do the second time");
  again.close();
  const fresh = new EngineStore(tmp());
  assert.equal(fresh.block.backupFile, null, "nothing to copy in a new file");
  fresh.close();
});

test("the copy made before a migration goes a week after it was made, with the hourly prune, while XenoNet runs", () => {
  const file = tmp();
  firstVersion(file);
  const store = new EngineStore(file);
  const copy = store.block.backupFile;
  const made = fs.statSync(copy).mtimeMs;
  store.prune(made + 6 * DAY);
  assert.ok(fs.existsSync(copy), "six days on, it's kept");
  store.prune(made + 7 * DAY + HOUR);
  assert.deepEqual([fs.existsSync(copy), store.block.backupFile], [false, null], "a week on, it's gone");
  assert.deepEqual(store.recent(0), [], "and the database itself prunes as before");
  store.close();
});

test("a newer XenoNet's database is left at its version", () => {
  const file = tmp();
  new EngineStore(file).close();
  const db = new DatabaseSync(file);
  db.exec("PRAGMA user_version = 40");
  db.close();
  const store = new EngineStore(file);
  assert.deepEqual(store.version, { from: 40, version: 40, newer: true });
  assert.equal(version(store.db), 40);
  store.close();
});

test("a step that fails takes nothing with it: the version stays where it was", () => {
  const db = new DatabaseSync(":memory:");
  const steps = [(d) => d.exec("CREATE TABLE a (x INTEGER)"), (d) => (d.exec("CREATE TABLE b (y INTEGER)"), d.exec("INSERT INTO nowhere VALUES (1)"))];
  assert.throws(() => migrateEngine(db, steps), /no such table: nowhere/);
  assert.equal(version(db), 1, "the first step stands");
  assert.deepEqual(db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").all().map((r) => r.name), ["a"], "the second left nothing behind");
  assert.deepEqual(migrateEngine(db, steps.slice(0, 1)), { from: 1, version: 1, newer: false });
  db.close();
});

// The engine.sqlite XenoNet wrote before actors (version 3), with a message in it.
function thirdVersion(file) {
  const db = new DatabaseSync(file);
  migrateEngine(db, ENGINE_MIGRATIONS.slice(0, 3));
  db.prepare("INSERT INTO messages (sender, target, body, created_at) VALUES (?, ?, ?, ?)").run("claude:aaaaaaaa-0001", "all", "hello", T);
  db.close();
}

const HOUR = 3600e3;
const DAY = 24 * HOUR;
const tables = (db) => db.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").all().map((r) => r.name);

test("version 4 adds the actors' tables to a file from before them, and keeps what was there", () => {
  const file = tmp();
  thirdVersion(file);
  const store = new EngineStore(file);
  assert.deepEqual(store.version, { from: 3, version: ENGINE_VERSION, newer: false });
  assert.deepEqual(tables(store.db).filter((t) => t.startsWith("actor")), ["actor_account_links", "actor_connections", "actor_events", "actor_external_refs", "actor_grants", "actor_requests", "actor_revoked", "actor_targets", "actors"]);
  assert.deepEqual(store.recent(0).map((m) => m.body), ["hello"]);
  assert.equal(path.basename(store.block.backupFile), "engine.v3.sqlite", "copied first");
  store.close();
});

test("version 7 gives each actor a gateway token of its own, beside its token, which it leaves as it was", () => {
  const file = tmp();
  const db = new DatabaseSync(file);
  migrateEngine(db, ENGINE_MIGRATIONS.slice(0, 6));
  db.prepare("INSERT INTO actors (slug, kind, name, via, capabilities, token_hash, token_at, created_at, allowed_at) VALUES ('muse', 'agent', 'Muse', '[\"observed\",\"registered\"]', '[\"message-agents\"]', 'abc', ?, ?, ?)").run(T, T, T);
  db.close();
  const store = new EngineStore(file);
  assert.deepEqual(store.version, { from: 6, version: ENGINE_VERSION, newer: false });
  assert.deepEqual([store.actorToken("muse").token_hash, store.gatewayToken("muse")], ["abc", null]);
  store.setGatewayToken("muse", "def", T + 1);
  assert.deepEqual([store.actorToken("muse").token_hash, { ...store.gatewayToken("muse") }, store.actorByGatewayToken("def")?.slug, store.actorByToken("def")], ["abc", { gateway_hash: "def", gateway_at: T + 1 }, "muse", null], "one apart from the other");
  assert.deepEqual(store.staleGatewayTokens(T + 2).map((r) => r.slug), ["muse"]);
  store.dropGatewayToken("muse");
  assert.equal(store.gatewayToken("muse"), null);
  store.close();
});

test("the issued tokens are the ones that work now: actors' own, their gateway tokens and waiting registrations'", () => {
  const store = new EngineStore(tmp());
  assert.deepEqual(store.issuedTokenHashes(), []);
  const actor = store.db.prepare("INSERT INTO actors (slug, kind, name, via, capabilities, created_at, token_hash, gateway_hash) VALUES (?, 'agent', ?, '[]', '[]', ?, ?, ?)");
  actor.run("muse", "Muse", T, "h-muse", "g-muse");
  actor.run("wren", "Wren", T, "h-wren", null);
  actor.run("moth", "Moth", T, null, null);
  store.addActorRequest({ id: "r1", slug: "heron", ask: "{}", token_hash: "h-waiting", created_at: T, expires_at: T + HOUR });
  store.addActorRequest({ id: "r2", slug: "heron", ask: "{}", token_hash: "h-denied", created_at: T, expires_at: T + HOUR });
  store.decideActorRequest("r2", "deny", T + 1);
  store.revokeActorToken({ token_hash: "h-old", actor: "actor:muse", reason: "rotated", at: T });
  assert.deepEqual(store.issuedTokenHashes().sort(), ["g-muse", "h-muse", "h-waiting", "h-wren"], "a revoked token, or a refused registration's, no longer works");
  store.close();
});

test("version 5 adds remote connections beside the actors, and leaves every actor, token and event as it was", () => {
  const file = tmp();
  const db = new DatabaseSync(file);
  migrateEngine(db, ENGINE_MIGRATIONS.slice(0, 4));
  db.prepare("INSERT INTO actors (slug, kind, name, product, via, capabilities, heartbeat_ms, token_hash, token_at, created_at, allowed_at) VALUES ('hermes', 'agent', 'Hermes', '{\"id\":\"hermes-agent\"}', '[\"observed\",\"registered\"]', '[\"run-commands\"]', 60000, 'abc', ?, ?, ?)").run(T, T, T);
  db.prepare("INSERT INTO actor_events (id, actor, at, kind, via) VALUES ('hermes:1', 'actor:hermes', ?, 'ran', 'registered')").run(T);
  db.prepare("INSERT INTO actor_targets (actor, target, relation, since, last_at) VALUES ('actor:hermes', 'project:/p', 'works-on', ?, ?)").run(T, T);
  const before = JSON.stringify([db.prepare("SELECT * FROM actors").all(), db.prepare("SELECT * FROM actor_events").all(), db.prepare("SELECT * FROM actor_targets").all()]);
  db.close();
  const store = new EngineStore(file);
  assert.deepEqual(store.version, { from: 4, version: ENGINE_VERSION, newer: false });
  assert.equal(path.basename(store.block.backupFile), "engine.v4.sqlite", "copied first");
  // Every actor as it was; a later version adds the gateway token's columns, empty.
  const actorsThen = store.db.prepare("SELECT * FROM actors").all().map(({ gateway_hash, gateway_at, ...row }) => (assert.deepEqual([gateway_hash, gateway_at], [null, null]), row));
  assert.equal(JSON.stringify([actorsThen, store.db.prepare("SELECT * FROM actor_events").all(), store.db.prepare("SELECT * FROM actor_targets").all()]), before);
  assert.deepEqual(tables(store.db).filter((t) => t.startsWith("assistant")), ["assistant_audit", "assistant_meta", "assistant_places"]);
  // A connection goes with its actor, and a grant with its connection; the
  // legacy token never moves.
  store.addConnection({ connection_id: "cn_1", actor_slug: "hermes", adapter_id: "hermes-agent", provider: "nous", transport_kind: "local-stdio", transport_binding: null, label: "Hermes remote", device_alias: null, audience_description: "me", audience_verification: "owner_verified", audience_scope: "owner_endpoint", created_at: T });
  store.addGrant({ grant_id: "gr_1", connection_id: "cn_1", device_id: "dev_1", permissions: '["read"]', project_allowlist: "[]", field_policy: "{}", data_mode: "real", lookback_ms: DAY, token_hash: "def", created_at: T, expires_at: T + DAY });
  assert.throws(() => store.addConnection({ connection_id: "cn_2", actor_slug: "nobody", adapter_id: "x", provider: "x", transport_kind: "x", transport_binding: null, label: "x", device_alias: null, audience_description: "x", audience_verification: "unverified", audience_scope: "owner_endpoint", created_at: T }), /FOREIGN KEY/);
  assert.equal(store.actorToken("hermes").token_hash, "abc");
  store.removeActor("hermes");
  assert.deepEqual([store.connections().length, store.grants().length], [0, 0]);
  store.close();
  // An older XenoNet leaves the newer file's shape alone and keeps reading
  // its actors.
  const older = new EngineStore(file);
  older.close();
  const raw = new DatabaseSync(file);
  assert.deepEqual(migrateEngine(raw, ENGINE_MIGRATIONS.slice(0, 4)), { from: ENGINE_VERSION, version: ENGINE_VERSION, newer: true });
  raw.close();
});

// XenoNet 0.3.0 took the first seven steps, as they are here (its
// core/engine/store.js differs only by step 8), so its engine.sqlite is at
// version 7. 0.4.0's service, in Rust, takes the same steps.
test("0.3.0's engine.sqlite takes step 8, copied first, with its messages, claims and actors as they were, and 0.3.0 still opens it after a rollback", () => {
  const file = tmp();
  const db = new DatabaseSync(file);
  migrateEngine(db, ENGINE_MIGRATIONS.slice(0, 7));
  db.prepare("INSERT INTO messages (sender, target, body, created_at) VALUES (?, ?, ?, ?)").run("claude:aaaaaaaa-0001", "codex:bbbbbbbb-0002", "hello", T);
  db.prepare("INSERT INTO deliveries (message_id, agent) VALUES (1, ?)").run("codex:bbbbbbbb-0002");
  db.prepare("INSERT INTO claims (agent, pattern, note, created_at, expires_at) VALUES (?, ?, ?, ?, ?)").run("claude:aaaaaaaa-0001", "src/a.js", "mine", T, T + DAY);
  db.prepare("INSERT INTO actors (slug, kind, name, via, capabilities, token_hash, token_at, created_at, allowed_at, gateway_hash, gateway_at) VALUES ('muse', 'agent', 'Muse', '[\"registered\"]', '[]', 'abc', ?, ?, ?, 'def', ?)").run(T, T, T, T);
  db.close();

  const store = new EngineStore(file);
  assert.deepEqual(store.version, { from: 7, version: ENGINE_VERSION, newer: false });
  assert.equal(path.basename(store.block.backupFile), "engine.v7.sqlite", "copied first");
  assert.deepEqual(store.unseen("codex:bbbbbbbb-0002").map((m) => [m.body, m.host]), [["hello", null]], "made here, waiting to be read");
  assert.deepEqual(store.claims(T).map((c) => [c.agent, c.pattern, c.note]), [["claude:aaaaaaaa-0001", "src/a.js", "mine"]]);
  assert.deepEqual([store.actorToken("muse").token_hash, store.actorByGatewayToken("def")?.slug], ["abc", "muse"]);
  store.close();

  // An update rolled back to 0.3.0: its seven steps leave the newer shape
  // alone, and what it reads is still there.
  const raw = new DatabaseSync(file);
  assert.deepEqual(migrateEngine(raw, ENGINE_MIGRATIONS.slice(0, 7)), { from: ENGINE_VERSION, version: ENGINE_VERSION, newer: true });
  assert.deepEqual(raw.prepare("SELECT body FROM messages").all().map((r) => r.body), ["hello"]);
  raw.close();
});

test("the remote reads' log keeps a week, and at most its cap", () => {
  const store = new EngineStore(tmp());
  for (let i = 0; i < 5; i++) store.addAudit({ at: T - 8 * DAY + i, kind: "access", operation: "status", result: "ok" });
  for (let i = 0; i < 5; i++) store.addAudit({ at: T - HOUR + i, kind: "access", operation: "status", result: "ok" });
  store.pruneAudit(T);
  assert.equal(store.audit({ limit: 100 }).length, 5);
  store.close();
});

test("an actor's row: saved and read back, its token's columns left alone, and removed with its events and targets", () => {
  const store = new EngineStore(tmp());
  const row = { slug: "wren", kind: "agent", name: "Wren", product: JSON.stringify({ id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" }), via: JSON.stringify(["observed"]), capabilities: "[]", heartbeat_ms: 60000, created_at: T };
  store.saveActor(row);
  store.db.prepare("UPDATE actors SET token_hash = 'h', token_at = ? WHERE slug = 'wren'").run(T);
  store.saveActor({ ...row, name: "Wren", presence: "working", presence_at: T + 1, last_seen: T + 2, seq: 7, created_at: T + 99, token_hash: "nope" });
  const back = store.actorRow("wren");
  assert.deepEqual([back.presence, back.presence_at, back.last_seen, back.seq, back.created_at, back.token_hash, back.enabled], ["working", T + 1, T + 2, 7, T, "h", 1], "created_at and the token stay as they were");
  store.addActorEvent({ id: "wren:1", actor: "actor:wren", at: T, kind: "ran", via: "observed" });
  store.saveActorTarget({ actor: "actor:wren", target: "project:/p", relation: "works-on", since: T, last_at: T });
  store.saveActor({ slug: "moth", kind: "agent", name: "Moth", via: "[]", capabilities: "[]", created_at: T + 1 });
  assert.deepEqual(store.actorRows().map((r) => r.slug), ["wren", "moth"], "oldest first");
  store.removeActor("wren");
  assert.deepEqual([store.actorRow("wren"), store.lastActorEvents("actor:wren"), store.actorTargets()], [null, [], []]);
  store.close();
});

test("actors' events: the last few, newest last; since a time, oldest first and without someone's; counts; refs", () => {
  const store = new EngineStore(tmp());
  const add = (id, actor, at, kind = "ran", ref = null) => store.addActorEvent({ id, actor, at, kind, via: "observed", ref });
  add("wren:1", "actor:wren", T - 3 * HOUR, "spawned");
  add("you:1", "actor:you", T - 2 * HOUR, "status");
  add("wren:2", "actor:wren", T - HOUR, "ran", "run-1");
  add("wren:3", "actor:wren", T - HOUR, "ran");
  add("moth:1", "actor:moth", T, "said");
  assert.deepEqual(store.lastActorEvents("actor:wren", 2).map((e) => e.id), ["wren:2", "wren:3"]);
  assert.deepEqual(store.actorEventsSince(T - 2 * HOUR).map((e) => e.id), ["you:1", "wren:2", "wren:3", "moth:1"]);
  assert.deepEqual(store.actorEventsSince(T - 2 * HOUR, { except: "actor:you", limit: 2 }).map((e) => e.id), ["wren:2", "wren:3"]);
  assert.deepEqual(store.actorEventsSince(0, { only: "actor:wren" }).map((e) => e.id), ["wren:1", "wren:2", "wren:3"]);
  assert.deepEqual(store.actorEventCounts(T - 2 * HOUR).map((r) => [r.actor, r.kind, r.n]).sort(), [["actor:moth", "said", 1], ["actor:wren", "ran", 2], ["actor:you", "status", 1]]);
  assert.equal(store.hasActorRef("actor:wren", "run-1", T - DAY), true);
  assert.equal(store.hasActorRef("actor:wren", "run-1", T), false, "not since then");
  assert.equal(store.hasActorRef("actor:moth", "run-1", T - DAY), false, "another actor's");
  store.close();
});

test("a target touched again keeps when it began and moves its last time on", () => {
  const store = new EngineStore(tmp());
  store.saveActorTarget({ actor: "actor:wren", target: "project:/p", relation: "works-on", since: T, last_at: T });
  store.saveActorTarget({ actor: "actor:wren", target: "project:/p", relation: "works-on", since: T + 5, last_at: T + 5 });
  store.saveActorTarget({ actor: "actor:wren", target: "project:/p", relation: "edited", since: T + 6, last_at: T + 6 });
  assert.deepEqual(store.actorTargets().map((t) => [t.relation, t.since, t.last_at]), [["works-on", T, T + 5], ["edited", T + 6, T + 6]]);
  store.dropActorTarget("actor:wren", "project:/p", "edited");
  assert.equal(store.actorTargets().length, 1);
  store.close();
});

test("what's kept: actors' events a week, the person's a day, targets a week after they were last touched, requests a day, revoked tokens a week", () => {
  const store = new EngineStore(tmp());
  const ev = (id, actor, at) => store.addActorEvent({ id, actor, at, kind: "ran", via: "observed" });
  ev("wren:1", "actor:wren", T - 8 * DAY);
  ev("wren:2", "actor:wren", T - 6 * DAY);
  ev("you:1", "actor:you", T - 25 * HOUR);
  ev("you:2", "actor:you", T - 23 * HOUR);
  const target = (actor, t, last) => store.saveActorTarget({ actor, target: t, relation: "works-on", since: last, last_at: last });
  target("actor:wren", "project:/old", T - 8 * DAY);
  target("actor:wren", "project:/new", T - 6 * DAY);
  target("actor:you", "project:/old", T - 25 * HOUR);
  target("actor:you", "project:/new", T - 23 * HOUR);
  store.db.prepare("INSERT INTO actor_requests (id, slug, ask, token_hash, created_at, expires_at) VALUES (?, ?, '{}', ?, ?, ?)").run("old", "wren", "h1", T - 25 * HOUR, T - 25 * HOUR + 600e3);
  store.db.prepare("INSERT INTO actor_requests (id, slug, ask, token_hash, created_at, expires_at) VALUES (?, ?, '{}', ?, ?, ?)").run("new", "moth", "h2", T - HOUR, T - HOUR + 600e3);
  store.db.prepare("INSERT INTO actor_revoked (token_hash, actor, reason, at) VALUES (?, ?, ?, ?)").run("h3", "actor:wren", "rotated", T - 8 * DAY);
  store.db.prepare("INSERT INTO actor_revoked (token_hash, actor, reason, at) VALUES (?, ?, ?, ?)").run("h4", "actor:wren", "rotated", T - 6 * DAY);
  store.saveActor({ slug: "you", kind: "human", name: "You", via: '["observed"]', capabilities: "[]", created_at: T - 2 * DAY, status: "Looking at atlas-api", status_at: T - 25 * HOUR });
  store.saveActor({ slug: "wren", kind: "agent", name: "Wren", via: '["observed"]', capabilities: "[]", created_at: T - 2 * DAY, status: "Triaging", status_at: T - 25 * HOUR });
  store.prune(T);
  assert.deepEqual(store.actorEventsSince(0).map((e) => e.id), ["wren:2", "you:2"]);
  assert.deepEqual(store.actorTargets().map((t) => [t.actor, t.target]), [["actor:wren", "project:/new"], ["actor:you", "project:/new"]]);
  assert.deepEqual(store.db.prepare("SELECT id FROM actor_requests").all().map((r) => r.id), ["new"]);
  assert.deepEqual(store.db.prepare("SELECT token_hash FROM actor_revoked").all().map((r) => r.token_hash), ["h4"]);
  assert.deepEqual([store.actorRow("you").status, store.actorRow("wren").status], [null, "Triaging"], "your status goes after a day, an agent's stays");
  store.close();
});

test(`at most ${ACTOR_EVENTS_KEPT} events an actor, the oldest going first`, () => {
  const store = new EngineStore(tmp());
  store.transaction(() => {
    for (let i = 1; i <= ACTOR_EVENTS_KEPT + 5; i++) store.addActorEvent({ id: `wren:${i}`, actor: "actor:wren", at: T - 60e3 + Math.floor((i + 1) / 2), kind: "ran", via: "observed" });
    store.addActorEvent({ id: "moth:1", actor: "actor:moth", at: T - 3 * DAY, kind: "ran", via: "observed" });
  });
  store.prune(T);
  const left = store.db.prepare("SELECT id FROM actor_events WHERE actor = 'actor:wren' ORDER BY rowid").all().map((r) => r.id);
  assert.equal(left.length, ACTOR_EVENTS_KEPT);
  assert.deepEqual([left[0], left.at(-1)], ["wren:6", `wren:${ACTOR_EVENTS_KEPT + 5}`], "the first five went, though the fifth and sixth share a time");
  assert.equal(store.lastActorEvents("actor:moth").length, 1, "another actor's stay");
  store.close();
});

test("a transaction inside another joins it, and one that throws takes its writes back", () => {
  const store = new EngineStore(tmp());
  assert.throws(() =>
    store.transaction(() => {
      store.addActorEvent({ id: "wren:1", actor: "actor:wren", at: T, kind: "ran", via: "observed" });
      store.transaction(() => store.addActorEvent({ id: "wren:2", actor: "actor:wren", at: T, kind: "ran", via: "observed" }));
      throw new Error("no");
    }),
  /no/);
  assert.deepEqual(store.lastActorEvents("actor:wren"), []);
  assert.equal(store.transaction(() => 42), 42);
  store.close();
});

test("version 6 keeps how XenoNet knew who sent a message, beside its words", () => {
  const store = new EngineStore(":memory:");
  const id = store.send({ sender: "actor:wren", target: "actor:moth", body: "hi", recipients: ["actor:moth"], envelope: { schema: "message/envelope/v1", auth: { method: "token" } } }, 1);
  assert.deepEqual(JSON.parse(store.message(id).envelope).auth, { method: "token" });
  const plain = store.send({ sender: "actor:wren", target: "actor:moth", body: "older", recipients: ["actor:moth"] }, 2);
  assert.equal(store.message(plain).envelope, null, "none for a message without one");
});
