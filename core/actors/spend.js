// What each actor's work costs today, at API prices: Token use's spend by
// actor, GET /api/spend and xenonet spend. Each session's calls today
// (core/live.js sessionCosts, priced by core/pricing.js) go to at most one
// actor, so nothing is counted twice and the parts add up to the day's
// estimate. A session is an actor's when XenoNet knows the actor started
// it:
//
//   1. A run an actor reported through the actor gateway
//      (core/reported-runs.js) is that actor's. Its id is made from the
//      actor's own token, so no other actor can name it.
//   2. Else a session an actor started, as its spawned events say: traced
//      back to it through its processes (attribution.js), or in its own
//      report. When more than one actor says so, what XenoNet saw itself
//      (observed, attributed) comes before what an actor said of itself
//      (registered), then the earliest, then the actor's id.
//   3. Else a subagent's, or a session another session started, goes with
//      the session it came from (its parentId, from the transcripts), by
//      the same rules, up to 8 sessions up. A fork doesn't: whoever forked
//      a session may not be whoever started it.
//   4. Else no actor's: unattributed. That includes the sessions you start
//      yourself. The window says you started one in a place, never which
//      one (you.js), and XenoNet doesn't guess from timing.
//
// A run reported from elsewhere has no transcript here, so XenoNet can't
// price it: it's counted apart, as one of the actor's runs elsewhere.
import { PRICES_CHECKED } from "../pricing.js";

export const UNATTRIBUTED = "unattributed";
const LINEAGE_MAX = 8;
// The parents whose children are their own work: a subagent, and a
// session one session's process started.
const INHERITS = new Set(["subagent", "spawn"]);
// What XenoNet saw for itself, rather than what an actor said of itself.
const SEEN = new Set(["observed", "attributed"]);

// Each session's actor by the rule: Map(session id → { actor, how }), with
// how "reported" (1), "started" (2) or "parent" (3), for the sessions that
// have one. `sessions`: the map's sessions ({ id, reportedBy, parentId,
// parentKind }); `starts`: who started what ({ actor, session, at, via });
// `known`: the actors that count (a Set of ids), or every one when null.
// `ids`: the sessions to answer for, the map's when left out.
export function attribute({ sessions = [], starts = [], known = null, ids = null } = {}) {
  const byId = new Map();
  for (const s of sessions) if (s?.id) byId.set(s.id, s);
  const counts = (actor) => typeof actor === "string" && (!known || known.has(actor));
  const best = new Map();
  for (const st of starts) {
    if (!counts(st?.actor) || typeof st.session !== "string") continue;
    const rank = SEEN.has(st.via) ? 2 : 1;
    const at = Number.isFinite(st.at) ? st.at : Infinity;
    const b = best.get(st.session);
    if (!b || rank > b.rank || (rank === b.rank && (at < b.at || (at === b.at && st.actor < b.actor)))) best.set(st.session, { actor: st.actor, rank, at });
  }
  const own = (id) => {
    const s = byId.get(id);
    if (counts(s?.reportedBy)) return { actor: s.reportedBy, how: "reported" };
    const b = best.get(id);
    return b ? { actor: b.actor, how: "started" } : null;
  };
  const out = new Map();
  for (const id of ids ?? byId.keys()) {
    const seen = new Set();
    let cur = id;
    for (let up = 0; cur && up <= LINEAGE_MAX && !seen.has(cur); up++) {
      seen.add(cur);
      const o = own(cur);
      if (o) {
        out.set(id, up ? { actor: o.actor, how: "parent" } : o);
        break;
      }
      const s = byId.get(cur);
      cur = s && INHERITS.has(s.parentKind) && typeof s.parentId === "string" ? s.parentId : null;
    }
  }
  return out;
}

// Today's spend by actor: { total, actors, unattributed }. `spend`: each
// session's calls today ({ id, cost, tokens, unpriced }); `names`: the
// actors that count, each one's name by its id (a Map or an object);
// `since`: when today began, for the runs reported from elsewhere today.
// Each actor: { id, name, cost, tokens, sessions, unpriced, elsewhere },
// the most spent first; tokens are the priced calls' and unpriced names
// the models without a price, as Token use has them. `elsewhere` counts
// its runs reported from elsewhere today, which aren't in its cost.
export function spendByActor({ spend = [], sessions = [], starts = [], names = new Map(), since = null } = {}) {
  const known = names instanceof Map ? names : new Map(Object.entries(names ?? {}));
  const list = [...spend].filter((s) => typeof s?.id === "string" && Number.isFinite(s.cost));
  // Only the sessions that count today: those with calls, and the runs
  // reported from elsewhere. The map can hold thousands of older ones.
  const runs = sessions.filter((s) => s?.observed === "gateway" && typeof s.id === "string");
  const who = attribute({ sessions, starts, known: new Set(known.keys()), ids: [...list.map((s) => s.id), ...runs.map((s) => s.id)] });
  const part = () => ({ cost: 0, tokens: 0, sessions: 0, unpriced: new Set() });
  const rows = new Map();
  const row = (id) => rows.get(id) ?? rows.set(id, { id, name: known.get(id) ?? id, ...part(), elsewhere: 0 }).get(id);
  const none = part();
  const counted = new Set();
  let total = 0;
  for (const s of list) {
    if (counted.has(s.id)) continue;
    counted.add(s.id);
    const w = who.get(s.id);
    const r = w ? row(w.actor) : none;
    r.cost += s.cost;
    r.tokens += Number.isFinite(s.tokens) ? s.tokens : 0;
    r.sessions++;
    for (const m of s.unpriced ?? []) r.unpriced.add(m);
    total += s.cost;
  }
  for (const s of runs) {
    if (counted.has(s.id) || (since != null && !(s.updatedAt >= since))) continue;
    const w = who.get(s.id);
    if (w) row(w.actor).elsewhere++;
  }
  const plain = ({ unpriced, ...r }) => ({ ...r, unpriced: [...unpriced].sort() });
  return {
    total,
    actors: [...rows.values()].map(plain).sort((a, b) => b.cost - a.cost || b.sessions - a.sessions || b.elsewhere - a.elsewhere || String(a.name).localeCompare(String(b.name)) || a.id.localeCompare(b.id)),
    unattributed: plain(none),
  };
}

// The start of the local day a time falls on, as core/live.js keeps days.
export const dayStart = (now) => {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

// Today's spend by actor on this XenoNet, as GET /api/spend answers it:
// the live tailer's costs (`live`), the map's nodes (`nodes`) and the
// actors (`actors`, the registry). { at, since, checked, total, actors,
// unattributed }, or null without actors.
export function actorSpend({ live, nodes, actors, now = Date.now() }) {
  if (!live || !actors) return null;
  const { names, starts } = actors.sessionsStarted({ now });
  const sessions = [];
  for (const n of nodes ?? []) if (n.kind === "session") sessions.push(n);
  const since = dayStart(now);
  return { at: now, since, checked: PRICES_CHECKED, ...spendByActor({ spend: live.sessionCosts(now), sessions, starts, names, since }) };
}
