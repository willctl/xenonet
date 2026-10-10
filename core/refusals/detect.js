// A provider's refusal, as a transcript line shows it (docs/guide/
// refusals.md): read from Claude Code's and Codex's own lines, the way
// they mark it, never guessed from what a model wrote.
//
// Claude Code writes an assistant line that carries the API's stop_reason
// "refusal": a synthetic one (isApiErrorMessage, model "<synthetic>") with
// the request's id and the API's stop_details (a category and an
// explanation), or a model's own reply that ends that way. Newer versions
// also write system lines: model_refusal_no_fallback when nothing took
// over, model_refusal_fallback when another model answered instead (a
// refusal recovered), and a reply may carry a fallback block whose trigger
// was a refusal. Codex writes an error event whose codex_error_info names a
// policy (cyber_policy, bio_policy, misalignment_policy_violation), and a
// model_reroute when it moved the turn to another model.
//
// claudeSignal(raw) and codexSignal(raw) take a line as the file has it
// and return { provider, kind, source, category, explanation, text,
// requestId, messageId, uuid, parentUuid, model, fallbackModel, code, at }
// or null. kind is refusal, refusal-recovered, reroute or possible-refusal
// (an invalid_prompt, which isn't always a policy). A credit token the API
// sends with a fallback is never kept.

const POLICY = /policy|cyber|bio|misalignment|safety|content_filter/i;
const textOf = (content) =>
  (Array.isArray(content) ? content : [content])
    .map((b) => (typeof b === "string" ? b : b?.type === "text" ? b.text : ""))
    .filter(Boolean)
    .join("\n")
    .slice(0, 4000) || null;
const timeOf = (t) => (typeof t === "string" ? Date.parse(t) || null : null);
const words = (v, max = 2000) => (typeof v === "string" && v ? v.slice(0, max) : null);

const base = (provider, raw) => ({ provider, kind: "refusal", source: null, category: null, explanation: null, text: null, requestId: null, messageId: null, uuid: raw?.uuid ?? null, parentUuid: raw?.parentUuid ?? null, model: null, fallbackModel: null, code: null, at: timeOf(raw?.timestamp) });

export function claudeSignal(raw) {
  if (!raw || typeof raw !== "object") return null;
  const m = raw.message && typeof raw.message === "object" ? raw.message : null;
  if (raw.type === "assistant" && m) {
    const model = m.model && m.model !== "<synthetic>" ? m.model : null;
    if (m.stop_reason === "refusal") {
      return {
        ...base("anthropic", raw),
        source: raw.isApiErrorMessage ? "api-error" : "stop-reason",
        category: words(m.stop_details?.category, 80),
        explanation: words(m.stop_details?.explanation),
        text: textOf(m.content),
        requestId: words(raw.requestId, 120),
        messageId: words(m.id, 120),
        model,
        code: words(raw.error, 80),
      };
    }
    const fallback = (Array.isArray(m.content) ? m.content : []).find((b) => b?.type === "fallback" && b.trigger?.type === "refusal");
    if (fallback) return { ...base("anthropic", raw), kind: "refusal-recovered", source: "fallback-block", category: words(fallback.trigger?.category, 80), requestId: words(raw.requestId, 120), messageId: words(m.id, 120), model: words(fallback.original_model, 120), fallbackModel: model };
    return null;
  }
  if (raw.type === "system" && (raw.subtype === "model_refusal_no_fallback" || raw.subtype === "model_refusal_fallback")) {
    const recovered = raw.subtype === "model_refusal_fallback";
    return {
      ...base("anthropic", raw),
      kind: recovered ? "refusal-recovered" : "refusal",
      source: raw.subtype,
      category: words(raw.apiRefusalCategory, 80),
      explanation: words(raw.apiRefusalExplanation),
      text: words(raw.content),
      requestId: words(raw.requestId, 120),
      model: words(raw.originalModel ?? raw.model, 120),
      fallbackModel: recovered ? words(raw.fallbackModel, 120) : null,
    };
  }
  return null;
}

export function codexSignal(raw) {
  if (!raw || typeof raw !== "object" || raw.type !== "event_msg") return null;
  const p = raw.payload ?? {};
  if (p.type === "error") {
    const info = p.codex_error_info;
    const code = typeof info === "string" ? info : info && typeof info === "object" ? Object.keys(info)[0] ?? null : null;
    if (!code) return null;
    const detail = info && typeof info === "object" ? info[code] : null;
    const kind = POLICY.test(code) ? "refusal" : code === "invalid_prompt" ? "possible-refusal" : null;
    if (!kind) return null;
    return { ...base("openai", raw), kind, source: "error-event", category: code, explanation: words(detail?.detailed_explanation ?? detail?.explanation), text: words(p.message), code };
  }
  if (p.type === "model_reroute") return { ...base("openai", raw), kind: "reroute", source: "model-reroute", category: words(p.reason, 80), model: words(p.from_model, 120), fallbackModel: words(p.to_model, 120) };
  return null;
}

// What makes two sightings of a refusal the same one: the request's id
// when there is one, else where it was written.
export const keyOf = (sig, sessionId) => `${sig.provider}|${sig.kind}|${sig.requestId ?? sig.messageId ?? sig.uuid ?? `${sessionId}@${sig.at}`}`;
