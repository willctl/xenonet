// What the engine remembers, in engine.sqlite in the state dir: messages
// between agents (and which of their recipients have seen them), the files
// agents have claimed, what each says it's doing, when XenoNet last heard
// from each agent directly (a hook or a tool call), which is how it knows an
// agent can receive messages, and which process holds each agent's session
// (pins; see verify.js). Actors keep their rows, events and targets here
// too, with registrations waiting for you and revoked tokens (core/actors,
// and docs/design/actors/interface.md, section 2), and remote actors'
// connections and grants, with the bounded log of what they read
// (core/actors/connections.js, core/assistant). One writer, this server;
// agents and actors never touch the file. The store block (core/blocks/sqlite.js) opens it and keeps its
// version, and copies a file with data in it before taking it a step up
// (engine.v<n>.sqlite beside it), as state.json keeps state.v1.json.
import { SqliteStore, migrateSqlite } from "../blocks/sqlite.js";
import { YOU } from "../schema/ids.js";

const YOU_SLUG = YOU.slice("actor:".length);
const DAY_MS = 24 * 3600e3;
const WEEK_MS = 7 * DAY_MS;
// Events kept for an actor, whatever their age; the oldest go first.
export const ACTOR_EVENTS_KEPT = 10000;
// Remote reads logged (assistant_audit), for a week and at most this many,
// the oldest going first: a record is about 200 bytes, so about 4 MB.
export const AUDIT_KEPT = 20000;
// The columns of an actor's row that core/actors writes, and their values
// when it leaves one out.
const ACTOR_DEFAULTS = { slug: null, kind: null, name: null, product: null, via: "[]", capabilities: "[]", heartbeat_ms: null, identity: null, enabled: 1, created_at: null, allowed_at: null, last_seen: null, presence: "unknown", presence_at: null, status: null, status_at: null, seq: 0 };

const hasColumn = (db, table, column) => db.prepare(`PRAGMA table_info(${table})`).all().some((c) => c.name === column);

// The database's shape, a step at a time. PRAGMA user_version counts the
// steps taken. A database from before it was counted (version 0) can have
// any of the first three already, so those look before they change.
export const ENGINE_MIGRATIONS = [
  // 1: the tables as they first were.
  (db) =>
    db.exec(`
      CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY, sender TEXT NOT NULL, target TEXT NOT NULL,
        body TEXT NOT NULL, reply_to INTEGER, created_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS deliveries (
        message_id INTEGER NOT NULL, agent TEXT NOT NULL, seen_at INTEGER,
        PRIMARY KEY (message_id, agent));
      CREATE INDEX IF NOT EXISTS deliveries_unseen ON deliveries (agent, seen_at);
      CREATE TABLE IF NOT EXISTS claims (
        id INTEGER PRIMARY KEY, agent TEXT NOT NULL, pattern TEXT NOT NULL,
        note TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS focus (agent TEXT PRIMARY KEY, text TEXT NOT NULL, at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS contacts (
        agent TEXT PRIMARY KEY, app TEXT NOT NULL, cwd TEXT, via TEXT NOT NULL,
        at INTEGER NOT NULL, ended_at INTEGER);
      CREATE TABLE IF NOT EXISTS pins (
        agent TEXT NOT NULL, pid INTEGER NOT NULL, start INTEGER NOT NULL,
        sure INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (agent, pid, start));
    `),
  // 2: a message's repo (its scope), for messages to everyone working there.
  (db) => hasColumn(db, "messages", "scope") || db.exec("ALTER TABLE messages ADD COLUMN scope TEXT"),
  // 3: an agent's process, to tell when it's gone.
  (db) => hasColumn(db, "contacts", "pid") || db.exec("ALTER TABLE contacts ADD COLUMN pid INTEGER"),
  // 4: actors. One row per registered or observed actor (the person at
  // the console's too), keyed by slug, with its last presence and status
  // and the number of its last event; JSON in product, via and
  // capabilities. A registered actor's token is kept only as its SHA-256.
  // Registrations waiting for you; what actors did, as XenoNet wrote it, with
  // the facts it was written from; what they act on; and revoked tokens,
  // so a program hears why.
  (db) =>
    db.exec(`
      CREATE TABLE IF NOT EXISTS actors (
        slug TEXT PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL,
        product TEXT, via TEXT NOT NULL, capabilities TEXT NOT NULL,
        heartbeat_ms INTEGER, identity TEXT,
        token_hash TEXT UNIQUE, token_at INTEGER,
        enabled INTEGER NOT NULL DEFAULT 1,
        created_at INTEGER NOT NULL, allowed_at INTEGER, last_seen INTEGER,
        presence TEXT NOT NULL DEFAULT 'unknown', presence_at INTEGER,
        status TEXT, status_at INTEGER,
        seq INTEGER NOT NULL DEFAULT 0);
      CREATE TABLE IF NOT EXISTS actor_requests (
        id TEXT PRIMARY KEY, slug TEXT NOT NULL, ask TEXT NOT NULL,
        token_hash TEXT NOT NULL UNIQUE, asker TEXT,
        created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
        decision TEXT, decided_at INTEGER);
      CREATE TABLE IF NOT EXISTS actor_events (
        id TEXT PRIMARY KEY, actor TEXT NOT NULL, at INTEGER NOT NULL,
        kind TEXT NOT NULL, target TEXT, detail TEXT, via TEXT NOT NULL,
        facts TEXT, ref TEXT);
      CREATE INDEX IF NOT EXISTS actor_events_by_actor ON actor_events (actor, at);
      CREATE INDEX IF NOT EXISTS actor_events_by_time ON actor_events (at);
      CREATE INDEX IF NOT EXISTS actor_events_by_ref ON actor_events (actor, ref) WHERE ref IS NOT NULL;
      CREATE TABLE IF NOT EXISTS actor_targets (
        actor TEXT NOT NULL, target TEXT NOT NULL, relation TEXT NOT NULL,
        since INTEGER NOT NULL, last_at INTEGER NOT NULL,
        PRIMARY KEY (actor, target, relation));
      CREATE TABLE IF NOT EXISTS actor_revoked (
        token_hash TEXT PRIMARY KEY, actor TEXT NOT NULL, reason TEXT NOT NULL,
        at INTEGER NOT NULL);
    `),
  // 5: remote actors' connections (core/actors/connections.js; docs/design/
  // cloud-assistants.md). Children of an actor's row, never a registry of
  // their own: a connection is one way an actor reaches XenoNet (a tunnel,
  // say), and its grant what it may read through it, until when, with a
  // credential kept only as its SHA-256. An actor's account links say how
  // sure XenoNet is of the account it acts as (verified, or only your word),
  // and its external refs name it in a provider's own ids, once verified.
  // The places a grant names go out under ids of their own (prj_<uuid>),
  // since a project's node id holds its path. The device's id and the
  // bounded log of what was read are kept here too. Removing an actor
  // takes its connections with it; the legacy token on its row is
  // untouched by any of this.
  (db) =>
    db.exec(`
      CREATE TABLE IF NOT EXISTS actor_connections (
        connection_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        adapter_id TEXT NOT NULL, provider TEXT NOT NULL,
        transport_kind TEXT NOT NULL, transport_binding TEXT,
        label TEXT NOT NULL, device_alias TEXT,
        audience_description TEXT NOT NULL, audience_verification TEXT NOT NULL,
        audience_scope TEXT NOT NULL,
        state TEXT NOT NULL DEFAULT 'active',
        created_at INTEGER NOT NULL, last_used_at INTEGER, revoked_at INTEGER);
      CREATE INDEX IF NOT EXISTS actor_connections_by_actor ON actor_connections (actor_slug);
      CREATE TABLE IF NOT EXISTS actor_grants (
        grant_id TEXT PRIMARY KEY,
        connection_id TEXT NOT NULL REFERENCES actor_connections (connection_id) ON DELETE CASCADE,
        device_id TEXT NOT NULL, permissions TEXT NOT NULL,
        project_allowlist TEXT NOT NULL, field_policy TEXT NOT NULL,
        data_mode TEXT NOT NULL, lookback_ms INTEGER NOT NULL,
        revision INTEGER NOT NULL DEFAULT 1, token_hash TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
        revoked_at INTEGER, revoked_reason TEXT, last_used_at INTEGER);
      CREATE INDEX IF NOT EXISTS actor_grants_by_connection ON actor_grants (connection_id);
      CREATE TABLE IF NOT EXISTS actor_account_links (
        link_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        identity_id TEXT, issuer TEXT, provider TEXT NOT NULL, workspace TEXT,
        proof_status TEXT NOT NULL, proof_method TEXT, subject_binding TEXT,
        checked_at INTEGER, expires_at INTEGER, created_at INTEGER NOT NULL);
      CREATE INDEX IF NOT EXISTS actor_account_links_by_actor ON actor_account_links (actor_slug);
      CREATE TABLE IF NOT EXISTS actor_external_refs (
        external_ref_id TEXT PRIMARY KEY,
        actor_slug TEXT NOT NULL REFERENCES actors (slug) ON DELETE CASCADE,
        adapter_id TEXT NOT NULL, issuer TEXT NOT NULL, account_binding TEXT NOT NULL,
        external_actor_id TEXT NOT NULL, verified_at INTEGER NOT NULL,
        UNIQUE (adapter_id, issuer, account_binding, external_actor_id));
      CREATE TABLE IF NOT EXISTS assistant_places (
        place TEXT PRIMARY KEY, project_id TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS assistant_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS assistant_audit (
        id INTEGER PRIMARY KEY, at INTEGER NOT NULL, kind TEXT NOT NULL,
        request_id TEXT, grant_id TEXT, grant_revision INTEGER, actor TEXT,
        connection_id TEXT, operation TEXT NOT NULL, result TEXT NOT NULL,
        latency_ms INTEGER, bytes INTEGER, projects INTEGER);
      CREATE INDEX IF NOT EXISTS assistant_audit_by_time ON assistant_audit (at);
    `),
  // 6: how XenoNet knows who sent a message (core/engine/attest.js): what it
  // checked, as JSON, apart from what the sender said.
  (db) => hasColumn(db, "messages", "envelope") || db.exec("ALTER TABLE messages ADD COLUMN envelope TEXT"),
  // 7: an actor's gateway token (server/actor-gateway.js), apart from its
  // own: its SHA-256, and when it was last used (or made).
  (db) => {
    if (!hasColumn(db, "actors", "gateway_hash")) db.exec("ALTER TABLE actors ADD COLUMN gateway_hash TEXT");
    if (!hasColumn(db, "actors", "gateway_at")) db.exec("ALTER TABLE actors ADD COLUMN gateway_at INTEGER");
  },
  // 8: a network across machines (docs/design/network.md): where a message
  // came from (null: made here) and its id across the network, which host
  // each delivery is on, its receipt and the frame that carries it, the
  // latest presence of each other host, their claims, and handoffs.
  (db) => {
    for (const [table, column, type] of [["messages", "host", "TEXT"], ["messages", "mid", "TEXT"], ["deliveries", "host", "TEXT"], ["deliveries", "receipt", "TEXT"], ["deliveries", "receipt_at", "INTEGER"], ["deliveries", "frame", "TEXT"]]) {
      if (!hasColumn(db, table, column)) db.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${type}`);
    }
    db.exec(`
      CREATE INDEX IF NOT EXISTS messages_by_mid ON messages (mid) WHERE mid IS NOT NULL;
      CREATE TABLE IF NOT EXISTS net_presence (
        host TEXT PRIMARY KEY, counter INTEGER NOT NULL, at INTEGER NOT NULL,
        you INTEGER NOT NULL DEFAULT 0, roster TEXT, agents TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS net_claims (
        host TEXT NOT NULL, repo_key TEXT NOT NULL, path TEXT NOT NULL,
        agent TEXT NOT NULL, branch TEXT, kind TEXT NOT NULL, until INTEGER,
        at INTEGER NOT NULL, PRIMARY KEY (host, repo_key, path, agent, kind));
      CREATE TABLE IF NOT EXISTS handoffs (
        id TEXT PRIMARY KEY, host TEXT, maker TEXT NOT NULL,
        to_agent TEXT, to_repo TEXT, to_scope TEXT, to_host TEXT,
        goal TEXT, done TEXT, next TEXT, files TEXT NOT NULL,
        branch TEXT, commit_sha TEXT,
        state TEXT NOT NULL DEFAULT 'open', taken_by TEXT, taken_at INTEGER,
        asked_by TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL);
    `);
  },
];

export const ENGINE_VERSION = ENGINE_MIGRATIONS.length;

// Takes `db` from its version up to the last one, each step in a
// transaction of its own with the version it reaches. A database a newer
// XenoNet has moved further is left as it is. Says where it started and
// where it is.
export const migrateEngine = (db, migrations = ENGINE_MIGRATIONS) => migrateSqlite(db, migrations);

export class EngineStore {
  constructor(file) {
    // Foreign keys on, so a connection goes with its actor and a grant with
    // its connection; the tables from before 5 declare none.
    this.block = new SqliteStore({ name: "engine", file, migrations: ENGINE_MIGRATIONS, pragmas: ["journal_mode = WAL", "foreign_keys = ON"], backup: true });
    this.db = this.block.open();
    this.version = this.block.version;
    this.prune();
  }

  // What's kept, at start and every hour (core/actors runs the clock):
  // messages and pins for a week, claims until they expire. Actors' events
  // for a week and at most ACTOR_EVENTS_KEPT each, their targets for a
  // week after they were last touched, registrations a day, revoked tokens
  // a week; the person at the console's events, targets and status for a
  // day. A copy made before a migration holds all of that too: it goes a
  // week after it was made, here as well as when the file is opened, since
  // XenoNet can run for weeks.
  prune(now = Date.now()) {
    this.block.pruneBackups(now);
    this.transaction(() => {
      this.db.prepare("DELETE FROM deliveries WHERE message_id IN (SELECT id FROM messages WHERE created_at < ?)").run(now - WEEK_MS);
      this.db.prepare("DELETE FROM messages WHERE created_at < ?").run(now - WEEK_MS);
      this.db.prepare("DELETE FROM claims WHERE expires_at < ?").run(now);
      this.db.prepare("DELETE FROM pins WHERE at < ?").run(now - WEEK_MS);
      this.db.prepare("DELETE FROM actor_events WHERE at < ? OR (actor = ? AND at < ?)").run(now - WEEK_MS, YOU, now - DAY_MS);
      for (const { actor } of this.db.prepare("SELECT actor FROM actor_events GROUP BY actor HAVING count(*) > ?").all(ACTOR_EVENTS_KEPT)) {
        const last = this.db.prepare("SELECT at, rowid FROM actor_events WHERE actor = ? ORDER BY at DESC, rowid DESC LIMIT 1 OFFSET ?").get(actor, ACTOR_EVENTS_KEPT - 1);
        this.db.prepare("DELETE FROM actor_events WHERE actor = ? AND (at < ? OR (at = ? AND rowid < ?))").run(actor, last.at, last.at, last.rowid);
      }
      this.db.prepare("DELETE FROM actor_targets WHERE last_at < ? OR (actor = ? AND last_at < ?)").run(now - WEEK_MS, YOU, now - DAY_MS);
      this.db.prepare("DELETE FROM actor_requests WHERE created_at < ?").run(now - DAY_MS);
      this.db.prepare("DELETE FROM actor_revoked WHERE at < ?").run(now - WEEK_MS);
      this.db.prepare("UPDATE actors SET status = NULL, status_at = NULL WHERE slug = ? AND status_at < ?").run(YOU_SLUG, now - DAY_MS);
      // Other hosts' claims until they lapse; handoffs a week past theirs.
      this.db.prepare("DELETE FROM net_claims WHERE until < ?").run(now);
      this.db.prepare("DELETE FROM handoffs WHERE expires_at < ?").run(now - WEEK_MS);
      this.pruneAudit(now);
    });
  }

  // The remote reads logged: a week of them, and at most AUDIT_KEPT.
  pruneAudit(now = Date.now()) {
    this.db.prepare("DELETE FROM assistant_audit WHERE at < ?").run(now - WEEK_MS);
    const last = this.db.prepare("SELECT id FROM assistant_audit ORDER BY id DESC LIMIT 1 OFFSET ?").get(AUDIT_KEPT - 1);
    if (last) this.db.prepare("DELETE FROM assistant_audit WHERE id < ?").run(last.id);
  }

  // A statement prepared once, for what runs often (an actor's reports).
  #statements = new Map();
  #sql(text) {
    let st = this.#statements.get(text);
    if (!st) this.#statements.set(text, (st = this.db.prepare(text)));
    return st;
  }

  // `fn` in a transaction of its own, unless one is under way already.
  transaction(fn) {
    if (this.db.isTransaction) return fn();
    this.db.exec("BEGIN");
    try {
      const out = fn();
      this.db.exec("COMMIT");
      return out;
    } catch (err) {
      this.db.exec("ROLLBACK");
      throw err;
    }
  }

  // ---------- contacts ----------

  touch(agent, { app, cwd = null, via, pid = null }, now = Date.now()) {
    this.db
      .prepare("INSERT INTO contacts (agent, app, cwd, via, at, pid) VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT(agent) DO UPDATE SET app = excluded.app, cwd = COALESCE(excluded.cwd, contacts.cwd), via = excluded.via, at = excluded.at, pid = COALESCE(excluded.pid, contacts.pid), ended_at = NULL")
      .run(agent, app, cwd, via, now, pid);
  }
  ended(agent, now = Date.now()) {
    this.db.prepare("UPDATE contacts SET ended_at = ? WHERE agent = ?").run(now, agent);
  }
  contact = (agent) => this.db.prepare("SELECT * FROM contacts WHERE agent = ?").get(agent) ?? null;
  contacts = (since) => this.db.prepare("SELECT * FROM contacts WHERE at >= ?").all(since);

  // ---------- pins: the process (pid and start time) holding a session ----------

  pins = (agent) => this.db.prepare("SELECT pid, start, sure, at FROM pins WHERE agent = ?").all(agent);
  // The sessions pinned to a process: which session a Claude Code or Codex
  // process holds, for attributing the ones an actor started.
  pinnedTo = (pid) => this.#sql("SELECT agent, start, sure, at FROM pins WHERE pid = ?").all(pid);
  pin(agent, { pid, start }, sure, now = Date.now()) {
    this.db.prepare("INSERT INTO pins (agent, pid, start, sure, at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(agent, pid, start) DO UPDATE SET sure = MAX(pins.sure, excluded.sure), at = excluded.at").run(agent, pid, start, sure ? 1 : 0, now);
  }
  // Takes back the guesses for an agent once its own process has spoken.
  dropGuesses(agent) {
    this.db.prepare("DELETE FROM pins WHERE agent = ? AND sure = 0").run(agent);
  }
  unpin(agent, { pid, start }) {
    this.db.prepare("DELETE FROM pins WHERE agent = ? AND pid = ? AND start = ?").run(agent, pid, start);
  }

  // ---------- messages ----------

  send({ sender, target, scope = null, body, replyTo = null, recipients, envelope = null }, now = Date.now()) {
    const { lastInsertRowid } = this.db.prepare("INSERT INTO messages (sender, target, scope, body, reply_to, created_at, envelope) VALUES (?, ?, ?, ?, ?, ?, ?)").run(sender, target, scope, body, replyTo, now, envelope ? JSON.stringify(envelope) : null);
    const id = Number(lastInsertRowid);
    for (const r of recipients) this.deliver(id, r);
    return id;
  }
  deliver(id, agent) {
    this.db.prepare("INSERT OR IGNORE INTO deliveries (message_id, agent) VALUES (?, ?)").run(id, agent);
  }
  // Messages to everyone in a repo since `since`, for an agent that joins it.
  scoped = (scope, since) => this.db.prepare("SELECT * FROM messages WHERE scope = ? AND created_at >= ? ORDER BY id").all(scope, since);
  message = (id) => this.db.prepare("SELECT * FROM messages WHERE id = ?").get(id) ?? null;
  unseen = (agent) =>
    this.db.prepare("SELECT m.* FROM deliveries d JOIN messages m ON m.id = d.message_id WHERE d.agent = ? AND d.seen_at IS NULL ORDER BY m.id").all(agent);
  markSeen(agent, ids, now = Date.now()) {
    const mark = this.db.prepare("UPDATE deliveries SET seen_at = ? WHERE agent = ? AND message_id = ? AND seen_at IS NULL");
    for (const id of ids) mark.run(now, agent, id);
  }
  // A message pushed into a session that never showed it got there waits
  // for the session's hooks again (core/delivery/router.js).
  markUnseen(agent, ids) {
    const mark = this.db.prepare("UPDATE deliveries SET seen_at = NULL WHERE agent = ? AND message_id = ?");
    for (const id of ids) mark.run(agent, id);
  }
  // Who a message went to, and who has seen it.
  deliveries = (id) => this.db.prepare("SELECT agent, seen_at FROM deliveries WHERE message_id = ?").all(id);
  recent = (since) => this.db.prepare("SELECT * FROM messages WHERE created_at >= ? ORDER BY id").all(since);

  // ---------- claims ----------

  claim(agent, pattern, note, expiresAt, now = Date.now()) {
    this.db.prepare("DELETE FROM claims WHERE agent = ? AND pattern = ?").run(agent, pattern);
    this.db.prepare("INSERT INTO claims (agent, pattern, note, created_at, expires_at) VALUES (?, ?, ?, ?, ?)").run(agent, pattern, note ?? null, now, expiresAt);
  }
  release(agent, patterns = null) {
    if (!patterns) return this.db.prepare("DELETE FROM claims WHERE agent = ?").run(agent).changes;
    let n = 0;
    for (const p of patterns) n += this.db.prepare("DELETE FROM claims WHERE agent = ? AND pattern = ?").run(agent, p).changes;
    return n;
  }
  claims = (now = Date.now()) => this.db.prepare("SELECT * FROM claims WHERE expires_at >= ? ORDER BY created_at").all(now);

  // ---------- focus ----------

  setFocus(agent, text, now = Date.now()) {
    if (!text) return this.db.prepare("DELETE FROM focus WHERE agent = ?").run(agent);
    this.db.prepare("INSERT INTO focus (agent, text, at) VALUES (?, ?, ?) ON CONFLICT(agent) DO UPDATE SET text = excluded.text, at = excluded.at").run(agent, text, now);
  }
  focus = (agent) => this.db.prepare("SELECT text, at FROM focus WHERE agent = ?").get(agent) ?? null;

  // ---------- actors (core/actors) ----------

  actorRows = () => this.db.prepare("SELECT * FROM actors ORDER BY created_at, rowid").all();
  actorRow = (slug) => this.db.prepare("SELECT * FROM actors WHERE slug = ?").get(slug) ?? null;
  // A row as core/actors keeps it: everything but the token, which only
  // registering, allowing and rotating touch. Columns left out take their
  // defaults on a new row, and are cleared on one that's there.
  saveActor(row) {
    const values = { ...ACTOR_DEFAULTS };
    for (const k of Object.keys(values)) if (row[k] !== undefined) values[k] = row[k];
    this
      .#sql(
        `INSERT INTO actors (slug, kind, name, product, via, capabilities, heartbeat_ms, identity, enabled, created_at, allowed_at, last_seen, presence, presence_at, status, status_at, seq)
         VALUES (:slug, :kind, :name, :product, :via, :capabilities, :heartbeat_ms, :identity, :enabled, :created_at, :allowed_at, :last_seen, :presence, :presence_at, :status, :status_at, :seq)
         ON CONFLICT(slug) DO UPDATE SET kind = excluded.kind, name = excluded.name, product = excluded.product, via = excluded.via, capabilities = excluded.capabilities,
           heartbeat_ms = excluded.heartbeat_ms, identity = excluded.identity, enabled = excluded.enabled, allowed_at = excluded.allowed_at, last_seen = excluded.last_seen,
           presence = excluded.presence, presence_at = excluded.presence_at, status = excluded.status, status_at = excluded.status_at, seq = excluded.seq`,
      )
      .run(values);
  }
  // An actor gone: its row, events and targets. Messages it sent stay with
  // their recipients until they age out.
  removeActor(slug) {
    this.transaction(() => {
      this.db.prepare("DELETE FROM actors WHERE slug = ?").run(slug);
      this.db.prepare("DELETE FROM actor_events WHERE actor = ?").run(`actor:${slug}`);
      this.db.prepare("DELETE FROM actor_targets WHERE actor = ?").run(`actor:${slug}`);
    });
  }

  // An actor's record let go and the actor kept: its events and targets
  // (yours, when you turn yourself off the map).
  forgetActor(slug) {
    this.transaction(() => {
      this.db.prepare("DELETE FROM actor_events WHERE actor = ?").run(`actor:${slug}`);
      this.db.prepare("DELETE FROM actor_targets WHERE actor = ?").run(`actor:${slug}`);
    });
  }

  addActorEvent({ id, actor, at, kind, target = null, detail = null, via, facts = null, ref = null }) {
    this.#sql("INSERT INTO actor_events (id, actor, at, kind, target, detail, via, facts, ref) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)").run(id, actor, at, kind, target, detail, via, facts, ref);
  }
  // An actor's last `limit` events, newest last.
  lastActorEvents = (actor, limit = 20) => this.#sql("SELECT * FROM actor_events WHERE actor = ? ORDER BY at DESC, rowid DESC LIMIT ?").all(actor, limit).reverse();
  // Events since `since`, oldest first: only `only`'s, or all but `except`'s.
  actorEventsSince = (since, { limit = 1000, except = null, only = null } = {}) =>
    only
      ? this.#sql("SELECT * FROM actor_events WHERE actor = ? AND at >= ? ORDER BY at, rowid LIMIT ?").all(only, since, limit)
      : this.#sql("SELECT * FROM actor_events WHERE at >= ? AND actor IS NOT ? ORDER BY at, rowid LIMIT ?").all(since, except, limit);
  // How many events of each kind each actor has had since `since`.
  actorEventCounts = (since) => this.#sql("SELECT actor, kind, count(*) AS n FROM actor_events WHERE at >= ? GROUP BY actor, kind").all(since);
  // The targets of each actor's spawned events since `since`, the first
  // time from each source: who started what, for what each actor's work
  // costs (core/actors/spend.js).
  actorSpawns = (since) => this.#sql("SELECT actor, target, via, MIN(at) AS at FROM actor_events WHERE kind = 'spawned' AND target IS NOT NULL AND at >= ? GROUP BY actor, target, via").all(since);
  // Whether an actor reported an event with this `ref` since `since`.
  hasActorRef = (actor, ref, since) => !!this.#sql("SELECT 1 FROM actor_events WHERE actor = ? AND ref = ? AND at >= ? LIMIT 1").get(actor, ref, since);

  actorTargets = () => this.db.prepare("SELECT * FROM actor_targets").all();
  saveActorTarget({ actor, target, relation, since, last_at }) {
    this
      .#sql("INSERT INTO actor_targets (actor, target, relation, since, last_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(actor, target, relation) DO UPDATE SET since = MIN(actor_targets.since, excluded.since), last_at = MAX(actor_targets.last_at, excluded.last_at)")
      .run(actor, target, relation, since, last_at);
  }
  dropActorTarget(actor, target, relation) {
    this.#sql("DELETE FROM actor_targets WHERE actor = ? AND target = ? AND relation = ?").run(actor, target, relation);
  }
  // What an actor's events of one kind said as facts (count, paths…) since
  // `since`, newest first, with the source that said it: an actor's edits,
  // for the engine.
  actorFactsSince = (actor, kind, since) => this.#sql("SELECT at, facts, via FROM actor_events WHERE actor = ? AND kind = ? AND at >= ? AND facts IS NOT NULL ORDER BY at DESC, rowid DESC LIMIT 200").all(actor, kind, since);

  // ---------- actors' tokens, registrations, revoked tokens ----------
  //
  // A token is kept only as its SHA-256 (core/actors/tokens.js), on the
  // actor's row once you allow it, on its registration while it waits, and
  // among the revoked for a week after, so a program hears why it stopped.

  // A registered actor's token, and when it was last used: one unused for
  // 30 days is revoked.
  setActorToken(slug, hash, at) {
    this.db.prepare("UPDATE actors SET token_hash = ?, token_at = ? WHERE slug = ?").run(hash, at, slug);
  }
  dropActorToken(slug) {
    this.db.prepare("UPDATE actors SET token_hash = NULL, token_at = NULL WHERE slug = ?").run(slug);
  }
  touchActorToken(slug, at) {
    this.#sql("UPDATE actors SET token_at = ? WHERE slug = ?").run(at, slug);
  }
  actorByToken = (hash) => this.#sql("SELECT slug, token_hash, token_at FROM actors WHERE token_hash = ?").get(hash) ?? null;
  // Each actor's token as XenoNet's window may see it: whether it has one,
  // and when it was last used (or made). Never the hash.
  actorTokenTimes = () =>
    this.db
      .prepare("SELECT slug, token_hash IS NOT NULL AS has_token, token_at, gateway_hash IS NOT NULL AS has_gateway, gateway_at FROM actors")
      .all()
      .map((r) => ({ slug: r.slug, hasToken: !!r.has_token, at: r.token_at ?? null, hasGateway: !!r.has_gateway, gatewayAt: r.gateway_at ?? null }));
  actorToken = (slug) => this.#sql("SELECT token_hash, token_at FROM actors WHERE slug = ? AND token_hash IS NOT NULL").get(slug) ?? null;
  staleActorTokens = (before) => this.#sql("SELECT slug, token_hash, token_at FROM actors WHERE token_hash IS NOT NULL AND token_at < ?").all(before);
  // Actors you allowed whose token is gone: it went unused for 30 days, so
  // the actor stays in the list, offline, until it's given a new one or
  // removed (the doctor says so). Their slugs and names, never a hash.
  // One that reports only through the gateway has no token of its own, and
  // isn't listed while its gateway token stands.
  expiredActorTokens = () => this.db.prepare("SELECT slug, name FROM actors WHERE allowed_at IS NOT NULL AND token_hash IS NULL AND gateway_hash IS NULL ORDER BY allowed_at, rowid").all().map((r) => ({ slug: r.slug, name: r.name }));

  // A registration waiting for you: what it asked (ask, JSON), its token's
  // hash, who asked (asker, JSON), and when it lapses.
  addActorRequest({ id, slug, ask, token_hash, asker = null, created_at, expires_at }) {
    this.db.prepare("INSERT INTO actor_requests (id, slug, ask, token_hash, asker, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)").run(id, slug, ask, token_hash, asker, created_at, expires_at);
  }
  actorRequest = (id) => this.db.prepare("SELECT * FROM actor_requests WHERE id = ?").get(id) ?? null;
  actorRequestByToken = (hash) => this.#sql("SELECT * FROM actor_requests WHERE token_hash = ?").get(hash) ?? null;
  waitingActorRequests = (now) => this.#sql("SELECT * FROM actor_requests WHERE decision IS NULL AND expires_at > ? ORDER BY created_at, rowid").all(now);
  lapsedActorRequests = (now) => this.#sql("SELECT * FROM actor_requests WHERE decision IS NULL AND expires_at <= ?").all(now);
  // Answers a waiting registration (allow, deny, expired or replaced);
  // whether it was still waiting.
  decideActorRequest(id, decision, at) {
    return this.db.prepare("UPDATE actor_requests SET decision = ?, decided_at = ? WHERE id = ? AND decision IS NULL").run(decision, at, id).changes > 0;
  }
  // When each registration since `since` was made, oldest first.
  actorRequestTimes = (since) => this.db.prepare("SELECT created_at FROM actor_requests WHERE created_at >= ? ORDER BY created_at").all(since).map((r) => r.created_at);

  // A registered actor's gateway token, kept the same way as its own:
  // whether it has one, and when it was last used, for the window.
  setGatewayToken(slug, hash, at) {
    this.db.prepare("UPDATE actors SET gateway_hash = ?, gateway_at = ? WHERE slug = ?").run(hash, at, slug);
  }
  dropGatewayToken(slug) {
    this.db.prepare("UPDATE actors SET gateway_hash = NULL, gateway_at = NULL WHERE slug = ?").run(slug);
  }
  touchGatewayToken(slug, at) {
    this.#sql("UPDATE actors SET gateway_at = ? WHERE slug = ?").run(at, slug);
  }
  actorByGatewayToken = (hash) => this.#sql("SELECT slug, gateway_hash, gateway_at FROM actors WHERE gateway_hash = ?").get(hash) ?? null;
  gatewayToken = (slug) => this.#sql("SELECT gateway_hash, gateway_at FROM actors WHERE slug = ? AND gateway_hash IS NOT NULL").get(slug) ?? null;
  staleGatewayTokens = (before) => this.#sql("SELECT slug, gateway_hash, gateway_at FROM actors WHERE gateway_hash IS NOT NULL AND gateway_at < ?").all(before);

  // A token that stopped working, and why: rotated, denied, expired, left
  // or removed. The first reason stands.
  revokeActorToken({ token_hash, actor, reason, at }) {
    this.db.prepare("INSERT OR IGNORE INTO actor_revoked (token_hash, actor, reason, at) VALUES (?, ?, ?, ?)").run(token_hash, actor, reason, at);
  }
  revokedActorToken = (hash) => this.#sql("SELECT * FROM actor_revoked WHERE token_hash = ?").get(hash) ?? null;

  // ---------- remote connections (core/actors/connections.js) ----------
  //
  // A connection belongs to an actor's row, a grant to a connection; a
  // grant's credential is kept as its SHA-256 only. Lists are JSON.

  meta = (key) => this.#sql("SELECT value FROM assistant_meta WHERE key = ?").get(key)?.value ?? null;
  setMeta(key, value) {
    this.db.prepare("INSERT INTO assistant_meta (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value").run(key, String(value));
  }

  // A place's own id for remote readers (prj_<uuid>), made the first time
  // it's asked for; `make` makes one.
  projectIdOf(place, make, now = Date.now()) {
    const had = this.#sql("SELECT project_id FROM assistant_places WHERE place = ?").get(place);
    if (had) return had.project_id;
    const id = make();
    this.db.prepare("INSERT INTO assistant_places (place, project_id, created_at) VALUES (?, ?, ?)").run(place, id, now);
    return id;
  }
  projectIdFor = (place) => this.#sql("SELECT project_id FROM assistant_places WHERE place = ?").get(place)?.project_id ?? null;
  placeOfProject = (projectId) => this.#sql("SELECT place FROM assistant_places WHERE project_id = ?").get(projectId)?.place ?? null;

  addConnection(row) {
    this.db
      .prepare(
        `INSERT INTO actor_connections (connection_id, actor_slug, adapter_id, provider, transport_kind, transport_binding, label, device_alias, audience_description, audience_verification, audience_scope, state, created_at)
         VALUES (:connection_id, :actor_slug, :adapter_id, :provider, :transport_kind, :transport_binding, :label, :device_alias, :audience_description, :audience_verification, :audience_scope, 'active', :created_at)`,
      )
      .run(row);
  }
  connection = (id) => this.#sql("SELECT * FROM actor_connections WHERE connection_id = ?").get(id) ?? null;
  connectionsOf = (slug) => this.#sql("SELECT * FROM actor_connections WHERE actor_slug = ? ORDER BY created_at, rowid").all(slug);
  connections = () => this.db.prepare("SELECT * FROM actor_connections ORDER BY created_at, rowid").all();
  // How a connection's transport is set up (a tunnel's id, where its key
  // is kept, whether it starts with XenoNet), as JSON; never a secret.
  setTransportBinding(id, binding) {
    this.db.prepare("UPDATE actor_connections SET transport_binding = ? WHERE connection_id = ?").run(binding, id);
  }
  endConnection(id, at) {
    this.db.prepare("UPDATE actor_connections SET state = 'revoked', revoked_at = COALESCE(revoked_at, ?) WHERE connection_id = ?").run(at, id);
  }

  addGrant(row) {
    this.db
      .prepare(
        `INSERT INTO actor_grants (grant_id, connection_id, device_id, permissions, project_allowlist, field_policy, data_mode, lookback_ms, revision, token_hash, created_at, expires_at)
         VALUES (:grant_id, :connection_id, :device_id, :permissions, :project_allowlist, :field_policy, :data_mode, :lookback_ms, 1, :token_hash, :created_at, :expires_at)`,
      )
      .run(row);
  }
  grant = (id) => this.#sql("SELECT * FROM actor_grants WHERE grant_id = ?").get(id) ?? null;
  grants = () => this.db.prepare("SELECT * FROM actor_grants ORDER BY created_at, rowid").all();
  // A grant's scope changed (its projects, fields or credential): its
  // revision moves on, so what was read under the old one can't be
  // continued or served again.
  changeGrant(id, { project_allowlist, field_policy, data_mode, token_hash, expires_at }) {
    const sets = Object.entries({ project_allowlist, field_policy, data_mode, token_hash, expires_at }).filter(([, v]) => v !== undefined);
    const sql = `UPDATE actor_grants SET ${[...sets.map(([k]) => `${k} = ?`), "revision = revision + 1"].join(", ")} WHERE grant_id = ? AND revoked_at IS NULL`;
    return this.db.prepare(sql).run(...sets.map(([, v]) => v), id).changes > 0;
  }
  // Revoked, for good: the first reason stands, and the revision moves on.
  revokeGrant(id, reason, at) {
    return this.db.prepare("UPDATE actor_grants SET revoked_at = ?, revoked_reason = ?, revision = revision + 1 WHERE grant_id = ? AND revoked_at IS NULL").run(at, reason, id).changes > 0;
  }
  touchGrant(id, at) {
    this.#sql("UPDATE actor_grants SET last_used_at = ? WHERE grant_id = ?").run(at, id);
    this.#sql("UPDATE actor_connections SET last_used_at = ? WHERE connection_id = (SELECT connection_id FROM actor_grants WHERE grant_id = ?)").run(at, id);
  }

  addAccountLink(row) {
    this.db
      .prepare(
        `INSERT INTO actor_account_links (link_id, actor_slug, identity_id, issuer, provider, workspace, proof_status, proof_method, subject_binding, checked_at, expires_at, created_at)
         VALUES (:link_id, :actor_slug, :identity_id, :issuer, :provider, :workspace, :proof_status, :proof_method, :subject_binding, :checked_at, :expires_at, :created_at)`,
      )
      .run(row);
  }
  accountLinks = (slug) => this.#sql("SELECT * FROM actor_account_links WHERE actor_slug = ? ORDER BY created_at, rowid").all(slug);
  dropAccountLink(id) {
    this.db.prepare("DELETE FROM actor_account_links WHERE link_id = ?").run(id);
  }

  addExternalRef(row) {
    this.db
      .prepare(
        `INSERT INTO actor_external_refs (external_ref_id, actor_slug, adapter_id, issuer, account_binding, external_actor_id, verified_at)
         VALUES (:external_ref_id, :actor_slug, :adapter_id, :issuer, :account_binding, :external_actor_id, :verified_at)`,
      )
      .run(row);
  }
  externalRef = ({ adapter_id, issuer, account_binding, external_actor_id }) =>
    this.#sql("SELECT * FROM actor_external_refs WHERE adapter_id = ? AND issuer = ? AND account_binding = ? AND external_actor_id = ?").get(adapter_id, issuer, account_binding, external_actor_id) ?? null;
  externalRefs = (slug) => this.#sql("SELECT * FROM actor_external_refs WHERE actor_slug = ?").all(slug);

  addAudit({ at, kind, request_id = null, grant_id = null, grant_revision = null, actor = null, connection_id = null, operation, result, latency_ms = null, bytes = null, projects = null }) {
    this.#sql("INSERT INTO assistant_audit (at, kind, request_id, grant_id, grant_revision, actor, connection_id, operation, result, latency_ms, bytes, projects) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)").run(at, kind, request_id, grant_id, grant_revision, actor, connection_id, operation, result, latency_ms, bytes, projects);
  }
  // The newest `limit` records since `since`, newest first.
  audit = ({ since = 0, limit = 200, grant = null } = {}) =>
    grant
      ? this.db.prepare("SELECT * FROM assistant_audit WHERE at >= ? AND grant_id = ? ORDER BY id DESC LIMIT ?").all(since, grant, limit)
      : this.db.prepare("SELECT * FROM assistant_audit WHERE at >= ? ORDER BY id DESC LIMIT ?").all(since, limit);

  close() {
    this.block.close();
  }
}
