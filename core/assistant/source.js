// What the assistant reads from (docs/design/cloud-assistants.md, section
// 5): XenoNet's own state as the service holds it in memory, never the map
// with other hosts' in it, and never a scan of its own. A view is what
// projection.js takes: each source's health, the places, the sessions with
// what the tailer saw of each, the collisions and the actors. One is made
// at most every second, however many reads come in.
//
// Each app's source stands as its session list's gatherer and the tailer
// do: ready while both work, degraded when one has a problem (XenoNet then
// serves what it had), unavailable when the list failed or hasn't run yet.
// The tailer sweeps every two seconds while it runs, so its view counts as
// observed now; the list's is when it was last gathered.

import { productOf } from "../actors/products.js";

const VIEW_MS = 1000;

function sourceOf(gatherer, tailer, t) {
  const g = gatherer?.health?.();
  const l = tailer?.health?.();
  if (!g || g.state === "idle" || g.lastOk == null) return { state: "unavailable", observedAt: null, coverage: "unknown", reason: "not_started" };
  if (g.state === "failed" || g.state === "stopped") return { state: "unavailable", observedAt: g.lastOk, coverage: "unknown", reason: g.state === "stopped" ? "stopped" : "read_failed" };
  const tailing = l?.state === "running";
  const degraded = g.state === "degraded" || !tailing;
  const liveAt = tailing ? t : (l?.lastOk ?? null);
  return { state: degraded ? "degraded" : "ready", observedAt: liveAt == null ? g.lastOk : Math.min(g.lastOk, liveAt), coverage: degraded ? "partial" : "complete", reason: degraded ? "read_failed" : "ok" };
}

// `state`: XenoNet's (core/state.js); `actors`: the registry, or null.
export function stateSource({ state, actors = null, now = Date.now }) {
  let cached = null;
  function view() {
    const t = now();
    if (cached && t - cached.at < VIEW_MS) return cached;
    const live = state.liveSnapshot();
    const liveById = new Map(live.sessions.map((s) => [s.id, s]));
    const places = new Map();
    const sessions = [];
    for (const n of state.index.values()) {
      if ((n.kind === "project" || n.kind === "zone") && !n.host) places.set(n.id, { name: n.name ?? null, kind: n.kind });
      if (n.kind !== "session" || n.host) continue;
      const l = liveById.get(n.id);
      sessions.push({
        id: n.id,
        app: n.app,
        place: state.homeOf(n.id),
        title: n.title ?? null,
        updatedAt: n.updatedAt ?? null,
        archived: !!n.archived,
        live: l ? { status: l.status, needsYou: l.needsYou ? { kind: l.needsYou.kind, since: l.needsYou.since ?? null } : null, stuck: l.stuck ? { reasons: [...l.stuck.reasons] } : null, turnStartedAt: l.turnStartedAt ?? null, lastTurnEndAt: l.lastTurnEndAt ?? null, interruptedAt: l.interruptedAt ?? null, lastEventAt: l.lastEventAt ?? null } : null,
      });
    }
    const list = actors ? actors.list({ now: t }).actors : [];
    cached = {
      at: t,
      fixture: false,
      sources: { claude: sourceOf(state.claudeSessions, state.live?.tailer, t), codex: sourceOf(state.codexThreads, state.live?.tailer, t), grok: sourceOf(state.grokSessions, state.live?.tailer, t) },
      places,
      sessions,
      collisions: live.collisions.map((c) => ({ a: c.a, b: c.b, files: c.files?.length ?? 0, ts: c.ts ?? null })),
      actors: list.map((a) => ({
        id: a.id,
        kind: a.kind,
        productId: productOf(a.product?.id)?.id ?? null,
        name: a.name,
        presence: a.presence,
        via: [...(a.via ?? [])],
        remote: a.remote ?? null,
        // Whether messages reach it, as the engine says: one that reports
        // itself and isn't offline. A remote connection never makes one.
        reachable: !!actors?.isRegistered(a.id) && a.presence !== "offline",
        targets: (a.targets ?? []).map((x) => ({ id: x.id, relation: x.relation })),
      })),
    };
    return cached;
  }
  return {
    view,
    // A session's stats from its transcript (core/stats.js), read only when
    // asked for.
    stats: (id) => state.sessionStats(id),
    // An actor as the assistant shows itself, from the registry.
    actor(id) {
      const a = actors?.snapshot(id);
      return a ? { id: a.id, kind: a.kind, productId: productOf(a.product?.id)?.id ?? null, name: a.name, presence: a.presence, via: [...a.via], remote: a.remote ?? null, reachable: false, targets: (a.targets ?? []).map((x) => ({ id: x.id, relation: x.relation })) } : null;
    },
    // A place's node id on the map, for an actor's watches target.
    has: (id) => state.index.has(id),
  };
}
