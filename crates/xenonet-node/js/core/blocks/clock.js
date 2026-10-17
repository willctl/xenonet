// core/blocks/clock.js beside Rust (crates/xenonet-blocks/src/clock.rs):
// the clock the blocks keep time with. Rust's cores take the time from
// whoever runs them, so this stays the JS's own: Date.now(), which a test
// can mock, and the JS timers, which a twin runs its cores on. The binary
// keeps time with Rust's SystemClock and tokio's timers.
export const systemClock = {
  now: () => Date.now(),
  setTimeout: (fn, ms, ...args) => setTimeout(fn, ms, ...args),
  clearTimeout: (t) => clearTimeout(t),
  setInterval: (fn, ms, ...args) => setInterval(fn, ms, ...args),
  clearInterval: (t) => clearInterval(t),
};
