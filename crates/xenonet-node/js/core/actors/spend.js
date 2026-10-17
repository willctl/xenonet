// core/actors/spend.js in Rust (crates/xenonet-actors/src/spend.rs): what
// each actor's work costs today. Which session is whose, and the sums, are
// Rust's; the local day's start (the system's time zone) and the gathering
// of the live costs and the map's sessions stay here.
import { PRICES_CHECKED } from "../pricing.js";
import { native } from "../../index.js";

export const UNATTRIBUTED = native.spendUnattributed();

export function attribute({ sessions = [], starts = [], known = null, ids = null } = {}) {
  return new Map(native.spendAttribute([...sessions], [...starts], known ? [...known] : null, ids ? [...ids] : null));
}

// Sets of models, as lists.
const listed = (s) => (s && typeof s === "object" && s.unpriced != null && !Array.isArray(s.unpriced) ? { ...s, unpriced: [...s.unpriced] } : s);

export function spendByActor({ spend = [], sessions = [], starts = [], names = new Map(), since = null } = {}) {
  const pairs = names instanceof Map ? [...names] : Object.entries(names ?? {});
  return native.spendByActor([...spend].map(listed), sessions, starts, pairs, since);
}

export const dayStart = (now) => {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

export function actorSpend({ live, nodes, actors, now = Date.now() }) {
  if (!live || !actors) return null;
  const { names, starts } = actors.sessionsStarted({ now });
  const sessions = [];
  for (const n of nodes ?? []) if (n.kind === "session") sessions.push(n);
  const since = dayStart(now);
  return { at: now, since, checked: PRICES_CHECKED, ...spendByActor({ spend: live.sessionCosts(now), sessions, starts, names, since }) };
}
