// What this machine lets a sender do to its sessions and actors, decided
// where the message arrives: a message from another machine before it's
// stored, each way the router wakes a session, and a start before anything
// runs. One place, so limits on who may reach whom go here, never in the
// transport.
//
//   message  open: the sender's own machine checked its scope, and the
//            policy's guards (the pause switch, rates, loops) still hold
//   wake     wake: network lets any sender wake a session, within the
//            budgets; else your allowance for the session (xenonet live
//            allow), which has to cover actors (--remote) when one sends
//   start    start: off refuses all; a start comes only from a window host
//            (this machine counts as one); you need a strong check (the
//            window key); sessions and actors need start: network
import { actorOf, isYou } from "./policy.js";
import { parseQualified } from "../schema/ids.js";

export const ACTIONS = ["message", "wake", "start"];

// A sender as this machine sees it: you, a session or an actor, on this
// machine (host null) or another; from a window host, checked weakly,
// unless the caller knows better.
export function senderOf(id) {
  const s = String(id ?? "");
  const make = (kind, host) => ({ kind, id: s, host, window: true, strong: false });
  if (isYou(s)) return make("you", null);
  const a = actorOf(s);
  if (a) return make("actor", a.host);
  return make("session", parseQualified(s)?.host ?? null);
}

// { ok: true }, or { ok: false, why }. `settings` is what this machine is
// set to: `consent` (whether you allowed the target, here and for actors),
// `start` (off, you or network) and `wake` (consent or network).
export function decide(action, sender, target, settings = {}) {
  if (action === "message") return { ok: true };
  if (action === "wake") {
    if (settings?.wake === "network") return { ok: true };
    const consent = settings?.consent ?? {};
    return (sender?.kind === "actor" ? consent.remote : consent.here) ? { ok: true } : { ok: false, why: "no consent" };
  }
  if (action === "start") {
    const start = ["off", "network"].includes(settings?.start) ? settings.start : "you";
    if (start === "off") return { ok: false, why: "this machine starts no sessions: its start setting is off" };
    // The receiver can tell only which machine signed the request and
    // whether it's a window host; what it says of its sender is that
    // machine's word.
    if (sender?.window === false) return { ok: false, why: `${sender.host ?? "that machine"} isn't a window host, and only window hosts start sessions` };
    if (sender?.kind === "you") return sender.strong ? { ok: true } : { ok: false, why: "a start as you comes only from a window host's console, checked by its window key" };
    return start === "network" ? { ok: true } : { ok: false, why: "only you start sessions here: its start setting is you" };
  }
  return { ok: false, why: `nothing decides "${action}" here` };
}
