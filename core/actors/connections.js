// Remote connections (docs/design/cloud-assistants.md, sections 3 and 4):
// how a remote actor, such as an OpenAI dot in the cloud, reaches XenoNet to
// read what's going on, and only that.
//
// A connection is one way an actor reaches XenoNet (a tunnel, or a local MCP
// client), and its grant what it may read through it: the projects you
// picked (none unless you pick some), whether names and titles go too
// (off unless you turn them on), how far back, and until when (30 days
// unless you choose otherwise). It belongs to an actor in the actors'
// table, the one the map, the Actors panel and agents' peers show, never
// to a registry of its own: an actor can have several connections, and
// one account several actors. Revoking a connection leaves its actor, its
// history, its other connections and any observer of it as they were; a
// registered actor's own token is never touched.
//
// Everything here is yours, from XenoNet's window or your own terminal
// (server/routes/assistant.js), never from a connection itself:
//
//   pair     a new connection and its grant, at once, with its credential
//            written where only your account can read it (credentials.js);
//            the window shows the command the tunnel client runs
//   update   its projects and fields; any change moves the grant's
//            revision on, so nothing read before it is continued or served
//            from a cache after it
//   rotate   a new credential; the old one stops working at once
//   revoke   for good, for one grant; disable, for all of them until you
//            turn them on again; both work with XenoNet's service stopped
//            (xenonet assistant), through tombstones in the state folder
//
// Until you've said you verified who can call a connection's endpoint
// (audience verification owner_verified), it reads made-up data only
// (data_mode fixture). A grant stops at once if this state folder turns up
// on another machine or account: the device's id changes and every grant
// must be made again.
import crypto from "node:crypto";
import os from "node:os";
import { EventEmitter } from "node:events";
import { YOU } from "../schema/ids.js";
import { TRANSPORT_SCOPES, GRANT_ID } from "../schema/assistant.js";
import { productOf } from "./products.js";
import { checkName, slugFrom, slugRefusal, suggestSlug } from "./names.js";
import { looksLikeKey } from "./detail.js";
import { ADAPTERS, TRANSPORTS, adapterFor } from "./adapters.js";
import { assertLink, linkState } from "./account-links.js";
import * as disk from "../assistant/credentials.js";

const DAY_MS = 24 * 3600e3;
export const GRANT_DAYS = [1, 7, 14, 30, 60, 90];
export const DEFAULT_GRANT_DAYS = 30;
// How far back a grant reads, at most; you can make it less.
export const LOOKBACK_DAYS = 30;
// Remote actors beside the 16 registered ones, counted once each however
// many connections they have; connections that are active, and the
// projects one grant names.
export const REMOTE_ACTORS_MAX = 8;
export const CONNECTIONS_MAX = 16;
export const PROJECTS_MAX = 64;
// When a grant was last used is written at most this often.
const TOUCH_MS = 60e3;

export class ConnectionError extends Error {
  constructor(status, code, message, extra = {}) {
    super(message);
    Object.assign(this, { status, code, extra });
  }
}
const fail = (status, code, message, extra) => {
  throw new ConnectionError(status, code, message, extra);
};

const newId = (prefix) => `${prefix}_${crypto.randomBytes(12).toString("hex")}`;
const uuidId = (prefix) => `${prefix}_${crypto.randomUUID()}`;
const iso = (ms) => (ms == null ? null : new Date(ms).toISOString());
const parse = (s, fallback) => {
  try {
    return s == null ? fallback : JSON.parse(s);
  } catch {
    return fallback;
  }
};

// One line of your own words (a connection's name, a computer's, who can
// call an endpoint), or null for none; never a key.
function words(value, what, max) {
  if (value == null || value === "") return null;
  if (typeof value !== "string") fail(400, "bad-request", `${what} is text`);
  const line = value.replace(/[\u0000-\u001f\u007f-\u009f\u2028\u2029\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g, " ").replace(/\s+/g, " ").trim();
  if (!line) return null;
  if ([...line].length > max) fail(400, "bad-request", `${what} is at most ${max} characters`);
  if (looksLikeKey(line)) fail(400, "bad-request", `${what} looks like it holds a key or a token`);
  return line;
}

// This installation as it is: its machine, account and state folder,
// hashed. A copy of the state folder elsewhere doesn't match, so its
// grants stop rather than speak for two computers. Never sent anywhere.
export const fingerprintOf = ({ stateDir, host = os.hostname(), user = os.userInfo().username }) => crypto.createHash("sha256").update([host, user, stateDir].join("\0")).digest("hex");

// What the tunnel client runs for a grant: the adapter, holding no secret.
export const launchFor = (grant, { port = 47840 } = {}) => ({ command: "xenonet", args: ["mcp", "assistant", "--grant", grant, ...(port !== 47840 ? ["--port", String(port)] : [])] });

// `dir`: the state folder's assistant/ (credentials.js). `fingerprint`:
// this installation's (fingerprintOf). `describe(place)`: a place's name on
// the map, for your view, or null. `taken(slug)`: whether a registration
// waits for that slug. `port`: the server's, for launch commands.
export function createConnections({ actors, store = actors.store, dir, fingerprint = "unchecked", now = () => actors.now(), adapters = ADAPTERS, describe = () => null, taken = () => false, port = 47840, credentialIo = {} }) {
  const events = new EventEmitter();
  // Adapters connected now, by connection.
  const bridges = new Map();
  const touched = new Map();

  // ---------- this device ----------

  let deviceId = null;
  function device() {
    if (deviceId) return deviceId;
    const had = store.meta("device_id");
    if (had && store.meta("device_fingerprint") === fingerprint) return (deviceId = had);
    const t = now();
    const id = uuidId("dev");
    store.transaction(() => {
      store.setMeta("device_id", id);
      store.setMeta("device_fingerprint", fingerprint);
      if (!store.meta("id_key")) store.setMeta("id_key", crypto.randomBytes(32).toString("hex"));
    });
    deviceId = id;
    // Moved or copied: every grant made for the old one stops.
    if (had) for (const g of store.grants()) if (g.revoked_at == null) revokeRow(g, "device-changed", t, "system");
    return id;
  }
  // A key for ids that mustn't say what they're made of (attention items'),
  // local to this installation.
  const idKey = () => (device(), store.meta("id_key"));

  // ---------- standing ----------

  const disabled = () => disk.isDisabled(dir);
  function stateOf(g, t = now()) {
    if (g.revoked_at != null || disk.isMarkedRevoked(dir, g.grant_id)) return "revoked";
    if (disabled()) return "disabled";
    if (g.expires_at <= t) return "expired";
    return "active";
  }

  // What each actor's connections come to, for the map and the Actors
  // panel (actors.setRemote).
  function sync(t = now()) {
    const grants = store.grants();
    const bySlug = new Map();
    for (const c of store.connections()) {
      const g = grants.find((x) => x.connection_id === c.connection_id);
      const list = bySlug.get(c.actor_slug) ?? [];
      list.push({ c, g });
      bySlug.set(c.actor_slug, list);
    }
    for (const id of actors.ids()) {
      const slug = id.slice("actor:".length);
      const list = bySlug.get(slug);
      if (!list) {
        if (actors.remoteOf(id)) actors.setRemote(id, null);
        continue;
      }
      const active = list.filter(({ g }) => g && stateOf(g, t) === "active");
      actors.setRemote(id, {
        active: active.length > 0,
        connections: list.length,
        bridge: active.some(({ c }) => bridges.get(c.connection_id) > 0) ? "connected" : "not_connected",
        accountLink: linkState(store, slug, t).state,
        permissions: [...new Set(active.flatMap(({ g }) => parse(g.permissions, [])))],
        transports: active.map(({ c }) => c.transport_kind),
      });
    }
  }

  // A grant's revision as it is now, on disk too, where an adapter with no
  // service to ask looks before it serves anything it read earlier.
  const noteRevision = (grantId) => {
    const g = store.grant(grantId);
    if (g) disk.markRevision(dir, grantId, g.revision);
  };

  function revokeRow(g, reason, t, by) {
    disk.markRevoked(dir, g.grant_id, t);
    disk.deleteCredential(dir, g.grant_id);
    const changed = store.transaction(() => {
      const r = store.revokeGrant(g.grant_id, reason, t);
      store.endConnection(g.connection_id, t);
      store.addAudit({ at: t, kind: by, grant_id: g.grant_id, grant_revision: g.revision, actor: `actor:${store.connection(g.connection_id)?.actor_slug ?? "?"}`, connection_id: g.connection_id, operation: "revoke", result: reason });
      return r;
    });
    noteRevision(g.grant_id);
    events.emit("revoked", { grant: g.grant_id, connection: g.connection_id, reason });
    return changed;
  }

  // A grant revoked while the service was stopped (xenonet assistant revoke),
  // or one a restored database has as active: the tombstone wins.
  function applyTombstones() {
    const t = now();
    for (const id of disk.markedRevoked(dir)) {
      const g = store.grant(id);
      if (g && g.revoked_at == null) revokeRow(g, "revoked-offline", t, "owner");
    }
  }

  // ---------- pairing ----------

  const remoteActors = () => actors.ids().filter((id) => actors.isRemote(id) && actors.remoteOf(id)?.active).length;
  const activeGrants = (t) => store.grants().filter((g) => stateOf(g, t) === "active").length;

  // The actor a pairing is for, without changing anything yet: one already
  // here, as you picked it (an observed Grok Bot, a Hermes profile, a dot
  // paired before), or a new one. A name a caller sends never picks one.
  function planActor(ask, adapter) {
    if (ask.actor != null) {
      const id = String(ask.actor);
      const info = actors.info(id);
      if (!info || id === YOU || info.via.includes("demo")) fail(404, "not-found", "XenoNet has no such actor to pair");
      const product = productOf(info.product?.id);
      if ((product?.id ?? null) !== adapter.product) fail(400, "bad-request", `${info.name} isn't ${adapter.product ? productOf(adapter.product).name : "an actor of your own"}`);
      return { id, create: null };
    }
    if (remoteActors() >= REMOTE_ACTORS_MAX) fail(409, "full", `${REMOTE_ACTORS_MAX} remote actors are paired already; revoke or remove one first`);
    const product = productOf(adapter.product);
    const isTaken = (s) => actors.has(`actor:${s}`) || taken(s);
    let slug;
    let name;
    if (product) {
      if (product.id === "dots") {
        slug = "dots";
        for (let i = 0; isTaken(slug); i++) {
          if (i > 50) fail(409, "taken", "XenoNet couldn't find a free slug for another dot");
          slug = `dots-${crypto.randomBytes(2).toString("hex")}`;
        }
      } else {
        slug = product.slug;
        if (isTaken(slug)) fail(409, "taken", `${product.name} is beside the map already, as actor:${slug}: pair that actor instead`);
      }
      const named = checkName(ask.name ?? product.short, product.id);
      if (!named.ok) fail(400, "bad-request", named.reason);
      name = named.name;
    } else {
      const named = checkName(ask.name, null);
      if (!named.ok) fail(400, "bad-request", named.reason);
      name = named.name;
      slug = ask.slug ?? slugFrom(name);
      const why = slugRefusal(slug, null);
      if (why) fail(400, "bad-request", why);
      if (isTaken(slug)) fail(409, "taken", `actor:${slug} is taken`, { suggest: suggestSlug(slug, { taken: isTaken }) });
    }
    return { id: `actor:${slug}`, create: { kind: "agent", name, product: product ? { id: product.id, vendor: product.vendor, version: null } : null } };
  }

  // A new connection and its grant (the window's Pair). `places`: the
  // places you may pick, by node id (what the map has now); `identities`:
  // the identities XenoNet knows, for a link that's your word. Your view of
  // it, with its launch command; the credential is only ever on disk.
  function pair(ask = {}, { places = new Map(), identities = new Set() } = {}) {
    const t = now();
    const adapter = adapterFor(ask.product ?? null, adapters);
    if (!adapter) fail(400, "bad-request", "that product can't be paired as a remote actor");
    if (!TRANSPORTS[ask.transport] || !adapter.transports.includes(ask.transport)) fail(400, "bad-request", `${adapter.product ? productOf(adapter.product).name : "An actor of your own"} reaches XenoNet through ${adapter.transports.map((x) => TRANSPORTS[x].name).join(" or ")}`);
    const label = words(ask.label, "a connection's name", 64) ?? fail(400, "bad-request", "a connection needs a name");
    const alias = words(ask.deviceAlias, "this computer's name", 64);
    const audience = {
      description: words(ask.audience?.description, "who can call it", 200) ?? fail(400, "bad-request", "say who can call this connection's endpoint"),
      verification: ["owner_verified", "unverified"].includes(ask.audience?.verification) ? ask.audience.verification : fail(400, "bad-request", "say whether you verified who can call it: owner_verified or unverified"),
      scope: TRANSPORT_SCOPES.includes(ask.audience?.scope) ? ask.audience.scope : fail(400, "bad-request", `its scope is ${TRANSPORT_SCOPES.join(", ")}`),
    };
    const days = ask.days ?? DEFAULT_GRANT_DAYS;
    if (!GRANT_DAYS.includes(days)) fail(400, "bad-request", `a grant lasts ${GRANT_DAYS.join(", ")} days`);
    const lookback = ask.lookbackDays ?? LOOKBACK_DAYS;
    if (!Number.isInteger(lookback) || lookback < 1 || lookback > LOOKBACK_DAYS) fail(400, "bad-request", `it reads back 1 to ${LOOKBACK_DAYS} days`);
    const chosen = [...new Set(ask.projects ?? [])];
    if (chosen.length > PROJECTS_MAX) fail(400, "bad-request", `a grant names at most ${PROJECTS_MAX} projects`);
    for (const p of chosen) if (!places.has(p)) fail(400, "bad-request", "pick projects from the map as it is now");
    const link = ask.accountLink?.state ?? "unverified";
    if (link === "verified") fail(400, "bad-request", "only its provider can verify an account; this connection's can't yet, so choose unverified or your own word");
    if (!["unverified", "owner_asserted"].includes(link)) fail(400, "bad-request", "an account link is unverified or owner_asserted");
    if (link === "owner_asserted" && !identities.has(ask.accountLink?.identity)) fail(400, "bad-request", "pick one of the identities XenoNet knows");
    if (activeGrants(t) >= CONNECTIONS_MAX) fail(409, "full", `${CONNECTIONS_MAX} connections are active already; revoke one first`);
    const plan = planActor(ask, adapter);

    const dev = device();
    const connection_id = newId("cn");
    const grant_id = newId("gr");
    const { token, hash } = disk.newToken();
    // The credential first: if it can't be kept private, nothing is made.
    disk.writeCredential(dir, { grant: grant_id, token, device: dev, expiresAt: t + days * DAY_MS }, credentialIo);
    try {
      store.transaction(() => {
        if (plan.create) actors.upsert({ id: plan.id, ...plan.create, via: ["remote"], heartbeatMs: null, capabilities: [], identity: null, at: t });
        else actors.addVia(plan.id, "remote");
        const slug = plan.id.slice("actor:".length);
        const projects = chosen.map((place) => store.projectIdOf(place, () => uuidId("prj"), t));
        store.addConnection({ connection_id, actor_slug: slug, adapter_id: adapter.id, provider: adapter.provider, transport_kind: ask.transport, transport_binding: null, label, device_alias: alias, audience_description: audience.description, audience_verification: audience.verification, audience_scope: audience.scope, created_at: t });
        store.addGrant({ grant_id, connection_id, device_id: dev, permissions: JSON.stringify(adapter.capabilities()), project_allowlist: JSON.stringify(projects), field_policy: JSON.stringify({ labels: ask.labels === true }), data_mode: audience.verification === "owner_verified" ? "real" : "fixture", lookback_ms: lookback * DAY_MS, token_hash: hash, created_at: t, expires_at: t + days * DAY_MS });
        if (link === "owner_asserted") assertLink(store, { slug, identity: ask.accountLink.identity, provider: adapter.provider, known: identities, now: t });
        store.addAudit({ at: t, kind: "owner", grant_id, grant_revision: 1, actor: plan.id, connection_id, operation: "pair", result: "ok", projects: projects.length });
      });
    } catch (err) {
      disk.deleteCredential(dir, grant_id);
      throw err;
    }
    noteRevision(grant_id);
    sync(t);
    events.emit("changed", { grant: grant_id });
    return view(store.grant(grant_id), t);
  }

  // ---------- changing one ----------

  function active(grantId, t = now()) {
    const g = GRANT_ID.test(String(grantId)) ? store.grant(grantId) : null;
    if (!g) fail(404, "not-found", "XenoNet has no such connection");
    if (stateOf(g, t) !== "active" && stateOf(g, t) !== "disabled") fail(409, "not-active", `that connection is ${stateOf(g, t)}`);
    return g;
  }

  // New projects or fields (the window): what it reads changes now, and
  // its revision moves on. Taking projects away needs nothing more; adding
  // them is a new approval, which is why this is the window's.
  function update(grantId, { projects, labels } = {}, { places = new Map() } = {}) {
    const t = now();
    const g = active(grantId, t);
    const change = {};
    if (projects !== undefined) {
      const chosen = [...new Set(projects)];
      if (chosen.length > PROJECTS_MAX) fail(400, "bad-request", `a grant names at most ${PROJECTS_MAX} projects`);
      // A place the grant names already may stay, on the map or not.
      const had = new Set(parse(g.project_allowlist, []));
      for (const p of chosen) if (!places.has(p) && !had.has(store.projectIdFor(p))) fail(400, "bad-request", "pick projects from the map as it is now");
      change.project_allowlist = JSON.stringify(chosen.map((place) => store.projectIdOf(place, () => uuidId("prj"), t)));
    }
    if (labels !== undefined) change.field_policy = JSON.stringify({ ...parse(g.field_policy, {}), labels: labels === true });
    if (!Object.keys(change).length) return view(g, t);
    store.transaction(() => {
      store.changeGrant(grantId, change);
      store.addAudit({ at: t, kind: "owner", grant_id: grantId, grant_revision: g.revision + 1, connection_id: g.connection_id, operation: "change", result: "ok", projects: change.project_allowlist ? parse(change.project_allowlist, []).length : null });
    });
    noteRevision(grantId);
    events.emit("changed", { grant: grantId });
    return view(store.grant(grantId), t);
  }

  // A new credential: the old one stops at once, and adapters holding it
  // are let go.
  function rotate(grantId) {
    const t = now();
    const g = active(grantId, t);
    const { token, hash } = disk.newToken();
    disk.writeCredential(dir, { grant: grantId, token, device: g.device_id, expiresAt: g.expires_at }, credentialIo);
    store.transaction(() => {
      store.changeGrant(grantId, { token_hash: hash });
      store.addAudit({ at: t, kind: "owner", grant_id: grantId, grant_revision: g.revision + 1, connection_id: g.connection_id, operation: "rotate", result: "ok" });
    });
    noteRevision(grantId);
    events.emit("rotated", { grant: grantId, connection: g.connection_id });
    return view(store.grant(grantId), t);
  }

  function revoke(grantId, { reason = "revoked" } = {}) {
    const g = GRANT_ID.test(String(grantId)) ? store.grant(grantId) : null;
    if (!g) fail(404, "not-found", "XenoNet has no such connection");
    const t = now();
    const changed = g.revoked_at == null ? revokeRow(g, reason, t, "owner") : false;
    sync(t);
    return { revoked: grantId, already: !changed };
  }

  // Every connection off, until you turn them on again; adapters are let
  // go at once.
  function disable() {
    const t = now();
    disk.markDisabled(dir, t);
    store.addAudit({ at: t, kind: "owner", operation: "disable", result: "ok" });
    sync(t);
    events.emit("disabled", {});
    return { disabled: true };
  }
  function enable() {
    const t = now();
    disk.clearDisabled(dir);
    store.addAudit({ at: t, kind: "owner", operation: "enable", result: "ok" });
    sync(t);
    events.emit("enabled", {});
    return { disabled: false };
  }

  // ---------- an adapter's calls ----------

  const no = (code, reason) => ({ ok: false, code, reason });

  // How a grant stands now, for a call through it: { ok, grant, connection,
  // actorId, revision } or { ok: false, code, reason }, where code is for
  // the caller and reason for the log alone.
  function standing(g, t = now()) {
    if (!g) return no("NOT_AUTHORIZED", "unknown-grant");
    if (disabled()) return no("DISABLED", "disabled");
    if (g.revoked_at != null || disk.isMarkedRevoked(dir, g.grant_id)) return no("GRANT_REVOKED", g.revoked_reason ?? "revoked");
    if (g.expires_at <= t) return no("GRANT_EXPIRED", "expired");
    if (g.device_id !== device()) return no("NOT_AUTHORIZED", "device-changed");
    const c = store.connection(g.connection_id);
    if (!c || c.state !== "active") return no("GRANT_REVOKED", "connection-ended");
    const actorId = `actor:${c.actor_slug}`;
    if (!actors.has(actorId)) return no("NOT_AUTHORIZED", "actor-gone");
    return { ok: true, grant: g, connection: c, actorId, revision: g.revision };
  }

  // An adapter saying which grant it's for, with the grant's credential.
  // A wrong credential is refused the same way as an unknown grant; one
  // that's right hears how its grant stands.
  function authenticate({ grant, token } = {}) {
    if (!GRANT_ID.test(String(grant)) || !disk.isToken(token)) return no("NOT_AUTHORIZED", "malformed");
    const g = store.grant(grant);
    if (!g) return no("NOT_AUTHORIZED", "unknown-grant");
    const theirs = Buffer.from(disk.hashToken(token), "hex");
    const ours = Buffer.from(g.token_hash, "hex");
    if (theirs.length !== ours.length || !crypto.timingSafeEqual(theirs, ours)) return no("NOT_AUTHORIZED", "wrong-token");
    return standing(g);
  }

  // The same, again, by grant and the revision a read began under, just
  // before its answer goes: changed is true when the revision moved on.
  function recheck(grantId, revision) {
    const s = standing(store.grant(grantId));
    return s.ok ? { ...s, changed: s.revision !== revision } : s;
  }

  // What a read through a grant may see (core/assistant/service.js).
  function policy(s) {
    const g = s.grant;
    const c = s.connection;
    const info = actors.info(s.actorId);
    const projects = parse(g.project_allowlist, []);
    const places = new Map();
    for (const p of projects) {
      const place = store.placeOfProject(p);
      if (place) places.set(p, place);
    }
    return {
      grantId: g.grant_id,
      revision: g.revision,
      actorId: s.actorId,
      connectionId: c.connection_id,
      deviceId: g.device_id,
      deviceAlias: c.device_alias ?? null,
      productId: productOf(info?.product?.id)?.id ?? null,
      accountLink: linkState(store, c.actor_slug).state,
      projects,
      places,
      labels: parse(g.field_policy, {}).labels === true,
      lookbackMs: g.lookback_ms,
      dataMode: g.data_mode === "real" ? "real" : "fixture",
      transportScope: c.audience_scope,
      audienceLabel: c.label,
      permissions: parse(g.permissions, []),
    };
  }

  // A read went out through a grant: when it was last used, written at
  // most once a minute.
  function used(grantId, t = now()) {
    if (t - (touched.get(grantId) ?? -Infinity) < TOUCH_MS) return;
    touched.set(grantId, t);
    store.touchGrant(grantId, t);
  }

  // An adapter connected through a connection, or went (`delta` +1 or -1).
  function bridge(connectionId, delta) {
    const n = Math.max(0, (bridges.get(connectionId) ?? 0) + delta);
    if (n) bridges.set(connectionId, n);
    else bridges.delete(connectionId);
    sync();
  }

  // ---------- your view ----------

  function view(g, t = now()) {
    const c = store.connection(g.connection_id);
    const link = linkState(store, c.actor_slug, t);
    return {
      schema: "actor/connection/v1",
      connection_id: c.connection_id,
      grant_id: g.grant_id,
      actor_id: `actor:${c.actor_slug}`,
      label: c.label,
      adapter_id: c.adapter_id,
      provider: c.provider,
      transport_kind: c.transport_kind,
      audience: { description: c.audience_description, verification: c.audience_verification, scope: c.audience_scope },
      account_link: { state: link.state, identity: link.identity },
      device: { id: g.device_id, alias: c.device_alias ?? null },
      permissions: parse(g.permissions, []),
      data_mode: g.data_mode === "real" ? "real" : "fixture",
      projects: parse(g.project_allowlist, []).map((p) => {
        const place = store.placeOfProject(p);
        return { project_id: p, place, name: place ? (describe(place) ?? null) : null };
      }),
      labels: parse(g.field_policy, {}).labels === true,
      lookback_days: Math.round(g.lookback_ms / DAY_MS),
      revision: g.revision,
      state: stateOf(g, t),
      bridge: bridges.get(c.connection_id) > 0 ? "connected" : "not_connected",
      created_at: iso(g.created_at),
      expires_at: iso(g.expires_at),
      revoked_at: iso(g.revoked_at),
      revoked_reason: g.revoked_reason ?? null,
      last_used_at: iso(g.last_used_at),
      launch: launchFor(g.grant_id, { port }),
    };
  }

  function list() {
    const t = now();
    return { at: t, disabled: disabled(), device: { id: device() }, connections: store.grants().map((g) => view(g, t)).reverse() };
  }

  // On the clock: grants that expired let their adapters go.
  const expired = new Set();
  function tick(t = now()) {
    for (const g of store.grants()) {
      if (g.revoked_at != null || g.expires_at > t || expired.has(g.grant_id)) continue;
      expired.add(g.grant_id);
      events.emit("expired", { grant: g.grant_id, connection: g.connection_id });
    }
    sync(t);
  }

  // Removing an actor (yours) revokes its connections first, while its
  // grants can still be named; the rows then go with the actor's.
  actors.beforeRemove.push((slug) => {
    const t = now();
    for (const c of store.connectionsOf(slug)) {
      const g = store.grants().find((x) => x.connection_id === c.connection_id);
      if (g && g.revoked_at == null) revokeRow(g, "actor-removed", t, "owner");
    }
  });

  device();
  applyTombstones();
  sync();

  return {
    events,
    on: (...args) => events.on(...args),
    device,
    idKey,
    disabled,
    pair,
    update,
    rotate,
    revoke,
    disable,
    enable,
    authenticate,
    recheck,
    policy,
    used,
    bridge,
    list,
    view: (grantId) => {
      const g = store.grant(grantId);
      return g ? view(g) : null;
    },
    tick,
    sync,
    // Whether any grant could be used now: the private endpoint listens
    // only then (server/assistant-endpoint.js).
    anyActive: () => !disabled() && store.grants().some((g) => stateOf(g) === "active"),
    audit: (opts) => store.audit(opts),
    dir,
  };
}
