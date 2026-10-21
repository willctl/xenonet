// Other machines in a network on the map, from presence (docs/design/
// network.md, section 7). Until the map's own frames cross the network,
// each machine is drawn from what its presence says, as a constellation the
// way the SSH hosts prototype draws one (hosts.js, hosts-draw.js): its host
// node, a project for each repo (or each folder that isn't one), named by
// the repo's last part, and its sessions under them with their app, title,
// state and what waits on you. A machine that's away dims, as a host that
// isn't live does.
//
// Every id is qualified @<host>:… as the hosts merge qualifies a host's,
// and every node is made here field by field from the network panel's
// agents (network-model.js's remoteAgents): nothing another machine sent
// is put on the map as it came. Its text is shown escaped where the map
// shows text, and never reaches an agent. A machine that's on the map over
// SSH already (XENONET_HOSTS) keeps that view, which has more in it. No DOM
// here; main.js merges the part into the map before it's drawn.
import { HOST_ID } from "./network-model.js";
import { hostNodeId } from "./hosts.js";

// As the map counts a session active or recent (core/graph.js's windows).
const ACTIVE_MS = 10 * 60e3;
const RECENT_MS = 24 * 3600e3;
// A project lit as a place where something ran in the last hour.
const PROJECT_ACTIVE_MS = 3600e3;

const list = (v) => (Array.isArray(v) ? v : []);
const placed = { fx: null, fy: null, fz: null, pinned: false, actions: [] };

// The last part of a repo's key or a folder: what the project is called.
function lastPart(path) {
  const parts = String(path ?? "")
    .split(/[\\/]+/)
    .filter(Boolean);
  return parts.length ? parts[parts.length - 1] : null;
}

function activityOf(at, waiting, now) {
  if (waiting) return "active";
  if (!Number.isFinite(at)) return "idle";
  return now - at <= ACTIVE_MS ? "active" : now - at <= RECENT_MS ? "recent" : "idle";
}

// The part of the map the network's other machines make: { hosts, nodes,
// links, stats }, from the network's state and its agents. Empty out of a
// network.
export function networkPart(state, agents, now = Date.now()) {
  const part = { hosts: [], nodes: [], links: [], stats: { projects: 0, sessions: 0, claude: 0, codex: 0, active: 0 }, self: state?.self?.id ?? null };
  if (!state?.network) return part;
  const self = state.self?.id ?? null;
  for (const h of list(state.hosts)) {
    if (!HOST_ID.test(h?.id ?? "") || h.id === self) continue;
    const host = h.id;
    const online = h.online !== false;
    const stale = !online;
    const hostId = hostNodeId(host);
    const theirs = list(agents).filter((a) => a.host === host && typeof a.id === "string" && a.id.startsWith(`@${host}:`) && !a.id.slice(host.length + 2).startsWith("@"));
    const projects = new Map();
    const sessions = [];
    for (const a of theirs) {
      // A project per repo, else per folder; a session with neither hangs
      // from the host node.
      const where = a.repoKey ? `repo:${a.repoKey}` : a.cwd ? `folder:${a.cwd}` : null;
      const name = lastPart(a.repoKey) ?? lastPart(a.cwd) ?? a.project;
      let parent = hostId;
      if (where && name) {
        const id = `@${host}:project:${where}`;
        let p = projects.get(id);
        if (!p) {
          p = { id, kind: "project", host, name, path: a.cwd ?? null, repoKey: a.repoKey ?? undefined, group: false, satelliteOf: null, repo: null, stack: [], description: null, sessionCount: 0, activeCount: 0, lastActivity: null, lastSessionAt: null, active: false, stale, network: true, ...placed };
          projects.set(id, p);
        }
        parent = id;
      }
      const at = a.lastActive ?? null;
      const working = a.status === "working" || a.waits || !!a.stuck;
      const activity = online ? activityOf(at, working, now) : "idle";
      const local = a.id.slice(host.length + 2);
      sessions.push({
        id: a.id,
        kind: "session",
        host,
        app: a.app ?? local.split(":")[0],
        title: a.title,
        rawId: local,
        cwd: a.cwd,
        branch: a.branch,
        createdAt: null,
        updatedAt: at,
        archived: false,
        activity,
        stale,
        network: true,
        // What its presence says, which the inspector and the map's glow
        // show: its state, what it's doing, what waits on you (with the
        // question's text, from a machine that shares more than titles),
        // what it said last and why it seems stuck.
        presence: { state: a.status ?? null, focus: a.focus ?? null, waits: !!a.waits, kind: a.waits ? (a.kind ?? "question") : null, question: a.text ?? null, since: a.since ?? null, lastMessage: a.lastMessage ?? null, stuck: a.stuck ?? null },
        ...placed,
      });
      part.links.push({ source: parent, target: a.id, kind: "session" });
      if (parent !== hostId) {
        const p = projects.get(parent);
        p.sessionCount++;
        if (activity === "active") p.activeCount++;
        if (Number.isFinite(at) && (p.lastSessionAt ?? -Infinity) < at) p.lastSessionAt = p.lastActivity = at;
      }
    }
    for (const p of projects.values()) {
      p.active = online && Number.isFinite(p.lastSessionAt) && now - p.lastSessionAt <= PROJECT_ACTIVE_MS;
      part.links.push({ source: hostId, target: p.id, kind: "contains" });
    }
    const active = sessions.filter((s) => s.activity === "active").length;
    const last = Math.max(-Infinity, ...sessions.map((s) => s.updatedAt).filter(Number.isFinite));
    part.nodes.push({
      id: hostId,
      kind: "host",
      host,
      name: host,
      path: null,
      calls: null,
      state: online ? "live" : "offline",
      since: null,
      lastHeard: Number.isFinite(h.seen) ? h.seen : null,
      offsetMs: null,
      version: null,
      platform: typeof h.platform === "string" ? h.platform : null,
      demo: null,
      error: null,
      cached: false,
      truncated: false,
      stale,
      network: true,
      window: !!h.window,
      sessionCount: sessions.length,
      activeCount: active,
      lastActivity: Number.isFinite(last) ? last : null,
      lastSessionAt: Number.isFinite(last) ? last : null,
      ...placed,
    });
    part.nodes.push(...projects.values(), ...sessions);
    part.links.unshift({ source: "core", target: hostId, kind: "host" });
    part.hosts.push({ id: host, state: online ? "live" : "offline", since: null, lastHeard: Number.isFinite(h.seen) ? h.seen : null, offsetMs: null, version: null, platform: typeof h.platform === "string" ? h.platform : null, demo: null, error: null, cached: false, truncated: false, nodes: projects.size + sessions.length, sessions: sessions.length, network: true });
    part.stats.projects += projects.size;
    part.stats.sessions += sessions.length;
    part.stats.active += active;
    for (const s of sessions) if (s.app === "claude" || s.app === "codex") part.stats[s.app]++;
  }
  return part;
}

// What changes the drawn part: the map is merged again only when this
// does, so the network panel's reads every few seconds redraw nothing.
export function partKey(part) {
  return JSON.stringify([part.self, part.hosts.map((x) => [x.id, x.state, x.state === "live" ? null : x.lastHeard]), part.nodes.map((n) => [n.id, n.state ?? n.activity, n.title ?? n.name, n.presence ?? null, n.updatedAt ?? null])]);
}

// The map with the network's other machines in it: their nodes after this
// machine's, their host links, their machines among the map's hosts, and
// this machine named. A machine the map has already (over SSH) is left as
// it is. The map given isn't changed.
export function withNetwork(graph, part) {
  if (!graph || !part?.hosts.length) return graph;
  const have = new Set(list(graph.nodes).map((n) => n.id));
  const known = new Set(list(graph.hosts).map((x) => x.id));
  const fresh = new Set(part.hosts.filter((x) => !known.has(x.id) && !have.has(hostNodeId(x.id))).map((x) => x.id));
  if (!fresh.size) return graph;
  const nodes = part.nodes.filter((n) => fresh.has(n.host) && !have.has(n.id));
  const ids = new Set(nodes.map((n) => n.id));
  const links = part.links.filter((l) => ids.has(l.target) && (l.source === "core" || ids.has(l.source)));
  const stats = { ...(graph.stats ?? {}) };
  for (const n of nodes) {
    if (n.kind === "project") stats.projects = (stats.projects ?? 0) + 1;
    if (n.kind !== "session") continue;
    stats.sessions = (stats.sessions ?? 0) + 1;
    if (n.app === "claude" || n.app === "codex") stats[n.app] = (stats[n.app] ?? 0) + 1;
    if (n.activity === "active") stats.active = (stats.active ?? 0) + 1;
  }
  return {
    ...graph,
    nodes: [...list(graph.nodes), ...nodes],
    links: [...list(graph.links), ...links],
    hosts: [...list(graph.hosts), ...part.hosts.filter((x) => fresh.has(x.id))],
    self: typeof graph.self === "string" ? graph.self : part.self,
    stats,
  };
}
