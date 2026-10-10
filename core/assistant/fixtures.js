// Made-up data for remote actors (docs/design/cloud-assistants.md, section
// 7): what a grant reads until you've verified who can call its endpoint,
// and what `xenonet mcp assistant --fixture` serves with no XenoNet at all, to
// check a client and a tunnel against before any real data is involved.
//
// It's a view in source.js's shape, so the same projection and checks run
// on it. It changes on a six-minute cycle, two minutes a phase, so a second
// read a little later sees new states with new times:
//
//   atlas-api   a Claude Code session working, then waiting on its user,
//               then done with its turn; a Codex session idle, then
//               working, then stuck repeating a failing call; the two
//               edit the same files while both work
//   ledger      a Claude Code session, idle
//
// Every answer built from it carries the warning FIXTURE_DATA. Nothing in
// it comes from this computer.
import crypto from "node:crypto";

export const PHASE_MS = 2 * 60e3;
const SESSION = (n) => `0190fa00-0000-7000-8000-${String(n).padStart(12, "0")}`;
export const FIXTURE = {
  places: { atlas: "fixture:atlas-api", ledger: "fixture:ledger" },
  // Their ids for a grant that reads the fixture: fixed, so a client can
  // ask for one again.
  projects: { atlas: "prj_0190fa00-0000-4000-8000-000000000001", ledger: "prj_0190fa00-0000-4000-8000-000000000002" },
  sessions: { work: `claude:${SESSION(1)}`, codex: `codex:${SESSION(2)}`, quiet: `claude:${SESSION(3)}` },
  watcher: "actor:fixture-watcher",
};

// The phase at `t`: 0, 1 or 2, and when it began.
export function phaseAt(t) {
  const n = Math.floor(t / PHASE_MS);
  return { phase: n % 3, since: n * PHASE_MS };
}

export function fixtureView(t = Date.now()) {
  const { phase, since } = phaseAt(t);
  const P = FIXTURE.places;
  const S = FIXTURE.sessions;
  const work = {
    status: phase === 0 ? "tool" : "idle",
    needsYou: phase === 1 ? { kind: "question", since: since + 5e3 } : phase === 2 ? { kind: "review", since: since + 5e3 } : null,
    stuck: null,
    turnStartedAt: phase === 0 ? since : null,
    lastTurnEndAt: phase === 0 ? null : since + 5e3,
    interruptedAt: null,
    lastEventAt: since + 5e3,
  };
  const codex = {
    status: phase === 0 ? "idle" : "thinking",
    needsYou: null,
    stuck: phase === 2 ? { reasons: ["4 failed tool calls in a row"] } : null,
    turnStartedAt: phase === 0 ? null : since,
    lastTurnEndAt: null,
    interruptedAt: null,
    lastEventAt: since + 20e3,
  };
  return {
    at: t,
    fixture: true,
    sources: {
      claude: { state: "ready", observedAt: t, coverage: "complete", reason: "ok" },
      codex: { state: "ready", observedAt: t, coverage: "complete", reason: "ok" },
    },
    places: new Map([
      [P.atlas, { name: "atlas-api (made up)" }],
      [P.ledger, { name: "ledger (made up)" }],
    ]),
    sessions: [
      { id: S.work, app: "claude", place: P.atlas, title: "Add partial refunds (made up)", updatedAt: since + 5e3, archived: false, live: work },
      { id: S.codex, app: "codex", place: P.atlas, title: "Fix the flaky orders test (made up)", updatedAt: since + 20e3, archived: false, live: codex },
      { id: S.quiet, app: "claude", place: P.ledger, title: "Tidy the ledger exports (made up)", updatedAt: t - 3 * 3600e3, archived: false, live: null },
    ],
    collisions: phase === 1 ? [{ a: S.work, b: S.codex, files: 2, ts: since + 30e3 }] : [],
    actors: [{ id: FIXTURE.watcher, kind: "agent", productId: null, name: "Fixture watcher (made up)", presence: "idle", via: ["observed"], remote: null, reachable: false, targets: [{ id: P.atlas, relation: "watches" }] }],
  };
}

// A grant's policy as it reads the fixture: both made-up projects, with its
// own labels switch and the rest as it is.
export function fixturePolicy(policy) {
  return {
    ...policy,
    projects: Object.values(FIXTURE.projects),
    places: new Map([
      [FIXTURE.projects.atlas, FIXTURE.places.atlas],
      [FIXTURE.projects.ledger, FIXTURE.places.ledger],
    ]),
    dataMode: "fixture",
  };
}

// Made-up stats for a fixture session.
export function fixtureStats(id) {
  const seed = crypto.createHash("sha256").update(id).digest();
  const n = (i, scale) => (seed[i] * 256 + seed[i + 1]) % scale;
  return { totals: { input: 2000 + n(0, 9000), cacheWrite: n(2, 4000), cacheRead: 40000 + n(4, 90000), output: 1500 + n(6, 6000) }, tools: [{ calls: 3 + n(8, 40) }], turns: Array.from({ length: 1 + n(10, 9) }, () => ({})) };
}

// The policy `xenonet mcp assistant --fixture` reads with: no grant, no
// device of this computer's, and nothing paired.
export function standalonePolicy({ labels = true } = {}) {
  return fixturePolicy({
    grantId: "gr_000000000000000000000000",
    revision: 1,
    actorId: "actor:fixture-client",
    connectionId: "cn_000000000000000000000000",
    deviceId: "dev_00000000-0000-4000-8000-000000000000",
    deviceAlias: "fixture device",
    productId: null,
    accountLink: "unverified",
    labels,
    lookbackMs: 30 * 24 * 3600e3,
    transportScope: "other_approved_scope",
    audienceLabel: "fixture",
    permissions: ["read"],
  });
}
