// xenonet status, and xenonet on its own: which XenoNet is running, what it
// reads, the agents and actors at work, what needs you, and the doctor's
// last word.
import { c, print, json, table, plural, ago } from "../lib/out.js";
import { find, get, nameOf, candidates } from "../../core/client.js";

export const help = `Which XenoNet is running, what it reads, the SI at work and the actors
apart from them, which sessions need you, and the doctor's last word.
Exits 1 when no XenoNet answers.`;

const NEED = { question: "has a question", permission: "is waiting for a permission" };
const PRESENCES = ["working", "idle", "away", "offline", "unknown"];

export async function run(_args, flags) {
  const s = await find(flags);
  if (!s) {
    if (flags.json) json({ running: false });
    else print(`${c.dim("○")} XenoNet isn't running ${c.dim(`(nothing answers on ${candidates(flags).map((p) => `127.0.0.1:${p}`).join(" or ")})`)}\n  Open XenoNet, or start XenoNet Dev from its checkout with npm run app.`);
    return 1;
  }
  const [summary, all, list, doctor] = await Promise.all([get(s, "/api/summary"), get(s, "/api/engine/agents").catch(() => []), get(s, "/api/actors").catch(() => null), get(s, "/api/doctor").catch(() => null)]);
  // The engine counts actors among the agents; here they're apart, as the
  // actors' list has them (you, outside XenoNet's window, only a placeholder).
  const agents = all.filter((a) => a.app !== "actor");
  const actors = list ? list.actors.filter((a) => a.id !== "actor:you" || a.presence !== "unknown").map((a) => ({ id: a.id, name: a.name, presence: a.presence, status: a.status })) : null;
  if (flags.json) return json({ running: true, name: nameOf(s), version: s.health.version ?? null, port: s.port, demo: !!s.health.demo, sources: s.health.sources, summary, agents, actors, doctor: doctor && { at: doctor.at, summary: doctor.summary } });

  const count = (name) => s.health.sources?.find((x) => x.name === name && x.ok)?.count;
  const reads = [count("projects") != null && plural(count("projects"), "project"), count("claude") != null && plural(count("claude"), "Claude session"), count("codex") != null && plural(count("codex"), "Codex thread")].filter(Boolean);
  const connected = agents.filter((a) => a.reachable).length;
  const working = agents.filter((a) => a.status === "working").length;
  const rows = [
    [c.dim("Reads"), reads.join(", ") || "nothing yet"],
    [c.dim("SI"), agents.length ? `${working} working, ${agents.length - working} idle${connected ? `, ${connected} connected to XenoNet` : ""}` : "none at work in the last two hours"],
  ];
  // Actors only where there are some, or the line would be noise.
  if (actors?.length) rows.push([c.dim("Actors"), PRESENCES.map((p) => [p, actors.filter((a) => a.presence === p).length]).filter(([, n]) => n).map(([p, n]) => `${n} ${p}`).join(", ")]);
  // A session on another machine (XENONET_HOSTS=1) says which.
  const named = (n) => `${n.host ? `${n.host} · ` : ""}${n.title}`;
  for (const [i, n] of summary.needsYou.entries()) rows.push([i ? "" : c.dim("Needs you"), `${c.yellow("●")} ${named(n)} ${c.dim(`(${n.app}) ${NEED[n.kind] ?? n.kind}`)}`]);
  if (summary.stuck.length) rows.push([c.dim("Stuck"), summary.stuck.map(named).join(", ")]);
  if (doctor) {
    const { fail = 0, warn = 0 } = doctor.summary;
    const verdict = fail || warn ? c[fail ? "red" : "yellow"](`${[fail && plural(fail, "problem"), warn && plural(warn, "warning")].filter(Boolean).join(" and ")}, which xenonet doctor explains`) : "everything XenoNet needs is in place";
    rows.push([c.dim("Doctor"), `${verdict} ${c.dim(`(checked ${ago(doctor.at)})`)}`]);
  }
  print(`${c.green("●")} ${c.bold(nameOf(s))}${s.health.version ? ` ${s.health.version}` : ""} ${c.dim(`on 127.0.0.1:${s.port}${s.health.demo ? ", made-up data" : ""}`)}`);
  print(table(rows, { indent: 2, wrap: true }));
}
