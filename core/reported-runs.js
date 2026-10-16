// Runs an actor reports through the actor gateway (server/actor-gateway.js,
// its report_run tool): a harness running somewhere else, such as in a
// SPACE sandbox, under the actor whose gateway token it holds. Each run is
// a session on the map, "<app>:<id>" with app its harness, in the
// Elsewhere zone, marked observed through the gateway and reported by its
// actor, which gets a beam to it. The actor is the identity that lasts;
// the run is one session of it, and one actor can report many.
//
// Kept in memory: a run goes when it hasn't reported for an hour, or ten
// minutes after it ended, and with the actor's gateway token.
import crypto from "node:crypto";
import { SESSION_APPS } from "./schema/ids.js";

const RUN = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const STALE_MS = 3600e3;
const ENDED_KEEP_MS = 10 * 60e3;
const PER_ACTOR = 32;

export class ReportedRuns {
  constructor({ now = () => Date.now() } = {}) {
    this.now = now;
    this.runs = new Map(); // session id → run
  }

  // A session id from the actor and its own name for the run, so one actor
  // can't name another's runs.
  static idOf(actor, app, run) {
    return `${app}:r${crypto.createHash("sha256").update(`${actor}\n${run}`).digest("hex").slice(0, 31)}`;
  }

  // { ok, id } or { ok: false, error }.
  report(actor, { run, app, title = null, presence = "working" } = {}) {
    if (typeof run !== "string" || !RUN.test(run)) return { ok: false, error: "run is 1 to 64 letters, digits, dots, dashes or underscores" };
    if (!Object.hasOwn(SESSION_APPS, app ?? "")) return { ok: false, error: `app is the harness that runs, one of ${Object.keys(SESSION_APPS).join(", ")}` };
    if (!["working", "idle", "ended"].includes(presence)) return { ok: false, error: "presence is working, idle or ended" };
    const now = this.now();
    this.#prune(now);
    const id = ReportedRuns.idOf(actor, app, run);
    let r = this.runs.get(id);
    if (!r) {
      if ([...this.runs.values()].filter((x) => x.actor === actor).length >= PER_ACTOR) return { ok: false, error: `an actor reports at most ${PER_ACTOR} runs at once` };
      r = { id, actor, app, run, createdAt: now, updatedAt: now, endedAt: null, title: null };
      this.runs.set(id, r);
    }
    r.updatedAt = now;
    if (typeof title === "string" && title.trim()) r.title = title.trim().replace(/\s+/g, " ").slice(0, 90);
    r.endedAt = presence === "ended" ? now : null;
    return { ok: true, id };
  }

  // Every run of an actor goes, its gateway token with it.
  forget(actor) {
    for (const [id, r] of this.runs) if (r.actor === actor) this.runs.delete(id);
  }

  #prune(now) {
    for (const [id, r] of this.runs) if (now - r.updatedAt > STALE_MS || (r.endedAt != null && now - r.endedAt > ENDED_KEEP_MS)) this.runs.delete(id);
  }

  sessions() {
    this.#prune(this.now());
    return [...this.runs.values()].map((r) => ({
      id: r.id,
      app: r.app,
      rawId: r.run,
      title: r.title ?? `${SESSION_APPS[r.app]}, ${r.run}`,
      preview: null,
      cwd: null,
      createdAt: r.createdAt,
      updatedAt: r.updatedAt,
      archived: false,
      model: null,
      branch: null,
      parentId: null,
      parentKind: null,
      observed: "gateway",
      reportedBy: r.actor,
    }));
  }

  live(id) {
    const r = this.runs.get(id);
    return r ? { lastEventAt: r.updatedAt, endedAt: r.endedAt, edits: new Map(), observed: true, status: (now) => (r.endedAt == null && now - r.updatedAt < 120e3 ? "working" : "idle"), needsYou: () => null } : null;
  }

  ids() {
    return [...this.runs.keys()];
  }
}

// The runs XenoNet observes without a reader of its own, from each source
// there is: numbat's records (core/numbat.js) and the runs actors report.
export function observedSessions(sources) {
  const list = sources.filter(Boolean);
  return {
    sources: list,
    // One session per id: the first source to list it wins.
    sessions: () => {
      const seen = new Set();
      return list.flatMap((s) => s.sessions()).filter((x) => !seen.has(x.id) && seen.add(x.id));
    },
    live: (id) => {
      for (const s of list) {
        const l = s.live(id);
        if (l) return l;
      }
      return null;
    },
  };
}
