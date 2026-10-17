// core/actors/limits.js in Rust (crates/xenonet-actors/src/limits.rs):
// rates, as buckets of tokens. The time is `now()`'s, read here.
import { native } from "../../index.js";

export function createBucket({ rate, burst, now = Date.now }) {
  const b = new native.Bucket(Number(rate), Number(burst), now());
  return {
    take: (n = 1) => b.take(Number(n), now()),
    fullIn: () => b.fullIn(now()),
  };
}

export function createBuckets({ now = Date.now, max = 1024 } = {}) {
  const all = new native.Buckets(Number(max));
  return {
    take: (key, { rate, burst }, n = 1) => all.take(String(key), Number(rate), Number(burst), Number(n), now()),
    forget: (prefix) => all.forget(String(prefix)),
    get size() {
      return all.size;
    },
  };
}
