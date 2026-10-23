// Other machines on the map (docs/design/hosts.md): over SSH with
// XENONET_HOSTS=1, and the machines of your network, drawn from their
// presence (network-map.js makes their nodes). Each host is a
// constellation of its own beside this machine's: its host node
// (@<host>:core) held on an outer orbit, and its projects and sessions on
// rings of their own around it. A dashed bridge joins checkouts of one
// repo on different machines (hosts-draw.js draws those and the host node).
// A host that isn't live keeps its constellation in place, dimmed and
// still. A host over SSH is read-only: Preview and Stats, nothing that
// opens, dismisses or messages anything on it. A network's sessions take a
// message from you, and nothing else.
//
// This module holds what isn't three.js: the layout's forces, the words
// for how a host stands, the chips under Filters, the cap on what's drawn
// of the hosts you're not looking at, and the bits of the inspector, the
// tooltip and the alert strip that say where a node lives. Every piece of
// text from a host is escaped before it reaches the page. With no host on
// the map nothing here runs, and the map is as it was.
import { escapeHtml as h, stamp } from "./format.js";

// The host node's size (sizeFor in nodes.js).
export const HOST_SIZE = 5;
// This machine's rings are 72 apart (dagLevelDistance in scene.js); a
// host's are this much of that, so it reads as a smaller system.
export const HOST_SCALE = 0.62;
const LEVEL = 72;
// Room between this machine's map and a host's, and between hosts.
const GAP = 120;
// How lit a host's nodes are while it isn't live.
export const STALE_LIGHT = 0.42;
// This machine, in the host filter; never a host's id, which can't start
// with @.
export const HERE = "@here";

const idOf = (end) => (end && typeof end === "object" ? end.id : end);

export const isRemote = (n) => typeof n?.host === "string" && n.host !== "";
export const hostNodeId = (host) => `@${host}:core`;
// The host a qualified id is from (contracts, section 2), or null.
export const hostOfId = (id) => (typeof id === "string" && id.startsWith("@") && id.indexOf(":") > 1 ? id.slice(1, id.indexOf(":")) : null);
const keyOf = (n) => (isRemote(n) ? n.host : HERE);

// ---------- how a host stands ----------

// A time as the console shows it: the clock today, the date before that.
function when(ms, now = Date.now()) {
  if (!Number.isFinite(ms)) return "—";
  const d = new Date(ms);
  return d.toDateString() === new Date(now).toDateString() ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : stamp(ms);
}

// How a host stands, in a few words: "live", "last seen 14:02"…
export function standing(host, now = Date.now()) {
  if (!host) return "";
  const seen = host.lastHeard ? `last seen ${when(host.lastHeard, now)}` : null;
  switch (host.state) {
    case "live":
      return "live";
    case "connecting":
      return seen ? `connecting · ${seen}` : "connecting";
    case "stale":
      return `no word since ${when(host.lastHeard, now)}`;
    case "offline":
      return seen ?? "offline";
    case "error":
      return "needs you";
    default:
      return String(host.state ?? "");
  }
}

// The same for the host node's label on the map, short and in capitals.
export function labelOf(host, now = Date.now()) {
  const name = String(host?.name ?? host?.host ?? "").toUpperCase();
  const s = host?.state === "live" ? "live" : host?.state === "error" ? "needs you" : host?.state === "connecting" ? "connecting" : host?.lastHeard ? `seen ${when(host.lastHeard, now)}` : String(host?.state ?? "");
  return `${name}\n${s.toUpperCase()}`;
}

// What changes the host node's look, for scene.js's rebuild check: its
// state and, while it isn't live, when it was last heard (which doesn't
// move then). For this machine's core, the name it shows beside its folder.
// A host's other nodes look the same either way: they dim and hold still
// frame by frame (scene.js), so a host going quiet rebuilds nothing else.
export function hostVisual(n) {
  if (n.kind === "host") return `${n.state}|${n.state === "live" ? "" : n.lastHeard}|${n.stale}`;
  if (n.kind === "core") return n.selfName ?? "";
  return "";
}

// ---------- the layout: each host a constellation of its own ----------

// How deep each node sits under its roots, the longest way down, as the
// force graph's own layout counts it, for the nodes `mine` lets through.
export function depthsOf(nodes, links, mine) {
  const indeg = new Map();
  for (const n of nodes) if (mine(n)) indeg.set(n.id, 0);
  const kids = new Map();
  for (const l of links) {
    if (l.kind === "host") continue;
    const s = idOf(l.source), t = idOf(l.target);
    if (s === t || !indeg.has(s) || !indeg.has(t)) continue;
    (kids.get(s) ?? kids.set(s, []).get(s)).push(t);
    indeg.set(t, indeg.get(t) + 1);
  }
  const depth = new Map();
  const queue = [];
  for (const [id, d] of indeg) if (!d) queue.push(id), depth.set(id, 0);
  for (let i = 0; i < queue.length; i++) {
    const d = depth.get(queue[i]);
    for (const k of kids.get(queue[i]) ?? []) {
      if ((depth.get(k) ?? -1) < d + 1) depth.set(k, d + 1);
      indeg.set(k, indeg.get(k) - 1);
      if (!indeg.get(k)) queue.push(k);
    }
  }
  // A loop of links leaves its nodes out of that order; they sit one ring
  // further out than whatever reached them, or on the first if nothing did.
  for (const id of indeg.keys()) if (!depth.has(id)) depth.set(id, 1);
  return depth;
}

// Where each host's constellation goes, and the forces that keep it there.
// This machine's map stays at the center, laid out as before; the force
// graph's radial layout leaves remote nodes alone (filter), each host node
// is held on its anchor, on an orbit clear of this machine's map and of
// the other hosts, and the host's nodes keep to rings of their own around
// it (force). `radial` says whether the map is in its radial layout; in
// the free one a host's nodes float around their host node as the rest do.
export function hostLayout({ level = LEVEL, scale = HOST_SCALE, gap = GAP, radial = () => true, random = Math.random } = {}) {
  let systems = []; // { host, node, anchor, radius, members: [{ n, r }] }
  const step = level * scale;

  const filter = (n) => !isRemote(n);

  // Called with the map before the force graph gets it: works out each
  // host's rings and anchor, holds its host node there (unless you pinned
  // it somewhere), and starts new nodes near their host rather than at the
  // center, so they don't fly through this machine's map. Says whether
  // there's a host on the map.
  function prepare(nodes, links) {
    const hostNodes = nodes.filter((n) => n.kind === "host" && isRemote(n)).sort((a, b) => (a.host < b.host ? -1 : a.host > b.host ? 1 : 0));
    if (!hostNodes.length) {
      systems = [];
      return false;
    }
    let localDepth = 1;
    for (const d of depthsOf(nodes, links, filter).values()) if (d > localDepth) localDepth = d;
    const depth = depthsOf(nodes, links, isRemote);
    const byHost = new Map(hostNodes.map((n) => [n.host, { host: n.host, node: n, anchor: null, radius: 0, members: [], deepest: 0 }]));
    for (const n of nodes) {
      const sys = isRemote(n) && n.kind !== "host" ? byHost.get(n.host) : null;
      if (!sys) continue;
      const d = Math.max(1, depth.get(n.id) ?? 1);
      sys.members.push({ n, r: d * step });
      if (d > sys.deepest) sys.deepest = d;
    }
    systems = [...byHost.values()];
    // A host with little on the map still gets room for a few rings, so
    // its anchor doesn't move much as its map arrives.
    for (const s of systems) s.radius = Math.max(3, s.deepest) * step;
    const widest = Math.max(...systems.map((s) => s.radius));
    let orbit = localDepth * level + gap + widest;
    if (systems.length > 2) orbit = Math.max(orbit, (widest + gap / 2) / Math.sin(Math.PI / systems.length));
    systems.forEach((s, i) => {
      const a = (2 * Math.PI * i) / systems.length;
      s.anchor = { x: Math.round(orbit * Math.cos(a)), y: 0, z: Math.round(orbit * Math.sin(a)) };
      const c = s.node;
      if (!(c.pinned && Number.isFinite(c.fx))) {
        const moved = c.fx !== s.anchor.x || c.fy !== s.anchor.y || c.fz !== s.anchor.z;
        Object.assign(c, { fx: s.anchor.x, fy: s.anchor.y, fz: s.anchor.z });
        if (moved || !Number.isFinite(c.x)) Object.assign(c, { x: s.anchor.x, y: s.anchor.y, z: s.anchor.z, vx: 0, vy: 0, vz: 0 });
      }
      const cx = Number.isFinite(c.x) ? c.x : s.anchor.x, cy = Number.isFinite(c.y) ? c.y : s.anchor.y, cz = Number.isFinite(c.z) ? c.z : s.anchor.z;
      for (const { n, r } of s.members) {
        if (Number.isFinite(n.x) && Number.isFinite(n.y) && Number.isFinite(n.z)) continue;
        // A point on its ring, in a random direction.
        const u = random() * 2 - 1, phi = random() * 2 * Math.PI, w = Math.sqrt(1 - u * u);
        Object.assign(n, { x: cx + r * w * Math.cos(phi), y: cy + r * u, z: cz + r * w * Math.sin(phi) });
      }
    });
    return true;
  }

  // Each of a host's nodes is drawn to its ring around the host node, as
  // the radial layout draws this machine's to theirs around the center.
  function force(alpha) {
    if (!systems.length || !radial()) return;
    for (const s of systems) {
      const c = s.node;
      if (!Number.isFinite(c.x)) continue;
      for (const { n, r } of s.members) {
        const dx = n.x - c.x || 1e-6, dy = n.y - c.y || 1e-6, dz = n.z - c.z || 1e-6;
        const d = Math.sqrt(dx * dx + dy * dy + dz * dz);
        const k = ((r - d) * alpha) / d;
        n.vx += dx * k;
        n.vy += dy * k;
        n.vz += dz * k;
      }
    }
  }

  // The force graph keeps the map's middle at the center by moving every
  // node; with hosts on their orbits that would pull this machine's map
  // off center, so only this machine's nodes count, and only they move.
  // With no remote node, it's the force graph's own centering.
  function center() {
    let nodes = [];
    const centering = () => {
      let count = 0, sx = 0, sy = 0, sz = 0;
      for (const n of nodes) if (!isRemote(n)) (sx += n.x || 0), (sy += n.y || 0), (sz += n.z || 0), count++;
      if (!count) return;
      (sx /= count), (sy /= count), (sz /= count);
      for (const n of nodes) {
        if (isRemote(n)) continue;
        if (sx) n.x -= sx;
        if (sy) n.y -= sy;
        if (sz) n.z -= sz;
      }
    };
    centering.initialize = (all) => (nodes = all);
    return centering;
  }

  return {
    filter,
    force,
    center,
    prepare,
    get systems() {
      return systems;
    },
  };
}

// Puts the layout into a force graph, the first time a host is on the map:
// remote nodes out of the radial layout, a host's link to this machine's
// core pulling on neither end (its host node is held on its orbit), the
// rings around each host, and the centering that leaves hosts where they
// are.
export function installHostLayout(graph, layout) {
  graph.dagNodeFilter(layout.filter);
  graph.d3Force("hosts", layout.force);
  graph.d3Force("center", layout.center());
  const link = graph.d3Force("link");
  const strength = link.strength();
  link.strength((l, i, all) => (l.kind === "host" ? 0 : strength(l, i, all)));
}

// ---------- where a node lives ----------

// The host node a remote node hangs from, from an index of the map.
const hostOf = (n, index) => (isRemote(n) ? (index?.get(hostNodeId(n.host)) ?? null) : null);

// "on vm1 · live", for a remote node; "" for one of this machine's.
export function whereText(n, index, now = Date.now()) {
  if (!isRemote(n) || n.kind === "host") return "";
  const host = hostOf(n, index);
  return `on ${n.host} · ${host ? standing(host, now) : n.stale ? "not live" : "live"}`;
}

// The inspector's line saying where a remote node lives, escaped.
export function whereHtml(n, index, now = Date.now()) {
  const text = whereText(n, index, now);
  if (!text) return "";
  const state = hostOf(n, index)?.state ?? (n.stale ? "offline" : "live");
  // A machine in your network shows what its presence says, and takes a
  // message from you; one over SSH is read-only.
  return `<div class="insp-where ${h(state)}"><i class="host-dot ${h(state)}"></i>${h(text)}<span class="dim"> · ${n.network ? "in your network" : "read-only"}</span></div>`;
}

// A local id of a remote node's own host as this map has it: a project's
// group or the repo it's a worktree of.
export const sameHost = (n, id) => (isRemote(n) && id && !id.startsWith("@") ? `@${n.host}:${id}` : id);

// The inspector's details for a host node, as rows made by `row`.
export function hostRows(n, { row, mono }, now = Date.now()) {
  // A machine in your network, drawn from its presence (network-map.js).
  if (n.network === true)
    return (
      row("state", `<span class="host-state ${h(n.state)}"><i class="host-dot ${h(n.state)}"></i>${h(standing(n, now))}</span>`) +
      row("sessions", `${Number(n.sessionCount) || 0}${n.activeCount ? ` <span class="activity active">${Number(n.activeCount)} live</span>` : ""}`) +
      row("last activity", n.lastActivity ? h(when(n.lastActivity, now)) : null) +
      row("platform", mono(n.platform)) +
      row("window", n.window ? "you use a XenoNet window on it" : "no window: a machine you reach in a terminal") +
      row("shows", `<span class="dim">what its presence says: its sessions, their states and what waits on you. Message any of them as you.</span>`, "wrap")
    );
  const offset = Number.isFinite(n.offsetMs) && Math.abs(n.offsetMs) >= 1000 ? `${n.offsetMs > 0 ? "+" : "−"}${Math.abs(n.offsetMs) >= 60e3 ? `${Math.round(Math.abs(n.offsetMs) / 60e3)} min` : `${Math.round(Math.abs(n.offsetMs) / 1000)} s`} against this clock` : null;
  return (
    row("state", `<span class="host-state ${h(n.state)}"><i class="host-dot ${h(n.state)}"></i>${h(standing(n, now))}</span>${n.cached ? ` <span class="tag">from its last visit</span>` : ""}${n.demo ? ` <span class="tag">made-up data</span>` : ""}`) +
    row("since", n.since ? h(when(n.since, now)) : null) +
    row("sessions", `${Number(n.sessionCount) || 0}${n.activeCount ? ` <span class="activity active">${Number(n.activeCount)} live</span>` : ""}`) +
    row("last activity", n.lastActivity ? h(when(n.lastActivity, now)) : null) +
    row("calls itself", n.calls ? h(n.calls) : null) +
    row("xenonet", mono(n.version)) +
    row("platform", mono(n.platform)) +
    row("clock", offset ? h(offset) : null) +
    row("problem", n.error ? `<span class="pressure">${h(n.error)}</span>` : null, "wrap") +
    row("left out", n.truncated ? "its oldest sessions: a host shows at most 5,000 nodes" : null, "wrap") +
    row(
      "access",
      n.network
        ? `<span class="dim">in your network: its map, as it sends it; transcripts and stats open in this window, unless it shares titles only. Message any of its sessions as you.</span>`
        : `<span class="dim">read-only: titles, states and counts; transcripts and stats when you open one</span>`,
      "wrap",
    )
  );
}

// What the tooltip adds for a node from another machine.
export function tipText(n, index, now = Date.now()) {
  if (n.kind === "host") return standing(n, now);
  return isRemote(n) ? `on ${n.host}` : "";
}

// The feed's and the active sessions' mark for a remote session, escaped.
export const hostTag = (n) => (isRemote(n) ? `<span class="host-tag">${h(n.host)}</span>` : "");

// ---------- the alert strip ----------

// What an alert carries about the machine its session is on.
export function alertFrom(s) {
  return isRemote(s) ? { host: s.host, stale: !!s.stale, asOf: s.needsYou?.asOf ?? null } : {};
}

// The alert's mark for a remote session: where it is, and for a host that
// isn't live, as of when.
export function alertHostHtml(it, now = Date.now()) {
  if (!it.host) return "";
  if (it.stale) return `<span class="al-host stale">as of ${h(when(it.asOf, now))}, ${h(it.host)} not live</span>`;
  return `<span class="al-host">on ${h(it.host)}</span>`;
}

// A collision on another machine: which machine, from its sessions' ids.
export const collisionFrom = (c) => (hostOfId(c.a) ? { host: hostOfId(c.a), stale: false } : {});

// A remote question's text stays on its machine unless the host opted in
// to more; the alert says where to read it.
export const alertText = (it) => (it.host && !it.text ? `The question is on ${it.host}: preview the transcript to read it.` : it.text);

// ---------- the frame budget: what's drawn of hosts you're not looking at ----------

// Each node a host puts on the map costs about 0.05 ms a frame, mostly in
// drawing it, so a few large hosts would take the map past the hosts'
// frame budget (docs/design/hosts.md; the benchmark's hosts part measures
// it). So what's drawn of the hosts you're not looking at is capped: every
// host node, and beyond those at most HOST_CAP of their nodes, shared
// between the hosts in turn. A host is looked at while the selection is
// one of its nodes or while its chip is picked, and then all of it is
// drawn. The rest stay in the layout, hidden as the filters hide nodes,
// so looking at a host draws them where they were, with nothing moving.
export const HOST_CAP = 24;

const PLACES = new Set(["project", "native", "zone"]);
// What's drawn first of a host you're not looking at: its sessions that
// wait on you or are stuck, then its places (where something ran lately
// first), then its sessions (active first), then the rest (its wiki and
// pages); the most recent first within each.
const rankOf = (n, attention) => (attention.has(n.id) ? 0 : PLACES.has(n.kind) ? (n.active ? 1 : 2) : n.kind === "session" ? (n.activity === "active" ? 3 : 4) : 5);
const timeOf = (n) => (PLACES.has(n.kind) ? n.lastActivity : n.updatedAt) ?? 0;

// Which of `nodes` (remote nodes of hosts you're not looking at) to leave
// undrawn: { hidden, hosts }, where hosts says per host how many of its
// nodes are drawn, of how many. A node is drawn with what it hangs from
// (parentOf), so none is cut off from its constellation; that, and what
// waits on you, which is always drawn, can take the count a little past
// `cap`. Of nodes alike, those in `keep` (drawn last time) stay, so a
// constellation doesn't flicker as its sessions take turns being the most
// recent.
export function capHosts(nodes, { parentOf = new Map(), attention = new Set(), cap = HOST_CAP, keep = new Set() } = {}) {
  const ids = new Set();
  const byHost = new Map();
  for (const n of nodes) {
    if (!isRemote(n) || n.kind === "host") continue;
    ids.add(n.id);
    (byHost.get(n.host) ?? byHost.set(n.host, []).get(n.host)).push(n);
  }
  const shown = new Set();
  const take = (n) => {
    let added = 0;
    for (let id = n.id; ids.has(id) && !shown.has(id); id = parentOf.get(id)) shown.add(id), added++;
    return added;
  };
  const order = (a, b) => rankOf(a, attention) - rankOf(b, attention) || keep.has(b.id) - keep.has(a.id) || timeOf(b) - timeOf(a) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const queues = [...byHost.keys()].sort().map((host) => ({ host, list: byHost.get(host).sort(order), at: 0 }));
  let left = cap;
  for (const q of queues) for (const n of q.list) if (attention.has(n.id)) left -= take(n);
  // Then each host in turn takes its next node, while there's room.
  for (let more = true; more && left > 0; ) {
    more = false;
    for (const q of queues) {
      while (q.at < q.list.length && shown.has(q.list[q.at].id)) q.at++;
      if (q.at >= q.list.length) continue;
      more = true;
      if (left <= 0) break;
      left -= take(q.list[q.at++]);
    }
  }
  const hidden = new Set();
  for (const id of ids) if (!shown.has(id)) hidden.add(id);
  return { hidden, hosts: new Map(queues.map((q) => [q.host, { drawn: q.list.filter((n) => shown.has(n.id)).length, of: q.list.length }])) };
}

// ---------- the chips under Filters: one per machine ----------

// The map's machines (this one and every host), each with the sessions it
// has on the map, and a filter that shows only some of them; and the cap
// on what's drawn of the hosts you're not looking at (capHosts). main.js
// tells it what you're looking at as it selects (look) and what waits on
// you as the live state comes (setLive), which is first as the page loads,
// before anything is selected. Until it tells it either, nothing is
// capped, since what's beyond what you're looking at depends on that.
export function createHostFilters({ sfx = () => {}, onFilter = () => {}, mount = null, cap = HOST_CAP } = {}) {
  let el = null;
  let hosts = [];
  let self = null;
  let byId = new Map();
  let parentOf = new Map();
  let counts = new Map();
  const selected = new Set();
  // Whether main.js has told it anything yet, the host of the selection,
  // the remote sessions that wait on you, how much of each host was drawn
  // last, and which of their nodes.
  let told = false;
  let lookHost = null;
  let attention = new Set();
  let drawn = new Map();
  let kept = new Set();
  const drawnKey = () => [...drawn].map(([host, d]) => `${host}:${d.drawn}/${d.of}`).join(" ");

  function element() {
    if (el || typeof document === "undefined") return el;
    el = document.getElementById("host-filters");
    if (!el) {
      el = document.createElement("div");
      el.id = "host-filters";
      el.className = "host-filters";
      el.hidden = true;
      const after = mount?.() ?? document.getElementById("id-filters");
      if (!after?.parentNode) return (el = null);
      after.parentNode.insertBefore(el, after.nextSibling);
      el.addEventListener("click", (e) => {
        const chip = e.target.closest("[data-host-key]");
        if (chip) return toggle(chip.dataset.hostKey);
        if (e.target.closest("[data-host-clear]")) setSelected([]);
      });
    }
    return el;
  }

  function setData(graph) {
    hosts = Array.isArray(graph?.hosts) ? graph.hosts : [];
    self = typeof graph?.self === "string" ? graph.self : null;
    byId = new Map((graph?.nodes ?? []).map((n) => [n.id, n]));
    parentOf = new Map();
    for (const l of graph?.links ?? []) if (l.kind !== "host" && l.kind !== "lineage-cross") parentOf.set(idOf(l.target), idOf(l.source));
    counts = new Map();
    for (const n of graph?.nodes ?? []) if (n.kind === "session") counts.set(keyOf(n), (counts.get(keyOf(n)) ?? 0) + 1);
    for (const k of [...selected]) if (k !== HERE && !hosts.some((x) => x.id === k)) selected.delete(k);
    render();
  }

  // How each host stands now (the hosts event), between maps. The event
  // gives the hub's view of each, whose error is the link's { code,
  // message, fix }; the map's summaries say it in words, and so does this.
  function setHosts(list) {
    if (!Array.isArray(list)) return;
    const now = new Map(list.map((x) => [x.id, x]));
    // A machine in a network is the network's (network-map.js), not the hub's.
    hosts = hosts.filter((x) => x.network || now.has(x.id)).map((x) => {
      if (!now.has(x.id)) return x;
      const y = now.get(x.id);
      return { ...x, ...y, error: typeof y.error === "string" ? y.error : (y.error?.message ?? null) };
    });
    render();
  }

  // This machine, by HERE or the name it gave itself; a host, by its id.
  const resolve = (k) => (hosts.some((x) => x.id === k) ? k : k === HERE || k === "here" || (self && k === self) || k === "" || k == null ? HERE : null);

  function setSelected(keys) {
    selected.clear();
    for (const k of keys ?? []) {
      const key = resolve(k);
      if (key) selected.add(key);
    }
    sfx("toggle");
    render();
    onFilter();
  }
  function toggle(key) {
    const next = new Set(selected);
    next.has(key) ? next.delete(key) : next.add(key);
    setSelected([...next]);
  }

  // What you're looking at, from the selection: while it's a node of a
  // host's, all of that host is drawn. main.js says before it selects, so
  // a remote project's sessions are drawn by the time they're gathered.
  function look(node) {
    const host = isRemote(node) ? node.host : null;
    if (told && host === lookHost) return;
    told = true;
    lookHost = host;
    if (hosts.length) onFilter();
  }

  // The remote sessions that wait on you or are stuck, from the live
  // state: drawn whatever the cap, as the alert strip lists them.
  function setLive(snap) {
    const next = new Set();
    for (const s of snap?.sessions ?? []) if (isRemote(s) && (s.needsYou || s.stuck)) next.add(s.id);
    const same = told && next.size === attention.size && [...next].every((id) => attention.has(id));
    told = true;
    attention = next;
    if (!same && hosts.length) onFilter();
  }

  // What's drawn, given what the other filters let through: host nodes are
  // part of the map's structure, and with machines picked only theirs
  // shows (this machine's core always does). Of the hosts you're not
  // looking at, only as much as the cap allows.
  function restrict(visible) {
    if (!hosts.length || !visible) return visible;
    const out = new Set(visible);
    for (const n of byId.values()) if (n.kind === "host") out.add(n.id);
    if (selected.size)
      for (const id of [...out]) {
        const n = byId.get(id);
        if (n && n.kind !== "core" && !selected.has(keyOf(n))) out.delete(id);
      }
    const before = drawnKey();
    drawn = new Map();
    if (told) {
      const beyond = [];
      for (const id of out) {
        const n = byId.get(id);
        if (isRemote(n) && n.kind !== "host" && n.host !== lookHost && !selected.has(n.host)) beyond.push(n);
      }
      const capped = capHosts(beyond, { parentOf, attention, cap, keep: kept });
      for (const id of capped.hidden) out.delete(id);
      drawn = capped.hosts;
      kept = new Set(beyond.filter((n) => !capped.hidden.has(n.id)).map((n) => n.id));
    }
    if (drawnKey() !== before) render();
    return out;
  }

  // A host's chip says how much of it is drawn while it's capped.
  const cappedOf = (id) => {
    const d = drawn.get(id);
    return d && d.drawn < d.of ? { capped: { drawn: d.drawn, of: d.of } } : {};
  };

  function chips() {
    if (!hosts.length) return [];
    return [
      { key: HERE, label: self ?? "this machine", state: "live", sessions: counts.get(HERE) ?? 0, on: selected.has(HERE) },
      ...hosts.map((x) => ({ key: x.id, label: x.id, state: x.state, sessions: counts.get(x.id) ?? 0, on: selected.has(x.id), ...cappedOf(x.id) })),
    ];
  }

  function render() {
    const box = hosts.length ? element() : el;
    if (!box) return;
    box.hidden = !hosts.length;
    if (!hosts.length) return;
    const now = Date.now();
    const chip = (c) => {
      const host = hosts.find((x) => x.id === c.key);
      const part = c.capped ? `, ${c.capped.drawn} of its ${c.capped.of} nodes drawn until you look at it: select one, or click to show only its work.` : ". Click to show only its work.";
      const title = c.key === HERE ? `This machine: ${c.sessions} sessions on the map. Click to show only its work.` : `${c.label} · ${standing({ ...host, name: c.label }, now)}${host?.error ? ` · ${host.error}` : ""}\n${c.sessions} sessions on the map${part}`;
      return `<button class="host-chip ${c.on ? "on" : ""}" data-host-key="${h(c.key)}" title="${h(title)}"><i class="host-dot ${h(c.state)}"></i><span class="host-label">${h(c.label)}</span><span class="count">${c.sessions}</span></button>`;
    };
    box.innerHTML = `
      <div class="host-head"><span class="sub-k">Machines</span>${selected.size ? `<button class="mini" data-host-clear title="Show every machine's work">show all</button>` : ""}</div>
      <div class="host-chips ${selected.size ? "filtering" : ""}">${chips().map(chip).join("")}</div>`;
  }

  return {
    setData,
    setHosts,
    setSelected,
    toggle,
    look,
    setLive,
    restrict,
    chips,
    get selected() {
      return [...selected];
    },
    get hosts() {
      return hosts;
    },
    get self() {
      return self;
    },
  };
}

// This machine's core says which machine it is once hosts are on the map
// ("studio · ~/Projects"), when this XenoNet named itself.
export function decorate(graph) {
  const core = graph?.nodes?.find((n) => n.id === "core");
  if (core) core.selfName = graph.hosts?.length && typeof graph.self === "string" ? graph.self : null;
}
