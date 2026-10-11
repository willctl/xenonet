// Which sessions an observed actor started (docs/design/actors/
// interface.md, section 1, "attributed"): a Claude Code or Codex process
// running under one of a product's own processes (a Hermes agent host, a
// command Grok Bot's app runs) was started by that actor. Never by timing
// alone: the process's ancestry says who started it, and only then is the
// process matched to the session it holds:
//
//   1. the engine's pins, which say which process holds which session
//      (Claude Code's own hooks, an agent's MCP calls; core/engine/verify.js);
//   2. else the one session on the map of that app that began in the
//      process's folder within two minutes of it, if exactly one did, while
//      the process still ran, and that no other process holds (by its pins).
//
// A process whose session isn't known after two minutes still counts, if
// it ran long enough to hold one: the actor started a session of that app
// in that place. One gone within seconds held none (claude --version,
// claude mcp list, codex login status), and isn't a session started.
import { pathKey } from "../paths.js";

export const SPAWN_WAIT_MS = 2 * 60e3;
// ps gives a process's start to the second, and a transcript's first line
// comes after it.
const SLACK_MS = 2000;
// A process gone this soon after it started, whose session XenoNet never
// knew, held none. XenoNet looks every few seconds, so one it saw gone at
// most this long after its start may have run for less.
export const LIVED_MS = 15e3;
const DONE_MAX = 2048;

// The agents whose sessions products' actors are said to start: Claude
// Code's and Codex's. Grok Build's processes are told apart too
// (procs.js), but attribution stays as it was before them.
const ATTRIBUTED = new Set(["claude", "codex"]);

// The agent processes in a census (procs.census()) that a product's
// processes started: the nearest process of note above each is one that
// `isRoot` picks, with no other agent between (a session's own children
// are that session's). [{ pid, start, agent, root }]
export function startedUnder({ table, of }, isRoot) {
  const out = [];
  for (const p of table.values()) {
    const agent = of(p).agent;
    if (!ATTRIBUTED.has(agent)) continue;
    let n = 0;
    for (let q = p.ppid > 1 ? table.get(p.ppid) : null; q && n < 64; q = q.ppid > 1 ? table.get(q.ppid) : null, n++) {
      if (ATTRIBUTED.has(of(q).agent)) break;
      if (isRoot(q)) {
        out.push({ pid: p.pid, start: p.start, agent, root: q.pid });
        break;
      }
    }
  }
  return out;
}

// The processes seen, each waiting up to two minutes for its session to be
// known. `store`: the engine's (its pins); `map`: what the map knows
// (mapOf's sessions()); `procs`: where each process works (cwds). Each
// settle is told which of them still run, and from then on a process that
// was gone can't have begun a session later.
export class Spawns {
  constructor({ store, map, procs }) {
    this.store = store;
    this.map = map;
    this.procs = procs;
    this.pending = new Map();
    this.done = new Set();
    this.claimed = new Set();
  }

  // Agent processes the actor `actor` started, as startedUnder finds them;
  // each is taken once, with the folder it works in. `cwd` and `place`
  // can come with one, where the observer knows them already.
  async saw(actor, list, at) {
    const key = (p) => `${p.pid}:${p.start}`;
    const fresh = list.filter((p) => !this.pending.has(key(p)) && !this.done.has(key(p)));
    if (!fresh.length) return;
    const unknown = fresh.filter((p) => p.cwd === undefined).map((p) => p.pid);
    const cwds = unknown.length ? await this.procs.cwds(unknown) : new Map();
    for (const p of fresh) this.pending.set(key(p), { actor, pid: p.pid, start: p.start, agent: p.agent, cwd: p.cwd !== undefined ? p.cwd : (cwds.get(p.pid) ?? null), seen: at, goneAt: null });
  }

  // The session a process holds, if XenoNet can say: its pin, else the one
  // session of its app that began in its folder just after it did, before
  // the process was seen gone, and that no other process is pinned to (a
  // session of yours, say, begun nearby while the actor's process ran).
  sessionOf(s) {
    const pins = this.store.pinnedTo(s.pid).filter((pin) => Math.abs(pin.start - s.start) <= SLACK_MS && pin.agent.startsWith(`${s.agent}:`) && !this.claimed.has(pin.agent));
    if (pins.length) return pins.sort((a, b) => b.sure - a.sure || b.at - a.at)[0].agent;
    if (!s.cwd) return null;
    const k = pathKey(s.cwd);
    const until = Math.min(s.start + SPAWN_WAIT_MS, s.goneAt == null ? Infinity : s.goneAt + SLACK_MS);
    const found = (this.map?.sessions?.() ?? []).filter(
      (x) => x.app === s.agent && x.createdAt != null && x.createdAt >= s.start - SLACK_MS && x.createdAt <= until && (pathKey(x.cwd) === k || pathKey(x.worktree) === k) && !this.claimed.has(x.id) && !this.#heldElsewhere(x.id, s),
    );
    return found.length === 1 ? found[0].id : null;
  }

  // Whether a session is pinned to a process other than this one.
  #heldElsewhere(session, s) {
    return this.store.pins(session).some((pin) => pin.pid !== s.pid || Math.abs(pin.start - s.start) > SLACK_MS);
  }

  // The processes settled now: those whose session is known, and those
  // that waited two minutes without, which count only if they ran long
  // enough to hold one. `alive(s)`: whether a process (its pid and start)
  // still runs, from the observer's last look; without it, every one is
  // taken to. [{ actor, agent, cwd, session, start }]
  settle(at, { alive = null } = {}) {
    const out = [];
    for (const [key, s] of this.pending) {
      if (s.goneAt == null && alive && !alive(s)) s.goneAt = at;
      const session = this.sessionOf(s);
      if (!session && at - s.seen < SPAWN_WAIT_MS) continue;
      this.pending.delete(key);
      this.done.add(key);
      if (session) this.claimed.add(session);
      else if (s.goneAt != null && s.goneAt - s.start < LIVED_MS) continue;
      out.push({ actor: s.actor, agent: s.agent, cwd: s.cwd, start: s.start, session });
    }
    if (this.done.size > DONE_MAX) this.done = new Set([...this.done].slice(-DONE_MAX / 2));
    if (this.claimed.size > DONE_MAX) this.claimed = new Set([...this.claimed].slice(-DONE_MAX / 2));
    return out;
  }
}

// A registered actor's spawned reports that named no session, only an app
// and a place (Actors#awaitSession; interface section 2), each waiting up
// to two minutes for a session to start under the actor's own processes
// in that place. When one does, it's recorded again: a second event,
// attributed, with the session for its target, beside the actor's own,
// which stands as it was told. Its process may have started a little
// before the report, which the actor sends once it has started it.
// Ancestry says it's the actor's, and Spawns which session it holds.
export class ReportedSpawns {
  constructor({ actors, procs }) {
    this.actors = actors;
    this.procs = procs;
    this.spawns = new Spawns({ store: actors.store, map: actors.map, procs });
  }

  async look(at) {
    const waiting = this.actors.waiting;
    for (let i = waiting.length - 1; i >= 0; i--) if (at >= waiting[i].until || !this.actors.has(waiting[i].actor)) waiting.splice(i, 1);
    if (!waiting.length) {
      this.spawns.pending.clear();
      return;
    }
    const census = await this.procs.census();
    const byActor = new Map();
    for (const w of waiting) (byActor.get(w.actor) ?? byActor.set(w.actor, []).get(w.actor)).push(w);
    for (const [actor, list] of byActor) {
      const roots = new Set([...this.actors.pidsOf(actor), ...list.flatMap((w) => [...w.own])]);
      const from = Math.min(...list.map((w) => w.at)) - SPAWN_WAIT_MS;
      const started = startedUnder(census, (q) => roots.has(q.pid)).filter((p) => p.start >= from && list.some((w) => !w.app || w.app === p.agent));
      if (started.length) await this.spawns.saw(actor, started, at);
    }
    for (const s of this.spawns.settle(at, { alive: (p) => census.table.get(p.pid)?.start === p.start })) {
      // A session it named itself already isn't the one it didn't.
      if (!s.session || this.actors.targetsOf(s.actor).some((t) => t.id === s.session && t.relation === "spawned")) continue;
      const place = s.cwd ? this.actors.placeOf(s.cwd) : null;
      const i = waiting.findIndex((w) => w.actor === s.actor && (!w.app || w.app === s.agent) && (w.place == null || w.place === place) && Math.abs(s.start - w.at) <= SPAWN_WAIT_MS);
      if (i < 0) continue;
      waiting.splice(i, 1);
      const ref = `spawned:${s.session}`;
      if (!this.actors.store.hasActorRef(s.actor, ref, at - 7 * 24 * 3600e3)) this.actors.record(s.actor, { kind: "spawned", target: s.session, app: s.agent, via: "attributed", at: s.start, ref });
    }
  }
}
