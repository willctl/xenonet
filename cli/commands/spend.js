// xenonet spend: about what today's work would cost at API prices, by app
// and by actor, as Token use in the console says it (core/live.js costs,
// core/actors/spend.js).
import { c, print, json, table, plural } from "../lib/out.js";
import { need, get } from "../../core/client.js";
import { money } from "../../core/units.js";

export const help = `About what today's work would cost at API prices: for Claude Code and for
Codex, at this pace, and by actor. Each session counts for one actor at
most: the one that reported it as a run, or started it, or started the
session it came from. The rest is unattributed, including the sessions you
start yourself, since XenoNet doesn't know which ones those are.

On a plan, the work costs the plan; this is what it would cost on the API.

  --json   the day's costs and the breakdown by actor, as JSON`;

export async function run(_args, flags) {
  const s = await need(flags);
  const [live, byActor] = await Promise.all([
    get(s, "/api/live").catch(() => null),
    // A XenoNet from before spend by actor, or with actors off, has none.
    get(s, "/api/spend/actors").catch((err) => (err.status === 404 ? null : Promise.reject(err))),
  ]);
  const costs = live?.costs ?? null;
  if (flags.json) return json({ costs, byActor });
  if (!costs && !byActor) return print(c.dim("This XenoNet doesn't say what the day's work would cost."));

  if (costs) {
    print(`About ${c.bold(money(costs.today))} today at API prices${costs.perHour > 0 ? `, ${money(costs.perHour)} an hour at this pace` : ""}`);
    print(table([[c.dim("Claude Code"), money(costs.byApp.claude)], [c.dim("Codex"), money(costs.byApp.codex)]], { indent: 2 }));
  } else if (byActor) print(`About ${c.bold(money(byActor.total))} today at API prices`);

  if (byActor) {
    print(`\n${c.bold("By actor")}`);
    const rows = byActor.actors.map((r) => [
      r.name,
      r.sessions ? money(r.cost) : c.dim("—"),
      c.dim([r.sessions && plural(r.sessions, "session"), r.elsewhere && `${plural(r.elsewhere, "run")} elsewhere, not priced`].filter(Boolean).join(", ")),
    ]);
    const u = byActor.unattributed;
    rows.push([c.dim("Unattributed"), money(u.cost), c.dim(`${plural(u.sessions, "session")} not tied to an actor, such as the ones you start`)]);
    print(table(rows, { indent: 2, wrap: true }));
    if (!byActor.actors.length) print(c.dim("  No actor started a session that ran today."));
  } else print(c.dim("\nThis XenoNet doesn't break the day down by actor."));

  const unpriced = [...new Set([...(costs?.unpriced ?? []), ...(byActor?.unattributed.unpriced ?? []), ...(byActor?.actors ?? []).flatMap((r) => r.unpriced)])].sort();
  const checked = costs?.checked ?? byActor?.checked;
  print(c.dim(`\n${unpriced.length ? `Not counted, with no price: ${unpriced.join(", ")}. ` : ""}Prices checked ${checked}. On a plan, the work costs the plan.`));
}
