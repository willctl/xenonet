// Other machines in a network on the map, from presence (src/network-map.js):
// each machine a constellation of its host node, a project per repo or
// folder, and its sessions; ids qualified as the hosts merge does, only the
// fields the map reads, a machine that's away dimmed, a machine on the map
// over SSH left as it is, and what's drawn of machines you're not looking
// at capped as the hosts drawing caps it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { networkPart, withNetwork, partKey } from "../src/network-map.js";
import { remoteAgents, ALONE } from "../src/network-model.js";
import { capHosts, HOST_CAP, hostLayout } from "../src/hosts.js";

const NOW = Date.UTC(2026, 9, 21, 9, 0);
const row = (id, more = {}) => ({ id, keyId: `xnk1_${"a".repeat(32)}`, window: true, founder: false, gateway: false, online: true, seen: NOW, agents: 0, waitsOnYou: 0, you: false, platform: "darwin", ...more });
const state = (hosts) => ({ ...ALONE, network: { id: `net_${"0".repeat(32)}`, name: "home" }, self: { id: "studio", keyId: `xnk1_${"b".repeat(32)}`, window: true, founder: true, gateway: true }, hosts });
const local = () => ({ nodes: [{ id: "core", kind: "core", name: "~/Projects", path: "~/Projects" }, { id: "project:atlas", kind: "project", name: "atlas" }], links: [{ source: "core", target: "project:atlas", kind: "contains" }], relations: [], stats: { sessions: 3, projects: 1, claude: 2, codex: 1, active: 1 } });

const agents = () =>
  remoteAgents([
    { id: "@laptop:claude:6f1c", host: "laptop", app: "claude", title: "Fix the login redirect", status: "waiting", waitsOnYou: true, needsYou: "question", question: "Keep the <b>query</b> string?", repoKey: "github.com/acme/atlas-web", cwd: "~/src/atlas-web", lastActive: NOW - 20e3, lastMessage: "Moved it.", injected: "<img src=x>" },
    { id: "@laptop:codex:019a", host: "laptop", app: "codex", title: "Tidy the billing tests", status: "working", stuck: ["ran cargo test 3 times in a row"], repoKey: "github.com/acme/ledger", cwd: "~/src/ledger", lastActive: NOW - 40e3 },
    { id: "@laptop:claude:77aa", host: "laptop", app: "claude", title: "Second clone", status: "idle", repoKey: "github.com/acme/atlas-web", cwd: "~/work/atlas-web", lastActive: NOW - 3 * 3600e3 },
    { id: "@rack:claude:8d0f", host: "rack", app: "claude", title: "Nightly import", status: "idle", repoKey: null, cwd: "~/jobs/importer", lastActive: NOW - 2 * 3600e3 },
    // Not its host's own: dropped.
    { id: "@laptop:@studio:claude:x", host: "laptop", title: "posing" },
  ]);

test("out of a network, nothing joins the map", () => {
  const part = networkPart({ ...ALONE }, agents(), NOW);
  assert.deepEqual([part.hosts, part.nodes, part.links], [[], [], []]);
  const g = local();
  assert.equal(withNetwork(g, part), g);
});

test("each other machine is a constellation: its host node, a project per repo or folder, and its sessions", () => {
  const s = state([row("studio", { founder: true, gateway: true }), row("laptop"), row("rack", { window: false, online: false, seen: NOW - 7200e3 })]);
  const part = networkPart(s, agents(), NOW);
  assert.deepEqual(part.hosts.map((x) => [x.id, x.state, x.network, x.sessions]), [["laptop", "live", true, 3], ["rack", "offline", true, 1]], "this machine isn't one of them");
  const byId = new Map(part.nodes.map((n) => [n.id, n]));
  // Every id is qualified with its machine.
  assert.ok(part.nodes.every((n) => n.id.startsWith(`@${n.host}:`)));
  assert.ok(!byId.has("@laptop:@studio:claude:x"));
  // Host nodes, as the hosts drawing has them; one that's away is stale.
  assert.deepEqual([byId.get("@laptop:core").kind, byId.get("@laptop:core").state, byId.get("@rack:core").state, byId.get("@rack:core").stale], ["host", "live", "offline", true]);
  // A project per repo key (two clones of atlas-web are one), named by its
  // last part, and one for a folder that isn't a repo.
  const projects = part.nodes.filter((n) => n.kind === "project");
  assert.deepEqual(projects.map((p) => [p.id, p.name, p.sessionCount]).sort(), [
    ["@laptop:project:repo:github.com/acme/atlas-web", "atlas-web", 2],
    ["@laptop:project:repo:github.com/acme/ledger", "ledger", 1],
    ["@rack:project:folder:~/jobs/importer", "importer", 1],
  ]);
  // Sessions with their app, title, state and what waits, from presence;
  // only the fields the map reads.
  const fix = byId.get("@laptop:claude:6f1c");
  assert.deepEqual([fix.kind, fix.app, fix.title, fix.rawId, fix.activity, fix.network], ["session", "claude", "Fix the login redirect", "claude:6f1c", "active", true]);
  assert.deepEqual(fix.presence, { state: "waiting", focus: null, waits: true, kind: "question", question: "Keep the <b>query</b> string?", since: NOW - 20e3, lastMessage: "Moved it.", stuck: null });
  assert.ok(!("injected" in fix) && !JSON.stringify(part).includes("<img"), "nothing it sent is passed on as it came");
  assert.deepEqual(byId.get("@laptop:codex:019a").presence.stuck, ["ran cargo test 3 times in a row"]);
  assert.equal(byId.get("@rack:claude:8d0f").activity, "idle", "a machine that's away has nothing active");
  // The tree: core → host → project → session.
  const parent = new Map(part.links.map((l) => [l.target, l.source]));
  assert.deepEqual([parent.get("@laptop:core"), parent.get("@laptop:project:repo:github.com/acme/ledger"), parent.get("@laptop:codex:019a")], ["core", "@laptop:core", "@laptop:project:repo:github.com/acme/ledger"]);
  assert.equal(part.links.find((l) => l.target === "@laptop:core").kind, "host");
});

test("merged into the map: after this machine's nodes, with the machines among its hosts and this one named; the map given is unchanged", () => {
  const s = state([row("studio"), row("laptop")]);
  const g = local();
  const merged = withNetwork(g, networkPart(s, agents(), NOW));
  assert.deepEqual(g.nodes.length, 2, "unchanged");
  assert.deepEqual(merged.nodes.slice(0, 2), g.nodes);
  assert.deepEqual(merged.hosts.map((x) => x.id), ["laptop"]);
  assert.equal(merged.self, "studio");
  assert.equal(merged.stats.sessions, 3 + 3);
  // The hosts layout takes them as it takes SSH hosts.
  const layout = hostLayout({ random: () => 0.5 });
  assert.equal(layout.prepare(merged.nodes, merged.links), true);
  assert.deepEqual(layout.systems.map((x) => x.host), ["laptop"]);
  // A machine on the map over SSH already keeps that view.
  const ssh = { ...g, nodes: [...g.nodes, { id: "@laptop:core", kind: "host", host: "laptop", state: "live" }], hosts: [{ id: "laptop", state: "live" }] };
  assert.equal(withNetwork(ssh, networkPart(s, agents(), NOW)), ssh);
});

test("the part's key changes with what the map shows of it, and not with a machine's pings", () => {
  const s = state([row("studio"), row("laptop")]);
  const a = agents();
  const key = partKey(networkPart(s, a, NOW));
  assert.equal(partKey(networkPart(state([row("studio"), row("laptop", { seen: NOW + 5000 })]), a, NOW)), key, "a live machine heard again draws nothing new");
  const quiet = a.map((x) => (x.id === "@laptop:claude:6f1c" ? { ...x, waits: false, text: null } : x));
  assert.notEqual(partKey(networkPart(s, quiet, NOW)), key, "a wait that ends does");
  assert.notEqual(partKey(networkPart(state([row("studio"), row("laptop", { online: false })]), a, NOW)), key, "a machine that goes away does");
});

test("what's drawn of machines you're not looking at is capped, waits first", () => {
  const many = [];
  for (const host of ["laptop", "desk", "rack"])
    for (let i = 0; i < 40; i++) many.push({ id: `@${host}:claude:${String(i).padStart(4, "0")}`, host, app: "claude", title: `s${i}`, status: "idle", repoKey: `github.com/acme/r${i % 4}`, cwd: `~/src/r${i % 4}`, lastActive: NOW - i * 60e3, waitsOnYou: host === "rack" && i === 39 });
  const s = state([row("studio"), row("laptop"), row("desk"), row("rack")]);
  const part = networkPart(s, remoteAgents(many), NOW);
  const parentOf = new Map(part.links.map((l) => [l.target, l.source]));
  const attention = new Set(part.nodes.filter((n) => n.presence?.waits).map((n) => n.id));
  const { hidden } = capHosts(part.nodes, { parentOf, attention });
  const drawn = part.nodes.filter((n) => n.kind !== "host" && !hidden.has(n.id));
  assert.ok(drawn.length <= HOST_CAP + 2, `${drawn.length} drawn`);
  assert.ok(drawn.some((n) => n.id === "@rack:claude:0039"), "what waits on you is drawn");
});
