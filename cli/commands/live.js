// xenonet live: messages that reach sessions as they're sent, and their
// controls: pause them all, let XenoNet wake or type into a session, grant an
// actor's token a scope, and read the audit log (server/routes/delivery.js).
import { c, print, json, table, Fail } from "../lib/out.js";
import { need, get, post } from "../../core/client.js";

export const help = `Live delivery puts a message into a session as it's sent, instead of when
the session next starts a turn: through a Claude Code channel, an OpenCode
session's own server, or, with your consent, typed into its tmux pane.

  xenonet live                         what's paused, granted and allowed, plans used, and
                                       the last deliveries
  xenonet live pause | resume          stop every message (and wake) at once, or start again
  xenonet live allow <session> [--hours n] [--remote]
                                       let XenoNet wake or type into one session (8 hours by
                                       default); --remote lets actors outside this machine do it
  xenonet live disallow <session>
  xenonet live grant <actor> <scope…> [--hours n]
                                       what an actor's token may see and message, such as
                                       grant hermes project:xenonet identity:adal (30 days)
  xenonet live revoke <actor>          take an actor's scope away, at once
  xenonet live audit                   the audit log's last entries
  --json                               as JSON

An actor sees and reaches no session until you grant it a scope. A wake
spends the session's plan: each sender and session has an hourly budget,
and a provider at 90% of its plan takes none. Set up Claude Code sessions to
take messages live with xenonet doctor --live.`;

const actorId = (a) => (String(a).startsWith("actor:") ? a : `actor:${a}`);

export async function run(args, flags) {
  const s = await need(flags);
  const [sub, ...rest] = args;
  const hours = flags.hours ? Number(flags.hours) : undefined;
  const say = (out) => (flags.json ? json(out) : print(typeof out === "string" ? out : JSON.stringify(out)));
  if (!sub || sub === "status") {
    const d = await get(s, "/api/delivery");
    if (flags.json) return json(d), 0;
    print(`${c.bold("Live delivery")} ${d.paused ? c.red("paused") : c.green("on")}`);
    const plans = Object.entries(d.usage ?? {}).filter(([, u]) => u != null);
    if (plans.length) print(`Plans used: ${plans.map(([app, u]) => `${app} ${Math.round(u * 100)}%${u >= 0.9 ? c.red(" (no wakes)") : ""}`).join(", ")}${c.dim(" · a provider at 90% takes no wakes")}`);
    print(c.bold("\nActors' scopes"));
    print(d.grants.length ? table(d.grants.map((g) => [g.actor, g.selector, g.expired ? c.red("expired") : `until ${new Date(g.expiresAt).toLocaleString()}`]), { indent: 2 }) : c.dim("  none: actors see no sessions"));
    print(c.bold("\nSessions XenoNet may wake or type into"));
    const consent = Object.entries(d.consent);
    print(consent.length ? table(consent.map(([id, x]) => [id, `until ${new Date(x.until).toLocaleString()}`, x.remote ? "remote too" : ""]), { indent: 2 }) : c.dim("  none"));
    print(c.bold("\nLast deliveries"));
    print(d.recent.length ? table(d.recent.slice(-10).map((r) => [`#${r.message}`, r.agent, r.state, r.how ?? "", r.ackedAt ? `acked in ${((r.ackedAt - r.at) / 1000).toFixed(1)} s` : ""]), { indent: 2 }) : c.dim("  none yet"));
    return 0;
  }
  if (sub === "pause" || sub === "resume") return say(await post(s, "/api/delivery/pause", { paused: sub === "pause" })), 0;
  if (sub === "allow" || sub === "disallow") {
    if (!rest[0]) throw new Fail(`say which session: xenonet live ${sub} <session id>`);
    return say(await post(s, `/api/delivery/${sub}`, { session: rest[0], hours, remote: !!flags.remote })), 0;
  }
  if (sub === "grant") {
    if (rest.length < 2) throw new Fail("say which actor and what scope: xenonet live grant hermes project:xenonet");
    return say(await post(s, "/api/delivery/grant", { actor: actorId(rest[0]), scope: rest.slice(1).join(" "), hours })), 0;
  }
  if (sub === "revoke") {
    if (!rest[0]) throw new Fail("say which actor: xenonet live revoke hermes");
    return say(await post(s, "/api/delivery/revoke", { actor: actorId(rest[0]) })), 0;
  }
  if (sub === "audit") {
    const d = await get(s, "/api/delivery?audit=100");
    if (flags.json) return json(d.audit), 0;
    for (const a of d.audit) print(`${c.dim(new Date(a.at).toLocaleString())} ${a.kind} ${Object.entries(a).filter(([k]) => !["at", "kind"].includes(k)).map(([k, v]) => `${k}=${v}`).join(" ")}`);
    return 0;
  }
  throw new Fail(`xenonet live doesn't know "${sub}"; xenonet live --help lists what it does`);
}
