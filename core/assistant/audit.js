// The log of what remote actors read (docs/design/cloud-assistants.md,
// section 6), kept in engine.sqlite for seven days and at most 20,000
// records (core/engine/store.js, AUDIT_KEPT): when, which request, grant,
// revision, actor and connection, which operation, a result code, how long
// it took, how many bytes went and how many projects it could see. Never a
// title, a name, an argument, a credential, a path or an account. Your own
// changes (pairing, revoking, turning access off) are kept beside them,
// marked as yours (connections.js).

const CODE = /^[A-Za-z_-]{1,40}$/;
const OPERATIONS = new Set(["status", "projects", "sessions", "attention", "session", "actors", "hello", "unknown"]);

export function createAudit({ store, now = Date.now }) {
  return {
    // A read, or a refusal, through a grant.
    access({ requestId = null, grant = null, revision = null, actor = null, connection = null, operation, result, latencyMs = null, bytes = null, projects = null }) {
      try {
        store.addAudit({
          at: now(),
          kind: "access",
          request_id: requestId,
          grant_id: grant,
          grant_revision: Number.isInteger(revision) ? revision : null,
          actor,
          connection_id: connection,
          operation: OPERATIONS.has(operation) ? operation : "unknown",
          result: CODE.test(String(result)) ? String(result) : "unknown",
          latency_ms: Number.isFinite(latencyMs) ? Math.round(latencyMs) : null,
          bytes: Number.isFinite(bytes) ? bytes : null,
          projects: Number.isInteger(projects) ? projects : null,
        });
      } catch {}
    },
    // The newest records, for your view and xenonet assistant audit, without
    // a database's own ids.
    recent: ({ since = now() - 7 * 24 * 3600e3, limit = 100, grant = null } = {}) =>
      store.audit({ since, limit: Math.min(Math.max(1, limit), 1000), grant }).map(({ id, ...r }) => ({ ...r, at: new Date(r.at).toISOString() })),
  };
}
