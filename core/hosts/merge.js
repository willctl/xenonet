// Other machines in this XenoNet's view (docs/design/hosts.md, "Ids and the
// graph"): pure functions over what hosts sent and what this XenoNet has.
//
// Every id from a host becomes @<host>:<its id>. An id that already starts
// with @ is dropped, with anything that points at it, so a host can't pass
// on another host's nodes or pose as this machine; the host's core becomes
// its host node, joined to this machine's core, and everything else it
// sent hangs from that node in one tree, however its links were made.
// Nodes are checked one at a time against graph/v1 and dropped when they
// don't fit, and a host has at most 5,000 on the map.
//
// Nothing a host sent is passed on as it came. Its nodes, live sessions,
// activity, identities and engine records are copied with only the fields
// their schemas declare (declared.js), and the rest of what it sends is
// rebuilt field by field, so a host can't slip in a field the page reads
// without escaping.
//
// What's yours stays the console's: its layout, under the qualified ids,
// over the host's; names and hidden identities over the host's. Actions
// that would open something on this machine are cut to Preview and Stats.
// Times move by the host's clock offset, and activity is worked out again
// by this machine's clock, so an offline host's sessions don't look busy
// forever.
import { activityOf } from "../graph.js";
import { repoKey, qualify } from "../schema/ids.js";
import { GRAPH_NODES } from "../schema/graph.js";
import { Identity } from "../schema/identity.js";
import { LiveSession, LiveActivity } from "../schema/live.js";
import { EngineAgent, EngineMessage } from "../schema/engine.js";
import { declared } from "./declared.js";

export const MAX_NODES = 5000;
export const DEFAULT_WINDOWS = { activeWindowMs: 10 * 60e3, recentWindowMs: 24 * 3600e3, projectActiveMs: 3600e3 };
const MAX_ID = 2048;
// The most of anything else one host may put on the map, so a host can't
// make this XenoNet check or draw without end: live sessions, collisions and
// shared files, relations, identities, and the engine's agents and
// messages.
const MAX_LIVE = 1000;
const MAX_PAIRS = 200;
const MAX_RELATIONS = 4 * MAX_NODES;
const MAX_IDENTITIES = 500;
const MAX_ENGINE = 500;
const first = (list, n) => (Array.isArray(list) ? list.slice(0, n) : []);
const LINKS = new Set(["contains", "nested", "satellite", "native", "zone", "session", "lineage", "lineage-cross", "wiki", "page", "resource"]);
const RELATIONS = new Set(["wikilink", "mentions"]);
// How a node whose parent didn't come hangs from its host node.
const UP = { session: "session", project: "contains", native: "native", zone: "zone", wiki: "wiki", page: "page", resource: "resource" };
const NODE_TIMES = ["createdAt", "updatedAt", "lastActivity", "lastSessionAt", "modifiedAt", "updated"];
const REMOTE_ACTIONS = [
  { id: "preview", label: "Preview transcript", client: true },
  { id: "stats", label: "Stats", client: true },
];
// How many bridges one repo gets, however many checkouts it has.
const BRIDGES_PER_REPO = 8;

const isObject = (v) => v != null && typeof v === "object" && !Array.isArray(v);
const num = (v) => (Number.isFinite(v) ? v : 0);
const str = (v, max = 400) => (typeof v === "string" ? v.slice(0, max) : null);
// A value kept under a name a host chose (a node's kind, an identity's id),
// looked up as the object's own: a name every object has, such as toString
// or __proto__, finds nothing rather than what Object gives it.
const own = (o, k) => (typeof k === "string" && Object.hasOwn(o, k) ? o[k] : undefined);

// A host's id as this XenoNet writes it (qualify, in core/schema/ids.js), or
// null for one that can't be the host's own: not a string, too long, or
// already qualified, which qualify would keep and this drops.
export function remoteId(host, id) {
  return typeof id === "string" && id && id.length <= MAX_ID && !id.startsWith("@") ? qualify(id, host) : null;
}

// An identity's id from a host: account ids are global, so they stay as
// they are, except openai:api-key, which is that machine's own key
// (contracts, section 2).
export function identityId(host, id) {
  if (typeof id !== "string" || !id || id.length > 512 || id.startsWith("@")) return null;
  return id === "openai:api-key" ? qualify(id, host) : id;
}

const placed = (n, layout) => {
  const p = layout[n.id];
  return Object.assign(n, p && [p.x, p.y, p.z].every(Number.isFinite) ? { fx: p.x, fy: p.y, fz: p.z, pinned: true } : { fx: null, fy: null, fz: null, pinned: false });
};

// A node a host sent, as this XenoNet keeps it: checked against graph/v1 for
// its kind, and copied with only the fields that declares (declared.js), so
// a host can't add one the page doesn't know to escape; null for a node
// that doesn't fit.
export function cleanNode(n) {
  const schema = isObject(n) ? own(GRAPH_NODES, n.kind) : undefined;
  return schema?.safeParse(n).success ? declared(schema, n) : null;
}

// cleanNode, remembered for one host: by the node itself, since the same
// map is merged again whenever this machine's changes, and by its text,
// since a host sends its whole map with each change and only a few of its
// nodes differ each time. Only those are checked and copied again. What it
// gives is shared between merges, so it's copied before it's changed.
export function cleanOnce() {
  const seen = new WeakMap();
  let known = new Map();
  let next = new Map();
  const clean = (n) => {
    if (seen.has(n)) {
      if (known.has(n.id)) next.set(n.id, known.get(n.id));
      return seen.get(n);
    }
    const text = JSON.stringify(n);
    const hit = known.get(n.id);
    const kept = hit && hit.text === text ? hit.kept : cleanNode(n);
    next.set(n.id, { text, kept });
    seen.set(n, kept);
    return kept;
  };
  // What the last map had, kept for the next; the rest forgotten.
  clean.done = () => ((known = next), (next = new Map()));
  return clean;
}

// One host's view, qualified: its host node first, then its nodes, links,
// relations and identities, and its stats for adding up.
export function qualifyGraph(graph, host, { layout = {}, now = Date.now(), windows = DEFAULT_WINDOWS, clean = cleanNode } = {}) {
  const h = host.id;
  const offset = Number.isFinite(host.offsetMs) ? host.offsetMs : 0;
  const shift = (t) => (Number.isFinite(t) ? t - offset : null);
  const stale = host.state !== "live";
  const sent = Array.isArray(graph?.nodes) ? graph.nodes : [];
  const core = sent.find((n) => isObject(n) && n.id === "core" && n.kind === "core") ?? null;
  let dropped = 0;
  let candidates = [];
  for (const n of sent) {
    if (n === core) continue;
    const kept = isObject(n) && n.id !== "core" && remoteId(h, n.id) ? clean(n) : null;
    if (kept) candidates.push(kept);
    else dropped++;
  }
  clean.done?.();
  let truncated = false;
  if (candidates.length > MAX_NODES - 1) {
    truncated = true;
    const places = candidates.filter((n) => n.kind !== "session");
    const sessions = candidates.filter((n) => n.kind === "session").sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));
    candidates = [...places, ...sessions].slice(0, MAX_NODES - 1);
  }

  const nodes = new Map();
  for (const n of candidates) {
    const id = remoteId(h, n.id);
    if (nodes.has(id)) continue;
    // n is clean's copy, with the declared fields only; this one is made
    // from it, since n may be merged again.
    const out = { ...n, id, host: h, stale };
    for (const k of NODE_TIMES) if (k in out) out[k] = shift(out[k]);
    if (isObject(out.repo)) out.repo = { ...out.repo, checkedAt: shift(out.repo.checkedAt) };
    if (Array.isArray(out.writes)) out.writes = out.writes.map((w) => ({ ...w, ts: shift(w.ts) }));
    if (out.kind === "session") {
      out.parentId = out.parentId == null ? null : remoteId(h, out.parentId);
      if (!out.parentId) out.parentKind = null;
      const a = activityOf(out.updatedAt, now, windows);
      out.activity = out.archived && a === "active" ? "recent" : a;
      if (isObject(out.ranAs)) {
        const ran = identityId(h, out.ranAs.id);
        out.ranAs = ran ? { ...out.ranAs, id: ran } : null;
      }
      out.actions = REMOTE_ACTIONS.map((x) => ({ ...x }));
    } else out.actions = [];
    if (out.kind === "project" && Array.isArray(out.identities)) out.identities = out.identities.map((l) => ({ ...l, id: identityId(h, l.id) })).filter((l) => l.id);
    nodes.set(id, placed(out, layout));
  }

  const hostId = `@${h}:core`;
  const hostNode = placed(
    {
      id: hostId,
      kind: "host",
      host: h,
      name: h,
      path: str(core?.name, 200),
      calls: host.hello?.id && host.hello.id !== h ? host.hello.id : null,
      state: host.state,
      since: host.since,
      lastHeard: host.lastHeard ?? null,
      offsetMs: Number.isFinite(host.offsetMs) ? host.offsetMs : null,
      version: host.hello?.version ?? null,
      platform: host.hello?.platform ?? null,
      demo: host.hello?.demo ?? null,
      error: host.error?.message ?? null,
      cached: !!host.cached,
      truncated,
      stale,
      sessionCount: Number.isInteger(core?.sessionCount) && core.sessionCount >= 0 ? core.sessionCount : 0,
      activeCount: 0,
      lastActivity: shift(core?.lastActivity),
      lastSessionAt: shift(core?.lastSessionAt),
      actions: [],
    },
    layout,
  );

  // One tree under the host node: each node keeps the first parent the
  // host gave it, and a node whose parent didn't come, or that a loop of
  // links cut off, hangs from the host node itself.
  const parentOf = new Map();
  const cross = [];
  for (const l of Array.isArray(graph?.links) ? graph.links : []) {
    if (!isObject(l) || !LINKS.has(l.kind)) continue;
    const source = l.source === "core" ? hostId : remoteId(h, l.source);
    const target = remoteId(h, l.target);
    if (!source || !target || !nodes.has(target) || !(source === hostId || nodes.has(source)) || source === target) continue;
    if (l.kind === "lineage-cross") cross.length < MAX_NODES && cross.push({ source, target, kind: l.kind });
    else if (!parentOf.has(target)) parentOf.set(target, { source, target, kind: l.kind });
  }
  const children = new Map();
  for (const l of parentOf.values()) (children.get(l.source) ?? children.set(l.source, []).get(l.source)).push(l.target);
  const reached = new Set();
  const reach = (from) => {
    const queue = [from];
    reached.add(from);
    while (queue.length) for (const c of children.get(queue.shift()) ?? []) if (!reached.has(c)) (reached.add(c), queue.push(c));
  };
  reach(hostId);
  for (const n of nodes.values()) {
    if (reached.has(n.id)) continue;
    parentOf.set(n.id, { source: hostId, target: n.id, kind: UP[n.kind] });
    (children.get(hostId) ?? children.set(hostId, []).get(hostId)).push(n.id);
    reach(n.id);
  }
  const links = [{ source: "core", target: hostId, kind: "host" }, ...parentOf.values(), ...cross];

  const relations = [];
  for (const r of first(graph?.relations, MAX_RELATIONS)) {
    if (!isObject(r) || !RELATIONS.has(r.kind)) continue;
    const a = remoteId(h, r.a);
    const b = remoteId(h, r.b);
    if (a && b && nodes.has(a) && nodes.has(b)) relations.push({ a, b, kind: r.kind });
  }

  // What's active, by this machine's clock.
  const places = [hostNode, ...nodes.values()].filter((n) => n.kind !== "session");
  for (const n of places) if (n !== hostNode) n.activeCount = 0;
  const up = new Map([...parentOf.values()].map((l) => [l.target, l.source]));
  for (const s of nodes.values()) {
    if (s.kind !== "session" || s.activity !== "active") continue;
    const seen = new Set();
    for (let cur = up.get(s.id); cur && !seen.has(cur); cur = up.get(cur)) {
      seen.add(cur);
      const n = cur === hostId ? hostNode : nodes.get(cur);
      if (n && n.kind !== "session") n.activeCount++;
    }
  }
  for (const n of nodes.values()) if (n.kind === "project" || n.kind === "native" || n.kind === "zone") n.active = n.lastSessionAt != null && now - n.lastSessionAt <= (windows.projectActiveMs ?? 3600e3);

  const identities = [];
  for (const i of first(graph?.identities, MAX_IDENTITIES)) {
    const id = identityId(h, i?.id);
    if (id && Identity.safeParse(i).success) identities.push(Object.assign(declared(Identity, i), { id }));
  }

  const all = [...nodes.values()];
  const sessions = all.filter((n) => n.kind === "session");
  const projects = all.filter((n) => n.kind === "project");
  const stats = {
    wikiPages: all.filter((n) => n.kind === "page").length,
    dirtyRepos: projects.filter((n) => n.repo?.dirty > 0).length,
    projects: projects.filter((n) => !n.group).length,
    groups: projects.filter((n) => n.group).length,
    sessions: sessions.length,
    claude: sessions.filter((n) => n.app === "claude").length,
    codex: sessions.filter((n) => n.app === "codex").length,
    active: sessions.filter((n) => n.activity === "active").length,
    archived: sessions.filter((n) => n.archived).length,
  };
  return { hostNode, nodes: [hostNode, ...all], links, relations, identities, stats, truncated, dropped };
}

// Bridges: a same-repo relation between checkouts of one repo on
// different machines, by repo key. A worktree or a project beside its repo
// is left to the checkout it belongs to, and one repo gets a few at most.
export function sameRepo(nodes, { cap = BRIDGES_PER_REPO } = {}) {
  const repos = new Map(); // key → host → [ids]
  for (const n of nodes) {
    if (n.kind !== "project" || n.group || n.satelliteOf || typeof n.repoKey !== "string" || !n.repoKey) continue;
    const byHost = repos.get(n.repoKey) ?? repos.set(n.repoKey, new Map()).get(n.repoKey);
    const host = n.host ?? "";
    (byHost.get(host) ?? byHost.set(host, []).get(host)).push(n.id);
  }
  const out = [];
  for (const byHost of repos.values()) {
    const groups = [...byHost.values()];
    let made = 0;
    for (let i = 0; i < groups.length; i++)
      for (let j = i + 1; j < groups.length; j++)
        for (const a of groups[i])
          for (const b of groups[j]) if (made++ < cap) out.push({ a, b, kind: "same-repo" });
  }
  return out;
}

// Identities from every machine, one per id: counts add up, a handle comes
// from whichever machine knows it, and `hosts` and `signedInOn` say where
// (null for this one). Your names and hidden ones apply to every machine's,
// and a name given on a host fills in where you gave none.
export function mergeIdentities(local, remotes, { names = {}, hidden = {} } = {}) {
  const byId = new Map(local.map((i) => [i.id, { ...i, sources: [...(i.sources ?? [])], hosts: [null], signedInOn: i.signedIn ? [null] : [] }]));
  for (const { host, identities } of remotes) {
    for (const r of identities) {
      const sources = (r.sources ?? []).filter((s) => typeof s === "string").map((s) => `${host}: ${s}`);
      const cur = byId.get(r.id);
      if (!cur) {
        byId.set(r.id, { ...r, sources, name: own(names, r.id) ?? r.name ?? null, hidden: !!own(hidden, r.id), hosts: [host], signedInOn: r.signedIn ? [host] : [] });
        continue;
      }
      cur.sessions = num(cur.sessions) + num(r.sessions);
      cur.projects = num(cur.projects) + num(r.projects);
      cur.handle ??= r.handle ?? null;
      cur.detail = { ...(isObject(r.detail) ? r.detail : {}), ...cur.detail };
      cur.name ??= own(names, r.id) ?? r.name ?? null;
      cur.active = !!cur.active || !!r.active;
      if (r.signedIn) (cur.signedIn = true), cur.signedInOn.push(host);
      if (Number.isFinite(r.firstSeen)) cur.firstSeen = Math.min(cur.firstSeen ?? r.firstSeen, r.firstSeen);
      if (Number.isFinite(r.seenAt)) cur.seenAt = Math.max(cur.seenAt ?? 0, r.seenAt);
      cur.hosts.push(host);
      cur.sources.push(...sources);
    }
  }
  return [...byId.values()].sort((a, b) => b.sessions - a.sessions || b.projects - a.projects || a.id.localeCompare(b.id));
}

// How a host stands, for graph.hosts and /api/hosts.
export function hostSummary(host, part = null) {
  return {
    id: host.id,
    state: host.state,
    since: host.since,
    lastHeard: host.lastHeard ?? null,
    offsetMs: Number.isFinite(host.offsetMs) ? host.offsetMs : null,
    version: host.hello?.version ?? null,
    platform: host.hello?.platform ?? null,
    demo: host.hello?.demo ?? null,
    error: host.error?.message ?? null,
    cached: !!host.cached,
    truncated: !!part?.truncated,
    nodes: part ? part.nodes.length - 1 : 0,
    sessions: part?.stats.sessions ?? 0,
  };
}

// This XenoNet's map with every host's in it. `hosts` is [{ info, snapshot,
// clean? }]: how each host stands (link.info(), with cached when its
// snapshot came from the cache), the last snapshot it sent or null, and
// how its nodes are checked and copied (cleanOnce, per host). This
// machine's projects get their repo keys here, from its own SSH aliases.
// With no hosts, the map is this XenoNet's own, as it was.
export function mergeGraph(local, hosts, { layout = {}, now = Date.now(), windows = DEFAULT_WINDOWS, self = null, names = {}, hidden = {}, aliases = new Map() } = {}) {
  if (!hosts.length) return local;
  const parts = hosts.map((h) => qualifyGraph(h.snapshot?.graph ?? { nodes: [], links: [] }, h.info, { layout, now, windows, clean: h.clean }));
  const nodes = local.nodes.map((n) => {
    if (n.kind !== "project") return n;
    const key = repoKey(n.git?.origin, aliases);
    return key ? { ...n, repoKey: key } : n;
  });
  for (const p of parts) nodes.push(...p.nodes);
  const stats = { ...local.stats };
  for (const p of parts) for (const [k, v] of Object.entries(p.stats)) stats[k] = num(stats[k]) + v;
  return {
    ...local,
    nodes,
    links: [...local.links, ...parts.flatMap((p) => p.links)],
    relations: [...local.relations, ...parts.flatMap((p) => p.relations), ...sameRepo(nodes)],
    stats,
    identities: mergeIdentities(local.identities ?? [], hosts.map((h, i) => ({ host: h.info.id, identities: parts[i].identities })), { names, hidden }),
    hosts: hosts.map((h, i) => hostSummary(h.info, parts[i])),
    self,
  };
}

// ---------- live state ----------

// A host's session's times, moved by its clock's offset from this one's.
const LIVE_TIMES = ["turnStartedAt", "lastTurnEndAt", "interruptedAt", "lastEventAt"];

// A host's collision, or its sessions sharing files (`shared`), rebuilt
// from the fields live/snapshot/v1 gives them and nothing else; null for a
// collision with no time it happened.
function pairOf(c, h, shift, shared = false) {
  if (!isObject(c)) return null;
  const a = remoteId(h, c.a);
  const b = remoteId(h, c.b);
  if (!a || !b) return null;
  const strings = (v) => (Array.isArray(v) ? v.filter((x) => typeof x === "string").slice(0, 50) : []);
  if (shared) return { a, b, kind: "file", files: strings(c.files) };
  const ts = shift(c.ts);
  if (ts == null) return null;
  return { a, b, files: strings(c.files), paths: strings(c.paths), ts, branches: [str(c.branches?.[0], 200), str(c.branches?.[1], 200)], sameBranch: c.sameBranch === true, titles: [str(c.titles?.[0]) ?? a, str(c.titles?.[1]) ?? b] };
}

// A host's plan limit reading, rebuilt from the fields that fit, with its
// times moved; null for one that has no time it was read.
function readingOf(app, r, shift) {
  if (!isObject(r)) return null;
  const n = (v) => (Number.isFinite(v) ? v : null);
  const observedAt = shift(r.observedAt);
  if (app === "claude" && r.stale === true) return { fiveHour: null, sevenDay: null, observedAt, stale: true, reason: str(r.reason, 200) };
  if (observedAt == null) return null;
  if (app === "claude") return { fiveHour: n(r.fiveHour), sevenDay: n(r.sevenDay), resets: { fiveHour: shift(r.resets?.fiveHour), sevenDay: shift(r.resets?.sevenDay) }, observedAt, source: r.source === "claude-desktop" ? "claude-desktop" : "anthropic" };
  const window = (w) => (isObject(w) ? { pct: n(w.pct), windowMinutes: n(w.windowMinutes), resetsAt: shift(w.resetsAt) } : null);
  return { primary: window(r.primary), secondary: window(r.secondary), observedAt };
}

// One host's live state, qualified. A host that isn't live shows its
// sessions idle, its rates at nothing and a question that was waiting as
// of when it was last heard; its collisions are dropped.
export function qualifyLive(live, host) {
  const h = host.id;
  const offset = Number.isFinite(host.offsetMs) ? host.offsetMs : 0;
  const shift = (t) => (Number.isFinite(t) ? t - offset : null);
  const stale = host.state !== "live";
  const sessions = [];
  for (const s of first(live?.sessions, MAX_LIVE)) {
    const id = remoteId(h, s?.id);
    if (!id || !LiveSession.safeParse(s).success) continue;
    const out = Object.assign(declared(LiveSession, s), { id, host: h, stale });
    for (const k of LIVE_TIMES) if (k in out) out[k] = shift(out[k]);
    if (out.needsYou) out.needsYou = { ...out.needsYou, since: shift(out.needsYou.since), ...(stale ? { asOf: host.lastHeard ?? null } : {}) };
    if (out.lastTool) out.lastTool = { ...out.lastTool, ts: shift(out.lastTool.ts) };
    out.files = out.files.map((f) => ({ ...f, ts: shift(f.ts) }));
    out.mcp = out.mcp.map((m) => ({ ...m, ts: shift(m.ts) }));
    out.subagents = out.subagents.map((a) => ({ ...a, startedAt: shift(a.startedAt), lastAt: shift(a.lastAt) }));
    if (stale) Object.assign(out, { status: "idle", tpm: 0, stuck: null, turnStartedAt: null });
    sessions.push(out);
  }
  const pairs = (list, shared = false) => (stale ? [] : first(list, MAX_PAIRS).map((c) => pairOf(c, h, shift, shared)).filter(Boolean));
  const readings = [];
  for (const app of ["claude", "codex"]) {
    const reading = readingOf(app, live?.limits?.[app], shift);
    if (reading) readings.push({ app, host: h, reading: { ...reading, identity: identityId(h, live.limits[app].identity) } });
  }
  return {
    sessions,
    collisions: pairs(live?.collisions),
    collab: pairs(live?.collab, true),
    series: stale || !isObject(live?.series) ? null : live.series,
    at: shift(live?.at),
    tpm: stale ? 0 : num(live?.tpm),
    today: num(live?.today),
    total24h: num(live?.total24h),
    readings,
  };
}

// Adds a host's per-second series into this machine's, lined up by time:
// both end at their snapshot's moment, and `lag` is how many seconds the
// host's ends before this one's.
function addSeries(into, from, lag) {
  for (const app of ["claude", "codex"]) {
    const a = into[app];
    const b = Array.isArray(from?.[app]) ? from[app] : [];
    if (!Array.isArray(a)) continue;
    for (let i = 0; i < b.length; i++) {
      const j = a.length - b.length + i - lag;
      if (j >= 0 && j < a.length && Number.isFinite(b[i])) a[j] += b[i];
    }
  }
}

// Plan limits are an account's, not a machine's: a newer reading of this
// machine's account from another one takes its place, and accounts signed
// in only elsewhere are kept as their own, the newest of each.
export function mergeLimits(local, readings) {
  const limits = { ...local };
  const others = new Map();
  for (const { app, host, reading } of readings) {
    const mine = limits[app];
    if (mine && reading.identity && mine.identity === reading.identity) {
      if (!reading.stale && (reading.observedAt ?? -Infinity) > (mine.observedAt ?? -Infinity)) limits[app] = { ...reading, host };
      continue;
    }
    const key = `${app} ${reading.identity ?? `@${host}`}`;
    const prev = others.get(key);
    if (!prev || (reading.observedAt ?? -Infinity) > (prev.reading.observedAt ?? -Infinity)) others.set(key, { app, identity: reading.identity ?? null, host, reading });
  }
  limits.others = [...others.values()];
  return limits;
}

// This XenoNet's live state with every host's in it.
export function mergeLive(local, hosts) {
  if (!hosts.length) return local;
  const out = {
    ...local,
    sessions: [...(local.sessions ?? [])],
    collisions: [...(local.collisions ?? [])],
    collab: [...(local.collab ?? [])],
    series: { claude: [...(local.series?.claude ?? [])], codex: [...(local.series?.codex ?? [])] },
  };
  const readings = [];
  for (const h of hosts) {
    if (!h.snapshot?.live) continue;
    const q = qualifyLive(h.snapshot.live, h.info);
    out.sessions.push(...q.sessions);
    out.collisions.push(...q.collisions);
    out.collab.push(...q.collab);
    out.tpm = num(out.tpm) + q.tpm;
    out.today = num(out.today) + q.today;
    out.total24h = num(out.total24h) + q.total24h;
    if (q.series && Number.isFinite(q.at) && Number.isFinite(local.at)) addSeries(out.series, q.series, Math.round((local.at - q.at) / 1000));
    readings.push(...q.readings);
  }
  out.limits = mergeLimits(local.limits ?? {}, readings);
  return out;
}

// The summary the tray, the menu bar and xenonet status read (state.summary)
// with every live host's in it: what needs you and what's stuck there,
// each with its host and its title from the host's map, and what's working
// and colliding counted in. A host that isn't live stays out, as its
// alerts stay out of the page's count: what it showed may be over.
export function mergeSummary(local, hosts) {
  if (!hosts.length) return local;
  const out = { ...local, needsYou: [...(local.needsYou ?? [])], stuck: [...(local.stuck ?? [])] };
  for (const h of hosts) {
    if (h.info.state !== "live" || !h.snapshot?.live) continue;
    const q = qualifyLive(h.snapshot.live, h.info);
    const titles = new Map();
    for (const n of first(h.snapshot.graph?.nodes, MAX_NODES)) if (isObject(n) && n.kind === "session") titles.set(remoteId(h.info.id, n.id), str(n.title, 200));
    const title = (id) => titles.get(id) ?? id;
    for (const s of q.sessions) {
      if (s.needsYou && s.needsYou.kind !== "review") out.needsYou.push({ id: s.id, app: s.app, title: title(s.id), kind: s.needsYou.kind, text: s.needsYou.text, host: h.info.id });
      if (s.stuck) out.stuck.push({ id: s.id, title: title(s.id), reasons: s.stuck.reasons, host: h.info.id });
    }
    out.working = num(out.working) + q.sessions.filter((s) => s.status !== "idle").length;
    out.review = num(out.review) + q.sessions.filter((s) => s.needsYou?.kind === "review").length;
    out.collisions = num(out.collisions) + q.collisions.length;
    out.tpm = num(out.tpm) + q.tpm;
  }
  return out;
}

// ---------- events and the engine ----------

// One of a host's events as this XenoNet passes it on, with its ids
// qualified, its time moved and only the fields its kind has, or null for
// one it doesn't.
export function qualifyEvent(name, data, host) {
  const h = host.id;
  const offset = Number.isFinite(host.offsetMs) ? host.offsetMs : 0;
  const shift = (t) => (Number.isFinite(t) ? t - offset : null);
  if (!isObject(data)) return null;
  switch (name) {
    case "activity": {
      const sessionId = remoteId(h, data.sessionId);
      if (!sessionId || !LiveActivity.safeParse(data).success) return null;
      const out = Object.assign(declared(LiveActivity, data), { sessionId, ts: shift(data.ts), host: h });
      if ("projectId" in out) {
        out.projectId = remoteId(h, out.projectId);
        if (!out.projectId) return null;
      }
      return out;
    }
    case "tokens": {
      const sessionId = remoteId(h, data.sessionId);
      return sessionId && Number.isFinite(data.n) ? { sessionId, app: data.app === "codex" ? "codex" : "claude", n: data.n, ts: shift(data.ts), host: h } : null;
    }
    case "pulse": {
      const nodeId = data.nodeId === "core" ? `@${h}:core` : remoteId(h, data.nodeId);
      return nodeId ? { nodeId, reason: str(data.reason, 40) ?? "updated", host: h } : null;
    }
    case "agent-message": {
      const from = remoteId(h, data.from);
      if (!from || (!Number.isSafeInteger(data.id) && typeof data.id !== "string")) return null;
      return { id: `@${h}:${String(data.id).slice(0, 64)}`, from, to: (Array.isArray(data.to) ? data.to : []).map((x) => remoteId(h, x)).filter(Boolean), target: str(data.target, 200), text: str(data.text, 4000) ?? "", ts: shift(data.ts), host: h };
    }
    default:
      return null;
  }
}

// A host's engine (its agents, what each says it's doing, the last day's
// messages) under qualified ids, for showing in its constellation: the
// agents and messages that fit engine/agent/v1 and engine/message/v1, with
// only the fields those declare.
export function qualifyEngine(engine, host) {
  const h = host.id;
  const agents = [];
  for (const a of first(engine?.agents, MAX_ENGINE)) {
    const id = isObject(a) ? remoteId(h, a.id) : null;
    if (id && EngineAgent.safeParse(a).success) agents.push(Object.assign(declared(EngineAgent, a), { id }));
  }
  const messages = [];
  for (const m of first(engine?.messages, MAX_ENGINE)) {
    const from = isObject(m) ? remoteId(h, m.from) : null;
    if (!from || !EngineMessage.safeParse(m).success) continue;
    const kept = declared(EngineMessage, m);
    const to = (Array.isArray(kept.to) ? kept.to : []).filter(isObject).map((d) => ({ ...d, id: remoteId(h, d.id) })).filter((d) => d.id);
    messages.push(Object.assign(kept, { id: `@${h}:${String(m.id).slice(0, 64)}`, from, to }));
  }
  return { agents, messages };
}
