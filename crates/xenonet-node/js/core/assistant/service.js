// core/assistant/service.js in Rust (crates/xenonet-assistant/src/
// service.rs): the assistant's reads. The request's checks, the six
// operations, paging, the answer's shape and size and the four-at-a-time
// limit are the Rust's, each answer a task this drives. The grant's
// standing (connections.js), its rate (actors/limits.js), the schemas'
// checks, the source, the audit and the timeout are asked or kept here.
import { native } from "../../index.js";
import { peer } from "../../peer.js";
import { drive } from "../../task.js";
import { fixtureView, fixtureStats, standalonePolicy } from "./fixtures.js";
import { createCursors, CORE } from "./cursors.js";

const { createBuckets } = await import(peer("actors/limits.js"));
const { INPUTS, RESULTS, OPERATIONS, RESULT_SCHEMA, ERROR_SCHEMA, ATTENTION_KINDS } = await import(peer("schema/assistant.js"));

const C = native.asstConstants();

export const LIMITS = { ...C.limits, rate: { rate: 1, burst: 10 } };
export const MESSAGES = C.messages;

export const errorEnvelope = (requestId, code, { retryAfterMs } = {}) => native.asstErrorEnvelope(ERROR_SCHEMA, requestId, String(code), Number.isFinite(retryAfterMs) ? retryAfterMs : null);

const plain = (v) => (v?.places instanceof Map ? { ...v, places: [...v.places] } : v);

export function createAssistantService({ connections, source, remote = null, audit = null, version = "0.0.0", now = Date.now, limits = LIMITS, cursors = createCursors({ now }), idKey = () => connections.idKey() }) {
  const own = cursors?.[CORE];
  if (!own) throw new TypeError("the service's page tokens come from createCursors (core/assistant/cursors.js)");
  const buckets = createBuckets({ now });
  const core = new native.AsstService({ operations: Object.entries(OPERATIONS), attentionKinds: ATTENTION_KINDS, resultSchema: RESULT_SCHEMA, errorSchema: ERROR_SCHEMA, version: String(version), limits }, own.core);

  // What an answer asks for.
  const perform = (q) => {
    switch (q.op) {
      case "now":
        return now();
      case "cursorNow":
        return own.now();
      case "view":
        return plain(source.view());
      case "actor":
        return source.actor?.(q.id) ?? null;
      case "stats":
        return source.stats(q.id);
      case "idKey": {
        const k = idKey();
        return [...(Buffer.isBuffer(k) ? k : Buffer.from(String(k)))];
      }
      case "check":
        return RESULTS[q.tool].safeParse(q.envelope).success;
      default:
        throw new Error(`no such request: ${q.op}`);
    }
  };
  const answer = (op, tool, args, policy, requestId) => drive(core.answer(op, tool, args, plain(policy), requestId), perform);

  const withTimeout = (promise, ms) => {
    let timer;
    return Promise.race([promise, new Promise((_, reject) => (timer = setTimeout(() => reject(Object.assign(new Error("TIMEOUT"), { code: "TIMEOUT", reason: "timeout", refused: true })), ms)))]).finally(() => clearTimeout(timer));
  };

  async function read(session, request) {
    const started = now();
    const b = core.begin(request);
    const { requestId, op } = b;
    const log = (result, extra = {}) => audit?.access({ requestId, grant: session.grantId, revision: extra.revision ?? null, actor: session.actorId, connection: session.connectionId, operation: op ?? "unknown", result, latencyMs: now() - started, ...extra });
    const no = (code, reason, extra = {}) => {
      log(reason ?? code, extra);
      return { ok: false, error: errorEnvelope(requestId, code, extra) };
    };
    if (b.no) return no(b.no.code, b.no.reason);
    let standing = connections.recheck(session.grantId, -1);
    if (!standing.ok) return no(standing.code, standing.reason);
    const wait = buckets.take(session.grantId, limits.rate);
    if (wait) return no("RATE_LIMITED", "rate", { retryAfterMs: wait });
    const grant = String(session.grantId);
    if (!core.enter(grant)) return no("RATE_LIMITED", "concurrency", { retryAfterMs: 1000 });
    // The slot is held until the work under it ends, even after a timeout
    // has answered.
    let work = Promise.resolve();
    try {
      const parsed = INPUTS[b.tool].safeParse(b.args);
      if (!parsed.success) return no("INVALID_ARGUMENT", "bad-arguments");
      for (let attempt = 0; attempt < 2; attempt++) {
        const policy = connections.policy(standing);
        const running = answer(op, b.tool, parsed.data, policy, requestId);
        work = running.catch(() => {});
        const out = await withTimeout(running, limits.timeoutMs);
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
      if (err?.refused) return no(err.code, err.reason);
      return no("INTERNAL_ERROR", "exception");
    } finally {
      work.then(() => core.leave(grant));
    }
  }

  return { read, forget: (grantId) => buckets.forget(grantId) };
}

export function createFixtureService({ version = "0.0.0", labels = true, now = Date.now } = {}) {
  const policy = standalonePolicy({ labels });
  const standing = { ok: true, revision: policy.revision, changed: false };
  const connections = { recheck: () => standing, policy: () => policy, used: () => {}, idKey: () => "fixture" };
  return createAssistantService({ connections, source: { view: () => fixtureView(now()), stats: async (id) => fixtureStats(id), actor: () => null }, version, now });
}
