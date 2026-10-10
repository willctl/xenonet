// xenonet session: one session's models, tokens, estimated cost, turns and
// tools (core/stats.js), the same as the console's session details.
import os from "node:os";
import { c, print, json, table, plural, clip, width, Fail } from "../lib/out.js";
import { need, get } from "../../core/client.js";
import { tilde } from "../../core/paths.js";
import { money, tokens, duration, share } from "../../core/units.js";

export const help = `A session's models and when it used them, its tokens and what they'd cost
at each model's API price on the day, its turns and its tools. Without
words, the latest session in this folder; with them, the newest session
whose id starts with them or whose title has them all.

  xenonet session                  the latest session here
  xenonet session refunds api      the newest one with both words in its title
  xenonet session 9eddb031         by id
  --json                         everything, as JSON`;

const clock = (t) => new Date(t).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

export async function run(words, flags) {
  const s = await need(flags);
  const q = words.join(" ").trim();
  const found = await get(s, `/api/sessions?limit=5&${q ? `q=${encodeURIComponent(q)}` : `cwd=${encodeURIComponent(process.cwd())}`}`);
  if (!found.length) throw new Fail(q ? `no session matches "${q}"` : "no session ran in this folder; name one: xenonet session <words or id>");
  const st = await get(s, `/api/session/${encodeURIComponent(found[0].id)}/stats`);
  if (flags.json) return json(st);
  const home = os.homedir();
  const t = st.totals;

  print(c.bold(st.title ?? st.id));
  const span = st.from && st.to ? `${duration(st.to - st.from)}, ${duration(st.activeMs)} at work` : null;
  print(c.dim([st.app, st.cwd && tilde(st.cwd, home), st.branch, span, plural(st.turns.length, "turn")].filter(Boolean).join(" · ")));
  if (found.length > 1 && q) print(c.dim(`(${plural(found.length - 1, "other session")} also match; xenonet session <id> picks one)`));
  print();
  print(table([
    [c.dim("Estimated cost"), `${c.bold(money(t.cost))} ${c.dim(`at API prices on the day, standard tier${st.pricing.unpriced.length ? `; no price for ${st.pricing.unpriced.join(", ")}` : ""}`)}`],
    [c.dim("Tokens"), `${c.bold(tokens(t.tokens))} ${c.dim(`in ${plural(t.calls, "call")}; ${share(t.cacheRead, t.tokens)} read from cache`)}`],
  ], { indent: 2, wrap: true }));

  print(`\n${c.bold("Models")}`);
  print(table(st.models.map((m) => [m.model, c.dim(plural(m.calls, "call")), money(m.cost), c.dim(share(m.cost ?? 0, t.cost ?? 0)), c.dim(`${clock(m.from)} to ${clock(m.to)}${m.efforts.length ? ` · ${m.efforts.join(", ")}` : ""}`)]), { indent: 2 }));

  print(`\n${c.bold("Tokens")}`);
  const kinds = [["cacheRead", "read from cache"], ["cacheWrite", "written to cache"], ["input", "input"], ["output", "output"]];
  print(table(kinds.map(([k, label]) => [label, tokens(st.kinds[k].tokens), money(st.kinds[k].cost), c.dim(share(st.kinds[k].cost ?? 0, t.cost ?? 0))]), { indent: 2 }));

  print(`\n${c.bold("Turns")}`);
  const room = Math.max(20, width() - 40);
  const shown = st.turns.slice(-12);
  if (st.turns.length > shown.length) print(c.dim(`  the last 12 of ${st.turns.length}`));
  print(table(shown.map((x) => [c.dim(String(x.n)), clock(x.start), duration(x.end - x.start), money(x.cost), c.dim(clip(x.prompt ?? "(before the first prompt)", room))]), { indent: 2 }));

  if (st.tools.length) {
    print(`\n${c.bold("Tools")}`);
    print(table(st.tools.slice(0, 10).map((x) => [x.name.replace(/^mcp__(.+?)__/, "$1."), c.dim(plural(x.calls, "call")), x.errors ? c.yellow(`${x.errors} failed`) : "", c.dim(x.ms != null ? duration(x.ms) : "")]), { indent: 2 }));
  }
  if (st.subagents.length) print(c.dim(`\n${plural(st.subagents.length, "subagent")}: ${money(st.subagents.reduce((a, x) => a + x.cost, 0))} of the cost`));
}
