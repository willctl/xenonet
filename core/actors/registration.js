// Registering actors (docs/design/actors/interface.md, section 2). Every
// registered actor exists because you allowed it:
//
//   - a program asks (register), and XenoNet answers at once with an id and a
//     token that works only once you click Allow in XenoNet's window (decide);
//     until then it opens only the actor's own GET /api/actors/me;
//   - Deny, or ten minutes without an answer, revokes the token, and the
//     same program asking again with the same name takes its old request's
//     place;
//   - adding an actor from the window (add) allows it at once, and a new
//     token (rotate) is made there only;
//   - removing an actor, or its leaving, revokes its token, releases its
//     claims and takes its record; a product's actor that XenoNet also
//     observes stays, as observed, and one XenoNet only observes can't be
//     removed (its observer's switch takes it off the map).
//
// Tokens (tokens.js) are kept as their SHA-256 only: on the actor's row
// once allowed, on the registration while it waits, and among the revoked
// for a week, so a program hears why its token stopped (rotated, denied,
// expired, left or removed). A token unused for 30 days is revoked, and its
// actor stays in the list until you give it a new one or remove it.
//
// Limits: 16 registered actors, 4 registrations waiting and 20 an hour. The
// observers of approved products are switched here too (observe), and the
// demo and test instances can make up a registration that waits
// (simulate), so the request card can be built against it.
import crypto from "node:crypto";
import os from "node:os";
import { EventEmitter } from "node:events";
import { YOU } from "../schema/ids.js";
import { tilde } from "../paths.js";
import { newToken, newGatewayToken, hashToken, isToken, isGatewayToken } from "./tokens.js";
import { checkName, slugFrom, slugRefusal, suggestSlug, productWordRefusal } from "./names.js";
import { productOf, WHY_NOT, OBSERVED } from "./products.js";
import { statusLine, clip } from "./detail.js";

export const REQUEST_MS = 10 * 60e3;
export const TOKEN_UNUSED_MS = 30 * 24 * 3600e3;
// Whether a token last used (or made) `at` has gone unused for 30 days by
// `t`: from that very millisecond, wherever it's asked (a call with it,
// the sweep, the window's view of the tokens).
export const tokenExpired = (at, t) => at == null || t - at >= TOKEN_UNUSED_MS;
export const REGISTERED_MAX = 16;
export const WAITING_MAX = 4;
export const REQUESTS_PER_HOUR = 20;
const HOUR_MS = 3600e3;
// When a token was last used is written at most this often, after its
// first use, which is written at once: a day's precision would do for 30
// days.
const TOUCH_MS = HOUR_MS;
const SWEEP_MS = 60e3;
const HEARTBEAT = { min: 5000, max: 3600e3, fallback: 60e3 };
export const CAPABILITIES = ["spawn-sessions", "edit-repos", "run-commands", "message-people", "message-agents", "schedule", "browse", "use-computer"];

// What a 401 revoked says, by its reason.
export const REVOKED = {
  rotated: "that token was replaced by a newer one",
  denied: "that registration was denied in XenoNet's window",
  expired: "that token expired: a registration lapses after ten minutes without an answer, and a token after 30 days without a call",
  left: "that actor left XenoNet",
  removed: "that actor was removed in XenoNet",
};

// An error a program can test: an HTTP status, a code from the interface's
// table, a sentence for a person, and extra fields (suggest, reason,
// retryAfterMs).
export class ActorError extends Error {
  constructor(status, code, message, extra = {}) {
    super(message);
    Object.assign(this, { status, code, extra });
  }
}
const fail = (status, code, message, extra) => {
  throw new ActorError(status, code, message, extra);
};

const parse = (s, fallback = null) => {
  try {
    return s == null ? fallback : JSON.parse(s);
  } catch {
    return fallback;
  }
};
const idOf = (slug) => `actor:${slug}`;

export function createRegistration({ actors, now = actors.now, home = os.homedir() }) {
  const store = actors.store;
  const events = new EventEmitter();
  let lastSweep = -Infinity;

  // ---------- what's asked ----------

  // A product's version or vendor, as the actor's own word: one line, no
  // key, 64 characters at most. It's shown quoted (“Acme Tools”), like a
  // name, so nothing in it may close the quote or pass for one.
  function wordOf(value, what) {
    if (value == null) return null;
    const checked = statusLine(value);
    if (!checked.ok) fail(400, "bad-request", `the product's ${what} looks like it holds a key or a token`);
    if (/["`\\\u201c\u201d]/.test(checked.status)) fail(400, "bad-request", `the product's ${what} can't hold quotation marks or backslashes, since it's shown quoted`);
    return clip(checked.status, 64);
  }

  // What a registration asks, checked as the interface's table has it: its
  // name, slug, product, capabilities (unknown ones listed back as ignored),
  // heartbeat and pid. A known product is held to its own rules: only one
  // that reports itself may register, it takes its own slug, and it joins
  // that product's observed actor. Any other product's id and vendor are
  // the actor's own word, and can't read as XenoNet's, a known app's or
  // product's, or a maker's (productWordRefusal). From the window, a pid
  // means nothing.
  function readAsk(body, { window = false } = {}) {
    const productId = body.product?.id ?? null;
    const known = productOf(productId);
    if (known && !known.registers) fail(403, "not-allowed", WHY_NOT[known.id]);
    const product = known ? { id: known.id, vendor: known.vendor, version: wordOf(body.product.version, "version") } : productId ? { id: productId, vendor: wordOf(body.product.vendor, "vendor"), version: wordOf(body.product.version, "version") } : null;
    if (product && !known) {
      const why = productWordRefusal(product.id, "id") ?? (product.vendor ? productWordRefusal(product.vendor, "vendor") : null);
      if (why) fail(400, "bad-request", why);
    }
    const named = checkName(body.name, known?.id ?? null);
    if (!named.ok) fail(400, "bad-request", named.reason);
    const slug = body.slug ?? (known ? known.slug : slugFrom(named.name));
    const why = slugRefusal(slug, known?.id ?? null);
    if (why) fail(400, "bad-request", why);
    const asked = [...new Set((body.capabilities ?? []).map(String))];
    const heartbeatMs = body.heartbeatMs ?? HEARTBEAT.fallback;
    if (heartbeatMs < HEARTBEAT.min || heartbeatMs > HEARTBEAT.max) fail(400, "bad-request", "heartbeatMs is 5,000 to 3,600,000 milliseconds");
    return {
      name: named.name,
      slug,
      product,
      capabilities: asked.filter((c) => CAPABILITIES.includes(c)),
      // Listed back to the program, and never shown on the card.
      ignored: asked.filter((c) => !CAPABILITIES.includes(c)).slice(0, 16).map((c) => clip(c.replace(/[^\x20-\x7e]/g, "?"), 40)),
      heartbeatMs,
      pid: window ? null : (body.pid ?? null),
    };
  }

  // Whether a registration that is `product` joins the actor already there
  // (`there`, as info has it) rather than finding its slug taken: only a
  // product XenoNet knows joins its own actor (Hermes's, which XenoNet
  // observes). Any other product id is the actor's own word, which anyone
  // can read and send, so it never makes another program the same actor.
  const joins = (there, product) => !!there && !!productOf(product) && there.product?.id === product;

  // Whether a slug is someone else's: an actor's (unless this joins it), or
  // a registration waiting, other than `except`.
  function takenBy(slug, { product = null, except = null, t = now() } = {}) {
    const there = actors.info(idOf(slug));
    if (there && !joins(there, product)) return true;
    return store.waitingActorRequests(t).some((r) => r.slug === slug && r.id !== except);
  }
  const taken = (slug, product) => fail(409, "taken", `actor:${slug} is taken${actors.isRegistered(idOf(slug)) ? "; to give it a new token, use New token in XenoNet's Actors panel" : ""}`, { suggest: suggestSlug(slug, { taken: (s) => takenBy(s), product }) });

  // Whether allowing this actor would take one more of the 16 places.
  const full = (id) => !actors.isRegistered(id) && actors.registeredCount() >= REGISTERED_MAX;
  const fullError = () => fail(409, "full", `${REGISTERED_MAX} actors are registered already; remove one in XenoNet's Actors panel first`);

  // What the card shows of who asked: the program, its parents up to the
  // first app or terminal, and its folder, with ~ for the home folder. Each
  // by its executable's real path where the check read it, since the name
  // ps gives (argv[0]) is whatever a process says it is.
  function askerOf(chain, cwd) {
    const file = (p) => p.exe ?? p.program;
    const parents = [];
    for (const p of chain.slice(1)) {
      parents.push(tilde(file(p), home));
      if (file(p).includes(".app/") || parents.length >= 8) break;
    }
    return { program: tilde(file(chain[0]), home), parents, folder: cwd ? tilde(cwd, home) : null };
  }

  // ---------- tokens ----------

  function revoke(hash, id, reason, t) {
    store.revokeActorToken({ token_hash: hash, actor: id, reason, at: t });
  }

  // A registration that waited too long: its token is revoked.
  function lapse(r, t) {
    if (store.decideActorRequest(r.id, "expired", t)) revoke(r.token_hash, idOf(r.slug), "expired", t);
  }
  function expireLapsed(t) {
    const lapsed = store.lapsedActorRequests(t);
    if (!lapsed.length) return;
    store.transaction(() => lapsed.forEach((r) => lapse(r, t)));
    changed(t);
  }

  // The registrations waiting changed: the window hears how many, and asks
  // for them with its key (GET /api/actors/requests).
  function changed(t = now()) {
    events.emit("requests", { at: t, waiting: store.waitingActorRequests(t).length });
  }

  // A registration, waiting from now: its credentials, and the request's id.
  function open(ask, asker, t) {
    return store.transaction(() => {
      const waiting = store.waitingActorRequests(t);
      // The same program asking again with the same name takes its old
      // request's place, and the old token stops working. The same program
      // is the same one with the same parents (the xenonet command run from
      // the same script, say); where XenoNet couldn't look, the same name.
      const programOf = (who) => (who ? JSON.stringify([who.program, who.parents]) : null);
      const same = waiting.find((r) => r.slug === ask.slug && parse(r.ask)?.name === ask.name && programOf(parse(r.asker)) === programOf(asker));
      if (takenBy(ask.slug, { product: ask.product?.id ?? null, except: same?.id, t })) taken(ask.slug, ask.product?.id ?? null);
      if (full(idOf(ask.slug))) fullError();
      if (waiting.filter((r) => r !== same).length >= WAITING_MAX) fail(409, "full", `${WAITING_MAX} registrations are waiting for an answer already; try again once they're answered or have lapsed`);
      // The hour before now, so a retry after Retry-After is taken.
      const hour = store.actorRequestTimes(t - HOUR_MS + 1);
      if (hour.length >= REQUESTS_PER_HOUR) fail(429, "too-many", `XenoNet takes at most ${REQUESTS_PER_HOUR} registrations an hour`, { retryAfterMs: hour[0] + HOUR_MS - t });
      if (same) {
        store.decideActorRequest(same.id, "replaced", t);
        revoke(same.token_hash, idOf(same.slug), "rotated", t);
      }
      const { token, hash } = newToken();
      const id = crypto.randomBytes(9).toString("base64url");
      store.addActorRequest({ id, slug: ask.slug, ask: JSON.stringify(ask), token_hash: hash, asker: asker ? JSON.stringify(asker) : null, created_at: t, expires_at: t + REQUEST_MS });
      return { id, credentials: { schema: "actor/credentials/v1", id: idOf(ask.slug), token, state: "waiting", expiresAt: t + REQUEST_MS, ignored: ask.ignored } };
    });
  }

  return {
    events,
    on: (...args) => events.on(...args),

    // A program asks to be an actor (POST /api/actors/register), with an
    // actor/registration/v1 body that fits its schema. `checked`, `chain`
    // and `cwd`: what the process check saw (verify.register), for the card.
    // The credentials, waiting.
    register(body, { checked = false, chain = null, cwd = null } = {}) {
      const t = now();
      const ask = readAsk(body);
      expireLapsed(t);
      const { id, credentials } = open(ask, checked && chain?.length ? askerOf(chain, cwd) : null, t);
      changed(t);
      events.emit("asked", { id, name: ask.name });
      return credentials;
    },

    // The registrations waiting for you, oldest first, for the window's
    // cards, as an actor/requests/v1: what each asks, and who asked (or
    // null where XenoNet couldn't look). `joins`: Allow would make it the
    // actor XenoNet already knows by that slug (a known product's, such as
    // the one it observes).
    requests() {
      const t = now();
      expireLapsed(t);
      return {
        schema: "actor/requests/v1",
        at: t,
        requests: store.waitingActorRequests(t).map((r) => {
          const ask = parse(r.ask, {});
          return { id: r.id, actor: idOf(r.slug), name: ask.name, product: ask.product ?? null, capabilities: ask.capabilities ?? [], heartbeatMs: ask.heartbeatMs ?? null, pid: ask.pid ?? null, asker: parse(r.asker), joins: joins(actors.info(idOf(r.slug)), ask.product?.id ?? null), at: r.created_at, expiresAt: r.expires_at };
        }),
      };
    },

    // Your answer to a registration, from the window: allow makes its token
    // work and the actor joins the list (unknown until its first
    // heartbeat); deny revokes it.
    decide(requestId, decision) {
      if (decision !== "allow" && decision !== "deny") fail(400, "bad-request", 'the decision is "allow" or "deny"');
      const t = now();
      expireLapsed(t);
      const out = store.transaction(() => {
        const r = store.actorRequest(String(requestId));
        if (!r || r.decision) fail(404, "not-found", "no registration with that id is waiting; it may have been answered or have lapsed");
        const id = idOf(r.slug);
        if (decision === "deny") {
          store.decideActorRequest(r.id, "deny", t);
          revoke(r.token_hash, id, "denied", t);
          return { request: r.id, decision, actor: id };
        }
        const ask = parse(r.ask, {});
        const there = actors.info(id);
        if (there && !joins(there, ask.product?.id ?? null)) taken(r.slug, ask.product?.id ?? null);
        if (full(id)) fullError();
        const old = store.actorToken(r.slug);
        if (old) revoke(old.token_hash, id, "rotated", t);
        actors.upsert({
          id,
          name: ask.name,
          // The product as XenoNet observes it, when it does.
          product: there?.product ?? ask.product ?? null,
          via: [...new Set([...(there?.via ?? []), "registered"])],
          heartbeatMs: ask.heartbeatMs,
          capabilities: ask.capabilities ?? [],
          identity: there?.identity ?? null,
          allowedAt: t,
          at: t,
        });
        store.setActorToken(r.slug, r.token_hash, t);
        store.decideActorRequest(r.id, "allow", t);
        if (ask.pid) actors.watchPid(id, ask.pid);
        return { request: r.id, decision, actor: actors.snapshot(id) };
      });
      changed(t);
      return out;
    },

    // An actor added from the window (POST /api/actors): allowed at once,
    // with its token, once.
    add(body) {
      const t = now();
      const ask = readAsk(body, { window: true });
      expireLapsed(t);
      return store.transaction(() => {
        const id = idOf(ask.slug);
        if (takenBy(ask.slug, { product: ask.product?.id ?? null, t })) taken(ask.slug, ask.product?.id ?? null);
        if (full(id)) fullError();
        const there = actors.info(id);
        const old = store.actorToken(ask.slug);
        if (old) revoke(old.token_hash, id, "rotated", t);
        actors.upsert({ id, name: ask.name, product: there?.product ?? ask.product, via: [...new Set([...(there?.via ?? []), "registered"])], heartbeatMs: ask.heartbeatMs, capabilities: ask.capabilities, identity: there?.identity ?? null, allowedAt: t, at: t });
        const { token, hash } = newToken();
        store.setActorToken(ask.slug, hash, t);
        return { schema: "actor/credentials/v1", id, token, state: "allowed", ignored: ask.ignored };
      });
    },

    // A new token for a registered actor, from the window: the old one stops
    // working at once, since rotating usually means it leaked.
    rotate(slug) {
      const id = idOf(slug);
      const there = actors.info(id);
      if (id === YOU || !there?.via.includes("registered")) fail(404, "not-found", "XenoNet has no registered actor by that slug");
      const t = now();
      return store.transaction(() => {
        const old = store.actorToken(slug);
        if (old) revoke(old.token_hash, id, "rotated", t);
        const { token, hash } = newToken();
        store.setActorToken(slug, hash, t);
        actors.allow(id, t);
        return { schema: "actor/credentials/v1", id, token, state: "allowed", ignored: [] };
      });
    },

    // An actor removed (yours, from the window or your terminal) or gone of
    // its own accord (`reason` left): its token revoked, its claims and its
    // line in the engine let go, and its record taken, unless it's a
    // product's actor XenoNet also observes, which stays as observed. One
    // XenoNet only observes isn't XenoNet's to remove, whoever asks: its
    // observer's switch takes it off the map (the decision at merge point
    // B), so it's a 409 that names the switch, and its product.
    remove(slug, { reason = "removed" } = {}) {
      const id = idOf(slug);
      if (id === YOU) fail(400, "bad-request", "you can't be removed; turn off Show me on the map in the Actors panel instead");
      if (!actors.has(id)) fail(404, "not-found", "XenoNet has no such actor");
      if (actors.onlyObserved(id)) {
        const a = actors.info(id);
        const product = productOf(a.product?.id);
        const how = product ? `the ${product.name} switch under observers in XenoNet's Actors panel, or xenonet actor observe ${product.id} off` : "its switch under observers in XenoNet's Actors panel";
        fail(409, "observed", `XenoNet only observes ${a.name}, so it can't be removed; turn off its observer instead, with ${how}`, product ? { product: product.id } : {});
      }
      const t = now();
      const registered = actors.isRegistered(id);
      store.transaction(() => {
        const old = store.actorToken(slug);
        if (old) revoke(old.token_hash, id, reason, t);
        store.dropActorToken(slug);
        const gate = store.gatewayToken(slug);
        if (gate) revoke(gate.gateway_hash, id, reason, t);
        store.dropGatewayToken(slug);
        store.release(id);
        store.setFocus(id, null);
        // Removed by you, its remote connections end with it; leaving on
        // its own, it keeps them.
        if (registered) actors.unregister(id, { keepRemote: reason !== "removed" });
        else actors.remove(id);
      });
      return { removed: id, ...(actors.has(id) ? { observed: true } : {}) };
    },

    // The actor a token belongs to: { ok, id, slug, state: "allowed" } once
    // you've allowed it, or "waiting" with its request; otherwise { ok:
    // false } with the status, code and sentence of a 401 (and the reason
    // for a revoked one).
    authenticate(token) {
      const unknown = { ok: false, status: 401, code: "bad-token", error: "XenoNet doesn't know that token" };
      if (!isToken(token)) return unknown;
      const hash = hashToken(token);
      const t = now();
      const held = store.actorByToken(hash);
      if (held && actors.has(idOf(held.slug))) {
        if (tokenExpired(held.token_at, t)) {
          store.transaction(() => {
            revoke(hash, idOf(held.slug), "expired", t);
            store.dropActorToken(held.slug);
          });
        } else {
          // Its first use is written exactly, so the window can tell a token
          // never used (one whose time is still when it was made, its
          // actor's allowedAt); after that, at most once an hour.
          const made = actors.info(idOf(held.slug))?.allowedAt ?? null;
          if ((made != null && held.token_at <= made) || t - held.token_at >= TOUCH_MS) store.touchActorToken(held.slug, t);
          return { ok: true, id: idOf(held.slug), slug: held.slug, state: "allowed" };
        }
      }
      const revoked = store.revokedActorToken(hash);
      if (revoked) return { ok: false, status: 401, code: "revoked", reason: revoked.reason, error: REVOKED[revoked.reason] ?? "that token was revoked" };
      const r = store.actorRequestByToken(hash);
      if (r && !r.decision) {
        if (r.expires_at > t) return { ok: true, id: idOf(r.slug), slug: r.slug, state: "waiting", request: r };
        lapse(r, t);
        changed(t);
        return { ok: false, status: 401, code: "revoked", reason: "expired", error: REVOKED.expired };
      }
      return unknown;
    },

    // ---------- the actor gateway (server/actor-gateway.js) ----------

    // A gateway token for an actor, from XenoNet's window only, shown once:
    // it works only at the gateway, from outside this machine, and the old
    // one stops working at once. A product's actor XenoNet only observes
    // (Muse, which runs nothing here, so can't register from under its
    // app) is made a registered actor first, as Add does, joining the one
    // XenoNet observes, with the capabilities asked for (message-agents unless
    // said). Any other actor must be registered already.
    gateway(slug, body = {}) {
      const id = idOf(slug);
      if (id === YOU) fail(400, "bad-request", "you reach XenoNet through its window, not the gateway");
      const t = now();
      expireLapsed(t);
      return store.transaction(() => {
        const there = actors.info(id);
        if (!there?.via.includes("registered")) {
          const product = productOf(there?.product?.id);
          if (!there || !product?.registers) fail(404, "not-found", "XenoNet has no registered actor by that slug; add or allow it first");
          if (full(id)) fullError();
          const ask = readAsk({ name: there.name, product: { id: product.id }, capabilities: Array.isArray(body.capabilities) ? body.capabilities : ["message-agents"], heartbeatMs: body.heartbeatMs ?? HEARTBEAT.max }, { window: true });
          actors.upsert({ id, name: ask.name, product: there.product, via: [...new Set([...there.via, "registered"])], heartbeatMs: ask.heartbeatMs, capabilities: ask.capabilities, identity: there.identity ?? null, allowedAt: t, at: t });
        }
        const old = store.gatewayToken(slug);
        if (old) revoke(old.gateway_hash, id, "rotated", t);
        const { token, hash } = newGatewayToken();
        store.setGatewayToken(slug, hash, t);
        changed(t);
        return { schema: "actor/gateway-credentials/v1", id, token };
      });
    },

    // An actor's gateway token taken away (from the window, or your own
    // terminal): it stops working at once; its own token, if any, stands.
    dropGateway(slug) {
      const id = idOf(slug);
      const t = now();
      return store.transaction(() => {
        const old = store.gatewayToken(slug);
        if (!old) fail(404, "not-found", "that actor has no gateway token");
        revoke(old.gateway_hash, id, "removed", t);
        store.dropGatewayToken(slug);
        return { actor: id, gateway: false };
      });
    },

    // The actor a gateway token belongs to, as authenticate has it for an
    // actor's own token: { ok, id, slug, state: "allowed" }, or a 401 (with
    // the actor, for a token revoked or expired in the last week). An
    // actor's own token is refused here, by name, so a program sent to the
    // gateway with the wrong one hears why.
    authenticateGateway(token) {
      const unknown = { ok: false, status: 401, code: "bad-token", error: "XenoNet doesn't know that gateway token" };
      if (isToken(token)) return { ok: false, status: 401, code: "wrong-token", error: "that's an actor's own token, which works only on this machine; the gateway takes a gateway token, made in XenoNet's window" };
      if (!isGatewayToken(token)) return unknown;
      const hash = hashToken(token);
      const t = now();
      const held = store.actorByGatewayToken(hash);
      if (held && actors.isRegistered(idOf(held.slug))) {
        if (tokenExpired(held.gateway_at, t)) {
          store.transaction(() => {
            revoke(hash, idOf(held.slug), "expired", t);
            store.dropGatewayToken(held.slug);
          });
        } else {
          if (t - held.gateway_at >= TOUCH_MS) store.touchGatewayToken(held.slug, t);
          return { ok: true, id: idOf(held.slug), slug: held.slug, state: "allowed" };
        }
      }
      const revoked = store.revokedActorToken(hash);
      if (revoked) return { ok: false, status: 401, code: "revoked", reason: revoked.reason, actor: revoked.actor, error: REVOKED[revoked.reason] ?? "that token was revoked" };
      return unknown;
    },

    // The product an authenticated actor is (its id), for the process check.
    productOf(auth) {
      if (auth.state === "waiting") return parse(auth.request.ask, {})?.product?.id ?? null;
      return actors.info(auth.id)?.product?.id ?? null;
    },

    // An actor as it sees itself (actor/me/v1): waiting or allowed, its
    // presence and heartbeat, and the engine messages waiting for it.
    me(auth, extra = {}) {
      if (auth.state === "waiting") return { schema: "actor/me/v1", id: auth.id, state: "waiting", presence: "unknown", heartbeatMs: parse(auth.request.ask, {})?.heartbeatMs ?? null, unread: 0, ...extra };
      const a = actors.info(auth.id);
      return { schema: "actor/me/v1", id: auth.id, state: "allowed", presence: a?.presence ?? "unknown", heartbeatMs: a?.heartbeatMs ?? null, unread: store.unseen(auth.id).length, ...extra };
    },

    // On the clock: registrations that lapsed, and once a minute, tokens
    // unused for 30 days (tokenExpired: those last used at or before then,
    // as times are whole milliseconds).
    tick(t = now()) {
      expireLapsed(t);
      if (t - lastSweep < SWEEP_MS) return;
      lastSweep = t;
      const stale = store.staleActorTokens(t - TOKEN_UNUSED_MS + 1);
      if (stale.length)
        store.transaction(() => {
          for (const row of stale) {
            revoke(row.token_hash, idOf(row.slug), "expired", t);
            store.dropActorToken(row.slug);
          }
        });
      const gates = store.staleGatewayTokens(t - TOKEN_UNUSED_MS + 1);
      if (gates.length)
        store.transaction(() => {
          for (const row of gates) {
            revoke(row.gateway_hash, idOf(row.slug), "expired", t);
            store.dropGatewayToken(row.slug);
          }
        });
    },

    // ---------- observers ----------

    // The products XenoNet observes, each with whether its observer is on and
    // the actors it knows of it.
    observers() {
      return { at: now(), observers: OBSERVED.map((p) => ({ product: p.id, name: p.name, on: this.observing(p.id), actors: actors.ofProduct(p.id) })) };
    },
    // Whether a product's observer is on: unless you turned it off.
    observing(product) {
      const ids = actors.ofProduct(product);
      return !ids.length || ids.every((id) => actors.enabled(id));
    },
    // Turns a product's observer on or off (yours): for each of its actors,
    // or, before XenoNet has seen it, for the actor it would be.
    observe(product, on) {
      const p = productOf(product);
      if (!p?.observed) fail(404, "not-found", "XenoNet has no observer for that product");
      if (typeof on !== "boolean") fail(400, "bad-request", "on is true or false");
      let ids = actors.ofProduct(product);
      if (!ids.length) {
        actors.upsert({ id: idOf(p.slug), name: p.short, product: { id: p.id, vendor: p.vendor, version: null }, via: ["observed"], enabled: on });
        ids = [idOf(p.slug)];
      } else for (const id of ids) actors.setEnabled(id, on);
      return { product, on, actors: ids };
    },

    // ---------- XenoNet Dev ----------

    // actor-request, in the demo and test instances only, whose data is made
    // up: a made-up program asks to register (named `a`, Heron by default),
    // so the request card can be built against it. What it made, with the
    // waiting token, or null outside the demo.
    simulate(kind, { a } = {}) {
      if (kind !== "actor-request" || !actors.demo) return null;
      const t = now();
      try {
        const named = checkName(typeof a === "string" && a.trim() ? a : "Heron");
        if (!named.ok) fail(400, "bad-request", named.reason);
        const base = slugFrom(named.name);
        const slug = !slugRefusal(base) && !takenBy(base, { t }) ? base : suggestSlug(base, { taken: (s) => takenBy(s, { t }) });
        const ask = { name: named.name, slug, product: null, capabilities: ["run-commands", "edit-repos", "message-agents"], ignored: [], heartbeatMs: 60e3, pid: null };
        const asker = { program: `~/bin/${slug}`, parents: ["/bin/zsh", "/System/Applications/Utilities/Terminal.app/Contents/MacOS/Terminal"], folder: `~/Projects/${slug}` };
        expireLapsed(t);
        const { id, credentials } = open(ask, asker, t);
        changed(t);
        return { ok: true, kind, a: credentials.id, request: id, token: credentials.token, expiresAt: credentials.expiresAt };
      } catch (err) {
        if (err instanceof ActorError) return { ok: false, kind, code: err.code, error: err.message };
        throw err;
      }
    },
  };
}
