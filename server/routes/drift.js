// Drift in the other apps' data XenoNet reads (core/drift.js, core/vendor.js),
// over the API, for xenonet drift and the console:
//
//   GET  /api/drift        the drift report (driftReport): each source with
//                          a schema, what this XenoNet has read of it since it
//                          started, and its drift that's new since you last
//                          looked; ?all=1 lists every record kept
//   POST /api/drift/seen   { at } you've looked at the report made at `at`
//                          (now, when left out): what's first seen after it
//                          stays new. Answers { lookedAt }
//
// Marking drift seen quiets the doctor's contracts.drift check, so
// `onSeen` runs the doctor again for the console to show: a fresh run
// (check({ fresh: true }), see core/doctor/service.js), since one under way
// looked before the mark and would show the warning still.
import { driftReport } from "../../core/vendor.js";

export function driftRoutes({ log, home, readJson, send, onSeen = null, now = () => Date.now() }) {
  return async function route(req, res, url) {
    if (req.method === "GET" && url.pathname === "/api/drift") return send(res, 200, driftReport({ log, all: url.searchParams.get("all") === "1", home, now: now() }));
    if (req.method === "POST" && url.pathname === "/api/drift/seen") {
      const { at } = (await readJson(req)) ?? {};
      if (at != null && !Number.isFinite(at)) return send(res, 400, { error: "at is when the report you looked at was made, in epoch milliseconds" });
      // Never past now: drift that turns up later is still new.
      log.looked(Math.min(at ?? now(), now()));
      log.flush();
      onSeen?.();
      return send(res, 200, { lookedAt: log.lookedAt });
    }
    return send(res, 404, { error: "not found" });
  };
}
