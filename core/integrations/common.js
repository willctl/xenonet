import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";

export class IntegrationError extends Error {
  constructor(status, code, message) { super(message); Object.assign(this, { status, code }); }
}
export const fail = (message, code = "bad-request", status = 400) => { throw new IntegrationError(status, code, message); };
export const clean = (value, max = 160) => String(value ?? "").replace(/sk-[\w-]{16,}|Bearer\s+\S+|(?:password|api[_-]?key|token|secret)\s*[=:]\s*[^\s,;]+/gi, "[redacted]").replace(/[\x00-\x1f\x7f]/g, " ").slice(0, max);
export const text = (value, name, max = 2000) => {
  if (typeof value !== "string" || !value.trim() || value.length > max || /\0/.test(value)) fail(`${name} must be text, at most ${max} characters.`);
  return value.trim();
};
// A job with no sandbox behind it: deleted, or never made because the
// command wasn't there. Only the others count toward the limit and survive retention.
export const sandboxGone = (r) => r.status === "deleted" || r.status === "not-created";

// Tickets contain reviewed inputs only in memory, expire, and can be used once.
export function createTickets({ now = Date.now } = {}) {
  const tickets = new Map();
  function check() { for (const [id, t] of tickets) if (t.until <= now()) tickets.delete(id); if (tickets.size >= 32) fail("Close an old preview before making another.", "busy", 429); }
  return {
    check,
    issue(value) {
      check();
      const ticket = randomUUID(), expires = now() + 10 * 60e3;
      tickets.set(ticket, { value, until: expires });
      return { ticket, expires };
    },
    take(id) {
      const t = tickets.get(id); tickets.delete(id);
      if (!t || t.until <= now()) fail("This preview expired or was already used. Preview again.", "expired", 409);
      return t.value;
    },
  };
}

export function createRecords(dir, { now = Date.now, limit = 1000 } = {}) {
  const memory = new Map();
  if (dir) fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  function list() {
    if (!dir) return [...memory.values()].sort((a, b) => b.at - a.at || a.id.localeCompare(b.id));
    return fs.readdirSync(dir).filter((n) => /^[\da-f-]{36}\.json$/.test(n)).flatMap((name) => {
      try { const file = path.join(dir, name); const st = fs.lstatSync(file); if (!st.isFile() || st.size > 64 * 1024) return []; const record = JSON.parse(fs.readFileSync(file, "utf8")); if (record?.id !== name.slice(0, -5) || !Number.isFinite(record.at)) return []; return [record]; } catch { return []; }
    }).sort((a, b) => b.at - a.at || a.id.localeCompare(b.id));
  }
  function save(record) {
    if (dir) {
      const tmp = path.join(dir, `${record.id}.${randomUUID()}.tmp`);
      try { fs.writeFileSync(tmp, JSON.stringify(record), { mode: 0o600, flag: "wx" }); fs.renameSync(tmp, path.join(dir, `${record.id}.json`)); }
      finally { try { fs.unlinkSync(tmp); } catch {} }
    } else memory.set(record.id, structuredClone(record));
    return record;
  }
  return {
    list,
    add(value) {
      const record = save({ ...value, id: randomUUID(), at: now() });
      // A saved live job is the owner's way to inspect or delete its remote
      // sandbox. Retention must not orphan it as newer decisions arrive.
      const completed = list().filter((r) => !r.reviewId && (r.kind !== "sandbox" || sandboxGone(r)));
      for (const old of completed.slice(limit)) { if (dir) fs.unlinkSync(path.join(dir, `${old.id}.json`)); else memory.delete(old.id); }
      return record;
    },
    update(id, fields) {
      const record = list().find((r) => r.id === id);
      if (!record) fail("No such record.", "missing", 404);
      return save({ ...record, ...fields, id: record.id, at: record.at });
    },
  };
}

export function queryRecords(records, { q = "", kind = "", status = "", sort = "newest", page = 1 } = {}) {
  const words = String(q).toLowerCase().split(/\s+/).filter(Boolean);
  records = records.filter((r) => !kind || r.kind === kind);
  const found = records.filter((r) => (!status || r.status === status) && words.every((w) => JSON.stringify(r).toLowerCase().includes(w)));
  found.sort(sort === "oldest" ? (a, b) => a.at - b.at || a.id.localeCompare(b.id) : (a, b) => b.at - a.at || a.id.localeCompare(b.id));
  page = Math.max(1, Math.min(Math.ceil(found.length / 50) || 1, Math.floor(Number(page) || 1)));
  return { total: records.length, matched: found.length, page, pages: Math.ceil(found.length / 50), records: found.slice((page - 1) * 50, page * 50) };
}
