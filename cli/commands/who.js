// xenonet who <path>: which agents have a hand in a file or folder, as the
// engine sees it (the who_has tool, for people).
import path from "node:path";
import { pathStyle } from "../../core/paths.js";
import { c, print, json, table, ago, Fail } from "../lib/out.js";
import { need, get } from "../../core/client.js";

export const help = `Which agents have a hand in a file or folder: editing it (in this working
tree, or the same file in another worktree of the repo, a merge conflict to
come) or holding a claim on it. An actor's edit that XenoNet inferred from
what it observes, rather than heard of from the actor, is marked (observed).

  --json   the answer as JSON`;

const HOW = { editing: "is editing it", worktree: "is editing it in another worktree", claimed: "claimed it" };

export async function run([target], flags) {
  if (!target) throw new Fail("say which file: xenonet who <path>", 2);
  const file = pathStyle(target).resolve(target);
  const s = await need(flags);
  const hands = await get(s, `/api/engine/who?path=${encodeURIComponent(file)}`);
  if (flags.json) return json(hands);
  if (!hands.length) return print(c.dim(`No agent has a hand in ${target} right now.`));
  const rows = hands.map((h) => [
    c.green("●"),
    h.agent.title,
    c.dim(h.agent.app),
    `${HOW[h.how] ?? h.how}${h.observed ? " (observed)" : ""}${h.how === "claimed" ? `${h.note ? ` ("${h.note}")` : ""} until ${new Date(h.until).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : `, ${ago(h.at)}`}`,
  ]);
  print(table(rows, { wrap: true }));
}
