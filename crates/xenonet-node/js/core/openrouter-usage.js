// core/openrouter-usage.js in Rust (crates/xenonet-sources/src/openrouter_usage.rs):
// what your OpenRouter key has spent. The key (from the credential store)
// and the request stay here, in JS; Rust says what OpenRouter's answer
// means and how long a reading stands.
import { KEY_TARGET } from "../../../../core/decide.js";
import { native } from "../index.js";

export const KEY_URL = native.openrouterKeyUrl();

export const usageOf = (body) => native.openrouterUsageOf(body);

export function createOpenRouterUsage({ secrets, fetch = globalThis.fetch, now = Date.now, log = null }) {
  let last = null;
  let at = 0;
  let pending = null;

  async function read() {
    const key = await secrets.get(KEY_TARGET).catch(() => null);
    if (!key) return native.openrouterReading(false, false, null, false, null, 0);
    let res;
    try {
      res = await fetch(KEY_URL, { headers: { authorization: `Bearer ${key}`, accept: "application/json" }, signal: AbortSignal.timeout(10000) });
    } catch (err) {
      log?.warn?.("openrouter.usage", `Couldn't reach OpenRouter for the key's usage: ${err.message}`);
      return native.openrouterReading(true, false, null, false, null, 0);
    }
    const refused = res.status === 401 || res.status === 403;
    const body = refused || !res.ok ? null : await res.json().catch(() => null);
    return native.openrouterReading(true, true, res.status, !!res.ok, body, refused || !res.ok ? 0 : now());
  }

  return {
    async status() {
      if (last && native.openrouterFresh(at, now())) return last;
      pending ??= read().then((r) => {
        last = r;
        at = now();
        pending = null;
        return r;
      });
      return pending;
    },
    // A new key, or none: read again on the next ask.
    forget() {
      last = null;
      at = 0;
    },
  };
}

export function createDemoOpenRouterUsage({ now = Date.now } = {}) {
  return {
    async status() {
      return native.openrouterDemo(now());
    },
    forget() {},
  };
}
