// Rates, as buckets of tokens (docs/design/actors/interface.md, section 2,
// "Limits"): up to `burst` at once, filled again at `rate` a second.
// take(n) takes n (one by default) and is 0 when there were enough, and
// otherwise takes none and says how many milliseconds until there are, for
// Retry-After. More than `burst` at once is never there.
export function createBucket({ rate, burst, now = Date.now }) {
  let tokens = burst;
  let at = now();
  const fill = () => {
    const t = now();
    tokens = Math.min(burst, tokens + (Math.max(0, t - at) / 1000) * rate);
    at = t;
  };
  return {
    take(n = 1) {
      fill();
      if (tokens >= n) {
        tokens -= n;
        return 0;
      }
      if (n > burst) return Infinity;
      return Math.ceil(((n - tokens) / rate) * 1000);
    },
    // Milliseconds until it's full again: 0 once a new bucket would be the
    // same as this one.
    fullIn() {
      fill();
      return tokens >= burst ? 0 : Math.ceil(((burst - tokens) / rate) * 1000);
    },
  };
}

// Buckets by key (an actor, an instance, a process), made as they're first
// needed, `max` at most. A full bucket can go, since a new one would be
// full too; one that isn't never does, or a caller could make new keys to
// push out the buckets counting it. So with `max` buckets all in use, a new
// key waits until one is full again.
export function createBuckets({ now = Date.now, max = 1024 } = {}) {
  const all = new Map();
  return {
    take(key, { rate, burst }, n = 1) {
      let b = all.get(key);
      if (!b) {
        if (all.size >= max) {
          let soonest = Infinity;
          for (const [k, other] of all) {
            const ms = other.fullIn();
            if (ms === 0) all.delete(k);
            else soonest = Math.min(soonest, ms);
          }
          if (all.size >= max) return soonest;
        }
        all.set(key, (b = createBucket({ rate, burst, now })));
      }
      return b.take(n);
    },
    // Lets go of every bucket whose key starts with `prefix` (an actor gone).
    forget(prefix) {
      for (const key of [...all.keys()]) if (key.startsWith(prefix)) all.delete(key);
    },
    get size() {
      return all.size;
    },
  };
}
