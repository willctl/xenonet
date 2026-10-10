// The assistant's reads (docs/design/cloud-assistants.md, section 5): the
// one thing a remote actor's adapter may ask of XenoNet once its grant's
// credential let it in (server/assistant-endpoint.js), which is
// assistant.read with one of six operations. All the checks are here, in
// the service, so a mistake in an adapter or a transport can't widen what
// a grant sees:
//
//   - the grant is looked at again for each read, and once more just
//     before its answer goes: revoked, expired or turned off by then, the
//     answer is dropped; changed in scope, the read runs again under the
//     new scope, once
//   - its arguments fit the tool's strict schema, in at most 16 KiB
//   - a grant gets 60 reads a minute, in bursts of up to ten, four at a
//     time at most, a read counting until its work ends even when it timed
//     out; the rest wait (RATE_LIMITED, with when to try again), never in
//     a queue
//   - each read has ten seconds, and its answer at most 256 KiB: a list
//     that would be bigger comes back shorter, with a page token for the
//     rest (PAGE_REDUCED)
//   - the answer is checked against the strict schema before it goes, so
//     a field the projection didn't mean can't
//
// Errors are fixed codes and sentences (assistant/error/v1), the same for
// a session that doesn't exist and one the grant doesn't see. The log
// (audit.js) keeps the precise reason.
//
// A read through a grant still in made-up mode (data_mode fixture) reads
// fixtures.js instead of the map. Each read that goes out tells the actors'
// registry the actor was at work, and which places it watched
// (remote-presence.js); nothing else changes.
import crypto from "node:crypto";
import { createBuckets } from "../actors/limits.js";
import { INPUTS, RESULTS, TOOLS, OPERATIONS, RESULT_SCHEMA, ERROR_SCHEMA, REQUEST_ID, ATTENTION_KINDS } from "../schema/assistant.js";
import { cleanLabel, visibleSessions, sessionSummary, emptyCounts, freshnessOf, sourcesData, attentionItems, actorSummaries, statsData, WINDOW_MS, LOOKBACK_MAX_MS } from "./projection.js";
import { fixtureView, fixturePolicy, fixtureStats, standalonePolicy } from "./fixtures.js";
import { createCursors } from "./cursors.js";

export const LIMITS = {
  inputBytes: 16 << 10,
  outputBytes: 256 << 10,
  concurrent: 4,
  rate: { rate: 1, burst: 10 },
  timeoutMs: 10e3,
  page: 20,
  pageMax: 50,
};

export const MESSAGES = {
  NOT_AUTHORIZED: "This connection isn't allowed to read that.",
  NOT_FOUND: "XenoNet has nothing by that id for this connection.",
  GRANT_EXPIRED: "This connection's grant has expired. Its owner can pair it again in XenoNet.",
  GRANT_REVOKED: "This connection was revoked in XenoNet.",
  DISABLED: "Cloud access is turned off in XenoNet on this computer.",
  INVALID_ARGUMENT: "The request doesn't fit this tool's arguments.",
  CURSOR_EXPIRED: "That page token has expired. Ask for the list again without one.",
  SOURCE_UNAVAILABLE: "XenoNet can't read that source on this computer right now.",
  SERVICE_UNAVAILABLE: "XenoNet Service is unavailable on this paired device.",
  RATE_LIMITED: "Too many requests through this connection. Wait, then try again.",
  INCOMPATIBLE_VERSION: "This adapter and XenoNet Service use different interface versions. Update XenoNet on this computer.",
  TIMEOUT: "XenoNet took too long to answer.",
  INTERNAL_ERROR: "XenoNet couldn't answer this request.",
};
const RETRY = { SOURCE_UNAVAILABLE: 5000, SERVICE_UNAVAILABLE: 2000, TIMEOUT: 2000, RATE_LIMITED: 1000 };

// An error's envelope, from its code alone.
export function errorEnvelope(requestId, code, { retryAfterMs } = {}) {
  const retry = retryAfterMs ?? RETRY[code] ?? null;
  return { schema: ERROR_SCHEMA, request_id: REQUEST_ID.test(String(requestId)) ? requestId : null, code, message: MESSAGES[code] ?? MESSAGES.INTERNAL_ERROR, retryable: retry != null, retry_after_ms: retry == null ? null : Math.max(0, Math.round(retry)) };
}

const TOOL_OF = Object.fromEntries(Object.entries(OPERATIONS).map(([tool, op]) => [op, tool]));
const iso = (ms) => new Date(ms).toISOString();
class Refused extends Error {
  constructor(code, reason = code) {
    super(code);
    Object.assign(this, { code, reason });
  }
}
const refuse = (code, reason) => {
  throw new Refused(code, reason);
};

// A page of `items` (sorted by `keyOf`) after the cursor's key: those
// whose key comes after it, `limit` of them, and the key to go on from.
function page(items, keyOf, after, limit) {
  const cmp = (a, b) => {
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
    return 0;
  };
  const rest = after ? items.filter((x) => cmp(keyOf(x), after) > 0) : items;
  const shown = rest.slice(0, limit);
  return { shown, next: rest.length > limit ? keyOf(shown[shown.length - 1]) : null };
}

// `connections`: grants and their standing (core/actors/connections.js).
// `source`: the view of the map (source.js). `remote`: what reads say of
// their actor (core/actors/remote-presence.js), or null. `audit`: the log.
export function createAssistantService({ connections, source, remote = null, audit = null, version = "0.0.0", now = Date.now, limits = LIMITS, cursors = createCursors({ now }), idKey = () => connections.idKey() }) {
  const buckets = createBuckets({ now });
  const inflight = new Map();

  const attentionId = (parts) => `att_${crypto.createHmac("sha256", idKey()).update(parts).digest("hex").slice(0, 24)}`;

  // What a policy reads from: the map, or the made-up one, with the actor
  // itself in it either way.
  function viewFor(policy) {
    if (policy.dataMode !== "fixture") {
      // Itself, even when the list it's read from left it out.
      const view = source.view();
      if (view.actors.some((a) => a.id === policy.actorId)) return { view, policy };
      const self = source.actor?.(policy.actorId);
      return { view: self ? { ...view, actors: [self, ...view.actors] } : view, policy };
    }
    const view = fixtureView(now());
    const self = source.actor?.(policy.actorId) ?? { id: policy.actorId, kind: "agent", productId: policy.productId ?? null, name: policy.actorId.slice("actor:".length), presence: "working", via: ["remote"], remote: { bridge: "connected", accountLink: policy.accountLink }, reachable: false, targets: [] };
    // The actor's own targets are on the real map, which the fixture isn't.
    return { view: { ...view, actors: [{ ...self, targets: [] }, ...view.actors] }, policy: fixturePolicy(policy) };
  }

  // ---------- the six operations ----------

  function limitOf(args) {
    return Math.min(args.limit ?? limits.page, limits.pageMax);
  }
  // Where a list goes on from, or a refusal for a bad token.
  function afterOf(args, policy, op, filters) {
    if (!args.cursor) return null;
    const r = cursors.read(args.cursor, { grant: policy.grantId, revision: policy.revision, op, filters });
    if (!r.ok) refuse(r.code, `cursor-${r.code.toLowerCase()}`);
    return r.after;
  }
  const nextOf = (key, policy, op, filters) => (key ? cursors.make({ grant: policy.grantId, revision: policy.revision, op, filters, after: key }) : null);
  // A project id the grant names, or NOT_FOUND, as for one that doesn't exist.
  const ownProject = (policy, id) => (id == null || policy.places.has(id) ? id : refuse("NOT_FOUND", "project-not-granted"));

  const ops = {
    status({ view, policy, t }) {
      const f = freshnessOf(view, t);
      return {
        ...f,
        data: {
          mode: "read_only",
          connection: "connected",
          assistant_api: 1,
          service_version: String(version).slice(0, 32),
          capabilities: [...TOOLS],
          actor: { actor_id: policy.actorId, product_id: policy.productId ?? null, account_link_state: policy.accountLink, participation: "read_only", messaging: "disabled" },
          device: { id: policy.deviceId, alias: policy.deviceAlias ?? null },
          authorized_project_count: policy.projects.length,
          labels_enabled: !!policy.labels,
          data_mode: policy.dataMode,
          sources: sourcesData(view),
          transport_scope: policy.transportScope,
          audience_label: policy.audienceLabel ?? null,
        },
      };
    },

    projects({ view, policy, t, args, limit }) {
      const windowStart = t - Math.min(WINDOW_MS, policy.lookbackMs ?? WINDOW_MS);
      const filters = {};
      const after = afterOf(args, policy, "projects", filters);
      const counts = new Map(policy.projects.map((p) => [p, emptyCounts()]));
      for (const v of visibleSessions(view, policy, t)) if (v.updated >= windowStart) counts.get(v.project)[v.status]++;
      const f = freshnessOf(view, t);
      const all = [...policy.projects].sort().map((p) => ({ project_id: p, label: policy.labels ? cleanPlace(view, policy, p) : null, session_counts: counts.get(p), observed_at: f.observedAt == null ? null : iso(f.observedAt) }));
      const { shown, next } = page(all, (x) => [x.project_id], after, limit);
      return { ...f, data: { window_start: iso(windowStart), projects: shown, next_cursor: nextOf(next, policy, "projects", filters) }, targets: [] };
    },

    sessions({ view, policy, t, args, limit }) {
      const project = ownProject(policy, args.project_id ?? null);
      const lookback = Math.min(policy.lookbackMs ?? LOOKBACK_MAX_MS, LOOKBACK_MAX_MS);
      const since = args.updated_since ? Date.parse(args.updated_since) : t - Math.min(WINDOW_MS, lookback);
      if (!Number.isFinite(since) || since > t || t - since > lookback) refuse("INVALID_ARGUMENT", "updated-since-out-of-range");
      const statuses = args.statuses ? new Set(args.statuses) : null;
      const filters = { project, statuses: args.statuses ? [...statuses].sort() : null, since: args.updated_since ?? null };
      const after = afterOf(args, policy, "sessions", filters);
      const list = visibleSessions(view, policy, t)
        .filter((v) => (!project || v.project === project) && (!statuses || statuses.has(v.status)) && v.updated >= since)
        .sort((a, b) => (b.s.updatedAt ?? 0) - (a.s.updatedAt ?? 0) || (a.s.id < b.s.id ? -1 : a.s.id > b.s.id ? 1 : 0));
      // Sorted newest first: the key is the updated time made negative.
      const { shown, next } = page(list, (v) => [-(v.s.updatedAt ?? 0), v.s.id], after, limit);
      const f = freshnessOf(view, t);
      return { ...f, data: { sessions: shown.map((v) => sessionSummary(v, policy)), next_cursor: nextOf(next, policy, "sessions", filters), window_start: iso(since) }, targets: project ? [policy.places.get(project)] : [] };
    },

    attention({ view, policy, t, args, limit }) {
      const project = ownProject(policy, args.project_id ?? null);
      const kinds = args.kinds ? new Set(args.kinds) : null;
      const filters = { project, kinds: args.kinds ? [...kinds].sort() : null };
      const after = afterOf(args, policy, "attention", filters);
      const items = attentionItems(view, policy, t, { idOf: attentionId })
        .filter((i) => (!project || i.project_id === project) && (!kinds || kinds.has(i.kind)))
        .map((i) => ({ i, key: [ATTENTION_KINDS.indexOf(i.kind), i.attention_id] }))
        .sort((x, y) => x.key[0] - y.key[0] || x.key[1].localeCompare(y.key[1]));
      // By kind (waiting first, as the contract lists them), then id.
      const { shown: found, next } = page(items, (x) => x.key, after, limit);
      const shown = found.map((x) => x.i);
      const f = freshnessOf(view, t);
      return { ...f, data: { items: shown, next_cursor: nextOf(next, policy, "attention", filters) }, targets: project ? [policy.places.get(project)] : [] };
    },

    async session({ view, policy, t, args, fixture }) {
      const v = visibleSessions(view, policy, t).find((x) => x.s.id === args.session_id) ?? refuse("NOT_FOUND", "session-not-visible");
      const f = freshnessOf(view, t, [v.s.app]);
      let stats = null;
      if (args.include_stats) {
        try {
          const raw = fixture ? fixtureStats(v.s.id) : await source.stats(v.s.id);
          stats = raw ? statsData(raw, now()) : null;
        } catch {
          stats = null;
        }
        if (!stats) f.warnings.push({ code: "STATS_UNAVAILABLE" });
      }
      return { ...f, data: { session: sessionSummary(v, policy), stats }, targets: [v.s.id] };
    },

    actors({ view, policy, t, args, limit }) {
      const project = ownProject(policy, args.project_id ?? null);
      const filters = { project };
      const after = afterOf(args, policy, "actors", filters);
      const list = actorSummaries(view, policy, t, { projectId: project })
        .map((a) => ({ a, key: [a.actor_id === policy.actorId ? 0 : 1, a.actor_id] }))
        .sort((x, y) => x.key[0] - y.key[0] || x.key[1].localeCompare(y.key[1]));
      const { shown, next } = page(list, (x) => x.key, after, limit);
      const f = freshnessOf(view, t);
      return { ...f, data: { self_actor_id: policy.actorId, actors: shown.map((x) => x.a), next_cursor: nextOf(next, policy, "actors", filters) }, targets: project ? [policy.places.get(project)] : [] };
    },
  };

  // A project's label: its place's name, cleaned (projection.js).
  function cleanPlace(view, policy, project) {
    return cleanLabel(view.places.get(policy.places.get(project))?.name ?? null);
  }

  // One operation, within its time, as an envelope, shortened until it
  // fits.
  async function answer(op, tool, args, policy0, requestId) {
    const { view, policy } = viewFor(policy0);
    let limit = limitOf(args);
    for (;;) {
      const t = now();
      const out = await ops[op]({ view, policy, t, args, limit, fixture: policy.dataMode === "fixture" });
      const warnings = [...out.warnings];
      if (view.fixture) warnings.push({ code: "FIXTURE_DATA" });
      if (limit < limitOf(args)) warnings.push({ code: "PAGE_REDUCED" });
      const envelope = {
        schema: RESULT_SCHEMA,
        request_id: requestId,
        device_id: policy.deviceId,
        grant_revision: policy.revision,
        observed_at: out.observedAt == null ? null : iso(out.observedAt),
        served_at: iso(now()),
        freshness: out.freshness,
        partial: !!out.partial,
        warnings: warnings.slice(0, 16),
        data: out.data,
      };
      const checked = RESULTS[tool].safeParse(envelope);
      if (!checked.success) refuse("INTERNAL_ERROR", "result-off-contract");
      const bytes = Buffer.byteLength(JSON.stringify(envelope));
      if (bytes <= limits.outputBytes) {
        const targets = view.fixture ? [] : (out.targets ?? []).filter(Boolean);
        // What it read, for the map's light: the place or session it named,
        // or every place its grant covers; nothing of the made-up data's.
        const reach = view.fixture ? [] : targets.length ? targets : [...policy.places.values()];
        return { envelope, bytes, targets, reach, projects: policy.projects.length };
      }
      if (limit <= 1) refuse("INTERNAL_ERROR", "result-too-big");
      limit = Math.max(1, Math.floor(limit / 2));
    }
  }

  const withTimeout = (promise, ms) => {
    let timer;
    return Promise.race([promise, new Promise((_, reject) => (timer = setTimeout(() => reject(new Refused("TIMEOUT", "timeout")), ms)))]).finally(() => clearTimeout(timer));
  };

  // assistant.read, for a session the endpoint let in: { grantId, actorId,
  // connectionId }. Answers { ok: true, result } or { ok: false, error },
  // each an envelope the adapter passes on as it is.
  async function read(session, request) {
    const started = now();
    const requestId = REQUEST_ID.test(String(request?.request_id)) ? request.request_id : null;
    const op = typeof request?.operation === "string" && Object.hasOwn(TOOL_OF, request.operation) ? request.operation : null;
    const log = (result, extra = {}) => audit?.access({ requestId, grant: session.grantId, revision: extra.revision ?? null, actor: session.actorId, connection: session.connectionId, operation: op ?? "unknown", result, latencyMs: now() - started, ...extra });
    const no = (code, reason, extra = {}) => {
      log(reason ?? code, extra);
      return { ok: false, error: errorEnvelope(requestId, code, extra) };
    };
    if (!request || typeof request !== "object" || Array.isArray(request) || Object.keys(request).some((k) => !["request_id", "operation", "arguments"].includes(k))) return no("INVALID_ARGUMENT", "bad-request");
    if (!requestId || !op) return no("INVALID_ARGUMENT", "bad-request");
    const args = request.arguments ?? {};
    if (Buffer.byteLength(JSON.stringify(args)) > limits.inputBytes) return no("INVALID_ARGUMENT", "too-big");
    let standing = connections.recheck(session.grantId, -1);
    if (!standing.ok) return no(standing.code, standing.reason);
    const wait = buckets.take(session.grantId, limits.rate);
    if (wait) return no("RATE_LIMITED", "rate", { retryAfterMs: wait });
    if ((inflight.get(session.grantId) ?? 0) >= limits.concurrent) return no("RATE_LIMITED", "concurrency", { retryAfterMs: 1000 });
    inflight.set(session.grantId, (inflight.get(session.grantId) ?? 0) + 1);
    // The slot is held until the work under it ends, even after a timeout
    // has answered: a read that times out keeps reading meanwhile.
    let work = Promise.resolve();
    try {
      const tool = TOOL_OF[op];
      const parsed = INPUTS[tool].safeParse(args);
      if (!parsed.success) return no("INVALID_ARGUMENT", "bad-arguments");
      for (let attempt = 0; attempt < 2; attempt++) {
        const policy = connections.policy(standing);
        const attempt = answer(op, tool, parsed.data, policy, requestId);
        work = attempt.catch(() => {});
        const out = await withTimeout(attempt, limits.timeoutMs);
        // Once more before it goes: a grant revoked, expired or turned off
        // meanwhile gets nothing, and one whose scope changed reads again.
        const again = connections.recheck(session.grantId, policy.revision);
        if (!again.ok) return no(again.code, again.reason, { revision: policy.revision });
        if (again.changed) {
          standing = again;
          continue;
        }
        connections.used(session.grantId);
        remote?.read(session.actorId, session.connectionId, { targets: out.targets, reach: out.reach });
        log("ok", { revision: policy.revision, bytes: out.bytes, projects: out.projects });
        return { ok: true, result: out.envelope };
      }
      return no("SERVICE_UNAVAILABLE", "scope-kept-changing");
    } catch (err) {
      if (err instanceof Refused) return no(err.code, err.reason);
      return no("INTERNAL_ERROR", "exception");
    } finally {
      work.then(() => {
        const n = (inflight.get(session.grantId) ?? 1) - 1;
        if (n) inflight.set(session.grantId, n);
        else inflight.delete(session.grantId);
      });
    }
  }

  return { read, forget: (grantId) => buckets.forget(grantId) };
}


// The service `xenonet mcp assistant --fixture` runs by itself, with no
// XenoNet, no grant and nothing of this computer's: the made-up data only,
// through the same checks, for trying a client or a tunnel first.
export function createFixtureService({ version = "0.0.0", labels = true, now = Date.now } = {}) {
  const policy = standalonePolicy({ labels });
  const standing = { ok: true, revision: policy.revision, changed: false };
  const connections = { recheck: () => standing, policy: () => policy, used: () => {}, idKey: () => "fixture" };
  return createAssistantService({ connections, source: { view: () => fixtureView(now()), stats: async (id) => fixtureStats(id), actor: () => null }, version, now });
}
