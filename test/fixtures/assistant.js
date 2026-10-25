// A made-up world for the remote actors' tests: an actor registry and its
// store in memory, connections with their files in a temporary folder, a
// clock the test moves, and a view of a made-up map in source.js's shape
// (two places a grant may name, one it doesn't, sessions in each, with
// canaries in their titles that must never leave).
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Actors } from "../../core/actors/index.js";
import { EngineStore } from "../../core/engine/store.js";
import { createConnections } from "../../core/actors/connections.js";

export const T = 1791230000000;
export const MIN = 60e3;
export const HOUR = 3600e3;
export const DAY = 24 * HOUR;

export const ATLAS = "project:/users/nova/projects/atlas-api";
export const LEDGER = "project:/users/nova/projects/ledger";
export const SECRET = "project:/users/nova/projects/secret-plans";
export const S = {
  waiting: "claude:0190d3e0-0000-7000-8000-00000000ab01",
  working: "codex:0190d3e0-0000-7000-8000-00000000ab02",
  stuck: "claude:0190d3e0-0000-7000-8000-00000000ab03",
  done: "claude:0190d3e0-0000-7000-8000-00000000ab04",
  ledger: "codex:0190d3e0-0000-7000-8000-00000000ab05",
  hidden: "claude:0190d3e0-0000-7000-8000-00000000ab06",
  archived: "claude:0190d3e0-0000-7000-8000-00000000ab07",
  old: "claude:0190d3e0-0000-7000-8000-00000000ab08",
};

// Things that must never be in an answer, a log or stderr.
export const CANARIES = ["sk-live-CANARY0123456789abcdef", "C:\\Users\\nova\\secret", "/Users/nova/secret", "nova@example.com", "hunter2-canary", "Should I deploy to prod?", "npm run db:push --force", "secret-plans", "ghp_CANARY0123456789012345678901234567"];

export function world({ now = T } = {}) {
  let t = now;
  const clock = { now: () => t, set: (v) => (t = v), add: (ms) => (t += ms) };
  const store = new EngineStore(":memory:");
  const actors = new Actors({ store, now: clock.now, map: null, alive: () => true });
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-assistant-"));
  const places = new Map([
    [ATLAS, { name: "atlas-api" }],
    [LEDGER, { name: "ledger" }],
    [SECRET, { name: "secret-plans" }],
  ]);
  const make = (opts = {}) =>
    createConnections({ actors, store, dir, fingerprint: "this-machine", now: clock.now, describe: (p) => places.get(p)?.name ?? null, credentialIo: { protect: () => {}, readers: () => [] }, ...opts });
  const connections = make();
  const done = () => {
    actors.stop();
    fs.rmSync(dir, { recursive: true, force: true, maxRetries: 5 });
  };
  return { clock, store, actors, dir, places, connections, make, done, identities: new Set(["chatgpt:demo-workspace", "claude:acct:org"]) };
}

// A pairing as the window sends it.
export const ask = (over = {}) => ({
  label: "My dot",
  product: "dots",
  transport: "openai-secure-mcp-tunnel",
  audience: { description: "Only my ChatGPT account, in my workspace", verification: "owner_verified", scope: "owner_endpoint" },
  projects: [ATLAS],
  ...over,
});

// A view of the made-up map, as source.js makes one, at `t`.
export function view(t = T, { claude = "ready", codex = "ready", grok = "ready", observedAgo = 1000 } = {}) {
  const src = (state) => ({ state, observedAt: state === "unavailable" ? null : t - observedAgo, coverage: state === "ready" ? "complete" : state === "degraded" ? "partial" : "unknown", reason: state === "ready" ? "ok" : "read_failed" });
  const live = (o) => ({ status: "idle", needsYou: null, stuck: null, turnStartedAt: null, lastTurnEndAt: null, interruptedAt: null, lastEventAt: t - 5 * MIN, ...o });
  return {
    at: t,
    fixture: false,
    sources: { claude: src(claude), codex: src(codex), grok: src(grok) },
    places: new Map([
      [ATLAS, { name: "atlas-api  sk-live-CANARY0123456789abcdef" }],
      [LEDGER, { name: "ledger" }],
      [SECRET, { name: "secret-plans" }],
    ]),
    sessions: [
      { id: S.waiting, app: "claude", place: ATLAS, title: "Refunds for nova@example.com\nC:\\Users\\nova\\secret password=hunter2-canary", updatedAt: t - 2 * MIN, archived: false, live: live({ needsYou: { kind: "question", since: t - 2 * MIN }, lastEventAt: t - 2 * MIN }) },
      { id: S.working, app: "codex", place: ATLAS, title: "Fix tests", updatedAt: t - 1 * MIN, archived: false, live: live({ status: "tool", turnStartedAt: t - 3 * MIN, lastEventAt: t - 1 * MIN }) },
      { id: S.stuck, app: "claude", place: ATLAS, title: "Migrate", updatedAt: t - 3 * MIN, archived: false, live: live({ status: "thinking", stuck: { reasons: ["ran npm run db:push --force 4 times in a row"] }, turnStartedAt: t - 9 * MIN, lastEventAt: t - 3 * MIN }) },
      { id: S.done, app: "claude", place: ATLAS, title: "Docs", updatedAt: t - 10 * MIN, archived: false, live: live({ needsYou: { kind: "review", since: t - 10 * MIN }, lastTurnEndAt: t - 10 * MIN, lastEventAt: t - 10 * MIN }) },
      { id: S.ledger, app: "codex", place: LEDGER, title: "Close the month", updatedAt: t - 2 * DAY, archived: false, live: null },
      { id: S.hidden, app: "claude", place: SECRET, title: "secret-plans launch", updatedAt: t - MIN, archived: false, live: live({ needsYou: { kind: "permission", since: t - MIN }, lastEventAt: t - MIN }) },
      { id: S.archived, app: "claude", place: ATLAS, title: "Old", updatedAt: t - MIN, archived: true, live: null },
      { id: S.old, app: "claude", place: ATLAS, title: "Ancient", updatedAt: t - 40 * DAY, archived: false, live: null },
    ],
    collisions: [
      { a: S.waiting, b: S.working, files: 3, ts: t - 4 * MIN },
      { a: S.working, b: S.hidden, files: 7, ts: t - 4 * MIN },
    ],
    actors: [
      { id: "actor:you", kind: "human", productId: null, name: "You", presence: "working", via: ["observed"], remote: null, reachable: false, targets: [{ id: ATLAS, relation: "works-on" }] },
      { id: "actor:kestrel", kind: "agent", productId: "grok-bot", name: "Kestrel", presence: "idle", via: ["observed"], remote: null, reachable: false, targets: [{ id: ATLAS, relation: "watches" }, { id: SECRET, relation: "works-on" }] },
      { id: "actor:wren", kind: "agent", productId: null, name: "Wren", presence: "working", via: ["registered"], remote: null, reachable: true, targets: [{ id: SECRET, relation: "works-on" }] },
    ],
  };
}
