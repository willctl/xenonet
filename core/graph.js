// Builds the hierarchy the UI renders:
//   core (~/Projects) → [groups] → projects → [Codex native project] → sessions → spawned/forked sessions
// plus "zones" for sessions whose folder is outside ~/Projects. A group is a
// folder of projects (see scanProjects); it's a project node with `group`
// set, so sessions run in the folder itself land on it, and it lights, dims
// and gathers like a project.
import { pathKey, isUnder, baseName, scratchZone, tilde } from "./paths.js";
import { projectRelations, normalizeRemote } from "./projects.js";
import { contextName } from "./xenolith.js";

export function activityOf(updatedAt, now, { activeWindowMs, recentWindowMs }) {
  if (!updatedAt) return "idle";
  const age = now - updatedAt;
  if (age <= activeWindowMs) return "active";
  if (age <= recentWindowMs) return "recent";
  return "idle";
}

export function buildGraph({ projectsDir, homeDir = null, projects, sessions, nativeProjects = [], resources = [], wiki = null, repos = new Map(), layout = {}, now = Date.now(), windows }) {
  const nodes = new Map();
  const links = [];
  const parentOfNode = new Map();
  const link = (source, target, kind) => {
    links.push({ source, target, kind });
    parentOfNode.set(target, source);
  };

  const rootKey = pathKey(projectsDir);
  // Named for the projects folder in use: ~/Projects, ~/code/work, or the
  // whole path of one outside the home folder.
  const rootName = homeDir ? tilde(projectsDir, homeDir).replace(/\\/g, "/") : "~/" + baseName(projectsDir);
  nodes.set("core", { id: "core", kind: "core", name: rootName, path: projectsDir, home: homeDir });

  // Projects inside their group (or the project they're nested in), with
  // satellites attached to their parent project.
  const { parentOf } = projectRelations(projects);
  for (const p of projects) {
    nodes.set(`project:${p.key}`, { id: `project:${p.key}`, kind: "project", ...p, group: p.role === "group", satelliteOf: parentOf.get(p.key) ?? null, repo: repos.get(p.key) ?? null });
  }
  for (const p of projects) {
    const satellite = parentOf.get(p.key);
    if (satellite) link(`project:${satellite}`, `project:${p.key}`, "satellite");
    else if (p.parent && nodes.has(`project:${p.parent}`)) link(`project:${p.parent}`, `project:${p.key}`, "nested");
    else link("core", `project:${p.key}`, "contains");
  }
  // The deepest project at or above a folder: the folder itself, then each
  // one up, in a few lookups rather than a pass over every project.
  const projectByKey = new Map(projects.map((p) => [p.key, p]));
  const projectForKey = (key) => {
    for (let k = key; k; k = k.slice(0, Math.max(0, k.lastIndexOf("/")))) if (projectByKey.has(k)) return projectByKey.get(k);
    return null;
  };
  const projectsByRemote = new Map();
  for (const p of projects) {
    const r = normalizeRemote(p.git?.origin);
    if (r && !projectsByRemote.has(r)) projectsByRemote.set(r, p);
  }

  // Codex desktop projects sit inside the dir project that contains their root.
  const nativeIds = new Set();
  for (const np of nativeProjects) {
    const id = `native:codex:${np.id}`;
    const home = np.roots.map((r) => projectForKey(pathKey(r))).find(Boolean);
    nodes.set(id, { id, kind: "native", app: "codex", name: np.name, roots: np.roots });
    link(home ? `project:${home.key}` : "core", id, "native");
    nativeIds.add(np.id);
  }

  const homeKey = homeDir ? pathKey(homeDir) : null;
  const zoneFor = (key, sessionPath) => {
    // A folder in your home folder isn't Temp, even when home itself sits
    // in a temp folder, as the demo's does.
    let scratch = scratchZone(key);
    if (scratch?.id === "zone:temp" && homeKey && isUnder(key, homeKey)) scratch = null;
    let id, name, zonePath, app;
    if (!key) [id, name, zonePath] = ["zone:unknown", "Unknown folder", null];
    else if (scratch) [id, name, zonePath, app] = [scratch.id, scratch.name, null, scratch.app];
    else if (key === homeKey) [id, name, zonePath] = ["zone:home", "Home folder", sessionPath];
    else [id, name, zonePath] = [`zone:${key}`, baseName(key), sessionPath];
    if (!nodes.has(id)) {
      nodes.set(id, { id, kind: "zone", name, path: zonePath, app: app ?? null, external: true });
      link("core", id, "zone");
    }
    return id;
  };

  // Where a session lives, ignoring lineage: native project, dir project, or zone.
  const homeOf = (s) => {
    // A run reported from another machine (a sandbox, through the actor
    // gateway) has no folder here.
    if (s.observed === "gateway") {
      if (!nodes.has("zone:elsewhere")) {
        nodes.set("zone:elsewhere", { id: "zone:elsewhere", kind: "zone", name: "Elsewhere", path: null, app: null, external: true });
        link("core", "zone:elsewhere", "zone");
      }
      return "zone:elsewhere";
    }
    if (s.nativeProjectId && nativeIds.has(s.nativeProjectId)) return `native:codex:${s.nativeProjectId}`;
    const key = pathKey(s.cwd);
    const p = key && isUnder(key, rootKey) ? projectForKey(key) : null;
    const byRemote = !p && s.origin ? projectsByRemote.get(normalizeRemote(s.origin)) : null;
    if (p) return `project:${p.key}`;
    if (byRemote) return `project:${byRemote.key}`;
    return zoneFor(key, s.cwd);
  };

  const byId = new Map(sessions.map((s) => [s.id, s]));
  for (const s of sessions) {
    // An archived session can be touched by the app, but it isn't live work.
    const activity = activityOf(s.updatedAt, now, windows);
    nodes.set(s.id, { ...s, kind: "session", activity: s.archived && activity === "active" ? "recent" : activity });
  }
  const homes = new Map(sessions.map((s) => [s.id, homeOf(s)]));
  for (const s of sessions) {
    const parent = s.parentId && s.parentId !== s.id ? byId.get(s.parentId) : null;
    // Spawned/forked sessions nest under their parent when they share a home;
    // otherwise they stay in their own project with a cross-link to the parent.
    if (parent && homes.get(parent.id) === homes.get(s.id)) {
      link(parent.id, s.id, "lineage");
      continue;
    }
    link(homes.get(s.id), s.id, "session");
    if (parent) links.push({ source: parent.id, target: s.id, kind: "lineage-cross" });
  }

  // The shared context (Xenolith, or Agent Wiki) as its own constellation.
  // Its root takes its MCP server's resource id so beams from sessions'
  // calls to that server land on it.
  const relations = [];
  if (wiki) {
    const server = wiki.server ?? "agent-wiki";
    const root = `resource:${server}`;
    nodes.set(root, { id: root, kind: "wiki", name: contextName(server), server, path: wiki.dir, pageCount: wiki.pages.length, inbox: wiki.inbox, lastActivity: wiki.log[wiki.log.length - 1]?.ts ?? null });
    link("core", root, "wiki");
    // A page of another space than this machine's (Xenolith's spaces) is
    // wiki:<space>/<slug>, and its [[links]] are to pages of its own space.
    const idOf = (space, slug) => `wiki:${space ? `${space}/` : ""}${slug}`;
    const ids = new Set(wiki.pages.map((p) => idOf(p.space, p.slug)));
    const seen = new Set();
    for (const p of wiki.pages) {
      const id = idOf(p.space, p.slug);
      // Its aliases and space only when it has them, so a page without
      // stays as it was.
      nodes.set(id, { id, kind: "page", name: p.title, slug: p.slug, type: p.type, summary: p.summary, tags: p.tags, updated: p.updated, updatedBy: p.updatedBy, path: p.path, writes: p.writes.slice(-6), lastActivity: p.updated, ...(Array.isArray(p.aliases) && p.aliases.length ? { aliases: p.aliases } : {}), ...(p.space ? { space: String(p.space) } : {}) });
      link(root, id, "page");
      for (const l of p.links) {
        const to = idOf(p.space, l);
        const k = [id, to].sort().join("|");
        if (ids.has(to) && !seen.has(k)) {
          seen.add(k);
          relations.push({ a: id, b: to, kind: "wikilink" });
        }
      }
      for (const key of p.projects) relations.push({ a: id, b: `project:${key}`, kind: "mentions" });
    }
  }

  // Shared systems sessions reach through MCP (a browser, ...).
  for (const r of resources) {
    const id = `resource:${r.server}`;
    if (nodes.has(id)) continue;
    nodes.set(id, { id, kind: "resource", name: r.server, lastActivity: r.ts });
    link("core", id, "resource");
  }

  // Positions you dragged into place stay put (null clears a previous pin).
  for (const n of nodes.values()) {
    const p = layout[n.id];
    Object.assign(n, p ? { fx: p.x, fy: p.y, fz: p.z, pinned: true } : { fx: null, fy: null, fz: null, pinned: false });
  }

  // Roll session counts and latest activity up the tree.
  for (const n of nodes.values()) {
    if (n.kind !== "session") Object.assign(n, { sessionCount: 0, activeCount: 0, lastActivity: n.lastActivity ?? n.modifiedAt ?? null, lastSessionAt: null });
  }
  for (const s of sessions) {
    let cur = parentOfNode.get(s.id);
    const seen = new Set();
    while (cur && !seen.has(cur)) {
      seen.add(cur);
      const n = nodes.get(cur);
      if (n && n.kind !== "session") {
        n.sessionCount++;
        if (nodes.get(s.id).activity === "active") n.activeCount++;
        if (s.updatedAt && s.updatedAt > (n.lastActivity ?? 0)) n.lastActivity = s.updatedAt;
        if (s.updatedAt && s.updatedAt > (n.lastSessionAt ?? 0)) n.lastSessionAt = s.updatedAt;
      }
      cur = parentOfNode.get(cur);
    }
  }
  // Places where a session ran lately are active; the map lights them and
  // dims the rest. A folder merely changing doesn't count.
  const activeMs = windows?.projectActiveMs ?? 3600e3;
  for (const n of nodes.values()) if (n.kind === "project" || n.kind === "native" || n.kind === "zone") n.active = n.lastSessionAt != null && now - n.lastSessionAt <= activeMs;

  const all = [...nodes.values()];
  const sessionNodes = all.filter((n) => n.kind === "session");
  return {
    generatedAt: now,
    nodes: all,
    links,
    relations: relations.filter((r) => nodes.has(r.a) && nodes.has(r.b)),
    stats: {
      wikiPages: wiki?.pages.length ?? 0,
      dirtyRepos: [...repos.values()].filter((r) => r.dirty > 0).length,
      projects: projects.filter((p) => p.role !== "group").length,
      groups: projects.filter((p) => p.role === "group").length,
      sessions: sessionNodes.length,
      claude: sessionNodes.filter((n) => n.app === "claude").length,
      codex: sessionNodes.filter((n) => n.app === "codex").length,
      active: sessionNodes.filter((n) => n.activity === "active").length,
      archived: sessionNodes.filter((n) => n.archived).length,
    },
  };
}
