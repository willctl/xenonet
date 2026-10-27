// What this machine lets a sender do to its sessions and actors, decided
// where the message arrives: a message from another machine before it's
// stored, and each way the router wakes a session. One place, so limits on
// who may reach whom go here, never in the transport.
//
//   message  open: the sender's own machine checked its scope, and the
//            policy's guards (the pause switch, rates, loops) still hold
//   wake     your allowance for the session (xenonet live allow), which
//            has to cover actors (--remote) when an actor sends
import { actorOf, isYou } from "./policy.js";
import { parseQualified } from "../schema/ids.js";

export const ACTIONS = ["message", "wake"];

// A sender as this machine sees it: you, a session or an actor, on this
// machine (host null) or another.
export function senderOf(id) {
  const s = String(id ?? "");
  if (isYou(s)) return { kind: "you", id: s, host: null };
  const a = actorOf(s);
  if (a) return { kind: "actor", id: s, host: a.host };
  return { kind: "session", id: s, host: parseQualified(s)?.host ?? null };
}

// { ok: true }, or { ok: false, why }. `settings` is what this machine is
// set to; for a wake, `consent`: whether you allowed the target, here
// (`here`) and for actors (`remote`).
export function decide(action, sender, target, settings = {}) {
  if (action === "message") return { ok: true };
  if (action === "wake") {
    const consent = settings?.consent ?? {};
    return (sender?.kind === "actor" ? consent.remote : consent.here) ? { ok: true } : { ok: false, why: "no consent" };
  }
  return { ok: false, why: `nothing decides "${action}" here` };
}
