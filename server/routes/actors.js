// Actors' HTTP API (core/actors; docs/design/actors/interface.md, section
// 2). Changes reach the console over the event stream too (`actors`,
// `actor-event`, and `actor-requests` when registrations wait for you).
//
// Anyone on this machine may read actors:
//
//   GET    /api/actors                the list, an actor/list/v1
//   GET    /api/actors/<slug>         one actor/snapshot/v1, or a 404
//   GET    /api/actors/events?since=  events since then, an actor/events/v1
//                                     (the last 7 days at most, oldest first)
//   GET    /api/actors/observers      the products XenoNet observes, and
//                                     whether each observer is on
//
// A program asks to be an actor, and then reports with its token
// (Authorization: Bearer, never in a URL), which works once you allow it:
//
//   POST   /api/actors/register       an actor/registration/v1; 202 with
//                                     actor/credentials/v1, waiting
//   POST   /api/actors/<slug>/gateway/ask
//                                     a session's (or your terminal's) ask
//                                     for the actor's gateway token; 202
//                                     with actor/gateway-ask/v1, which holds
//                                     no token: the ask waits for your Allow
//   GET    /api/actors/me             actor/me/v1: waiting or allowed, its
//                                     presence, its unread messages
//   POST   /api/actors/heartbeat      actor/report/v1: presence, status,
//                                     instance, cwd, pid
//   POST   /api/actors/events         actor/report/v1: events, as facts
//   POST   /api/actors/targets        actor/report/v1: targets, remove,
//                                     replace
//   DELETE /api/actors/me             it leaves, which revokes its token
//   GET    /api/actors/me/stream      text/event-stream: "inbox" with how
//                                     many messages wait, as each arrives
//                                     (core/actors/wake.js)
//
// XenoNet's own window, with its key (x-xenonet-window, see core/actors/key.js)
// and on macOS from XenoNet's own app with no agent nearer (verify.window):
//
//   GET    /api/actors/requests       the registrations waiting for you
//   POST   /api/actors/requests/<id>  {"decision": "allow" | "deny"}; for a
//                                     session's ask for a gateway token,
//                                     allow answers with that token, once
//                                     (credentials: actor/gateway-credentials/v1)
//   POST   /api/actors                adds an actor at once; 201 with its
//                                     token, once
//   POST   /api/actors/<slug>/token   a new token, once; the old one stops
//   POST   /api/actors/you            the window reports you
//                                     (actor/console/v1)
//   GET    /api/actors/you            your record: your actor/snapshot/v1,
//                                     or with ?since= your actor/events/v1
//   DELETE /api/actors/you            your record deleted at once, when you
//                                     turn off Show me on the map
//   GET    /api/actors/tokens         actor/tokens/v1: each registered
//                                     actor's token, whether it works, when
//                                     it expires, when it was last used;
//                                     never the token
//
// Yours, from the window or your own terminal (verify.person: not from
// under an agent or an actor product, and never with a token):
//
//   DELETE /api/actors/<slug>         removes an actor; 409 for one XenoNet
//                                     only observes, whose observer's
//                                     switch takes it off the map instead
//   POST   /api/actors/observers      {"product": "grok-bot", "on": false}
//   GET    /api/actors/gateway        the actor gateway (server/gateway-
//                                     control.js): on or off, its tunnel,
//                                     its address and whether it reaches
//                                     it, the tunnels this computer has,
//                                     and each actor's connection
//   POST   /api/actors/gateway        {"on": true, "tunnel": {"kind":
//                                     "ngrok", "host": "<name>"}}, any of
//                                     those, or {"check": true} or
//                                     {"repair": true}
//   DELETE /api/actors/<slug>/gateway its gateway token, taken away
//   GET    /api/actors/wakes          how each actor is woken when a
//                                     message arrives, and how its last
//                                     wake went; never a webhook's URL or key
//   DELETE /api/actors/<slug>/wake    it's woken no more
//   POST   /api/actors/<slug>/wake/test
//                                     its webhook, called now
//
// The window's alone: POST /api/actors/<slug>/gateway, a gateway token for
// the actor, once (actor/gateway-credentials/v1), registering a product's
// actor XenoNet only observes first, such as Muse; and PUT
// /api/actors/<slug>/wake, {"kind": "webhook", "url", "key"} or {"kind":
// "notify" | "none"}, whose URL and key go to the credential store.
//
// You are only the window's to see: everywhere else actor:you is a
// placeholder and your events are left out (the demo's made-up you
// excepted). On macOS a token is held to processes too (verify.actor).
// Errors are {"error": "<a sentence for a person>", "code": "<code>"},
// with a reason for a revoked token, a suggestion for a slug that's taken,
// and Retry-After over a rate. The server never logs an actor's body, its
// status or its token.
import { YOU, parseActorId } from "../../core/schema/ids.js";
import { check } from "../../core/schema/index.js";
import { ReportEvent, ReportTarget } from "../../core/schema/actor.js";
import { createBucket, createBuckets } from "../../core/actors/limits.js";
import { bearerOf, tokenInUrl } from "../../core/actors/tokens.js";
import { createRegistration, ActorError } from "../../core/actors/registration.js";
import { takeHeartbeat, takeEvents, takeTargets } from "../../core/actors/reports.js";
import { ownOf } from "../../core/engine/verify.js";
import { statusLine } from "../../core/actors/detail.js";
import { WakeError } from "../../core/actors/wake.js";

const CONSOLE_BODY = 8 << 10;
const REGISTER_BODY = 4 << 10;
const HEARTBEAT_BODY = 4 << 10;
const TARGETS_BODY = 16 << 10;
const EVENTS_BODY = 64 << 10;
const SMALL_BODY = 1 << 10;
// A webhook's URL (up to 2,048 characters) and its key (up to 1,024).
const WAKE_BODY = 4 << 10;
// A stream held open says it's still there this often, and checks its
// token still works.
const STREAM_BEAT_MS = 20e3;
const EVENTS_PER_CALL = 50;
// Rates, per the interface's table of limits. An actor names its instances
// itself, so they're held together too: eight a second at most, its eight
// instances' worth, however many names it makes up. A new status is an
// event on the feed, and counts as one.
export const RATES = {
  heartbeat: { rate: 1, burst: 2 }, // one a second per instance
  heartbeats: { rate: 8, burst: 16 }, // and eight a second per actor
  events: { rate: 1, burst: 120 }, // 60 a minute per actor, in bursts of up to 120
  targets: { rate: 0.5, burst: 10 }, // 30 a minute
  badTokens: { rate: 20 / 60, burst: 20 }, // from one process, then 429 for a minute
  messages: { rate: 60 / 3600, burst: 60 }, // through the engine, 60 an hour per actor
  wide: { rate: 6 / 3600, burst: 6 }, // of which 6 to repo or all
};
const BLOCK_MS = 60e3;
// A snapshot without its 24-hour counts, which a heartbeat doesn't need.
const NO_COUNTS = new Map();

const retryAfter = (ms) => ({ "retry-after": String(Math.max(1, Math.ceil((Number.isFinite(ms) ? ms : BLOCK_MS) / 1000))) });
const issue = (r) => `${r.issues[0]?.path || "the body"} ${r.issues[0]?.message ?? ""}`.trim();

// Who's calling with an actor's token, shared by these routes and the
// engine's (server/routes/engine.js): the token's actor, refused while it
// waits for you (unless `waiting`), and held to its processes on macOS. A
// process that sends 20 tokens XenoNet doesn't know in a minute is answered
// 429 for a minute after (good tokens still work). Failures are { ok:
// false, status, code, error, extra, headers }.
//
// The actors' rates and the bad tokens' are kept apart, so processes
// sending bad tokens can't crowd out an actor's buckets.
export function createActorCaller({ registration, verify = null, now = Date.now }) {
  const buckets = createBuckets({ now });
  const bad = createBuckets({ now });
  const blocked = new Map();

  const no = (status, code, error, extra = {}, headers = {}) => ({ ok: false, status, code, error, extra, headers });
  const unauthorized = (code, error, extra) => no(401, code, error, extra, { "www-authenticate": `Bearer realm="xenonet"${code === "no-token" ? "" : ', error="invalid_token"'}` });

  async function badToken(req) {
    const peer = (await verify?.peer?.(req.socket)) ?? "?";
    const t = now();
    if ((blocked.get(peer) ?? 0) > t) return no(429, "too-many", "too many tokens XenoNet doesn't know came from this process; wait a minute", {}, retryAfter(blocked.get(peer) - t));
    if (bad.take(`bad|${peer}`, RATES.badTokens)) {
      if (blocked.size > 256) blocked.clear();
      blocked.set(peer, t + BLOCK_MS);
      return no(429, "too-many", "too many tokens XenoNet doesn't know came from this process; wait a minute", {}, retryAfter(BLOCK_MS));
    }
    return null;
  }

  return {
    // The actor a request's token belongs to: { ok, auth }.
    async authenticate(req, { waiting = false } = {}) {
      const token = bearerOf(req.headers);
      if (token == null) return unauthorized("no-token", "this route takes an actor's token, as Authorization: Bearer <token>");
      if (req.headers["x-xenonet-agent"] !== undefined) return no(400, "bad-request", "a caller is a session or an actor, never both, so send x-xenonet-agent or a token, not both");
      const auth = registration.authenticate(token);
      if (!auth.ok) return (auth.code === "bad-token" && (await badToken(req))) || unauthorized(auth.code, auth.error, auth.reason ? { reason: auth.reason } : {});
      if (auth.state === "waiting" && !waiting) return no(403, "waiting", "this actor is waiting for you to allow it in XenoNet's window; try again later");
      return { ok: true, auth };
    },
    // Whether the request comes from where its token may be used, naming
    // `pid` or not: { ok, verdict }.
    async check(req, auth, { pid = null } = {}) {
      const verdict = verify?.actor ? await verify.actor(req.socket, { slug: auth.slug, product: registration.productOf(auth) }, { pid }) : { ok: true, checked: false };
      return verdict.ok ? { ok: true, verdict } : no(403, "wrong-process", verdict.reason);
    },
    // A rate (RATES) for `key`: 0, or the milliseconds until it allows n.
    take: (key, spec, n = 1) => buckets.take(key, spec, n),
    forget: (slug) => buckets.forget(`a:${slug}|`),
  };
}

export function actorRoutes({ actors, registration = null, caller = null, windowKey = null, verify = null, gateway = null, wakes = null, readJson, send, now = Date.now }) {
  registration ??= actors ? createRegistration({ actors, now }) : null;
  caller ??= registration ? createActorCaller({ registration, verify, now }) : null;
  const fail = (res, status, code, error, headers = {}, extra = {}) => send(res, status, { error, code, ...extra }, headers);
  const failWith = (res, r) => send(res, r.status, { error: r.error, code: r.code, ...r.extra }, r.headers);
  const notFound = (res) => fail(res, 404, "not-found", "XenoNet has no such route for actors");
  // The window reports at most once a second; this leaves room for bursts.
  const reports = createBucket({ rate: 2, burst: 10, now });

  // A time in epoch milliseconds from ?since=: null when there's none, and
  // undefined when it isn't one.
  const sinceOf = (url) => {
    const raw = url.searchParams.get("since");
    if (raw == null || raw === "") return null;
    const n = Number(raw);
    return Number.isFinite(n) ? n : undefined;
  };
  const limitOf = (url) => Number(url.searchParams.get("limit")) || 1000;
  // A body that must be an object: null, a list or a number is the
  // client's mistake, and a 400, rather than a TypeError further on.
  const objectOf = async (req, limit) => {
    const body = await readJson(req, limit);
    if (body === null || typeof body !== "object" || Array.isArray(body)) throw new ActorError(400, "bad-request", "the body is a JSON object");
    return body;
  };
  const slugOf = (raw) => {
    try {
      const id = `actor:${decodeURIComponent(raw)}`;
      return parseActorId(id)?.host === null ? id.slice("actor:".length) : null;
    } catch {
      return null;
    }
  };

  // Whether a request comes from XenoNet's own window: its key, and on macOS
  // its process. An actor's token never makes a request the window's.
  async function fromWindow(req) {
    if (bearerOf(req.headers) !== undefined) return "this is for XenoNet's own window, and an actor can't do it";
    if (!windowKey?.matches(req.headers["x-xenonet-window"])) return "this is for XenoNet's own window, and the request didn't come with its key";
    const checked = verify?.window ? await verify.window(req.socket) : { ok: true };
    return checked.ok ? null : checked.reason;
  }

  // Whether a request is yours: from the window, or else from your own
  // terminal, which on macOS isn't under an agent or an actor product. An
  // actor's token never makes a request yours.
  async function fromYou(req) {
    if (bearerOf(req.headers) !== undefined) return "this is yours to do, and an actor can't do it";
    if (req.headers["x-xenonet-window"] !== undefined) return fromWindow(req);
    const checked = verify?.person ? await verify.person(req.socket) : { ok: true };
    return checked.ok ? null : checked.reason;
  }

  async function you(req, res, url) {
    // Without a key it's any program asking about you, which gets what the
    // list says.
    if (req.method === "GET" && req.headers["x-xenonet-window"] === undefined) return send(res, 200, actors.snapshot(YOU));
    if (!["GET", "POST", "DELETE"].includes(req.method)) return notFound(res);
    const refused = await fromWindow(req);
    if (refused) return fail(res, 403, "yours", refused);
    // Show me on the map turned off: your record goes at once.
    if (req.method === "DELETE") {
      actors.you.forget();
      return send(res, 200, { deleted: YOU });
    }
    if (req.method === "GET") {
      const since = sinceOf(url);
      if (since === undefined) return fail(res, 400, "bad-request", "since is a time, in epoch milliseconds");
      return send(res, 200, since == null ? actors.snapshot(YOU, { view: "window" }) : actors.events({ since, only: YOU, view: "window", limit: limitOf(url) }));
    }
    const wait = reports.take();
    if (wait) return fail(res, 429, "too-many", "the window is reporting faster than XenoNet takes reports", retryAfter(wait));
    const body = await readJson(req, CONSOLE_BODY);
    const checked = check("actor/console/v1", body);
    if (!checked.ok) return fail(res, 400, "bad-request", `that isn't a report of you (actor/console/v1): ${issue(checked)}`);
    const { accepted, dropped } = actors.you.report(body);
    const me = actors.snapshot(YOU, { view: "window" });
    return send(res, 200, { schema: "actor/me/v1", id: YOU, state: "allowed", presence: me.presence, heartbeatMs: me.heartbeatMs, unread: 0, accepted, dropped });
  }

  // ---------- registering ----------

  async function register(req, res) {
    if (bearerOf(req.headers) !== undefined) return fail(res, 403, "not-allowed", "an actor can't register actors: that's for you, in XenoNet's window or your own terminal");
    const body = await readJson(req, REGISTER_BODY);
    const checked = check("actor/registration/v1", body);
    if (!checked.ok) return fail(res, 400, "bad-request", `that isn't a registration (actor/registration/v1): ${issue(checked)}`);
    const verdict = verify?.register ? await verify.register(req.socket, { product: body.product?.id ?? null, pid: body.pid ?? null }) : { ok: true, checked: false };
    if (!verdict.ok) return fail(res, 403, "wrong-process", verdict.reason);
    return send(res, 202, registration.register(body, verdict));
  }

  // ---------- an actor's own calls ----------

  // The token's actor, checked, or false once the answer has gone out.
  async function actorOf(req, res, { waiting = false } = {}) {
    const who = await caller.authenticate(req, { waiting });
    if (who.ok) return who.auth;
    failWith(res, who);
    return null;
  }
  async function checked(req, res, auth, opts) {
    const r = await caller.check(req, auth, opts);
    if (r.ok) return r.verdict;
    failWith(res, r);
    return null;
  }

  // The processes an actor counts as its own, for the sessions it says it
  // spawned: the ones it named, or else the caller and its parents below
  // the first app (the terminal it runs in). On macOS neither holds an
  // app's process, since the process check takes a named pid only below
  // that app too (ownOf in core/engine/verify.js); elsewhere the spawned
  // check doesn't run.
  function ownPids(id, verdict) {
    const named = actors.pidsOf(id);
    if (named.size) return named;
    const mine = new Set(ownOf(verdict?.chain ?? []).map((p) => p.pid));
    return mine.size ? mine : null;
  }

  async function me(req, res) {
    const auth = await actorOf(req, res, { waiting: true });
    if (!auth || !(await checked(req, res, auth))) return;
    return send(res, 200, registration.me(auth));
  }

  async function leave(req, res) {
    const auth = await actorOf(req, res);
    if (!auth || !(await checked(req, res, auth))) return;
    const out = registration.remove(auth.slug, { reason: "left" });
    caller.forget(auth.slug);
    await wakes?.forget(auth.slug);
    return send(res, 200, out);
  }

  // A report's body, checked against actor/report/v1, the parts this route
  // takes (`parts`) left out of the check to be taken a part at a time.
  async function reportOf(req, res, limit, parts = []) {
    const body = await objectOf(req, limit);
    const envelope = { ...body };
    for (const k of parts) if (k in envelope) envelope[k] = [];
    const r = check("actor/report/v1", envelope);
    if (r.ok) return body;
    fail(res, 400, "bad-request", `that isn't a report (actor/report/v1): ${issue(r)}`);
    return null;
  }

  // A heartbeat's rates, once the process check has passed, so a caller it
  // refuses never spends the actor's: the actor's eight a second, then its
  // instance's one, then an event's worth for a new status. { wait, why }
  // when it has to wait, or null.
  function heartbeatWait(auth, body) {
    const wait = caller.take(`a:${auth.slug}|hb`, RATES.heartbeats) || caller.take(`a:${auth.slug}|hb|${body.instance ?? "main"}`, RATES.heartbeat);
    if (wait) return { wait, why: "one heartbeat a second per instance, and eight a second per actor, is the most XenoNet takes" };
    const line = statusLine(body.status);
    if (!line.ok || !line.status || line.status === actors.snapshot(auth.id, { counts: NO_COUNTS })?.status) return null;
    const busy = caller.take(`a:${auth.slug}|events`, RATES.events);
    return busy ? { wait: busy, why: "a new status is an event on the feed, and 60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes" } : null;
  }

  async function heartbeat(req, res) {
    const auth = await actorOf(req, res);
    if (!auth) return;
    const body = await reportOf(req, res, HEARTBEAT_BODY);
    if (!body) return;
    const misplaced = ["events", "targets", "remove", "replace"].find((k) => k in body);
    if (misplaced) return fail(res, 400, "bad-request", `${misplaced} go to ${misplaced === "events" ? "POST /api/actors/events" : "POST /api/actors/targets"}, not to a heartbeat`);
    if (!(await checked(req, res, auth, { pid: body.pid ?? null }))) return;
    const limited = heartbeatWait(auth, body);
    if (limited) return fail(res, 429, "too-many", limited.why, retryAfter(limited.wait));
    takeHeartbeat(actors, auth.id, body);
    return send(res, 200, registration.me(auth));
  }

  async function events(req, res) {
    const auth = await actorOf(req, res);
    if (!auth) return;
    const body = await reportOf(req, res, EVENTS_BODY, ["events"]);
    if (!body) return;
    if (!Array.isArray(body.events)) return fail(res, 400, "bad-request", "events is a list of what it did");
    if (body.events.length > EVENTS_PER_CALL) return fail(res, 413, "too-big", `${EVENTS_PER_CALL} events a call at most`);
    const verdict = await checked(req, res, auth);
    if (!verdict) return;
    const wait = caller.take(`a:${auth.slug}|events`, RATES.events, Math.max(1, body.events.length));
    if (wait) return fail(res, 429, "too-many", "60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes", retryAfter(wait));
    // Each event checked on its own: one that doesn't fit is dropped, and
    // the others are taken.
    const invalid = [];
    const list = body.events.map((e, index) => {
      const r = ReportEvent.safeParse(e);
      if (!r.success) invalid.push({ index, reason: `that isn't an event: ${r.error.issues[0]?.path?.join(".") || "it"} ${r.error.issues[0]?.message ?? ""}`.trim() });
      return r.success ? r.data : null;
    });
    const taken = await takeEvents(actors, auth.id, list, { own: ownPids(auth.id, verdict), lineage: verify?.lineage ?? null, skip: new Set(invalid.map((d) => d.index)) });
    const dropped = [...invalid, ...taken.dropped].sort((a, b) => a.index - b.index);
    return send(res, 200, registration.me(auth, { accepted: taken.accepted, dropped }));
  }

  async function targets(req, res) {
    const auth = await actorOf(req, res);
    if (!auth) return;
    const body = await reportOf(req, res, TARGETS_BODY, ["targets", "remove"]);
    if (!body) return;
    for (const k of ["targets", "remove"]) if (body[k] !== undefined && !Array.isArray(body[k])) return fail(res, 400, "bad-request", `${k} is a list`);
    const verdict = await checked(req, res, auth);
    if (!verdict) return;
    const wait = caller.take(`a:${auth.slug}|targets`, RATES.targets);
    if (wait) return fail(res, 429, "too-many", "30 target calls a minute is the most XenoNet takes", retryAfter(wait));
    const invalid = [];
    const list = (body.targets ?? []).map((t, index) => {
      const r = ReportTarget.safeParse(t);
      if (!r.success) invalid.push({ index, reason: `that isn't a target: ${r.error.issues[0]?.path?.join(".") || "it"} ${r.error.issues[0]?.message ?? ""}`.trim() });
      return r.success ? r.data : null;
    });
    const remove = (body.remove ?? []).filter((t) => ReportTarget.safeParse(t).success);
    const taken = await takeTargets(actors, auth.id, { targets: list, remove, replace: body.replace === true }, { own: ownPids(auth.id, verdict), lineage: verify?.lineage ?? null, skip: new Set(invalid.map((d) => d.index)) });
    const dropped = [...invalid, ...taken.dropped].sort((a, b) => a.index - b.index);
    return send(res, 200, registration.me(auth, { accepted: taken.accepted, dropped }));
  }

  // ---------- the window's ----------

  async function windowOnly(req, res) {
    const refused = await fromWindow(req);
    if (refused) fail(res, 403, "yours", refused);
    return !refused;
  }

  async function add(req, res) {
    if (!(await windowOnly(req, res))) return;
    const body = await readJson(req, REGISTER_BODY);
    const r = check("actor/registration/v1", body);
    if (!r.ok) return fail(res, 400, "bad-request", `that isn't an actor to add (actor/registration/v1): ${issue(r)}`);
    return send(res, 201, registration.add(body));
  }

  async function decide(req, res, id) {
    if (!(await windowOnly(req, res))) return;
    const body = await objectOf(req, SMALL_BODY);
    let request = null;
    try {
      request = decodeURIComponent(id);
    } catch {}
    if (!request) return fail(res, 404, "not-found", "no registration with that id is waiting");
    const out = registration.decide(request, body.decision);
    // An allowed ask for a gateway token answers the window with the token,
    // once, and the gateway's address to go with it.
    if (out.credentials) out.credentials.url = gateway?.url() ?? null;
    return send(res, 200, out);
  }

  async function rotate(req, res, raw) {
    if (!(await windowOnly(req, res))) return;
    const slug = slugOf(raw);
    if (!slug) return fail(res, 404, "not-found", "XenoNet has no such actor");
    return send(res, 200, registration.rotate(slug));
  }

  // ---------- yours ----------

  async function remove(req, res, raw) {
    const refused = await fromYou(req);
    if (refused) return fail(res, 403, "yours", refused);
    const slug = slugOf(raw);
    if (!slug) return fail(res, 404, "not-found", "XenoNet has no such actor");
    // registration.remove refuses one XenoNet only observes (409 observed,
    // with its product), from the window or a program alike.
    const out = registration.remove(slug);
    caller.forget(slug);
    // One that stays, as XenoNet observes it, keeps no wake either.
    await wakes?.forget(slug);
    return send(res, 200, out);
  }

  // ---------- the actor gateway ----------

  async function gatewayState(req, res) {
    if (!gateway) return fail(res, 503, "off", "the actor gateway isn't here");
    const refused = await fromYou(req);
    if (refused) return fail(res, 403, "yours", refused);
    if (req.method === "GET") return send(res, 200, await gateway.status());
    const body = await objectOf(req, SMALL_BODY);
    const pick = body.tunnel === null ? null : body.tunnel && typeof body.tunnel === "object" ? { kind: body.tunnel.kind, name: body.tunnel.name, host: body.tunnel.host } : undefined;
    try {
      return send(res, 200, await gateway.set({ on: typeof body.on === "boolean" ? body.on : undefined, tunnel: pick, check: body.check === true, repair: body.repair === true }));
    } catch (err) {
      return fail(res, err.status ?? 400, err.code ?? "bad-request", err.message);
    }
  }

  async function gatewayToken(req, res, raw) {
    const slug = slugOf(raw);
    if (!slug) return fail(res, 404, "not-found", "XenoNet has no such actor");
    if (req.method === "DELETE") {
      const refused = await fromYou(req);
      if (refused) return fail(res, 403, "yours", refused);
      return send(res, 200, registration.dropGateway(slug));
    }
    if (!(await windowOnly(req, res))) return;
    const body = await objectOf(req, SMALL_BODY);
    const creds = registration.gateway(slug, { capabilities: Array.isArray(body.capabilities) ? body.capabilities : undefined });
    return send(res, 201, { ...creds, url: gateway?.url() ?? null });
  }

  // A session's, or your terminal's, ask for an actor's gateway token. Any
  // program may ask, as any may register, because the answer holds no
  // token: it says the ask waits for an Allow in XenoNet's window, where
  // the token is made and shown masked (src/secret.js). Never an actor's
  // token (an actor can't ask for its own way in), and never from under an
  // actor product's processes (verify.asker).
  async function gatewayAsk(req, res, raw) {
    if (bearerOf(req.headers) !== undefined) return fail(res, 403, "not-allowed", "an actor can't ask for gateway tokens: a session or you can, and the token is made in XenoNet's window");
    const slug = slugOf(raw);
    if (!slug) return fail(res, 404, "not-found", "XenoNet has no such actor");
    await objectOf(req, SMALL_BODY);
    const verdict = verify?.asker ? await verify.asker(req.socket) : { ok: true, checked: false };
    if (!verdict.ok) return fail(res, 403, "wrong-process", verdict.reason);
    return send(res, 202, registration.askGateway(slug, verdict));
  }

  // ---------- waking actors (core/actors/wake.js) ----------

  // How each actor is woken when a message arrives: yours to see, never a
  // webhook's URL or key.
  async function wakesState(req, res) {
    if (!wakes) return fail(res, 503, "off", "waking actors isn't here");
    const refused = await fromYou(req);
    if (refused) return fail(res, 403, "yours", refused);
    return send(res, 200, { at: now(), wakes: wakes.list() });
  }

  // PUT sets how an actor is woken, from the window alone, since a webhook's
  // URL and key are secrets; DELETE stops waking it, and POST …/test calls
  // its webhook now, yours from your terminal too.
  async function wake(req, res, raw, { test = false } = {}) {
    if (!wakes) return fail(res, 503, "off", "waking actors isn't here");
    const slug = slugOf(raw);
    if (!slug || slug === "you") return fail(res, 404, "not-found", "XenoNet has no such actor");
    if (test || req.method === "DELETE") {
      const refused = await fromYou(req);
      if (refused) return fail(res, 403, "yours", refused);
      return send(res, 200, test ? await wakes.test(slug) : await wakes.set(slug, { kind: "none" }));
    }
    if (!(await windowOnly(req, res))) return;
    const body = await objectOf(req, WAKE_BODY);
    return send(res, 200, await wakes.set(slug, { kind: body.kind, url: body.url, key: body.key }));
  }

  // An actor's own stream (text/event-stream): "inbox" with how many
  // messages wait, as each arrives, so a program of its own needn't keep
  // asking. Its token and process are checked as for its other calls, and
  // its token again every 20 seconds; the stream ends when that stops
  // working.
  async function stream(req, res) {
    if (!wakes) return fail(res, 503, "off", "waking actors isn't here");
    const auth = await actorOf(req, res);
    if (!auth || !(await checked(req, res, auth))) return;
    let open = true;
    const write = (text) => open && res.write(text);
    const sink = { send: (event, data) => write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`), end: () => (open && res.end(), (open = false)) };
    let off;
    try {
      res.writeHead(200, { "content-type": "text/event-stream; charset=utf-8", "cache-control": "no-store", "x-content-type-options": "nosniff" });
      write("retry: 5000\n\n");
      off = wakes.subscribe(auth.id, sink);
    } catch (err) {
      sink.send("closed", { code: err.code ?? "error", error: err.message });
      return sink.end();
    }
    const beat = setInterval(async () => {
      const who = await caller.authenticate(req).catch(() => ({ ok: false, code: "error" }));
      if (who.ok && who.auth.id === auth.id) return write(": still here\n\n");
      sink.send("closed", { code: who.code ?? "revoked", error: "its token stopped working" });
      sink.end();
    }, STREAM_BEAT_MS);
    beat.unref?.();
    req.on("close", () => {
      open = false;
      clearInterval(beat);
      off?.();
    });
  }

  async function observe(req, res) {
    const refused = await fromYou(req);
    if (refused) return fail(res, 403, "yours", refused);
    const body = await objectOf(req, SMALL_BODY);
    return send(res, 200, registration.observe(body.product, body.on));
  }

  return async function route(req, res, url) {
    if (!actors) return fail(res, 503, "off", "Actors are off here (XENONET_ACTORS=off)");
    if (tokenInUrl(url)) return fail(res, 400, "bad-request", "a token goes in the Authorization header, never in a URL");
    const p = url.pathname;
    const m = req.method;
    try {
      if (p === "/api/actors") return m === "GET" ? send(res, 200, actors.list()) : m === "POST" ? await add(req, res) : notFound(res);
      if (p === "/api/actors/events") {
        if (m === "POST") return await events(req, res);
        if (m !== "GET") return notFound(res);
        const since = sinceOf(url);
        if (since === undefined) return fail(res, 400, "bad-request", "since is a time, in epoch milliseconds");
        return send(res, 200, actors.events({ since: since ?? undefined, limit: limitOf(url) }));
      }
      if (p === "/api/actors/you") return await you(req, res, url);
      if (p === "/api/actors/register") return m === "POST" ? await register(req, res) : notFound(res);
      if (p === "/api/actors/me") return m === "GET" ? await me(req, res) : m === "DELETE" ? await leave(req, res) : notFound(res);
      if (p === "/api/actors/me/stream") return m === "GET" ? await stream(req, res) : notFound(res);
      if (p === "/api/actors/wakes") return m === "GET" ? await wakesState(req, res) : notFound(res);
      if (p === "/api/actors/heartbeat") return m === "POST" ? await heartbeat(req, res) : notFound(res);
      if (p === "/api/actors/targets") return m === "POST" ? await targets(req, res) : notFound(res);
      if (p === "/api/actors/requests") return m === "GET" ? ((await windowOnly(req, res)) ? send(res, 200, registration.requests()) : undefined) : notFound(res);
      if (p === "/api/actors/tokens" && m === "GET") {
        if (!(await windowOnly(req, res))) return;
        const t = now();
        return send(res, 200, { schema: "actor/tokens/v1", at: t, tokens: actors.tokens(t) });
      }
      if (p === "/api/actors/observers") return m === "GET" ? send(res, 200, registration.observers()) : m === "POST" ? await observe(req, res) : notFound(res);
      if (p === "/api/actors/gateway") return ["GET", "POST"].includes(m) ? await gatewayState(req, res) : notFound(res);
      const woke = /^\/api\/actors\/([^/]+)\/wake(\/test)?$/.exec(p);
      if (woke) return woke[2] ? (m === "POST" ? await wake(req, res, woke[1], { test: true }) : notFound(res)) : ["PUT", "DELETE"].includes(m) ? await wake(req, res, woke[1]) : notFound(res);
      const gate = /^\/api\/actors\/([^/]+)\/gateway$/.exec(p);
      if (gate) return ["POST", "DELETE"].includes(m) ? await gatewayToken(req, res, gate[1]) : notFound(res);
      const asked = /^\/api\/actors\/([^/]+)\/gateway\/ask$/.exec(p);
      if (asked) return m === "POST" ? await gatewayAsk(req, res, asked[1]) : notFound(res);
      const decision = /^\/api\/actors\/requests\/([^/]+)$/.exec(p);
      if (decision) return m === "POST" ? await decide(req, res, decision[1]) : notFound(res);
      const token = /^\/api\/actors\/([^/]+)\/token$/.exec(p);
      if (token) return m === "POST" ? await rotate(req, res, token[1]) : notFound(res);
      const one = /^\/api\/actors\/([^/]+)$/.exec(p);
      if (one && m === "GET") {
        const slug = slugOf(one[1]);
        const found = slug ? actors.snapshot(`actor:${slug}`) : null;
        return found ? send(res, 200, found) : fail(res, 404, "not-found", "XenoNet has no such actor");
      }
      if (one && m === "DELETE") return await remove(req, res, one[1]);
      return notFound(res);
    } catch (err) {
      if (err instanceof WakeError) return fail(res, err.status, err.code, err.message);
      if (err instanceof ActorError) {
        const { retryAfterMs, ...extra } = err.extra ?? {};
        return fail(res, err.status, err.code, err.message, retryAfterMs != null ? retryAfter(retryAfterMs) : {}, extra);
      }
      if (err.status === 413) return fail(res, 413, "too-big", "that body is over the size this route takes");
      if (err.status === 400) return fail(res, 400, "bad-request", "that body isn't JSON");
      throw err;
    }
  };
}
