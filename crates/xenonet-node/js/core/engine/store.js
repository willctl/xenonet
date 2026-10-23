// core/engine/store.js in Rust (crates/xenonet-engine/src/store.rs): what
// the engine remembers, in engine.sqlite. Rust opens the file (rusqlite,
// SQLite bundled), migrates it with the same numbered steps, keeps what's
// kept and runs every call, binding and reading as node:sqlite does. The
// twin keeps the calls' JS signatures and defaults, and `db`, a
// DatabaseSync of node:sqlite's kind over the same connection, for the
// tests that look into it.
import { migrateSqlite } from "../../../../../core/blocks/sqlite.js";
import { native } from "../../index.js";
import { Block } from "../blocks/lifecycle.js";

// The Rust store behind a twin, for the Rust parts that use it directly.
export const STORE = Symbol.for("xenonet.engine-store");

export const ACTOR_EVENTS_KEPT = 10000;
export const AUDIT_KEPT = 20000;

const columns = (db, table) => db.prepare(`PRAGMA table_info(${table})`).all().map((c) => c.name);

// Each step as the JS ran it on a database (a DatabaseSync, or this
// store's own): the statements are Rust's, given the columns it has.
export const ENGINE_MIGRATIONS = Array.from({ length: native.engineVersion() }, (_, i) => (db) => {
  for (const sql of native.engineStep(i, (table, column) => columns(db, table).includes(column))) db.exec(sql);
});

export const ENGINE_VERSION = ENGINE_MIGRATIONS.length;

export const migrateEngine = (db, migrations = ENGINE_MIGRATIONS) => migrateSqlite(db, migrations);

// The store's block, for the list of blocks and their health, with what
// core/blocks/sqlite.js's SqliteStore showed of the file.
class EngineBlock extends Block {
  #store;
  constructor(store) {
    super({ name: "engine", kind: "store", core: store.block() });
    this.#store = store;
    this.file = store.file;
  }
  get backupFile() {
    return this.#store.backupFile ?? null;
  }
  get version() {
    return this.#store.version;
  }
}

class StatementSync {
  #core;
  #sql;
  constructor(core, sql) {
    core.prepare(sql);
    this.#core = core;
    this.#sql = sql;
  }
  all(...params) {
    return this.#core.all(this.#sql, params);
  }
  get(...params) {
    return this.#core.get(this.#sql, params);
  }
  run(...params) {
    return this.#core.run(this.#sql, params);
  }
}

class DatabaseSync {
  #core;
  constructor(core) {
    this.#core = core;
  }
  prepare(sql) {
    return new StatementSync(this.#core, String(sql));
  }
  exec(sql) {
    this.#core.exec(String(sql));
  }
  get isTransaction() {
    return this.#core.inTransaction;
  }
  get isOpen() {
    return this.#core.isOpen;
  }
  close() {
    try {
      this.#core.close(Date.now());
    } finally {
      this.block.stop();
    }
  }
}

export class EngineStore {
  #core;
  #c;
  constructor(file) {
    const core = new native.EngineStoreCore(String(file), Date.now());
    this.#core = core;
    this.#c = (name, args) => core.call(name, args);
    this[STORE] = core;
    this.block = new EngineBlock(core);
    this.db = new DatabaseSync(core);
    this.version = core.version;
  }

  prune(now = Date.now()) {
    this.#core.prune(now);
  }
  pruneAudit(now = Date.now()) {
    this.#c("pruneAudit", [now]);
  }

  transaction(fn) {
    return this.#core.transaction(fn);
  }

  // ---------- contacts ----------

  touch(agent, contact, now = Date.now()) {
    this.#c("touch", [agent, contact, now]);
  }
  ended(agent, now = Date.now()) {
    this.#c("ended", [agent, now]);
  }
  contact = (agent) => this.#c("contact", [agent]);
  contacts = (since) => this.#c("contacts", [since]);

  // ---------- pins ----------

  pins = (agent) => this.#c("pins", [agent]);
  pinnedTo = (pid) => this.#c("pinnedTo", [pid]);
  pin(agent, p, sure, now = Date.now()) {
    this.#c("pin", [agent, p, sure, now]);
  }
  dropGuesses(agent) {
    this.#c("dropGuesses", [agent]);
  }
  unpin(agent, p) {
    this.#c("unpin", [agent, p]);
  }

  // ---------- messages ----------

  send(m, now = Date.now()) {
    return this.#c("send", [m, now]);
  }
  deliver(id, agent) {
    this.#c("deliver", [id, agent]);
  }
  scoped = (scope, since) => this.#c("scoped", [scope, since]);
  message = (id) => this.#c("message", [id]);
  unseen = (agent) => this.#c("unseen", [agent]);
  markSeen(agent, ids, now = Date.now()) {
    this.#c("markSeen", [agent, [...ids], now]);
  }
  markUnseen(agent, ids) {
    this.#c("markUnseen", [agent, [...ids]]);
  }
  deliveries = (id) => this.#c("deliveries", [id]);
  recent = (since) => this.#c("recent", [since]);

  // ---------- claims ----------

  claim(agent, pattern, note, expiresAt, now = Date.now()) {
    this.#c("claim", [agent, pattern, note, expiresAt, now]);
  }
  release(agent, patterns = null) {
    return this.#c("release", [agent, patterns && typeof patterns !== "string" ? [...patterns] : patterns]);
  }
  claims = (now = Date.now()) => this.#c("claims", [now]);

  // ---------- focus ----------

  setFocus(agent, text, now = Date.now()) {
    return this.#c("setFocus", [agent, text, now]);
  }
  focus = (agent) => this.#c("focus", [agent]);

  // ---------- actors ----------

  actorRows = () => this.#c("actorRows", []);
  actorRow = (slug) => this.#c("actorRow", [slug]);
  saveActor(row) {
    this.#c("saveActor", [row]);
  }
  removeActor(slug) {
    this.#c("removeActor", [slug]);
  }
  forgetActor(slug) {
    this.#c("forgetActor", [slug]);
  }
  addActorEvent(event) {
    this.#c("addActorEvent", [event]);
  }
  lastActorEvents = (actor, limit = 20) => this.#c("lastActorEvents", [actor, limit]);
  actorEventsSince = (since, options = {}) => this.#c("actorEventsSince", [since, options]);
  actorEventCounts = (since) => this.#c("actorEventCounts", [since]);
  actorSpawns = (since) => this.#c("actorSpawns", [since]);
  hasActorRef = (actor, ref, since) => this.#c("hasActorRef", [actor, ref, since]);
  actorTargets = () => this.#c("actorTargets", []);
  saveActorTarget(target) {
    this.#c("saveActorTarget", [target]);
  }
  dropActorTarget(actor, target, relation) {
    this.#c("dropActorTarget", [actor, target, relation]);
  }
  actorFactsSince = (actor, kind, since) => this.#c("actorFactsSince", [actor, kind, since]);

  // ---------- actors' tokens, registrations, revoked tokens ----------

  setActorToken(slug, hash, at) {
    this.#c("setActorToken", [slug, hash, at]);
  }
  dropActorToken(slug) {
    this.#c("dropActorToken", [slug]);
  }
  touchActorToken(slug, at) {
    this.#c("touchActorToken", [slug, at]);
  }
  actorByToken = (hash) => this.#c("actorByToken", [hash]);
  actorTokenTimes = () => this.#c("actorTokenTimes", []);
  actorToken = (slug) => this.#c("actorToken", [slug]);
  staleActorTokens = (before) => this.#c("staleActorTokens", [before]);
  expiredActorTokens = () => this.#c("expiredActorTokens", []);
  issuedTokenHashes = () => this.#c("issuedTokenHashes", []);
  addActorRequest(request) {
    this.#c("addActorRequest", [request]);
  }
  actorRequest = (id) => this.#c("actorRequest", [id]);
  actorRequestByToken = (hash) => this.#c("actorRequestByToken", [hash]);
  waitingActorRequests = (now) => this.#c("waitingActorRequests", [now]);
  lapsedActorRequests = (now) => this.#c("lapsedActorRequests", [now]);
  decideActorRequest(id, decision, at) {
    return this.#c("decideActorRequest", [id, decision, at]);
  }
  actorRequestTimes = (since) => this.#c("actorRequestTimes", [since]);
  setGatewayToken(slug, hash, at) {
    this.#c("setGatewayToken", [slug, hash, at]);
  }
  dropGatewayToken(slug) {
    this.#c("dropGatewayToken", [slug]);
  }
  touchGatewayToken(slug, at) {
    this.#c("touchGatewayToken", [slug, at]);
  }
  actorByGatewayToken = (hash) => this.#c("actorByGatewayToken", [hash]);
  gatewayToken = (slug) => this.#c("gatewayToken", [slug]);
  staleGatewayTokens = (before) => this.#c("staleGatewayTokens", [before]);
  revokeActorToken(revoked) {
    this.#c("revokeActorToken", [revoked]);
  }
  revokedActorToken = (hash) => this.#c("revokedActorToken", [hash]);

  // ---------- remote connections ----------

  meta = (key) => this.#c("meta", [key]);
  setMeta(key, value) {
    this.#c("setMeta", [key, value]);
  }
  projectIdOf(place, make, now = Date.now()) {
    return this.#core.projectIdOf(place, make, now);
  }
  projectIdFor = (place) => this.#c("projectIdFor", [place]);
  placeOfProject = (projectId) => this.#c("placeOfProject", [projectId]);
  addConnection(row) {
    this.#c("addConnection", [row]);
  }
  connection = (id) => this.#c("connection", [id]);
  connectionsOf = (slug) => this.#c("connectionsOf", [slug]);
  connections = () => this.#c("connections", []);
  setTransportBinding(id, binding) {
    this.#c("setTransportBinding", [id, binding]);
  }
  endConnection(id, at) {
    this.#c("endConnection", [id, at]);
  }
  addGrant(row) {
    this.#c("addGrant", [row]);
  }
  grant = (id) => this.#c("grant", [id]);
  grants = () => this.#c("grants", []);
  changeGrant(id, change) {
    return this.#c("changeGrant", [id, change]);
  }
  revokeGrant(id, reason, at) {
    return this.#c("revokeGrant", [id, reason, at]);
  }
  touchGrant(id, at) {
    this.#c("touchGrant", [id, at]);
  }
  addAccountLink(row) {
    this.#c("addAccountLink", [row]);
  }
  accountLinks = (slug) => this.#c("accountLinks", [slug]);
  dropAccountLink(id) {
    this.#c("dropAccountLink", [id]);
  }
  addExternalRef(row) {
    this.#c("addExternalRef", [row]);
  }
  externalRef = (key) => this.#c("externalRef", [key]);
  externalRefs = (slug) => this.#c("externalRefs", [slug]);
  addAudit(record) {
    this.#c("addAudit", [record]);
  }
  audit = (options = {}) => this.#c("audit", [options]);

  close() {
    try {
      this.#core.close(Date.now());
    } finally {
      this.block.stop();
    }
  }
}
