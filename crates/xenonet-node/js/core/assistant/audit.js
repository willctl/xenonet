// core/assistant/audit.js in Rust (crates/xenonet-assistant/src/audit.rs):
// the log of what remote actors read. What a record holds is the Rust's;
// the store is the engine's (core/engine/store.js).
import { native } from "../../index.js";

export function createAudit({ store, now = Date.now }) {
  return {
    access(a) {
      try {
        store.addAudit(native.asstAccess(a, now()));
      } catch {}
    },
    recent: ({ since = now() - 7 * 24 * 3600e3, limit = 100, grant = null } = {}) => native.asstAuditRecent(store.audit({ since, limit: native.asstAuditLimit(Number(limit)), grant })),
  };
}
