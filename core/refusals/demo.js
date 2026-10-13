// Only the demo/test instance uses these records. No user transcripts.
export function demoRefusals(now = Date.now()) {
  return [
    ["anthropic", "refusal", "claude-sonnet", "safety", "The provider declined this request."],
    ["anthropic", "refusal-recovered", "claude-opus", "policy", "The request was answered by a fallback model."],
    ["openai", "reroute", "gpt-example", "safety", "The provider routed this request to another model."],
    ["openai", "possible-refusal", "gpt-example", "invalid_prompt", "The provider returned invalid_prompt; a refusal is not confirmed."],
  ].map(([provider, kind, model, category, explanation], i) => ({
    schema: "refusal/record/v1", id: `20261001-${(i + 1).toString(16).padStart(8, "0")}`,
    at: now - i * 3600e3, recordedAt: now, provider, app: provider === "openai" ? "codex" : "claude-code", kind,
    session: { id: `demo-session-${i + 1}`, cwd: "~/Projects/demo", model },
    signal: { kind, model, category, explanation, ...(kind === "refusal-recovered" || kind === "reroute" ? { fallbackModel: "fallback-example" } : {}) },
    source: { file: `~/demo/transcript-${i + 1}.jsonl` }, context: i === 0 ? { file: "context.jsonl.gz", request: "request.json", lines: 3, bytes: 256, truncated: false } : null,
    missing: i === 0 ? ["tool definitions"] : [],
  }));
}
