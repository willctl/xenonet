// Delivery's controls, yours only (never an agent's or an actor's): the
// pause switch, scopes granted to actors' tokens, consent to wake or type
// into a session, and the audit log. GET /api/delivery reads them; the
// POSTs change them. Every change goes in the audit log (policy.js).
import { bearerOf } from "../../core/actors/tokens.js";

export function deliveryRoutes({ policy, router, verify, readJson, send }) {
  return async function route(req, res, url) {
    const route = url.pathname;
    try {
      if (bearerOf(req.headers) !== undefined) return send(res, 403, { error: "delivery's controls are yours, and an actor can't change them", code: "yours" });
      if (req.method === "GET" && route === "/api/delivery") {
        // Plans as the wake budget reads them (a provider at 90% takes no
        // wakes), and the sessions listening after a turn now.
        const usage = Object.fromEntries(["claude", "codex"].map((app) => [app, router?.usage?.(app) ?? null]));
        const listening = [...(router?.engine?.listening?.entries() ?? [])].map(([agent, until]) => ({ agent, until }));
        return send(res, 200, { paused: policy.paused, grants: policy.grants(), consent: policy.data.consent, usage, listening, recent: router?.list(50) ?? [], audit: policy.readAudit(Number(url.searchParams.get("audit")) || 50) });
      }
      if (req.method !== "POST") return send(res, 404, { error: "not found" });
      const you = await verify.person(req.socket);
      if (!you.ok) return send(res, 403, { error: you.reason });
      const b = await readJson(req);
      const hours = (h, d) => Math.max(1 / 60, Math.min(Number(h) || d, 24 * 90)) * 3600e3;
      if (route === "/api/delivery/pause") return send(res, 200, policy.setPaused(b.paused !== false));
      if (route === "/api/delivery/grant") return send(res, 200, policy.grant(String(b.actor ?? ""), String(b.scope ?? ""), { ttlMs: hours(b.hours, 24 * 30) }));
      if (route === "/api/delivery/revoke") return send(res, 200, policy.revoke(String(b.actor ?? "")));
      if (route === "/api/delivery/allow") return send(res, 200, policy.allow(String(b.session ?? ""), { remote: !!b.remote, ttlMs: hours(b.hours, 8) }));
      if (route === "/api/delivery/disallow") return send(res, 200, (policy.disallow(String(b.session ?? "")), { ok: true }));
      return send(res, 404, { error: "not found" });
    } catch (err) {
      return send(res, 400, { error: String(err?.message ?? err) });
    }
  };
}
