// xenonet gateway: the actor gateway (server/actor-gateway.js), from your own
// terminal: whether it's on, the tunnel that puts it on the internet and
// whether its address reaches it, each app's connection, picking a tunnel,
// checking and repairing it, asking for an actor's gateway token, and taking
// one away. A gateway token is made in XenoNet's window only (Actors, then
// Connect… on the actor, or Allow on an ask), where it's shown once, masked.
// This command only asks, and never holds one.
import { c, print, json, ago, pad, Fail } from "../lib/out.js";
import { need, get, request, nameOf } from "../../core/client.js";
import { handoff } from "../../core/actors/handoff.js";

export const help = `XenoNet's actor gateway lets actors outside this machine, such as Muse or a
Grok Bot in their makers' clouds, message the sessions and actors here over
MCP. It listens on this machine alone. A tunnel you pick puts it on the
internet, and XenoNet keeps that tunnel running and checks that the address
works.

  xenonet gateway                    whether it's on, its tunnel, its address
                                   and whether that reaches it, and each
                                   app's connection
  xenonet gateway tunnels            the tunnels XenoNet can use, and which of
                                   them this computer has
  xenonet gateway on [--tunnel <kind>] [--host <name>] [--name <tunnel>]
                                   turns it on, through the tunnel named:
                                     tailscale         Tailscale Funnel
                                     cloudflare        a named Cloudflare
                                                       Tunnel (--name, --host)
                                     cloudflare-quick  a Cloudflare quick
                                                       tunnel (a new address
                                                       at every start)
                                     ngrok             ngrok (--host: your
                                                       ngrok domain)
                                     own               one you run (--host)
  xenonet gateway off                turns it off, with the tunnel XenoNet runs
  xenonet gateway check              checks now that the address reaches it
  xenonet gateway repair             sets the tunnel up again, and checks
  xenonet gateway address <slug>     what to tell an app whose address is old
  xenonet gateway token <slug>       asks for a new gateway token for an actor.
                                   It makes none and prints none: the ask
                                   waits as a card in XenoNet's window
  xenonet gateway revoke <slug>      takes an actor's gateway token away

An actor's gateway token is made in XenoNet's window: Allow on the card an
ask leaves, or Actors, then Connect… on the actor. There it's masked until
you choose Show, and Copy copies it without showing it. It works only at the
gateway.

  --json   answers as JSON`;

const REASONS = { rotated: "replaced", removed: "taken away", expired: "expired" };

function connection(a) {
  const bits = [];
  if (a.refused) bits.push(c.yellow(`! calls with a gateway token that was ${REASONS[a.refused.reason] ?? "refused"}: Connect… in XenoNet's window gives it a new one`));
  if (a.oldAddress) bits.push(c.yellow(`! last called ${a.lastHost}, an older address: xenonet gateway address ${a.id.replace(/^actor:/, "")}`));
  if (a.token && a.expiring) bits.push(c.yellow(`! its token stops working on ${new Date(a.expiresAt).toISOString().slice(0, 10)} unless it calls`));
  if (!bits.length) bits.push(a.lastAt ? c.dim(`last call ${ago(a.lastAt)}`) : c.dim("hasn't called yet"));
  return bits.join("  ");
}

function reachOf(st) {
  const r = st.reach ?? {};
  if (!st.on) return "";
  if (r.state === "ok") return c.green(` ✓ reachable${r.local ? " (checked on this computer alone)" : ""}, checked ${ago(r.at)}`);
  if (r.state === "checking") return c.dim(" checking…");
  if (r.state === "failing") return c.yellow(" ✗ not reachable");
  return c.dim(" not checked yet");
}

function show(st, s) {
  const state = st.on ? (st.listening ? c.green("on") : c.yellow("on, but not listening")) : c.dim("off");
  print(`${c.bold(nameOf(s))} ${c.dim("·")} the actor gateway is ${state}`);
  const t = st.tunnel ?? {};
  print(`  ${c.dim("tunnel")}           ${t.label ? `${t.label}${t.state && t.state !== "yours" ? c.dim(` (${t.state})`) : ""}` : c.yellow("none yet: xenonet gateway tunnels")}`);
  print(`  ${c.dim("address")}          ${st.url ?? c.dim("none yet")}${reachOf(st)}`);
  print(`  ${c.dim("on this machine")}  ${st.local}`);
  if (st.problem) print(c.yellow(`  ! ${st.problem}`));
  if (t.stable === false) print(c.dim(`  ${t.label}'s address changes every time it starts.`));
  if (st.actors?.length) {
    print(`  ${c.dim("apps")}`);
    const wide = Math.max(...st.actors.map((a) => a.name.length));
    for (const a of st.actors) print(`    ${pad(a.name, wide)}  ${connection(a)}`);
  }
  if (st.calls) print(c.dim(`  ${st.calls} calls since XenoNet started${st.lastActor ? `, last from ${st.lastActor}` : ""}`));
}

function showKinds(st) {
  print(`Tunnels for the actor gateway. XenoNet runs and checks the one you pick; its sign-in stays in the tool's own settings.`);
  for (const k of st.kinds ?? []) {
    const here = k.tool ? (k.found ? c.green(`found${k.path ? ` at ${k.path}` : ""}${k.version ? ` (${k.version})` : ""}`) : c.dim(`${k.tool} isn't on this computer`)) : c.dim("nothing to install");
    print(`\n  ${c.bold(k.kind)}  ${k.label}  ${here}${st.tunnel?.kind === k.kind ? c.green("  ← in use") : ""}`);
    print(`    ${k.needs}`);
    if (k.note) print(c.dim(`    ${k.note}`));
    if (k.docs) print(c.dim(`    ${k.docs}`));
  }
}

export async function run([what, slug], flags = {}) {
  const s = await need(flags);
  if (!what || what === "tunnels") {
    const st = await get(s, "/api/actors/gateway");
    if (flags.json) return json(what ? st.kinds : st);
    return what ? showKinds(st) : show(st, s);
  }
  if (what === "on" || what === "off") {
    const body = { on: what === "on" };
    if (what === "on" && (flags.tunnel || flags.host || flags.name)) {
      const now = await get(s, "/api/actors/gateway");
      const kind = typeof flags.tunnel === "string" ? flags.tunnel : (now.kinds ?? []).find((k) => k.kind === now.tunnel?.kind)?.asks?.host ? now.tunnel.kind : "own";
      body.tunnel = { kind, ...(typeof flags.host === "string" ? { host: flags.host } : {}), ...(typeof flags.name === "string" ? { name: flags.name } : {}) };
    }
    const st = await request(s, "POST", "/api/actors/gateway", body);
    return flags.json ? json(st) : show(st, s);
  }
  if (what === "check" || what === "repair") {
    const st = await request(s, "POST", "/api/actors/gateway", { [what]: true });
    return flags.json ? json(st) : show(st, s);
  }
  if (what === "address") {
    if (!slug) throw new Fail("say whose: xenonet gateway address <slug>", 2);
    const st = await get(s, "/api/actors/gateway");
    const id = `actor:${String(slug).replace(/^actor:/, "")}`;
    const a = (st.actors ?? []).find((x) => x.id === id);
    if (!a) throw new Fail(`${id} has no gateway token; connect it in XenoNet's window (Actors, then Connect… on it)`, 1);
    if (!st.url) throw new Fail("the gateway has no address yet: xenonet gateway on --tunnel <kind>", 1);
    const h = handoff(a.product, { url: st.url });
    if (flags.json) return json({ actor: id, url: st.url, ...h });
    print(`${c.bold(a.name)}: the gateway's address is ${st.url}, and its token still works`);
    h.steps.forEach((step, i) => print(`  ${i + 1}. ${step.text}`));
    if (h.message) print(`\n${h.message}`);
    return;
  }
  if (what === "token") {
    if (!slug) throw new Fail("say whose: xenonet gateway token <slug>", 2);
    const r = await request(s, "POST", `/api/actors/${encodeURIComponent(String(slug).replace(/^actor:/, ""))}/gateway/ask`, {});
    if (flags.json) return json(r);
    print(`${c.green("✓")} asked: ${r.id}'s gateway token waits for you to click Allow in XenoNet's window${r.replaces ? ", and replaces the one its app has" : ""}`);
    return print(c.dim("  The token is made and shown there, masked; this command never sees it."));
  }
  if (what === "revoke") {
    if (!slug) throw new Fail("say whose: xenonet gateway revoke <slug>", 2);
    const r = await request(s, "DELETE", `/api/actors/${encodeURIComponent(String(slug).replace(/^actor:/, ""))}/gateway`);
    return flags.json ? json(r) : print(`${c.green("✓")} ${r.actor}'s gateway token stopped working`);
  }
  throw new Fail(`xenonet gateway doesn't know ${what}: tunnels, on, off, check, repair, address, token or revoke`, 2);
}
