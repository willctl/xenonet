// A gatherer takes a snapshot from a source: a list of sessions from an
// app, a file XenoNet reads, a reading from a web API, a command's output. It
// gathers when asked and when it's due, one gather at a time, times each
// one, and after a failure either keeps serving what it had or falls back.
//
//   const sessions = new Gatherer({ name: "claude-sessions", gather: listClaudeSessions });
//   await sessions.run();            // gathers now: no cadence, always due
//   const usage = new Gatherer({ name: "claude-usage", gather: fetchUsage, everyMs: 5 * 60e3, nextIn: (r) => r.retryAfterMs });
//   await usage.run({ now });        // gathers at most every 5 minutes, or later when the source says
//   const account = new Gatherer({ name: "claude-account", gather: readIt, stamp: () => mtimeOf(file) });
//   account.get();                   // a synchronous source, read again only when its mtime changes
//
// When it's due:
//   - with `stamp`, when the stamp (a file's mtime, say) differs from the
//     one at the last gather;
//   - otherwise once `everyMs` has passed since the last gather (always,
//     without it), or the time the source asked for (`nextIn`), or, after
//     failures, the backoff's delay, which doubles with each one.
// A run that isn't due serves what the gatherer has. `force` gathers
// anyway.
//
// A failure is a gather that throws. With `keep`, the gatherer serves the
// last good value and is degraded; without it, or with nothing to keep, it
// serves `fallback` and is failed. Either way the next success makes it
// running again. With a stamp, a failed gather waits for the stamp to
// change, like a good one (a broken file isn't read again until it's
// written), unless `retry` says to try again on the next run.
import { Block, messageOf } from "./lifecycle.js";

export class Gatherer extends Block {
  constructor({ name, kind = "gatherer", gather, everyMs = null, nextIn = null, backoff = null, stamp = null, retry = false, keep = false, fallback = null, clock } = {}) {
    super({ name, kind, clock });
    if (typeof gather !== "function") throw new Error(`gatherer ${name} needs something to gather`);
    this.gather = gather;
    this.everyMs = everyMs;
    this.nextIn = nextIn;
    this.backoff = backoff && { factor: 2, ...backoff, maxMs: backoff.maxMs ?? backoff.baseMs * 32 };
    this.stamp = stamp;
    this.retry = retry;
    this.keep = keep;
    this.fallback = fallback;
    this.value = fallback;
    // { value, at, ms, ok, error, stamp } of the last gather, or null.
    this.last = null;
    this.good = false; // whether `value` came from a gather that worked
    this.nextAt = 0;
    this.failures = 0;
    this.running = null;
    this.timer = null;
  }

  // Whether a run now would gather. Without a cadence, a backoff or a
  // source that says when, always: even if the system clock steps back.
  due(now = this.clock.now()) {
    if (!this.last) return true;
    if (this.stamp) return !Object.is(this.stamp(), this.last.stamp);
    if (!this.everyMs && !this.nextIn && !this.backoff) return true;
    return now >= this.nextAt;
  }

  // Gathers when due (or with `force`), and resolves with what the
  // gatherer serves. Runs at once share one gather.
  run({ force = false, now } = {}) {
    if (this.running) return this.running;
    const at = now ?? this.clock.now();
    if (!force && !this.due(at)) {
      this.count("skipped");
      return Promise.resolve(this.value);
    }
    const stamp = this.stamp?.();
    const t0 = this.clock.now();
    let pending;
    try {
      pending = Promise.resolve(this.gather({ now: at, last: this.last }));
    } catch (err) {
      pending = Promise.reject(err);
    }
    this.running = pending.then(
      (value) => this.#done({ value, at, ms: this.clock.now() - t0, stamp }),
      (err) => this.#failed({ err, at, ms: this.clock.now() - t0, stamp }),
    ).finally(() => (this.running = null));
    return this.running;
  }

  // The same for a synchronous source: gathers when due, and returns what
  // the gatherer serves.
  get({ force = false, now } = {}) {
    const at = now ?? this.clock.now();
    if (!force && !this.due(at)) {
      this.count("skipped");
      return this.value;
    }
    const stamp = this.stamp?.();
    const t0 = this.clock.now();
    let value;
    try {
      value = this.gather({ now: at, last: this.last });
    } catch (err) {
      return this.#failed({ err, at, ms: this.clock.now() - t0, stamp });
    }
    if (typeof value?.then === "function") throw new Error(`gatherer ${this.name} has an asynchronous source: use run()`);
    return this.#done({ value, at, ms: this.clock.now() - t0, stamp });
  }

  #done({ value, at, ms, stamp }) {
    this.count("runs");
    this.failures = 0;
    this.value = value;
    this.good = true;
    this.last = { value, at, ms, ok: true, error: null, stamp };
    const wait = this.nextIn?.(value);
    this.nextAt = at + (Number.isFinite(wait) && wait > 0 ? wait : (this.everyMs ?? 0));
    this.ok(at);
    this.emit("gathered", this.last);
    return value;
  }

  #failed({ err, at, ms, stamp }) {
    this.count("runs");
    this.failures++;
    const kept = this.keep && this.good;
    if (!kept) {
      this.value = this.fallback;
      this.good = false;
    }
    this.last = { value: this.value, at, ms, ok: false, error: messageOf(err), stamp: this.retry ? this.last?.stamp : stamp };
    const { baseMs, factor, maxMs } = this.backoff ?? {};
    this.nextAt = at + (this.backoff ? Math.min(maxMs, baseMs * factor ** (this.failures - 1)) : (this.everyMs ?? 0));
    if (kept) this.problem(err, at);
    else this.fail(err, at);
    this.emit("gathered", this.last);
    return this.value;
  }

  // Forgets what it gathered, so the next run gathers again.
  forget() {
    this.last = null;
    this.value = this.fallback;
    this.good = false;
    this.nextAt = 0;
    this.failures = 0;
  }

  // On a cadence: a run now (unless `immediately` is false), then another
  // each time one falls due. Needs `everyMs`.
  start({ immediately = true } = {}) {
    this.immediately = immediately;
    return super.start();
  }

  // Each start has a cadence of its own, which its stop ends: a tick of a
  // start that no longer stands (stopped, and maybe started again while
  // its gather ran) neither gathers nor sets another timer, so one stop
  // ends them all.
  onStart(run) {
    if (!this.everyMs) return;
    const tick = () => {
      if (!this.current(run)) return;
      this.run().catch(() => {}).finally(() => this.current(run) && next());
    };
    const next = () => {
      const wait = this.last ? Math.max(0, this.nextAt - this.clock.now()) : this.everyMs;
      this.timer = this.clock.setTimeout(tick, wait);
    };
    if (this.immediately) tick();
    else next();
  }

  onStop() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
