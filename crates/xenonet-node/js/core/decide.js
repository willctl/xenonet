// core/decide.js in Rust (crates/xenonet-app/src/decide.rs): decisions
// handed to a decision model on OpenRouter. The Rust builds each request
// and reads each answer; the waiting (for the credential store and for
// OpenRouter) is done here, with the functions given.
import { native } from "../index.js";

const C = native.appDecideConstants();
export const DECISIONS_URL = C.url;
export const DEFAULT_MODEL = C.model;
export const KEY_TARGET = C.keyTarget;
export const KEY = new RegExp(C.key);

export class DecisionError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}

const fail = (e) => new DecisionError(e.status, e.code, e.message);
const text = (v) => (v == null ? null : String(v));

export function createDecider({ secrets, fetch = globalThis.fetch, log = null, now = Date.now, env = process.env, timeoutMs = C.timeoutMs }) {
  const { url, model } = native.appDecideSettings(text(env.XENONET_DECISIONS_URL), text(env.XENONET_DECISION_MODEL));
  let hasKey = null;
  const say = (line) => log?.[line.level](line.code, line.message);

  async function key() {
    const k = await secrets.get(KEY_TARGET).catch(() => null);
    hasKey = !!k;
    return k;
  }

  return {
    model,
    url,
    async status() {
      if (hasKey == null) hasKey = await secrets.has(KEY_TARGET).catch(() => false);
      return { model, key: hasKey ? "stored" : "missing", store: secrets.kind };
    },
    async storeKey(k) {
      const checked = native.appDecideCheckKey(String(k ?? ""));
      if (checked.error) throw fail(checked.error);
      await secrets.set(KEY_TARGET, checked.ok, { comment: C.keyComment });
      hasKey = true;
      return { key: "stored", store: secrets.kind };
    },
    async forgetKey() {
      await secrets.remove(KEY_TARGET).catch(() => false);
      hasKey = false;
      return { key: "missing" };
    },
    async decide({ kind, state, questions }) {
      const k = await key();
      if (!k) throw fail(native.appDecideNoKey());
      const started = now();
      const req = native.appDecideRequest(model, String(k), state, questions);
      let res;
      try {
        res = await fetch(url, { method: "POST", headers: req.headers, body: req.body, signal: AbortSignal.timeout(timeoutMs) });
      } catch (err) {
        const r = native.appDecideUnreachable(url, String(kind), err?.name === "TimeoutError", String(err?.message));
        say(r.log);
        throw fail(r.error);
      }
      const body = await res.json().catch(() => null);
      if (!res.ok) {
        const r = native.appDecideRefused(Number(res.status), body ?? null, String(kind));
        say(r.log);
        throw fail(r.error);
      }
      const r = native.appDecideAnswered(body ?? null, model, String(kind), Object.keys(questions).length, now() - started);
      say(r.log);
      return r.decision;
    },
  };
}
