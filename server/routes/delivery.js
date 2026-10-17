// Delivery's controls, yours only (never an agent's or an actor's): the
// pause switch, scopes granted to actors' tokens, consent to wake or type
// into a session, and the audit log. GET /api/delivery reads them; the
// POSTs change them. Every change goes in the audit log (policy.js).
import { bearerOf } from "../../core/actors/tokens.js";
import { isSessionId } from "../../core/schema/ids.js";
import { fromOpenCodeId } from "../../core/delivery/opencode.js";

// Consent is kept under the id the router looks it up by, so an OpenCode
// session's own ses_… becomes XenoNet's opencode:ses-…, and anything that
// isn't a session's id (title words, a typo) is refused rather than kept
// where nothing would ever read it.
function sessionOf(text) {
  const id = fromOpenCodeId(text);
  if (!isSessionId(id)) throw new Error(`"${String(text ?? "")}" isn't a session's id: give its id, such as opencode:ses-… or claude:…, which xenonet agents --json lists`);
  return id;
}

export function deliveryRoutes({ policy, router, verify, readJson, send }) {
  return async function route(req, res, url) {
    const route = url.pathname;
    try {
      if (bearerOf(req.headers) !== undefined) return send(res, 403, { error: "delivery's controls are yours, and an actor can't change them", code: "yours" });
      if (req.method === "GET" && route === "/api/delivery") {
        return send(res, 200, { paused: policy.paused, grants: policy.grants(), consent: policy.data.consent, recent: router?.list(50) ?? [], audit: policy.readAudit(Number(url.searchParams.get("audit")) || 50) });
      }
      if (req.method !== "POST") return send(res, 404, { error: "not found" });
      const you = await verify.person(req.socket);
      if (!you.ok) return send(res, 403, { error: you.reason });
      const b = await readJson(req);
      const hours = (h, d) => Math.max(1 / 60, Math.min(Number(h) || d, 24 * 90)) * 3600e3;
      if (route === "/api/delivery/pause") return send(res, 200, policy.setPaused(b.paused !== false));
      if (route === "/api/delivery/grant") return send(res, 200, policy.grant(String(b.actor ?? ""), String(b.scope ?? ""), { ttlMs: hours(b.hours, 24 * 30) }));
      if (route === "/api/delivery/revoke") return send(res, 200, policy.revoke(String(b.actor ?? "")));
      if (route === "/api/delivery/allow") {
        const session = sessionOf(b.session);
        return send(res, 200, { session, ...policy.allow(session, { remote: !!b.remote, ttlMs: hours(b.hours, 8) }) });
      }
      // Taking consent back works for any key, so one kept before this check can still go.
      if (route === "/api/delivery/disallow") return send(res, 200, (policy.disallow(fromOpenCodeId(b.session)), { ok: true }));
      return send(res, 404, { error: "not found" });
    } catch (err) {
      return send(res, 400, { error: String(err?.message ?? err) });
    }
  };
}
