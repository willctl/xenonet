// What your OpenRouter key has spent, for the Token use panel: today, this
// week and this month (UTC, as OpenRouter counts them), and the key's limit
// if it has one. It reads OpenRouter's GET /api/v1/key with the key XenoNet
// keeps for decision models (decide.js, in the computer's credential
// store), at most once a minute and only while a window asks. The key goes
// only to OpenRouter, in a header; the page gets the figures.
import { KEY_TARGET } from "./decide.js";

export const KEY_URL = "https://openrouter.ai/api/v1/key";
const EVERY_MS = 60e3;

const dollars = (v) => (Number.isFinite(Number(v)) && v !== null && v !== "" ? Number(v) : null);

// OpenRouter's answer as the panel uses it.
export function usageOf(body) {
  const d = body?.data ?? {};
  return {
    today: dollars(d.usage_daily),
    week: dollars(d.usage_weekly),
    month: dollars(d.usage_monthly),
    total: dollars(d.usage),
    limit: dollars(d.limit),
    remaining: dollars(d.limit_remaining),
    reset: typeof d.limit_reset === "string" ? d.limit_reset : null,
    freeTier: d.is_free_tier === true,
  };
}

export function createOpenRouterUsage({ secrets, fetch = globalThis.fetch, now = Date.now, log = null }) {
  let last = null;
  let at = 0;
  let pending = null;

  async function read() {
    const key = await secrets.get(KEY_TARGET).catch(() => null);
    if (!key) return { key: "missing" };
    let res;
    try {
      res = await fetch(KEY_URL, { headers: { authorization: `Bearer ${key}`, accept: "application/json" }, signal: AbortSignal.timeout(10000) });
    } catch (err) {
      log?.warn?.("openrouter.usage", `Couldn't reach OpenRouter for the key's usage: ${err.message}`);
      return { key: "stored", error: "OpenRouter didn't answer" };
    }
    if (res.status === 401 || res.status === 403) return { key: "rejected", error: "OpenRouter didn't take the key" };
    if (!res.ok) return { key: "stored", error: `OpenRouter answered ${res.status}` };
    return { key: "stored", at: now(), ...usageOf(await res.json().catch(() => null)) };
  }

  return {
    async status() {
      if (last && now() - at < EVERY_MS) return last;
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

// The demo's OpenRouter key, made up.
export function createDemoOpenRouterUsage({ now = Date.now } = {}) {
  return {
    async status() {
      return { key: "stored", at: now(), today: 0.84, week: 6.12, month: 21.4, total: 63.75, limit: 50, remaining: 28.6, reset: "monthly", freeTier: false };
    },
    forget() {},
  };
}
