import { test } from "node:test";
import assert from "node:assert/strict";
import { scopeGraph, scopeCounts, loadScope, saveScope, DEFAULT_SCOPE } from "../src/scope.js";
import { subAgentOf, threadToSession } from "../core/codex.js";

const now = Date.parse("2026-10-02T12:00:00Z");
const day = 86400e3;
const session = (id, ageDays, extra = {}) => ({ id, kind: "session", app: "codex", updatedAt: now - ageDays * day, ...extra });
const graph = {
  nodes: [
    { id: "core", kind: "core" },
    { id: "project:a", kind: "project", lastActivity: now - 1 * day },
    { id: "project:stale", kind: "project", lastActivity: now - 200 * day },
    { id: "zone:tmp", kind: "zone" },
    { id: "native:codex:1", kind: "native" },
    session("new", 1),
    session("old", 40),
    session("old-child", 40, { parentId: "old" }),
    session("guardian", 1, { background: true }),
    session("in-zone", 60),
    session("pinned", 90, { pinned: true }),
  ],
  links: [
    { source: "core", target: "project:a", kind: "contains" },
    { source: "core", target: "project:stale", kind: "contains" },
    { source: "core", target: "zone:tmp", kind: "zone" },
    { source: "project:a", target: "native:codex:1", kind: "native" },
    { source: "project:a", target: "new", kind: "session" },
    { source: "native:codex:1", target: "old", kind: "session" },
    { source: "old", target: "old-child", kind: "lineage" },
    { source: "project:a", target: "guardian", kind: "session" },
    { source: "zone:tmp", target: "in-zone", kind: "session" },
    { source: "project:a", target: "pinned", kind: "session" },
  ],
  relations: [],
};
const ids = (g) => g.nodes.map((n) => n.id).sort();

test("the default scope draws the last week without background agents", () => {
  const g = scopeGraph(graph, DEFAULT_SCOPE, { now });
  assert.deepEqual(ids(g), ["core", "new", "pinned", "project:a"]);
  assert.deepEqual(g.scope, { sessions: { shown: 2, total: 6 }, projects: { shown: 1, total: 2 } });
  assert.ok(g.links.every((l) => ids(g).includes(l.source) && ids(g).includes(l.target)));
});

test("projects untouched within the scope leave the map, unless kept or pinned", () => {
  assert.ok(!ids(scopeGraph(graph, { window: "30d", background: false }, { now })).includes("project:stale"));
  assert.ok(ids(scopeGraph(graph, { window: "all", background: false }, { now })).includes("project:stale"));
  assert.ok(ids(scopeGraph(graph, DEFAULT_SCOPE, { now, keep: new Set(["project:stale"]) })).includes("project:stale"));
  const pinned = { ...graph, nodes: graph.nodes.map((n) => (n.id === "project:stale" ? { ...n, pinned: true } : n)) };
  assert.ok(ids(scopeGraph(pinned, DEFAULT_SCOPE, { now })).includes("project:stale"));
});

test("a drawn session keeps its project on the map, however old the project", () => {
  const quiet = { ...graph, nodes: graph.nodes.map((n) => (n.id === "project:a" ? { ...n, lastActivity: now - 300 * day } : n)) };
  assert.ok(ids(scopeGraph(quiet, DEFAULT_SCOPE, { now })).includes("project:a"));
});

test("kept sessions bring the sessions and folders they hang from", () => {
  const g = scopeGraph(graph, DEFAULT_SCOPE, { now, keep: new Set(["old-child", "in-zone"]) });
  assert.deepEqual(ids(g), ["core", "in-zone", "native:codex:1", "new", "old", "old-child", "pinned", "project:a", "zone:tmp"]);
});

test("All with background agents draws everything", () => {
  const g = scopeGraph(graph, { window: "all", background: true }, { now });
  assert.equal(g.nodes.length, graph.nodes.length);
  assert.equal(g.links.length, graph.links.length);
  assert.deepEqual(scopeCounts(graph, { window: "7d", background: false }, now), {
    "24h": { sessions: 1, projects: 1 },
    "7d": { sessions: 1, projects: 1 },
    "30d": { sessions: 1, projects: 1 },
    all: { sessions: 5, projects: 2 },
  });
});

test("the scope is remembered, and a bad saved value falls back to the default", () => {
  const store = new Map();
  const storage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, v) };
  assert.deepEqual(loadScope(storage), DEFAULT_SCOPE);
  saveScope({ window: "all", background: true }, storage);
  assert.deepEqual(loadScope(storage), { window: "all", background: true });
  store.set("xenonet.scope", JSON.stringify({ window: "1y" }));
  assert.deepEqual(loadScope(storage), DEFAULT_SCOPE);
});

test("Codex sub-agents: spawned ones nest under their parent, the rest are background", () => {
  const spawned = { id: "c1", source: { subAgent: { thread_spawn: { parent_thread_id: "p1", depth: 1, agent_nickname: "Popper" } } } };
  assert.deepEqual(subAgentOf(spawned), { parent: "p1", nickname: "Popper", kind: "spawned" });
  const s = threadToSession(spawned, false);
  assert.equal(s.parentId, "codex:p1");
  assert.equal(s.parentKind, "subagent");
  assert.equal(s.agentNickname, "Popper");
  assert.equal(s.background, false);
  const guardian = threadToSession({ id: "g1", source: { subAgent: { other: "guardian" } } }, false);
  assert.equal(guardian.agentKind, "guardian");
  assert.equal(guardian.background, true);
  assert.equal(guardian.parentId, null);
  assert.equal(threadToSession({ id: "r1", source: { subAgent: "review" } }, false).background, true);
  assert.equal(threadToSession({ id: "e1", source: "exec" }, false).background, true);
  assert.equal(threadToSession({ id: "v1", source: "vscode" }, false).background, false);
});
