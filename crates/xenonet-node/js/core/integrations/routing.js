// core/integrations/routing.js in Rust (crates/xenonet-integrations/src/
// routing.rs): a suggestion of which agent should take a task. What goes
// out, the check of the decider's answer and the record are the Rust's;
// JevRouter (an npm library) routes, here.
import { JevRouter } from "jevrouter";
import { native } from "../../index.js";
import { settle, fail, createTickets } from "./common.js";

const NOTE = native.intgConstants().routingNote;

export const routingInventory = (nodes = []) => native.intgInventory(nodes);

export function createRouting({ decider, inventory, records, now = Date.now }) {
  const tickets = createTickets({ now });
  let busy = false;
  return {
    async preview(body) {
      const request = settle(native.intgRoutingRequest(body));
      const candidates = inventory();
      return { ...tickets.issue({ request, candidates }), outgoing: native.intgOutgoing(decider.model, request, candidates), candidates, note: NOTE };
    },
    async route(ticket) {
      if (busy) fail("A routing decision is already running.", "busy", 409);
      const { request, candidates } = tickets.take(ticket);
      if (JSON.stringify(candidates) !== JSON.stringify(inventory())) fail("The available agents changed. Preview again.", "changed", 409);
      busy = true;
      let metrics = { model: decider.model, cost: 0, ms: 0 };
      try {
        const provider = {
          name: "XenoNet OpenRouter",
          model: decider.model,
          async decide() {
            const response = await decider.decide({ kind: "routing", ...native.intgPayload(request, candidates) });
            metrics = response;
            if (!native.intgAnswerOk(response.answers, candidates)) throw new Error("The provider returned an invalid probability distribution.");
            return { answers: response.answers, model: response.model };
          },
        };
        const result = await new JevRouter(provider, { min_confidence: 0.75, require_verified_candidates: true, allowed_risk_levels: ["low"], confirmation_risk_levels: ["low"], single_stage_max_candidates: 32 }).route({ request, actor_permissions: ["suggest"] }, candidates);
        return records.add(native.intgDecisionRecord(result, request, metrics));
      } finally {
        busy = false;
      }
    },
    outcome(id, outcome) {
      settle(native.intgOutcomeOk(outcome));
      if (records.list().find((r) => r.id === id)?.kind !== "decision") fail("No such decision.", "missing", 404);
      return records.update(id, { outcome });
    },
  };
}
