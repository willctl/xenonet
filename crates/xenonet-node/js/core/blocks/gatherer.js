// core/blocks/gatherer.js in Rust (crates/xenonet-blocks/src/gatherer.rs):
// a snapshot from a source, when asked and when it's due, one gather at a
// time. Rust keeps the schedule and decides: whether a run is due, when
// the next falls due (the cadence, what the source asked for, the
// backoff), and whether a failure keeps the last good value or falls
// back. The values, the stamps, the gather and the cadence's timer stay
// here.
import { systemClock } from "../../../../../core/blocks/clock.js";
import { native } from "../../index.js";
import { Block, CORE, EMIT, messageOf } from "./lifecycle.js";

export class Gatherer extends Block {
  constructor({ name, kind = "gatherer", gather, everyMs = null, nextIn = null, backoff = null, stamp = null, retry = false, keep = false, fallback = null, clock = systemClock } = {}) {
    const b = backoff ? [Number(backoff.baseMs), backoff.factor ?? null, backoff.maxMs ?? null] : null;
    super({ name, kind, clock, core: new native.Gatherer(String(name), everyMs ?? null, typeof nextIn === "function", b, typeof stamp === "function", !!keep, clock.now()) });
    if (typeof gather !== "function") throw new Error(`gatherer ${name} needs something to gather`);
    this.gather = gather;
    this.everyMs = everyMs;
    this.nextIn = nextIn;
    this.backoff = backoff && { ...backoff, ...this[CORE].backoff };
    this.stamp = stamp;
    this.retry = retry;
    this.keep = keep;
    this.fallback = fallback;
    this.value = fallback;
    // { value, at, ms, ok, error, stamp } of the last gather, or null.
    this.last = null;
    this.running = null;
    this.timer = null;
  }

  get good() {
    return this[CORE].good;
  }
  get nextAt() {
    return this[CORE].nextAt;
  }
  get failures() {
    return this[CORE].failures;
  }

  due(now = this.clock.now()) {
    const same = !!this.last && !!this.stamp && Object.is(this.stamp(), this.last.stamp);
    return this[CORE].due(now, same);
  }

  // Gathers when due (or with `force`), and resolves with what the
  // gatherer serves. Runs at once share one gather.
  run({ force = false, now } = {}) {
    if (this.running) return this.running;
    const at = now ?? this.clock.now();
    if (!force && !this.due(at)) {
      this[CORE].skipped();
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

  // The same for a synchronous source.
  get({ force = false, now } = {}) {
    const at = now ?? this.clock.now();
    if (!force && !this.due(at)) {
      this[CORE].skipped();
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
    this.value = value;
    this.last = { value, at, ms, ok: true, error: null, stamp };
    const wait = this.nextIn?.(value);
    this[CORE].done(at, typeof wait === "number" ? wait : null);
    this[EMIT]();
    this.emit("gathered", this.last);
    return value;
  }

  #failed({ err, at, ms, stamp }) {
    const kept = this[CORE].failed(at, messageOf(err));
    if (!kept) this.value = this.fallback;
    this.last = { value: this.value, at, ms, ok: false, error: messageOf(err), stamp: this.retry ? this.last?.stamp : stamp };
    this[EMIT]();
    this.emit("gathered", this.last);
    return this.value;
  }

  // Forgets what it gathered, so the next run gathers again.
  forget() {
    this.last = null;
    this.value = this.fallback;
    this[CORE].forget();
  }

  // On a cadence: a run now (unless `immediately` is false), then another
  // each time one falls due. Needs `everyMs`.
  start({ immediately = true } = {}) {
    this.immediately = immediately;
    return super.start();
  }

  onStart(run) {
    if (!this.everyMs) return;
    const tick = () => {
      if (!this.current(run)) return;
      this.run().catch(() => {}).finally(() => this.current(run) && next());
    };
    const next = () => {
      this.timer = this.clock.setTimeout(tick, this[CORE].nextWait(this.clock.now()));
    };
    if (this.immediately) tick();
    else next();
  }

  onStop() {
    this.clock.clearTimeout(this.timer);
    this.timer = null;
  }
}
