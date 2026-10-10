// Decisions XenoNet hands to a decision model (docs/guide/decisions.md): a
// model that answers typed questions (yes or no, one of a few options, a
// place on a scale) with how sure it is, rather than writing text. Jev,
// TypeSafe's model on OpenRouter, is the default: it's quick and cheap
// for questions like "is this folder a project, a group of projects or
// neither?", and says how sure it is, so XenoNet keeps only the answers it's
// sure enough of and its own rules decide the rest.
//
// Nothing goes unless you ask, and you see what goes first: each kind of
// decision builds exactly what's sent from shapes (paths inside the
// projects folder, counts, file types), never transcripts. The key is
// OpenRouter's, which you give XenoNet once; it's kept in your computer's
// credential store (core/secret-store.js) and used only for this. Each
// decision goes in XenoNet's log with its model, time and cost, never what
// was asked.
//
//   decide({ kind, state, questions }) → { answers, model, provider, cost, ms }
//
// questions are Jev's: { <key>: { type: choice|noul|score, instructions,
// criteria } }, and answers the same keys: { choice, confidence,
// probabilities } for a choice.

export const DECISIONS_URL = "https://openrouter.ai/api/alpha/decisions";
export const DEFAULT_MODEL = "~typesafe/jev-latest";
export const KEY_TARGET = "XenoNet/OpenRouter/decisions";
// An OpenRouter key.
export const KEY = /^sk-or-[A-Za-z0-9_-]{20,200}$/;

export class DecisionError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}

export function createDecider({ secrets, fetch = globalThis.fetch, log = null, now = Date.now, env = process.env, timeoutMs = 30000 }) {
  const url = env.XENONET_DECISIONS_URL || DECISIONS_URL;
  const model = env.XENONET_DECISION_MODEL || DEFAULT_MODEL;
  let hasKey = null;

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
      k = String(k ?? "").trim();
      if (!KEY.test(k)) throw new DecisionError(400, "bad-request", "that isn't an OpenRouter key (sk-or-…)");
      await secrets.set(KEY_TARGET, k, { comment: "OpenRouter key XenoNet uses for decision models" });
      hasKey = true;
      return { key: "stored", store: secrets.kind };
    },
    async forgetKey() {
      await secrets.remove(KEY_TARGET).catch(() => false);
      hasKey = false;
      return { key: "missing" };
    },
    // One request: every question in it answered together. Zero data
    // retention asked of the provider; no user or session ids sent.
    async decide({ kind, state, questions }) {
      const k = await key();
      if (!k) throw new DecisionError(409, "no-key", "XenoNet has no OpenRouter key for decisions yet");
      const started = now();
      let res;
      try {
        res = await fetch(url, {
          method: "POST",
          headers: { authorization: `Bearer ${k}`, "content-type": "application/json", "x-title": "XenoNet" },
          body: JSON.stringify({ model, state, questions, provider: { zdr: true } }),
          signal: AbortSignal.timeout(timeoutMs),
        });
      } catch (err) {
        log?.warn("decide.unreachable", `couldn't reach ${new URL(url).host} for a ${kind} decision: ${err.name === "TimeoutError" ? "it took too long" : err.message}`);
        throw new DecisionError(502, "unreachable", `couldn't reach OpenRouter: ${err.name === "TimeoutError" ? "it took too long" : err.message}`);
      }
      const body = await res.json().catch(() => null);
      if (!res.ok) {
        const why = typeof body?.error?.message === "string" ? body.error.message.slice(0, 200) : `HTTP ${res.status}`;
        log?.warn("decide.refused", `OpenRouter answered ${res.status} to a ${kind} decision: ${why}`);
        throw new DecisionError(res.status === 401 || res.status === 403 ? 409 : 502, res.status === 401 ? "bad-key" : "refused", `OpenRouter said: ${why}`);
      }
      const ms = now() - started;
      const cost = Number(body?.usage?.cost) || 0;
      log?.info(`decide.${kind}`, `${body?.model ?? model} answered ${Object.keys(questions).length} question${Object.keys(questions).length === 1 ? "" : "s"} in ${ms} ms, $${cost.toFixed(5)}`);
      return { answers: body?.answers ?? {}, model: body?.model ?? model, provider: body?.provider ?? null, cost, ms };
    },
  };
}
