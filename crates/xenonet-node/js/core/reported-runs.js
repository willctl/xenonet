// core/reported-runs.js in Rust (crates/xenonet-sources/src/reported_runs.rs):
// runs an actor reports through the actor gateway, each a session on the
// map under its actor. Rust keeps the runs, checks each report, makes the
// ids and lets runs go; the clock (`now`) stays this one's.
import { native } from "../index.js";

export class ReportedRuns {
  #core = new native.ReportedRunsCore();

  constructor({ now = () => Date.now() } = {}) {
    this.now = now;
  }

  // A session id from the actor and its own name for the run, so one actor
  // can't name another's runs.
  static idOf(actor, app, run) {
    return native.reportedRunId(`${actor}`, `${app}`, `${run}`);
  }

  // The runs, by session id (a copy: Rust keeps them).
  get runs() {
    return new Map(this.#core.runs().map((r) => [r.id, r]));
  }

  // { ok, id } or { ok: false, error }.
  report(actor, { run, app, title = null, presence = "working" } = {}) {
    return this.#core.report(String(actor), typeof run === "string" ? run : null, String(app ?? ""), typeof title === "string" ? title : null, typeof presence === "string" ? presence : null, () => Number(this.now()));
  }

  // Every run of an actor goes, its gateway token with it.
  forget(actor) {
    this.#core.forget(String(actor));
  }

  sessions() {
    return this.#core.sessions(Number(this.now()));
  }

  live(id) {
    const r = typeof id === "string" ? this.#core.live(id) : null;
    if (!r) return null;
    const { updatedAt, endedAt, serial } = r;
    return { lastEventAt: updatedAt, endedAt, edits: new Map(), observed: true, status: (now) => this.#core.statusOf(id, serial, endedAt, updatedAt, Number(now)), needsYou: () => null };
  }

  ids() {
    return this.#core.ids();
  }
}

// The runs XenoNet observes without a reader of its own, from each source
// there is: numbat's records (core/numbat.js) and the runs actors report.
export function observedSessions(sources) {
  const list = sources.filter(Boolean);
  return {
    sources: list,
    // One session per id: the first source to list it wins (Rust says
    // which those are).
    sessions: () => {
      const all = list.flatMap((s) => s.sessions());
      const first = native.observedFirsts(all.map((x) => x.id));
      return all.filter((_, i) => first[i]);
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
