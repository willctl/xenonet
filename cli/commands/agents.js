// xenonet agents: the agents at work on this machine, as the engine sees them
// (core/engine): what each is doing, where, and what it has in hand. The
// engine counts actors among them; they're listed apart, after the
// sessions, and never counted as sessions connected to XenoNet.
import path from "node:path";
import { c, print, json, ago, clip, width } from "../lib/out.js";
import { need, get } from "../../core/client.js";

export const help = `The Claude Code, Codex and Grok Build sessions at work in the last two hours: whether
each is working, where (project, branch), what it says it's doing, and the
files it's editing or has claimed. Connected sessions (xenonet connect) can
see and message each other. The actors at work (xenonet actors) follow,
apart from them.

  --repo   only the sessions in the current directory's repo
  --json   the full list as JSON, actors (app "actor") included`;

export async function run(_args, flags) {
  const s = await need(flags);
  let agents = await get(s, "/api/engine/agents");
  if (flags.repo) {
    const here = process.cwd();
    agents = agents.filter((a) => [a.top, a.cwd].some((d) => d && (here === d || here.startsWith(d + path.sep))));
  }
  if (flags.json) return json(agents);
  if (!agents.length) return print(c.dim(flags.repo ? "No session or actor has worked in this repo in the last two hours." : "No session or actor has worked in the last two hours."));
  const sessions = agents.filter((a) => a.app !== "actor");
  const actors = agents.filter((a) => a.app === "actor");

  // A line for each agent, and a second for what it has in hand.
  const room = width() - 2;
  const show = (a) => {
    const dot = a.status === "working" ? c.green("●") : c.dim("○");
    const meta = [a.app !== "actor" && a.app, a.project, a.branch && clip(a.branch, 28), a.status !== "working" && ago(a.lastActive)].filter(Boolean).join(" · ");
    const title = clip(a.title, Math.max(20, room - [...meta].length - 2));
    print(meta ? `${dot} ${c.bold(title)}  ${c.dim(meta)}` : `${dot} ${c.bold(title)}`);
    const notes = [
      a.needsYou && c.yellow(a.needsYou === "question" ? "has a question for you" : "waiting for your permission"),
      a.focus && `says: ${a.focus}`,
      a.editing.length && `editing ${a.editing.slice(0, 3).map((e) => `${path.basename(e.path)}${e.observed ? " (observed)" : ""}`).join(", ")}${a.editing.length > 3 ? ` and ${a.editing.length - 3} more` : ""}`,
      a.claims.length && `claimed ${a.claims.map((x) => (a.top && x.pattern.startsWith(a.top + "/") ? x.pattern.slice(a.top.length + 1) : x.pattern)).join(", ")}`,
    ].filter(Boolean);
    if (notes.length) print(`  ${clip(notes.join(" · "), room)}`);
  };
  sessions.forEach(show);
  if (actors.length) {
    if (sessions.length) print();
    print(c.bold("Actors"));
    actors.forEach(show);
  }
  // Only sessions connect to XenoNet; an actor reaches it with its token.
  const connected = sessions.filter((a) => a.reachable).length;
  if (sessions.length && !connected) print(c.dim(`\nNone of these sessions is connected to XenoNet, so they can't see or message each other. xenonet connect sets that up.`));
}
