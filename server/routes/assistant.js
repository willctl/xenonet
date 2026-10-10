// Remote actors' connections over the API (core/actors/connections.js;
// docs/guide/cloud-assistants.md). None of this is reachable from a
// connection itself, which has only the private endpoint's one read
// (server/assistant-endpoint.js).
//
// Yours, from XenoNet's window or your own terminal (verify.person: not from
// under an agent or an actor product, and never with a token):
//
//   GET    /api/assistant               the connections, whether cloud
//                                       access is off, this device's id,
//                                       the products that can be paired, and
//                                       diagnostics: versions, whether the
//                                       endpoint listens and how many
//                                       adapters are connected, and how
//                                       each source stands
//   GET    /api/assistant/places        the projects and folders a grant
//                                       can name, and the identities a link
//                                       can be your word for
//   GET    /api/assistant/audit         what was read lately: times, codes,
//                                       sizes; ?grant= for one
//   POST   /api/assistant/<grant>/revoke  revokes it, for good
//   POST   /api/assistant/disable       turns every connection off
//   GET    /api/assistant/tunnels       the tunnels XenoNet can run for
//                                       connections through one: whether
//                                       tunnel-client is here, and for
//                                       each, its tunnel, whether a key is
//                                       kept (never the key), and how it
//                                       runs (core/assistant/tunnels.js)
//   POST   /api/assistant/<grant>/tunnel/stop    stops its tunnel
//   POST   /api/assistant/<grant>/tunnel/forget  takes away the key XenoNet
//                                       kept for it
//
// XenoNet's own window only, with its key (x-xenonet-window), since they make
// or widen access:
//
//   POST   /api/assistant/preview       {projects, labels}: exactly what
//                                       would leave, made-up first, then
//                                       the real fields for those projects
//   POST   /api/assistant/pair          a new connection: 201 with your
//                                       view of it and its launch command;
//                                       its credential only ever on disk
//   POST   /api/assistant/<grant>       {projects, labels}: a new scope
//   POST   /api/assistant/<grant>/rotate  a new credential
//   POST   /api/assistant/enable        on again, once you've turned it off
//   POST   /api/assistant/<grant>/tunnel      {tunnelId, keyTarget,
//                                       autostart}: which tunnel, where its
//                                       key is kept, whether it starts with
//                                       XenoNet
//   POST   /api/assistant/<grant>/tunnel/key  {key}: the tunnel's runtime
//                                       key, into your credential store; it
//                                       never comes back
//   POST   /api/assistant/<grant>/tunnel/start  runs it
//   POST   /api/assistant/<grant>/tunnel/check  tunnel-client doctor, with
//                                       the key: its lines, keys taken out
//
// Errors are {"error": "<a sentence>", "code": "<code>"}.
import { bearerOf } from "../../core/actors/tokens.js";
import { ConnectionError } from "../../core/actors/connections.js";
import { TunnelError } from "../../core/assistant/tunnels.js";
import { ADAPTERS, TRANSPORTS } from "../../core/actors/adapters.js";
import { productOf } from "../../core/actors/products.js";
import { createFixtureService } from "../../core/assistant/service.js";
import { visibleSessions, sessionSummary, emptyCounts, cleanLabel, sourcesData, WINDOW_MS } from "../../core/assistant/projection.js";
import { GRANT_ID } from "../../core/schema/assistant.js";
import crypto from "node:crypto";

const BODY = 16 << 10;
const iso = (ms) => new Date(ms).toISOString();

export function assistantRoutes({ connections, tunnels = null, audit, source, state, endpoint = null, windowKey = null, verify = null, readJson, send, version = "0.0.0", now = Date.now }) {
  const fail = (res, status, code, error, extra = {}) => send(res, status, { error, code, ...extra });

  async function fromWindow(req) {
    if (bearerOf(req.headers) !== undefined) return "this is for XenoNet's own window, and an actor can't do it";
    if (!windowKey?.matches(req.headers["x-xenonet-window"])) return "this is for XenoNet's own window, and the request didn't come with its key";
    const checked = verify?.window ? await verify.window(req.socket) : { ok: true };
    return checked.ok ? null : checked.reason;
  }
  async function fromYou(req) {
    if (bearerOf(req.headers) !== undefined) return "this is yours to do, and an actor can't do it";
    if (req.headers["x-xenonet-window"] !== undefined) return fromWindow(req);
    const checked = verify?.person ? await verify.person(req.socket) : { ok: true };
    return checked.ok ? null : checked.reason;
  }
  const guard = async (req, res, check) => {
    const refused = await check(req);
    if (refused) fail(res, 403, "yours", refused);
    return !refused;
  };
  const objectOf = async (req) => {
    const body = await readJson(req, BODY);
    if (body === null || typeof body !== "object" || Array.isArray(body)) throw new ConnectionError(400, "bad-request", "the body is a JSON object");
    return body;
  };

  // The projects and folders on the map now, by node id, most active first.
  function places() {
    const t = now();
    const counts = new Map();
    for (const n of state.index.values()) {
      if (n.kind !== "session" || n.host || n.archived || (n.updatedAt ?? 0) < t - WINDOW_MS) continue;
      const home = state.homeOf(n.id);
      counts.set(home, (counts.get(home) ?? 0) + 1);
    }
    return [...state.index.values()]
      .filter((n) => (n.kind === "project" || n.kind === "zone") && !n.host)
      .map((n) => ({ place: n.id, name: n.name ?? null, kind: n.kind, sessions7d: counts.get(n.id) ?? 0, lastActivity: n.lastActivity ?? null }))
      .sort((a, b) => b.sessions7d - a.sessions7d || (b.lastActivity ?? 0) - (a.lastActivity ?? 0) || String(a.name).localeCompare(String(b.name)));
  }
  const placeMap = () => new Map(places().map((p) => [p.place, p]));
  const identities = () => new Set((state.graph?.identities ?? []).map((i) => i.id));

  // What a grant with these projects and labels would send: the made-up
  // answers first, then xenonet_projects and xenonet_sessions as they'd read
  // the map now, under ids made for the preview alone.
  async function preview(body) {
    const known = placeMap();
    const chosen = [...new Set(Array.isArray(body.projects) ? body.projects : [])].filter((p) => known.has(p)).slice(0, 64);
    const labels = body.labels === true;
    const fixture = createFixtureService({ version, labels, now });
    const session = { grantId: "gr_000000000000000000000000", actorId: "actor:fixture-client", connectionId: "cn_000000000000000000000000" };
    const synthetic = {};
    for (const op of ["projects", "sessions", "attention"]) synthetic[op] = (await fixture.read(session, { request_id: `r_${crypto.randomUUID()}`, operation: op, arguments: {} })).result?.data ?? null;
    const policy = { labels, lookbackMs: WINDOW_MS, places: new Map(chosen.map((place) => [`prj_${crypto.randomUUID()}`, place])) };
    const t = now();
    const view = source.view();
    const visible = visibleSessions(view, policy, t);
    const counts = new Map([...policy.places.keys()].map((p) => [p, emptyCounts()]));
    for (const v of visible) counts.get(v.project)[v.status]++;
    return {
      at: iso(t),
      labels,
      synthetic,
      real: {
        projects: [...policy.places.keys()].map((p) => ({ project_id: p, label: labels ? cleanLabel(view.places.get(policy.places.get(p))?.name ?? null) : null, session_counts: counts.get(p) })),
        sessions: visible.sort((a, b) => (b.s.updatedAt ?? 0) - (a.s.updatedAt ?? 0)).slice(0, 20).map((v) => sessionSummary(v, policy)),
      },
      // Which preview id is which of your places: for this window, never sent.
      yours: [...policy.places].map(([project_id, place]) => ({ project_id, place, name: known.get(place)?.name ?? null })),
    };
  }

  const products = () =>
    Object.entries(ADAPTERS).map(([key, a]) => ({ key, product: a.product, name: a.product ? productOf(a.product).name : "An actor of your own", transports: a.transports.map((t) => ({ id: t, name: TRANSPORTS[t].name })), support: a.support }));

  return async function route(req, res, url) {
    const p = url.pathname;
    const m = req.method;
    if (!connections) return fail(res, 503, "off", "Actors are off here (XENONET_ACTORS=off), and remote actors with them");
    try {
      if (p === "/api/assistant" && m === "GET") {
        if (!(await guard(req, res, fromYou))) return;
        const listening = endpoint?.info() ?? null;
        const diagnostics = { service_version: version, assistant_api: 1, endpoint: listening ? { listening: listening.listening, adapters: listening.sessions } : null, sources: sourcesData(source.view()) };
        return send(res, 200, { ...connections.list(), products: products(), diagnostics });
      }
      if (p === "/api/assistant/places" && m === "GET") {
        if (!(await guard(req, res, fromYou))) return;
        return send(res, 200, { places: places(), identities: (state.graph?.identities ?? []).filter((i) => !i.hidden).map((i) => ({ id: i.id, kind: i.kind, name: i.name ?? null })) });
      }
      if (p === "/api/assistant/audit" && m === "GET") {
        if (!(await guard(req, res, fromYou))) return;
        const grant = url.searchParams.get("grant");
        if (grant && !GRANT_ID.test(grant)) return fail(res, 400, "bad-request", "grant is a grant's id");
        return send(res, 200, { records: audit.recent({ grant: grant || null, limit: Number(url.searchParams.get("limit")) || 100 }) });
      }
      if (p === "/api/assistant/disable" && m === "POST") {
        if (!(await guard(req, res, fromYou))) return;
        return send(res, 200, connections.disable());
      }
      // Turning access back on widens it again, so it's the window's alone;
      // turning it off is anyone's who is you, even from a terminal.
      if (p === "/api/assistant/enable" && m === "POST") {
        if (!(await guard(req, res, fromWindow))) return;
        return send(res, 200, connections.enable());
      }
      if (p === "/api/assistant/preview" && m === "POST") {
        if (!(await guard(req, res, fromWindow))) return;
        return send(res, 200, await preview(await objectOf(req)));
      }
      if (p === "/api/assistant/pair" && m === "POST") {
        if (!(await guard(req, res, fromWindow))) return;
        const body = await objectOf(req);
        return send(res, 201, connections.pair(body, { places: placeMap(), identities: identities() }));
      }
      if (p === "/api/assistant/tunnels" && m === "GET") {
        if (!(await guard(req, res, fromYou))) return;
        if (!tunnels) return fail(res, 503, "off", "XenoNet runs no tunnels here: remote actors are off");
        return send(res, 200, await tunnels.status());
      }
      const tunnel = /^\/api\/assistant\/(gr_[0-9a-f]{24})\/tunnel(?:\/(key|start|stop|check|forget))?$/.exec(p);
      if (tunnel && m === "POST") {
        const [, grant, action] = tunnel;
        if (!tunnels) return fail(res, 503, "off", "XenoNet runs no tunnels here: remote actors are off");
        // Stopping one, or taking away its key, narrows what reaches this
        // computer: yours from a terminal too. The rest is the window's.
        if (action === "stop" || action === "forget") {
          if (!(await guard(req, res, fromYou))) return;
          return send(res, 200, action === "stop" ? tunnels.stop(grant) : await tunnels.forgetKey(grant));
        }
        if (!(await guard(req, res, fromWindow))) return;
        if (action === "start") return send(res, 200, await tunnels.start(grant));
        if (action === "check") return send(res, 200, await tunnels.check(grant));
        const body = await objectOf(req);
        if (action === "key") return send(res, 200, await tunnels.storeKey(grant, body.key));
        return send(res, 200, tunnels.configure(grant, { tunnelId: body.tunnelId, keyTarget: body.keyTarget, autostart: body.autostart }));
      }
      const one = /^\/api\/assistant\/(gr_[0-9a-f]{24})(?:\/(revoke|rotate))?$/.exec(p);
      if (one && m === "POST") {
        const [, grant, action] = one;
        if (action === "revoke") {
          if (!(await guard(req, res, fromYou))) return;
          return send(res, 200, connections.revoke(grant));
        }
        if (!(await guard(req, res, fromWindow))) return;
        if (action === "rotate") return send(res, 200, connections.rotate(grant));
        const body = await objectOf(req);
        return send(res, 200, connections.update(grant, { projects: body.projects, labels: body.labels }, { places: placeMap() }));
      }
      return fail(res, 404, "not-found", "XenoNet has no such route for remote actors");
    } catch (err) {
      if (err instanceof ConnectionError) return fail(res, err.status, err.code, err.message, err.extra);
      if (err instanceof TunnelError) return fail(res, err.status, err.code, err.message);
      if (err.status === 413) return fail(res, 413, "too-big", "that body is over the size this route takes");
      if (err.status === 400) return fail(res, 400, "bad-request", "that body isn't JSON");
      if (/couldn't keep the grant's credential private/.test(err.message)) return fail(res, 500, "credential", err.message);
      throw err;
    }
  };
}
