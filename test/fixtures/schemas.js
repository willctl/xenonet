// Made-up data for XenoNet's schemas: the edge cases the demo world doesn't
// reach, and a few that mustn't pass. Everything here is invented; the
// actor and drift examples are the contracts' own (docs/design/contracts.md).
const T = 1791230000000;
const HOME = "/Users/nova";
const P = `${HOME}/Projects`;

const placed = { fx: null, fy: null, fz: null, pinned: false, actions: [] };
const rolled = { sessionCount: 0, activeCount: 0, lastActivity: null, lastSessionAt: null };

const claudeSession = {
  id: "claude:0190d3e0-0000-7000-8000-0000000000c1",
  app: "claude",
  rawId: "0190d3e0-0000-7000-8000-0000000000c1",
  desktopId: null,
  inDesktop: false,
  title: "(untitled session)",
  preview: null,
  cwd: null,
  worktree: null,
  createdAt: null,
  updatedAt: null,
  archived: false,
  starred: false,
  model: null,
  effort: null,
  permissionMode: "acceptEdits",
  branch: null,
  tag: "spike",
  sizeBytes: 48211.5,
  parentId: null,
  parentKind: null,
  claudeAccount: { account: "0190d3e0-0000-7000-a000-000000000009", org: "0190d3e0-0000-7000-b000-000000000009" },
};

const codexSession = {
  id: "codex:0190d3e0-0000-7000-8000-0000000000d1",
  app: "codex",
  rawId: "0190d3e0-0000-7000-8000-0000000000d1",
  title: "Review the rate limiter",
  preview: "Review the rate limiter\nand say what's missing",
  cwd: `${P}/atlas-api`,
  createdAt: T - 3600e3,
  updatedAt: T - 60e3,
  archived: true,
  status: "notLoaded",
  model: "gpt-5-codex",
  provider: "ollama",
  effort: "high",
  branch: "feat/rate-limit",
  origin: "git@github.com:atlas-co/atlas-api.git",
  source: "subAgent",
  originator: "codex_exec",
  nativeProjectId: null,
  parentId: "codex:0190d3e0-0000-7000-8000-0000000000d0",
  parentKind: "subagent",
  agentNickname: "reviewer",
  agentKind: "review",
  background: true,
};

// A project in a group, found because sessions ran in it, under a rule
// Codex picked, with a jj repo on a detached head.
const geofence = {
  name: "geofence",
  path: `${P}/labs/geofence`,
  key: `${P.toLowerCase()}/labs/geofence`,
  rel: "labs/geofence",
  role: "project",
  detected: "sessions",
  marker: ".jj",
  parent: `${P.toLowerCase()}/labs`,
  git: { branch: "4be1c0ffee12", origin: null, worktreeOf: null, submoduleOf: null },
  stack: ["node", "typescript"],
  description: "Delivery zones drawn from postcodes.",
  modifiedAt: T - 5000.25,
  rule: { path: `${P}/labs/geofence`, as: "project", by: "codex", reason: "Its own package.json and code." },
};

// A map with one node of every kind, identities off (no ranAs, no project
// identities), a group, a resource, a page whose front matter title is a
// list, and an empty wiki inbox.
const graph = {
  generatedAt: T,
  nodes: [
    { id: "core", kind: "core", name: "~/Projects", path: P, home: HOME, ...placed, ...rolled, sessionCount: 2 },
    {
      id: `project:${P.toLowerCase()}/labs`,
      kind: "project",
      name: "labs",
      path: `${P}/labs`,
      key: `${P.toLowerCase()}/labs`,
      rel: "labs",
      role: "group",
      detected: "rule",
      marker: null,
      parent: null,
      git: null,
      stack: [],
      description: null,
      modifiedAt: null,
      rule: { path: `${P}/labs`, as: "group", at: T - 86400e3, by: "you" },
      group: true,
      satelliteOf: null,
      repo: null,
      ...placed,
      pinned: true,
      fx: 1.5,
      fy: -2,
      fz: 0,
      ...rolled,
      active: false,
    },
    {
      id: `project:${P.toLowerCase()}/labs/geofence`,
      kind: "project",
      ...geofence,
      group: false,
      satelliteOf: null,
      repo: { branch: null, ahead: 0, behind: 2, staged: 1, modified: 0, untracked: 3, dirty: 4, checkedAt: T },
      ...placed,
      ...rolled,
      active: true,
    },
    { id: "native:codex:np-1", kind: "native", app: "codex", name: "Atlas", roots: [`${P}/atlas-api`], ...placed, ...rolled, active: false },
    { id: "zone:home", kind: "zone", name: "Home folder", path: HOME, app: null, external: true, ...placed, ...rolled, active: false },
    { id: "zone:claude-scratch", kind: "zone", name: "Claude scratch", path: null, app: "claude", external: true, ...placed, ...rolled, active: true },
    { ...claudeSession, kind: "session", activity: "idle", ...placed, actions: [{ id: "preview", label: "Preview transcript", client: true }] },
    { ...codexSession, kind: "session", activity: "recent", ...placed, actions: [{ id: "open-app", label: "Open in ChatGPT · Codex" }] },
    { id: "resource:agent-wiki", kind: "wiki", name: "Agent Wiki", path: `${HOME}/wiki`, pageCount: 1, inbox: [], ...placed, ...rolled },
    {
      id: "wiki:q4-close",
      kind: "page",
      name: ["Q4", "close"],
      slug: "q4-close",
      type: "topic",
      summary: "",
      tags: ["ledger", 2026],
      updated: null,
      updatedBy: null,
      path: `${HOME}/wiki/pages/q4-close.md`,
      writes: [{ ts: T - 7200e3, app: "curator", title: "Page updated: Q4 close", pages: ["q4-close"] }],
      ...placed,
      ...rolled,
    },
    { id: "resource:Claude_Browser", kind: "resource", name: "Claude_Browser", ...placed, ...rolled, lastActivity: T - 9000 },
  ],
  links: [
    { source: "core", target: `project:${P.toLowerCase()}/labs`, kind: "contains" },
    { source: `project:${P.toLowerCase()}/labs`, target: `project:${P.toLowerCase()}/labs/geofence`, kind: "nested" },
    { source: "codex:0190d3e0-0000-7000-8000-0000000000d0", target: codexSession.id, kind: "lineage-cross" },
  ],
  relations: [{ a: "wiki:q4-close", b: `project:${P.toLowerCase()}/labs/geofence`, kind: "mentions" }],
  stats: { wikiPages: 1, dirtyRepos: 1, projects: 1, groups: 1, sessions: 2, claude: 1, codex: 1, active: 0, archived: 1 },
  identities: [],
  hideIdentities: true,
  sources: [
    { name: "projects", ok: true, ms: 12, count: 2, note: "1 projects in 1 group, 14 folders read" },
    { name: "codex", ok: false, ms: 20001, error: "thread/list timed out" },
    { name: "live", ok: false, count: 0 },
  ],
};

const forecast = { pct: 61, perHour: 4.2, eta: null, resetsAt: T + 3600e3, hitsBeforeReset: false };

// A quiet machine: no sessions, Codex's limits with no second window and no
// reset time, no Claude reading for a while, and a Codex forecast with no
// usage.
const quietLive = {
  at: T,
  sessions: [],
  series: { claude: [0, 0, 0], codex: [0, 0, 0] },
  tpm: 0,
  total24h: 0,
  today: 0,
  limits: {
    codex: { primary: { pct: 12, windowMinutes: 300, resetsAt: null }, secondary: null, observedAt: T - 60e3 },
    claude: { fiveHour: null, sevenDay: null, observedAt: null, stale: true, reason: "no Claude Code sign-in found" },
  },
  limitForecast: { codex: { windowMinutes: 300 }, claudeFiveHour: null },
  collab: [],
  collisions: [],
  resources: [],
  status: { watching: false, files: 0, lines: 0, errors: 3 },
};

// A session waiting for permission and stuck, with a subagent, mid-tool.
const busySession = {
  id: "claude:0190d3e0-0000-7000-8000-0000000000c2",
  app: "claude",
  status: "tool",
  tokens: { processed: 1200345, output: 42000 },
  tokensToday: 900000,
  tpm: 31000,
  context: { used: 1040000, window: 1000000 },
  forecast: { pct: 1.04, growthPerTurn: null, turnsLeft: 0 },
  model: "claude-opus-4-1[1m]",
  turnStartedAt: T - 300e3,
  lastTool: { name: "mcp__plugin_x_github__create_pr", category: "mcp", detail: "", ts: T - 5000 },
  toolCounts: { mcp: 3, shell: 12 },
  subagents: [{ id: "a1b2", label: "subagent", startedAt: T - 200e3, lastAt: T - 100e3, tools: 0, done: false, active: false }],
  files: [{ path: `${P}/atlas-api/src/a.ts`, ts: T - 9000 }],
  mcp: [{ server: "github", ts: T - 5000 }],
  needsYou: { kind: "permission", text: "Claude needs your permission to use Bash", since: T - 4000 },
  stuck: { reasons: ["ran npm run db:push 4 times in a row", "4 failed tool calls in a row"] },
  lastMessage: null,
  lastTurnEndAt: null,
  lastEventAt: T - 4000,
};

const busyLive = {
  ...quietLive,
  sessions: [busySession],
  limits: { codex: null, claude: { fiveHour: 64, sevenDay: null, resets: { fiveHour: T + 3600e3, sevenDay: null }, observedAt: T - 60e3, source: "claude-desktop" } },
  limitForecast: { claudeFiveHour: forecast, claudeSevenDay: null },
  collab: [{ a: busySession.id, b: codexSession.id, kind: "file", files: ["a.ts"] }],
  collisions: [{ a: busySession.id, b: codexSession.id, files: ["a.ts"], paths: [`${P.toLowerCase()}/atlas-api/src/a.ts`], ts: T - 9000, branches: [null, "feat/rate-limit"], sameBranch: false, titles: ["claude:0190d3e0", "Review the rate limiter"] }],
  resources: [{ server: "github", ts: T - 5000 }],
};

const S = busySession.id;
const activity = [
  { sessionId: S, app: "claude", kind: "tool", tool: "mcp__agent-wiki__wiki_log", category: "mcp", detail: "agent-wiki.wiki_log", files: [], server: "agent-wiki", pages: ["refunds", "atlas-api"], ts: T },
  { sessionId: S, app: "codex", kind: "tool", tool: "apply_patch", category: "edit", detail: "a.ts, b.ts", files: ["/x/a.ts", "/x/b.ts"], server: null, ts: T },
  { sessionId: S, app: "claude", kind: "tool-error", ts: T, agentId: null },
  { sessionId: S, app: "codex", kind: "tool-error", ts: T },
  { sessionId: S, app: "claude", kind: "turn-end", duration: null, ts: T },
  { sessionId: S, app: "codex", kind: "turn-start", ts: T },
  { sessionId: S, app: "claude", kind: "subagent-stop", agentId: "a1b2", label: "Profile the solver", ts: T },
  { sessionId: S, app: "claude", kind: "hook", detail: "Notification", ts: T },
  { sessionId: S, app: "claude", kind: "ship-it", detail: "3 uncommitted files in atlas-api", projectId: `project:${P.toLowerCase()}/atlas-api`, ts: T },
  { kind: "identity", app: "gh", identity: null, before: "github:github.com/nova-dev", ts: T },
];

const brief = { id: S, title: "Add partial refunds", app: "claude", project: "atlas-api", branch: null };

const theme = {
  name: "Rime, by me",
  about: "Snow over the map",
  extends: "xenonet",
  colors: { bg: "#0b1016", actor: "#c9a0ff", human: "#FFF3D6" },
  stars: { shape: "flake", drift: [0, -0.6, 0], twinkle: 0.5 },
  grid: { style: "hex" },
  sweep: { shape: "none" },
  sonar: { style: "ping" },
  lines: { style: "filament", curvature: 0.3, width: 1 },
  motion: { pace: 0.6, ease: "stepped", breathe: 0.1 },
  actors: {
    form: "lattice-bloom",
    params: { rings: 3, wobble: true },
    palette: ["#5fe1ff", "#b18cff"],
    human: { form: "orb" },
    glow: { opacity: 0.5, scale: 2.4 },
    breathe: { period: 4.5, depth: 0.12 },
    beam: { style: "stream", width: 1.4, speed: 1 },
    react: { flash: 1.6, ripple: true },
  },
};

const picks = { ranAt: T, ms: 9100, fingerprint: "0123456789abcdef", sent: 14, picks: [{ path: "labs/snippets", as: "project", reason: "Its own pyproject.toml." }] };

const stateV1 = {
  layout: { core: { x: 0, y: 0, z: 0 }, "project:/users/nova/projects/atlas-api": { x: 12.5, y: -3.25, z: 40 } },
  acks: { [S]: T },
  marks: { tour: "2026-10-03" },
  projects: { rules: { "/users/nova/projects/labs": { path: `${P}/labs`, as: "group", at: T } }, codex: picks },
  identities: {
    names: { "github:github.com/atlas-co": "work" },
    hidden: { "git:0123456789abcdef": true },
    pins: { [S]: { identity: "github:github.com/atlas-co", at: T } },
    known: { "claude:a:b": { kind: "claude", handle: "nova@atlas.example", detail: { plan: "Team", org: "Atlas" }, firstSeen: T - 30 * 86400e3, lastSeen: T } },
    signins: { claude: [{ id: "claude:a:b", from: T - 30 * 86400e3, to: T }] },
  },
  theme: "phosphor",
};

const stateV2 = {
  version: 2,
  workspace: { root: `${HOME}/Code` },
  layout: {},
  acks: {},
  marks: {},
  projects: { rules: {}, codexByRoot: { "/users/nova/projects": picks, "/users/nova/code": { ...picks, picks: [] } } },
  identities: { names: {}, hidden: {}, pins: {}, known: {}, signins: {} },
};

// The contracts' examples, as written there.
const wrenEvent = { schema: "actor/event/v1", id: "wren:41", actor: "actor:wren", at: 1791234400000, kind: "spawned", target: "claude:6c1f0000-0000-4000-8000-000000000001", detail: "Started a Claude Code session in atlas-api", via: "attributed" };
const wren = {
  schema: "actor/snapshot/v1",
  id: "actor:wren",
  kind: "agent",
  name: "Wren",
  product: { id: "hermes-agent", vendor: "Nous Research", version: "0.21.5" },
  host: null,
  via: ["observed", "attributed"],
  presence: "working",
  presenceSince: 1791230000000,
  lastSeen: 1791234567000,
  heartbeatMs: 60000,
  status: "Triaging issues in atlas-api",
  capabilities: ["spawn-sessions", "edit-repos", "run-commands", "message-people", "schedule"],
  targets: [
    { id: "project:/users/will/projects/atlas-api", relation: "works-on", since: 1791231000000, lastAt: 1791234500000 },
    { id: "claude:6c1f0000-0000-4000-8000-000000000001", relation: "spawned", since: 1791233000000, lastAt: 1791234400000 },
  ],
  recent: [],
  counts: { events24h: 42, spawned24h: 3, messages24h: 5 },
  identity: null,
};
const drift = { schema: "drift/record/v1", source: "anthropic/claude-code/transcript-line", vendorVersion: "2.1.286", kind: "new-field", path: "message.content[].citations", shape: "array<object{type:string,url:string}>", firstSeen: 1791230000000, lastSeen: 1791234567000, count: 12 };

// Hosts: a console with a lab VM and another XenoNet on the same machine.
const hostList = {
  schema: "host/list/v1",
  self: { id: "studio", key: "7c1e0000000000000000000000000001", createdAt: T },
  hosts: [
    {
      id: "vm1",
      transport: "ssh",
      ssh: { hostname: "127.0.0.1", user: "lab", port: 60851, identity: "~/Library/Application Support/XenoNet Dev/lab/console_ed25519", knownHosts: "~/Library/Application Support/XenoNet Dev/lab/known_hosts", hostKeyAlias: "xenonet-lab-vm1" },
      port: 47840,
      command: "/home/lab/.local/bin/xenonet",
      since: "7d",
      detail: false,
      key: "9a2b0000000000000000000000000002",
      name: null,
      addedAt: T,
    },
    { id: "xenonet", transport: "local", ssh: null, port: 47840, command: null, since: "7d", detail: false, key: null, name: null, addedAt: T },
  ],
};
const hello = { t: "hello", schema: "host/hello/v1", protocol: 1, version: "0.4.0", host: { id: "vm1", key: "9a2b0000000000000000000000000002" }, platform: "linux", demo: false, now: T, sinceMs: 7 * 86400e3, detail: false };

export const FIXTURES = [
  // ---------- the map ----------
  { id: "session/v1", name: "a Claude Code session with nothing known but its id", data: claudeSession },
  { id: "session/v1", name: "an archived Codex sub-agent thread on another provider", data: codexSession },
  { id: "session/v1", name: "a session from another host", data: { ...codexSession, id: `@vm1:${codexSession.id}` } },
  { id: "session/v1", name: "an app XenoNet doesn't read", data: { ...codexSession, id: "gemini:x", app: "gemini" }, valid: false },
  { id: "session/v1", name: "a timestamp as text", data: { ...claudeSession, updatedAt: "2026-10-05T12:00:00Z" }, valid: false },
  { id: "graph/v1", name: "one node of every kind, identities off", data: graph },
  { id: "graph/v1", name: "a node of a kind XenoNet doesn't draw", data: { ...graph, nodes: [{ id: "x", kind: "comet", ...placed }] }, valid: false },
  { id: "graph/v1", name: "a link of a kind XenoNet doesn't make", data: { ...graph, links: [{ source: "core", target: "x", kind: "orbit" }] }, valid: false },
  { id: "project/v1", name: "a project Codex picked", data: geofence },
  { id: "project/v1", name: "a project before rules apply", data: (({ rule, ...p }) => p)(geofence) },
  { id: "project/v1", name: "a group found by a way there isn't", data: { ...geofence, role: "group", detected: "vibes" }, valid: false },
  { id: "identity/v1", name: "a pinned identity XenoNet only remembers", data: { id: "claude:a:b", kind: "claude", handle: "nova@atlas.example", detail: { plan: "Team", org: "Atlas" }, signedIn: false, active: false, sources: [], firstSeen: T, name: null, hidden: false, sessions: 0, projects: 0 } },
  { id: "identity/v1", name: "an OpenAI API key", data: { id: "openai:api-key", kind: "openai", handle: null, detail: { plan: null }, signedIn: true, active: false, sources: ["~/.codex/auth.json"], firstSeen: T, seenAt: T, name: "key", hidden: true, sessions: 4, projects: 0 } },

  // ---------- live ----------
  { id: "live/snapshot/v1", name: "a quiet machine with stale Claude usage", data: quietLive },
  { id: "live/snapshot/v1", name: "a session that needs permission and seems stuck", data: busyLive },
  { id: "live/session/v1", name: "a context past its window", data: busySession },
  { id: "live/session/v1", name: "a status XenoNet doesn't set", data: { ...busySession, status: "sleeping" }, valid: false },
  { id: "live/snapshot/v1", name: "a Claude reading from an unknown source", data: { ...busyLive, limits: { codex: null, claude: { ...busyLive.limits.claude, source: "guess" } } }, valid: false },
  ...activity.map((data) => ({ id: "live/activity/v1", name: `activity: ${data.kind}${data.app === "codex" ? " (Codex)" : ""}`, data })),
  { id: "live/activity/v1", name: "a kind nothing emits", data: { sessionId: S, app: "claude", kind: "updated", ts: T }, valid: false },
  { id: "live/activity/v1", name: "a tool call without its category", data: { ...activity[1], category: undefined }, valid: false },

  // ---------- the engine ----------
  { id: "engine/agent/v1", name: "an agent named in a message", data: brief },
  { id: "engine/agent/v1", name: "an agent a message was sent to", data: { ...brief, delivery: "during its current turn" } },
  { id: "engine/agent/v1", name: "an agent in the console's snapshot", data: { ...brief, status: "idle", focus: null, editing: 2, claims: [{ pattern: `${P}/atlas-api/docs/`, note: null, until: T }], reachable: false } },
  { id: "engine/agent/v1", name: "a status the engine doesn't give", data: { ...brief, status: "busy" }, valid: false },
  { id: "engine/agent/v1", name: "an actor among the agents", data: { id: "actor:wren", title: "Wren", app: "actor", project: "atlas-api", branch: "main", status: "working", focus: "Rebuilding the docs", editing: [], claims: [], reachable: true, delivery: "when it next reads its inbox" } },
  { id: "engine/agent/v1", name: "an app the engine doesn't know", data: { ...brief, app: "gemini" }, valid: false },
  { id: "engine/message/v1", name: "a message from an actor, in an inbox", data: { id: 10, from: { id: "actor:wren", title: "Wren", app: "actor", project: "atlas-api", branch: null }, to: "repo:atlas-api", text: "Docs rebuilt.", at: T, replyTo: null } },
  { id: "engine/message/v1", name: "a message as engine.sqlite keeps it", data: { id: 7, sender: S, target: "repo:atlas-api", body: "Moving refunds into a module.", reply_to: null, created_at: T, scope: `${P}/atlas-api/.git` } },
  { id: "engine/message/v1", name: "a reply in an inbox", data: { id: 8, from: brief, to: "you", text: "OK.", at: T, replyTo: 7 } },
  { id: "engine/message/v1", name: "a message with no sender", data: { id: 9, target: "all", text: "hi", at: T, to: [] }, valid: false },
  { id: "engine/claim/v1", name: "a claim as engine.sqlite keeps it", data: { id: 3, agent: S, pattern: `${P}/atlas-api/src/**/*.ts`, note: "routing", created_at: T, expires_at: T + 3600e3 } },
  { id: "engine/contact/v1", name: "an agent that ended", data: { agent: S, app: "claude", cwd: null, via: "hook", at: T, ended_at: T + 5, pid: null } },
  { id: "engine/contact/v1", name: "a contact by another way", data: { agent: S, app: "claude", cwd: null, via: "email", at: T, ended_at: null, pid: 4242 }, valid: false },

  // ---------- the doctor, themes, state ----------
  { id: "doctor/run/v1", name: "a run the xenonet command asked of a running XenoNet, as doctor.json keeps it", data: { schema: "doctor/run/v1", at: T, ms: 412, by: "XenoNet Dev", results: [{ id: "codex.auth", group: "Codex", title: "Codex is signed in", status: "fail", detail: "not signed in", fix: "Run codex login.", ms: 120 }], summary: { ok: 0, info: 0, warn: 0, fail: 1, skip: 0 } } },
  { id: "doctor/result/v1", name: "a status the doctor doesn't give", data: { id: "x.y", group: "X", title: "Y", status: "error", detail: "", ms: 1 }, valid: false },
  { id: "theme/v1", name: "a theme of your own with the new tokens, and a form XenoNet doesn't know yet", data: theme },
  { id: "theme/v1", name: "an empty theme file", data: {} },
  { id: "theme/v1", name: "a color by name", data: { colors: { bg: "black" } }, valid: false },
  { id: "theme/v1", name: "breathing past 1", data: { motion: { breathe: 1.5 } }, valid: false },
  { id: "state/v1", name: "a v1 file with every part", data: stateV1 },
  { id: "state/v1", name: "an empty v1 file", data: {} },
  { id: "state/v2", name: "a v2 file with Codex's picks for two roots", data: stateV2 },
  { id: "state/v2", name: "a v2 file without its version", data: { ...stateV2, version: undefined }, valid: false },
  { id: "state/v2", name: "a rule of a kind there isn't", data: { ...stateV2, projects: { rules: { "/x": { path: "/x", as: "planet", at: T } }, codexByRoot: {} } }, valid: false },

  // ---------- actors and drift ----------
  { id: "actor/snapshot/v1", name: "the contracts' Wren", data: wren },
  { id: "actor/event/v1", name: "the contracts' event", data: wrenEvent },
  { id: "actor/snapshot/v1", name: "you, at another host, with recent events", data: { ...wren, id: "@studio:actor:you", kind: "human", name: "You", product: null, host: "studio", via: ["observed"], presence: "unknown", presenceSince: null, lastSeen: null, heartbeatMs: null, status: null, capabilities: [], targets: [], recent: [wrenEvent], identity: "github:github.com/nova-dev" } },
  { id: "actor/snapshot/v1", name: "a presence and capability from a newer XenoNet", data: { ...wren, presence: "dreaming", capabilities: ["teleport"], via: ["telepathy"] } },
  { id: "actor/list/v1", name: "a list of one", data: { schema: "actor/list/v1", at: T, actors: [wren] } },
  { id: "actor/registration/v1", name: "a custom actor registering", data: { name: "Moth", capabilities: ["message-agents"], heartbeatMs: 30000, pid: 4242 } },
  { id: "actor/snapshot/v1", name: "a status over 140 characters", data: { ...wren, status: "x".repeat(141) }, valid: false },
  { id: "actor/snapshot/v1", name: "more than 20 recent events", data: { ...wren, recent: Array(21).fill(wrenEvent) }, valid: false },
  { id: "actor/snapshot/v1", name: "an id that isn't an actor's", data: { ...wren, id: "wren" }, valid: false },
  { id: "actor/event/v1", name: "a detail over 200 characters", data: { ...wrenEvent, detail: "x".repeat(201) }, valid: false },
  { id: "actor/registration/v1", name: "a registration without a name", data: { capabilities: [] }, valid: false },
  { id: "drift/log/v1", name: "drift.json with a record, looked at once", data: { version: 1, lookedAt: 1791232000000, records: [drift] } },
  { id: "drift/log/v1", name: "drift.json before anything was found", data: { version: 1, lookedAt: null, records: [] } },
  { id: "drift/log/v1", name: "a drift.json of a version XenoNet doesn't write", data: { version: 2, lookedAt: null, records: [] }, valid: false },
  { id: "drift/log/v1", name: "a drift.json whose record isn't one", data: { version: 1, lookedAt: null, records: [{ ...drift, kind: "vibes" }] }, valid: false },
  { id: "drift/record/v1", name: "a field gone from one kind of line", data: { ...drift, kind: "missing-field", path: "payload.cli_version", variant: ["session_meta"], shape: null } },
  { id: "drift/record/v1", name: "a variant that names none", data: { ...drift, variant: [] }, valid: false },
  { id: "drift/record/v1", name: "the contracts' drift record", data: drift },
  { id: "drift/record/v1", name: "a line that isn't JSON", data: { ...drift, kind: "unparsable", vendorVersion: null, path: null, shape: null, count: 1 } },
  { id: "drift/record/v1", name: "a kind of drift there isn't", data: { ...drift, kind: "vibes" }, valid: false },
  { id: "drift/record/v1", name: "a record seen no times", data: { ...drift, count: 0 }, valid: false },

  // ---------- hosts ----------
  { id: "host/list/v1", name: "hosts.json with a lab VM over ssh and another XenoNet here", data: hostList },
  { id: "host/list/v1", name: "hosts.json before hosts were first used", data: { schema: "host/list/v1", self: null, hosts: [] } },
  { id: "host/list/v1", name: "a host named after the machine it's on", data: { ...hostList, hosts: [{ ...hostList.hosts[0], id: "Novas-MacBook-Pro" }] }, valid: false },
  { id: "host/list/v1", name: "an ssh host name that's an option", data: { ...hostList, hosts: [{ ...hostList.hosts[0], ssh: { ...hostList.hosts[0].ssh, hostname: "-oProxyCommand=evil" } }] }, valid: false },
  { id: "host/list/v1", name: "a xenonet command with a shell in it", data: { ...hostList, hosts: [{ ...hostList.hosts[0], command: "xenonet; rm -rf ~" }] }, valid: false },
  { id: "host/list/v1", name: "a key file that breaks a line of ssh's config", data: { ...hostList, hosts: [{ ...hostList.hosts[0], ssh: { ...hostList.hosts[0].ssh, identity: "/tmp/key\nProxyCommand evil" } }] }, valid: false },
  { id: "host/hello/v1", name: "a relay's hello from a VM that named itself", data: hello },
  { id: "host/hello/v1", name: "a hello from a host that hasn't named itself, sending everything", data: { ...hello, host: { id: null, key: hello.host.key }, sinceMs: null, detail: true } },
  { id: "host/hello/v1", name: "a hello that isn't one", data: { ...hello, t: "snapshot" }, valid: false },
  { id: "host/hello/v1", name: "a host key that isn't one XenoNet makes", data: { ...hello, host: { id: "vm1", key: "not-a-key" } }, valid: false },
  { id: "host/hello/v1", name: "a protocol of zero", data: { ...hello, protocol: 0 }, valid: false },
];
