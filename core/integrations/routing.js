import { createHash } from "node:crypto";
import { JevRouter } from "jevrouter";
import { clean, fail, text, createTickets } from "./common.js";

export function routingInventory(nodes = []) {
  return [
    { id: "codex", name: "Codex", description: "Coding, repository inspection and tool-assisted research in Codex.", available: nodes.some((n) => n.kind === "session" && n.app === "codex") },
    { id: "claude", name: "Claude Code", description: "Coding and repository work in Claude Code.", available: nodes.some((n) => n.kind === "session" && n.app === "claude") },
    { id: "review", name: "Human review", description: "Keep the task for human review when requirements, permissions or evidence are unclear.", available: true },
  ].map(({ available, ...c }) => ({ ...c, type: c.id === "review" ? "skill" : "subagent", permissions: ["suggest"], risk: { level: "low" }, availability: { available }, verification: { status: "verified", source: "XenoNet observed sessions" }, policy: { requires_confirmation: true } }));
}

export function createRouting({ decider, inventory, records, now = Date.now }) {
  const tickets = createTickets({ now });
  let busy = false;
  function payload(request, candidates) {
    return { state: { request }, questions: { tool: { type: "choice", instructions: "Which single candidate should handle this task? Prefer human review if none fits. This is a suggestion only.", criteria: Object.fromEntries(candidates.map((c) => [c.id, { description: c.description, available: c.availability.available }])) } } };
  }
  return {
    async preview(body) {
      const request = text(body?.request, "Task");
      if (/sk-[\w-]{16,}|Bearer\s+\S+|(?:password|api[_-]?key|token|secret)\s*[=:]/i.test(request)) fail("Remove credentials from the task before sending it.");
      const candidates = inventory();
      const outgoing = payload(request, candidates);
      return { ...tickets.issue({ request, candidates }), outgoing: { model: decider.model, ...outgoing, provider: { zdr: true } }, candidates, note: "Only this task and the listed candidate descriptions go to OpenRouter. The saved record contains a task hash, scores and outcome, never task text. No agent starts." };
    },
    async route(ticket) {
      if (busy) fail("A routing decision is already running.", "busy", 409);
      const { request, candidates } = tickets.take(ticket);
      if (JSON.stringify(candidates) !== JSON.stringify(inventory())) fail("The available agents changed. Preview again.", "changed", 409);
      busy = true;
      let metrics = { model: decider.model, cost: 0, ms: 0 };
      try {
        const provider = { name: "XenoNet OpenRouter", model: decider.model, async decide() {
          const response = await decider.decide({ kind: "routing", ...payload(request, candidates) });
          metrics = response;
          const answer = response.answers.tool;
          if (!answer || !Number.isFinite(answer.confidence) || answer.confidence < 0 || answer.confidence > 1 || !candidates.some((c) => c.id === answer.choice) || Object.keys(answer.probabilities ?? {}).length !== candidates.length || candidates.some((c) => !Number.isFinite(answer.probabilities?.[c.id]) || answer.probabilities[c.id] < 0 || answer.probabilities[c.id] > 1) || Math.abs(Object.values(answer.probabilities).reduce((sum, p) => sum + p, 0) - 1) > 0.02) throw new Error("The provider returned an invalid probability distribution.");
          return { answers: response.answers, model: response.model };
        } };
        const result = await new JevRouter(provider, { min_confidence: 0.75, require_verified_candidates: true, allowed_risk_levels: ["low"], confirmation_risk_levels: ["low"], single_stage_max_candidates: 32 }).route({ request, actor_permissions: ["suggest"] }, candidates);
        return records.add({ kind: "decision", status: result.status, selected: result.decision.selected, taskHash: createHash("sha256").update(request).digest("hex"), model: clean(metrics.model), cost: Number(metrics.cost) || 0, ms: Number(metrics.ms) || 0, reason: result.fallback.type ?? null, candidates: result.decision.candidates.map((c) => ({ id: c.id, name: c.name, probability: c.jev_probability, filtered: c.router.filtered, reason: clean(c.router.filter_reason), confirmation: c.router.requires_confirmation })), outcome: "unreviewed" });
      } finally { busy = false; }
    },
    outcome(id, outcome) {
      if (!["accepted", "rejected", "useful", "unhelpful", "unreviewed"].includes(outcome)) fail("Choose a valid decision outcome.");
      if (records.list().find((r) => r.id === id)?.kind !== "decision") fail("No such decision.", "missing", 404);
      return records.update(id, { outcome });
    },
  };
}
