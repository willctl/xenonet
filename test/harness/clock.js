// A clock a test moves by hand. now() says what the test set, and timers
// fire only when the test moves the clock past them, in the order they fall
// due, so a cadence, a backoff or a debounce runs the same way every time.
//
//   const clock = fakeClock("2026-10-05T12:00:00Z");
//   const block = new Gatherer({ ..., clock });
//   await clock.advance(30e3); // runs every timer due in the next 30 s
//
// It has the shape the blocks take (core/blocks/clock.js): now, setTimeout,
// clearTimeout, setInterval and clearInterval. Handles answer unref() and
// ref() as Node's do, and do nothing.
export function fakeClock(start = "2026-10-05T12:00:00Z") {
  let now = typeof start === "number" ? start : Date.parse(start);
  let seq = 0;
  const timers = new Map(); // id → { at, fn, args, every }

  const handle = (id) => ({ id, unref: () => handle(id), ref: () => handle(id), hasRef: () => false, [Symbol.toPrimitive]: () => id });
  const idOf = (h) => (h && typeof h === "object" ? h.id : h);
  const add = (fn, ms, args, every) => {
    const id = ++seq;
    timers.set(id, { at: now + Math.max(0, Number(ms) || 0), fn, args, every });
    return handle(id);
  };

  // The next timer due at or before `until`: the earliest, and of those the
  // one set first.
  const next = (until) => {
    let best = null;
    for (const [id, t] of timers) if (t.at <= until && (!best || t.at < best.t.at || (t.at === best.t.at && id < best.id))) best = { id, t };
    return best;
  };

  const clock = {
    now: () => now,
    setTimeout: (fn, ms, ...args) => add(fn, ms, args, null),
    setInterval: (fn, ms, ...args) => add(fn, ms, args, Math.max(1, Number(ms) || 0)),
    clearTimeout: (h) => void timers.delete(idOf(h)),
    clearInterval: (h) => void timers.delete(idOf(h)),

    // Lets what the timers started run: promises, and work queued behind
    // them. Real file reads take longer; wait for those on the block itself.
    async flush(turns = 5) {
      for (let i = 0; i < turns; i++) await new Promise((r) => setImmediate(r));
    },

    // Moves the clock forward by `ms`, firing each timer that falls due on
    // the way at its own time, and letting what it started run before the
    // next one. What's already under way runs first, so a timer it is about
    // to set counts from now.
    async advance(ms) {
      await clock.flush();
      const until = now + ms;
      for (let n = next(until); n; n = next(until)) {
        now = Math.max(now, n.t.at);
        if (n.t.every) n.t.at += n.t.every;
        else timers.delete(n.id);
        n.t.fn(...n.t.args);
        await clock.flush();
      }
      now = until;
      await clock.flush();
    },

    // Sets the time without firing anything, as when a machine sleeps.
    set(t) {
      now = typeof t === "number" ? t : Date.parse(t);
    },

    // How many timers wait, and when the next one falls due.
    pending: () => timers.size,
    nextAt: () => Math.min(...[...timers.values()].map((t) => t.at)),
  };
  return clock;
}
