// Waking an actor when a message arrives for it, so it needn't keep asking
// its inbox. Three ways, by what the actor can do:
//
//   listening  a program of its own holds a stream open (GET
//              /api/actors/me/stream, xenonet actor listen), and hears of a
//              message at once
//   webhook    XenoNet calls a URL the owner gave it, such as a Grok Bot
//              routine's, which starts the actor; it then reads its inbox
//   notify     XenoNet tells you, for an actor nothing can wake (Muse today),
//              once a message has waited two minutes, and at most hourly
//
// A wake says only that messages wait, and how many: never what they say,
// who sent them, or a token. The actor reads them with its own token, so a
// wake that reaches the wrong place gives nothing away.
//
// A webhook's URL and key are the owner's to set, from XenoNet's window
// (server/routes/actors.js), and are kept in the computer's credential store
// (core/secret-store.js), never in XenoNet's files, its logs or the page. Its
// request follows Standard Webhooks (https://www.standardwebhooks.com): a
// JSON body { type, timestamp, data }, webhook-id and webhook-timestamp
// headers, and a webhook-signature (HMAC-SHA256) when the key is a whsec_
// secret; any other key goes as Authorization: Bearer, which is what a Grok
// Bot routine asks for. It goes to https:// only (http:// on this computer
// alone), follows no redirect, and gives up after ten seconds.
//
// Messages in a burst make one wake. An actor is woken at most every 30
// seconds and 20 times an hour, and not again while the messages it was
// woken for wait unread, for two minutes, since any agent can write to it
// and each wake may start a run its owner pays for. If they're still unread
// then, it's woken once more. A webhook that fails
// waits longer each time before the next try, up to half an hour.
//
// The credential store keeps a webhook's URL and key off disk and out of
// XenoNet's own files, but programs running as you can read them from it, on
// a Mac too. So before each call the URL is checked again, and its host
// must be the one set in XenoNet's window.
import crypto from "node:crypto";
import dns from "node:dns";
import { SECRET } from "../secret-store.js";

export const WAKE_KINDS = ["webhook", "notify", "none"];
const META = "wakes";
export const COALESCE_MS = 1500;
export const MIN_GAP_MS = 30e3;
export const TIMEOUT_MS = 10e3;
export const BACKOFF_MS = [30e3, 120e3, 600e3, 1800e3];
export const NOTIFY_AFTER_MS = 120e3;
export const NOTIFY_EVERY_MS = 3600e3;
export const STREAMS_PER_ACTOR = 4;
// One actor's wakes: 20 an hour, 5 at once.
export const PER_ACTOR = { rate: 20 / 3600, burst: 5 };
// Woken, and the messages it was woken for still unread: it's at them, or
// can't be, so it isn't woken again for this long.
export const UNREAD_HOLD_MS = 2 * 60e3;
const TEST_GAP_MS = 5e3;
// All wakes together, whatever the actors: 120 an hour, 30 at once.
const ALL = { rate: 120 / 3600, burst: 30 };
const LOOPBACK = new Set(["127.0.0.1", "localhost", "[::1]"]);

export class WakeError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}
const fail = (status, code, message) => {
  throw new WakeError(status, code, message);
};

// The item in the credential store for an actor's webhook URL or key.
export const wakeTarget = (slug, part) => `XenoNet/actor wake/${slug}/${part}`;

// An address on the local link (169.254.0.0/16, fe80::/10, and IPv4's as
// IPv6 maps it), where a computer's metadata services answer.
export const linkLocal = (ip) => /^169\.254\./.test(ip) || /^fe[89ab][0-9a-f]:/i.test(ip) || /^::ffff:(169\.254\.|a9fe:)/i.test(ip);

// A webhook URL as XenoNet takes it: https://, or http:// on this computer;
// no user name or password in it (the key goes apart), not link-local, at
// most 2,048 characters. { url, host }, or a 400.
export function checkWakeUrl(raw) {
  const s = String(raw ?? "").trim();
  if (s.length > 2048) fail(400, "bad-request", "that URL is over 2,048 characters");
  let u;
  try {
    u = new URL(s);
  } catch {
    fail(400, "bad-request", "that isn't a URL");
  }
  if (u.username || u.password) fail(400, "bad-request", "a URL with a user name or password in it isn't taken; give the key on its own");
  if (u.protocol !== "https:" && !(u.protocol === "http:" && LOOPBACK.has(u.hostname))) fail(400, "bad-request", "a wake URL starts https://, or http:// for this computer alone");
  if (linkLocal(u.hostname.replace(/^\[|\]$/g, ""))) fail(400, "bad-request", "that address is link-local, which a wake never goes to");
  u.hash = "";
  return { url: u.toString(), host: u.host };
}

// Standard Webhooks' signature: v1, then the base64 HMAC-SHA256 of
// "<id>.<timestamp>.<body>" with the secret's bytes (a whsec_ secret is
// base64 after its prefix).
export function wakeSignature(secret, id, timestamp, body) {
  const key = secret.startsWith("whsec_") ? Buffer.from(secret.slice(6), "base64") : Buffer.from(secret, "utf8");
  return `v1,${crypto.createHmac("sha256", key).update(`${id}.${timestamp}.${body}`).digest("base64")}`;
}

// The request a wake makes: its body says which actor and how many messages
// wait, nothing more.
export function wakeRequest({ key, actor, unread, version = "0.0.0", now = Date.now(), id = `msg_${crypto.randomBytes(16).toString("hex")}` }) {
  const timestamp = Math.floor(now / 1000);
  const body = JSON.stringify({ type: "xenonet.inbox", timestamp: new Date(now).toISOString(), data: { actor, unread } });
  const headers = { "content-type": "application/json", "user-agent": `XenoNet/${version}`, "webhook-id": id, "webhook-timestamp": String(timestamp) };
  if (key.startsWith("whsec_")) headers["webhook-signature"] = wakeSignature(key, id, timestamp, body);
  else headers.authorization = `Bearer ${key}`;
  return { method: "POST", headers, body, redirect: "manual" };
}

// Why a failed call failed: `why` in words, with the host, for you; and
// `reason`, without it, for what anyone on this computer may read (the
// doctor). Never the URL's path or query, or the key.
function failed(err, host) {
  const code = err?.cause?.code ?? err?.code ?? err?.name;
  if (code === "TimeoutError" || code === "ABORT_ERR" || code === "UND_ERR_CONNECT_TIMEOUT") return { ok: false, why: `${host} didn't answer within ${TIMEOUT_MS / 1000} seconds`, reason: `no answer within ${TIMEOUT_MS / 1000} seconds` };
  return { ok: false, why: `XenoNet couldn't reach ${host}${code ? ` (${code})` : ""}`, reason: `couldn't connect${code ? ` (${code})` : ""}` };
}
const answered = (status, host) =>
  status >= 200 && status < 300
    ? { ok: true, status }
    : status >= 300 && status < 400
      ? { ok: false, status, why: `${host} answered with a redirect (HTTP ${status}), which XenoNet doesn't follow`, reason: `a redirect (HTTP ${status})` }
      : { ok: false, status, why: `${host} answered HTTP ${status}`, reason: `HTTP ${status}` };

// `secrets`: the credential store. `notify({ actor, name, unread })`: tells
// you messages wait for an actor nothing wakes. The rest stand in for tests.
export function createWakes({ engine, actors, store, secrets, notify = () => {}, version = "0.0.0", log = () => {}, fetch = globalThis.fetch, resolve = (host) => dns.promises.lookup(host, { all: true }), now = Date.now, setTimer = setTimeout, clearTimer = clearTimeout }) {
  const state = new Map(); // actor id → how its wakes went
  const streams = new Map(); // actor id → Set of { send, end }
  let allTokens = ALL.burst;
  let allAt = now();
  const slugOf = (id) => String(id).replace(/^actor:/, "");
  const st = (id) => state.get(id) ?? state.set(id, { timer: null, notifyTimer: null, inFlight: false, again: false, testing: false, lastAt: null, last: null, failures: 0, notBefore: 0, notifiedAt: 0, testedAt: 0, timerAt: 0, woke: null, rechecking: false, tokens: PER_ACTOR.burst, tokensAt: now() }).get(id);

  const configs = () => {
    try {
      return JSON.parse(store.meta(META) ?? "{}") ?? {};
    } catch {
      return {};
    }
  };
  const saveConfigs = (c) => store.setMeta(META, JSON.stringify(c));
  // How an actor is woken: what you set, else Notify me for one that
  // reaches XenoNet through the gateway, which nothing else wakes.
  function kindOf(slug) {
    const c = configs()[slug];
    if (c && WAKE_KINDS.includes(c.kind)) return c.kind;
    return store.gatewayToken?.(slug) ? "notify" : "none";
  }
  const unread = (id) => store.unseen(id).length;

  function takeAll() {
    const t = now();
    allTokens = Math.min(ALL.burst, allTokens + ((t - allAt) / 1000) * ALL.rate);
    allAt = t;
    if (allTokens < 1) return false;
    allTokens -= 1;
    return true;
  }

  // ---------- a message arrives ----------

  function arrived(id) {
    if (!String(id).startsWith("actor:") || !actors.isRegistered(id)) return;
    const n = unread(id);
    const live = streams.get(id);
    if (live?.size) {
      for (const sink of live) sink.send("inbox", { unread: n });
      return;
    }
    const kind = kindOf(slugOf(id));
    if (kind === "webhook") schedule(id);
    else if (kind === "notify") scheduleNotify(id);
  }

  // When its next wake may go: messages in a burst together, 30 seconds
  // after the last, after a failure's wait, and not while the messages it
  // was last woken for wait unread, for two minutes.
  function nextAt(id) {
    const s = st(id);
    const unseen = new Set(store.unseen(id).map((m) => m.id));
    const held = s.woke?.length && s.woke.every((m) => unseen.has(m)) ? (s.lastAt ?? 0) + UNREAD_HOLD_MS : 0;
    const refill = s.tokens >= 1 ? 0 : s.tokensAt + ((1 - s.tokens) / PER_ACTOR.rate) * 1000;
    return Math.max(now() + COALESCE_MS, (s.lastAt ?? 0) + MIN_GAP_MS, s.notBefore, held, refill);
  }

  // One timer an actor: a sooner wake (its messages read, and a new one
  // come) takes the place of a later one (the second look).
  function schedule(id) {
    const s = st(id);
    if (s.inFlight) return void (s.again = true);
    const at = nextAt(id);
    if (s.timer && s.timerAt <= at) return;
    if (s.timer) clearTimer(s.timer);
    s.timerAt = at;
    s.timer = setTimer(() => {
      s.timer = null;
      if (s.inFlight) return void (s.again = true);
      fire(id).catch((err) => log("actor.wake", `${id}: ${err.message}`));
    }, at - now());
    s.timer?.unref?.();
  }

  function scheduleNotify(id) {
    const s = st(id);
    if (s.notifyTimer) return;
    s.notifyTimer = setTimer(() => {
      s.notifyTimer = null;
      const n = unread(id);
      const t = now();
      if (!n || !actors.isRegistered(id) || t - s.notifiedAt < NOTIFY_EVERY_MS) return;
      s.notifiedAt = t;
      notify({ actor: id, name: actors.info(id)?.name ?? slugOf(id), unread: n });
    }, NOTIFY_AFTER_MS);
    s.notifyTimer?.unref?.();
  }

  // One of the actor's own wakes.
  function takeOne(s) {
    const t = now();
    s.tokens = Math.min(PER_ACTOR.burst, s.tokens + ((t - s.tokensAt) / 1000) * PER_ACTOR.rate);
    s.tokensAt = t;
    if (s.tokens < 1) return false;
    s.tokens -= 1;
    return true;
  }

  // Calls the webhook kept for `slug`, saying `n` messages wait: { ok,
  // status, why, reason }. Its URL is checked again first, as it's read back
  // from the store, and its host must be the one set in the window; an
  // address that turns out to be link-local is refused.
  async function call(id, slug, n, t) {
    const [url, key] = await Promise.all([secrets.get(wakeTarget(slug, "url")).catch(() => null), secrets.get(wakeTarget(slug, "key")).catch(() => null)]);
    const host = configs()[slug]?.host ?? "its webhook";
    if (!url || !key) return { ok: false, why: "its webhook isn't in your credential store any more; set its wake again", reason: "its webhook is missing from the credential store" };
    let checked;
    try {
      checked = checkWakeUrl(url);
    } catch {
      checked = null;
    }
    if (!checked || checked.host !== configs()[slug]?.host || !SECRET.test(key)) return { ok: false, why: "the webhook in your credential store isn't the one set in XenoNet's window; set its wake again", reason: "its webhook changed outside XenoNet" };
    const bare = new URL(checked.url).hostname.replace(/^\[|\]$/g, "");
    if (!/^[\d.]+$/.test(bare) && !bare.includes(":")) {
      const addrs = await Promise.resolve(resolve(bare)).catch(() => []);
      if ((addrs ?? []).some((a) => linkLocal(a.address))) return { ok: false, why: `${host} is a link-local address, which a wake never goes to`, reason: "a link-local address" };
    }
    try {
      const res = await fetch(checked.url, { ...wakeRequest({ key, actor: id, unread: n, version, now: t }), signal: AbortSignal.timeout(TIMEOUT_MS) });
      res.body?.cancel?.().catch(() => {});
      return answered(res.status, host);
    } catch (err) {
      return failed(err, host);
    }
  }

  // A wake for the messages waiting, when its time comes: one at a time.
  async function fire(id) {
    const slug = slugOf(id);
    const s = st(id);
    if (s.inFlight) return void (s.again = true);
    s.inFlight = true;
    s.again = false;
    let out = null;
    try {
      const unseen = store.unseen(id).map((m) => m.id);
      // The second look, two minutes after a wake whose messages all wait
      // unread: its last.
      const recheck = s.rechecking && !!s.woke?.length && s.woke.every((m) => unseen.includes(m));
      s.rechecking = false;
      if (!unseen.length || kindOf(slug) !== "webhook" || !actors.isRegistered(id)) return { ok: true, skipped: "nothing to wake it for" };
      // Its time may have moved since it was set: the actor's own limit, or
      // the messages it was last woken for still unread.
      if (nextAt(id) > now() + COALESCE_MS) return void (s.again = true);
      if (!takeOne(s)) return void (s.again = true);
      if (!takeAll()) return (out = record(id, now(), { ok: false, why: "XenoNet sends at most 120 wakes an hour", reason: "XenoNet's limit of 120 wakes an hour" }));
      const t = now();
      out = record(id, t, await call(id, slug, unseen.length, t));
      if (out.ok) {
        s.woke = unseen;
        // Still unread when the hold is up: woken once more, then not
        // again for these.
        if (!recheck) Object.assign(s, { rechecking: true, again: true });
      }
      return out;
    } finally {
      s.inFlight = false;
      if (s.again && unread(id)) schedule(id);
    }
  }

  // Your test: called now, even with nothing waiting. It doesn't hold the
  // next wake back, nor stop one on its way.
  async function fireTest(id) {
    const s = st(id);
    if (s.testing) fail(429, "too-many", "a test is on its way");
    s.testing = true;
    try {
      if (!takeAll()) return record(id, now(), { ok: false, why: "XenoNet sends at most 120 wakes an hour", reason: "XenoNet's limit of 120 wakes an hour" }, { test: true });
      const t = now();
      return record(id, t, await call(id, slugOf(id), unread(id), t), { test: true });
    } finally {
      s.testing = false;
    }
  }

  // How a wake went. Your test says so too, but neither holds the next
  // wake back nor counts as a failure to wait out.
  function record(id, t, out, { test = false } = {}) {
    const s = st(id);
    s.last = { ok: out.ok, at: t, status: out.status ?? null, why: out.why ?? null, reason: out.reason ?? null, test };
    if (test) return out;
    s.lastAt = t;
    if (out.ok) {
      s.failures = 0;
      s.notBefore = 0;
    } else {
      s.failures += 1;
      s.notBefore = t + BACKOFF_MS[Math.min(s.failures, BACKOFF_MS.length) - 1];
      log("actor.wake", `${id}: ${out.why}`);
    }
    return out;
  }

  const onMessage = ({ to }) => {
    for (const r of to ?? []) arrived(r);
  };
  engine.on("message", onMessage);

  // ---------- what's set ----------

  function view(id) {
    const slug = slugOf(id);
    const c = configs()[slug] ?? {};
    const s = state.get(id);
    const kind = kindOf(slug);
    return {
      actor: id,
      kind,
      set: !!configs()[slug],
      host: kind === "webhook" ? (c.host ?? null) : null,
      signed: kind === "webhook" ? !!c.signed : null,
      listening: streams.get(id)?.size ?? 0,
      lastAt: s?.last?.at ?? null,
      last: s?.last ?? null,
      failures: s?.failures ?? 0,
    };
  }

  function forgetState(id) {
    const s = state.get(id);
    if (!s) return;
    clearTimer(s.timer);
    clearTimer(s.notifyTimer);
    state.delete(id);
  }

  return {
    arrived,
    view,
    // Each actor that reports itself, and how it's woken.
    list() {
      const reporting = (store.actorTokenTimes?.() ?? []).filter((r) => r.hasToken || r.hasGateway).map((r) => `actor:${r.slug}`);
      const ids = new Set([...reporting, ...Object.keys(configs()).map((s) => `actor:${s}`), ...streams.keys(), ...state.keys()]);
      return [...ids].filter((id) => actors.isRegistered(id)).map(view);
    },
    // { kind, url, key }: a webhook takes both, kept in the credential store;
    // notify and none forget them.
    async set(slug, { kind, url, key } = {}) {
      const id = `actor:${slug}`;
      if (!actors.isRegistered(id)) fail(404, "not-found", "XenoNet has no actor by that slug that reports itself");
      if (!WAKE_KINDS.includes(kind)) fail(400, "bad-request", `a wake is ${WAKE_KINDS.join(", ")}`);
      const c = configs();
      if (kind === "webhook") {
        const checked = checkWakeUrl(url);
        if (!SECRET.test(checked.url)) fail(400, "bad-request", "that URL has a character XenoNet can't keep, such as a quote or a backslash");
        const k = String(key ?? "").trim();
        if (!SECRET.test(k)) fail(400, "bad-request", "a webhook's key is 8 to 1,024 printable characters, without spaces, quotes or backslashes");
        if (!secrets.available()) fail(501, "no-store", `XenoNet can't reach ${secrets.kind ?? "a credential store"} on this computer, so it can't keep a webhook's key`);
        await secrets.set(wakeTarget(slug, "url"), checked.url, { comment: "The webhook XenoNet calls to wake an actor" });
        await secrets.set(wakeTarget(slug, "key"), k, { comment: "The key XenoNet sends with an actor's wake" });
        c[slug] = { kind, host: checked.host, signed: k.startsWith("whsec_"), at: now() };
      } else {
        await Promise.all([secrets.remove(wakeTarget(slug, "url")).catch(() => false), secrets.remove(wakeTarget(slug, "key")).catch(() => false)]);
        c[slug] = { kind, at: now() };
      }
      saveConfigs(c);
      forgetState(id);
      return view(id);
    },
    // A wake now, for the owner to see the webhook work.
    async test(slug) {
      const id = `actor:${slug}`;
      if (!actors.isRegistered(id)) fail(404, "not-found", "XenoNet has no actor by that slug that reports itself");
      if (kindOf(slug) !== "webhook") fail(409, "no-webhook", "it has no webhook to call");
      const s = st(id);
      if (now() - s.testedAt < TEST_GAP_MS) fail(429, "too-many", "one test every five seconds");
      s.testedAt = now();
      return { ...(await fireTest(id)), wake: view(id) };
    },
    // The actor left or was removed: its wake and its streams go.
    async forget(slug) {
      const id = `actor:${slug}`;
      const c = configs();
      if (c[slug]) {
        delete c[slug];
        saveConfigs(c);
      }
      await Promise.all([secrets.remove(wakeTarget(slug, "url")).catch(() => false), secrets.remove(wakeTarget(slug, "key")).catch(() => false)]);
      forgetState(id);
      for (const sink of streams.get(id) ?? []) sink.end();
      streams.delete(id);
    },
    // A program of the actor's holding a stream open: `sink` is { send(event,
    // data), end() }. What it's told first is how many messages wait.
    subscribe(id, sink) {
      const set = streams.get(id) ?? streams.set(id, new Set()).get(id);
      if (set.size >= STREAMS_PER_ACTOR) fail(429, "too-many", `an actor holds at most ${STREAMS_PER_ACTOR} streams open`);
      set.add(sink);
      const n = unread(id);
      if (n) sink.send("inbox", { unread: n });
      return () => {
        set.delete(sink);
        if (!set.size) streams.delete(id);
      };
    },
    stop() {
      engine.off?.("message", onMessage);
      for (const id of [...state.keys()]) forgetState(id);
      for (const set of streams.values()) for (const sink of set) sink.end();
      streams.clear();
    },
  };
}
