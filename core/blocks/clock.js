// The clock the blocks keep time with. Tests pass one they move by hand
// (test/harness/clock.js), so a cadence, a backoff or a debounce runs the
// same way every time.
export const systemClock = {
  now: () => Date.now(),
  setTimeout: (fn, ms, ...args) => setTimeout(fn, ms, ...args),
  clearTimeout: (t) => clearTimeout(t),
  setInterval: (fn, ms, ...args) => setInterval(fn, ms, ...args),
  clearInterval: (t) => clearInterval(t),
};
