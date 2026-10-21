// graph/v1: the map, as GET /api/graph and the `graph` event send it:
// buildGraph's nodes, links and relations (core/graph.js), with what
// state.js adds to them (actions, identities, the sources and how each
// went). With hosts on (XENONET_HOSTS=1, docs/design/hosts.md), other
// machines' nodes join it under @<host>: ids, each host with a node of its
// own, and checkouts of one repo on different machines are related.
import { define } from "./registry.js";
import { z, obj, ms, count, nstr } from "./common.js";
import { Session } from "./session.js";
import { Project } from "./project.js";
import { Identity, RanAs, ProjectIdentity } from "./identity.js";

const Action = obj({
  id: z.enum(["open-app", "preview", "stats", "handoff", "open-folder", "open-file", "open-vscode", "new-claude"]),
  label: z.string(),
  client: z.literal(true).optional().describe("The page handles it itself, rather than asking the server to open something"),
});

// Every node: where you pinned it, if you did, and what it offers.
const placed = {
  fx: z.number().nullable(),
  fy: z.number().nullable(),
  fz: z.number().nullable(),
  pinned: z.boolean(),
  actions: z.array(Action),
};

// Every node but a session: the sessions under it, rolled up.
const rolled = {
  sessionCount: count,
  activeCount: count,
  lastActivity: ms.nullable(),
  lastSessionAt: ms.nullable(),
};

// Places where a session ran lately are lit, and the rest dim.
const lit = { active: z.boolean() };

// A node from another machine says which, and whether that host is live.
const remote = {
  host: z.string().optional().describe("The host it's from, for a node from another machine; its id is then @<host>:…"),
  stale: z.boolean().optional().describe("Its host isn't live, so it shows what was true when the host last was"),
};
// An id, here or from another host.
const HOSTED = "(?:@[a-z][a-z0-9-]{1,31}:)?";
const hosted = (rest) => new RegExp(`^${HOSTED}${rest}`);

export const RepoStatus = obj({
  branch: nstr,
  ahead: count,
  behind: count,
  staged: count,
  modified: count,
  untracked: count,
  dirty: count.describe("Lines git status reports: files changed, staged or new"),
  checkedAt: ms,
});

// Wiki pages take their fields from front matter as it's written: a
// value is text, or JSON when it starts with [ (so a list, or anything).
const front = z.union([z.string(), z.array(z.unknown())]).describe("A front matter value: text, or a JSON list when it starts with [");

const SessionNode = z
  .intersection(
    Session,
    obj({
      kind: z.literal("session"),
      activity: z.enum(["active", "recent", "idle"]).describe("By its last write; an archived session is at most recent"),
      ranAs: RanAs.nullable().optional().describe("Who it ran as; left out when identities are off"),
      ...placed,
      ...remote,
    }),
  )
  .describe("A session (session/v1) where it lives on the map");

const ProjectNode = z
  .intersection(
    Project,
    obj({
      id: z.string().regex(hosted("project:")),
      kind: z.literal("project"),
      group: z.boolean(),
      satelliteOf: nstr.describe("The key of the project it orbits: its repo for a worktree, or the project it's named after"),
      repo: RepoStatus.nullable(),
      identities: z.array(ProjectIdentity).optional().describe("Left out when identities are off"),
      repoKey: z.string().optional().describe("Its remote as its own host resolves it (repoKey in core/schema/ids.js): checkouts of one repo on different machines share it. With hosts on"),
      ...placed,
      ...rolled,
      ...lit,
      ...remote,
    }),
  )
  .describe("A project or group (project/v1) on the map");

const CoreNode = obj({
  id: z.literal("core"),
  kind: z.literal("core"),
  name: z.string().describe("~/ and the projects folder's name"),
  path: z.string(),
  home: nstr,
  ...placed,
  ...rolled,
}).describe("The projects folder, at the center");

const NativeNode = obj({
  id: z.string().regex(hosted("native:codex:")),
  kind: z.literal("native"),
  app: z.literal("codex"),
  name: z.string(),
  roots: z.array(z.string()),
  ...placed,
  ...rolled,
  ...lit,
  ...remote,
}).describe("A Codex desktop project");

const ZoneNode = obj({
  id: z.string().regex(hosted("zone:")),
  kind: z.literal("zone"),
  name: z.string(),
  path: nstr,
  app: z.enum(["claude", "codex"]).nullable(),
  external: z.literal(true),
  ...placed,
  ...rolled,
  ...lit,
  ...remote,
}).describe("A folder outside the projects folder where sessions ran: the home folder, scratch folders, temp, or one folder");

const WikiNode = obj({
  id: z.string().regex(hosted("resource:(xenolith|agent-wiki)$")),
  kind: z.literal("wiki"),
  name: z.string(),
  server: z.enum(["xenolith", "agent-wiki"]).optional().describe("The MCP server that writes to it"),
  path: z.string(),
  pageCount: count,
  inbox: z.array(obj({ id: z.string(), title: z.string(), app: nstr })),
  ...placed,
  ...rolled,
  ...remote,
}).describe("The shared context: Xenolith, or Agent Wiki before it");

const PageNode = obj({
  id: z.string().regex(hosted("wiki:")),
  kind: z.literal("page"),
  name: front,
  slug: z.string(),
  type: front,
  summary: front,
  tags: z.array(z.unknown()),
  updated: ms.nullable(),
  updatedBy: front.nullable(),
  path: z.string(),
  writes: z.array(obj({ ts: ms, app: z.string(), title: z.string(), pages: z.array(z.string()) })).max(6),
  aliases: z.array(z.string()).optional().describe("Other names it goes by, from its aliases, in lowercase; left out when it has none"),
  space: z.string().optional().describe("The Xenolith space it's in, such as project:atlas, when that isn't this machine's own; its id is then wiki:<space>/<slug>"),
  ...placed,
  ...rolled,
  ...remote,
}).describe("A page of the shared context's wiki");

const ResourceNode = obj({
  id: z.string().regex(hosted("resource:")),
  kind: z.literal("resource"),
  name: z.string().describe("The MCP server"),
  ...placed,
  ...rolled,
  ...remote,
}).describe("A shared system sessions reach through MCP");

const HOST_STATES = ["connecting", "live", "stale", "offline", "error"];

const HostNode = obj({
  id: z.string().regex(/^@[a-z][a-z0-9-]{1,31}:core$/),
  kind: z.literal("host"),
  host: z.string(),
  name: z.string().describe("Its id here"),
  path: nstr.describe("Its projects folder, as it names it (~/Projects)"),
  calls: nstr.describe("What it calls itself, when that isn't its id here"),
  state: z.enum(HOST_STATES).describe("connecting; live; stale (no word for 45 seconds); offline (the connection ended, and XenoNet tries again); error (it needs you)"),
  since: ms,
  lastHeard: ms.nullable(),
  offsetMs: z.number().nullable().describe("Its clock minus this one's, once known"),
  version: nstr.describe("Its XenoNet"),
  platform: nstr,
  demo: z.boolean().nullable().describe("It shows made-up data"),
  error: nstr.describe("Why it isn't live, in words"),
  cached: z.boolean().describe("What's shown is its last snapshot, kept from before this XenoNet started"),
  truncated: z.boolean().describe("It sent more than the 5,000 nodes a host may have on the map, so its oldest sessions are left out"),
  stale: z.boolean(),
  ...placed,
  ...rolled,
}).describe("Another machine's XenoNet, at the center of its constellation (with hosts on)");

const HostSummary = obj({
  id: z.string(),
  state: z.enum(HOST_STATES),
  since: ms,
  lastHeard: ms.nullable(),
  offsetMs: z.number().nullable(),
  version: nstr,
  platform: nstr,
  demo: z.boolean().nullable(),
  error: nstr,
  cached: z.boolean(),
  truncated: z.boolean(),
  nodes: count,
  sessions: count,
});

const LINK_KINDS = ["contains", "nested", "satellite", "native", "zone", "session", "lineage", "lineage-cross", "wiki", "page", "resource", "host"];

export const Graph = define(
  "graph",
  1,
  obj({
    generatedAt: ms,
    nodes: z.array(z.union([SessionNode, ProjectNode, CoreNode, NativeNode, ZoneNode, WikiNode, PageNode, ResourceNode, HostNode])),
    links: z.array(obj({ source: z.string(), target: z.string(), kind: z.enum(LINK_KINDS) })).describe("The tree, by node ids; lineage-cross ties a session to its parent in another place, and host ties another machine's node to this one's core"),
    relations: z.array(obj({ a: z.string(), b: z.string(), kind: z.enum(["wikilink", "mentions", "same-repo"]) })).describe("Ties outside the tree: wiki pages that link, pages that mention a project, and checkouts of one repo on different machines"),
    stats: obj({
      wikiPages: count,
      dirtyRepos: count,
      projects: count,
      groups: count,
      sessions: count,
      claude: count,
      codex: count,
      active: count,
      archived: count,
    }),
    identities: z.array(Identity),
    hideIdentities: z.boolean().describe("Start with identities masked (XENONET_HIDE_IDENTITIES=1)"),
    demo: z.boolean().optional().describe("The demo's made-up map, whose sessions exist only in XenoNet"),
    hosts: z.array(HostSummary).optional().describe("The other machines on the map and how each is, with hosts on"),
    self: nstr.optional().describe("This XenoNet's own host id, if it named itself, with hosts on"),
    sources: z
      .array(
        obj({
          name: z.string().describe("projects, codex, codex-projects, claude, live, wiki, search, identities"),
          ok: z.boolean(),
          ms: z.number().nullable().optional(),
          count: z.number().nullable().optional(),
          error: z.string().optional(),
          note: z.string().optional(),
        }),
      )
      .describe("Where the map came from, and how each source went this time"),
  }),
  {
    title: "Graph",
    description: "The map: the projects folder, its groups and projects, Codex desktop projects, the folders outside it where sessions ran, the sessions, Xenolith and the MCP servers sessions share, with the identities work ran as.",
  },
);

// The schema for each kind of node, for checking another host's nodes one
// at a time (core/hosts/merge.js).
export const GRAPH_NODES = { session: SessionNode, project: ProjectNode, native: NativeNode, zone: ZoneNode, wiki: WikiNode, page: PageNode, resource: ResourceNode };
