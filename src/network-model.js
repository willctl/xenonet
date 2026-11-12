// The network across machines (docs/design/network.md) as the console
// words it: GET /api/net/state (network/state/v1) read leniently, how this
// machine stands in it in a word, each machine's line, other machines' agents
// from the engine's list, what of theirs waits on you or seems stuck, and
// whether what waits sounds here. No DOM here: the Network panel
// (network.js), the attention strip (alerts.js), the map (network-map.js)
// and the tests share it.

// A machine's id in a network, which the person picks.
export const HOST_ID = /^[a-z][a-z0-9-]{1,31}$/;
// How long an invite works.
export const INVITE_MS = 10 * 60e3;

// A machine that isn't in a network.
export const ALONE = Object.freeze({
  schema: "network/state/v1",
  network: null,
  self: null,
  link: null,
  hosts: [],
  invites: { open: [], burned: [] },
  proposals: [],
  frozen: [],
  rosterHead: null,
  rosterMismatch: [],
  titlesOnly: false,
  shareLoad: true,
  start: "you",
  wake: "consent",
});

const list = (v) => (Array.isArray(v) ? v : []);

// The networks a state lists (a machine can be in several), each with
// this machine's host id in it, what it shares into it and its relays.
export function networksOf(raw) {
  return list(raw?.networks)
    .filter((n) => n && typeof n.id === "string")
    .map((n) => ({ ...n, name: typeof n.name === "string" ? n.name : n.id, host: typeof n.host === "string" ? n.host : null, relays: list(n.relays).filter((r) => typeof r === "string"), mailboxes: list(n.mailboxes).filter((m) => typeof m === "string"), sharing: n.sharing === "all" ? "all" : list(n.sharing).filter((p) => typeof p === "string") }));
}

// The state as the page uses it: a network/state/v1 answer, or a machine on
// its own for anything else (no answer, an older service, an error). With
// `chosen`, a network's id, that network's part of it, as the first's is
// at the top of the answer; else the first network's.
export function stateOf(raw, chosen = null) {
  if (!raw || typeof raw !== "object" || raw.schema !== "network/state/v1") return { ...ALONE, hosts: [], invites: { open: [], burned: [] }, proposals: [], frozen: [], rosterMismatch: [], networks: [] };
  const networks = networksOf(raw);
  const pick = networks.find((n) => n.id === chosen) ?? null;
  if (pick) raw = { ...raw, ...pick, network: { id: pick.id, name: pick.name } };
  return {
    ...ALONE,
    ...raw,
    networks,
    hosts: list(raw.hosts).filter((x) => x && typeof x.id === "string"),
    invites: { open: list(raw.invites?.open).filter((t) => t && typeof t.host === "string"), burned: list(raw.invites?.burned).filter((t) => t && typeof t.host === "string") },
    proposals: list(raw.proposals).filter((p) => p && typeof p.id === "string" && typeof p.host === "string"),
    frozen: list(raw.frozen).filter((f) => f && typeof f.author === "string"),
    rosterMismatch: list(raw.rosterMismatch).filter((x) => typeof x === "string"),
    mailboxes: list(raw.mailboxes).filter((m) => typeof m === "string"),
    link: raw.link && typeof raw.link === "object" ? raw.link : null,
  };
}

export const inNetwork = (s) => !!s?.network;

// Who may start sessions on a machine and wake them, in the console's
// words (network/state/v1's start and wake, and each host's reach).
export const STARTS = ["off", "you", "network", "delegate"];
export const WAKES = ["consent", "network"];
export const START_WORDS = { off: "Nobody", you: "You", network: "The network", delegate: "Delegated" };
export const WAKE_WORDS = { consent: "Sessions you allowed", network: "The network" };

// A host's reach as its row says, read leniently: null for a machine whose
// XenoNet doesn't say (an older one), else { start, wake, pending,
// delegated }, delegated being who put it on delegation and when.
export function reachOf(host) {
  const r = host?.reach;
  if (!r || typeof r !== "object") return null;
  return {
    start: STARTS.includes(r.start) ? r.start : "you",
    wake: WAKES.includes(r.wake) ? r.wake : "consent",
    pending: list(r.pending).filter((w) => w && typeof w.id === "string" && (w.setting === "start" || w.setting === "wake")),
    delegated: r.delegated && typeof r.delegated.by === "string" ? { by: r.delegated.by, at: Number.isFinite(r.delegated.at) ? r.delegated.at : null } : null,
  };
}

// Whether a machine takes `what` from this one (start, answer): what its
// presence says its XenoNet can do.
export const hostCan = (host, what) => list(host?.can).includes(what);

// A widening that waits on a machine, as a line.
export const widenWords = (w, host) =>
  w.value === "delegate" ? `${w.by} asked to put ${host} on your standing delegation` : `${w.by} asked to let the network ${w.setting} sessions on ${host}`;

// A time as the console shows it: the clock today, the date before that.
export function clock(ms, now = Date.now()) {
  if (!Number.isFinite(ms)) return "";
  const d = new Date(ms);
  return d.toDateString() === new Date(now).toDateString() ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString([], { month: "short", day: "numeric" });
}

const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;

// How this machine stands in the network: a state for its dot (ok, bad,
// off), a word, and a line under it. Its machines reach each other
// straight, so there's no one machine to reach: each row says whether its
// machine is online.
export function linkWords(l, now = Date.now()) {
  const since = Number.isFinite(l?.since) ? ` since ${clock(l.since, now)}` : "";
  const why = l?.reason ? `: ${l.reason}` : "";
  switch (l?.state) {
    case "live":
      return { state: "ok", word: "In the network", sub: `Reaching its machines${since}, on the LAN, by direct paths or through a relay.` };
    case "removed":
      return { state: "bad", word: "Removed from the network", sub: `Removed${since}${why}. Run xenonet net leave on this machine to use it on its own again.` };
    default:
      return { state: "off", word: "Off", sub: "This machine isn't reaching the network's machines: XenoNet's service isn't running its node." };
  }
}

// A machine's line in the panel: online with its agents and what waits on
// you, or since when it's been away.
export function hostLine(host, now = Date.now()) {
  if (!host) return "";
  const agents = Number.isFinite(host.agents) ? plural(host.agents, "agent") : null;
  const waits = host.waitsOnYou > 0 ? `${host.waitsOnYou} waiting on you` : null;
  const held = host.held > 0 ? `${plural(host.held, "item")} held for it` : null;
  if (host.online) return ["online", agents, waits, held].filter(Boolean).join(" · ");
  const away = Number.isFinite(host.seen) ? `offline since ${clock(host.seen, now)}` : "offline";
  return [away, agents && `${agents} when last seen`, waits, held].filter(Boolean).join(" · ");
}

// The marks beside a machine's name.
export function hostTags(host, self) {
  return [host.id === self?.id && "this machine", host.you && "you're here", host.founder && "founder", host.window && "window"].filter(Boolean);
}

export const inviteWords = (t, now = Date.now()) => `${t.host}${t.window ? ", with a window" : ""}: works until ${clock(t.expires, now)}${t.wrong > 0 ? ` (${plural(t.wrong, "wrong code")})` : ""}`;

// An entry that waits for the founder or a second window host to approve
// it (state.proposals): what it does, and who signed it so far.
export function proposalWords(p) {
  const by = list(p?.signers).join(" and ") || "nobody yet";
  const what = p.op === "add" ? `add ${p.host}${p.window ? " as a window host" : ""}` : `${p.op} ${p.host}`;
  return `${what}, asked by ${by}`;
}

// A member's feed a fork froze here (state.frozen): the machine signed two
// different entries at one place, so nothing more from it is taken until
// you unfreeze it.
export function frozenWords(f) {
  const at = Number.isFinite(f?.at?.seq) ? ` (entry ${f.at.seq})` : "";
  return `${f?.host ?? f?.author} signed two different entries at one place in its feed${at}, so nothing more from it is taken here`;
}

// Text from another machine, as the page keeps it: a string, cut to `max`,
// or null. It's shown escaped, as transcripts are, and never reaches an
// agent.
const words = (v, max) => (typeof v === "string" && v.trim() ? v.slice(0, max) : null);

// An agent of another machine, by the agt_ its machine made.
export const AGENT_ID = /^agt_[a-z2-7]{32}$/;

// Other machines' agents from the engine's list (GET /api/engine/agents):
// those with an agt_ id and the machine they're on (`host`), each with
// what waits on you, however the
// engine words it (needsYou as a kind or as { kind, text, since }), and
// the detail their machine shares unless it shares titles only: the
// question's text, the last message and why it seems stuck.
export function remoteAgents(raw) {
  const out = [];
  for (const a of list(raw)) {
    const host = typeof a?.host === "string" ? a.host : null;
    if (!HOST_ID.test(host ?? "") || !AGENT_ID.test(a?.id ?? "")) continue;
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
      projectKey: words(a.projectKey, 300),
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

// An invite's answer (POST /api/net/invite, or replace): the line to run
// on the new machine, the part of it that's secret (the code), and when it
// lapses. When the invite names no relay, the line says where to find this
// machine (--at, or --mdns on the LAN). Null if there's no invite.
export function joinOf(data, now = Date.now()) {
  if (!data || typeof data !== "object") return null;
  const invite = typeof data.invite === "string" && data.invite.startsWith("xni2_") ? data.invite : null;
  const code = typeof data.code === "string" && data.code ? data.code : null;
  if (!invite || !code) return null;
  const at = list(data.at).filter((a) => typeof a === "string" && a);
  const how = data.relay ? "" : data.lan ? " --mdns" : at.length ? ` --at ${at[0]}` : "";
  return { line: `xenonet net join ${invite} ${code}${how}`, secret: code, expires: Number.isFinite(data.expires) ? data.expires : now + INVITE_MS };
}

// The invite, its code and where to find the machine that made it, from
// what was typed or pasted: either one, or the whole line Add a machine
// showed.
export function joinFields(invite, code) {
  const m = /^\s*xenonet\s+net\s+join\s+(\S+)\s+(\S+)(?:\s+--at\s+(\S+)|\s+(--mdns))?\s*$/.exec(invite ?? "");
  if (m) return { invite: m[1], code: m[2], at: m[3] ?? null, lan: !!m[4] };
  return { invite: String(invite ?? "").trim(), code: String(code ?? "").trim(), at: null, lan: false };
}

// What a standing delegation is, as the console says it where it's set:
// who put the machine on it and when, what runs without a question, and
// what is always one (delegation.rs; docs/guide/delegation.md).
export function delegationWords(d, now = Date.now()) {
  const who = d?.by ? `Delegated by ${d.by}${Number.isFinite(d.at) ? `, ${clock(d.at, now)}` : ""}.` : "Delegated.";
  return `${who} Sessions and actors that ask this machine for work run without asking about reading, editing files under the folder they start in, routine commands and XenoNet's own update and doctor. Anything else, and always what deletes, handles secrets, reaches the network or spends money, comes to your console as a question.`;
}
