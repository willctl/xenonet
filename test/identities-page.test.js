import { test } from "node:test";
import assert from "node:assert/strict";
import { linksOf, passing } from "../src/identities.js";

// A group with one repo, a session in it, one spawned by that session, and
// a session outside any project. Made-up identities.
const nodes = [
  { id: "project:labs", kind: "project", group: true, identities: [{ id: "github:github.com/orbitworks", role: "owner", confidence: "likely" }] },
  { id: "project:labs/geo", kind: "project", identities: [{ id: "github:github.com/orbitworks", role: "owner", confidence: "exact" }, { id: "git:aaaa", role: "commits-as", confidence: "exact" }] },
  { id: "codex:1", kind: "session", ranAs: { id: "chatgpt:ws", confidence: "likely" } },
  { id: "codex:2", kind: "session", ranAs: null },
  { id: "claude:9", kind: "session", ranAs: { id: "claude:a:b", confidence: "exact" } },
];
const parent = new Map([["project:labs/geo", "project:labs"], ["codex:1", "project:labs/geo"], ["codex:2", "codex:1"], ["claude:9", "zone:tmp"]]);
const byId = new Map(nodes.map((n) => [n.id, n]));

test("a session's identities are its own account plus its project's", () => {
  assert.deepEqual(linksOf(byId.get("codex:1"), byId, parent).map((l) => `${l.role} ${l.id}${l.inherited ? " (project)" : ""}`), ["ran-as chatgpt:ws", "owner github:github.com/orbitworks (project)", "commits-as git:aaaa (project)"]);
  // A spawned session reaches its project through the one that spawned it.
  assert.deepEqual(linksOf(byId.get("codex:2"), byId, parent).map((l) => l.role), ["owner", "commits-as"]);
  assert.deepEqual(linksOf(byId.get("claude:9"), byId, parent).map((l) => l.id), ["claude:a:b"]);
});

test("the identity filter lets through the sessions and projects linked to it", () => {
  assert.deepEqual([...passing(nodes, parent, new Set(["github:github.com/orbitworks"]))].sort(), ["codex:1", "codex:2", "project:labs", "project:labs/geo"]);
  assert.deepEqual([...passing(nodes, parent, new Set(["claude:a:b"]))], ["claude:9"]);
  assert.deepEqual([...passing(nodes, parent, new Set(["chatgpt:ws"]))], ["codex:1"]);
});
