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

export function spendByActor({ spend = [], sessions = [], starts = [], names = new Map(), since = null, reported = [] } = {}) {
  const pairs = names instanceof Map ? [...names] : Object.entries(names ?? {});
  return native.spendByActor([...spend].map(listed), sessions, starts, pairs, since, [...(reported ?? [])]);
}

export const dayStart = (now) => {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
};

// What Hermes and OpenCode report as spent today, as the core has it.
export function reportedToday({ state = null, since, usages = [], sessions = [] } = {}) {
  const [next, items] = native.spendReportedToday(state, since, usages, sessions);
  return { state: next, items };
}

export function actorSpend({ live, nodes, actors, now = Date.now(), meter = {} }) {
  if (!live || !actors) return null;
  const { names, starts } = actors.sessionsStarted({ now });
  const sessions = [];
  for (const n of nodes ?? []) if (n.kind === "session") sessions.push(n);
  const since = dayStart(now);
  const usages = [];
  for (const id of names.keys()) {
    const snap = actors.snapshot(id, { now });
    if (snap?.usage) usages.push({ id, product: snap.product, usage: snap.usage });
  }
  const { state, items } = reportedToday({ state: meter.state, since, usages, sessions });
  meter.state = state;
  return { at: now, since, checked: PRICES_CHECKED, ...spendByActor({ spend: live.sessionCosts(now), sessions, starts, names, since, reported: items }) };
}
