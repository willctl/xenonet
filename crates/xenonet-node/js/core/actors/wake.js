// core/actors/wake.js in Rust (crates/xenonet-actors/src/wake.rs): waking an
// actor when a message arrives for it. When a wake may go, what it says,
// the URL and host checks and every limit are Rust's; the timers, a
// stream's sinks, the credential store, DNS, fetch and WHATWG URL parsing
// are the twin's, called as the JS called them. The twin keeps the
// WakeError class programs test for.
import dns from "node:dns";
import { native } from "../../index.js";
import { drive, calls } from "../../drive.js";
import { STORE } from "../engine/store.js";

const C = native.wakeConstants();
export const WAKE_KINDS = C.kinds;
export const COALESCE_MS = C.coalesceMs;
export const MIN_GAP_MS = C.minGapMs;
export const TIMEOUT_MS = C.timeoutMs;
export const BACKOFF_MS = C.backoffMs;
export const NOTIFY_AFTER_MS = C.notifyAfterMs;
export const NOTIFY_EVERY_MS = C.notifyEveryMs;
export const STREAMS_PER_ACTOR = C.streamsPerActor;
export const PER_ACTOR = C.perActor;
export const UNREAD_HOLD_MS = C.unreadHoldMs;

export class WakeError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}

const run = (fn) => {
  try {
    return fn();
  } catch (err) {
    if (err?.xenonetWake) throw new WakeError(err.status, err.code, err.message);
    throw err;
  }
};
const unwrap = (out) => {
  if (out?.refused) throw new WakeError(out.refused.status, out.refused.code, out.refused.message);
  return out?.done;
};

export const wakeTarget = (slug, part) => native.wakeTarget(slug, part);
export const linkLocal = (ip) => native.wakeLinkLocal(ip);

// A URL as WHATWG's parser reads it, its hash taken off; null for what
// isn't one.
function parseUrl(s) {
  let u;
  try {
    u = new URL(s);
  } catch {
    return null;
  }
  u.hash = "";
  return { username: u.username, password: u.password, protocol: u.protocol, hostname: u.hostname, host: u.host, href: u.toString() };
}

export const checkWakeUrl = (raw) => run(() => native.wakeCheckUrl({ parseUrl }, raw));

export const wakeSignature = (secret, id, timestamp, body) => native.wakeSignature(secret, `${id}`, `${timestamp}`, `${body}`);

export const wakeRequest = ({ key, actor, unread, version = "0.0.0", now = Date.now(), id }) => native.wakeRequest(key, actor, unread, version, now, id === undefined ? undefined : `${id}`);

export function createWakes({ engine, actors, store, secrets, notify = () => {}, version = "0.0.0", log = () => {}, fetch = globalThis.fetch, resolve = (host) => dns.promises.lookup(host, { all: true }), now = Date.now, setTimer = setTimeout, clearTimer = clearTimeout }) {
  const core = new native.WakesCore(store[STORE], version, now());
  // The timers set for each actor, and the sinks of the streams held open.
  const timers = new Map();
  const sinks = new Map();
  let sinkNo = 0;
  const fire = (id) => drive(core.fire(id), calls(deps)).then(unwrap);
  const deps = {
    actors,
    secrets,
    now: () => now(),
    log: (code, detail) => log(code, detail),
    notify: (n) => notify(n),
    parseUrl,
    resolve: (host) => resolve(host),
    // A call as the JS made it: no redirect followed, ten seconds at most,
    // and the body let go. What it threw, as its parts.
    async fetch(url, init) {
      try {
        const res = await fetch(url, { ...init, signal: AbortSignal.timeout(TIMEOUT_MS) });
        res.body?.cancel?.().catch(() => {});
        return { status: res.status };
      } catch (err) {
        return { failed: { cause: err?.cause?.code, code: err?.code, name: err?.name } };
      }
    },
    setTimer(kind, id, ms) {
      const held = timers.get(id) ?? timers.set(id, { wake: null, notify: null }).get(id);
      const due =
        kind === "wake"
          ? () => {
              held.wake = null;
              if (core.wakeDue(id)) fire(id).catch((err) => log("actor.wake", `${id}: ${err.message}`));
            }
          : () => {
              held.notify = null;
              run(() => core.notifyDue(deps, id));
            };
      // A sooner wake takes the place of a later one.
      if (held[kind]) clearTimer(held[kind]);
      held[kind] = setTimer(due, ms);
      held[kind]?.unref?.();
      return !!held[kind];
    },
    clearTimers(id) {
      const held = timers.get(id);
      clearTimer(held?.wake ?? null);
      clearTimer(held?.notify ?? null);
      timers.delete(id);
    },
    send: (no, event, data) => sinks.get(no)?.send(event, data),
    end: (no) => sinks.get(no)?.end(),
  };

  const api = {
    arrived: (id) => run(() => core.arrived(deps, id)),
    view: (id) => run(() => core.view(id)),
    // Each actor that reports itself, and how it's woken.
    list: () => run(() => core.list(deps)),
    // { kind, url, key }: a webhook takes both, kept in the credential
    // store; notify and none forget them.
    async set(slug, { kind, url, key } = {}) {
      return unwrap(await drive(core.set(slug, { kind, url, key }), calls(deps)));
    },
    // A wake now, for the owner to see the webhook work.
    async test(slug) {
      return unwrap(await drive(core.test(slug), calls(deps)));
    },
    // The actor left or was removed: its wake and its streams go.
    async forget(slug) {
      unwrap(await drive(core.forget(slug), calls(deps)));
    },
    // A program of the actor's holding a stream open: `sink` is { send(event,
    // data), end() }. What it's told first is how many messages wait.
    subscribe(id, sink) {
      const no = ++sinkNo;
      sinks.set(no, sink);
      let set;
      try {
        set = run(() => core.subscribe(deps, id, no));
      } catch (err) {
        if (err instanceof WakeError) sinks.delete(no);
        throw err;
      }
      return () => {
        core.unsubscribe(id, set, no);
        sinks.delete(no);
      };
    },
    stop() {
      engine.off?.("message", onMessage);
      run(() => core.stop(deps));
    },
  };
  const onMessage = ({ to }) => {
    for (const r of to ?? []) api.arrived(r);
  };
  engine.on("message", onMessage);
  return api;
}
