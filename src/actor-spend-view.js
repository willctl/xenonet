// What Token use shows of each actor's spend today (GET /api/spend's
// `actors`, crates/xenonet-actors/src/spend.rs): the top few actors, the rest folded
// under "more" until it's opened, and the unattributed rest last, each
// with its share of the day and what it covers on hover. Plain data in
// and out, so the panel's words can be tested without a page.
import { plural, tokens } from "./shared/units.js";

export const TOP = 3;

const dollars = (v) => "$" + (Math.abs(v) >= 100 ? Math.round(v).toLocaleString() : v.toFixed(2));

// Null when there's nothing to say: no answer (a XenoNet from before it),
// or no actor whose work ran today. Otherwise { rows, more, total }, each
// row { id, name, value, share (0 to 1), reported, title, unattributed };
// `more` is how many actors are folded away (0 once `expanded`).
// `reported` is what Hermes or OpenCode say they spent today, a figure of
// its own that is never in `value` or the share: "$0.50 reported" (or
// "12k tokens reported" when none of it was priced), else null.
// "$0.50" (or "12k tokens", when none of it was priced) for a reported figure, or null without one.
const reportedText = (rep, money) => {
  if (!rep) return null;
  return rep.usd != null ? money(rep.usd) : `${tokens(rep.tokens)} tokens`;
};

export function actorSpendView(answer, { expanded = false, top = TOP, money = dollars } = {}) {
  const list = answer?.actors;
  if (!Array.isArray(list) || !list.length || !answer.unattributed) return null;
  const total = Number.isFinite(answer.total) ? answer.total : 0;
  const shareOf = (cost) => (total > 0 ? Math.max(0, Math.min(1, cost / total)) : 0);
  const noPrice = (r) => (r.unpriced?.length ? ` Not counted, with no price: ${r.unpriced.join(", ")}.` : "");
  const reportedTitle = (r) => (r.reported ? ` ${r.reported.source} reports ${r.reported.usd != null ? `${money(r.reported.usd)} and ` : ""}${tokens(r.reported.tokens)} tokens today, its own figure, not in the estimate${r.reported.usd == null ? " (no price reported)" : ""}.` : "");
  const row = (r) => {
    const parts = [];
    if (r.sessions) parts.push(`About ${money(r.cost)} today over ${plural(r.sessions, "session")} XenoNet ties to it.`);
    if (r.elsewhere) parts.push(`${plural(r.elsewhere, "run")} it reported from elsewhere today ${r.elsewhere === 1 ? "isn't" : "aren't"} counted: XenoNet can't see ${r.elsewhere === 1 ? "its" : "their"} calls.`);
    return { id: r.id, name: r.name, value: r.sessions ? money(r.cost) : "—", share: shareOf(r.cost), reported: reportedText(r.reported, money), title: `${r.name}: ${parts.join(" ")}${noPrice(r)}${reportedTitle(r)}`, unattributed: false };
  };
  const shown = expanded ? list : list.slice(0, top);
  const u = answer.unattributed;
  const rest = {
    id: "unattributed",
    name: "unattributed",
    value: money(u.cost),
    share: shareOf(u.cost),
    reported: reportedText(u.reported, money),
    title: `About ${money(u.cost)} today over ${plural(u.sessions, "session")} not tied to an actor, such as the ones you start: XenoNet doesn't know which session you started.${noPrice(u)}${reportedTitle(u)}`,
    unattributed: true,
  };
  return { rows: [...shown.map(row), rest], more: list.length - shown.length, total };
}
