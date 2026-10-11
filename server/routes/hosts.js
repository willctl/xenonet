// Hosts over the API (docs/design/hosts.md). Being a host needs no flag:
//
//   GET    /api/hosts/self         this XenoNet as a host: the name it gave
//                                  itself, its key (made the first time
//                                  anything asks) and the folder ~ stands
//                                  for, which xenonet relay needs before
//                                  anything leaves this machine
//   POST   /api/hosts/self         { id } names it (null takes the name
//                                  away); yours to do
//
// Showing other hosts is behind XENONET_HOSTS=1 (`hub`); without it these
// answer 404:
//
//   GET    /api/hosts              this XenoNet's own id and key, and every
//                                  host with how it stands
//   POST   /api/hosts              { id, target, port, … } adds one (see
//                                  hostEntry); yours to do
//   GET    /api/hosts/:id          one host, with its engine's agents and
//                                  messages under qualified ids
//   DELETE /api/hosts/:id          forgets it, and with ?forget=1 where you
//                                  placed its nodes; yours to do
//   POST   /api/hosts/:id/reconnect  tries now
//   POST   /api/hosts/:id/test     one try, saying how far it got
//
// and a remote session's transcript and stats, and a collision on one
// host, read through its relay (GET /api/session/@<host>:…/transcript,
// /stats, GET /api/collision?a=@<host>:…&b=@<host>:…). Every other route
// that takes an id refuses remote ones, since it only knows this
// machine's: hosts are read-only. Returns whether it answered.
import { parseQualified, isHostId } from "../../core/schema/ids.js";

const SESSION = /^(?:claude|codex|grok):[0-9A-Za-z][0-9A-Za-z._-]{0,127}$/;
const LIMIT = /^\d{1,4}$/;
const CURSOR = /^[0-9A-Za-z._:-]{1,200}$/;
const OFF = "hosts are off in this XenoNet: start it with XENONET_HOSTS=1 to show other machines' XenoNets (a prototype)";

const remoteSession = (id) => {
  const q = parseQualified(id);
  return q?.host && SESSION.test(q.local) ? q : null;
};

export function hostRoutes({ hosts, hub = null, verify, readJson, send, home }) {
  const answer = (res, status, body) => (send(res, status, body), true);
  const yours = async (req, res) => {
    const you = await verify.person(req.socket);
    if (!you.ok) answer(res, 403, { error: you.reason });
    return you.ok;
  };

  return async function route(req, res, url) {
    const p = url.pathname;
    try {
      if (p === "/api/hosts/self") {
        if (req.method === "GET") return answer(res, 200, { ...hosts.self(), home });
        if (req.method === "POST") {
          if (!(await yours(req, res))) return true;
          const { id } = await readJson(req);
          if (id === undefined) return answer(res, 400, { error: "say what to call this XenoNet: { id }, or id null to take its name away" });
          return answer(res, 200, { ...hosts.setSelf(id), home });
        }
        return answer(res, 405, { error: "GET or POST" });
      }

      // A remote session's transcript or stats, through its host's relay.
      const read = /^\/api\/session\/([^/]+)\/(transcript|stats)$/.exec(p);
      if (read && req.method === "GET") {
        let id;
        try {
          id = decodeURIComponent(read[1]);
        } catch {
          return false;
        }
        if (!id.startsWith("@") || !hub) return false;
        const q = remoteSession(id);
        if (!q || !hub.has(q.host)) return answer(res, 404, { error: "XenoNet doesn't know that session" });
        const params = new URLSearchParams();
        for (const [k, test] of [["limit", LIMIT], ["cursor", CURSOR]]) {
          const v = url.searchParams.get(k);
          if (v != null && read[2] === "transcript") {
            if (!test.test(v)) return answer(res, 400, { error: `${k} isn't one XenoNet reads` });
            params.set(k, v);
          }
        }
        const r = await hub.get(q.host, `/api/session/${q.local}/${read[2]}${params.size ? `?${params}` : ""}`);
        const body = r.body && typeof r.body === "object" && !Array.isArray(r.body) ? { ...r.body, ...(typeof r.body.id === "string" ? { id } : {}), host: q.host } : r.body;
        return answer(res, r.status, body);
      }

      // A collision on one host, with both sides' diffs, through its relay.
      if (p === "/api/collision" && req.method === "GET" && hub && [url.searchParams.get("a"), url.searchParams.get("b")].some((x) => x?.startsWith("@"))) {
        const a = remoteSession(url.searchParams.get("a"));
        const b = remoteSession(url.searchParams.get("b"));
        if (!a || !b || a.host !== b.host) return answer(res, 400, { error: "collisions across machines come later: both sessions have to be on one host" });
        if (!hub.has(a.host)) return answer(res, 404, { error: `there's no host called ${a.host}` });
        const r = await hub.get(a.host, `/api/collision?a=${a.local}&b=${b.local}`);
        const body = r.body && typeof r.body === "object" ? { ...r.body, a: { ...r.body.a, id: url.searchParams.get("a") }, b: { ...r.body.b, id: url.searchParams.get("b") }, host: a.host } : r.body;
        return answer(res, r.status, body);
      }

      if (p !== "/api/hosts" && !p.startsWith("/api/hosts/")) return false;
      if (!hub) return answer(res, 404, { error: OFF });

      if (p === "/api/hosts") {
        if (req.method === "GET") return answer(res, 200, { self: hosts.self(), sshConfig: hosts.sshConfig, hosts: hub.list() });
        if (req.method === "POST") {
          if (!(await yours(req, res))) return true;
          const body = await readJson(req);
          return answer(res, 200, hub.add(body, { replace: body?.replace === true }));
        }
        return answer(res, 405, { error: "GET or POST" });
      }
      const m = /^\/api\/hosts\/([^/]+)(?:\/(reconnect|test))?$/.exec(p);
      if (!m || !isHostId(m[1])) return answer(res, 404, { error: "not found" });
      const [, id, action] = m;
      if (!action && req.method === "GET") return answer(res, 200, hub.detail(id));
      if (!action && req.method === "DELETE") {
        if (!(await yours(req, res))) return true;
        return answer(res, 200, hub.remove(id, { forget: url.searchParams.get("forget") === "1" }));
      }
      if (action === "reconnect" && req.method === "POST") return answer(res, 200, hub.reconnect(id));
      if (action === "test" && req.method === "POST") {
        if (!hub.has(id)) return answer(res, 404, { error: `there's no host called ${id}` });
        return answer(res, 200, await hub.test(id));
      }
      return answer(res, 405, { error: "not that way" });
    } catch (err) {
      return answer(res, err.status ?? 400, { error: String(err.message ?? err) });
    }
  };
}
