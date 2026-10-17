// core/integrations/common.js in Rust (crates/xenonet-integrations/src/
// common.rs): credentials cleaned out of text, previews' tickets, and the
// records of decisions and sandbox jobs with their retention and queries.
import { native } from "../../index.js";

export class IntegrationError extends Error {
  constructor(status, code, message) {
    super(message);
    Object.assign(this, { status, code });
  }
}
export const fail = (message, code = "bad-request", status = 400) => {
  throw new IntegrationError(status, code, message);
};

// The Rust's error as the JS threw it: an IntegrationError, or a system
// error with its code.
export const errorOf = (e) => (e.status ? new IntegrationError(e.status, e.code, e.message) : Object.assign(new Error(e.message), e.code ? { code: e.code } : {}));
export function settle(r) {
  if (r.error) throw errorOf(r.error);
  return r.ok;
}
// A Rust task's refusal, as an IntegrationError.
const integration = (err) => (err?.integration ? new IntegrationError(err.status, err.code, err.message) : err);
export async function driven(promise) {
  try {
    return await promise;
  } catch (err) {
    throw integration(err);
  }
}
export function drivenSync(fn) {
  try {
    return fn();
  } catch (err) {
    throw integration(err);
  }
}

export const clean = (value, max = 160) => native.intgClean(value, Number(max));
export const text = (value, name, max = 2000) => settle(native.intgText(value, String(name), Number(max)));
export const sandboxGone = (r) => native.intgSandboxGone(r);

export function createTickets({ now = Date.now } = {}) {
  const core = new native.IntgTickets();
  return {
    check: () => void settle(core.check(now())),
    issue: (value) => settle(core.issue(value, now())),
    take: (id) => settle(core.take(String(id), now())),
  };
}

export function createRecords(dir, { now = Date.now, limit = 1000 } = {}) {
  const core = native.IntgRecords.of(dir ? String(dir) : null, Number(limit));
  return {
    list: () => settle(core.list()),
    add: (value) => settle(core.add(value, now())),
    update: (id, fields) => settle(core.update(id, fields ?? {})),
  };
}

export const queryRecords = (records, params = {}) => native.intgQuery(records, params);
