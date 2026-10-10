// Rates by key (core/actors/limits.js): a bucket that's full again can go
// to make room, and one still counting never does, so a caller can't make
// new keys to push out the buckets that count it. On a clock the test moves.
import { test } from "node:test";
import assert from "node:assert/strict";
import { createBucket, createBuckets } from "../core/actors/limits.js";

const SLOW = { rate: 1 / 3600, burst: 2 };
const FAST = { rate: 1, burst: 1 };

test("a bucket says how long until it's full again", () => {
  const clock = { t: 0 };
  const b = createBucket({ rate: 2, burst: 3, now: () => clock.t });
  assert.equal(b.fullIn(), 0);
  b.take(2);
  assert.equal(b.fullIn(), 1000);
  clock.t += 600;
  assert.equal(b.fullIn(), 400);
  clock.t += 400;
  assert.equal(b.fullIn(), 0);
});

test("with every place taken, a full bucket makes room and one still counting stays", () => {
  const clock = { t: 0 };
  const buckets = createBuckets({ now: () => clock.t, max: 3 });
  // The one that counts: spent, and an hour from full.
  buckets.take("a:wren|wide", SLOW, 2);
  assert.ok(buckets.take("a:wren|wide", SLOW) > 0);
  buckets.take("x-1", FAST);
  buckets.take("x-2", FAST);
  // Made-up keys while x-1 and x-2 still count: none gets in.
  assert.equal(buckets.take("x-3", FAST), 1000, "it waits for the first to be full again");
  assert.equal(buckets.size, 3);
  // A second on, x-1 and x-2 are full again and make room.
  clock.t += 1000;
  for (let i = 3; i < 100; i++) {
    assert.equal(buckets.take(`x-${i}`, FAST), 0, String(i));
    clock.t += 1000;
  }
  assert.ok(buckets.take("a:wren|wide", SLOW) > 0, "never pushed out, so still spent");
  // Forgetting an actor lets go of its own.
  buckets.forget("a:wren|");
  assert.equal(buckets.take("a:wren|wide", SLOW), 0);
});

test("the oldest of the full ones isn't the only one to go: every full bucket does", () => {
  const clock = { t: 0 };
  const buckets = createBuckets({ now: () => clock.t, max: 4 });
  for (const k of ["a", "b", "c", "d"]) buckets.take(k, FAST);
  clock.t += 1000;
  buckets.take("e", FAST);
  assert.equal(buckets.size, 1, "four full ones went at once");
});
