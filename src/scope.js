// How much history the map draws. Thousands of old or automatic sessions,
// and dozens of projects nobody has touched in months, turn it into a
// hairball, so by default it draws the sessions and projects worked on in
// the last 7 days and leaves out background agents (Codex's guardian checks,
// reviews and compaction threads, and `codex exec` runs). Sessions that are
// working, waiting on you, stuck, pinned or opened from search are always
// drawn, with the sessions they branched from and their project; so are
// pinned projects and ones opened from search. A group (a folder of
// projects) is drawn while something in it is. The Filters panel widens it
// to everything.
//
// Sessions outside the scope leave the layout entirely, rather than being
// hidden, so they don't push the drawn ones apart.
export const WINDOWS = [
  { id: "24h", label: "24h", ms: 24 * 3600e3 },
  { id: "7d", label: "7d", ms: 7 * 86400e3 },
  { id: "30d", label: "30d", ms: 30 * 86400e3 },
  { id: "all", label: "All", ms: Infinity },
];
export const DEFAULT_SCOPE = { window: "7d", background: false };

const KEY = "xenonet.scope";

export function loadScope(storage = globalThis.localStorage) {
  try {
    const saved = JSON.parse(storage?.getItem(KEY) ?? "null");
    if (saved && WINDOWS.some((w) => w.id === saved.window)) return { ...DEFAULT_SCOPE, ...saved, background: !!saved.background };
  } catch {}
  return { ...DEFAULT_SCOPE };
}

export function saveScope(scope, storage = globalThis.localStorage) {
  try {
    storage?.setItem(KEY, JSON.stringify(scope));
  } catch {}
}

const idOf = (end) => (typeof end === "object" ? end.id : end);

// Does a session or project fall inside the scope on its own? A project's
// last activity is its folder changing or its newest session.
export function inScope(n, { window, background }, now = Date.now()) {
  if (n.background && !background) return false;
  const ms = WINDOWS.find((w) => w.id === window)?.ms ?? Infinity;
  return ms === Infinity || now - ((n.kind === "project" ? n.lastActivity : n.updatedAt) ?? 0) <= ms;
}

// The part of the graph to draw. `keep` holds ids drawn whatever the scope.
export function scopeGraph(graph, scope, { keep = new Set(), now = Date.now() } = {}) {
  const parent = new Map();
  for (const l of graph.links) if (l.kind !== "lineage-cross") parent.set(idOf(l.target), idOf(l.source));
  const shown = new Set();
  const show = (id) => {
    for (let cur = id; cur && !shown.has(cur); cur = parent.get(cur)) shown.add(cur);
  };
  const total = { sessions: 0, projects: 0 };
  for (const n of graph.nodes) {
    if (n.kind === "session" || (n.kind === "project" && !n.group)) {
      total[`${n.kind}s`]++;
      if (n.pinned || keep.has(n.id) || inScope(n, scope, now)) show(n.id);
    } else if (n.group && (n.pinned || keep.has(n.id))) show(n.id);
  }
  // The rest of the structure stays; Codex projects and folders outside
  // ~/Projects only while something in them is drawn.
  for (const n of graph.nodes) if (!["session", "project", "native", "zone"].includes(n.kind)) show(n.id);
  const nodes = graph.nodes.filter((n) => shown.has(n.id));
  const count = (kind) => nodes.filter((n) => n.kind === kind && !n.group).length;
  return {
    ...graph,
    nodes,
    links: graph.links.filter((l) => shown.has(idOf(l.source)) && shown.has(idOf(l.target))),
    relations: (graph.relations ?? []).filter((r) => shown.has(r.a) && shown.has(r.b)),
    scope: { sessions: { shown: count("session"), total: total.sessions }, projects: { shown: count("project"), total: total.projects } },
  };
}

// Sessions and projects each scope would draw on their own, for the
// buttons' tooltips.
export function scopeCounts(graph, scope, now = Date.now()) {
  const of = (kind) => graph.nodes.filter((n) => n.kind === kind && !n.group);
  const [sessions, projects] = [of("session"), of("project")];
  return Object.fromEntries(
    WINDOWS.map((w) => {
      const within = (n) => inScope(n, { ...scope, window: w.id }, now);
      return [w.id, { sessions: sessions.filter(within).length, projects: projects.filter(within).length }];
    }),
  );
}
