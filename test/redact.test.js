// What a poster or film of the day may say (core/redact.js): star names
// in place of project names, the same all day and never reversible, and
// nothing of the day's names, paths, hosts or times. The fixture's names
// are made up to look real: a home folder with a user name, repos,
// branches, a host and prompts.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { dayLog } from "../core/digest.js";
import { starName, starNames, posterSalt, archetypeOf, constellationOf, labelsOf, dayKey } from "../core/redact.js";

const salt = "a".repeat(64);
const at = (h, m = 0) => new Date(2026, 9, 3, h, m).getTime();

// A day with the kinds of names an export must never carry.
const HOME = "/Users/dana.whitfield";
const SECRETS = ["dana", "whitfield", "atlas-api", "ledger", "harbor", "feat/refund-flow", "dana-mbp.local", "github.com", "partial refunds", "routes.ts", "reconcile.py", HOME];
const events = [
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", kind: "prompt", ts: at(9), detail: "Add partial refunds to the orders API" },
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", kind: "tool", category: "edit", files: [`${HOME}/Projects/atlas-api/src/orders/routes.ts`], ts: at(9, 5) },
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", kind: "tool", category: "shell", ts: at(9, 6) },
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", kind: "message", detail: "Pushed feat/refund-flow to github.com/dana/atlas-api", ts: at(9, 8) },
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000001", app: "claude", kind: "turn-end", ts: at(9, 9) },
  { sessionId: "codex:0190d3e0-0000-7000-8000-000000000002", app: "codex", kind: "tool", category: "edit", files: [`${HOME}/Projects/ledger/ledger/reconcile.py`], ts: at(14) },
  { sessionId: "codex:0190d3e0-0000-7000-8000-000000000002", app: "codex", kind: "tool", category: "shell", ts: at(14, 1) },
  { sessionId: "claude:0190d3e0-0000-7000-8000-000000000003", app: "claude", kind: "tool", category: "read", ts: at(14, 2) },
];
const homes = { "claude:0190d3e0-0000-7000-8000-000000000001": `project:${HOME}/Projects/atlas-api`, "codex:0190d3e0-0000-7000-8000-000000000002": `project:${HOME}/Projects/ledger`, "claude:0190d3e0-0000-7000-8000-000000000003": `project:${HOME}/Clients/harbor-api` };
const day = () =>
  dayLog(events, {
    now: at(18),
    titleOf: (id) => (id.startsWith("claude") ? "Add partial refunds to the orders API" : "Reconcile the ledger on dana-mbp.local"),
    homeOf: (id) => homes[id],
    nameOf: (id) => id.split("/").pop(),
    live: { today: 123456, sessions: [] },
    repos: [{ name: "atlas-api", dirty: 2, branch: "feat/refund-flow" }],
  });

test("a star name is the same all day, and another the next day or under another salt", () => {
  const id = `project:${HOME}/Projects/atlas-api`;
  const a = starName(id, { salt, day: "2026-10-03" });
  assert.equal(starName(id, { salt, day: "2026-10-03" }), a);
  assert.match(a, /^[A-Z][a-z]{3,}$/, a);
  assert.notEqual(starName(id, { salt, day: "2026-10-04" }), a, "the next day");
  assert.notEqual(starName(id, { salt: "b".repeat(64), day: "2026-10-03" }), a, "another salt");
  assert.notEqual(starName(`project:${HOME}/Projects/atlas-web`, { salt, day: "2026-10-03" }), a, "another project");
  assert.equal(dayKey(at(9)), "2026-10-03");
});

test("a star name carries nothing of the id", () => {
  for (const id of [`project:${HOME}/Projects/atlas-api`, "project:C:\\Users\\dana\\src\\ledger", "zone:dana-mbp.local"]) {
    const name = starName(id, { salt, day: "2026-10-03" }).toLowerCase();
    for (const word of id.toLowerCase().split(/[^a-z]+/).filter((w) => w.length >= 3)) assert.ok(!name.includes(word), `${name} carries ${word}`);
  }
});

test("a set of ids gets a different name each, whatever order they come in", () => {
  const ids = Array.from({ length: 2000 }, (_, i) => `project:${HOME}/Projects/repo-${i}`);
  const names = starNames(ids, { salt, day: "2026-10-03" });
  assert.equal(new Set(names.values()).size, ids.length);
  const again = starNames([...ids].reverse(), { salt, day: "2026-10-03" });
  for (const id of ids) assert.equal(again.get(id), names.get(id));
  for (const name of names.values()) assert.match(name, /^[A-Z][a-z]+( \d+)?$/, name);
});

test("the salt is made once, in the state folder, for you alone", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-salt-"));
  try {
    const a = posterSalt(dir);
    assert.match(a, /^[0-9a-f]{64}$/);
    assert.equal(posterSalt(dir), a, "kept");
    assert.equal(fs.readFileSync(path.join(dir, "poster-salt"), "utf8").trim(), a);
    if (process.platform !== "win32") assert.equal(fs.statSync(path.join(dir, "poster-salt")).mode & 0o777, 0o600);
    fs.writeFileSync(path.join(dir, "poster-salt"), "not a salt\n");
    const b = posterSalt(dir);
    assert.match(b, /^[0-9a-f]{64}$/);
    assert.notEqual(b, a, "a broken file is replaced");
    assert.notEqual(posterSalt(dir), "not a salt");
    assert.equal(posterSalt(null), posterSalt(null), "without a folder, one salt for the run");
    assert.notEqual(posterSalt(null), b);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("the day's archetype", () => {
  const hours = (spec) => Object.assign(new Array(24).fill(0), spec);
  assert.equal(archetypeOf({ totals: { sessions: 0 }, hours: hours({}) }), "quiet sky");
  assert.equal(archetypeOf({ totals: { sessions: 1 }, hours: hours({ 9: 40 }) }), "lone star");
  assert.equal(archetypeOf({ totals: { sessions: 2 }, hours: hours({ 9: 40 }) }), "binary");
  assert.equal(archetypeOf({ totals: { sessions: 3 }, hours: hours({ 9: 30, 15: 5 }) }), "pulsar", "a burst");
  assert.equal(archetypeOf({ totals: { sessions: 3 }, hours: hours({ 9: 3, 15: 1 }) }), "cluster", "too few calls for a burst");
  assert.equal(archetypeOf({ totals: { sessions: 4 }, hours: hours({ 9: 10, 12: 10, 15: 10 }) }), "cluster");
  assert.equal(archetypeOf({ totals: { sessions: 6 }, hours: hours({ 9: 10, 12: 10, 15: 10 }) }), "swarm");
  assert.equal(archetypeOf({}), "quiet sky");
});

test("the constellation of the day: star names, totals and the archetype, and nothing of the names, paths, hosts or times", () => {
  const log = day();
  // The log itself has them, so the check means something.
  const raw = JSON.stringify(log);
  assert.ok(SECRETS.every((s) => raw.toLowerCase().includes(s.toLowerCase())), "the fixture carries each secret");
  assert.ok(log.busiestHour != null && log.firstAt != null);

  const c = constellationOf(log, { salt, now: at(18) });
  const text = JSON.stringify(c);
  for (const s of SECRETS) assert.ok(!text.toLowerCase().includes(s.toLowerCase()), `${s} leaked: ${text}`);
  assert.doesNotMatch(text, /\b\d{1,2}:\d{2}\b/, "a clock time");
  assert.doesNotMatch(text, /\d{13}/, "a timestamp");
  for (const key of ["hours", "busiestHour", "firstAt", "lastAt", "sessions", "uncommitted", "topProjects"]) assert.ok(!(key in c), key);
  assert.deepEqual(Object.keys(c).sort(), ["apps", "archetype", "date", "stars", "tokens", "totals"]);
  assert.equal(c.date, log.date);
  assert.equal(c.archetype, "cluster");
  assert.deepEqual(c.totals, { sessions: 3, turns: 1, toolCalls: 5, files: 2 });
  assert.equal(c.tokens, 123456);
  assert.deepEqual(c.apps, { claude: 2, codex: 1 });
  assert.equal(c.stars.length, 3);
  assert.deepEqual(c.stars.map((s) => s.toolCalls), [2, 2, 1]);
  const names = starNames(log.topProjects.map((p) => p.id), { salt, day: "2026-10-03" });
  assert.deepEqual(c.stars.map((s) => s.name), log.topProjects.map((p) => names.get(p.id)), "the same names the day's other exports get");
  assert.ok(c.stars.every((s) => !("project" in s) && !("id" in s)));

  // Revealed: each star carries its project's name, and nothing else changes.
  const shown = constellationOf(log, { salt, now: at(18), names: true });
  assert.deepEqual(shown.stars.map((s) => s.project), ["atlas-api", "ledger", "harbor-api"]);
  assert.deepEqual(shown.stars.map((s) => s.name), c.stars.map((s) => s.name));

  const quiet = constellationOf(dayLog([], { now: at(18) }), { salt, now: at(18) });
  assert.deepEqual({ ...quiet, date: null }, { date: null, archetype: "quiet sky", totals: { sessions: 0, turns: 0, toolCalls: 0, files: 0 }, tokens: 0, stars: [], apps: {} });
});

// The map's nodes an export would show, of every kind the map labels, with
// the day's projects among them.
const NODES = [
  { id: "core", kind: "core", name: "Projects", path: `${HOME}/Projects` },
  { id: `project:${HOME}/Projects/atlas-api`, kind: "project", name: "atlas-api" },
  { id: `project:${HOME}/Projects/ledger`, kind: "project", name: "ledger" },
  { id: `project:${HOME}/Clients/harbor-api`, kind: "project", name: "harbor-api" },
  { id: `project:${HOME}/Clients`, kind: "project", name: "Clients", group: true },
  { id: `zone:${HOME}/Scratch`, kind: "zone", name: "Scratch" },
  { id: "claude:0190d3e0-0000-7000-8000-000000000001", kind: "session", title: "Add partial refunds to the orders API" },
  { id: "page:partial-refunds", kind: "page", name: "Partial refunds" },
  { id: "resource:github", kind: "resource", name: "github" },
  { id: "native:codex-1", kind: "native", name: "Codex" },
  { id: `wiki:${HOME}/wiki`, kind: "wiki", name: "Agent Wiki" },
  { id: "host:dana-mbp.local", kind: "host", name: "dana-mbp.local" },
  { id: "actor:hermes", kind: "actor", name: "Hermes" },
];

test("the map's labels for an export: a star name for every labelled node, none for a session or an actor, the same names the constellation gets, and nothing of the names", () => {
  const log = day();
  const labels = labelsOf(NODES, log, { salt, now: at(18) });
  const labelled = NODES.filter((n) => !["session", "actor"].includes(n.kind));
  assert.deepEqual(Object.keys(labels).sort(), labelled.map((n) => n.id).sort());
  for (const n of labelled) assert.match(labels[n.id], /^[A-Z][a-z]+( \d+)?$/, `${n.id}: ${labels[n.id]}`);
  assert.equal(new Set(Object.values(labels)).size, labelled.length, "each node its own name");
  const values = Object.values(labels).join(" ").toLowerCase();
  for (const s of SECRETS) assert.ok(!values.includes(s.toLowerCase()), `${s} in a label`);
  // The constellation, made with the same nodes, names its stars the same way.
  const c = constellationOf(log, { salt, now: at(18), nodes: NODES });
  assert.deepEqual(c.stars.map((s) => s.name), log.topProjects.map((p) => labels[p.id]));
  assert.ok(!("labels" in c), "the constellation itself carries no lookup");
  // The same all day, and new the next day.
  assert.deepEqual(labelsOf(NODES, log, { salt, now: at(23) }), labels);
  assert.notDeepEqual(labelsOf(NODES, log, { salt, now: at(18) + 24 * 3600e3 }), labels);
  // Without nodes, nothing.
  assert.deepEqual(labelsOf([], log, { salt, now: at(18) }), {});
});
