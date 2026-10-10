// Registrations waiting for you (docs/design/actors/interface.md, section
// 2), and the window's own calls they're answered with. A program asks to
// be an actor and gets a token that works only once you click Allow here,
// in XenoNet's window: the request card in the attention strip (alerts.js)
// and the Actors panel (actors-panel.js) show what it asks and who asked,
// and answer it.
//
// The routes that see and answer requests, add actors, give them new
// tokens and say how each token stands take the key only this window holds
// (x-xenonet-window), which the desktop app hands to XenoNet's own page
// through the preload bridge. The key stays in memory here and never goes
// in a URL. A browser tab has no key: it hears only how many wait (the
// actor-requests event carries a count, never who asked), and says to
// answer them in XenoNet's window.
import { timeAgo } from "./format.js";

// Allow waits this long after a card first shows, so a click meant for
// what was there a moment before can't land on it.
export const ARM_MS = 1000;

// Calls to the window's routes (server/routes/actors.js), with the key.
// A 403 `yours` may mean the server started again with a new key, so the
// key is asked for again once. Each answers { ok, status, code, error,
// data }; without a key, status 0 and code no-key.
export function createWindowCalls({ desktop = null, request = (...a) => globalThis.fetch(...a) } = {}) {
  let key = null;
  const none = { ok: false, status: 0, code: "no-key", error: "Only XenoNet's own window can do that", data: {} };

  async function call(method, path, body) {
    if (!desktop?.windowKey) return none;
    for (let tries = 0; tries < 2; tries++) {
      key ??= (await desktop.windowKey().catch(() => null)) ?? null;
      if (!key) return none;
      let res;
      try {
        res = await request(path, { method, headers: { "x-xenonet-window": key, ...(body === undefined ? {} : { "content-type": "application/json" }) }, ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
      } catch {
        return { ok: false, status: 0, code: "unreachable", error: "XenoNet didn't answer", data: {} };
      }
      const data = await res.json().catch(() => ({}));
      if (res.status === 403 && data.code === "yours" && !tries) {
        key = null;
        continue;
      }
      return { ok: res.ok, status: res.status, code: data.code ?? null, error: data.error ?? null, data };
    }
  }

  return {
    call,
    // Whether this window holds a key: the desktop app gives one only to
    // XenoNet's own page, and only when it started the server.
    async ready() {
      if (!desktop?.windowKey) return false;
      key ??= (await desktop.windowKey().catch(() => null)) ?? null;
      return !!key;
    },
  };
}

// The registrations waiting, as the window sees them: refreshed at the
// start and whenever the event stream says the count changed. onChange
// gets view() after each change.
export function createActorRequests({ calls, onChange = () => {}, now = Date.now }) {
  let requests = [];
  // How many wait, from the event stream: all a window without a key knows.
  let waiting = 0;
  // Whether this window can see and answer them: null until it has asked.
  let canDecide = null;
  let off = false;
  let seq = 0;
  // When each request's card first showed, for Allow's wait (ARM_MS).
  const seen = new Map();

  const view = () => ({ requests, waiting: canDecide ? requests.length : waiting, canDecide: !!canDecide, off });
  const changed = () => onChange(view());

  async function refresh() {
    if (off) return view();
    const mine = ++seq;
    const r = await calls.call("GET", "/api/actors/requests");
    // A newer refresh is on its way; its answer wins.
    if (mine !== seq) return view();
    if (r.ok) {
      canDecide = true;
      requests = Array.isArray(r.data.requests) ? r.data.requests : [];
      const t = now();
      for (const q of requests) if (!seen.has(q.id)) seen.set(q.id, t);
      for (const id of seen.keys()) if (!requests.some((q) => q.id === id)) seen.delete(id);
    } else if (r.status === 503) {
      off = true;
      requests = [];
    } else if (r.code === "no-key" || r.status === 403) {
      canDecide = false;
      requests = [];
    }
    changed();
    return view();
  }

  return {
    refresh,
    view,
    // The actor-requests event: how many wait now.
    heard({ waiting: n } = {}) {
      waiting = Number.isInteger(n) && n >= 0 ? n : 0;
      if (canDecide === false) return changed();
      refresh();
    },
    // Actors are off here (XENONET_ACTORS=off): nothing waits, or ever will.
    setOff() {
      off = true;
      requests = [];
      waiting = 0;
      changed();
    },
    // Allow or deny one, then the list again. The call's answer.
    async decide(id, decision) {
      const r = await calls.call("POST", `/api/actors/requests/${encodeURIComponent(id)}`, { decision });
      await refresh();
      return r;
    },
    // Whether Allow is ready for a click: a moment after its card showed.
    armed: (id) => seen.has(id) && now() - seen.get(id) >= ARM_MS,
    armsIn: (id) => (seen.has(id) ? Math.max(0, seen.get(id) + ARM_MS - now()) : ARM_MS),
  };
}

// Each registered actor's token as this window sees it (GET
// /api/actors/tokens, with the key): whether it works, expired after 30
// days unused, or there's none (the demo's made-up actor); when it stops
// working unless it's used; and when it was last used. Never a token: the
// route gives none. The Actors panel reads it as it opens and when the
// actors that report themselves change; the driver reads it too.
export function createActorTokens({ calls }) {
  let tokens = [];
  let at = null;
  // Whether this window can see them: null until it has asked.
  let canSee = null;
  let off = false;
  let seq = 0;

  const view = () => ({ canSee: !!canSee, off, at, tokens });

  async function refresh() {
    if (off) return view();
    const mine = ++seq;
    const r = await calls.call("GET", "/api/actors/tokens");
    // A newer read is on its way; its answer wins.
    if (mine !== seq) return view();
    if (r.ok) {
      canSee = true;
      at = Number.isFinite(r.data.at) ? r.data.at : null;
      tokens = (Array.isArray(r.data.tokens) ? r.data.tokens : []).filter((t) => typeof t?.actor === "string");
    } else if (r.status === 503) {
      off = true;
      tokens = [];
    } else if (r.code === "no-key" || r.status === 403) {
      canSee = false;
      tokens = [];
    }
    return view();
  }

  return { refresh, view, of: (id) => tokens.find((t) => t.actor === id) ?? null };
}

const DAY_MS = 86400e3;
const TOKEN_STATE = { works: "token works", expired: "token expired, unused for 30 days", none: "no token" };

// How a token stands (one of createActorTokens'), in words: what it is,
// when it expires and when it was last used, each a part, as the panel's
// row shows them: "token works", "expires in 30 days", "last used 2h ago"
// (or "never used"). `used` is when it was last used, for a line that
// keeps its age up to date, or null.
export function tokenParts(t, now = Date.now()) {
  if (!t) return null;
  const state = TOKEN_STATE[t.state] ?? `token ${String(t.state ?? "unknown")}`;
  let expires = null;
  if (t.state === "works" && Number.isFinite(t.expiresAt)) {
    const left = t.expiresAt - now;
    const days = Math.ceil(left / DAY_MS);
    expires = left <= 0 ? "expiring" : left < DAY_MS ? "expires in under a day" : `expires in ${days} day${days === 1 ? "" : "s"}`;
  }
  const used = Number.isFinite(t.usedAt) ? t.usedAt : null;
  // A token that works and has no last use was never used; one that
  // expired, and was revoked, has nothing more to say.
  const usedWords = used != null ? `last used ${timeAgo(used, now)}` : t.state === "works" ? "never used" : null;
  return { state, expires, used, usedWords };
}

// The same, as one line: "token works · expires in 30 days · never used".
export function tokenWords(t, now = Date.now()) {
  const p = tokenParts(t, now);
  return p && [p.state, p.expires, p.usedWords].filter(Boolean).join(" · ");
}

// ---------- what a card says ----------

// The program that asked, where it ran from, and its folder (registration's
// asker, with ~ for the home folder): parents by name, an app by its
// bundle's. Null when XenoNet couldn't look (not macOS).
export function askedBy(asker) {
  if (!asker?.program) return null;
  const nameOf = (p) => /([^/]+)\.app(\/|$)/.exec(String(p))?.[1] ?? String(p).split("/").filter(Boolean).pop() ?? String(p);
  return { program: asker.program, from: (asker.parents ?? []).map(nameOf), parents: asker.parents ?? [], folder: asker.folder ?? null };
}

// What Allow does for a request that joins an actor XenoNet knows (`joins`:
// now only a product XenoNet knows, such as Hermes, joining the actor it
// observes): the program's token and what it asks to do take the place of
// that actor's own.
export const JOINS_ALLOW = "Allow replaces its token and capabilities with this program's";

// A number and its unit, kept on one line.
const unit = (n, u) => `${n}\u00a0${u}`;

// What a request asks for, in a few words: what it would do and how often
// it would check in.
export function asksFor(r, words) {
  const can = (r.capabilities ?? []).map((c) => words[c] ?? c);
  const beat = r.heartbeatMs ? `a heartbeat every ${r.heartbeatMs >= 120e3 ? unit(Math.round(r.heartbeatMs / 60e3), "min") : unit(Math.round(r.heartbeatMs / 1000), "s")}` : null;
  return [can.length ? `asks to ${can.join(", ")}` : "asks for nothing but to report", beat].filter(Boolean).join(" · ");
}

// How long until a request lapses.
export function leftWords(expiresAt, t = Date.now()) {
  const ms = expiresAt - t;
  if (!(ms > 0)) return "lapsing";
  if (ms < 60e3) return "under a minute left";
  return `${unit(Math.ceil(ms / 60e3), "min")} left`;
}
