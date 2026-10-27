// The network across machines (docs/design/network.md) as the console
// words it: GET /api/net/state (network/state/v1) read leniently, the link
// to the gateway in a word, each machine's line, other machines' agents
// from the engine's list, what of theirs waits on you or seems stuck, and
// whether what waits sounds here. No DOM here: the Network panel
// (network.js), the attention strip (alerts.js), the map (network-map.js)
// and the tests share it.
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
  proposals: [],
  outbox: { frames: 0, bytes: 0, oldest: null },
  rosterHead: null,
  rosterMismatch: [],
  titlesOnly: false,
  shareLoad: true,
  start: "you",
  wake: "consent",
});

const list = (v) => (Array.isArray(v) ? v : []);

// The state as the page uses it: a network/state/v1 answer, or a machine on
// its own for anything else (no answer, an older service, an error).
export function stateOf(raw) {
  if (!raw || typeof raw !== "object" || raw.schema !== "network/state/v1") return { ...ALONE, hosts: [], tickets: [], proposals: [], rosterMismatch: [] };
  return {
    ...ALONE,
    ...raw,
    hosts: list(raw.hosts).filter((x) => x && typeof x.id === "string"),
    tickets: list(raw.tickets),
    proposals: list(raw.proposals).filter((p) => p && typeof p.id === "string" && typeof p.host === "string"),
    rosterMismatch: list(raw.rosterMismatch).filter((x) => typeof x === "string"),
    outbox: raw.outbox && typeof raw.outbox === "object" ? raw.outbox : ALONE.outbox,
  };
}

export const inNetwork = (s) => !!s?.network;

// Who may start sessions on a machine and wake them, in the console's
// words (network/state/v1's start and wake, and each host's reach).
export const STARTS = ["off", "you", "network"];
export const WAKES = ["consent", "network"];
export const START_WORDS = { off: "Nobody", you: "You", network: "The network" };
export const WAKE_WORDS = { consent: "Sessions you allowed", network: "The network" };

// A host's reach as its row says, read leniently: null for a machine whose
// XenoNet doesn't say (an older one), else { start, wake, pending }.
export function reachOf(host) {
  const r = host?.reach;
  if (!r || typeof r !== "object") return null;
  return {
    start: STARTS.includes(r.start) ? r.start : "you",
    wake: WAKES.includes(r.wake) ? r.wake : "consent",
    pending: list(r.pending).filter((w) => w && typeof w.id === "string" && (w.setting === "start" || w.setting === "wake")),
  };
}

// Whether a machine takes `what` from this one (start, answer): what its
// presence says its XenoNet can do.
export const hostCan = (host, what) => list(host?.can).includes(what);

// A widening that waits on a machine, as a line.
export const widenWords = (w, host) => `${w.by} asked to let the network ${w.setting} sessions on ${host}`;

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

// An entry that waits for the founder or a second window host to approve
// it (state.proposals): what it does, and who signed it so far.
export function proposalWords(p) {
  const by = list(p?.signers).join(" and ") || "nobody yet";
  const what = p.op === "gateway" ? `${p.host} to be the gateway${p.gateway ? `, at ${p.gateway}` : ""}` : p.op === "add" ? `add ${p.host}${p.window ? " as a window host" : ""}` : `${p.op} ${p.host}`;
  return `${what}, asked by ${by}`;
}

// Text from another machine, as the page keeps it: a string, cut to `max`,
// or null. It's shown escaped, as transcripts are, and never reaches an
// agent.
const words = (v, max) => (typeof v === "string" && v.trim() ? v.slice(0, max) : null);

// Other machines' agents from the engine's list (GET /api/engine/agents):
// those with an @<host>: id, each with what waits on you, however the
// engine words it (needsYou as a kind or as { kind, text, since }), and
// the detail their machine shares unless it shares titles only: the
// question's text, the last message and why it seems stuck.
export function remoteAgents(raw) {
  const out = [];
  for (const a of list(raw)) {
    const host = typeof a?.host === "string" && a.host ? a.host : hostOfId(a?.id);
    if (!HOST_ID.test(host ?? "") || typeof a.id !== "string" || !a.id.startsWith(`@${host}:`) || a.id.length > 300) continue;
    const need = a.needsYou && typeof a.needsYou === "object" ? a.needsYou : null;
    const kind = need?.kind ?? (typeof a.needsYou === "string" ? a.needsYou : null);
    const waits = a.waitsOnYou === true || !!kind;
    const stuck = list(a.stuck).map((r) => words(r, 280)).filter(Boolean).slice(0, 8);
    out.push({
      id: a.id,
      host,
      app: typeof a.app === "string" ? a.app : null,
      title: words(a.title, 200) ?? a.id,
      status: a.status ?? null,
      project: words(a.project, 200),
      repoKey: words(a.repoKey, 300),
      cwd: words(a.cwd, 500),
      branch: words(a.branch, 200),
      focus: words(a.focus ?? a.doing, 280),
      lastActive: Number.isFinite(a.lastActive) ? a.lastActive : null,
      seen: Number.isFinite(a.seen) ? a.seen : null,
      online: a.online !== false,
      waits,
      kind: waits ? (kind ?? "question") : null,
      // The question's text, from a machine that shares more than titles.
      text: waits ? words(need?.text ?? a.question, 320) : null,
      since: waits ? (need?.since ?? a.since ?? a.lastActive ?? null) : null,
      lastMessage: words(a.lastMessage, 280),
      stuck: stuck.length ? stuck : null,
    });
  }
  return out;
}

// How long you can be away from every console before what waits sounds on
// all of them (docs/design/network.md, section 8).
export const AWAY_MS = 5 * 60e3;

// Whether what waits on you, here or on another machine, sounds here, with
// its notification. On its own a machine always does. In a network only
// the machine where you last worked does (its presence's youAt, the newest
// of the machines that are up; two that tie both do), or every one once
// you've been away from all of them for five minutes. Machines that don't
// say when fall back on where you are (you), or every one when you're at
// none.
export function chimeHere(state, now = Date.now()) {
  if (!inNetwork(state)) return true;
  const self = state.self?.id;
  const hosts = list(state.hosts).filter((x) => x.id === self || x.online !== false);
  const at = (x) => (Number.isFinite(x?.youAt) ? x.youAt : null);
  const me = hosts.find((x) => x.id === self);
  const timed = hosts.filter((x) => at(x) != null);
  if (timed.length) {
    const latest = Math.max(...timed.map(at));
    if (now - latest >= AWAY_MS) return true;
    return at(me) != null && at(me) >= latest;
  }
  if (me?.you) return true;
  return !hosts.some((x) => x.you);
}

// What waits on you or seems stuck on other machines, as cards for the
// attention strip, with the question's text and the stuck reason as their
// machine shares them.
export function remoteWaits(state, agents, now = Date.now()) {
  if (!inNetwork(state)) return [];
  const chime = chimeHere(state, now);
  const online = new Map(list(state.hosts).map((x) => [x.id, x]));
  const out = [];
  for (const a of list(agents)) {
    const host = online.get(a.host);
    const where = { host: a.host, app: a.app, name: a.title, stale: !!host && !host.online, asOf: host?.seen ?? null };
    if (a.waits) {
      // A machine that shares titles only sends no question's text.
      const text = a.text || `The question shows in ${a.host}'s console.`;
      out.push({ kind: "remote-need", id: a.id, ...where, sub: a.kind === "permission" ? "needs permission" : "is waiting for you", text, since: a.since ?? now, chime });
    } else if (a.stuck) out.push({ kind: "remote-stuck", id: a.id, ...where, text: a.stuck.join(" · ") });
  }
  return out;
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
