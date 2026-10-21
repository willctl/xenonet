// The network across machines (docs/design/network.md) as the console
// words it: GET /api/net/state (network/state/v1) read leniently, the link
// to the gateway in a word, each machine's line, other machines' agents
// from the engine's list, and what of theirs waits on you. No DOM here:
// the Network panel (network.js), the attention strip (alerts.js) and the
// tests share it.
import { hostOfId } from "./hosts.js";

// A machine's id in a network, which the person picks.
export const HOST_ID = /^[a-z][a-z0-9-]{1,31}$/;
// How long a ticket works.
export const TICKET_MS = 10 * 60e3;

// A machine that isn't in a network.
export const ALONE = Object.freeze({
  schema: "network/state/v1",
  network: null,
  self: null,
  gateway: null,
  hosts: [],
  tickets: [],
  outbox: { frames: 0, bytes: 0, oldest: null },
  rosterHead: null,
  rosterMismatch: [],
  titlesOnly: false,
});

const list = (v) => (Array.isArray(v) ? v : []);

// The state as the page uses it: a network/state/v1 answer, or a machine on
// its own for anything else (no answer, an older service, an error).
export function stateOf(raw) {
  if (!raw || typeof raw !== "object" || raw.schema !== "network/state/v1") return { ...ALONE, hosts: [], tickets: [], rosterMismatch: [] };
  return {
    ...ALONE,
    ...raw,
    hosts: list(raw.hosts).filter((x) => x && typeof x.id === "string"),
    tickets: list(raw.tickets),
    rosterMismatch: list(raw.rosterMismatch).filter((x) => typeof x === "string"),
    outbox: raw.outbox && typeof raw.outbox === "object" ? raw.outbox : ALONE.outbox,
  };
}

export const inNetwork = (s) => !!s?.network;

// A time as the console shows it: the clock today, the date before that.
export function clock(ms, now = Date.now()) {
  if (!Number.isFinite(ms)) return "";
  const d = new Date(ms);
  return d.toDateString() === new Date(now).toDateString() ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString([], { month: "short", day: "numeric" });
}

const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

export const KIND_WORDS = { tunnel: "through a tunnel", lan: "on the LAN", loopback: "on this machine only" };

// How far this machine's clock is from the gateway's, when it's enough to
// mention: the link signs with the offset, so it's for the doctor and you.
export function skewWords(ms) {
  if (!Number.isFinite(ms) || Math.abs(ms) < 2000) return "";
  const s = Math.round(Math.abs(ms) / 1000);
  return `clock ${s < 120 ? `${s} s` : `${Math.round(s / 60)} min`} off the gateway's`;
}

// The link to the gateway: a state for its dot (ok, wait, bad, off, none),
// a word, and a line under it.
export function linkWords(g, now = Date.now()) {
  if (!g) return { state: "none", word: "No address yet", sub: "The other machines reach the gateway at an address: a tunnel, or this machine's address on your LAN. Set it with the gateway's Set up… in the Actors panel." };
  const since = Number.isFinite(g.since) ? ` since ${clock(g.since, now)}` : "";
  const why = g.reason ? `: ${g.reason}` : "";
  switch (g.link) {
    case "live":
      return { state: "ok", word: "Linked", sub: `Linked${since}` };
    case "connecting":
      return { state: "wait", word: "Connecting…", sub: `Reaching the gateway${since}${why}` };
    case "offline":
      return { state: "bad", word: "Can't reach the gateway", sub: `No answer${since}${why}. This machine keeps working; what's for other machines waits in the outbox and goes when the gateway is back.` };
    case "refused":
      return { state: "bad", word: "The gateway refused this machine", sub: `Refused${since}${why}. The doctor says more.` };
    case "removed":
      return { state: "bad", word: "Removed from the network", sub: `Removed${since}${why}. Run xenonet net leave on this machine to use it on its own again.` };
    default:
      return { state: "off", word: "Off", sub: "The link to the gateway is off." };
  }
}

// A machine's line in the panel: online with its agents and what waits on
// you, or since when it's been away.
export function hostLine(host, now = Date.now()) {
  if (!host) return "";
  const agents = Number.isFinite(host.agents) ? plural(host.agents, "agent") : null;
  const waits = host.waitsOnYou > 0 ? `${host.waitsOnYou} waiting on you` : null;
  if (host.online) return ["online", agents, waits].filter(Boolean).join(" · ");
  const away = Number.isFinite(host.seen) ? `offline since ${clock(host.seen, now)}` : "offline";
  return [away, agents && `${agents} when last seen`, waits].filter(Boolean).join(" · ");
}

// The marks beside a machine's name.
export function hostTags(host, self) {
  return [host.id === self?.id && "this machine", host.you && "you're here", host.founder && "founder", host.gateway && "gateway", host.window && "window"].filter(Boolean);
}

// What's waiting to go to other machines.
export function outboxWords(o, now = Date.now()) {
  const n = Number(o?.frames) || 0;
  if (!n) return "Nothing waits to go to other machines.";
  const kb = Number(o.bytes) > 0 ? ` (${Math.max(1, Math.round(o.bytes / 1024))} KB)` : "";
  const oldest = Number.isFinite(o.oldest) ? `, the oldest from ${clock(o.oldest, now)}` : "";
  return `${plural(n, "item")} wait${n === 1 ? "s" : ""} in the outbox${kb}${oldest}. They go when the gateway answers.`;
}

export const ticketWords = (t, now = Date.now()) => `${t.host}${t.window ? ", with a window" : ""}: works until ${clock(t.expires, now)}`;

// Other machines' agents from the engine's list (GET /api/engine/agents):
// those with an @<host>: id, each with what waits on you, however the
// engine words it (needsYou as a kind or as { kind, text, since }).
export function remoteAgents(raw) {
  const out = [];
  for (const a of list(raw)) {
    const host = typeof a?.host === "string" && a.host ? a.host : hostOfId(a?.id);
    if (!HOST_ID.test(host ?? "") || typeof a.id !== "string" || !a.id.startsWith("@")) continue;
    const need = a.needsYou && typeof a.needsYou === "object" ? a.needsYou : null;
    const kind = need?.kind ?? (typeof a.needsYou === "string" ? a.needsYou : null);
    const waits = a.waitsOnYou === true || !!kind;
    out.push({
      id: a.id,
      host,
      app: typeof a.app === "string" ? a.app : null,
      title: typeof a.title === "string" && a.title ? a.title : a.id,
      status: a.status ?? null,
      project: a.project ?? null,
      branch: a.branch ?? null,
      focus: a.focus ?? a.doing ?? null,
      lastActive: Number.isFinite(a.lastActive) ? a.lastActive : null,
      seen: Number.isFinite(a.seen) ? a.seen : null,
      waits,
      kind: waits ? (kind ?? "question") : null,
      text: waits ? (need?.text ?? a.question ?? a.focus ?? null) : null,
      since: waits ? (need?.since ?? a.since ?? a.lastActive ?? null) : null,
    });
  }
  return out;
}

// Whether a wait on another machine sounds here: only on the machine you're
// at, or on every one when you're at none of them (docs/design/network.md,
// section 8).
export function chimeHere(state) {
  const hosts = list(state?.hosts);
  if (hosts.find((x) => x.id === state?.self?.id)?.you) return true;
  return !hosts.some((x) => x.you);
}

// What waits on you on other machines, as cards for the attention strip.
export function remoteWaits(state, agents) {
  if (!inNetwork(state)) return [];
  const chime = chimeHere(state);
  const online = new Map(list(state.hosts).map((x) => [x.id, x]));
  return list(agents)
    .filter((a) => a.waits)
    .map((a) => {
      const host = online.get(a.host);
      // A machine that shares titles only sends no question's text.
      const text = a.text || `The question shows in ${a.host}'s console.`;
      return { kind: "remote-need", id: a.id, host: a.host, app: a.app, name: a.title, sub: a.kind === "permission" ? "needs permission" : "is waiting for you", text, since: a.since ?? Date.now(), chime, stale: !!host && !host.online, asOf: host?.seen ?? null };
    });
}

// Why a new machine's id won't do, or null.
export function hostIdProblem(id, state) {
  if (!HOST_ID.test(id ?? "")) return "A machine's id is 2 to 32 lowercase letters, digits and dashes, starting with a letter.";
  if (list(state?.hosts).some((x) => x.id === id)) return `There's a machine called ${id} in the network already. If it was set up again, use Replace on its row.`;
  return null;
}

// A ticket's answer (POST /api/net/ticket, or replace): the line to run,
// the part of it that's secret, and when it lapses. Null if there's no line.
export function joinOf(data, now = Date.now()) {
  if (!data || typeof data !== "object") return null;
  const ticket = typeof data.ticket === "string" && data.ticket ? data.ticket : null;
  const line = typeof data.line === "string" && data.line ? data.line : ticket && typeof data.address === "string" ? `xenonet net join ${data.address} ${ticket}` : null;
  if (!line) return null;
  return { line, secret: ticket ?? line.trim().split(/\s+/).pop(), expires: Number.isFinite(data.expires) ? data.expires : now + TICKET_MS };
}

// The address and ticket from what was typed or pasted: either one, or the
// whole line Add a machine showed.
export function joinFields(address, ticket) {
  const m = /^\s*xenonet\s+net\s+join\s+(\S+)\s+(\S+)\s*$/.exec(address ?? "");
  return m ? { address: m[1], ticket: m[2] } : { address: String(address ?? "").trim(), ticket: String(ticket ?? "").trim() };
}
