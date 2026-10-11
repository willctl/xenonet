// xenonet messages: what agents said to each other in the last day, from the
// engine (core/engine), with who has read each one.
import { c, print, json, table } from "../lib/out.js";
import { need, get } from "../../core/client.js";

export const help = `The messages sessions and actors sent each other in the last 24 hours,
oldest first, with who has read each one.

  --json   the messages as JSON`;

export async function run(_args, flags) {
  const s = await need(flags);
  const [{ messages }, agents] = await Promise.all([get(s, "/api/engine/state"), get(s, "/api/engine/agents").catch(() => [])]);
  if (flags.json) return json(messages);
  if (!messages.length) return print(c.dim("No messages between sessions or actors in the last 24 hours."));
  const titles = new Map(agents.map((a) => [a.id, a.title]));
  const name = (id) => titles.get(id) ?? id.replace(/^(\w+):(.{8}).*$/, "$1:$2");
  const rows = [];
  for (const m of [...messages].sort((a, b) => a.at - b.at)) {
    const time = new Date(m.at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    const to = m.target.startsWith("repo:") || m.target === "all" ? m.target.replace(/^repo:/, "everyone in ") : name(m.target);
    const seen = m.to.filter((d) => d.seen).length;
    rows.push([c.dim(time), `${c.bold(name(m.from))} ${c.dim("to")} ${to} ${c.dim(`#${m.id}, read by ${seen} of ${m.to.length}`)}`]);
    rows.push(["", m.text]);
  }
  print(table(rows, { wrap: true }));
}
