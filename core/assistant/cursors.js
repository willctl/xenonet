// Page tokens for the assistant's lists (docs/design/cloud-assistants.md,
// section 5): opaque to the caller, signed so they can't be changed or
// made, and good for five minutes. Each is bound to the grant, its
// revision, the tool and its filters, and holds where the last page ended
// (the sort key of its last item), never a path or a title. Pages are
// read from the view as it is when each is asked for: an item that changes
// between pages can move past the end of one, or show on two.
//
// The key is made when the service starts and never written down, so a
// restart ends every token, as a revocation or a change of scope does
// (the revision moves on).
import crypto from "node:crypto";

export const CURSOR_MS = 5 * 60e3;

const digest = (value) => crypto.createHash("sha256").update(JSON.stringify(value ?? null)).digest("base64url").slice(0, 16);

export function createCursors({ now = Date.now, key = crypto.randomBytes(32) } = {}) {
  const sign = (payload) => crypto.createHmac("sha256", key).update(payload).digest("base64url");
  return {
    // A token for the page after `after`.
    make({ grant, revision, op, filters, after }) {
      const payload = Buffer.from(JSON.stringify({ v: 1, g: grant, r: revision, o: op, f: digest(filters), a: after, t: now() })).toString("base64url");
      return `${payload}.${sign(payload)}`;
    },
    // Where a token says to go on from: { ok, after }, or { ok: false,
    // code }: INVALID_ARGUMENT for one that was changed, made up, or is for
    // another grant, tool or filters; CURSOR_EXPIRED for one too old or from
    // before the grant changed.
    read(cursor, { grant, revision, op, filters }) {
      const bad = { ok: false, code: "INVALID_ARGUMENT" };
      const [payload, mac] = String(cursor).split(".");
      if (!payload || !mac) return bad;
      const ours = Buffer.from(sign(payload));
      const theirs = Buffer.from(mac);
      if (ours.length !== theirs.length || !crypto.timingSafeEqual(ours, theirs)) return bad;
      let p;
      try {
        p = JSON.parse(Buffer.from(payload, "base64url").toString("utf8"));
      } catch {
        return bad;
      }
      if (p?.v !== 1 || p.g !== grant || p.o !== op || p.f !== digest(filters)) return bad;
      if (!Number.isFinite(p.t) || now() - p.t > CURSOR_MS || p.t > now() + 1000 || p.r !== revision) return { ok: false, code: "CURSOR_EXPIRED" };
      return { ok: true, after: p.a };
    },
  };
}
